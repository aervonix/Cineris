//! The editor's prompt history on disk: the state directory's `history`
//! (`docs/architecture/08-configuration.md`, "The state directory"), one
//! JSON line for each prompt sent, with the project it was sent in
//! (`docs/architecture/09-tui.md`, "History").
//!
//! - **The interface never waits on the disk.** One thread of the store's
//!   own reads the file, keeps this project's newest [`KEEP`] prompts, and
//!   then appends each prompt sent, in order. Until it has read the file,
//!   the store holds only what was sent in this run.
//! - **It is private,** created for the user alone, as every file of the
//!   state directory is, and a line it cannot read is passed over.
//! - **It stays small.** When it holds more than [`COMPACT`] lines, the
//!   reader keeps each project's newest [`KEEP`] and writes the file again
//!   whole, under the store's lock, so that another process's appends are
//!   not lost meanwhile.
//! - **What was sent is kept at the end.** Dropping the store waits, at
//!   most [`FLUSH_WAIT`], for the thread to write what it was handed.

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cortex_tui::history::HistoryStore;
use serde_json::{Value, json};

/// The most prompts of a project kept.
pub(crate) const KEEP: usize = 1000;

/// Past this many lines the file is written again with each project's
/// newest [`KEEP`].
pub(crate) const COMPACT: usize = 20_000;

/// How long the end of the program waits for the last prompts to reach
/// the disk.
const FLUSH_WAIT: Duration = Duration::from_secs(1);

/// One line of the file.
#[derive(Debug)]
struct Entry {
    project: String,
    text: String,
}

impl Entry {
    /// The line read, when it is one of the store's.
    fn parse(line: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line).ok()?;
        Some(Self {
            project: value.get("project")?.as_str()?.to_owned(),
            text: value.get("text")?.as_str()?.to_owned(),
        })
    }

    /// The line to write, with its end.
    fn line(&self) -> String {
        let mut line = json!({"project": self.project, "text": self.text}).to_string();
        line.push('\n');
        line
    }
}

/// The prompt history of one project, kept in the state directory's
/// `history`. See the [module documentation](self).
#[derive(Debug)]
pub(crate) struct FileHistory {
    /// The project's earlier prompts, oldest first, once the file is read.
    earlier: Arc<OnceLock<Vec<String>>>,
    /// The prompts sent in this run, oldest first.
    sent: Vec<String>,
    /// The writer's queue; `None` once it has gone.
    writer: Option<Sender<String>>,
    /// Said when the writer has written everything and ended.
    done: Option<Receiver<()>>,
}

impl FileHistory {
    /// The history of the project `project` in the file at `path`, read
    /// and kept on a thread of its own.
    pub(crate) fn open(path: PathBuf, project: String) -> Self {
        let earlier = Arc::new(OnceLock::new());
        let (writer, queue) = mpsc::channel::<String>();
        let (finished, done) = mpsc::channel();
        let filled = Arc::clone(&earlier);
        let spawned = std::thread::Builder::new()
            .name("cortex-history".to_owned())
            .spawn(move || {
                let _ = filled.set(read(&path, &project));
                append(&path, &project, &queue);
                let _ = finished.send(());
            });
        let (writer, done) = match spawned {
            Ok(_) => (Some(writer), Some(done)),
            // No thread: a history in memory, for this run alone.
            Err(_) => {
                let _ = earlier.set(Vec::new());
                (None, None)
            }
        };
        Self {
            earlier,
            sent: Vec::new(),
            writer,
            done,
        }
    }

    /// Whether the earlier prompts have been read.
    #[cfg(test)]
    pub(crate) fn is_read(&self) -> bool {
        self.earlier.get().is_some()
    }

    fn earlier(&self) -> &[String] {
        self.earlier.get().map_or(&[], Vec::as_slice)
    }
}

impl HistoryStore for FileHistory {
    fn len(&self) -> usize {
        self.earlier().len() + self.sent.len()
    }

    fn get(&self, index: usize) -> Option<&str> {
        let earlier = self.earlier();
        match earlier.get(index) {
            Some(prompt) => Some(prompt),
            None => self.sent.get(index - earlier.len()).map(String::as_str),
        }
    }

    fn push(&mut self, prompt: &str) {
        let newest = self.sent.last().or_else(|| self.earlier().last());
        if prompt.trim().is_empty() || newest.is_some_and(|newest| newest == prompt) {
            return;
        }
        self.sent.push(prompt.to_owned());
        if let Some(writer) = &self.writer
            && writer.send(prompt.to_owned()).is_err()
        {
            self.writer = None;
        }
    }
}

impl Drop for FileHistory {
    fn drop(&mut self) {
        // The queue closes, and the writer ends once it has written it.
        self.writer = None;
        if let Some(done) = self.done.take() {
            let _ = done.recv_timeout(FLUSH_WAIT);
        }
    }
}

/// The project's newest prompts in the file, oldest first. The file is
/// written again, smaller, when it has grown past [`COMPACT`] lines.
fn read(path: &Path, project: &str) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let entries: Vec<Entry> = text.lines().filter_map(Entry::parse).collect();
    let mine: Vec<String> = newest(entries.iter().filter(|entry| entry.project == project));
    if entries.len() > COMPACT {
        let _ = compact(path);
    }
    mine
}

/// The newest [`KEEP`] of `entries`, oldest first.
fn newest<'a>(entries: impl DoubleEndedIterator<Item = &'a Entry>) -> Vec<String> {
    let mut kept: Vec<String> = entries
        .rev()
        .take(KEEP)
        .map(|entry| entry.text.clone())
        .collect();
    kept.reverse();
    kept
}

