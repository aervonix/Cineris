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
use cortex_term::{Inline, Overrides, Sink, Terminal};

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
    /// Folders an editor found by its name is never taken from: the
    /// session's write roots, where a command the agent ran could have put
    /// a program (`cortex_base::program::find_outside`). An absolute path the
    /// user gave is taken wherever it is.
    pub avoid: Vec<PathBuf>,
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
            avoid: Vec::new(),
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
    /// The file the editor left is not UTF-8. It is kept, at this path,
    /// so that what was written in it is not lost.
    NotText(PathBuf),
    /// Handing the terminal over failed partway. It has been taken back,
    /// and the editor was not run.
    Release(io::Error),
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
            Self::NotText(path) => write!(
                f,
                "the editor left text that is not UTF-8; it is kept in {}",
                path.display()
            ),
            Self::Release(error) => write!(f, "the terminal could not be handed over: {error}"),
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

/// The [`Handoff`] of the interface's own terminal: the inline presenter
/// erases its live region and the terminal is left, then entered again, and
/// the presenter starts drawing afresh where the cursor is.
///
/// Leaving puts back the modes the terminal had before the interface first
/// entered it (on Windows the console's modes and code pages), and entering
/// again sets the interface's modes from those, so an editor that changed
/// the modes and did not put them back changes nothing for either.
#[derive(Debug)]
pub struct TerminalHandoff<'a> {
    /// The terminal: taken out while the editor runs, put back after.
    pub terminal: &'a mut Option<Terminal>,
    /// The inline presenter.
    pub inline: &'a mut Inline,
    /// The settings the terminal is entered with.
    pub overrides: Overrides,
}

impl Handoff for TerminalHandoff<'_> {
    fn release(&mut self) -> io::Result<()> {
        let Some(mut terminal) = self.terminal.take() else {
            return Ok(());
        };
        let caps = *terminal.caps();
        let finished = self.inline.finish(&caps, &mut terminal);
        // Left whatever the presenter managed.
        let left = terminal.leave();
        finished.and(left)
    }

    fn reclaim(&mut self) -> io::Result<()> {
        // A signal came while the editor had the terminal: the program
        // ends, and the terminal stays the shell's. The loop finds no
        // terminal, and ends as the signal asks (the review of step 23).
        if self.terminal.is_some() || cortex_term::signalled().is_some() {
            return Ok(());
        }
        let mut terminal = Terminal::enter(self.overrides)?;
        // What an editor that was killed may have left on: the alternate
        // screen, mouse reporting, application keys, its keyboard flags. The
        // interface's own keyboard flags are pushed after this, when the
        // terminal answers.
        terminal.write_frame(LEFT_ON)?;
        let (cols, rows) = terminal.size()?;
        self.inline.resize(cols, rows);
        self.inline.restart();
        *self.terminal = Some(terminal);
        Ok(())
    }
}

/// Turns off what a full-screen editor turns on: the alternate screen
/// (without moving the cursor, as 1049 would), mouse reporting, application
/// cursor keys and keypad, every level of the enhanced keyboard protocol's
/// flags, and a hidden cursor.
const LEFT_ON: &[u8] =
    b"\x1b[?1047l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1l\x1b>\x1b[<99u\x1b[?25h";

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
    // The whole value, if it is the absolute path of a program as it is
    // (spaces and all), or a name found on PATH.
    if let Some(path) = absolute_program(value, env) {
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
    if path.is_absolute() || (env.windows && is_windows_absolute(name)) {
        return Ok(absolute_program(name, env));
    }
    let has_separator = name.contains('/') || (env.windows && name.contains('\\'));
    if has_separator {
        return Err(EditorError::Relative(name.to_owned()));
    }
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Ok(None);
    }
    if env.windows && Path::new(name).extension().is_none() {
        // Folder by folder, as the command prompt looks: a wrapper early in
        // PATH wins over a program later in it.
        for folder in &env.path {
            for extension in ["exe", "com", "cmd", "bat"] {
                let found = program::find_outside(
                    std::slice::from_ref(folder),
                    &format!("{name}.{extension}"),
                    true,
                    &env.avoid,
                );
                if found.is_some() {
                    return Ok(found);
                }
            }
        }
        return Ok(None);
    }
    Ok(program::find_outside(
        &env.path,
        name,
        env.windows,
        &env.avoid,
    ))
}

