//! `Ctrl+G`: the prompt in the user's own editor, and back
//! (`docs/architecture/09-tui.md`, "External editor").
//!
//! 1. **Which editor.** `$VISUAL`, then `$EDITOR`, then a default for the
//!    system: `vi`, then `nano`, on Linux; Notepad on Windows, by its
//!    absolute path in the system folder. A value is the whole of a program
//!    when it names one as it is (`C:\Program Files\Notepad++\notepad++.exe`,
//!    spaces and all); otherwise it is a program and its arguments, split
//!    without a shell ([`split`]): `code --wait`, `vim -c 'set tw=72'`.
//!    The program is found by [`cortex_base::program`] in the absolute
//!    folders of `PATH`, never by a bare name and never in a relative
//!    folder (which would be the workspace). A relative path such as
//!    `./ed` is refused for the same reason. On Windows a name with no
//!    extension is looked for as `.exe`, `.com`, `.cmd` and `.bat` (VS
//!    Code's `code` is `code.cmd`), never as the file with no extension.
//! 2. **The file.** The prompt goes into `prompt.md` in a new folder of the
//!    system's temporary folder that only the user can open
//!    ([`cortex_base::private`]); the file is created private as well. An
//!    editor that saves by writing a new file and renaming it leaves that
//!    file in the private folder. Both are removed afterwards, whatever
//!    happened. A chip is written as its label (`[pasted 148 lines]`, with
//!    `#2` added when two have the same) and becomes the chip again when
//!    the label is still there, whole, on reading back.
//! 3. **The terminal.** It is handed over ([`Handoff::release`]: the live
//!    region erased, raw mode, the keyboard protocols, bracketed paste and
//!    focus reports off, and on Windows the console's own modes and code
//!    pages back), the editor runs with the terminal as its standard input
//!    and output, and the terminal is taken back ([`Handoff::reclaim`]) in
//!    every case: when the editor could not start, failed or was killed too.
//! 4. **What comes back.** The file's text, with `\r\n` as `\n` and the
//!    one line break an editor adds at the end taken off. When the editor
//!    could not start, failed (a non-zero exit, as `:cq` in vim), was
//!    killed, or left a file that is not UTF-8, the prompt stays as it was
//!    and the error says why. A file left empty, or only white space, also
//!    leaves the prompt as it was: emptying the prompt is `Ctrl+C`, and
//!    text is not lost to an editor that wrote nothing.
//!
//! The interface's thread waits for the editor: it has handed the terminal
//! over, so there is nothing for it to draw meanwhile.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the terminal is handed to the editor and the interface waits for it: blocking is \
              the point"
)]

use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use cortex_base::program;
use cortex_base::rng::{Rng, SystemRng};

use crate::editor::{Chip, Fragment};

/// What decides which editor runs: the variables and where to look.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `$VISUAL`.
    pub visual: Option<OsString>,
    /// `$EDITOR`.
    pub editor: Option<OsString>,
    /// The folders of `PATH`, in order.
    pub path: Vec<PathBuf>,
    /// The system folder on Windows (`%SystemRoot%`).
    pub system_root: Option<PathBuf>,
    /// Whether to go by Windows's rules.
    pub windows: bool,
}

impl Env {
    /// This process's environment.
    #[must_use]
    pub fn current() -> Self {
        Self {
            visual: std::env::var_os("VISUAL"),
            editor: std::env::var_os("EDITOR"),
            path: program::path_directories(),
            system_root: std::env::var_os("SystemRoot").map(PathBuf::from),
            windows: cfg!(windows),
        }
    }
}

/// An editor to run: the program, by its absolute path, and the arguments
/// before the file's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The program.
    pub path: PathBuf,
    /// Its arguments, before the file's path.
    pub args: Vec<String>,
}

