//! A program an extension's `cineris.exec` names, or a hook command's
//! first word (`docs/steps/28/answered.md`, "The processes"): found, written
//! out for the permission engine, and made into the command line the
//! runner starts.
//!
//! - **Found** ([`find`]). A bare name in the absolute directories of
//!   `PATH`, never in the workspace or a write root of the session or of
//!   the extension, where a command could have put one
//!   (`cineris_base::program::find_runnable_outside`). On Windows a
//!   directory at a time, as `name.exe`, `name.cmd`, `name.bat`; nothing
//!   else of `PATHEXT` is ever run. An absolute path is taken as it is
//!   named, except inside a write root while commands are confined.
//! - **Written for the engine** ([`engine_line`]). The program as named,
//!   and each argument quoted in the session shell's syntax, so the
//!   command analysis reads the words the program is given. An argument
//!   the shell cannot quote refuses the call, naming it.
//! - **Started** ([`command_line`]). A program as it is, its arguments one
//!   by one. On Windows a `.cmd` or `.bat` is started through
//!   `%SystemRoot%\System32\cmd.exe` with one line of its own
//!   ([`batch_line`]): the script's path and each argument in double
//!   quotes, inside which `cmd.exe` takes `^ & | < > ( )` as plain
//!   characters; a run of backslashes before each closing quote doubled,
//!   so that the program the script starts reads the argument as it was
//!   given; and `"`, `%`, `!`, a carriage return, a line feed and a NUL
//!   refused, naming the argument and the character, since `cmd.exe` has
//!   no way to carry them unchanged.

use std::path::{Path, PathBuf};

use cineris_ext::manifest::Program;
use cineris_tools::process::CommandLine;
use cineris_tools::{Shell, ShellKind};

/// The most characters of a line `cmd.exe` reads, less room for the
/// program and its own arguments: as the shell tool holds a `cmd.exe`
/// command to.
const BATCH_LINE_CHARS: usize = 8_000;

/// The arguments `cmd.exe` is started with before a script's line: no
/// `AutoRun`, command extensions on (the shims package managers write use
/// `%~dp0` and `call :label`), delayed expansion off, and `/s /c` to take
/// the outer pair of quotes off the line and run it.
pub(crate) const CMD_ARGUMENTS: [&str; 5] = ["/d", "/e:on", "/v:off", "/s", "/c"];

/// Where a program is found from: the directories of `PATH`, and the
/// places it may not be taken from.
#[derive(Debug, Clone)]
pub(crate) struct Search<'a> {
    /// The directories of the `PATH` programs inherit, in order.
    pub(crate) path: &'a [PathBuf],
    /// Where a program found by name is never taken from: the workspace,
    /// the session's write roots while it would confine, and the
    /// extension's own write roots.
    pub(crate) avoid: &'a [PathBuf],
    /// Where a program named by its path is refused: the write roots of
    /// the session and the extension, while commands are confined. Empty
    /// otherwise.
    pub(crate) confined_roots: &'a [PathBuf],
    /// Whether to find it by Windows's rules.
    pub(crate) windows: bool,
}

/// The program `program` names, found by `search`'s rules. Blocks: it
/// looks at the file system.
///
/// # Errors
///
/// A message naming the program, when it is not found, is not one these
/// rules start, or lies inside a write root while commands are confined.
pub(crate) fn find(program: &Program, search: &Search<'_>) -> Result<PathBuf, String> {
    match program {
        Program::Name(name) => cineris_base::program::find_runnable_outside(
            search.path,
            name,
            search.windows,
            search.avoid,
        )
        .ok_or_else(|| {
            let kinds = if search.windows {
                " (as a .exe, .cmd or .bat)"
            } else {
                ""
            };
            format!(
                "`{name}` was not found{kinds} in the absolute directories of PATH outside the \
                 workspace and the writable roots"
            )
        }),
        Program::Path(path) => {
            let named = Path::new(path);
            if !named.is_absolute() {
                return Err(format!(
                    "`{path}` is not an absolute path on this system, so it is not started"
                ));
            }
            if !cineris_base::program::runnable_by_extension(named, search.windows) {
                return Err(format!(
                    "`{path}` is not started: on Windows a program is a .exe, .cmd or .bat"
                ));
            }
            if !cineris_base::program::is_program(named, search.windows) {
                return Err(format!("`{path}` was not found, or is not a program"));
            }
            let roots = cineris_base::program::resolved(search.confined_roots);
            if !roots.is_empty() && cineris_base::program::passes_through(named, &roots) {
                return Err(format!(
                    "`{path}` is inside a writable root, where a confined command could have \
                     changed it, so it is not started"
                ));
            }
            Ok(named.to_path_buf())
        }
    }
}

