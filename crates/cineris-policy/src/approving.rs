//! The harness's own approving commands, refused to every command at every
//! level, `unrestricted` included, with no rule able to lift them (since
//! step 28's third pull request, the owner's answer: where a command is not
//! confined, the model could otherwise approve an extension, or trust a
//! project, for itself).
//!
//! **What is refused:** a line that runs Cineris (`cineris`, `cin`, or the
//! running binary by its name, with any directory before it, in any case,
//! with or without `.exe`) with one of the subcommands that approve, trust
//! or answer for the person:
//!
//! - `ext add`, `ext remove`, `ext disable`: they write `pins.toml`, the
//!   registration cache, the user's folder of extensions and `[extensions]`;
//! - `permissions trust`, `untrust`, `revoke` and `forget`: they write
//!   `trust.toml` and the stored approvals;
//! - `setup`: it writes the user's own `permissions.read_skill_folders`;
//! - `serve`, over either transport, and `attach`: a client of a server
//!   answers its `trust/request` and its approvals;
//! - a run with no subcommand that is neither headless (`-p`) nor a call for
//!   the help or the version: the interactive application answers a
//!   project's trust question, and a command can give it a terminal
//!   (`script -c cineris`).
//!
//! **However the line names it.** The line is read by the engine's own
//! analysis, segment by segment (`&&`, `||`, `;`, `|`, a subshell, a line
//! break), and in each segment Cineris is looked for at the program's place,
//! past `env` (its options and its `NAME=value` words) and the wrappers that
//! run what follows them (`nohup`, `timeout`, `xargs`, `sudo`…), and at any
//! other word too, followed by an approving subcommand: so that `strace
//! cineris ext add x` is refused as `cineris ext add x` is. A word that is a
//! command line of its own (what `sh -c`, `bash -c`, `pwsh -c`,
//! `powershell -Command`, `cmd /c`, `script -c` or `su -c` run) is read as a
//! line again, in both shells' syntax. The subcommand is found past the
//! global options. **A line the analysis cannot model fails closed:** Cineris
//! named anywhere in its words, with one of the approving subcommands' names
//! after it, refuses it.
//!
//! **What it does not reach:** a program that runs the binary some other way
//! (`cargo run -p cineris-cli -- ext add x`, a script file, a copy under
//! another name, an encoded PowerShell command). The approving subcommands
//! look at their own line of parents, or the harness's job on Windows, and
//! refuse to approve under a session's command: that is the backstop.

use std::sync::OnceLock;

use crate::command::{Analysis, Part, ShellSyntax, analyse};

/// What a refused line is told: the person runs it themselves.
pub(crate) fn reason(what: &str) -> String {
    format!(
        "`{what}` approves extensions, trusts projects or answers for you, so no command of a \
         session may run it, at any level and whatever the rules say: run it in your own \
         terminal."
    )
}

/// How deep a command line inside a word is read again.
const DEPTH: usize = 4;

/// The names Cineris runs under: its own two, and the running binary's.
fn own_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = vec!["cineris".to_owned(), "cin".to_owned()];
        if let Some(stem) = std::env::current_exe().ok().and_then(|exe| {
            exe.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        }) {
            let stem = program_name(&stem);
            if !stem.is_empty() && !names.contains(&stem) {
                names.push(stem);
            }
        }
        names
    })
}

/// The name of the program a word names: its last name, after any `/` or
/// `\`, in lower case, without a `.exe`.
fn program_name(word: &str) -> String {
    let last = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let lower = last.to_ascii_lowercase();
    lower
        .strip_suffix(".exe")
        .map_or_else(|| lower.clone(), str::to_owned)
}

fn is_own(word: &str) -> bool {
    let name = program_name(word);
    own_names().contains(&name)
}

/// Whether `word` names Cineris by one of its own two names: the
/// application's run is told by them alone, since the running binary under
/// another name (a test's binary, which runs itself as a hook's program) is
/// refused only with an approving subcommand.
fn is_named(word: &str) -> bool {
    matches!(program_name(word).as_str(), "cineris" | "cin")
}

/// The subcommands that approve, trust or answer, by the subcommand's word
/// and, where only some of its actions do, those.
const APPROVING: [(&str, Option<&[&str]>); 5] = [
    ("ext", Some(&["add", "remove", "disable"])),
    (
        "permissions",
        Some(&["trust", "untrust", "revoke", "forget"]),
    ),
    ("setup", None),
    ("serve", None),
    ("attach", None),
];

