//! The harness's own commands that approve, trust or answer for the person,
//! refused to every command of a session at every level, `unrestricted`
//! included, with no rule able to lift them (since step 28's third pull
//! request, the owner's answer: where a command is not confined, the model
//! could otherwise approve an extension, or trust a project, for itself).
//!
//! **What is refused** is said by the front end's own grammar where it
//! gives one ([`OwnCommands`], the CLI's: its parser, with a choice made for
//! each subcommand, action and flag), and otherwise by the fallback here,
//! which reads the same command line the same way:
//!
//! - the subcommands that approve, trust, keep credentials or answer:
//!   `ext add|remove|disable`, `permissions trust|untrust|revoke|forget`,
//!   `project accept|add|edit|promote|disable|delete`, `auth login|logout`,
//!   `stats reset`, `setup`, `serve` and `attach`; and the internal
//!   `__sandbox` and `__tree-guard`;
//! - the application itself (no subcommand, not headless, not the help or
//!   the version), which answers a project's trust question;
//! - a headless run that trusts or widens: `--trust-project`,
//!   `--permission-mode`, `--allow`, `-c`, `--profile`, `--continue`,
//!   `--resume`, `--fork`; and any run of Cineris in a line that sets or
//!   unsets a variable that moves its stores (`HOME`, `USERPROFILE`,
//!   `APPDATA`, `LOCALAPPDATA`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME`,
//!   `XDG_DATA_HOME`, or any `CINERIS_…`).
//!
//! The help (`-h`, `--help` before `--`), the version, and the subcommands
//! that only read are not refused.
//!
//! **Where Cineris is looked for.** The line is read by the engine's own
//! analysis, segment by segment, and in each simple command:
//!
//! - at the program's place, past `NAME=value` words and `env` with its
//!   options and their values; by its names (`cineris`, `cin`, and the
//!   names the front end runs under), with any directory before it, in any
//!   case, with or without `.exe`, with Windows's trailing dots and blanks
//!   and its `::$DATA` taken off, as an 8.3 short name, or as a pattern
//!   that could match one of them (`cineri?`, `cin[e]ris`);
//! - at any word, where the program runs its arguments (`sudo`, `timeout`,
//!   `xargs`, `find`, `npx`, and the terminal runners: `script`, `tmux`,
//!   `screen`, `ssh`, `socat`, `Start-Process`…); a word of theirs with a
//!   blank in it is read as a line again;
//! - in a piece of a word split at `=`, `!`, `,` and `:` (`--bin=cin`,
//!   `alias.x=!cineris`, `EXEC:cineris,pty`), and in the text after an `=`
//!   or a `!`, read as a line;
//! - in the line a program is told to run: after `-c` and its clusters
//!   (`-lc`, `-qc`) for a shell, `script` or `su`; `env -S`, attached or
//!   not, and `--split-string`; `--command=`; `pwsh -c`, `powershell
//!   -Command` and PowerShell's other words; `cmd /c` and `/k`, the rest of
//!   the line as `cmd` reads it; and an interpreter's code (`python -c`,
//!   `perl -e`, `node -e`, an `awk` program, `expect -c`), read as text;
//! - with PowerShell's `Start-Process`, by its `-FilePath` or first word and
//!   its `-ArgumentList` or second.
//!
//! **A line the analysis cannot model fails closed.** Its quotes and
//! escapes are taken out as its shell takes them (`\` escapes in POSIX and
//! separates on Windows, a backtick in PowerShell, `^` in `cmd`), and then:
//! a name of Cineris anywhere is judged with the words after it, up to the
//! first that the shell expands (`$`, a backtick, `%…%`, `!…!`, `@name`);
//! where one does, it is refused unless what is written out already fixes a
//! subcommand that only reads. A line that names Cineris and holds an
//! approving pair anywhere is refused too.
//!
//! **What it does not reach:** a name the shell builds (`${X}neris`,
//! `$(printf ci)neris`), a script file, an encoded PowerShell command, a
//! copy under another name than the front end knows, `cargo run -p
//! cineris-cli`. The approving subcommands look at their own line of
//! parents, or the harness's job on Windows, and refuse under a session's
//! command: that is the backstop (`cineris_sandbox::session`).

use std::cell::{Cell, RefCell};
use std::sync::Arc;

pub use crate::command::ShellSyntax;
use crate::command::{Analysis, Part, analyse};
use crate::expansion::NamePattern;

mod cases;
pub use cases::CASES;

/// How a command line of the harness's own stands, as the front end reads
/// it: given the words after the program's name, and whether more words
/// may follow that are not known (an expansion the shell makes).
pub type Grammar = Arc<dyn Fn(&[String], bool) -> Standing + Send + Sync>;

/// How one run of Cineris stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Standing {
    /// It only reads, or does nothing a session may not.
    Safe,
    /// It is refused.
    Refused(Refusal),
}

/// What is refused, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// The command, in words: `cineris ext add`.
    pub what: String,
    /// Why.
    pub kind: Kind,
}

impl Refusal {
    /// A refusal of `what` for `kind`.
    #[must_use]
    pub fn new(what: impl Into<String>, kind: Kind) -> Self {
        Self {
            what: what.into(),
            kind,
        }
    }
}