/// `name` as an absolute path to a program, if it is one. On Windows a path
/// with no extension is the one with `.exe`, which is what starting it
/// would run.
fn absolute_program(name: &str, env: &Env) -> Option<PathBuf> {
    let path = Path::new(name);
    if !(path.is_absolute() || (env.windows && is_windows_absolute(name))) {
        return None;
    }
    if env.windows && path.extension().is_none() {
        // What starting it would run: `.exe`; and a wrapper beside it, as
        // VS Code's `bin\code` is `code.cmd`.
        return ["exe", "com", "cmd", "bat"]
            .into_iter()
            .map(|extension| path.with_extension(extension))
            .find(|candidate| program::is_program(candidate, true));
    }
    program::is_program(path, env.windows).then(|| path.to_path_buf())
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
        if let Some(path) = program::find_outside(&env.path, name, false, &env.avoid) {
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
/// - **Linux**, as a POSIX shell splits words: a space, a tab or a line
///   break separates;
///   `'…'` keeps everything in it; `"…"` keeps everything but a backslash
///   before `"`, `\`, `$` or `` ` ``; a backslash outside quotes keeps the
///   next character. Nothing is expanded.
/// - **Windows**, as programs split their command line: the program is
///   taken as it is, up to its closing `"` when it starts with one or to a
///   space or a tab; after it a space or a tab separates, `"…"` groups,
///   backslashes are literal except before a `"`, where each two make one
///   and an odd one makes the `"` literal, and `'` is not a quote.
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
            ' ' | '\t' | '\n' => {
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
    let value = value.trim_start_matches([' ', '\t']);
    let (program, rest) = match value.strip_prefix('"') {
        Some(quoted) => match quoted.find('"') {
            Some(end) => (&quoted[..end], &quoted[end + 1..]),
            None => (quoted, ""),
        },
        None => {
            let end = value.find([' ', '\t']).unwrap_or(value.len());
            (&value[..end], &value[end..])
        }
    };
    let mut words = split_windows_arguments(rest)?;
    if !program.is_empty() {
        words.insert(0, program.to_owned());
    }
    Some(words)
}

fn split_windows_arguments(value: &str) -> Option<Vec<String>> {
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
            ' ' | '\t' if !quoted => {
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
    if !temp.is_absolute() {
        return Err(EditorError::File(io::Error::other(
            "the temporary folder is not an absolute path",
        )));
    }
    let (text, labels) = to_file(prompt);
    let folder = Folder::new(temp).map_err(EditorError::File)?;
    let file = folder.path.join("prompt.md");
    write_private(&file, &text).map_err(EditorError::File)?;

    // A key that interrupts reaches the editor, and leaves the interface,
    // and the prompt, alone: from before the terminal is left to after it
    // is taken back, so that no key falls in between. The editor keeps its
    // own Ctrl+C (see the guard).
    let ignored = cortex_term::ignore_interrupts();
    let released = handoff.release();
    let status = released.is_ok().then(|| run(editor, &file, &folder.path));
    // Taken back whatever the editor did, and even when handing it over
    // failed partway: part of it may have been put back.
    let reclaimed = handoff.reclaim();
    drop(ignored);
    reclaimed.map_err(EditorError::Terminal)?;
    released.map_err(EditorError::Release)?;

    let status = status
        .unwrap_or_else(|| Err(io::Error::other("the terminal was not handed over")))
        .map_err(|error| EditorError::Start(editor.path.clone(), error))?;
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
    let Some(read) = read_back(&file).map_err(EditorError::File)? else {
        // Kept, with what was written in it.
        let kept = file.clone();
        std::mem::forget(folder);
        return Err(EditorError::NotText(kept));
    };
    drop(folder);

    let read = match read.strip_prefix('\u{FEFF}') {
        Some(rest) if !text.starts_with('\u{FEFF}') => rest,
        _ => &read,
    };
    let mut read = crate::editor::normalize(read).into_owned();
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

/// Runs the editor on `file`, in `folder`, and waits for it. Its working
/// folder is the private one, not the workspace: an editor that reads a
/// configuration from where it starts (or, on Windows, a `.cmd` wrapper
/// whose command prompt looks for programs there) finds nothing planted.
fn run(editor: &Program, file: &Path, folder: &Path) -> io::Result<std::process::ExitStatus> {
    let mut command = Command::new(&editor.path);
    command.args(&editor.args).arg(file).current_dir(folder);
    if cfg!(windows) {
        command.env("NoDefaultCurrentDirectoryInExePath", "1");
    }
    cortex_base::hook::started(command.get_program());
    command.spawn().and_then(|mut child| child.wait())
}

/// The file's text, or `None` when it is not UTF-8.
fn read_back(file: &Path) -> io::Result<Option<String>> {
    let (mut handle, _) =
        cortex_base::open_regular(file).map_err(|error| io::Error::other(format!("{error}")))?;
    let mut bytes = Vec::new();
    handle.read_to_end(&mut bytes)?;
    Ok(String::from_utf8(bytes).ok())
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
        assert_eq!(split("ed a\u{3000}b", false).unwrap(), ["ed", "a\u{3000}b"]);
    }

    #[test]
    fn windows_words_follow_the_command_line_rules() {
        assert_eq!(
            split(r#""C:\Program Files\Code\code.cmd" --wait"#, true).unwrap(),
            [r"C:\Program Files\Code\code.cmd", "--wait"]
        );
        assert_eq!(
            split(r#"ed a\\\"b c\d"#, true).unwrap(),
            ["ed", r#"a\"b"#, r"c\d"]
        );
        assert_eq!(
            split(r#"ed a\\"b c" d"#, true).unwrap(),
            ["ed", r"a\b c", "d"]
        );
        assert_eq!(split("ed 'a b'", true).unwrap(), ["ed", "'a", "b'"]);
        assert_eq!(split(r#""C:\x\" -n"#, true).unwrap(), [r"C:\x\", "-n"]);
        assert_eq!(split(r#"ed "open"#, true), None);
        assert_eq!(split("ed a\u{A0}b", true).unwrap(), ["ed", "a\u{A0}b"]);
    }

    #[test]
    fn an_absolute_program_is_taken_whole_or_with_its_arguments() {
        let exe = std::env::current_exe().unwrap();
        let env = |value: String| Env {
            visual: Some(value.into()),
            windows: cfg!(windows),
            ..Env::default()
        };
        let whole = find(&env(exe.display().to_string())).unwrap();
        assert_eq!((whole.path.as_path(), whole.args.len()), (exe.as_path(), 0));
        let split = find(&env(format!("\"{}\" --wait -n", exe.display()))).unwrap();
        assert_eq!(split.path, exe);
        assert_eq!(split.args, ["--wait", "-n"]);
    }

    /// A folder of the test's own with these files in it, each marked as a
    /// program on Linux.
    fn folder(test: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cortex-tui-external-{test}-{:016x}",
            SystemRng::new().next_u64()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for name in files {
            let path = dir.join(name);
            std::fs::write(&path, b"").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        dir
    }

    #[test]
    fn on_windows_a_name_is_looked_for_folder_by_folder_with_its_extensions_only() {
        let first = folder("first", &["ed", "ed.bat"]);
        let second = folder("second", &["ed.exe"]);
        let env = Env {
            editor: Some("ed --wait".into()),
            path: vec![PathBuf::from("relative"), first.clone(), second.clone()],
            windows: true,
            ..Env::default()
        };
        let found = find(&env).unwrap();
        assert_eq!(
            found.path,
            first.join("ed.bat"),
            "the wrapper earlier in PATH"
        );
        assert_eq!(found.args, ["--wait"]);
        // The file with no extension is never the program.
        let only_bare = folder("bare", &["vim"]);
        let env = Env {
            editor: Some("vim".into()),
            path: vec![only_bare.clone()],
            windows: true,
            ..Env::default()
        };
        assert!(matches!(find(&env), Err(EditorError::NotFound(_))));
        // An absolute path with no extension is the one with `.exe`.
        let env = Env {
            editor: Some(second.join("ed").display().to_string().into()),
            windows: true,
            ..Env::default()
        };
        assert_eq!(find(&env).unwrap().path, second.join("ed.exe"));
        for dir in [first, second, only_bare] {
            std::fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn an_editor_in_a_write_root_is_passed_over() {
        let planted = folder("planted", &["vim", "vi"]);
        let real = folder("real", &["vim"]);
        let env = Env {
            editor: Some("vim -n".into()),
            path: vec![planted.clone(), real.clone()],
            avoid: vec![planted.clone()],
            ..Env::default()
        };
        assert_eq!(find(&env).unwrap().path, real.join("vim"));
        // The default too: `vi` only in the write root is no `vi`.
        let env = Env {
            path: vec![planted.clone()],
            avoid: vec![planted.clone()],
            ..Env::default()
        };
        assert!(matches!(find(&env), Err(EditorError::NotFound(_))));
        let windows = Env {
            editor: Some("vim".into()),
            path: vec![planted.clone()],
            avoid: vec![planted.clone()],
            windows: true,
            ..Env::default()
        };
        std::fs::write(planted.join("vim.exe"), b"").unwrap();
        assert!(matches!(find(&windows), Err(EditorError::NotFound(_))));
        std::fs::remove_dir_all(planted).unwrap();
        std::fs::remove_dir_all(real).unwrap();
    }

    #[test]
    fn the_defaults_are_vi_then_nano_and_notepad_in_the_system_folder() {
        let tools = folder("defaults", &["nano"]);
        let env = Env {
            path: vec![tools.clone()],
            ..Env::default()
        };
        assert_eq!(find(&env).unwrap().path, tools.join("nano"));
        let root = folder("root", &[]);
        std::fs::create_dir_all(root.join("System32")).unwrap();
        std::fs::write(root.join("System32").join("notepad.exe"), b"").unwrap();
        let env = Env {
            system_root: Some(root.clone()),
            path: vec![tools.clone()],
            windows: true,
            ..Env::default()
        };
        assert_eq!(
            find(&env).unwrap().path,
            root.join("System32").join("notepad.exe")
        );
        // An empty variable is passed over.
        let env = Env {
            visual: Some("  ".into()),
            path: vec![tools.clone()],
            ..Env::default()
        };
        assert_eq!(find(&env).unwrap().path, tools.join("nano"));
        std::fs::remove_dir_all(tools).unwrap();
        std::fs::remove_dir_all(root).unwrap();
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