/// Why the prompt was not edited.
#[derive(Debug)]
#[non_exhaustive]
pub enum EditorError {
    /// No editor was found: the value looked for, or the defaults.
    NotFound(String),
    /// The value names a program by a relative path, which would be found
    /// in the workspace.
    Relative(String),
    /// The value cannot be split into words: a quote is not closed.
    Unsplittable(String),
    /// The private file could not be made, written or read.
    File(io::Error),
    /// The editor could not be started.
    Start(PathBuf, io::Error),
    /// The editor ended with this exit code (`None`: no code).
    Failed(Option<i32>),
    /// The editor was killed by this signal.
    Killed(i32),
    /// The file the editor left is not UTF-8.
    NotText,
    /// The terminal could not be taken back. The caller should end.
    Terminal(io::Error),
}

impl fmt::Display for EditorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(what) => write!(f, "no editor found: {what}"),
            Self::Relative(what) => write!(
                f,
                "the editor `{what}` is a relative path; give an absolute path or a name on PATH"
            ),
            Self::Unsplittable(what) => {
                write!(f, "the editor `{what}` has a quote that is not closed")
            }
            Self::File(error) => write!(f, "the prompt's file: {error}"),
            Self::Start(path, error) => write!(f, "{} could not start: {error}", path.display()),
            Self::Failed(Some(code)) => write!(f, "the editor ended with exit code {code}"),
            Self::Failed(None) => write!(f, "the editor failed"),
            Self::Killed(signal) => write!(f, "the editor was killed (signal {signal})"),
            Self::NotText => write!(f, "the editor left text that is not UTF-8"),
            Self::Terminal(error) => write!(f, "the terminal could not be taken back: {error}"),
        }
    }
}

impl std::error::Error for EditorError {}

/// What the editor did to the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edited {
    /// It changed it to this.
    Changed(Fragment),
    /// It left it as it was.
    Unchanged,
    /// It left the file empty: the prompt is kept as it was.
    Empty,
}

/// The terminal, handed over to the editor and taken back.
pub trait Handoff {
    /// Puts the terminal back as the user's shell had it: the live region
    /// erased, every mode the interface turned on turned off.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be written to.
    fn release(&mut self) -> io::Result<()>;

    /// Takes the terminal back: raw mode and the modes on again, and the
    /// live region drawn afresh where the cursor is.
    ///
    /// # Errors
    ///
    /// When the terminal cannot be taken back.
    fn reclaim(&mut self) -> io::Result<()>;
}

/// The editor to run, from `env`.
///
/// # Errors
///
/// [`EditorError::NotFound`], [`EditorError::Relative`] or
/// [`EditorError::Unsplittable`].
pub fn find(env: &Env) -> Result<Program, EditorError> {
    for value in [&env.visual, &env.editor].into_iter().flatten() {
        let value = value.to_string_lossy();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        return from_value(value, env);
    }
    default(env)
}

fn from_value(value: &str, env: &Env) -> Result<Program, EditorError> {
    // The whole value, if it names a program as it is.
    if let Some(path) = locate(value, env)? {
        return Ok(Program {
            path,
            args: Vec::new(),
        });
    }
    let mut words = split(value, env.windows)
        .ok_or_else(|| EditorError::Unsplittable(value.to_owned()))?
        .into_iter();
    let Some(name) = words.next() else {
        return Err(EditorError::NotFound(value.to_owned()));
    };
    match locate(&name, env)? {
        Some(path) => Ok(Program {
            path,
            args: words.collect(),
        }),
        None => Err(EditorError::NotFound(value.to_owned())),
    }
}

