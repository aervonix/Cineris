//! The editor's prompt history on disk: the state directory's `history`
//! (`docs/architecture/08-configuration.md`, "The state directory"), one
//! JSON line for each prompt sent, with the project it was sent in
//! (`docs/architecture/09-tui.md`, "History").
//!
//! - **A prompt is kept as it was written** (since the follow-ups of step
//!   23): `{"project", "text", "chips"}`, its text with a U+FFFC for each
//!   chip and each chip's label and text, within the bounds
//!   `cineris_tui::history::kept` sets (64 KiB of a chip's text). A prompt
//!   with no chip is the line it always was, `{"project", "text"}`.
//! - **The interface never waits on the disk.** One thread of the store's
//!   own reads the file, keeps this project's newest prompts (at most
//!   [`KEEP`], and [`KEEP_BYTES`] of lines), and then appends each prompt
//!   sent, in order. Until it has read the file, the store holds only what
//!   was sent in this run.
//! - **It is private,** created for the user alone, as every file of the
//!   state directory is, and a line it cannot read is passed over.
//! - **It stays small.** When it holds more than [`COMPACT`] lines or
//!   [`COMPACT_BYTES`], the reader writes it again whole, under the store's
//!   lock (so that another process's appends are not lost meanwhile), with
//!   each project's newest within [`KEEP`] and [`KEEP_BYTES`], and the
//!   newest of all within [`FILE_KEEP_BYTES`]. A file larger than
//!   [`READ_LIMIT`] (an older build's, which kept pastes whole) is read from
//!   its last [`READ_LIMIT`] alone, so a start never reads more.
//! - **What was sent is kept at the end.** Dropping the store waits, at
//!   most [`FLUSH_WAIT`], for the thread to write what it was handed.

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use cineris_tui::editor::{Chip, Fragment};
use cineris_tui::history::HistoryStore;
use serde_json::{Value, json};

/// The most prompts of a project kept.
pub(crate) const KEEP: usize = 1000;

/// The most bytes of a project's lines kept, newest first: what its
/// history can hold in memory for the run.
pub(crate) const KEEP_BYTES: usize = 2 * 1024 * 1024;

/// Past this many lines the file is written again.
pub(crate) const COMPACT: usize = 20_000;

/// Past this many bytes the file is written again.
pub(crate) const COMPACT_BYTES: u64 = 8 * 1024 * 1024;

/// The most bytes a file written again holds, the newest lines of every
/// project together.
pub(crate) const FILE_KEEP_BYTES: usize = 4 * 1024 * 1024;

/// The most bytes read of the file: its end, when it is longer.
pub(crate) const READ_LIMIT: u64 = 16 * 1024 * 1024;

/// The bounds a history keeps to: the constants above in the program. A
/// test gives smaller ones, in the same order, so that a file past them is
/// a megabyte and not tens of them (`docs/steps/ci-budgets/answered.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    /// [`KEEP`].
    pub(crate) keep: usize,
    /// [`KEEP_BYTES`].
    pub(crate) keep_bytes: usize,
    /// [`COMPACT`].
    pub(crate) compact: usize,
    /// [`COMPACT_BYTES`].
    pub(crate) compact_bytes: u64,
    /// [`FILE_KEEP_BYTES`].
    pub(crate) file_keep_bytes: usize,
    /// [`READ_LIMIT`].
    pub(crate) read_limit: u64,
}

impl Limits {
    /// The program's.
    pub(crate) const PROGRAM: Self = Self {
        keep: KEEP,
        keep_bytes: KEEP_BYTES,
        compact: COMPACT,
        compact_bytes: COMPACT_BYTES,
        file_keep_bytes: FILE_KEEP_BYTES,
        read_limit: READ_LIMIT,
    };
}

/// How long the end of the program waits for the last prompts to reach
/// the disk.
const FLUSH_WAIT: Duration = Duration::from_secs(1);

/// One line of the file.
#[derive(Debug)]
struct Entry {
    project: String,
    prompt: Fragment,
    /// The bytes of its line as this build writes it, its end included:
    /// within the bounds, whatever wrote the line (an older build's line of
    /// a pasted log counts as the chip it is kept as, the review of the
    /// follow-ups of step 23).
    size: usize,
}