/// Every subcommand's word: a run with none of them is the application,
/// unless it is headless or asks for the help or the version.
const SUBCOMMANDS: [&str; 12] = [
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

/// The approving subcommand `rest` (the words after Cineris's name) runs,
/// in words, when it runs one: the subcommand found past the options, and
/// its action past them too.
fn approving(rest: &[String]) -> Option<String> {
    let plain: Vec<&String> = rest.iter().filter(|word| !word.starts_with('-')).collect();
    for (at, word) in plain.iter().enumerate() {
        let word = word.to_ascii_lowercase();
        if let Some((subcommand, actions)) = APPROVING.iter().find(|(name, _)| *name == word) {
            match actions {
                None => return Some(format!("cineris {subcommand}")),
                Some(actions) => {
                    if let Some(action) =
                        plain.get(at + 1).map(|action| action.to_ascii_lowercase())
                        && actions.contains(&action.as_str())
                    {
                        return Some(format!("cineris {subcommand} {action}"));
                    }
                }
            }
        }
    }
    None
}

/// Whether `rest` is a run of the interactive application: no subcommand,
/// and neither headless nor a call for the help or the version.
fn interactive(rest: &[String]) -> bool {
    let named = rest
        .iter()
        .any(|word| SUBCOMMANDS.contains(&word.to_ascii_lowercase().as_str()));
    let other = rest.iter().any(|word| {
        matches!(
            word.as_str(),
            "-p" | "--print" | "-h" | "--help" | "-V" | "--version"
        ) || word.starts_with("--print=")
    });
    !named && !other
}

/// Programs that run the words after them as a command.
const WRAPPERS: [&str; 16] = [
    "nohup", "nice", "ionice", "time", "timeout", "stdbuf", "setsid", "exec", "command", "builtin",
    "xargs", "sudo", "doas", "chrt", "taskset", "unbuffer",
];

/// Where the program of `words` is, past `env` and the wrappers.
fn program_at(words: &[String]) -> usize {
    let mut at = 0;
    while let Some(word) = words.get(at) {
        let name = program_name(word);
        if name == "env" {
            at += 1;
            while let Some(word) = words.get(at) {
                if matches!(
                    word.as_str(),
                    "-u" | "--unset" | "-C" | "--chdir" | "-S" | "--split-string"
                ) {
                    at += 2;
                } else if word.starts_with('-') || word.contains('=') {
                    at += 1;
                } else {
                    break;
                }
            }
        } else if WRAPPERS.contains(&name.as_str()) {
            at += 1;
            while words.get(at).is_some_and(|word| word.starts_with('-')) {
                at += 1;
            }
            // `timeout`'s duration, `nice`'s level given apart.
            if name == "timeout"
                && words
                    .get(at)
                    .is_some_and(|word| word.starts_with(|c: char| c.is_ascii_digit()))
            {
                at += 1;
            }
        } else {
            break;
        }
    }
    at
}

/// Whether `flag` comes before a word that is a command line of its own.
fn runs_a_line(flag: &str) -> bool {
    matches!(
        flag.to_ascii_lowercase().as_str(),
        "-c" | "/c" | "/k" | "-command" | "--command" | "-cmd"
    )
}

/// What approving command `words`, one simple command, runs, if one.
fn in_words(words: &[String], depth: usize) -> Option<String> {
    let program = program_at(words);
    for (at, word) in words.iter().enumerate() {
        if is_own(word) {
            let rest = &words[at + 1..];
            if let Some(what) = approving(rest) {
                return Some(what);
            }
            if at == program && is_named(word) && interactive(rest) {
                return Some("cineris".to_owned());
            }
        }
        // A command line inside the word: what a shell, `cmd`, `script` or
        // `su` is told to run, or any word with a blank in it.
        let inner = (at > 0 && runs_a_line(&words[at - 1]))
            || word.contains(char::is_whitespace)
            || word.contains([';', '&', '|']);
        if inner
            && depth < DEPTH
            && let Some(what) = in_line(word, depth + 1)
        {
            return Some(what);
        }
    }
    // `cmd /c` and `cmd /k` run the rest of the line, as `cmd` reads it,
    // which the analysis does not model: read as one line, failing closed.
    if program_name(words.get(program).map_or("", String::as_str)) == "cmd"
        && let Some(flag) = words.iter().position(|word| runs_a_line(word))
        && depth < DEPTH
    {
        let rest = words[flag + 1..].join(" ");
        if let Some(what) = in_line(&rest, depth + 1) {
            return Some(what);
        }
    }
    None
}

fn in_parts(parts: &[Part], depth: usize) -> Option<String> {
    parts.iter().find_map(|part| {
        let words: Vec<String> = part.words.iter().map(|word| word.text.clone()).collect();
        in_words(&words, depth)
    })
}

/// A line read in both shells' syntax, and as text where neither models it.
fn in_line(line: &str, depth: usize) -> Option<String> {
    let mut modelled = false;
    for syntax in [ShellSyntax::Posix, ShellSyntax::PowerShell] {
        if let Analysis::Parts(parts) = analyse(line, syntax) {
            modelled = true;
            if let Some(what) = in_parts(&parts, depth) {
                return Some(what);
            }
        }
    }
    if modelled { None } else { in_text(line) }
}

/// A line the analysis cannot model, failing closed: Cineris named in any
/// of its words, with an approving subcommand's name after it.
fn in_text(line: &str) -> Option<String> {
    let words: Vec<String> = line
        .split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    ';' | '&'
                        | '|'
                        | '('
                        | ')'
                        | '"'
                        | '\''
                        | '`'
                        | '<'
                        | '>'
                        | '$'
                        | '{'
                        | '}'
                        | '='
                )
        })
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect();
    let first = words.iter().position(|word| is_own(word))?;
    let rest = &words[first + 1..];
    // At the line's start, or where a shell or `script` is told to run it:
    // the application, unless it is headless or asks for the help.
    let at_start = first == 0 || runs_a_line(&words[first - 1]);
    if at_start && is_named(&words[first]) && interactive(rest) {
        return Some("cineris".to_owned());
    }
    approving(rest).or_else(|| {
        rest.iter()
            .find(|word| {
                APPROVING
                    .iter()
                    .any(|(name, _)| word.eq_ignore_ascii_case(name))
            })
            .map(|word| format!("cineris {}", word.to_ascii_lowercase()))
    })
}