/// Where the program `name` is: an absolute path that is a program, or a
/// name found in the absolute folders of `PATH`. A relative path is
/// refused.
fn locate(name: &str, env: &Env) -> Result<Option<PathBuf>, EditorError> {
    let path = Path::new(name);
    let absolute = path.is_absolute() || (env.windows && is_windows_absolute(name));
    if absolute {
        return Ok(program::is_program(path, env.windows).then(|| path.to_path_buf()));
    }
    let has_separator = name.contains('/') || (env.windows && name.contains('\\'));
    if has_separator {
        return Err(EditorError::Relative(name.to_owned()));
    }
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Ok(None);
    }
    if env.windows && Path::new(name).extension().is_none() {
        for extension in ["exe", "com", "cmd", "bat"] {
            let found = program::find(&env.path, &format!("{name}.{extension}"), true);
            if found.is_some() {
                return Ok(found);
            }
        }
        return Ok(None);
    }
    Ok(program::find(&env.path, name, env.windows))
}

/// `C:\…` or `\\server\…`: absolute by Windows's rules, whatever system
/// this is.
fn is_windows_absolute(name: &str) -> bool {
    let bytes = name.as_bytes();
    (bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/'))
        || name.starts_with("\\\\")
}

fn default(env: &Env) -> Result<Program, EditorError> {
    if env.windows {
        let root = env
            .system_root
            .clone()
            .filter(|root| root.is_absolute() || is_windows_absolute(&root.to_string_lossy()))
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let notepad = root.join("System32").join("notepad.exe");
        if program::is_program(&notepad, true) {
            return Ok(Program {
                path: notepad,
                args: Vec::new(),
            });
        }
        return Err(EditorError::NotFound(format!(
            "VISUAL and EDITOR are not set, and there is no {}",
            notepad.display()
        )));
    }
    for name in ["vi", "nano"] {
        if let Some(path) = program::find(&env.path, name, false) {
            return Ok(Program {
                path,
                args: Vec::new(),
            });
        }
    }
    Err(EditorError::NotFound(
        "VISUAL and EDITOR are not set, and neither vi nor nano is on PATH".to_owned(),
    ))
}

/// The words of an editor's value, as the system's own rules split a
/// command line, without running a shell. `None` when a quote is not
/// closed.
///
/// - **Linux**, as a POSIX shell splits words: white space separates;
///   `'…'` keeps everything in it; `"…"` keeps everything but a backslash
///   before `"`, `\`, `$` or `` ` ``; a backslash outside quotes keeps the
///   next character. Nothing is expanded.
/// - **Windows**, as programs split their command line: white space
///   separates; `"…"` groups; backslashes are literal except before a `"`,
///   where each two make one and an odd one makes the `"` literal; `''` is
///   not a quote.
#[must_use]
pub fn split(value: &str, windows: bool) -> Option<Vec<String>> {
    if windows {
        split_windows(value)
    } else {
        split_posix(value)
    }
}

fn split_posix(value: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            c @ ('"' | '\\' | '$' | '`') => word.push(c),
                            '\n' => {}
                            c => {
                                word.push('\\');
                                word.push(c);
                            }
                        },
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some('\n') => {}
                    Some(c) => word.push(c),
                    None => word.push('\\'),
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

fn split_windows(value: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut quoted = false;
    let chars: Vec<char> = value.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        match c {
            '\\' => {
                let run = chars[at..].iter().take_while(|c| **c == '\\').count();
                in_word = true;
                if chars.get(at + run) == Some(&'"') {
                    word.extend(std::iter::repeat_n('\\', run / 2));
                    if run % 2 == 1 {
                        word.push('"');
                        at += run + 1;
                    } else {
                        at += run;
                    }
                } else {
                    word.extend(std::iter::repeat_n('\\', run));
                    at += run;
                }
                continue;
            }
            '"' => {
                in_word = true;
                // `""` inside quotes is one literal quote.
                if quoted && chars.get(at + 1) == Some(&'"') {
                    word.push('"');
                    at += 2;
                    continue;
                }
                quoted = !quoted;
            }
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
        at += 1;
    }
    if quoted {
        return None;
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// The prompt as the file shows it, and the chips each label stands for.
fn to_file(fragment: &Fragment) -> (String, Vec<(String, Chip)>) {
    let mut out = String::with_capacity(fragment.text.len());
    let mut labels: Vec<(String, Chip)> = Vec::new();
    let mut chips = fragment.chips.iter();
    for piece in fragment.text.split_inclusive(crate::editor::cluster::CHIP) {
        let Some(before) = piece.strip_suffix(crate::editor::cluster::CHIP) else {
            out.push_str(piece);
            continue;
        };
        out.push_str(before);
        let Some(chip) = chips.next() else {
            continue;
        };
        if chip.is_literal() {
            out.push_str(chip.content());
            continue;
        }
        let mut label = chip.label().to_owned();
        let mut n = 1;
        while labels.iter().any(|(taken, _)| *taken == label)
            || fragment_has(&fragment.text, &label)
        {
            n += 1;
            label = match chip.label().strip_suffix(']') {
                Some(open) => format!("{open} #{n}]"),
                None => format!("{} #{n}", chip.label()),
            };
        }
        out.push_str(&label);
        labels.push((label, chip.clone()));
    }
    (out, labels)
}

/// Whether the prompt's own text already holds `label`, so that a chip's
/// label would not be told from it.
fn fragment_has(text: &str, label: &str) -> bool {
    text.contains(label)
}

/// The file's text as a fragment: each label still there becomes its chip
/// again.
fn from_file(text: &str, labels: &[(String, Chip)]) -> Fragment {
    let mut found: Vec<(usize, &str, &Chip)> = labels
        .iter()
        .filter_map(|(label, chip)| {
            text.find(label.as_str())
                .map(|at| (at, label.as_str(), chip))
        })
        .collect();
    found.sort_by_key(|(at, _, _)| *at);
    let mut fragment = Fragment::default();
    let mut from = 0;
    for (at, label, chip) in found {
        if at < from {
            continue;
        }
        fragment.push(Fragment::text(&text[from..at]));
        fragment.push(Fragment::chip(chip.clone()));
        from = at + label.len();
    }
    fragment.push(Fragment::text(&text[from..]));
    fragment
}

/// Edits `prompt` with `editor`, handing the terminal over through
/// `handoff`. The file goes in a new private folder under `temp`.
///
/// # Errors
///
/// See [`EditorError`]. The terminal has been taken back whatever the
/// error, except [`EditorError::Terminal`].
pub fn edit(
    prompt: &Fragment,
    editor: &Program,
    handoff: &mut dyn Handoff,
    temp: &Path,
) -> Result<Edited, EditorError> {
    let (text, labels) = to_file(prompt);
    let folder = Folder::new(temp).map_err(EditorError::File)?;
    let file = folder.path.join("prompt.md");
    write_private(&file, &text).map_err(EditorError::File)?;

    handoff.release().map_err(EditorError::Terminal)?;
    let status = Command::new(&editor.path)
        .args(&editor.args)
        .arg(&file)
        .status();
    // Taken back whatever the editor did.
    handoff.reclaim().map_err(EditorError::Terminal)?;

    let status = status.map_err(|error| EditorError::Start(editor.path.clone(), error))?;
    if !status.success() {
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            if let Some(signal) = status.signal() {
                return Err(EditorError::Killed(signal));
            }
        }
        return Err(EditorError::Failed(status.code()));
    }
    let read = read_back(&file)?;
    drop(folder);

    let mut read = crate::editor::normalize(&read).into_owned();
    if read.ends_with('\n') && !text.ends_with('\n') {
        read.pop();
    }
    if read.trim().is_empty() {
        return Ok(if text.trim().is_empty() {
            Edited::Unchanged
        } else {
            Edited::Empty
        });
    }
    if read == text {
        return Ok(Edited::Unchanged);
    }
    Ok(Edited::Changed(from_file(&read, &labels)))
}

fn write_private(file: &Path, text: &str) -> io::Result<()> {
    let mut handle = cortex_base::private::create_new(file)?;
    handle.write_all(text.as_bytes())?;
    handle.flush()
}

fn read_back(file: &Path) -> Result<String, EditorError> {
    let (mut handle, _) = cortex_base::open_regular(file)
        .map_err(|error| EditorError::File(io::Error::other(format!("{error}"))))?;
    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes).map_err(EditorError::File)?;
    String::from_utf8(bytes).map_err(|_| EditorError::NotText)
}

/// A new folder only the user can open, removed with what is in it when
/// dropped.
#[derive(Debug)]
struct Folder {
    path: PathBuf,
}

impl Folder {
    fn new(temp: &Path) -> io::Result<Self> {
        let mut rng = SystemRng::new();
        for _ in 0..8 {
            let path = temp.join(format!("cortex-prompt-{:016x}", rng.next_u64()));
            if path.symlink_metadata().is_ok() {
                continue;
            }
            cortex_base::private::create_dir_all(&path)?;
            let folder = Self { path };
            // Made by someone else between the look and the creation: not
            // ours, and not used.
            if !cortex_base::private::is_private(&folder.path)? {
                std::mem::forget(folder);
                return Err(io::Error::other("the prompt's folder is not private"));
            }
            return Ok(folder);
        }
        Err(io::Error::other("no free name for the prompt's folder"))
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_words_follow_the_shell_without_expanding() {
        assert_eq!(split("code --wait", false).unwrap(), ["code", "--wait"]);
        assert_eq!(
            split(r#"vim -c 'set tw=72' "a \"b\" $c""#, false).unwrap(),
            ["vim", "-c", "set tw=72", "a \"b\" $c"]
        );
        assert_eq!(split(r"my\ editor -x", false).unwrap(), ["my editor", "-x"]);
        assert_eq!(split("vim 'open", false), None);
        assert_eq!(split("  ", false).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn windows_words_follow_the_command_line_rules() {
        assert_eq!(
            split(r#""C:\Program Files\Code\code.cmd" --wait"#, true).unwrap(),
            [r"C:\Program Files\Code\code.cmd", "--wait"]
        );
        assert_eq!(split(r#"a\\\"b c\d"#, true).unwrap(), [r#"a\"b"#, r"c\d"]);
        assert_eq!(split(r#"a\\"b c" d"#, true).unwrap(), [r"a\b c", "d"]);
        assert_eq!(split("'a b'", true).unwrap(), ["'a", "b'"]);
        assert_eq!(split(r#""open"#, true), None);
    }

    #[test]
    fn a_relative_editor_is_refused_and_a_missing_one_is_not_found() {
        let env = Env {
            editor: Some("./ed".into()),
            ..Env::default()
        };
        assert!(matches!(find(&env), Err(EditorError::Relative(_))));
        let env = Env {
            editor: Some("no-such-editor-anywhere --x".into()),
            path: vec![std::env::temp_dir()],
            ..Env::default()
        };
        assert!(matches!(find(&env), Err(EditorError::NotFound(_))));
    }

    #[test]
    fn chips_go_out_as_labels_and_come_back_where_the_label_is_whole() {
        let mut prompt = Fragment::text("look at ");
        prompt.push(Fragment::chip(Chip::new("A\nB", "[pasted 2 lines]")));
        prompt.push(Fragment::text(" and "));
        prompt.push(Fragment::chip(Chip::new("C\nD", "[pasted 2 lines]")));
        prompt.push(Fragment::text(" \u{FFFC}"));
        let (text, labels) = to_file(&prompt);
        assert_eq!(
            text,
            "look at [pasted 2 lines] and [pasted 2 lines #2] \u{FFFC}"
        );
        let back = from_file(&text.replace("look at", "see"), &labels);
        assert_eq!(back.expanded(), "see A\nB and C\nD \u{FFFC}");
        assert_eq!(back.chips.len(), 3);
        // A label edited is text.
        let edited = from_file("[pasted 2 line] [pasted 2 lines #2]", &labels);
        assert_eq!(edited.expanded(), "[pasted 2 line] C\nD");
    }
}