impl Entry {
    /// The line read, when it is one of the store's.
    fn parse(line: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(line).ok()?;
        let text = value.get("text")?.as_str()?;
        let prompt = match value.get("chips") {
            None => Fragment::text(text),
            Some(chips) => {
                let chips = chips
                    .as_array()?
                    .iter()
                    .map(|chip| {
                        Some(Chip::new(
                            chip.get("text")?.as_str()?,
                            chip.get("label")?.as_str()?,
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                Fragment::with_chips(text, chips)?
            }
        };
        let project = value.get("project")?.as_str()?.to_owned();
        // Within the bounds, whatever wrote the line.
        let prompt = cineris_tui::history::kept(&prompt);
        let size = if prompt.chips().is_empty() && prompt.as_str() == text {
            line.len() + 1
        } else {
            Self::line(&project, &prompt).len()
        };
        Some(Self {
            project,
            prompt,
            size,
        })
    }

    /// The line to write, with its end.
    fn line(project: &str, prompt: &Fragment) -> String {
        let mut line = if prompt.chips().is_empty() {
            json!({"project": project, "text": prompt.as_str()})
        } else {
            let chips: Vec<Value> = prompt
                .chips()
                .iter()
                .map(|chip| json!({"label": chip.label(), "text": chip.content()}))
                .collect();
            json!({"project": project, "text": prompt.as_str(), "chips": chips})
        }
        .to_string();
        line.push('\n');
        line
    }
}

/// The prompt history of one project, kept in the state directory's
/// `history`. See the [module documentation](self).
#[derive(Debug)]
pub(crate) struct FileHistory {
    /// The project's earlier prompts, oldest first, once the file is read.
    earlier: Arc<OnceLock<Vec<Fragment>>>,
    /// How many of the earliest of `earlier` are no longer held: dropped
    /// for the prompts sent since, so that the whole stays within [`KEEP`]
    /// and [`KEEP_BYTES`] (the review of the follow-ups of step 23).
    skipped: usize,
    /// The prompts sent in this run, oldest first.
    sent: std::collections::VecDeque<Fragment>,
    /// The writer's queue; `None` once it has gone.
    writer: Option<Sender<Fragment>>,
    /// Said when the writer has written everything and ended.
    done: Option<Receiver<()>>,
    /// The bounds it keeps to.
    limits: Limits,
}

impl FileHistory {
    /// The history of the project `project` in the file at `path`, read
    /// and kept on a thread of its own.
    pub(crate) fn open(path: PathBuf, project: String) -> Self {
        Self::open_with(path, project, Limits::PROGRAM)
    }

    /// [`open`](Self::open), within `limits`.
    pub(crate) fn open_with(path: PathBuf, project: String, limits: Limits) -> Self {
        let earlier = Arc::new(OnceLock::new());
        let (writer, queue) = mpsc::channel::<Fragment>();
        let (finished, done) = mpsc::channel();
        let filled = Arc::clone(&earlier);
        let spawned = std::thread::Builder::new()
            .name("cineris-history".to_owned())
            .spawn(move || {
                let _ = filled.set(read(&path, &project, limits));
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
            skipped: 0,
            sent: std::collections::VecDeque::new(),
            writer,
            done,
            limits,
        }
    }

    /// Drops the oldest held until what is held is within [`KEEP`] and
    /// [`KEEP_BYTES`], the newest always kept.
    fn bound(&mut self) {
        let mut held: usize = self
            .held_earlier()
            .iter()
            .chain(self.sent.iter())
            .map(held_bytes)
            .sum();
        while self.len() > 1 && (self.len() > self.limits.keep || held > self.limits.keep_bytes) {
            let dropped = if self.skipped < self.earlier().len() {
                self.skipped += 1;
                held_bytes(&self.earlier()[self.skipped - 1])
            } else {
                self.sent
                    .pop_front()
                    .map_or(0, |prompt| held_bytes(&prompt))
            };
            held = held.saturating_sub(dropped);
        }
    }

    /// Whether the earlier prompts have been read.
    #[cfg(test)]
    pub(crate) fn is_read(&self) -> bool {
        self.earlier.get().is_some()
    }

    /// The earlier prompts still held.
    fn earlier(&self) -> &[Fragment] {
        self.earlier.get().map_or(&[], Vec::as_slice)
    }

    fn held_earlier(&self) -> &[Fragment] {
        let earlier = self.earlier();
        &earlier[self.skipped.min(earlier.len())..]
    }
}

/// What a prompt takes in memory: its text and its chips'.
fn held_bytes(prompt: &Fragment) -> usize {
    prompt.as_str().len()
        + prompt
            .chips()
            .iter()
            .map(|chip| chip.content().len() + chip.label().len())
            .sum::<usize>()
}

impl HistoryStore for FileHistory {
    fn len(&self) -> usize {
        self.held_earlier().len() + self.sent.len()
    }

    fn get(&self, index: usize) -> Option<&Fragment> {
        let earlier = self.held_earlier();
        match earlier.get(index) {
            Some(prompt) => Some(prompt),
            None => self.sent.get(index - earlier.len()),
        }
    }

    fn push(&mut self, prompt: &Fragment) {
        // Within the bounds, whoever handed it over.
        let prompt = cineris_tui::history::kept(prompt);
        let newest = self.sent.back().or_else(|| self.held_earlier().last());
        if cineris_tui::history::is_blank(&prompt) || newest.is_some_and(|newest| *newest == prompt)
        {
            return;
        }
        self.sent.push_back(prompt.clone());
        self.bound();
        if let Some(writer) = &self.writer
            && writer.send(prompt).is_err()
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
/// written again, smaller, when it has grown past [`COMPACT`] lines or
/// [`COMPACT_BYTES`].
fn read(path: &Path, project: &str, limits: Limits) -> Vec<Fragment> {
    let Ok((bytes, size)) = read_end(path, limits.read_limit) else {
        return Vec::new();
    };
    // Newest first, and only until the project's bounds are met: what is
    // held is the project's newest, never every line of the file (the
    // follow-ups of step 23). A line names its project as this harness
    // writes it, a key in hexadecimal, which JSON writes as it is; a line
    // that does not hold it is not parsed.
    let marker = format!("\"project\":{}", json!(project));
    let mut mine = Vec::new();
    let mut held = 0;
    for line in newest_first(&bytes) {
        if mine.len() == limits.keep {
            break;
        }
        if !line.contains(&marker) {
            continue;
        }
        let Some(entry) = Entry::parse(line).filter(|entry| entry.project == project) else {
            continue;
        };
        held += entry.size;
        if held > limits.keep_bytes {
            break;
        }
        mine.push(entry.prompt);
    }
    mine.reverse();
    let lines = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .count();
    if lines > limits.compact || size > limits.compact_bytes {
        let _ = compact(path, limits);
    }
    mine
}

/// The file's last `read_limit` bytes ([`READ_LIMIT`]), from the start of a
/// line, and the file's whole size.
fn read_end(path: &Path, read_limit: u64) -> io::Result<(Vec<u8>, u64)> {
    use std::io::{Read as _, Seek as _};
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    if size <= read_limit {
        let mut bytes = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        file.read_to_end(&mut bytes)?;
        return Ok((bytes, size));
    }
    file.seek(io::SeekFrom::Start(size - read_limit))?;
    let mut bytes = Vec::new();
    file.take(read_limit).read_to_end(&mut bytes)?;
    // The first line read may be the end of one: it is passed over.
    let start = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(bytes.len(), |at| at + 1);
    bytes.drain(..start);
    Ok((bytes, size))
}

/// The file's lines, newest first: a line that is not UTF-8 is passed
/// over, and the others are given (the review of step 23: one bad byte
/// emptied every project's history).
fn newest_first(bytes: &[u8]) -> impl Iterator<Item = &str> {
    bytes
        .split(|byte| *byte == b'\n')
        .rev()
        .filter_map(|line| std::str::from_utf8(line.strip_suffix(b"\r").unwrap_or(line)).ok())
}

/// Writes the file again with each project's newest within [`KEEP`] and
/// [`KEEP_BYTES`], and all of them within [`FILE_KEEP_BYTES`], under the
/// store's lock: read again under it, so that what another process
/// appended meanwhile is kept.
fn compact(path: &Path, limits: Limits) -> io::Result<()> {
    let _lock = cineris_base::private::hold_lock(path)?;
    let (bytes, _) = read_end(path, limits.read_limit)?;
    // Newest first, one line parsed at a time: what is held is what is
    // kept, never every line of the file.
    let mut counts: std::collections::HashMap<String, (usize, usize)> =
        std::collections::HashMap::new();
    let mut total = 0;
    let mut kept: Vec<Entry> = Vec::new();
    for entry in newest_first(&bytes).filter_map(Entry::parse) {
        if total + entry.size > limits.file_keep_bytes {
            break;
        }
        let (count, held) = counts.entry(entry.project.clone()).or_insert((0, 0));
        if *count < limits.keep && *held + entry.size <= limits.keep_bytes {
            *count += 1;
            *held += entry.size;
            total += entry.size;
            kept.push(entry);
        }
    }
    kept.reverse();
    let mut out = String::with_capacity(total);
    for entry in &kept {
        out.push_str(&Entry::line(&entry.project, &entry.prompt));
    }
    let options = cineris_base::AtomicWriteOptions {
        owner_only: true,
        ..cineris_base::AtomicWriteOptions::default()
    };
    cineris_base::write_atomic(
        path,
        out.as_bytes(),
        &options,
        &cineris_base::SystemClock::new(),
    )
    .map_err(|failure| io::Error::other(failure.to_string()))
}

/// Appends each prompt handed over, one line each, until the queue closes:
/// under the store's lock, the file opened for each line, so that a
/// compaction by another process, which writes a new file in the old one's
/// place under the same lock, keeps every line before it and has every line
/// after it (the review of step 23's second pull request).
fn append(path: &Path, project: &str, queue: &Receiver<Fragment>) {
    for prompt in queue {
        let _lock = cineris_base::private::hold_lock(path).ok().flatten();
        if let Ok(mut file) = open(path) {
            // A last line cut short (a write a kill or a full disk ended)
            // is ended first, so that this one is not glued to it and both
            // lost (the review of step 23).
            let mut line = if unfinished(path) {
                "\n".to_owned()
            } else {
                String::new()
            };
            line.push_str(&Entry::line(project, &prompt));
            // One write of a whole line: another process's lines are not
            // cut into.
            let _ = file.write_all(line.as_bytes());
        }
    }
}

/// Whether the file's last line has no end.
fn unfinished(path: &Path) -> bool {
    use std::io::{Read as _, Seek as _};
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut last = [0_u8];
    file.seek(io::SeekFrom::End(-1)).is_ok()
        && file.read_exact(&mut last).is_ok()
        && last[0] != b'\n'
}

/// The file, open to append to: created private when it is not there.
fn open(path: &Path) -> io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        cineris_base::private::create_dir_all(parent)?;
    }
    match cineris_base::private::create_new(path) {
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
                .join(format!("cineris-cli-history-{}-{test}", std::process::id()));
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

    /// Smaller bounds, in the program's order, a project's still past what
    /// one prompt holds at most ([`cineris_tui::history::TEXT_KEEP`] of
    /// text and [`cineris_tui::history::CHIP_TEXT_KEEP`] of chips): a file
    /// past them is a megabyte or two, where the program's are tens of them.
    const SMALL: Limits = Limits {
        keep: 50,
        keep_bytes: 384 * 1024,
        compact: 1_000,
        compact_bytes: 1536 * 1024,
        file_keep_bytes: 768 * 1024,
        read_limit: 4 * 1024 * 1024,
    };

    /// The tests at [`SMALL`] stand for the program only while both keep
    /// their bounds in one order, and a file past its bytes, of three
    /// projects in turn, holds more of each than a project keeps: else the
    /// bound on what a project holds is never reached (the review of the
    /// CI-budgets fix).
    #[test]
    fn the_small_limits_keep_the_programs_order() {
        for limits in [Limits::PROGRAM, SMALL] {
            assert!(limits.keep_bytes < limits.file_keep_bytes, "{limits:?}");
            assert!(
                (limits.file_keep_bytes as u64) < limits.compact_bytes,
                "{limits:?}"
            );
            assert!(limits.compact_bytes < limits.read_limit, "{limits:?}");
            assert!(limits.keep < limits.compact, "{limits:?}");
            let one = cineris_tui::history::TEXT_KEEP + cineris_tui::history::CHIP_TEXT_KEEP;
            assert!(limits.keep_bytes > one, "{limits:?}");
            assert!(
                limits.compact_bytes / 3 > limits.keep_bytes as u64,
                "{limits:?}"
            );
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
        first.push(&Fragment::text("fix the build"));
        first.push(&Fragment::text("fix the build"));
        first.push(&Fragment::text("and the docs"));
        let mut other = FileHistory::open(path.clone(), "p_two".to_owned());
        wait_read(&other);
        other.push(&Fragment::text("another project's"));
        drop(first);
        drop(other);

        let again = FileHistory::open(path.clone(), "p_one".to_owned());
        wait_read(&again);
        assert_eq!(
            again.len(),
            2,
            "a prompt the same as the newest is kept once"
        );
        assert_eq!(again.get(0).map(Fragment::as_str), Some("fix the build"));
        assert_eq!(again.get(1).map(Fragment::as_str), Some("and the docs"));
        assert!(cineris_base::private::is_private(&path).unwrap());
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
        assert_eq!(history.get(0).map(Fragment::as_str), Some("kept"));
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
        history.push(&Fragment::text("before"));
        // (The writer is a thread of its own, which this waits for.)
        while !std::fs::read_to_string(&path).is_ok_and(|text| text.contains("before")) {
            std::thread::yield_now();
        }
        compact(&path, Limits::PROGRAM).expect("the compaction");
        history.push(&Fragment::text("after"));
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
        for n in 0..=SMALL.compact {
            let project = if n % 2 == 0 { "a" } else { "b" };
            text.push_str(&format!(
                "{{\"project\":\"{project}\",\"text\":\"prompt {n}\"}}\n"
            ));
        }
        std::fs::write(&path, text).unwrap();
        let history = FileHistory::open_with(path.clone(), "a".to_owned(), SMALL);
        wait_read(&history);
        assert_eq!(history.len(), SMALL.keep);
        assert_eq!(
            history.get(SMALL.keep - 1).map(Fragment::as_str),
            Some(format!("prompt {}", SMALL.compact).as_str())
        );
        drop(history);
        let lines = std::fs::read_to_string(&path).unwrap().lines().count();
        assert_eq!(
            lines,
            2 * SMALL.keep,
            "each project's newest {}",
            SMALL.keep
        );
        // The file written again is private, as the one it replaced was
        // made (the review of step 23).
        assert!(cineris_base::private::is_private(&path).unwrap());
    }

    /// A byte that is not UTF-8 costs its own line, not the file: the
    /// other lines are read (the review of step 23).
    #[test]
    fn a_line_that_is_not_utf_8_costs_only_itself() {
        let scratch = Scratch::new("not_utf8");
        let path = scratch.join("history");
        let mut bytes = b"{\"project\":\"p\",\"text\":\"before\"}\n".to_vec();
        bytes.extend_from_slice(b"{\"project\":\"p\",\"text\":\"bad \xff\"}\r\n");
        bytes.extend_from_slice(b"{\"project\":\"p\",\"text\":\"after\"}\r\n");
        std::fs::write(&path, &bytes).unwrap();
        let history = FileHistory::open(path.clone(), "p".to_owned());
        wait_read(&history);
        assert_eq!(history.len(), 2);
        assert_eq!(history.get(0).map(Fragment::as_str), Some("before"));
        assert_eq!(history.get(1).map(Fragment::as_str), Some("after"));
    }

    /// A last line cut short is ended before the next prompt is written:
    /// the prompt is kept, not glued to it (the review of step 23).
    #[test]
    fn a_prompt_after_a_cut_line_is_kept() {
        let scratch = Scratch::new("cut_line");
        let path = scratch.join("history");
        std::fs::write(
            &path,
            "{\"project\":\"p\",\"text\":\"whole\"}\n{\"project\":\"p\",\"te",
        )
        .unwrap();
        let mut history = FileHistory::open(path.clone(), "p".to_owned());
        wait_read(&history);
        history.push(&Fragment::text("next"));
        drop(history);
        let again = FileHistory::open(path.clone(), "p".to_owned());
        wait_read(&again);
        assert_eq!(again.len(), 2);
        assert_eq!(again.get(0).map(Fragment::as_str), Some("whole"));
        assert_eq!(again.get(1).map(Fragment::as_str), Some("next"));
    }

    /// A prompt with a large paste is kept as its chip, cut to the cap, and
    /// comes back a chip at the next start (the follow-ups of step 23: it
    /// was stored expanded, whole, and recalled as text).
    #[test]
    fn a_pasted_chip_is_kept_cut_and_comes_back_a_chip() {
        let scratch = Scratch::new("chip");
        let path = scratch.join("history");
        let paste = "log line\n".repeat(100_000);
        let prompt = Fragment::with_chips(
            "why does \u{FFFC} fail?",
            vec![Chip::new(paste.as_str(), "[pasted 100,000 lines]")],
        )
        .unwrap();
        let mut history = FileHistory::open(path.clone(), "p".to_owned());
        wait_read(&history);
        history.push(&prompt);
        drop(history);
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(
            size < 80 * 1024,
            "the line holds the chip cut to 64 KiB, not 900 KB: {size}"
        );
        let again = FileHistory::open(path.clone(), "p".to_owned());
        wait_read(&again);
        let back = again.get(0).unwrap();
        assert_eq!(back.as_str(), "why does \u{FFFC} fail?");
        assert_eq!(back.chips().len(), 1);
        assert_eq!(
            back.chips()[0].label(),
            "[pasted 100,000 lines, first 64 KiB kept]"
        );
        assert!(paste.starts_with(back.chips()[0].content()));
        // A line whose marks and chips do not pair is passed over.
        std::fs::write(
            &path,
            "{\"project\":\"p\",\"text\":\"a \u{FFFC}\",\"chips\":[]}\n\
             {\"project\":\"p\",\"text\":\"b\",\"chips\":[{\"label\":\"l\",\"text\":\"t\"}]}\n\
             {\"project\":\"p\",\"text\":\"\u{FFFC}\",\"chips\":[{\"label\":\"l\",\"text\":\"t\"}]}\n",
        )
        .unwrap();
        let read = FileHistory::open(path, "p".to_owned());
        wait_read(&read);
        assert_eq!(read.len(), 1);
        assert_eq!(read.get(0).unwrap().expanded(), "t");
    }

    /// A file of one project alone, past its bytes, is written again with
    /// that project's newest within [`KEEP_BYTES`], though the file could
    /// hold more: the bound on each project in a file written again, which
    /// a file of three projects in turn never reaches, the file's own
    /// bound coming first (the review of the CI-budgets fix).
    #[test]
    fn a_file_of_one_project_is_written_again_within_its_bytes() {
        let scratch = Scratch::new("one_project");
        let path = scratch.join("history");
        let filler = "w".repeat(60 * 1024);
        let mut text = String::new();
        let mut n = 0;
        while text.len() as u64 <= SMALL.compact_bytes {
            text.push_str(&format!(
                "{{\"project\":\"a\",\"text\":\"{n} {filler}\"}}\n"
            ));
            n += 1;
        }
        std::fs::write(&path, &text).unwrap();
        let history = FileHistory::open_with(path.clone(), "a".to_owned(), SMALL);
        wait_read(&history);
        drop(history);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.len() <= SMALL.keep_bytes, "{}", written.len());
        // Past one more line it would be over: the bound is the project's,
        // not the file's.
        assert!(
            written.len() + 62 * 1024 > SMALL.keep_bytes,
            "{}",
            written.len()
        );
        const { assert!(SMALL.keep_bytes + 62 * 1024 < SMALL.file_keep_bytes) };
        assert!(
            written.contains(&format!("\"{} ", n - 1)),
            "the newest line"
        );
    }

    /// A file past [`COMPACT_BYTES`] is written again within
    /// [`FILE_KEEP_BYTES`], each project within [`KEEP_BYTES`], newest
    /// kept; and what one project holds in memory is within
    /// [`KEEP_BYTES`].
    #[test]
    fn a_file_past_its_bytes_is_written_again_within_them() {
        let scratch = Scratch::new("bytes");
        let path = scratch.join("history");
        let filler = "w".repeat(60 * 1024);
        let mut text = String::new();
        let mut n = 0;
        while text.len() as u64 <= SMALL.compact_bytes {
            let project = ["a", "b", "c"][n % 3];
            text.push_str(&format!(
                "{{\"project\":\"{project}\",\"text\":\"{n} {filler}\"}}\n"
            ));
            n += 1;
        }
        std::fs::write(&path, &text).unwrap();
        let history = FileHistory::open_with(path.clone(), "a".to_owned(), SMALL);
        wait_read(&history);
        let held: usize = (0..history.len())
            .map(|index| history.get(index).unwrap().as_str().len())
            .sum();
        assert!(held <= SMALL.keep_bytes, "{held}");
        let newest_a = (0..n).rev().find(|n| n % 3 == 0).unwrap();
        assert!(
            history
                .get(history.len() - 1)
                .unwrap()
                .as_str()
                .starts_with(&format!("{newest_a} ")),
            "the newest is kept"
        );
        drop(history);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.len() <= SMALL.file_keep_bytes, "{}", written.len());
        assert!(
            written.len() + 3 * 70 * 1024 > SMALL.file_keep_bytes,
            "filled"
        );
        for project in ["a", "b", "c"] {
            let bytes: usize = written
                .lines()
                .filter(|line| line.contains(&format!("\"project\":\"{project}\"")))
                .map(|line| line.len() + 1)
                .sum();
            assert!(bytes <= SMALL.keep_bytes, "{project}: {bytes}");
        }
        assert!(
            written.contains(&format!("\"{} ", n - 1)),
            "the newest line"
        );
        assert!(cineris_base::private::is_private(&path).unwrap());
    }

    /// An older build's line of a pasted log, megabytes long, counts as the
    /// chip it is kept as: it neither hides a project's other prompts at a
    /// start nor empties the file when it is written again (the review of
    /// the follow-ups of step 23: the bounds counted the line as written).
    #[test]
    fn an_older_builds_long_line_counts_as_what_is_kept() {
        let scratch = Scratch::new("old_long_line");
        let path = scratch.join("history");
        // Past what a project keeps, as the megabytes of an older build's were.
        let log = "a line of a log\n".repeat(40_000);
        assert!(log.len() > SMALL.keep_bytes);
        let mut text = String::from("{\"project\":\"p\",\"text\":\"first\"}\n");
        text.push_str(&serde_json::json!({"project": "p", "text": log}).to_string());
        text.push('\n');
        text.push_str("{\"project\":\"q\",\"text\":\"other\"}\n");
        // Past the file's bytes, so that it is written again.
        let filler = "f".repeat(usize::try_from(SMALL.compact_bytes).unwrap());
        text.push_str(&format!("{{\"project\":\"r\",\"text\":\"{filler}\"}}\n"));
        assert!(text.len() as u64 > SMALL.compact_bytes);
        std::fs::write(&path, &text).unwrap();
        let history = FileHistory::open_with(path.clone(), "p".to_owned(), SMALL);
        wait_read(&history);
        assert_eq!(history.len(), 2, "the long line hid the prompt before it");
        assert_eq!(history.get(0).map(Fragment::as_str), Some("first"));
        assert!(history.get(1).unwrap().as_str().len() <= cineris_tui::history::TEXT_KEEP + 128);
        drop(history);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"first\""), "{}", written.len());
        assert!(written.contains("\"other\""), "{}", written.len());
    }

    /// What one project holds stays within [`KEEP_BYTES`] with the prompts
    /// sent in a run, the newest kept (the review of the follow-ups of
    /// step 23: those sent were held without a bound).
    #[test]
    fn the_prompts_sent_in_a_run_are_held_within_the_bounds() {
        let scratch = Scratch::new("sent_bounds");
        let path = scratch.join("history");
        let mut history = FileHistory::open_with(path, "p".to_owned(), SMALL);
        wait_read(&history);
        let paste = "x".repeat(cineris_tui::history::CHIP_KEEP);
        for n in 0..12 {
            let chips = (0..4).map(|_| Chip::new(paste.as_str(), "[p]")).collect();
            history.push(
                &Fragment::with_chips(format!("{n} \u{FFFC}\u{FFFC}\u{FFFC}\u{FFFC}"), chips)
                    .unwrap(),
            );
        }
        let held: usize = (0..history.len())
            .map(|index| held_bytes(history.get(index).unwrap()))
            .sum();
        assert!(held <= SMALL.keep_bytes, "{held}");
        assert!(history.len() < 12);
        assert!(
            history
                .get(history.len() - 1)
                .unwrap()
                .as_str()
                .starts_with("11 ")
        );
        for n in 0..SMALL.keep + 5 {
            history.push(&Fragment::text(format!("short {n}")));
        }
        assert_eq!(history.len(), SMALL.keep);
    }

    /// A file past [`READ_LIMIT`] (an older build's, with pastes kept
    /// whole) is read from its end alone, a line cut at the start passed
    /// over, and written again within the bounds.
    #[test]
    fn a_file_past_the_read_limit_is_read_from_its_end() {
        let scratch = Scratch::new("read_limit");
        let path = scratch.join("history");
        let mut text = format!(
            "{{\"project\":\"p\",\"text\":\"old {}\"}}\n",
            "o".repeat(usize::try_from(SMALL.read_limit).unwrap())
        );
        text.push_str("{\"project\":\"p\",\"text\":\"newest\"}\n");
        std::fs::write(&path, &text).unwrap();
        let history = FileHistory::open_with(path.clone(), "p".to_owned(), SMALL);
        wait_read(&history);
        assert_eq!(history.len(), 1);
        assert_eq!(history.get(0).map(Fragment::as_str), Some("newest"));
        drop(history);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"project\":\"p\",\"text\":\"newest\"}\n"
        );
    }

    /// A history of `bytes`, of twenty projects in turn, each prompt with a
    /// paste kept to its 64 KiB chip one time in eight and a line of text
    /// otherwise: written as this build writes them.
    fn history_of(path: &Path, bytes: u64) -> u64 {
        use std::fmt::Write as _;
        let paste = "a log line of a build that failed, say\n".repeat(2_000);
        let mut text = String::new();
        let mut n = 0_usize;
        loop {
            let project = format!("{:032x}", n % 20);
            let prompt = if n.is_multiple_of(8) {
                Fragment::with_chips(
                    format!("why does this fail, {n}? \u{FFFC}"),
                    vec![Chip::new(paste.as_str(), "[pasted 2,000 lines]")],
                )
                .unwrap()
            } else {
                Fragment::text(format!(
                    "fix the parser's error on line {n}, and run the tests"
                ))
            };
            let line = Entry::line(&project, &cineris_tui::history::kept(&prompt));
            if (text.len() + line.len()) as u64 > bytes {
                break;
            }
            let _ = write!(text, "{line}");
            n += 1;
        }
        std::fs::write(path, &text).unwrap();
        text.len() as u64
    }

    /// The read at a start, of a file at its caps and of an older build's
    /// past them (the follow-ups of step 23's measurement): its time, and
    /// what one project holds after it. Run by hand, in a release build:
    /// `cargo test --release -p cineris-cli --bin cineris perf_history -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    fn perf_history_read_at_its_caps() {
        let scratch = Scratch::new("perf");
        for (name, bytes) in [
            ("at_compact_bytes", COMPACT_BYTES),
            ("past_the_read_limit", READ_LIMIT * 4),
        ] {
            let path = scratch.join(name);
            let mut times = Vec::new();
            let mut held = 0;
            for _ in 0..15 {
                let size = history_of(&path, bytes);
                let started = std::time::Instant::now();
                // The first project, a paste in every other prompt of its.
                let mine = read(&path, &format!("{:032x}", 0), Limits::PROGRAM);
                times.push(started.elapsed().as_secs_f64() * 1000.0);
                held = mine
                    .iter()
                    .map(|prompt| {
                        prompt.as_str().len()
                            + prompt
                                .chips()
                                .iter()
                                .map(|chip| chip.content().len() + chip.label().len())
                                .sum::<usize>()
                    })
                    .sum::<usize>();
                let _ = size;
            }
            times.sort_by(f64::total_cmp);
            cineris_testkit::numbers::report(
                &format!("history.read.{name}.median"),
                times[times.len() / 2],
                "ms",
            );
            cineris_testkit::numbers::report(
                &format!("history.read.{name}.slowest"),
                times[times.len() - 1],
                "ms",
            );
            cineris_testkit::numbers::report(
                &format!("history.read.{name}.held"),
                held as f64 / 1024.0,
                "KiB",
            );
            cineris_testkit::numbers::report(
                &format!("history.read.{name}.after"),
                std::fs::metadata(&path).unwrap().len() as f64 / 1024.0,
                "KiB",
            );
        }
    }
}