/// The command line the permission engine reads for `program`, as the
/// manifest names it, with `arguments`: the program's own word, and each
/// argument quoted by `shell`'s rules.
///
/// The program is written as it is when it is a plain word (letters,
/// digits, `_`, `-`, `.`, and the path's separators), so that a rule such
/// as `exec gh issue *` matches it; otherwise it is quoted too, and in
/// PowerShell called with `&`, which a quoted command needs there.
///
/// # Errors
///
/// When the shell cannot carry the program or an argument as one word,
/// naming which and why.
pub(crate) fn engine_line(
    shell: &Shell,
    program: &str,
    arguments: &[String],
) -> Result<String, String> {
    let kind = shell.kind();
    let windows_kind = matches!(
        kind,
        ShellKind::Cmd | ShellKind::Pwsh | ShellKind::WindowsPowerShell
    );
    let plain = program
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric() || first == '_' || first == '/')
        && program.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '_' | '-' | '.' | '/')
                || (windows_kind && matches!(character, '\\' | ':'))
        });
    let mut line = if plain {
        program.to_owned()
    } else {
        let quoted = shell.quote(program).map_err(|why| {
            format!("the program `{program}` cannot be written for the shell: {why}")
        })?;
        if kind.is_powershell() {
            format!("& {quoted}")
        } else {
            quoted
        }
    };
    for (index, argument) in arguments.iter().enumerate() {
        let quoted = shell.quote(argument).map_err(|why| {
            format!(
                "argument {} cannot be written for the session's shell, {}, as one word: {why}",
                index + 1,
                shell.name()
            )
        })?;
        line.push(' ');
        line.push_str(&quoted);
    }
    Ok(line)
}

/// Why a script's line for `cmd.exe` was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BatchRefusal {
    /// The script's path, or an argument, holds a character `cmd.exe`
    /// cannot carry unchanged.
    Character {
        /// The script's path, or `argument N`.
        which: String,
        /// The character, named.
        character: &'static str,
    },
    /// The line is longer than `cmd.exe` reads.
    TooLong {
        /// Its length, in UTF-16 units.
        length: usize,
    },
}

impl std::fmt::Display for BatchRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Character { which, character } => write!(
                f,
                "{which} holds {character}, which cmd.exe cannot pass to a .cmd or .bat script \
                 unchanged"
            ),
            Self::TooLong { length } => write!(
                f,
                "the script's command line is {length} characters, and cmd.exe reads about \
                 {BATCH_LINE_CHARS}"
            ),
        }
    }
}

/// The characters `cmd.exe` cannot carry in a script's line unchanged, and
/// their names: a quote ends the quoting; `%` and `!` are expanded inside
/// quotes (`!` where the script turns delayed expansion on); and a line
/// break or a NUL ends the line.
const REFUSED: [(char, &str); 6] = [
    ('"', "a double quote (\")"),
    ('%', "a percent sign (%)"),
    ('!', "an exclamation mark (!)"),
    ('\r', "a carriage return"),
    ('\n', "a line feed"),
    ('\0', "a NUL character"),
];

/// The line `cmd.exe /s /c` runs a batch script with: `""<script>"
/// "<argument>" …"`, the whole in an outer pair of quotes that `/s` takes
/// off, and the script's path and each argument in quotes of their own.
/// Inside those, `^ & | < > ( )` are plain characters to `cmd.exe`, and a
/// run of backslashes before the closing quote is doubled, so that a
/// program the script hands `%*` to reads it as one argument ending in
/// those backslashes (by the C runtime's rules), never as an escaped quote
/// that runs on into the next argument.
///
/// # Errors
///
/// When the path or an argument holds a character of [`REFUSED`], naming
/// which and the character; or when the line is longer than `cmd.exe`
/// reads.
pub(crate) fn batch_line(script: &str, arguments: &[String]) -> Result<String, BatchRefusal> {
    let mut line = String::from("\"");
    for (index, item) in std::iter::once(script)
        .chain(arguments.iter().map(String::as_str))
        .enumerate()
    {
        if let Some((_, character)) = REFUSED.iter().find(|(refused, _)| item.contains(*refused)) {
            return Err(BatchRefusal::Character {
                which: if index == 0 {
                    "the script's path".to_owned()
                } else {
                    format!("argument {index}")
                },
                character,
            });
        }
        if index > 0 {
            line.push(' ');
        }
        let trailing = item.chars().rev().take_while(|c| *c == '\\').count();
        line.push('"');
        line.push_str(item);
        line.push_str(&"\\".repeat(trailing));
        line.push('"');
    }
    line.push('"');
    let length = line.encode_utf16().count();
    if length > BATCH_LINE_CHARS {
        return Err(BatchRefusal::TooLong { length });
    }
    Ok(line)
}