/// Why a run of Cineris is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// It approves, trusts, keeps credentials or answers for the person.
    Approves,
    /// It is the application, which answers a project's trust question.
    Application,
    /// A run that trusts the project, widens what it may do, or reads its
    /// configuration and stores from elsewhere.
    Widens,
    /// What follows it is not known until the shell expands it.
    Hidden,
    /// One of the harness's internal subcommands.
    Internal,
}

/// The words every refusal begins with, which a front end may look for.
pub const REFUSED: &str = "Cineris refuses";

/// What a refused line is told.
#[must_use]
pub fn reason(refusal: &Refusal) -> String {
    let what = &refusal.what;
    let why = match refusal.kind {
        Kind::Approves => {
            return format!(
                "{REFUSED} `{what}` to every command of a session, whatever the level and the \
                 rules: it approves extensions, trusts projects, keeps credentials or answers \
                 for you. If you mean it, run it in your own terminal."
            );
        }
        Kind::Application => {
            return format!(
                "{REFUSED} to start its own application from a command of a session, whatever \
                 the level and the rules: it would answer a project's trust question and its \
                 approvals for you. If you mean it, run it in your own terminal."
            );
        }
        Kind::Widens => {
            "a run started by a command may not trust a project, widen what it may do, or take \
             its configuration and its stores from elsewhere"
        }
        Kind::Hidden => {
            "what follows it is known only once the shell expands it, so it may approve or \
             widen"
        }
        Kind::Internal => "it is one of the harness's own internal subcommands",
    };
    format!(
        "{REFUSED} `{what}` to every command of a session, whatever the level and the rules: \
         {why}. If you mean it, run it in your own terminal."
    )
}

/// What a front end tells the engine of its own commands: the names it
/// runs under, beside `cineris` and `cin`, and its grammar.
pub struct OwnCommands {
    names: Vec<String>,
    grammar: Option<Grammar>,
}

impl std::fmt::Debug for OwnCommands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnCommands")
            .field("names", &self.names)
            .field("grammar", &self.grammar.is_some())
            .finish()
    }
}

impl OwnCommands {
    /// The front end's names (its program's, as it was invoked and as it
    /// is on disk) and its grammar.
    #[must_use]
    pub fn new(names: impl IntoIterator<Item = String>, grammar: Option<Grammar>) -> Self {
        let mut all = vec!["cineris".to_owned(), "cin".to_owned()];
        for name in names {
            let name = name_of(&name);
            if !name.is_empty() && !all.contains(&name) {
                all.push(name);
            }
        }
        Self {
            names: all,
            grammar,
        }
    }

    /// How a run of Cineris with these words stands, by the grammar given,
    /// or the fallback's.
    #[must_use]
    pub fn standing(&self, rest: &[String], open: bool) -> Standing {
        match &self.grammar {
            Some(grammar) => grammar(rest, open),
            None => fallback(rest, open),
        }
    }
}

/// How deep a command line inside a word is read again.
const DEPTH: usize = 4;

/// How many lines one check reads at most: past it, a line that names
/// Cineris is refused.
const WORK: usize = 64;

/// The subcommands, by their first word.
const TOPICS: [&str; 12] = [
    "sessions",
    "config",
    "permissions",
    "auth",
    "models",
    "usage",
    "stats",
    "project",
    "ext",
    "setup",
    "serve",
    "attach",
];

/// The subcommands that approve, trust or answer: all of a topic's actions
/// (`None`), or those named. A topic with actions not named here only reads.
const APPROVING: [(&str, Option<&[&str]>); 8] = [
    ("ext", Some(&["add", "remove", "disable"])),
    (
        "permissions",
        Some(&["trust", "untrust", "revoke", "forget"]),
    ),
    (
        "project",
        Some(&["accept", "add", "edit", "promote", "disable", "delete"]),
    ),
    ("auth", Some(&["login", "logout"])),
    ("stats", Some(&["reset"])),
    ("setup", None),
    ("serve", None),
    ("attach", None),
];

/// The topics whose action is the next word: with that word not known, so
/// is what runs.
const WITH_ACTIONS: [&str; 7] = [
    "sessions",
    "config",
    "permissions",
    "auth",
    "stats",
    "project",
    "ext",
];

/// A run's flags that take a value apart from them.
const VALUE_FLAGS: [&str; 15] = [
    "--output",
    "--resume",
    "--fork",
    "--model",
    "--effort",
    "--speed",
    "--permission-mode",
    "--allow",
    "--deny",
    "--max-requests",
    "--max-cost",
    "--max-time",
    "--system-append",
    "--profile",
    "--log-level",
];

/// A run's flags that trust the project or widen what the run may do.
const WIDENING: [&str; 8] = [
    "--trust-project",
    "--permission-mode",
    "--allow",
    "-c",
    "--profile",
    "--continue",
    "--resume",
    "--fork",
];