/// Writes the file again with each project's newest [`KEEP`], under the
/// store's lock: read again under it, so that what another process
/// appended meanwhile is kept.
fn compact(path: &Path) -> io::Result<()> {
    let _lock = cortex_base::private::hold_lock(path)?;
    let text = std::fs::read_to_string(path)?;
    let entries: Vec<Entry> = text.lines().filter_map(Entry::parse).collect();
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut kept: Vec<&Entry> = Vec::new();
    for entry in entries.iter().rev() {
        let count = counts.entry(entry.project.as_str()).or_insert(0);
        if *count < KEEP {
            *count += 1;
            kept.push(entry);
        }
    }
    kept.reverse();
    let mut out = String::new();
    for entry in kept {
        out.push_str(&entry.line());
    }
    let options = cortex_base::AtomicWriteOptions {
        owner_only: true,
        ..cortex_base::AtomicWriteOptions::default()
    };
    cortex_base::write_atomic(
        path,
        out.as_bytes(),
        &options,
        &cortex_base::SystemClock::new(),
    )
    .map_err(|failure| io::Error::other(failure.to_string()))
}

/// Appends each prompt handed over, one line each, until the queue closes:
/// under the store's lock, the file opened for each line, so that a
/// compaction by another process, which writes a new file in the old one's
/// place under the same lock, keeps every line before it and has every line
/// after it (the review of step 23's second pull request).
fn append(path: &Path, project: &str, queue: &Receiver<String>) {
    for text in queue {
        let entry = Entry {
            project: project.to_owned(),
            text,
        };
        let _lock = cortex_base::private::hold_lock(path).ok().flatten();
        if let Ok(mut file) = open(path) {
            // One write of a whole line: another process's lines are not
            // cut into.
            let _ = file.write_all(entry.line().as_bytes());
        }
    }
}

/// The file, open to append to: created private when it is not there.
fn open(path: &Path) -> io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        cortex_base::private::create_dir_all(parent)?;
    }
    match cortex_base::private::create_new(path) {
        Ok(file) => Ok(file),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            OpenOptions::new().append(true).open(path)
        }
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of the test's own, gone when it ends.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("cortex-cli-history-{}-{test}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn wait_read(history: &FileHistory) {
        // (The reader is a thread of its own, which this waits for; one that
        // never read is stopped by the test's time limit.)
        while !history.is_read() {
            std::thread::yield_now();
        }
    }

    #[test]
    fn prompts_sent_are_there_at_the_next_start_for_their_project_alone() {
        let scratch = Scratch::new("file");
        let path = scratch.join("state").join("history");
        let mut first = FileHistory::open(path.clone(), "p_one".to_owned());
        wait_read(&first);
        assert!(first.is_empty());
        first.push("fix the build");
        first.push("fix the build");
        first.push("and the docs");
        let mut other = FileHistory::open(path.clone(), "p_two".to_owned());
        wait_read(&other);
        other.push("another project's");
        drop(first);
        drop(other);

        let again = FileHistory::open(path.clone(), "p_one".to_owned());
        wait_read(&again);
        assert_eq!(
            again.len(),
            2,
            "a prompt the same as the newest is kept once"
        );
        assert_eq!(again.get(0), Some("fix the build"));
        assert_eq!(again.get(1), Some("and the docs"));
        assert!(cortex_base::private::is_private(&path).unwrap());
    }

    #[test]
    fn a_line_that_cannot_be_read_is_passed_over() {
        let scratch = Scratch::new("bad_line");
        let path = scratch.join("history");
        std::fs::write(
            &path,
            "not json\n{\"project\":\"p\",\"text\":\"kept\"}\n{\"project\":\"p\"}\n",
        )
        .unwrap();
        let history = FileHistory::open(path, "p".to_owned());
        wait_read(&history);
        assert_eq!(history.len(), 1);
        assert_eq!(history.get(0), Some("kept"));
    }

    /// A prompt sent after another process wrote the file again (its
    /// compaction) is in the file it wrote, not lost in the one it
    /// replaced.
    #[test]
    fn a_prompt_after_another_process_compacts_the_file_is_kept() {
        let scratch = Scratch::new("after_compact");
        let path = scratch.join("history");
        let mut history = FileHistory::open(path.clone(), "p".to_owned());
        wait_read(&history);
        history.push("before");
        // (The writer is a thread of its own, which this waits for.)
        while !std::fs::read_to_string(&path).is_ok_and(|text| text.contains("before")) {
            std::thread::yield_now();
        }
        compact(&path).expect("the compaction");
        history.push("after");
        drop(history);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("before") && text.contains("after"), "{text}");
    }

    /// A file past its size is written again with each project's newest.
    #[test]
    fn a_long_file_keeps_each_projects_newest() {
        let scratch = Scratch::new("compact");
        let path = scratch.join("history");
        let mut text = String::new();
        for n in 0..=COMPACT {
            let project = if n % 2 == 0 { "a" } else { "b" };
            text.push_str(&format!(
                "{{\"project\":\"{project}\",\"text\":\"prompt {n}\"}}\n"
            ));
        }
        std::fs::write(&path, text).unwrap();
        let history = FileHistory::open(path.clone(), "a".to_owned());
        wait_read(&history);
        assert_eq!(history.len(), KEEP);
        assert_eq!(
            history.get(KEEP - 1),
            Some(format!("prompt {COMPACT}").as_str())
        );
        drop(history);
        let lines = std::fs::read_to_string(&path).unwrap().lines().count();
        assert_eq!(lines, 2 * KEEP, "each project's newest {KEEP}");
    }
}