/// The command line that starts `found` with `arguments`: the program as
/// it is, or on Windows a `.cmd` or `.bat` through `cmd`, which is
/// `%SystemRoot%\System32\cmd.exe` ([`system_cmd`]).
///
/// # Errors
///
/// The reason a script's line was refused, or that there is no `cmd.exe`
/// to start one with.
pub(crate) fn command_line(
    found: &Path,
    arguments: &[String],
    windows: bool,
    cmd: Option<&Path>,
) -> Result<CommandLine, String> {
    if !(windows && cineris_base::program::is_batch_script(found)) {
        return Ok(CommandLine::new(found).args(arguments.iter().cloned()));
    }
    let Some(cmd) = cmd else {
        return Err(format!(
            "`{}` is a batch script, and there is no %SystemRoot%\\System32\\cmd.exe to start it with",
            found.display()
        ));
    };
    let script = found.to_str().ok_or_else(|| {
        format!(
            "the script's path `{}` is not text, which cmd.exe needs",
            found.display()
        )
    })?;
    let line = batch_line(script, arguments).map_err(|why| why.to_string())?;
    Ok(CommandLine::new(cmd).args(CMD_ARGUMENTS).verbatim(line))
}

/// `%SystemRoot%\System32\cmd.exe`, from the environment programs inherit,
/// as the shell's search finds it: `SystemRoot` absolute, and a file
/// there. Never `%ComSpec%`, which names any program; and never one that
/// passes through `avoid`. Blocks.
pub(crate) fn system_cmd(
    inherited: &[(std::ffi::OsString, std::ffi::OsString)],
    avoid: &[PathBuf],
) -> Option<PathBuf> {
    let root = inherited
        .iter()
        .find(|(name, _)| cineris_base::child_env::is_named(name, "SystemRoot", true))
        .map(|(_, value)| PathBuf::from(value))
        .filter(|root| root.is_absolute())?;
    let cmd = root.join("System32").join("cmd.exe");
    let avoid = cineris_base::program::resolved(avoid);
    (cineris_base::program::is_program(&cmd, true)
        && !cineris_base::program::passes_through(&cmd, &avoid))
    .then_some(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    /// The line `cmd.exe` is given: the outer quotes, each item quoted, the
    /// characters `cmd.exe` would act on left plain inside the quotes.
    #[test]
    fn a_scripts_line_quotes_each_item_inside_an_outer_pair() {
        assert_eq!(
            batch_line(
                r"C:\Program Files\nodejs\npx.cmd",
                &args(&["a b", "x^y&z|w<v>u(t)"])
            )
            .unwrap(),
            r#"""C:\Program Files\nodejs\npx.cmd" "a b" "x^y&z|w<v>u(t)"""#
        );
        assert_eq!(
            batch_line(r"C:\t\x.bat", &args(&["", "plain"])).unwrap(),
            r#"""C:\t\x.bat" "" "plain"""#
        );
    }

    /// A run of backslashes before a closing quote is doubled, so that the
    /// quote still ends the argument for the program the script starts:
    /// `x\` followed by `a --upload-pack=evil` stays two arguments.
    #[test]
    fn a_trailing_backslash_does_not_end_an_argument_early() {
        assert_eq!(
            batch_line(r"C:\t\x.cmd", &args(&["x\\", "a --upload-pack=evil"])).unwrap(),
            r#"""C:\t\x.cmd" "x\\" "a --upload-pack=evil"""#
        );
        assert_eq!(
            batch_line(r"C:\t\x.cmd", &args(&[r"a\\\", r"b\c"])).unwrap(),
            r#"""C:\t\x.cmd" "a\\\\\\" "b\c"""#
        );
    }

    /// Every character `cmd.exe` cannot carry is refused, in the path or in
    /// any argument, naming which and the character.
    #[test]
    fn each_character_cmd_cannot_carry_is_refused_with_its_name() {
        for (character, name) in REFUSED {
            let bad = format!("a{character}b");
            let refused = batch_line(r"C:\t\x.cmd", &args(&["ok", &bad])).unwrap_err();
            assert_eq!(
                refused,
                BatchRefusal::Character {
                    which: "argument 2".to_owned(),
                    character: name,
                }
            );
            assert!(refused.to_string().contains("argument 2"), "{refused}");
            let script = format!(r"C:\t{character}\x.cmd");
            let refused = batch_line(&script, &[]).unwrap_err();
            assert!(
                refused.to_string().starts_with("the script's path holds"),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_line_longer_than_cmd_reads_is_refused() {
        let long = "a".repeat(BATCH_LINE_CHARS);
        assert!(matches!(
            batch_line(r"C:\t\x.cmd", &[long]),
            Err(BatchRefusal::TooLong { .. })
        ));
    }

    /// A batch script on Windows is started through `cmd.exe` with the
    /// script's line as one verbatim argument; anything else, and every
    /// program elsewhere, as it is.
    #[test]
    fn a_batch_script_is_started_through_cmd_and_a_program_as_it_is() {
        use cineris_tools::process::Argument;
        let cmd = Path::new(r"C:\Windows\System32\cmd.exe");
        let line =
            command_line(Path::new(r"C:\t\x.CMD"), &args(&["a&b"]), true, Some(cmd)).unwrap();
        assert_eq!(line.program, cmd);
        // No AutoRun, command extensions on (the shims' `%~dp0` and
        // `call :label`), delayed expansion off, the outer quotes taken off.
        let mut expected: Vec<Argument> = ["/d", "/e:on", "/v:off", "/s", "/c"]
            .iter()
            .map(|one| Argument::Exact((*one).to_owned()))
            .collect();
        expected.push(Argument::Verbatim(r#"""C:\t\x.CMD" "a&b"""#.to_owned()));
        assert_eq!(line.arguments, expected);
        assert!(command_line(Path::new(r"C:\t\x.bat"), &[], true, None).is_err());

        for (program, windows) in [(r"C:\t\x.exe", true), ("/usr/bin/x.cmd", false)] {
            let line =
                command_line(Path::new(program), &args(&["a&b"]), windows, Some(cmd)).unwrap();
            assert_eq!(line.program, Path::new(program));
            assert_eq!(line.arguments, vec![Argument::Exact("a&b".to_owned())]);
        }
    }

    /// The line the engine reads: the program as named when it is a plain
    /// word, each argument quoted by the shell's rules; an argument the
    /// shell cannot carry refuses the call, naming it.
    #[test]
    fn the_engines_line_quotes_each_argument_in_the_shells_syntax() {
        let bash = Shell::new("/bin/bash");
        assert_eq!(
            engine_line(&bash, "gh", &args(&["issue", "it's", "a b;c"])).unwrap(),
            r"gh 'issue' 'it'\''s' 'a b;c'"
        );
        assert_eq!(
            engine_line(&bash, "/opt/my tools/x", &args(&["-v"])).unwrap(),
            "'/opt/my tools/x' '-v'"
        );
        let pwsh = Shell::of_kind(ShellKind::Pwsh, r"C:\pwsh\pwsh.exe");
        assert_eq!(
            engine_line(&pwsh, r"C:\Program Files\x.exe", &args(&["it's"])).unwrap(),
            r"& 'C:\Program Files\x.exe' 'it''s'"
        );
        assert_eq!(
            engine_line(&pwsh, r"C:\tools\x.exe", &args(&["a"])).unwrap(),
            r"C:\tools\x.exe 'a'"
        );
        let cmd = Shell::of_kind(ShellKind::Cmd, r"C:\Windows\System32\cmd.exe");
        assert_eq!(
            engine_line(&cmd, "npx", &args(&["a&b", "c\\"])).unwrap(),
            r#"npx "a&b" "c\\""#
        );
        let refused = engine_line(&cmd, "npx", &args(&["ok", "100%"])).unwrap_err();
        assert!(refused.contains("argument 2"), "{refused}");
        let refused = engine_line(&bash, "x", &args(&["a\0b"])).unwrap_err();
        assert!(refused.contains("argument 1"), "{refused}");
        // A name that is not a plain word is quoted, never read as more
        // than one command.
        assert_eq!(engine_line(&bash, "x;rm", &[]).unwrap(), "'x;rm'");
    }
}