/// The fallback's grammar: the CLI's, as this crate reads it where the
/// front end gives none.
fn fallback(rest: &[String], open: bool) -> Standing {
    let refused = |what: String, kind| Standing::Refused(Refusal::new(what, kind));
    let help = |words: &[String]| {
        words
            .iter()
            .take_while(|word| *word != "--")
            .any(|word| word == "-h" || word == "--help")
    };
    if let Some(first) = rest.first().map(String::as_str) {
        if first == "__sandbox" || first == "__tree-guard" {
            return refused(format!("cineris {first}"), Kind::Internal);
        }
        if let Some(topic) = TOPICS.iter().find(|topic| **topic == first) {
            if help(&rest[1..]) {
                return Standing::Safe;
            }
            let action = rest[1..].iter().find(|word| !word.starts_with('-'));
            match APPROVING.iter().find(|(name, _)| name == topic) {
                Some((_, None)) => return refused(format!("cineris {topic}"), Kind::Approves),
                Some((_, Some(actions))) => {
                    if let Some(action) = action
                        && actions.contains(&action.as_str())
                    {
                        return refused(format!("cineris {topic} {action}"), Kind::Approves);
                    }
                }
                None => {}
            }
            if open && action.is_none() && WITH_ACTIONS.contains(topic) {
                return refused(format!("cineris {topic}"), Kind::Hidden);
            }
            return Standing::Safe;
        }
    }
    let mut print = false;
    let mut widens: Option<String> = None;
    let mut at = 0;
    while let Some(word) = rest.get(at) {
        at += 1;
        if word == "--" {
            break;
        }
        if !word.starts_with('-') || word == "-" {
            continue;
        }
        let (flag, attached) = match word.split_once('=') {
            Some((flag, _)) if word.starts_with("--") => (flag, true),
            _ => (word.as_str(), false),
        };
        match flag {
            "-h" | "--help" | "-V" | "--version" => return Standing::Safe,
            "-p" | "--print" => print = true,
            "-q" | "--quiet" | "-v" | "--verbose" => {}
            long if long.starts_with("--") => {
                let value = if attached {
                    word.split_once('=').map(|(_, value)| value)
                } else {
                    rest.get(at).map(String::as_str)
                };
                // A mode no wider than `supervised` narrows.
                let narrows = long == "--permission-mode"
                    && value.is_some_and(|mode| matches!(mode, "read-only" | "supervised"));
                if WIDENING.contains(&long) && !narrows {
                    widens = Some(long.to_owned());
                }
                if VALUE_FLAGS.contains(&long) && !attached {
                    at += 1;
                }
            }
            // A cluster of short flags: `-pq`, `-cKEY=VALUE`.
            short => {
                for (index, letter) in short.char_indices().skip(1) {
                    match letter {
                        'p' => print = true,
                        'h' | 'V' => return Standing::Safe,
                        'c' => {
                            widens = Some("-c".to_owned());
                            if index + 1 == short.len() {
                                at += 1;
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if let Some(flag) = widens.filter(|_| print) {
        refused(format!("cineris -p {flag}"), Kind::Widens)
    } else if open {
        refused("cineris".to_owned(), Kind::Hidden)
    } else if !print {
        refused("cineris".to_owned(), Kind::Application)
    } else {
        Standing::Safe
    }
}

/// The name of the program a word names: its last name, after any `/` or
/// `\`, in lower case, with what Windows takes off a file's name (an
/// `::$DATA` stream, trailing dots and blanks) and the extension of a
/// program Windows runs by its name taken off (`.exe`, and a shim's
/// `.cmd`, `.bat`, `.com` or `.ps1`).
fn name_of(word: &str) -> String {
    let last = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let mut name: String = last
        .trim_start_matches('&')
        .chars()
        .filter(|c| !matches!(c, '`' | '^' | '"' | '\''))
        .collect::<String>()
        .to_ascii_lowercase();
    if let Some(at) = name.find("::$data") {
        name.truncate(at);
    }
    let name = name.trim_end_matches(['.', ' ']);
    let name = [".exe", ".cmd", ".bat", ".com", ".ps1"]
        .iter()
        .find_map(|extension| name.strip_suffix(extension))
        .unwrap_or(name);
    name.trim_end_matches(['.', ' ']).to_owned()
}

/// Whether `name` is Cineris's as an 8.3 short name: `cineri~1`.
fn short_name(name: &str) -> bool {
    name.strip_prefix("cineri~")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether the shell expands something in `word`: `$`, a backtick, `%…%`,
/// `!…!`, or PowerShell's `@name`.
fn expands(word: &str) -> bool {
    let between = |mark: char| word.matches(mark).count() >= 2;
    word.contains('$')
        || word.contains('`')
        || between('%')
        || between('!')
        || word
            .strip_prefix('@')
            .and_then(|rest| rest.chars().next())
            .is_some_and(char::is_alphabetic)
}

/// The variables that move Cineris's configuration and stores.
fn moves_stores(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    matches!(
        name.as_str(),
        "HOME"
            | "USERPROFILE"
            | "APPDATA"
            | "LOCALAPPDATA"
            | "XDG_CONFIG_HOME"
            | "XDG_STATE_HOME"
            | "XDG_DATA_HOME"
    ) || name.starts_with("CINERIS_")
}

/// The variable `word` sets or unsets, when it is one that moves the
/// stores: `NAME=value`, `$env:NAME`, `--unset=NAME`, or `NAME` after
/// `-u`, `--unset` or `unset`.
fn moved(word: &str, before: Option<&str>) -> Option<String> {
    let unsets = before.is_some_and(|before| matches!(before, "-u" | "--unset" | "unset"));
    // Most words set nothing: passed over before anything is made of them.
    if !unsets && !word.contains(['=', ':']) {
        return None;
    }
    let upper = word.to_ascii_uppercase();
    if let Some(at) = upper.find("$ENV:") {
        let name: String = upper[at + 5..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if moves_stores(&name) {
            return Some(name);
        }
    }
    let assigned = upper
        .strip_prefix("--UNSET=")
        .map(str::to_owned)
        .or_else(|| upper.split_once('=').map(|(name, _)| name.to_owned()));
    if let Some(name) = assigned
        && moves_stores(&name)
    {
        return Some(name);
    }
    if unsets && moves_stores(&upper) {
        return Some(upper);
    }
    None
}

/// Programs that run the simple command after their options: each with
/// the options that take a value apart from them, and how many words of
/// their own come before the command (`timeout`'s duration).
const WRAPPERS: [(&str, &[&str], usize); 33] = [
    (
        "sudo",
        &[
            "-u",
            "-g",
            "-h",
            "-p",
            "-C",
            "-D",
            "-r",
            "-t",
            "-U",
            "-T",
            "--user",
            "--group",
            "--host",
            "--prompt",
            "--chdir",
            "--role",
            "--type",
            "--other-user",
        ],
        0,
    ),
    ("doas", &["-u", "-C"], 0),
    ("pkexec", &["--user"], 0),
    ("nice", &["-n", "--adjustment"], 0),
    (
        "ionice",
        &["-c", "-n", "-p", "-P", "-u", "--class", "--classdata"],
        0,
    ),
    ("timeout", &["-s", "-k", "--signal", "--kill-after"], 1),
    (
        "stdbuf",
        &["-i", "-o", "-e", "--input", "--output", "--error"],
        0,
    ),
    ("setsid", &[], 0),
    ("nohup", &[], 0),
    ("unbuffer", &[], 0),
    ("command", &[], 0),
    ("builtin", &[], 0),
    ("exec", &["-a"], 0),
    ("chrt", &[], 1),
    ("taskset", &[], 1),
    (
        "flock",
        &["-w", "-E", "--timeout", "--conflict-exit-code"],
        1,
    ),
    ("time", &["-f", "-o", "--format", "--output"], 0),
    (
        "strace",
        &[
            "-o", "-e", "-p", "-s", "-u", "-E", "-a", "-b", "-I", "-O", "-P", "-S", "-X",
        ],
        0,
    ),
    (
        "ltrace",
        &["-o", "-e", "-p", "-s", "-u", "-a", "-n", "-l", "-x"],
        0,
    ),
    ("valgrind", &[], 0),
    (
        "xargs",
        &[
            "-a",
            "-d",
            "-E",
            "-I",
            "-L",
            "-n",
            "-P",
            "-s",
            "--arg-file",
            "--delimiter",
            "--eof",
            "--replace",
            "--max-lines",
            "--max-args",
            "--max-procs",
            "--max-chars",
        ],
        0,
    ),
    ("chroot", &["--userspec", "--groups"], 1),
    (
        "unshare",
        &["--propagation", "--setgroups", "--wd", "--root"],
        0,
    ),
    (
        "nsenter",
        &["-t", "--target", "-S", "-G", "--wd", "--root"],
        0,
    ),
    (
        "systemd-run",
        &[
            "-u",
            "--unit",
            "-p",
            "--property",
            "--slice",
            "-E",
            "--setenv",
            "--uid",
            "--gid",
            "-M",
            "-H",
            "--description",
            "-D",
        ],
        0,
    ),
    ("busybox", &[], 0),
    ("npx", &["-p", "--package"], 0),
    ("bunx", &[], 0),
    ("pnpx", &[], 0),
    ("uvx", &["--from", "--with", "-p", "--python"], 0),
    ("wsl", &["-d", "--distribution", "-u", "--user", "--cd"], 0),
    ("call", &[], 0),
    ("spawn", &[], 0),
];

/// Programs that run what their words say in ways not told apart here, the
/// terminal runners among them (they give what they run a terminal): a
/// name of Cineris at any of their words is a run of it, and a word of
/// theirs with a blank in it is a line.
const TERMINALS: [&str; 29] = [
    "script",
    "tmux",
    "screen",
    "dtach",
    "abduco",
    "zellij",
    "expect",
    "socat",
    "ssh",
    "mosh",
    "xterm",
    "gnome-terminal",
    "konsole",
    "alacritty",
    "kitty",
    "wezterm",
    "wt",
    "mintty",
    "conhost",
    "start",
    "start-process",
    "bwrap",
    "firejail",
    "parallel",
    "watch",
    "hyperfine",
    "eval",
    "iex",
    "invoke-expression",
];

/// Where the simple command a wrapper runs begins, when `words[program]`
/// is a wrapper.
fn wrapped(name: &str, words: &[String], program: usize) -> Option<usize> {
    let (_, values, positional) = WRAPPERS.iter().find(|(wrapper, _, _)| *wrapper == name)?;
    let mut at = program + 1;
    while let Some(word) = words.get(at) {
        if word == "--" {
            at += 1;
            break;
        }
        if !word.starts_with('-') || word == "-" {
            break;
        }
        at += if values.contains(&word.as_str()) {
            2
        } else {
            1
        };
    }
    at += positional;
    (at < words.len()).then_some(at)
}

/// The shells, and the programs that run a line after `-c` like one.
const SHELLS: [&str; 14] = [
    "sh", "bash", "dash", "zsh", "ksh", "mksh", "ash", "fish", "csh", "tcsh", "busybox", "script",
    "su", "runuser",
];

/// Interpreters whose code a flag gives: read as text, failing closed.
const INTERPRETERS: [(&str, &[&str]); 16] = [
    ("python", &["-c"]),
    ("python2", &["-c"]),
    ("python3", &["-c"]),
    ("pypy", &["-c"]),
    ("pypy3", &["-c"]),
    ("perl", &["-e", "-E"]),
    ("ruby", &["-e"]),
    ("node", &["-e", "--eval", "-p", "--print"]),
    ("nodejs", &["-e", "--eval", "-p", "--print"]),
    ("bun", &["-e", "--eval", "-p", "--print"]),
    ("deno", &["eval"]),
    ("php", &["-r"]),
    ("lua", &["-e"]),
    ("osascript", &["-e"]),
    ("expect", &["-c"]),
    ("tclsh", &["-c"]),
];

/// The `awk`s, whose program is their first word that is not an option.
const AWKS: [&str; 4] = ["awk", "gawk", "mawk", "nawk"];

/// How a word that names a line is to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// A shell's line.
    Shell,
    /// An interpreter's code: read as text.
    Code,
}

/// Whether the word at `at` is a line a program of the simple command
/// `words` (whose program is at `program`) is told to run, and how: after
/// the flag that names it, for the program that flag belongs to (the last
/// word before the flag that is not an option).
fn line_at(words: &[String], program: usize, at: usize) -> Option<Line> {
    if at == 0 || at <= program {
        return None;
    }
    let flag = words[at - 1].as_str();
    // Every flag that gives a line begins with `-`, or with `/` for `cmd`.
    if !flag.starts_with(['-', '/']) {
        return None;
    }
    let owner = words[program..at - 1]
        .iter()
        .rev()
        .find(|word| !word.starts_with('-'))
        .map(|word| name_of(word))?;
    let lower = flag.to_ascii_lowercase();
    if matches!(owner.as_str(), "pwsh" | "powershell")
        && matches!(lower.as_str(), "-c" | "-command" | "-cmd" | "--command")
    {
        return Some(Line::Shell);
    }
    if owner == "env" && matches!(flag, "-S" | "--split-string") {
        return Some(Line::Shell);
    }
    if SHELLS.contains(&owner.as_str()) {
        let clustered = flag.len() > 1
            && flag.starts_with('-')
            && !flag.starts_with("--")
            && flag[1..].bytes().all(|b| b.is_ascii_alphabetic())
            && flag.ends_with('c');
        if clustered || flag == "--command" {
            return Some(Line::Shell);
        }
    }
    if let Some((_, flags)) = INTERPRETERS.iter().find(|(name, _)| **name == owner)
        && flags.contains(&flag)
    {
        return Some(Line::Code);
    }
    None
}

/// The line a word gives attached to its flag: `--command=…`,
/// `--split-string=…`, `env -S…`.
fn attached_line<'w>(program: &str, word: &'w str) -> Option<&'w str> {
    if let Some(text) = word
        .strip_prefix("--command=")
        .or_else(|| word.strip_prefix("--split-string="))
    {
        return Some(text);
    }
    if program == "env" {
        return word.strip_prefix("-S").filter(|text| !text.is_empty());
    }
    None
}

/// The pieces of a word split at `=`, `!`, `,` and `:`, a drive's `:`
/// (`C:\…`) kept.
fn pieces(word: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for (at, character) in word.char_indices() {
        let drive = character == ':' && at == 1;
        if matches!(character, '=' | '!' | ',' | ':') && !drive {
            out.push(&word[start..at]);
            start = at + 1;
        }
    }
    out.push(&word[start..]);
    out
}

/// Where the program of `words` is: past `NAME=value` words and `env` with
/// its options and their values.
fn program_at(words: &[String]) -> usize {
    let assignment = |word: &str| {
        word.split_once('=').is_some_and(|(name, _)| {
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        })
    };
    let mut at = 0;
    while let Some(word) = words.get(at) {
        if assignment(word) {
            at += 1;
        } else if name_of(word) == "env" {
            at += 1;
            while let Some(word) = words.get(at) {
                if matches!(
                    word.as_str(),
                    "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string"
                ) {
                    at += 2;
                } else if word == "--" {
                    at += 1;
                    break;
                } else if word.starts_with('-') || assignment(word) {
                    at += 1;
                } else {
                    break;
                }
            }
        } else {
            break;
        }
    }
    at
}

/// One check of one command line.
struct Reader<'a> {
    own: &'a OwnCommands,
    /// The names, as they may be found in a line once its quotes and
    /// escapes are out.
    work: Cell<usize>,
    /// A variable that moves the stores, set or unset in the line.
    moved: RefCell<Option<String>>,
    /// Whether the line runs Cineris at all.
    runs: Cell<bool>,
    /// Whether the next run found takes more words than are written: what
    /// `xargs` and `parallel` add from their input.
    adds: Cell<bool>,
}

impl Reader<'_> {
    fn is_own(&self, word: &str) -> bool {
        let name = name_of(word);
        !name.is_empty() && (self.own.names.contains(&name) || short_name(&name))
    }

    /// Whether `word`, at a program's place, is a pattern that could match
    /// one of Cineris's names.
    fn could_be_own(&self, word: &str) -> bool {
        // Its first letter written out, as a name of Cineris begins: a
        // pattern that begins with a wildcard or a set could match anything.
        let name = name_of(word);
        if !word.contains(['?', '*', '['])
            || !self
                .own
                .names
                .iter()
                .any(|own| own.chars().next() == name.chars().next())
        {
            return false;
        }
        let pattern = NamePattern::parse(&name);
        self.own
            .names
            .iter()
            .any(|name| pattern.could_match_name(name, false))
    }

    /// Whether `text` could name Cineris at all: one of its names in it once
    /// quotes and escapes are out, or a pattern.
    fn mentioned(&self, text: &str) -> bool {
        let plain: String = text
            .chars()
            .filter(|c| !matches!(c, '\\' | '\'' | '"' | '`' | '^'))
            .flat_map(char::to_lowercase)
            .collect();
        self.own
            .names
            .iter()
            .any(|name| plain.contains(name.as_str()))
            || plain.contains("cineri~")
            || text.contains(['?', '*', '['])
    }

    fn spent(&self) -> bool {
        let work = self.work.get() + 1;
        self.work.set(work);
        work > WORK
    }

    /// A run of Cineris with the words `rest`, `open` when more may follow
    /// that are not known.
    fn run(&self, rest: &[String], open: bool) -> Option<Refusal> {
        self.runs.set(true);
        let open = open || self.adds.replace(false);
        // PowerShell's `--%` passes what follows as it is, and is not
        // itself an argument.
        let rest: Vec<String> = rest.iter().filter(|word| *word != "--%").cloned().collect();
        // Up to the first word the shell expands: what follows it is not
        // known.
        let known = rest.iter().position(|word| expands(word));
        let (rest, open) = match known {
            Some(at) => (&rest[..at], true),
            None => (&rest[..], open),
        };
        match self.own.standing(rest, open) {
            Standing::Safe => None,
            Standing::Refused(refusal) => Some(refusal),
        }
    }

    fn note_moved(&self, words: &[String]) {
        for (at, word) in words.iter().enumerate() {
            let before = at.checked_sub(1).map(|before| words[before].as_str());
            if let Some(name) = moved(word, before) {
                *self.moved.borrow_mut() = Some(name);
            }
        }
    }

    fn in_parts(&self, parts: &[Part], depth: usize) -> Option<Refusal> {
        parts.iter().find_map(|part| {
            let words: Vec<String> = part.words.iter().map(|word| word.text.clone()).collect();
            self.in_words(&words, depth)
        })
    }

    /// `env -S`'s lines, attached or not, and `--split-string`'s.
    fn env_lines(&self, words: &[String], depth: usize) -> Option<Refusal> {
        let mut in_env = false;
        for (at, word) in words.iter().enumerate() {
            let maybe_env = word.len() >= 3 && word.to_ascii_lowercase().contains("env");
            if maybe_env && name_of(word) == "env" {
                in_env = true;
                continue;
            }
            if !in_env {
                continue;
            }
            if !word.starts_with('-') && !word.contains('=') {
                in_env = false;
                continue;
            }
            let line = if matches!(word.as_str(), "-S" | "--split-string") {
                words.get(at + 1).map(String::as_str)
            } else {
                word.strip_prefix("--split-string=")
                    .or_else(|| word.strip_prefix("-S").filter(|text| !text.is_empty()))
            };
            if let Some(line) = line
                && self.mentioned(line)
                && let Some(refusal) = self.read(line, Line::Shell, depth)
            {
                return Some(refusal);
            }
        }
        None
    }

    /// One simple command.
    fn in_words(&self, words: &[String], depth: usize) -> Option<Refusal> {
        self.note_moved(words);
        if let Some(refusal) = self.env_lines(words, depth) {
            return Some(refusal);
        }
        let program = program_at(words);
        let first = words.get(program)?;
        let name = name_of(first);
        if self.is_own(first) || self.could_be_own(first) {
            return self.run(&words[program + 1..], false);
        }
        // A wrapper's command is a simple command of its own; `xargs`
        // adds words to it from its input.
        if let Some(next) = wrapped(&name, words, program) {
            if name == "xargs" {
                self.adds.set(true);
            }
            let found = self.in_words(&words[next..], depth);
            self.adds.set(false);
            return found;
        }
        if name == "find" {
            for (at, word) in words.iter().enumerate() {
                if matches!(word.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
                    let end = words[at + 1..]
                        .iter()
                        .position(|word| matches!(word.as_str(), ";" | "+" | "\\;"))
                        .map_or(words.len(), |end| at + 1 + end);
                    if let Some(refusal) = self.in_words(&words[at + 1..end], depth) {
                        return Some(refusal);
                    }
                }
            }
        }
        if name == "cargo"
            && words[program + 1..].iter().any(|word| word == "run")
            && let Some(refusal) = self.cargo_run(&words[program + 1..])
        {
            return Some(refusal);
        }
        // PowerShell's `Start-Process` names its program and its arguments
        // by its own parameters.
        if matches!(name.as_str(), "start-process" | "saps" | "start") {
            return self.start_process(&words[program + 1..], depth);
        }
        let terminal = TERMINALS.contains(&name.as_str());
        let mut awk_program = AWKS.contains(&name.as_str());
        for at in program + 1..words.len() {
            let word = &words[at];
            if terminal && self.is_own(word) {
                return self.run(&words[at + 1..], false);
            }
            let line = line_at(words, program, at).or_else(|| {
                let positional = !word.starts_with('-');
                let powershell = matches!(name.as_str(), "pwsh" | "powershell");
                if (terminal && word.contains(char::is_whitespace)) || (powershell && positional) {
                    Some(Line::Shell)
                } else if awk_program && positional {
                    awk_program = false;
                    Some(Line::Code)
                } else {
                    None
                }
            });
            if let Some(line) = line
                && self.mentioned(word)
                && let Some(refusal) = self.read(word, line, depth)
            {
                return Some(refusal);
            }
            if let Some(text) = attached_line(&name, word)
                && self.mentioned(text)
                && let Some(refusal) = self.read(text, Line::Shell, depth)
            {
                return Some(refusal);
            }
            if word.contains(['=', '!', ',', ':']) && self.mentioned(word) {
                let split = pieces(word);
                for (index, piece) in split.iter().enumerate() {
                    if self.is_own(piece) {
                        let rest = if index + 1 == split.len() {
                            let rest = &words[at + 1..];
                            rest.strip_prefix(&["--".to_owned()]).unwrap_or(rest)
                        } else {
                            &[]
                        };
                        return self.run(rest, false);
                    }
                }
                for (mark, _) in word.match_indices(['=', '!']) {
                    let text = &word[mark + 1..];
                    if self.mentioned(text)
                        && let Some(refusal) = self.read(text, Line::Shell, depth)
                    {
                        return Some(refusal);
                    }
                }
            }
        }
        if name == "cmd"
            && let Some(flag) = words
                .iter()
                .position(|word| matches!(word.to_ascii_lowercase().as_str(), "/c" | "/k"))
        {
            let rest = words[flag + 1..].join(" ");
            if let Some(refusal) = self.in_text(&rest, ShellSyntax::Cmd) {
                return Some(refusal);
            }
        }
        None
    }

    /// `cargo run` of Cineris: `--bin` naming it, or the package
    /// `cineris-cli`, whose binary it is; its words are those after `--`.
    fn cargo_run(&self, words: &[String]) -> Option<Refusal> {
        let names_it = words.iter().enumerate().any(|(at, word)| {
            let next = words.get(at + 1);
            (word == "--bin" && next.is_some_and(|next| self.is_own(next)))
                || (matches!(word.as_str(), "-p" | "--package")
                    && next.is_some_and(|next| next == "cineris-cli"))
                || word
                    .strip_prefix("--bin=")
                    .is_some_and(|name| self.is_own(name))
                || word.strip_prefix("--package=") == Some("cineris-cli")
        });
        if !names_it {
            return None;
        }
        let rest = words
            .iter()
            .position(|word| word == "--")
            .map_or(&[][..], |dash| &words[dash + 1..]);
        self.run(rest, false)
    }

    /// PowerShell's `Start-Process`: its program, by `-FilePath` or its
    /// first word, and its arguments, by `-ArgumentList` or its second.
    fn start_process(&self, words: &[String], depth: usize) -> Option<Refusal> {
        let mut program = None;
        let mut arguments: Vec<String> = Vec::new();
        let mut positional = 0;
        let mut at = 0;
        while let Some(word) = words.get(at) {
            let lower = word.to_ascii_lowercase();
            if matches!(lower.as_str(), "-filepath" | "-file" | "-path") {
                program = words.get(at + 1).cloned();
                at += 2;
                continue;
            }
            if matches!(lower.as_str(), "-argumentlist" | "-args" | "-arguments") {
                arguments.extend(words.get(at + 1).cloned());
                at += 2;
                continue;
            }
            if word.starts_with('-') || word.starts_with('/') {
                at += 1;
                continue;
            }
            match positional {
                0 if program.is_none() => program = Some(word.clone()),
                _ => arguments.push(word.clone()),
            }
            positional += 1;
            at += 1;
        }
        let program = program?;
        let mut line = vec![program];
        for argument in &arguments {
            line.extend(
                argument
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|piece| !piece.is_empty())
                    .map(str::to_owned),
            );
        }
        self.in_words(&line, depth + 1)
    }

    /// A line a word gives.
    fn read(&self, text: &str, line: Line, depth: usize) -> Option<Refusal> {
        if depth >= DEPTH || self.spent() {
            return Some(Refusal::new("cineris", Kind::Hidden));
        }
        match line {
            Line::Code => self.in_text(text, ShellSyntax::Posix),
            Line::Shell => {
                let mut modelled = false;
                for syntax in [ShellSyntax::Posix, ShellSyntax::PowerShell] {
                    if let Analysis::Parts(parts) = analyse(text, syntax) {
                        modelled = true;
                        if let Some(refusal) = self.in_parts(&parts, depth + 1) {
                            return Some(refusal);
                        }
                    }
                }
                if modelled {
                    None
                } else {
                    self.in_text(text, ShellSyntax::Unknown)
                }
            }
        }
    }

    /// A line the analysis cannot model, read as text, failing closed.
    fn in_text(&self, text: &str, syntax: ShellSyntax) -> Option<Refusal> {
        match syntax {
            ShellSyntax::Posix => self.in_plain(&plain(text, "\\'\"`")),
            ShellSyntax::PowerShell => self.in_plain(&plain(text, "'\"`")),
            ShellSyntax::Cmd => self.in_plain(&plain(text, "\"^")),
            ShellSyntax::Unknown => self
                .in_plain(&plain(text, "\\'\"`^"))
                .or_else(|| self.in_plain(&plain(text, "'\"`^"))),
        }
    }

    /// Text with its quotes and escapes out.
    fn in_plain(&self, text: &str) -> Option<Refusal> {
        let segments: Vec<Vec<String>> = text
            .split(['\n', ';', '&', '|', '(', ')', '{', '}'])
            .map(|segment| {
                segment
                    .split(|c: char| c.is_whitespace() || matches!(c, '<' | '>'))
                    .filter(|word| !word.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .collect();
        let mut named = false;
        for words in &segments {
            self.note_moved(words);
            // A whole word names Cineris at the program's place, or anywhere
            // after a wrapper or a runner's name; `Start-Process` by its
            // parameters.
            let program = program_at(words);
            if words.get(program).is_some_and(|first| {
                matches!(name_of(first).as_str(), "start-process" | "saps" | "start")
            }) && let Some(refusal) = self.start_process(&words[program + 1..], DEPTH - 1)
            {
                return Some(refusal);
            }
            let runs_words = words.get(program).is_some_and(|first| {
                let name = name_of(first);
                TERMINALS.contains(&name.as_str())
                    || SHELLS.contains(&name.as_str())
                    || AWKS.contains(&name.as_str())
                    || matches!(name.as_str(), "cmd" | "pwsh" | "powershell" | "env")
                    || INTERPRETERS
                        .iter()
                        .any(|(interpreter, _)| *interpreter == name)
                    || WRAPPERS.iter().any(|(wrapper, _, _)| *wrapper == name)
            });
            for (at, word) in words.iter().enumerate() {
                let placed = at == program || (runs_words && at > program);
                let own = ((placed && self.is_own(word))
                    || (at == program && self.could_be_own(word)))
                .then_some(at + 1)
                .or_else(|| {
                    if !word.contains(['=', '!', ',', ':']) {
                        return None;
                    }
                    let split = pieces(word);
                    split
                        .iter()
                        .position(|piece| self.is_own(piece))
                        .map(|index| {
                            if index + 1 == split.len() {
                                at + 1
                            } else {
                                words.len()
                            }
                        })
                });
                if let Some(from) = own {
                    named = true;
                    let open = expands(word);
                    if let Some(refusal) = self.run(&words[from.min(words.len())..], open) {
                        return Some(refusal);
                    }
                }
            }
        }
        // An approving pair anywhere, where Cineris is named.
        if named {
            let all: Vec<String> = segments.concat();
            for pair in all.windows(2) {
                let [topic, action] = pair else { continue };
                let (topic, action) = (topic.to_ascii_lowercase(), action.to_ascii_lowercase());
                let approves = APPROVING.iter().any(|(name, actions)| {
                    *name == topic
                        && actions.is_some_and(|actions| actions.contains(&action.as_str()))
                });
                if approves {
                    return Some(Refusal::new(
                        format!("cineris {topic} {action}"),
                        Kind::Approves,
                    ));
                }
            }
        }
        None
    }
}

/// `text` with the characters in `out` taken out.
fn plain(text: &str, out: &str) -> String {
    text.chars().filter(|c| !out.contains(*c)).collect()
}

/// What `command`, in the session's shell `shell`, runs of Cineris that is
/// refused, when it does: what the engine refuses at every level.
#[must_use]
pub fn refused(command: &str, shell: ShellSyntax, own: Option<&OwnCommands>) -> Option<Refusal> {
    let fallback;
    let own = match own {
        Some(own) => own,
        None => {
            fallback = OwnCommands::new(Vec::new(), None);
            &fallback
        }
    };
    let reader = Reader {
        own,
        work: Cell::new(0),
        moved: RefCell::new(None),
        runs: Cell::new(false),
        adds: Cell::new(false),
    };
    if !reader.mentioned(command) {
        return None;
    }
    let refusal = match analyse(command, shell) {
        Analysis::Parts(parts) => reader.in_parts(&parts, 0),
        Analysis::NotModelled { .. } => reader.in_text(command, shell),
    };
    refusal.or_else(|| {
        let moved = reader.moved.borrow().clone()?;
        reader
            .runs
            .get()
            .then(|| Refusal::new(format!("cineris with {moved} set"), Kind::Widens))
    })
}

#[cfg(test)]
mod tests;