/// The approving command `command` runs, in the session's shell `shell`,
/// when it runs one: what the engine refuses at every level.
pub(crate) fn refused(command: &str, shell: ShellSyntax) -> Option<String> {
    match analyse(command, shell) {
        Analysis::Parts(parts) => in_parts(&parts, 0),
        Analysis::NotModelled { .. } => in_text(command),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every form a line can name an approving command in is refused, in
    /// the shells' syntax it is written for; and what only reads, or names
    /// Cineris in passing, is not.
    #[test]
    fn every_way_a_line_names_an_approving_command_is_refused() {
        let posix = [
            "cineris ext add ./x",
            "cin ext remove x",
            "cineris ext disable x",
            "/usr/local/bin/cineris permissions trust",
            "./cineris permissions untrust",
            "../bin/cin permissions revoke a_1",
            "cineris permissions forget",
            "CINERIS ext add x",
            "cineris.exe ext add x",
            "'cineris' 'ext' 'add' x",
            "\"/opt/c i/cineris\" ext add x",
            "env cineris ext add x",
            "env -i FOO=1 -u HOME cineris ext add x",
            "env -- cin serve --stdio",
            "sh -c 'cineris ext add x'",
            "bash -c \"echo hi && cin permissions trust\"",
            "ls; cineris ext add x",
            "true && cineris ext add x",
            "false || cineris ext add x",
            "echo y | cineris ext add x",
            "(cineris ext add x)",
            "echo a\ncineris ext add x",
            "cineris --profile p ext add x",
            "cineris -c x=1 permissions trust",
            "cineris serve --socket",
            "cineris serve --stdio",
            "cineris attach",
            "cineris setup skill-folders on",
            "cineris",
            "cin some prompt words",
            "script -c cineris /dev/null",
            "nohup cineris ext add x &",
            "timeout 5 cineris ext add x",
            "xargs cineris ext add",
            "strace -f cineris ext add x",
            "cineris ext add $HOME/x",
            "cineris ext add \"$(pwd)\"",
            "x=$(cineris ext add y)",
        ];
        let powershell = [
            "cineris ext add x",
            "& 'C:\\Tools\\cineris.exe' ext add x",
            "& \"C:\\Program Files\\Cineris\\CIN.EXE\" permissions trust",
            "pwsh -c 'cineris ext add x'",
            "powershell -Command \"cin permissions trust\"",
            "cmd /c cineris ext add x",
            "cmd /k \"cin permissions untrust\"",
            "ls; cineris ext add x",
            "Get-Item x | cineris ext add y",
            ".\\cineris.exe ext remove x",
            "..\\cin ext disable x",
        ];
        for (shell, lines) in [
            (ShellSyntax::Posix, &posix[..]),
            (ShellSyntax::PowerShell, &powershell[..]),
            (ShellSyntax::Cmd, &powershell[..]),
            (ShellSyntax::Unknown, &posix[..]),
        ] {
            for line in lines {
                assert!(
                    refused(line, shell).is_some(),
                    "{line:?} in {shell:?} was not refused"
                );
            }
        }
        for line in [
            "cineris ext list",
            "cineris ext inspect tracker",
            "cineris sessions list",
            "cineris -p hello",
            "cineris --version",
            "cin --help",
            "grep cineris notes.txt",
            "cd ~/Coding/Cineris && cargo test",
            "cargo run -p cineris-cli -- --help",
            "git log --grep=trust",
            "ls ext add",
        ] {
            for shell in [ShellSyntax::Posix, ShellSyntax::PowerShell] {
                assert_eq!(refused(line, shell), None, "{line:?} in {shell:?}");
            }
        }
        // The reason says to run it in one's own terminal.
        let what = refused("cin ext add x", ShellSyntax::Posix).unwrap();
        assert!(reason(&what).contains("run it in your own terminal"));
    }
}
