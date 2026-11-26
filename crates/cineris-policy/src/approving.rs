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
//!   its `-ArgumentList` or second, and `New-ScheduledTaskAction` by its
//!   `-Execute` and `-Argument`;
//! - in what a line feeds a program that runs it (the full review of the
//!   third part): where a shell, an interpreter, `cmd`, `pwsh -Command -`,
//!   `Invoke-Expression`, `at`, `batch`, `crontab` or `make -f -` reads its
//!   input, or `eval`, `source` or `.` runs a substitution, what the line's
//!   other programs print is read as a line (`echo '…' | sh`, `sh <(echo
//!   …)`), with `printf`'s `\n` and `\t` read as what they print;
//! - in the runners of a line that reach a daemon or a service (`at`,
//!   `crontab`, `systemd-run`, `busctl`, `gdbus`, `dbus-send`, `schtasks`,
//!   `wmic`, `Invoke-WmiMethod`, `Invoke-CimMethod`,
//!   `Register-ScheduledTask`, `explorer`), and in the keys `tmux`,
//!   `screen`, `xdotool`, `ydotool` and `wtype` are told to type, joined;
//! - in a line one word of a program's flag gives (`flock -c`, `rsync -e`,
//!   `tar -I` and `--to-command`, `zip -TT`, `nodemon --exec`, `entr -s`,
//!   `man -P`, an editor's `-c` and `+command`), in `git`'s configuration
//!   values that are lines (`core.pager`, `core.fsmonitor`, `core.editor`,
//!   `sequence.editor`, an `alias.*` that begins with `!`; `git` otherwise
//!   only prints), and in a `sed` script with an `e`;
//! - by shells and interpreters with a version after their names
//!   (`python3.12`, `lua5.4`, `bash5`), flag clusters that end in their code
//!   flag (`-Ic`, `-we`), and the shells `nu`, `xonsh`, `elvish`, `yash`,
//!   `oksh`, `osh`, `rc` and `hush`; an interpreter's lists and maps are
//!   words (`["cin","ext","add"]`); PowerShell's arrays, groups and casts are
//!   a native program's words (`cin ext @('add','C:\x')`); and in a line
//!   read as text, in any shell, a native program's words are read as code
//!   too, `\"` a quote and a list's brackets and commas blanks, as
//!   PowerShell, `cmd` and a service told to start `python -c "…"` hand
//!   them on;
//! - with line continuations taken out (`ci\⏎neris`), and look-alike letters
//!   read as the Latin ones (`ｃｉｎ`, `cın`, `cinerıſ`).
//!
//! **A line the analysis cannot model fails closed.** Its quotes and
//! escapes are taken out as its shell takes them (`\` escapes in POSIX and
//! separates on Windows, a backtick in PowerShell, `^` in `cmd`), and then:
//! a name of Cineris anywhere is judged with the words after it, up to the
//! first that the shell expands (`$`, a backtick, `%…%`, `!…!`, `@name`);
//! where one does, it is refused unless what is written out already fixes a
//! subcommand that only reads. A line that names Cineris and holds an
//! approving pair anywhere is refused too, and so is a brace expansion or a
//! word the shell completes that names it (`{cin,} ext add`,
//! `cin${IFS}ext`). The help or the version alone, with nothing after it,
//! is a run that reads, however the line was read.
//!
//! **What it does not reach:** a name the shell builds (`${X}neris`,
//! `$(printf ci)neris`), a script file or a task made before, an encoded
//! PowerShell command, a copy under another name than the front end knows,
//! a `cargo run` that names neither the binary nor its package (`cargo run
//! --bin cineris` and `-p cineris-cli` are read), keys typed into a terminal
//! by its device. The approving subcommands look at their own line of
//! parents, or the harness's job on Windows, and refuse under a session's
//! command: that is the backstop (`cineris_sandbox::session`).

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::sync::Arc;

pub use crate::command::ShellSyntax;
use crate::command::{Analysis, Part, analyse};
use crate::expansion::NamePattern;

mod cases;
pub use cases::CASES;

/// How a command line of the harness's own stands, as the front end reads
/// it: given the words after the program's name, and whether more words
/// may follow that are not known (an expansion the shell makes).
pub type Grammar = Arc<dyn Fn(&Invocation<'_>) -> Standing + Send + Sync>;

/// One run of Cineris a line holds, as a front end's grammar is asked of it.
#[derive(Debug, Clone, Copy)]
pub struct Invocation<'a> {
    /// The words after the program's name, up to the first that the shell
    /// expands.
    pub words: &'a [String],
    /// More words follow that only the shell knows.
    pub open: bool,
    /// The words are the program's own arguments: the line was modelled and
    /// none of them is a pattern. Where they may not be, a request for the
    /// help or the version, or words the grammar cannot read, are not taken
    /// as safe (the re-look at the early review, H2).
    pub exact: bool,
}

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
    /// or the fallback's. A grammar that panics refuses: it is not trusted
    /// past it.
    #[must_use]
    pub fn standing(&self, words: &[String], open: bool, exact: bool) -> Standing {
        match &self.grammar {
            Some(grammar) => {
                let invocation = Invocation { words, open, exact };
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| grammar(&invocation)))
                    .unwrap_or_else(|_| Standing::Refused(Refusal::new("cineris", Kind::Hidden)))
            }
            None => fallback(words, open, exact),
        }
    }
}

/// How deep a command line inside a word is read again.
const DEPTH: usize = 4;

/// How many lines one check reads at most: past it, a line that names
/// Cineris is refused.
const WORK: usize = 64;

/// How deep one check nests simple commands within each other at most (a
/// wrapper's command, `find -exec`'s, `Start-Process`'s, a line's), and how
/// many it reads in all: past either, a line that names Cineris is refused.
/// Without them a line of a thousand `nohup`s overflowed the core thread's
/// stack (the full review of the third part, M1).
const NESTING: usize = 24;
const STEPS: usize = 4096;

/// The longest line that names Cineris this check reads: a longer one is
/// refused.
const LONGEST: usize = 32 * 1024;

/// How many of a word's `=` and `!` begin a line read again: past them, the
/// word is refused (reading each was quadratic in the word).
const MARKS: usize = 16;

/// The most words after a name of Cineris that one run of it is read with:
/// a run with more is refused, wherever it stands. Each name found reads
/// the words after it, so that without this a line of many names, as
/// separate words, read the rest of the line once for each and was
/// quadratic (the narrow re-check of the managing part: `x cin cin …` of
/// 8,000 names took more than 10 s); with it, each name reads at most this
/// many, and the pass is linear in the line.
const RUN_WORDS: usize = 128;

/// The most runs of Cineris one check reads, every reading of the line
/// counted (its continuations joined, its text read in each shell's way):
/// past them, the line is refused. The cap behind [`RUN_WORDS`]: a line of
/// many short runs reads each one's words once, and is still refused
/// soon.
const MENTIONS: usize = 64;

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
fn fallback(rest: &[String], open: bool, exact: bool) -> Standing {
    let refused = |what: String, kind| Standing::Refused(Refusal::new(what, kind));
    // The help or the version, asked for in words that may not be the
    // program's own, is not taken as asked.
    let help_or = || {
        if exact {
            Standing::Safe
        } else {
            Standing::Refused(Refusal::new("cineris", Kind::Hidden))
        }
    };
    let help = |words: &[String]| {
        let words: Vec<&String> = words.iter().take_while(|word| *word != "--").collect();
        words.iter().enumerate().any(|(at, word)| {
            let value = at.checked_sub(1).is_some_and(|before| {
                matches!(
                    words[before].as_str(),
                    "--hash"
                        | "--extension"
                        | "--hook"
                        | "--purpose"
                        | "--command"
                        | "--cwd"
                        | "--kind"
                        | "--layer"
                        | "--limit"
                        | "--since"
                        | "--socket"
                        | "--resume"
                        | "--log-level"
                        | "--profile"
                )
            });
            !value && (*word == "-h" || *word == "--help")
        })
    };
    if let Some(first) = rest.first().map(String::as_str) {
        if first == "__sandbox" || first == "__tree-guard" {
            return refused(format!("cineris {first}"), Kind::Internal);
        }
        if let Some(topic) = TOPICS.iter().find(|topic| **topic == first) {
            if help(&rest[1..]) {
                return help_or();
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
            "-h" | "--help" | "-V" | "--version" => return help_or(),
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
                        // What follows `p` in its cluster is the prompt.
                        'p' => {
                            print = true;
                            break;
                        }
                        'h' | 'V' => return help_or(),
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
    // A call's `&`, and a block's or a group's brace or parenthesis, glued
    // to the name.
    let mut name: String = folded(
        &last
            .trim_start_matches(['&', '{', '('])
            .trim_end_matches(['}', ')'])
            .chars()
            .filter(|c| !matches!(c, '`' | '^' | '"' | '\''))
            .collect::<String>(),
    );
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

/// `text` in lower case, with the letters that look like the Latin ones of
/// Cineris's names written as those: full-width forms (`ｃｉｎ`), the dotless
/// and the dotted `i` (`cın`, `CİN`), the long `s` (`cinerıſ`), Cyrillic
/// and Greek look-alikes, small capitals, Roman numerals and accented
/// letters; combining marks are dropped (the full review of the third part).
/// A name written so runs Cineris only through a link of that name, which a
/// copy under any name reaches too; it is read as Cineris's all the same.
fn folded(text: &str) -> String {
    // Most names are plain ASCII: lowered as they were before.
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    text.chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| match c {
            '\u{0300}'..='\u{036f}' => None,
            '\u{ff01}'..='\u{ff5e}' => {
                char::from_u32(u32::from(c) - 0xfee0).map(|c| c.to_ascii_lowercase())
            }
            'ı' | 'ɪ' | 'ɩ' | 'і' | 'ι' | 'ⅰ' | 'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' => {
                Some('i')
            }
            'ſ' | 'ѕ' | 'ꜱ' | 'ś' | 'š' | 'ş' | 'ș' => Some('s'),
            'с' | 'ϲ' | 'ᴄ' | 'ⅽ' | 'ç' | 'ć' | 'č' => Some('c'),
            'е' | 'ᴇ' | 'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => Some('e'),
            'ո' | 'ɴ' | 'η' | 'ñ' | 'ń' | 'ň' => Some('n'),
            'ʀ' | 'ŕ' | 'ř' | 'г' => Some('r'),
            other => Some(other),
        })
        .collect()
}

/// `command` with its line continuations taken out, as its shell joins
/// them: `\` before a line break (`⏎` or `\r⏎`) in POSIX, `^` in `cmd`, a
/// backtick in PowerShell, and all of them where the shell is not known
/// (the full review of the third part: `ci\⏎neris` is `cineris`). A mark
/// that a join brings before another line break joins that one too
/// (`ci\\⏎⏎neris`), so the result has no continuation left: it is made in
/// one pass, as a stack, with no reading again (the narrow re-check of the
/// third part, M1': a line of 3,000 `\` and 3,000 line breaks was read
/// again once for each, and overflowed the stack).
fn joined(command: &str, shell: ShellSyntax) -> String {
    let marks: &[char] = match shell {
        ShellSyntax::Posix => &['\\'],
        ShellSyntax::Cmd => &['^'],
        ShellSyntax::PowerShell => &['`'],
        ShellSyntax::Unknown => &['\\', '^', '`'],
    };
    let mut out = String::with_capacity(command.len());
    let mut characters = command.chars().peekable();
    while let Some(character) = characters.next() {
        let breaks = character == '\n' || (character == '\r' && characters.peek() == Some(&'\n'));
        if breaks && out.ends_with(marks) {
            if character == '\r' {
                characters.next();
            }
            out.pop();
            continue;
        }
        out.push(character);
    }
    out
}

/// Whether `name` is Cineris's as an 8.3 short name: `cineri~1`.
fn short_name(name: &str) -> bool {
    name.strip_prefix("cineri~")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Whether the shell, or the program, puts something else in `word`'s
/// place: `$`, a backtick, `%…%`, `!…!`, PowerShell's `@name`, a brace's
/// `{a,b}`, `find`'s `{}` or `parallel`'s `:::`.
fn expands(word: &str) -> bool {
    let between = |mark: char| word.matches(mark).count() >= 2;
    let braces = word
        .find('{')
        .and_then(|open| {
            word[open..]
                .find('}')
                .map(|close| &word[open..open + close])
        })
        .is_some_and(|inside| inside.contains(',') || inside.contains(".."));
    // `find`'s `{}`, `parallel`'s `:::`, a brace's words.
    braces
        || word == "{}"
        || word == ":::"
        || word == "::::"
        || word.contains('$')
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
    if let Some(name) = word.strip_prefix("-u").filter(|name| !name.is_empty())
        && moves_stores(name)
    {
        return Some(name.to_ascii_uppercase());
    }
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
        // Only an assignment moves it: `$env:HOME = …`, not a read of it.
        let after = upper[at + 5 + name.len()..].trim_start();
        if moves_stores(&name) && after.starts_with('=') && !after.starts_with("==") {
            return Some(name);
        }
        return None;
    }
    // PowerShell's drive of the environment: `Set-Item env:HOME x`.
    if let Some(name) = upper
        .strip_prefix("ENV:")
        .or_else(|| upper.strip_prefix("ENV:\\"))
        && moves_stores(name)
    {
        return Some(name.to_owned());
    }
    let assigned = upper
        .strip_prefix("--UNSET=")
        .map(str::to_owned)
        .or_else(|| {
            upper
                .split_once('=')
                .map(|(name, _)| name.trim_end_matches('+').to_owned())
        });
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
const WRAPPERS: &[(&str, &[&str], usize)] = &[
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
    ("fakeroot", &[], 0),
    ("poetry", &[], 1),
    ("uv", &[], 1),
    ("npm", &[], 1),
    ("pnpm", &[], 1),
    ("bundle", &[], 1),
    ("direnv", &[], 2),
    ("mise", &[], 1),
    ("faketime", &[], 1),
    (
        "numactl",
        &[
            "-N",
            "-C",
            "-m",
            "-p",
            "-i",
            "-P",
            "--cpunodebind",
            "--membind",
            "--physcpubind",
        ],
        0,
    ),
    ("proxychains4", &["-f"], 0),
    ("proxychains", &["-f"], 0),
    ("dbus-run-session", &["--config-file", "--dbus-daemon"], 0),
    ("wine", &[], 0),
    ("gsudo", &["-u", "--user"], 0),
    ("rlwrap", &["-f", "-H", "-p", "-s", "-S"], 0),
    ("winpty", &[], 0),
];

/// Programs that run what their words say in ways not told apart here, the
/// terminal runners among them (they give what they run a terminal): a
/// name of Cineris at any of their words is a run of it, and a word of
/// theirs with a blank in it is a line.
const TERMINALS: &[&str] = &[
    // The daemons' and the services' runners of a line (the full review of
    // the third part): what they start is not the session's command's own
    // child.
    "at",
    "batch",
    "crontab",
    "busctl",
    "gdbus",
    "dbus-send",
    "systemd-run",
    "xdotool",
    "ydotool",
    "wtype",
    "explorer",
    "schtasks",
    "wmic",
    "invoke-wmimethod",
    "invoke-cimmethod",
    "register-scheduledtask",
    "new-scheduledtaskaction",
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
    // `flock FILE -c LINE` runs a line: read as one, by its flag.
    if name == "flock"
        && words[program + 1..]
            .iter()
            .any(|word| word == "-c" || word == "--command")
    {
        return None;
    }
    // `command -v` and `-V` say what a name is, and run nothing.
    if name == "command"
        && words[program + 1..]
            .iter()
            .any(|word| word == "-v" || word == "-V")
    {
        return None;
    }
    let mut at = program + 1;
    while let Some(word) = words.get(at) {
        if word == "--" {
            at += 1;
            break;
        }
        if !word.starts_with('-') || word == "-" {
            break;
        }
        // A cluster of short options whose last takes a value: `-Eu root`.
        let last = word
            .chars()
            .last()
            .filter(|_| word.len() > 2 && !word.starts_with("--"))
            .map(|letter| format!("-{letter}"));
        let takes = values.contains(&word.as_str())
            || last.is_some_and(|last| values.contains(&last.as_str()));
        at += if takes { 2 } else { 1 };
    }
    at += positional;
    if words.get(at).is_some_and(|word| word == "--") {
        at += 1;
    }
    (at < words.len()).then_some(at)
}

/// The shells, and the programs that run a line after `-c` like one.
const SHELLS: &[&str] = &[
    "sh", "bash", "dash", "zsh", "ksh", "mksh", "ash", "fish", "csh", "tcsh", "busybox", "script",
    "su", "runuser", "nu", "xonsh", "elvish", "yash", "oksh", "osh", "rc", "hush", "posh", "loksh",
    "pdksh",
];

/// A program's name with a version after it taken off (`python3.12`,
/// `lua5.4`, `bash5`), and the other names of an interpreter written as its
/// own (`luajit`, `nodejs`): what the shells and the interpreters are looked
/// for by.
fn kind_of(name: &str) -> &str {
    let bare = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    match bare {
        "luajit" => "lua",
        "nodejs" => "node",
        "" => name,
        other => other,
    }
}

/// Interpreters whose code a flag gives: read as text, failing closed.
const INTERPRETERS: &[(&str, &[&str])] = &[
    ("python", &["-c"]),
    ("pypy", &["-c"]),
    ("perl", &["-e", "-E"]),
    ("ruby", &["-e"]),
    ("node", &["-e", "--eval", "-p", "--print"]),
    ("bun", &["-e", "--eval", "-p", "--print"]),
    ("deno", &["eval"]),
    ("php", &["-r", "-B", "-R", "-E"]),
    ("lua", &["-e"]),
    ("osascript", &["-e"]),
    ("expect", &["-c"]),
    ("tclsh", &["-c"]),
    ("rscript", &["-e"]),
    ("julia", &["-e", "-E", "--eval", "--print"]),
];

/// Whether `flag` gives an interpreter's code in the word after it, by one
/// of its `flags`: alone, as `deno`'s `eval`, or last in a cluster of short
/// flags (`-Ic`, `-we`, `-le`, `-pe`).
fn gives_code(flag: &str, flags: &[&str]) -> bool {
    if flags.contains(&flag) {
        return true;
    }
    let cluster = flag.len() > 2
        && flag.starts_with('-')
        && !flag.starts_with("--")
        && flag[1..].bytes().all(|byte| byte.is_ascii_alphabetic());
    cluster
        && flag
            .chars()
            .last()
            .is_some_and(|last| flags.contains(&format!("-{last}").as_str()))
}

/// Programs that run a line one of their flags gives, in one word (the
/// full review of the third part), and how it is read.
const LINE_FLAGS: &[(&str, &[&str], Line)] = &[
    ("flock", &["-c", "--command"], Line::Shell),
    ("rsync", &["-e", "--rsh", "--rsync-path"], Line::Shell),
    (
        "tar",
        &[
            "-I",
            "-F",
            "--use-compress-program",
            "--to-command",
            "--info-script",
            "--new-volume-script",
        ],
        Line::Shell,
    ),
    ("zip", &["-TT", "--unzip-command"], Line::Shell),
    ("nodemon", &["--exec", "-x"], Line::Shell),
    ("entr", &["-s"], Line::Shell),
    ("man", &["-P", "--pager"], Line::Shell),
    ("vim", &["-c", "--cmd"], Line::Code),
    ("vi", &["-c", "--cmd"], Line::Code),
    ("nvim", &["-c", "--cmd"], Line::Code),
    ("gvim", &["-c", "--cmd"], Line::Code),
    ("ex", &["-c", "--cmd"], Line::Code),
    ("view", &["-c", "--cmd"], Line::Code),
];

/// The editors whose `+command` word is a command they run.
const VIMS: [&str; 6] = ["vim", "vi", "nvim", "gvim", "ex", "view"];

/// The keys of `git`'s configuration whose value is a line it runs, beside
/// any `alias.*` whose value begins with `!`.
const GIT_LINES: [&str; 8] = [
    "core.fsmonitor",
    "core.pager",
    "core.editor",
    "core.sshcommand",
    "sequence.editor",
    "diff.external",
    "pager.log",
    "credential.helper",
];

/// Whether a `sed` script runs a line: an `e` command, or an `s` command
/// with the `e` flag. Any other script only edits text, so its words are
/// not read as lines (a false refusal of the full review).
fn sed_runs(script: &str) -> bool {
    let bytes = script.as_bytes();
    // An `e` command: after the script's start, a separator, an address's
    // digit or `$`, or a block's brace, then blanks.
    let command = bytes.iter().enumerate().any(|(at, byte)| {
        *byte == b'e'
            && bytes
                .get(at + 1)
                .is_none_or(|next| next.is_ascii_whitespace() || *next == b';')
            && bytes[..at]
                .iter()
                .rev()
                .find(|before| !before.is_ascii_whitespace())
                .is_none_or(|before| {
                    before.is_ascii_digit()
                        || matches!(before, b';' | b'\n' | b'{' | b'}' | b'$' | b',')
                })
    });
    if command {
        return true;
    }
    // `s` commands: the flags after the third delimiter.
    let mut at = 0;
    while let Some(found) = script[at..].find('s') {
        let start = at + found;
        at = start + 1;
        let Some(delimiter) = script[start + 1..].chars().next() else {
            break;
        };
        if delimiter.is_alphanumeric() || delimiter.is_whitespace() || delimiter == '\\' {
            continue;
        }
        let mut seen = 0;
        let mut escaped = false;
        let mut end = None;
        for (offset, c) in script[start + 1 + delimiter.len_utf8()..].char_indices() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == delimiter {
                seen += 1;
                if seen == 2 {
                    end = Some(start + 1 + delimiter.len_utf8() + offset + c.len_utf8());
                    break;
                }
            }
        }
        if let Some(end) = end {
            let flags: String = script[end..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if flags.contains('e') {
                return true;
            }
            at = end;
        }
    }
    false
}

/// The `awk`s, whose program is their first word that is not an option
/// ([`awk_program`]).
const AWKS: [&str; 4] = ["awk", "gawk", "mawk", "nawk"];

/// How a word that names a line is to be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// A shell's line.
    Shell,
    /// An interpreter's code: read as text.
    Code,
}

/// Whether the word at `at` is a line the program of the simple command
/// `words` (at `program`) is told to run, and how.
fn line_at(words: &[String], program: usize, at: usize) -> Option<Line> {
    if at <= program {
        return None;
    }
    let name = name_of(&words[program]);
    let owner = kind_of(&name);
    if SHELLS.contains(&owner) || owner == "sg" {
        return (shell_line(words, program) == Some(at)).then_some(Line::Shell);
    }
    let flag = words[at - 1].as_str();
    if owner == "deno" && flag == "eval" {
        return Some(Line::Code);
    }
    // Every flag that gives a line begins with `-`, or with `/` for `cmd`.
    if !flag.starts_with(['-', '/']) {
        return None;
    }
    let lower = flag.to_ascii_lowercase();
    if matches!(owner, "pwsh" | "powershell")
        && matches!(lower.as_str(), "-c" | "-command" | "-cmd" | "--command")
    {
        return Some(Line::Shell);
    }
    if owner == "env" && matches!(flag, "-S" | "--split-string") {
        return Some(Line::Shell);
    }
    if matches!(owner, "nix-shell" | "nix") && matches!(flag, "--run" | "--command") {
        return Some(Line::Shell);
    }
    if let Some((_, flags)) = INTERPRETERS.iter().find(|(name, _)| *name == owner)
        && gives_code(flag, flags)
    {
        return Some(Line::Code);
    }
    if let Some((_, flags, line)) = LINE_FLAGS.iter().find(|(name, _, _)| *name == owner)
        && flags.contains(&flag)
    {
        return Some(*line);
    }
    None
}

/// Where a shell's line is: the first word that is not an option after its
/// `-c` (alone, or in a cluster), past `--` and the options after it; the
/// values of `-o`, `-O`, `--rcfile` and `--init-file` are passed over, and
/// so is the first word of `su`, `runuser` and `sg` (a user, a group).
fn shell_line(words: &[String], program: usize) -> Option<usize> {
    let name = name_of(&words[program]);
    let mut named = !matches!(kind_of(&name), "su" | "runuser" | "sg");
    let mut lined = false;
    let mut at = program + 1;
    while let Some(word) = words.get(at) {
        let option = word.starts_with(['-', '+']) && word.len() > 1;
        if lined {
            if option {
                at += 1;
                continue;
            }
            return Some(at);
        }
        if matches!(
            word.as_str(),
            "-o" | "-O" | "+o" | "+O" | "--rcfile" | "--init-file"
        ) {
            at += 2;
            continue;
        }
        let cluster = word.starts_with('-')
            && !word.starts_with("--")
            && word.len() > 1
            && word[1..].bytes().all(|byte| byte.is_ascii_alphabetic());
        if word == "--command" || (cluster && word.contains('c')) {
            lined = true;
        } else if !option {
            if named {
                // A script's name: what it runs is in a file.
                return None;
            }
            named = true;
        }
        at += 1;
    }
    None
}

/// Where an `awk`'s program is: its first word that is not an option, past
/// the values of `-v`, `-F` and `-W`; none with `-f`.
fn awk_program(words: &[String], program: usize) -> Option<usize> {
    let mut at = program + 1;
    while let Some(word) = words.get(at) {
        match word.as_str() {
            "-f" | "--file" => return None,
            "-v" | "-F" | "-W" | "--assign" | "--field-separator" => at += 2,
            "--" => return words.get(at + 1).map(|_| at + 1),
            option if option.starts_with('-') && option.len() > 1 => at += 1,
            _ => return Some(at),
        }
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
            let name = name.strip_suffix('+').unwrap_or(name);
            !name.is_empty()
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !name.starts_with(|c: char| c.is_ascii_digit())
        })
    };
    let mut at = 0;
    while let Some(word) = words.get(at) {
        // A group's brace, or a pipeline's `!`, before the program.
        if matches!(word.as_str(), "{" | "}" | "!") || assignment(word) {
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

/// Programs whose words are only printed, searched or looked at: a name of
/// Cineris among them is no run of it.
const PRINTERS: [&str; 35] = [
    "echo",
    "printf",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ag",
    "ack",
    "cat",
    "bat",
    "less",
    "more",
    "head",
    "tail",
    "wc",
    "ls",
    "tree",
    "file",
    "stat",
    "du",
    "jq",
    "diff",
    "sort",
    "uniq",
    "cut",
    "man",
    "select-string",
    "sls",
    "write-output",
    "write-host",
    "get-content",
    "gc",
    "findstr",
    "get-childitem",
    "git",
];

/// Programs that change the directory what follows runs in.
const CHANGES_DIRECTORY: [&str; 6] = [
    "cd",
    "pushd",
    "chdir",
    "set-location",
    "sl",
    "push-location",
];

/// Programs that run a file's lines in this shell, or a line they make.
const SOURCES: [&str; 3] = ["source", ".", "eval"];

/// Whether the simple command `words` runs text that it does not hold
/// itself as a line: a shell, an interpreter, `make -f -`, `cmd`, `pwsh`,
/// `at`, `batch` or `crontab` given no line, no code and no file of its own,
/// or its standard input as its file, reading what a pipe or a
/// substitution gives it; a shell whose line is an expansion or that `xargs`
/// gives its line; `eval`, `source`, `.` and `Invoke-Expression`. Where a
/// line holds one, what its other programs print is read as a line too
/// (the full review of the third part: `echo '…' | sh`).
fn reads_a_line(words: &[String]) -> bool {
    let mut program = program_at(words);
    for _ in 0..NESTING {
        let Some(first) = words.get(program) else {
            return false;
        };
        match wrapped(&name_of(first), words, program) {
            Some(next) => program = next + program_at(&words[next..]),
            None => break,
        }
    }
    let Some(first) = words.get(program) else {
        return false;
    };
    let name = name_of(first);
    let kind = kind_of(&name);
    let rest = &words[program + 1..];
    let input = |word: &str| matches!(word, "-" | "/dev/stdin" | "/dev/fd/0" | "/proc/self/fd/0");
    let positional = |skip: usize| rest.iter().filter(|word| !word.starts_with('-')).nth(skip);
    if SOURCES.contains(&kind) || first == "." {
        return true;
    }
    match kind {
        "at" | "batch" | "crontab" | "iex" | "invoke-expression" => return true,
        "cmd" => {
            return !rest.iter().any(|word| {
                let lower = word.to_ascii_lowercase();
                lower.starts_with("/c") || lower.starts_with("/k")
            });
        }
        "pwsh" | "powershell" => {
            let lower: Vec<String> = rest.iter().map(|word| word.to_ascii_lowercase()).collect();
            let given = lower.iter().any(|word| {
                matches!(
                    word.as_str(),
                    "-c" | "-command"
                        | "-cmd"
                        | "--command"
                        | "-f"
                        | "-file"
                        | "-e"
                        | "-ec"
                        | "-encodedcommand"
                )
            }) || lower.iter().any(|word| !word.starts_with('-'));
            return lower.iter().any(|word| word == "-") || !given;
        }
        "make" | "gmake" => {
            return rest.windows(2).any(|pair| {
                matches!(pair[0].as_str(), "-f" | "--file" | "--makefile") && input(&pair[1])
            }) || rest
                .iter()
                .any(|word| matches!(word.as_str(), "-f-" | "--file=-" | "--makefile=-"));
        }
        _ => {}
    }
    if SHELLS.contains(&kind) {
        if let Some(at) = shell_line(words, program) {
            return expands(&words[at]);
        }
        let lined = rest.iter().any(|word| {
            word == "--command"
                || (word.starts_with('-')
                    && !word.starts_with("--")
                    && word.len() > 1
                    && word[1..].bytes().all(|byte| byte.is_ascii_alphabetic())
                    && word.contains('c'))
        });
        let skip = usize::from(matches!(kind, "su" | "runuser" | "sg"));
        return lined
            || rest.iter().any(|word| word == "-s")
            || positional(skip).is_none_or(|word| input(word));
    }
    if let Some((_, flags)) = INTERPRETERS
        .iter()
        .find(|(interpreter, _)| *interpreter == kind)
    {
        if rest.iter().any(|word| gives_code(word, flags)) {
            return false;
        }
        return positional(0).is_none_or(|word| input(word));
    }
    false
}

/// The keys a terminal is told to type, joined as they would be typed:
/// the words after `tmux`'s and `screen`'s `send-keys`, `send` or `stuff`,
/// and `xdotool`'s or `ydotool`'s `type`, or all of `wtype`'s, past their
/// options.
fn typed(name: &str, words: &[String]) -> String {
    let from = if name == "wtype" {
        Some(0)
    } else {
        words
            .iter()
            .position(|word| matches!(word.as_str(), "send-keys" | "send" | "stuff" | "type"))
            .map(|at| at + 1)
    };
    let Some(mut at) = from else {
        return String::new();
    };
    let mut keys = String::new();
    while let Some(word) = words.get(at) {
        if matches!(
            word.as_str(),
            "-t" | "-N" | "-c" | "-d" | "-M" | "-m" | "-k" | "-P" | "-s" | "--delay" | "--window"
        ) {
            at += 2;
            continue;
        }
        if !(word.starts_with('-') && word.len() > 1) {
            keys.push_str(word);
        }
        at += 1;
    }
    keys
}

/// The brace expansions of `text` (`{cin,}`, `{a..b}`), each as where its
/// `{` and its `}` are: from the first `{` after a blank or the last `}`,
/// to the next `}`, with no blank between and a `,` or a `..` inside. Of
/// the `{`s before one `}`, the first's text holds each later one's, so
/// only it is looked at. One pass, remembering that `{` (the narrow
/// re-check of the third part, M1': a search for the `}` from each `{` was
/// quadratic, and `cin ` and 32,000 `{` took more than 10 s).
fn brace_expansions(text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut open = None;
    for (at, character) in text.char_indices() {
        match character {
            blank if blank.is_whitespace() => open = None,
            '{' if open.is_none() => open = Some(at),
            '}' => {
                if let Some(start) = open.take() {
                    let inside = &text[start + 1..at];
                    if inside.contains(',') || inside.contains("..") {
                        found.push((start, at));
                    }
                }
            }
            _ => {}
        }
    }
    found
}

/// A brace expansion in `text` (`{cin,}`, `{a..b}`), with no blank in it,
/// one of whose words is a name of Cineris: `{cin,} ext add` runs it, and
/// the reading here does not expand braces. A word is split at a `{` too,
/// so that a brace inside (`{a{cin,x}`) is read.
fn braced_name(text: &str, own: impl Fn(&str) -> bool) -> bool {
    brace_expansions(text)
        .into_iter()
        .any(|(open, close)| text[open + 1..close].split(['{', ',', '.']).any(&own))
}

/// Whether `word` names Cineris once what the shell expands in it is taken
/// for a blank: `cin${IFS}ext`, `$HOME/.local/bin/cin`.
fn names_through_expansion(word: &str, own: impl Fn(&str) -> bool) -> bool {
    word.contains('$')
        && word
            .replace(['$', '{', '}', '(', ')'], " ")
            .split_whitespace()
            .any(own)
}

/// PowerShell text with its type literals taken out: `[string]'add'` is
/// the word `add`. A literal's name is looked at only as far as it goes,
/// so that each character is read at most twice (a search for the `]`
/// from each `[` was quadratic).
fn without_casts(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let name = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_')))
            .unwrap_or(after.len());
        if name > 0 && after[name..].starts_with(']') {
            rest = &after[name + 1..];
        } else {
            out.push('[');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Code an interpreter runs, with its lists, maps and blocks taken for
/// blanks: `["cin","ext","add"]` and `{args:["ext"]}` are read as words.
fn code_words(text: &str) -> String {
    text.replace(['[', ']', ',', '{', '}'], " ")
}

/// Text as a native program's words are read where it may be code (the
/// narrow re-check of the third part, M2'): `\"` is a quote, as Windows's
/// programs read their command line, and lists, maps and blocks are blanks
/// ([`code_words`]). PowerShell, `cmd`, and a line a service is told to
/// start (`Invoke-WmiMethod … 'python -c "…[\"cin\",\"ext\",\"add\"]…"'`)
/// hand a runtime's code on as it is written.
fn native_code_words(text: &str) -> String {
    code_words(&without_casts(&text.replace("\\\"", "\"")))
}

/// `cmd`'s `/c` or `/k` with what follows glued to it (`/ccin`) written as
/// two words.
fn cmd_switches(words: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(words.len() + 1);
    for word in words {
        let lower = word.to_ascii_lowercase();
        if (lower.starts_with("/c") || lower.starts_with("/k")) && word.len() > 2 {
            out.push(word[..2].to_owned());
            out.push(word[2..].to_owned());
        } else {
            out.push(word.clone());
        }
    }
    out
}

/// Whether `words` (a run's) start a session: `-p`, before `--`.
fn headless(words: &[String]) -> bool {
    words.iter().take_while(|word| *word != "--").any(|word| {
        matches!(word.as_str(), "-p" | "--print")
            || word.starts_with("--print=")
            || (word.starts_with('-')
                && !word.starts_with("--")
                && word[1..].starts_with(|c: char| c.is_ascii_alphabetic())
                && word[1..].contains('p'))
    })
}

#[cfg(test)]
thread_local! {
    /// The words the runs of Cineris read on this thread, all checks
    /// together: what a test of the pass's cost counts.
    static LOOKED: Cell<usize> = const { Cell::new(0) };
}

/// One check of one command line.
struct Reader<'a> {
    own: &'a OwnCommands,
    /// How many lines were read.
    work: Cell<usize>,
    /// A variable that moves the stores, set or unset in the line.
    moved: RefCell<Option<String>>,
    /// Whether the line runs Cineris at all.
    runs: Cell<bool>,
    /// Whether the next run found takes more words than are written: what
    /// `xargs` and `parallel` add from their input.
    adds: Cell<bool>,
    /// The words the analysis said are patterns the shell expands: an
    /// ordered set, so that a run's word is looked up in it, not compared
    /// with each, and an empty one costs nothing to ask.
    globs: RefCell<BTreeSet<String>>,
    /// Whether the line being read was modelled: its words are the
    /// program's own arguments, as the shell gives them.
    exact: Cell<bool>,
    /// Whether the line sources a file or `eval`s, before or after.
    sources: Cell<bool>,
    /// Whether the line has changed the directory by now.
    moved_dir: Cell<bool>,
    /// How many simple commands were read.
    steps: Cell<usize>,
    /// Whether the line being read runs what another program of it prints
    /// ([`reads_a_line`]): printers are then no printers.
    fed: Cell<bool>,
    /// How many runs of Cineris were read ([`MENTIONS`]).
    mentions: Cell<usize>,
}

impl<'a> Reader<'a> {
    fn new(own: &'a OwnCommands) -> Self {
        Self {
            own,
            work: Cell::new(0),
            moved: RefCell::new(None),
            runs: Cell::new(false),
            adds: Cell::new(false),
            globs: RefCell::new(BTreeSet::new()),
            exact: Cell::new(false),
            sources: Cell::new(false),
            moved_dir: Cell::new(false),
            steps: Cell::new(0),
            fed: Cell::new(false),
            mentions: Cell::new(0),
        }
    }

    fn is_own(&self, word: &str) -> bool {
        let name = name_of(word);
        !name.is_empty() && (self.own.names.contains(&name) || short_name(&name))
    }

    /// Whether `word`, at a program's place, is a pattern that could match
    /// one of Cineris's names: one with a letter written out, and not
    /// PowerShell's `[Type]::`.
    fn could_be_own(&self, word: &str) -> bool {
        let name = name_of(word);
        if !word.contains(['?', '*', '['])
            || !name.chars().any(char::is_alphabetic)
            || (name.starts_with('[') && name.contains("]::"))
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
        let plain: String = folded(
            &unquoted(text)
                .chars()
                .filter(|c| !matches!(c, '\\' | '\'' | '"' | '`' | '^'))
                .collect::<String>(),
        );
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

    /// Whether the shell expands `word`, or the analysis said it does.
    fn expanded(&self, word: &str) -> bool {
        expands(word) || self.globs.borrow().contains(word)
    }

    /// A run of Cineris with the words `rest`, `open` when more may follow
    /// that are not known; `placed` when it stands at a program's place.
    /// One that does not is refused only where it approves, widens or is
    /// internal: `grep cineris file` runs nothing of Cineris.
    fn run(&self, rest: &[String], open: bool, placed: bool) -> Option<Refusal> {
        // Each run read looks at the words after its name, once: past
        // [`MENTIONS`] runs in one check, or past [`RUN_WORDS`] words in
        // this one, the line is refused, wherever the name stands, before
        // anything is made of them (the narrow re-check of the managing
        // part: a line of many names read the rest of the line again for
        // each).
        let mentions = self.mentions.get() + 1;
        self.mentions.set(mentions);
        if mentions > MENTIONS {
            return Some(Refusal::new("cineris", Kind::Hidden));
        }
        if rest.len() > RUN_WORDS {
            return Some(Refusal::new("cineris", Kind::Hidden));
        }
        #[cfg(test)]
        LOOKED.with(|looked| looked.set(looked.get() + rest.len()));
        let open = open || self.adds.replace(false);
        // PowerShell's `--%` passes what follows as it is, and is not
        // itself an argument; a group's closing brace after the words
        // (`{ cin --version }`) is the group's.
        let mut rest: Vec<String> = rest.iter().filter(|word| *word != "--%").cloned().collect();
        while rest.last().is_some_and(|word| word == "}") {
            rest.pop();
        }
        // The help or the version alone, and nothing after it: that is all
        // the program is given, however the line was read, and it runs
        // nothing (the full review's false refusals).
        if !open
            && let [only] = rest.as_slice()
            && matches!(only.as_str(), "-h" | "--help" | "-V" | "--version")
        {
            return None;
        }
        self.runs.set(true);
        let exact = self.exact.get() && !rest.iter().any(|word| self.expanded(word));
        // Up to the first word the shell expands: what follows it is not
        // known. After a `--`, every word is the prompt's.
        let known = rest.iter().position(|word| self.expanded(word));
        let dashes = rest.iter().position(|word| word == "--");
        let (words, open) = match (known, dashes) {
            (Some(at), Some(dash)) if dash < at => {
                let mut words = rest[..=dash].to_vec();
                words.push("x".to_owned());
                (words, false)
            }
            (_, Some(dash)) if open => {
                let mut words = rest[..=dash].to_vec();
                words.push("x".to_owned());
                (words, false)
            }
            (Some(at), _) => (rest[..at].to_vec(), true),
            (None, _) => (rest, open),
        };
        let refusal = if self.moved_dir.get() && headless(&words) {
            Some(Refusal::new(
                "cineris -p, in another directory",
                Kind::Widens,
            ))
        } else {
            match self.own.standing(&words, open, exact) {
                Standing::Safe => None,
                Standing::Refused(refusal) => Some(refusal),
            }
        };
        refusal
            .filter(|refusal| placed || !matches!(refusal.kind, Kind::Application | Kind::Hidden))
    }

    fn note_moved(&self, words: &[String]) {
        // A shell's own commands that set, unset or export a variable by its
        // name alone: `export -n HOME`, `unset -v HOME`.
        let program = words.get(program_at(words)).map(|first| name_of(first));
        let by_name = program.as_deref().is_some_and(|name| {
            matches!(
                name,
                "export" | "unset" | "declare" | "typeset" | "local" | "readonly" | "setx"
            )
        });
        for (at, word) in words.iter().enumerate() {
            let before = at.checked_sub(1).map(|before| words[before].as_str());
            if let Some(name) = moved(word, before) {
                *self.moved.borrow_mut() = Some(name);
            } else if by_name && moves_stores(word) {
                *self.moved.borrow_mut() = Some(word.to_ascii_uppercase());
            }
        }
    }

    /// What a simple command's program says of the line around it: that it
    /// sources a file, or changes the directory.
    fn note_program(&self, words: &[String], program: usize) {
        let Some(first) = words.get(program) else {
            return;
        };
        let name = name_of(first);
        // `direnv`'s `exec` and `allow` read a project's `.envrc`.
        if SOURCES.contains(&name.as_str()) || first == "." || name == "direnv" {
            self.sources.set(true);
        }
        let env_moves = words[..program]
            .iter()
            .any(|word| matches!(word.as_str(), "-C" | "--chdir") || word.starts_with("--chdir="));
        if CHANGES_DIRECTORY.contains(&name.as_str()) || env_moves {
            self.moved_dir.set(true);
        }
    }

    fn in_parts(&self, parts: &[Part], depth: usize) -> Option<Refusal> {
        let was = self.exact.replace(true);
        let lines: Vec<Vec<String>> = parts
            .iter()
            .map(|part| {
                let mut words = Vec::new();
                for word in &part.words {
                    // A comment the analysis left as a word.
                    if !word.quoted && word.text.starts_with('#') {
                        break;
                    }
                    if word.glob {
                        self.globs.borrow_mut().insert(word.text.clone());
                    }
                    words.push(word.text.clone());
                }
                words
            })
            .collect();
        let fed = self
            .fed
            .replace(lines.iter().any(|words| reads_a_line(words)));
        let found = lines.iter().find_map(|words| self.in_words(words, depth));
        self.fed.set(fed);
        self.exact.set(was);
        found
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
        let steps = self.steps.get() + 1;
        self.steps.set(steps);
        if depth > NESTING || steps > STEPS {
            return Some(Refusal::new("cineris", Kind::Hidden));
        }
        self.note_moved(words);
        if let Some(refusal) = self.env_lines(words, depth) {
            return Some(refusal);
        }
        let program = program_at(words);
        self.note_program(words, program);
        let first = words.get(program)?;
        let name = name_of(first);
        if self.is_own(first) || self.could_be_own(first) {
            return self.run(&words[program + 1..], false, true);
        }
        // A wrapper's command is a simple command of its own; `xargs`
        // adds words to it from its input.
        if let Some(next) = wrapped(&name, words, program) {
            if name == "xargs" {
                self.adds.set(true);
            }
            let found = self.in_words(&words[next..], depth + 1);
            self.adds.set(false);
            return found;
        }
        if matches!(name.as_str(), "find" | "gdb") {
            let mut runs = 0;
            for (at, word) in words.iter().enumerate() {
                if matches!(
                    word.as_str(),
                    "-exec" | "-execdir" | "-ok" | "-okdir" | "--args"
                ) {
                    // A few, as a line has them; past that, the line is
                    // refused rather than read again for each.
                    runs += 1;
                    if runs > MARKS {
                        return Some(Refusal::new("cineris", Kind::Hidden));
                    }
                }
                if matches!(
                    word.as_str(),
                    "-exec" | "-execdir" | "-ok" | "-okdir" | "--args"
                ) {
                    let end = words[at + 1..]
                        .iter()
                        .position(|word| matches!(word.as_str(), ";" | "+" | "\\;"))
                        .map_or(words.len(), |end| at + 1 + end);
                    if let Some(refusal) = self.in_words(&words[at + 1..end], depth + 1) {
                        return Some(refusal);
                    }
                }
            }
        }
        let runs_cargo = name == "cargo" && words[program + 1..].iter().any(|word| word == "run");
        if runs_cargo && let Some(refusal) = self.cargo_run(&words[program + 1..]) {
            return Some(refusal);
        }
        // PowerShell's `Start-Process` names its program and its arguments
        // by its own parameters.
        if matches!(
            name.as_str(),
            "start-process" | "saps" | "start" | "new-scheduledtaskaction"
        ) {
            return self.start_process(&words[program + 1..], depth);
        }
        let terminal = TERMINALS.contains(&name.as_str());
        let prints = (PRINTERS.contains(&name.as_str()) || (name == "cargo" && !runs_cargo))
            && !self.fed.get();
        if name == "git"
            && let Some(refusal) = self.git_lines(words, program, depth)
        {
            return Some(refusal);
        }
        if matches!(
            name.as_str(),
            "tmux" | "screen" | "xdotool" | "ydotool" | "wtype"
        ) {
            let keys = typed(&name, &words[program + 1..]);
            if self.mentioned(&keys)
                && let Some(refusal) = self.read(&keys, Line::Shell, depth)
            {
                return Some(refusal);
            }
        }
        if name == "parallel" {
            self.adds.set(true);
        }
        let awk = AWKS
            .contains(&name.as_str())
            .then(|| awk_program(words, program))
            .flatten();
        for at in program + 1..words.len() {
            let word = &words[at];
            // A name of Cineris at any other word: what its program may run.
            if !prints
                && self.is_own(word)
                && let Some(refusal) = self.run(&words[at + 1..], false, terminal)
            {
                return Some(refusal);
            }
            let line = line_at(words, program, at).or_else(|| {
                let positional = !word.starts_with('-');
                let powershell = matches!(name.as_str(), "pwsh" | "powershell");
                // A word with a blank in it may be a line the program runs
                // (`sed`'s `e`, a runner's): read as one, but a printer's,
                // and a `sed` script's that runs no line.
                let blank = !prints
                    && word.contains(char::is_whitespace)
                    && (name != "sed" || self.fed.get() || sed_runs(word));
                if ((terminal || blank) && word.contains(char::is_whitespace))
                    || (powershell && positional)
                {
                    Some(Line::Shell)
                } else if awk == Some(at) {
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
            // An editor's `+command`.
            if VIMS.contains(&name.as_str())
                && let Some(command) = word.strip_prefix('+')
                && self.mentioned(command)
                && let Some(refusal) = self.read(command, Line::Code, depth)
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
                        if let Some(refusal) = self.run(rest, false, terminal) {
                            return Some(refusal);
                        }
                    }
                }
                for (count, (mark, _)) in word.match_indices(['=', '!']).enumerate() {
                    if count >= MARKS {
                        return Some(Refusal::new("cineris", Kind::Hidden));
                    }
                    let text = &word[mark + 1..];
                    if self.mentioned(text)
                        && let Some(refusal) = self.read(text, Line::Shell, depth)
                    {
                        return Some(refusal);
                    }
                }
            }
        }
        self.adds.set(false);
        let switches = if name == "cmd" {
            cmd_switches(words)
        } else {
            Vec::new()
        };
        if let Some(flag) = switches
            .iter()
            .position(|word| matches!(word.to_ascii_lowercase().as_str(), "/c" | "/k"))
        {
            let rest = switches[flag + 1..].join(" ");
            if let Some(refusal) = self.in_text(&rest, ShellSyntax::Cmd) {
                return Some(refusal);
            }
        }
        None
    }

    /// The lines `git` is told to run by its configuration: the value of a
    /// key that is a line ([`GIT_LINES`]) or of an `alias.*` that begins
    /// with `!`, given apart (`git config core.pager LINE`) or after `=`
    /// (`git -c core.pager=LINE`). `git` otherwise only prints its words:
    /// a commit's message is no line.
    fn git_lines(&self, words: &[String], program: usize, depth: usize) -> Option<Refusal> {
        for (at, word) in words.iter().enumerate().skip(program + 1) {
            let (key, attached) = match word.split_once('=') {
                Some((key, value)) => (key.to_ascii_lowercase(), Some(value)),
                None => (word.to_ascii_lowercase(), None),
            };
            let alias = key.starts_with("alias.");
            if !alias && !GIT_LINES.contains(&key.as_str()) {
                continue;
            }
            let Some(value) = attached.or_else(|| words.get(at + 1).map(String::as_str)) else {
                continue;
            };
            let line = if alias {
                match value.strip_prefix('!') {
                    Some(line) => line,
                    None => continue,
                }
            } else {
                value
            };
            if self.mentioned(line)
                && let Some(refusal) = self.read(line, Line::Shell, depth)
            {
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
        self.run(rest, false, true)
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
            if matches!(lower.as_str(), "-filepath" | "-file" | "-path" | "-execute") {
                program = words.get(at + 1).cloned();
                at += 2;
                continue;
            }
            if matches!(
                lower.as_str(),
                "-argumentlist" | "-args" | "-arguments" | "-argument"
            ) {
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
        // Its argument list is one string the program reads as it will:
        // the words here are not taken as the program's own.
        let was = self.exact.replace(false);
        let mut line = vec![program];
        for argument in &arguments {
            line.extend(
                argument
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .filter(|piece| !piece.is_empty())
                    .map(str::to_owned),
            );
        }
        let found = self.in_words(&line, depth + 1);
        self.exact.set(was);
        found
    }

    /// A line a word gives.
    fn read(&self, text: &str, line: Line, depth: usize) -> Option<Refusal> {
        if depth >= DEPTH || self.spent() {
            return Some(Refusal::new("cineris", Kind::Hidden));
        }
        match line {
            Line::Code => self.in_text(&code_words(text), ShellSyntax::Posix),
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
                if !modelled {
                    return self.in_text(text, ShellSyntax::Unknown);
                }
                // What `printf` or `echo -e` prints of it: `\n` and `\t` are a
                // line break and a tab between words.
                if text.contains("\\n") || text.contains("\\t") {
                    let printed = text.replace("\\n", "\n").replace("\\t", "\t");
                    return self.in_text(&printed, ShellSyntax::Unknown);
                }
                None
            }
        }
    }

    /// A line the analysis cannot model, read as text, failing closed.
    fn in_text(&self, text: &str, syntax: ShellSyntax) -> Option<Refusal> {
        let posix = matches!(syntax, ShellSyntax::Posix | ShellSyntax::Unknown);
        // A brace's word that names Cineris: `{cin,} ext add`.
        if posix && braced_name(&plain(text, "\\'\"`"), |word| self.is_own(word)) {
            return Some(Refusal::new("cineris", Kind::Hidden));
        }
        self.note_set_by_dotnet(text);
        let was = self.exact.replace(false);
        let written = unquoted(text);
        let text = braced(&written);
        let mut found = match syntax {
            ShellSyntax::Posix => self.in_plain(&plain(&text, "\\'\"`")),
            ShellSyntax::PowerShell => self.in_plain(&plain(&text, "'\"`")),
            ShellSyntax::Cmd => self.in_plain(&plain(&text, "\"^")),
            ShellSyntax::Unknown => self
                .in_plain(&plain(&text, "\\'\"`^"))
                .or_else(|| self.in_plain(&plain(&text, "'\"`^"))),
        };
        // PowerShell's arrays and groups are a native program's words:
        // `cin ext @('add','C:\x')` runs `cin ext add C:\x`.
        if found.is_none() && matches!(syntax, ShellSyntax::PowerShell | ShellSyntax::Unknown) {
            let arrays = without_casts(&plain(&text, "'\"`"))
                .replace("@(", " ")
                .replace(['(', ')', ','], " ");
            found = self.in_plain(&arrays);
        }
        // `printf`'s and `$'…'`'s escapes, read as what they print: a line
        // break or a tab between words.
        if found.is_none() && posix && text.contains(['\\']) {
            let escapes = text
                .replace("\\n", "\n")
                .replace("\\t", "\t")
                .replace("\\r", "\n");
            if escapes != text {
                found = self.in_plain(&plain(&escapes, "\\'\"`"));
            }
        }
        // The words a native program is handed, read as a runtime's code
        // reads them (the narrow re-check of the third part, M2'): `python
        // -c '…run(["cin","ext","add"])'` in PowerShell, which does not
        // model it, or `wmic process call create "python -c \"…['cin','ext',
        // 'add']…\""`. A `\` stays a separator but in POSIX.
        if found.is_none() && written.contains(['[', ',', '{', '"']) {
            let quotes = if matches!(syntax, ShellSyntax::Posix) {
                "\\'\"`"
            } else {
                "'\"`^"
            };
            found = self.in_plain(&plain(&native_code_words(&written), quotes));
        }
        self.exact.set(was);
        found
    }

    /// A store's variable set by .NET in a PowerShell line:
    /// `[Environment]::SetEnvironmentVariable('HOME', …)`.
    fn note_set_by_dotnet(&self, text: &str) {
        let upper = text.to_ascii_uppercase();
        let Some(at) = upper.find("SETENVIRONMENTVARIABLE(") else {
            return;
        };
        let name: String = upper[at + "SETENVIRONMENTVARIABLE(".len()..]
            .trim_start_matches([' ', '\'', '"'])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if moves_stores(&name) {
            *self.moved.borrow_mut() = Some(name);
        }
    }

    /// Text with its quotes and escapes out.
    fn in_plain(&self, text: &str) -> Option<Refusal> {
        let segments = segments(text);
        let mut named = false;
        let fed = self.fed.get() || segments.iter().any(|words| reads_a_line(words));
        for words in &segments {
            let switched;
            let words = if words
                .get(program_at(words))
                .is_some_and(|first| name_of(first) == "cmd")
            {
                switched = cmd_switches(words);
                &switched
            } else {
                words
            };
            self.note_moved(words);
            let program = program_at(words);
            self.note_program(words, program);
            // `Start-Process` by its parameters.
            if words.get(program).is_some_and(|first| {
                matches!(
                    name_of(first).as_str(),
                    "start-process" | "saps" | "start" | "new-scheduledtaskaction"
                )
            }) && let Some(refusal) = self.start_process(&words[program + 1..], DEPTH - 1)
            {
                return Some(refusal);
            }
            let first = words.get(program).map(|first| name_of(first));
            let first = first.as_deref().unwrap_or("");
            // At the program's place, or anywhere after a program that runs
            // what its words say, a run counts whatever it is; anywhere
            // else, where it approves, widens or is internal.
            let describes =
                first == "command" && words.iter().any(|word| word == "-v" || word == "-V");
            let kind = kind_of(first);
            let runs_words = !describes
                && (TERMINALS.contains(&first)
                    || SHELLS.contains(&kind)
                    || AWKS.contains(&first)
                    || matches!(first, "cmd" | "pwsh" | "powershell" | "env")
                    || INTERPRETERS
                        .iter()
                        .any(|(interpreter, _)| *interpreter == kind)
                    || WRAPPERS.iter().any(|(wrapper, _, _)| *wrapper == first));
            let prints = PRINTERS.contains(&first) && !fed;
            if matches!(first, "xargs" | "parallel") {
                self.adds.set(true);
            }
            for (at, word) in words.iter().enumerate() {
                let executes = at.checked_sub(1).is_some_and(|before| {
                    matches!(
                        words[before].as_str(),
                        "-exec" | "-execdir" | "-ok" | "-okdir" | "--args"
                    )
                });
                let placed = at == program || executes || (runs_words && at > program);
                // A name of Cineris the shell completes: `cin${IFS}ext`.
                if placed && names_through_expansion(word, |piece| self.is_own(piece)) {
                    return Some(Refusal::new("cineris", Kind::Hidden));
                }
                if word.contains(['=', '!']) {
                    for (count, (mark, _)) in word.match_indices(['=', '!']).enumerate() {
                        if count >= MARKS {
                            return Some(Refusal::new("cineris", Kind::Hidden));
                        }
                        let text = &word[mark + 1..];
                        if self.mentioned(text)
                            && let Some(refusal) = self.read(text, Line::Shell, DEPTH - 2)
                        {
                            return Some(refusal);
                        }
                    }
                }
                let own = (self.is_own(word) || (at == program && self.could_be_own(word)))
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
                    if prints && at != program {
                        continue;
                    }
                    named = true;
                    let rest = &words[from.min(words.len())..];
                    if let Some(refusal) = self.run(rest, expands(word), placed) {
                        return Some(refusal);
                    }
                    if let Some(refusal) = after_a_name(rest) {
                        return Some(refusal);
                    }
                }
            }
            self.adds.set(false);
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
            if let Some(topic) = all.iter().find(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "setup" | "serve" | "attach"
                )
            }) {
                return Some(Refusal::new(
                    format!("cineris {}", topic.to_ascii_lowercase()),
                    Kind::Approves,
                ));
            }
            let widening = all.iter().find(|word| {
                let flag = word.split('=').next().unwrap_or_default();
                matches!(
                    flag,
                    "--trust-project"
                        | "--allow"
                        | "--profile"
                        | "--continue"
                        | "--resume"
                        | "--fork"
                )
            });
            if let Some(flag) = widening
                && headless(&all)
            {
                return Some(Refusal::new(format!("cineris -p {flag}"), Kind::Widens));
            }
        }
        None
    }
}

/// What the words after a name of Cineris, in a line read as text, hold
/// beyond what the shell expands: `setup`, `serve` or `attach` past a word
/// it expands, or a flag that widens a headless run anywhere.
fn after_a_name(rest: &[String]) -> Option<Refusal> {
    let expanded = rest.iter().position(|word| expands(word));
    // Asked once, not again for each flag that widens.
    let headless = headless(rest);
    for (at, word) in rest.iter().enumerate() {
        let lower = word.to_ascii_lowercase();
        if expanded.is_some_and(|expanded| expanded < at)
            && matches!(lower.as_str(), "setup" | "serve" | "attach")
        {
            return Some(Refusal::new(format!("cineris {lower}"), Kind::Approves));
        }
        let flag = lower.split('=').next().unwrap_or_default();
        let widens = matches!(
            flag,
            "--trust-project" | "--allow" | "--profile" | "--continue" | "--resume" | "--fork"
        ) || (flag == "--permission-mode"
            && !lower.ends_with("=read-only")
            && !lower.ends_with("=supervised")
            && !rest
                .get(at + 1)
                .is_some_and(|mode| matches!(mode.as_str(), "read-only" | "supervised")));
        if widens && headless {
            return Some(Refusal::new(format!("cineris -p {flag}"), Kind::Widens));
        }
    }
    None
}

/// Text split into simple commands, each into its words: at `;`, `&`,
/// `|`, a line break or a parenthesis; a comment (`#` at a word's start, to
/// the end of its line, and PowerShell's `<#…#>`) and the target of an
/// output's redirection are left out.
fn segments(text: &str) -> Vec<Vec<String>> {
    // PowerShell's block comments, taken out in one pass.
    let mut uncommented = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<#") {
        uncommented.push_str(&rest[..start]);
        uncommented.push(' ');
        rest = rest[start..]
            .find("#>")
            .map_or("", |end| &rest[start + end + 2..]);
    }
    uncommented.push_str(rest);
    let text = uncommented;
    let mut segments = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut target = false;
    let mut comment = false;
    let finish = |word: &mut String, words: &mut Vec<String>, target: &mut bool| {
        if !word.is_empty() {
            if *target {
                *target = false;
            } else {
                words.push(std::mem::take(word));
            }
            word.clear();
        }
    };
    for character in text.chars() {
        if comment {
            if character == '\n' {
                comment = false;
                finish(&mut word, &mut words, &mut target);
                segments.push(std::mem::take(&mut words));
            }
            continue;
        }
        match character {
            '#' if word.is_empty() => comment = true,
            '\n' | ';' | '&' | '|' | '(' | ')' => {
                finish(&mut word, &mut words, &mut target);
                segments.push(std::mem::take(&mut words));
            }
            '>' => {
                // A descriptor's number before it is no word.
                if word.bytes().all(|byte| byte.is_ascii_digit()) {
                    word.clear();
                }
                finish(&mut word, &mut words, &mut target);
                target = true;
            }
            '<' => finish(&mut word, &mut words, &mut target),
            blank if blank.is_whitespace() => finish(&mut word, &mut words, &mut target),
            other => word.push(other),
        }
    }
    finish(&mut word, &mut words, &mut target);
    segments.push(words);
    segments
}

/// `text` with each brace expansion (`{add,}`, `{a..b}`) written as a word
/// the shell expands, so that it is not split at its braces.
fn braced(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    for (open, close) in brace_expansions(text) {
        out.push_str(&text[from..open]);
        out.push_str("$brace");
        from = close + 1;
    }
    out.push_str(&text[from..]);
    out
}

/// `text` with the quoting that builds nothing taken out: `$'…'` and
/// `$"…"` read as quotes, and an empty `$()` as nothing.
fn unquoted(text: &str) -> String {
    text.replace("$()", "")
        .replace("$'", "'")
        .replace("$\"", "\"")
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
    // A line with no line break has no continuation: most are passed over
    // here, before anything is made of them. The join is one pass, and the
    // line it makes is read once, beside the line as written (the narrow
    // re-check of the third part, M1').
    let joined = command
        .contains('\n')
        .then(|| joined(command, shell))
        .filter(|joined| joined != command);
    if let Some(joined) = &joined
        && let Some(refusal) = read_line(joined, shell, own)
    {
        return Some(refusal);
    }
    read_line(command, shell, own)
}

/// One line, as written or with its continuations joined: what of it is
/// refused.
fn read_line(command: &str, shell: ShellSyntax, own: &OwnCommands) -> Option<Refusal> {
    let reader = Reader::new(own);
    let typing = ["tmux", "screen", "xdotool", "ydotool", "wtype"]
        .iter()
        .any(|typer| command.contains(typer));
    let names = |text: &str| {
        reader.mentioned(text)
            || (typing && reader.mentioned(&text.replace(char::is_whitespace, "")))
    };
    // Past the longest line read, nothing more than whether it names
    // Cineris is asked of it: each of those looks is one pass.
    if command.len() > LONGEST {
        return names(command).then(|| Refusal::new("cineris", Kind::Hidden));
    }
    if !names(command) {
        return None;
    }
    // A carriage return ends a line in PowerShell and `cmd`, and the
    // analysis is not trusted to say so: it ends one here everywhere.
    let command = command.replace('\r', "\n");
    let refusal = match analyse(&command, shell) {
        Analysis::Parts(parts) => reader.in_parts(&parts, 0),
        Analysis::NotModelled { .. } => reader.in_text(&command, shell),
    };
    refusal.or_else(|| {
        if !reader.runs.get() {
            return None;
        }
        if reader.sources.get() {
            return Some(Refusal::new(
                "cineris, in a line that sources a file or evaluates one",
                Kind::Widens,
            ));
        }
        let moved = reader.moved.borrow().clone()?;
        Some(Refusal::new(
            format!("cineris with {moved} set"),
            Kind::Widens,
        ))
    })
}

#[cfg(test)]
mod tests;
