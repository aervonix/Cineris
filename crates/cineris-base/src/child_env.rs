//! The environment of a program the harness starts
//! (`docs/security.md`, section 1, "Every process the harness starts").
//!
//! Two kinds of process are started with an environment built here:
//!
//! - **A command the model wrote,** through the shell tool, whose builder
//!   (`cineris-tools`, `shell/env.rs`) settles more besides: no pager, no
//!   colour, what the command analysis relies on.
//! - **A program the harness starts for itself,** unasked: its own git
//!   ([`crate::git::start`]) and the shell asked its version. Such a
//!   program can run what the workspace names (a filter a repository's
//!   configuration names, which git runs on `git status`), so it sees no
//!   more than a command sees: [`for_harness`]. Since the harness's own
//!   processes' fixes (`docs/steps/harness-process-fixes/answered.md`).
//!
//! Both take out the providers' key variables ([`KNOWN_KEYS`], and the
//! list the core gives, [`Withheld`]), and both give `PATH` the same rule
//! ([`one_absolute_path`]): its absolute directories only, since an empty
//! or relative entry names the directory the program runs in, which is the
//! workspace. On Windows both tell `cmd.exe` not to look there either
//! ([`NO_CURRENT_DIRECTORY`]). Everything else passes as it came: `HOME`,
//! `XDG_CONFIG_HOME`, the user's `GIT_*`, the locale, and on Windows
//! `SystemRoot` and `USERPROFILE`, which git reads the user's
//! configuration through.

use std::ffi::{OsStr, OsString};
use std::sync::{Mutex, PoisonError};

/// The variables that are always taken out: the keys of the providers the
/// harness knows, whatever list a caller gives. The core adds the key
/// variable of every provider it is configured with ([`Withheld`]).
pub const KNOWN_KEYS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
];

/// The `PATH` a program is given on Unix when the harness's own names no
/// absolute directory, or there is none: the system's own directories,
/// which root writes and a command does not. The smallest standard set: the
/// directories `getconf PATH` gives, with `/usr/bin` first because a
/// merged-`/usr` system links `/bin` to it. An empty `PATH` would be the
/// directory the program runs in, the workspace; and with no `PATH` a shell
/// takes a default of its own, which upstream bash ends with `.`.
pub const NO_ABSOLUTE_PATH: &str = "/usr/bin:/bin";

/// The variable whose presence, with any value, turns off `cmd.exe`'s look
/// for a program in the directory it runs in, ahead of `PATH` (Windows'
/// `NeedCurrentDirectoryForExePathW`). Every program `cmd.exe` starts
/// inherits it. A program named by its path (`.\build.cmd`) still runs; a
/// batch file that calls another of its directory by its bare name no
/// longer finds it, and `call "%~dp0other.cmd"` does.
pub const NO_CURRENT_DIRECTORY: &str = "NoDefaultCurrentDirectoryInExePath";

/// The variable programs are looked up through.
const PATH: &str = "PATH";

/// Whether the variable `name` is the one `pattern` names: the same name,
/// or, for a pattern that ends with `*`, any name that starts so. On
/// Windows a name is matched in any case, as Windows does.
#[must_use]
pub fn matches(name: &str, pattern: &str, windows: bool) -> bool {
    let (pattern, prefix) = match pattern.strip_suffix('*') {
        Some(prefix) => (prefix, true),
        None => (pattern, false),
    };
    let (head, tail) = match name.is_char_boundary(pattern.len()) {
        true if name.len() >= pattern.len() => name.split_at(pattern.len()),
        _ => return false,
    };
    let same = if windows {
        head.eq_ignore_ascii_case(pattern)
    } else {
        head == pattern
    };
    same && (prefix || tail.is_empty())
}

/// [`matches`] for a name as the system gives it. A name that is not
/// Unicode is no name on any list.
#[must_use]
pub fn is_named(name: &OsStr, pattern: &str, windows: bool) -> bool {
    name.to_str()
        .is_some_and(|name| matches(name, pattern, windows))
}

/// Whether the variable `name` is one no program the harness starts is
/// given: one of [`KNOWN_KEYS`], or one `withheld` names.
#[must_use]
pub fn is_withheld(name: &OsStr, withheld: &[String], windows: bool) -> bool {
    KNOWN_KEYS
        .iter()
        .copied()
        .chain(withheld.iter().map(String::as_str))
        .any(|pattern| is_named(name, pattern, windows))
}

/// The environment of a program the harness starts for itself: `inherited`
/// without [`KNOWN_KEYS`] and what `withheld` names, with one `PATH` of
/// absolute directories ([`one_absolute_path`]) none of which is inside
/// `avoid` (the workspace, and the write roots while commands are
/// confined), and on Windows [`NO_CURRENT_DIRECTORY`]. The rest is as it
/// came.
///
/// A program the harness starts unasked finds what it runs by a bare name
/// (git a filter the user's configuration names, `git-lfs`) only where no
/// command could have put it: a commands' `PATH` keeps an activated
/// `.venv/bin`, and this one does not (found by the early review of the
/// harness's own processes' fixes).
///
/// A caller clears the child's environment and gives it this, so that no
/// variable reaches it by inheritance.
#[must_use]
pub fn for_harness(
    inherited: impl IntoIterator<Item = (OsString, OsString)>,
    withheld: &[String],
    avoid: &[std::path::PathBuf],
    windows: bool,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = inherited
        .into_iter()
        .filter(|(name, _)| !is_withheld(name, withheld, windows))
        .collect();
    if windows {
        no_current_directory(&mut env);
    }
    one_absolute_path(&mut env, windows);
    if !avoid.is_empty() {
        keep_out_of(&mut env, avoid, windows);
    }
    env
}

/// Takes out of the one `PATH` of `env` ([`one_absolute_path`] left it)
/// every directory reached through one of `avoid`
/// ([`crate::program::passes_through`]). On Unix a `PATH` left with none
/// is [`NO_ABSOLUTE_PATH`], as one with no absolute directory is.
fn keep_out_of(env: &mut [(OsString, OsString)], avoid: &[std::path::PathBuf], windows: bool) {
    let avoid = crate::program::resolved(avoid);
    let separator = if windows { ';' } else { ':' };
    let Some((_, value)) = env
        .iter_mut()
        .find(|(name, _)| is_named(name, PATH, windows))
    else {
        return;
    };
    let Some(text) = value.to_str() else {
        let kept = std::env::split_paths(value)
            .filter(|entry| !crate::program::passes_through(entry, &avoid))
            .collect::<Vec<_>>();
        *value = std::env::join_paths(kept).unwrap_or_default();
        return;
    };
    let kept: Vec<&str> = text
        .split(separator)
        .filter(|entry| {
            let directory = if windows {
                entry.trim_matches('"')
            } else {
                entry
            };
            !crate::program::passes_through(std::path::Path::new(directory), &avoid)
        })
        .collect();
    *value = if kept.is_empty() && !windows {
        OsString::from(NO_ABSOLUTE_PATH)
    } else {
        OsString::from(kept.join(&separator.to_string()))
    };
}

/// Sets [`NO_CURRENT_DIRECTORY`] once, under its own spelling, in place of
/// any the environment held: Windows' rule, whatever the system running
/// this is.
pub fn no_current_directory(env: &mut Vec<(OsString, OsString)>) {
    env.retain(|(name, _)| !is_named(name, NO_CURRENT_DIRECTORY, true));
    env.push((OsString::from(NO_CURRENT_DIRECTORY), OsString::from("1")));
}

/// Leaves `env` one `PATH`, of absolute directories only, as its last
/// variable; and returns whether it is [`NO_ABSOLUTE_PATH`] in place of
/// the environment's own.
///
/// - **An empty or relative entry goes** (`.`, `bin`, `./node_modules/.bin`):
///   it names the directory the program runs in. The rest keep their order.
/// - **An environment may hold the name twice:** the harness reads the
///   first (as `getenv` does), and a child would be given the last, so
///   every one goes and the first comes back without its relative entries.
/// - **On Unix a `PATH` with none left, or none at all, is not passed on
///   empty or left out:** an empty one is the same "here", and with none a
///   shell takes a default of its own. The program is given
///   [`NO_ABSOLUTE_PATH`] in its place.
/// - **On Windows the `PATH` is passed on as it is left,** empty if nothing
///   absolute was in it, and none is made where there was none: the system
///   finds its own programs through `SystemRoot`, and with
///   [`NO_CURRENT_DIRECTORY`] set an empty `PATH` finds nothing by a bare
///   name.
pub fn one_absolute_path(env: &mut Vec<(OsString, OsString)>, windows: bool) -> bool {
    let is_path = |name: &OsString| is_named(name, PATH, windows);
    let first = env
        .iter()
        .find(|(name, _)| is_path(name))
        .map(|(name, value)| (name.clone(), absolute_entries(value, windows)));
    env.retain(|(name, _)| !is_path(name));
    let replaced = !windows && first.as_ref().is_none_or(|(_, value)| value.is_empty());
    match first {
        Some((name, value)) if windows || !value.is_empty() => env.push((name, value)),
        // On Windows the `PATH` is passed on as it is left.
        _ if windows => {}
        _ => env.push((OsString::from(PATH), OsString::from(NO_ABSOLUTE_PATH))),
    }
    replaced
}

/// `path` without its empty and its relative entries, which both name the
/// directory a program runs in.
#[must_use]
pub fn absolute_entries(path: &OsStr, windows: bool) -> OsString {
    let Some(text) = path.to_str() else {
        // Not Unicode: the system's own reading of it, which is what the
        // program would be given anyway.
        let kept = std::env::split_paths(path).filter(|entry| entry.is_absolute());
        return std::env::join_paths(kept).unwrap_or_default();
    };
    let separator = if windows { ';' } else { ':' };
    let absolute = |entry: &str| {
        let bytes = entry.as_bytes();
        if windows {
            // A drive and a root (`C:\`), or a share (`\\server\…`).
            // `C:bin` and `\bin` are relative to where the program is.
            let slash = |byte: &u8| *byte == b'\\' || *byte == b'/';
            (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && slash(&bytes[2]))
                || (bytes.len() >= 2 && slash(&bytes[0]) && slash(&bytes[1]))
        } else {
            bytes.first() == Some(&b'/')
        }
    };
    let kept: Vec<&str> = text
        .split(separator)
        .filter(|entry| {
            // Windows allows an entry in quotes, for one with a semicolon.
            // On Linux a quote is part of a name: `"/x"` is the directory
            // `x` in a directory named `"`, which is relative.
            if windows {
                absolute(entry.trim_matches('"'))
            } else {
                absolute(entry)
            }
        })
        .collect();
    OsString::from(kept.join(&separator.to_string()))
}

/// The providers' key variables as they are now: the one list that the
/// shell's commands, the harness's git and the shell's version probe are
/// all kept from. The core makes it from its configuration when a session
/// starts, a reload sets it, and every reader holds the same value, so that
/// none keeps an old copy.
#[derive(Debug, Default)]
pub struct Withheld(Mutex<Vec<String>>);

impl Withheld {
    /// The list a session starts with.
    #[must_use]
    pub fn new(variables: Vec<String>) -> Self {
        Self(Mutex::new(variables))
    }

    /// The variables now.
    #[must_use]
    pub fn get(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The variables, in place of those there were.
    pub fn set(&self, variables: Vec<String>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = variables;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    fn get<'a>(env: &'a [(OsString, OsString)], name: &str) -> Vec<&'a str> {
        env.iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.to_str().expect("text"))
            .collect()
    }

    /// The keys go, the known ones and those the core names, in any case on
    /// Windows; everything git reads the user's configuration through
    /// stays.
    #[test]
    fn a_harness_program_is_given_no_key_and_the_rest_as_it_came() {
        let inherited = env(&[
            ("ANTHROPIC_API_KEY", "sk-known"),
            ("MY_PROVIDER_KEY", "sk-configured"),
            ("TEAM_TOKEN_A", "sk-pattern"),
            ("HOME", "/srv/u"),
            ("XDG_CONFIG_HOME", "/srv/u/.config"),
            ("GIT_CONFIG_GLOBAL", "/srv/u/.gitconfig"),
            ("LANG", "C.UTF-8"),
            ("PATH", "/usr/bin"),
        ]);
        let withheld = ["MY_PROVIDER_KEY".to_owned(), "TEAM_TOKEN_*".to_owned()];
        let built = for_harness(inherited.clone(), &withheld, &[], false);
        for gone in ["ANTHROPIC_API_KEY", "MY_PROVIDER_KEY", "TEAM_TOKEN_A"] {
            assert!(get(&built, gone).is_empty(), "{gone}: {built:?}");
        }
        for (name, value) in &inherited[3..] {
            assert_eq!(
                get(&built, name.to_str().unwrap()),
                [value.to_str().unwrap()]
            );
        }
        assert!(
            !built
                .iter()
                .any(|(_, value)| value.to_string_lossy().contains("sk-")),
            "{built:?}"
        );
        // Without the core's list the known keys still go, and the others
        // do not: the list is what reaches them.
        let bare = for_harness(inherited, &[], &[], false);
        assert!(get(&bare, "ANTHROPIC_API_KEY").is_empty());
        assert_eq!(get(&bare, "MY_PROVIDER_KEY"), ["sk-configured"]);

        // On Windows in any case; on Linux `My_Provider_Key` is another
        // variable.
        let mixed = env(&[("My_Provider_Key", "sk"), ("Anthropic_Api_Key", "sk")]);
        assert!(
            for_harness(mixed.clone(), &withheld, &[], true)
                .iter()
                .all(|(name, _)| name != "My_Provider_Key" && name != "Anthropic_Api_Key")
        );
        assert_eq!(
            get(
                &for_harness(mixed, &withheld, &[], false),
                "My_Provider_Key"
            ),
            ["sk"]
        );
    }

    /// The `PATH` rule, as the shell's: absolute entries only, the first
    /// `PATH`, and on Unix the system's directories when nothing is left.
    #[test]
    fn a_harness_program_searches_only_absolute_directories() {
        let built = for_harness(
            env(&[("PATH", ".::bin:/usr/local/bin:./x:/bin:")]),
            &[],
            &[],
            false,
        );
        assert_eq!(get(&built, "PATH"), ["/usr/local/bin:/bin"]);
        for nothing in [
            &env(&[("PATH", ".::bin")])[..],
            &env(&[("PATH", "")]),
            &env(&[]),
        ] {
            let built = for_harness(nothing.to_vec(), &[], &[], false);
            assert_eq!(get(&built, "PATH"), [NO_ABSOLUTE_PATH], "{nothing:?}");
        }
        let twice = env(&[("PATH", "/usr/bin"), ("PATH", ".")]);
        assert_eq!(
            get(&for_harness(twice, &[], &[], false), "PATH"),
            ["/usr/bin"]
        );
        // A `PATH` the core's list names is a `PATH` still: it is not left
        // out, it is replaced.
        let removed = for_harness(env(&[("PATH", "/usr/bin")]), &["P*".to_owned()], &[], false);
        assert_eq!(get(&removed, "PATH"), [NO_ABSOLUTE_PATH]);
        assert_eq!(NO_ABSOLUTE_PATH, "/usr/bin:/bin");
    }

    /// A directory of `PATH` inside an avoided root (an activated
    /// `.venv/bin` in the workspace) is not searched by a program the
    /// harness starts for itself: a filter git finds by a bare name would
    /// be a file a command wrote.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "the test makes its directories")]
    fn a_harness_program_does_not_search_the_workspace() {
        use crate::rng::{Rng, SystemRng};
        let dir = std::env::temp_dir().join(format!(
            "cineris-base-child-env-{:016x}",
            SystemRng::new().next_u64()
        ));
        let ws = dir.join("ws");
        let venv = ws.join(".venv").join("bin");
        let usr = dir.join("usr");
        std::fs::create_dir_all(&venv).unwrap();
        std::fs::create_dir_all(&usr).unwrap();
        let windows = cfg!(windows);
        let separator = if windows { ";" } else { ":" };
        let path = format!("{}{separator}{}", venv.display(), usr.display());
        let built = for_harness(
            env(&[("PATH", &path)]),
            &[],
            std::slice::from_ref(&ws),
            windows,
        );
        assert_eq!(get(&built, "PATH"), [usr.display().to_string()]);
        // Nothing avoided, it is searched.
        let built = for_harness(env(&[("PATH", &path)]), &[], &[], windows);
        assert_eq!(get(&built, "PATH"), [path.as_str()]);
        // Only the workspace's: on Unix the system's directories.
        if !windows {
            let only = venv.display().to_string();
            let built = for_harness(env(&[("PATH", &only)]), &[], &[ws], false);
            assert_eq!(get(&built, "PATH"), [NO_ABSOLUTE_PATH]);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On Windows `cmd.exe` is told not to look where it runs, once, under
    /// Windows' spelling; and a `Path` with nothing absolute in it is passed
    /// on empty, and none is made where there was none.
    #[test]
    fn on_windows_a_harness_program_is_told_not_to_look_where_it_runs() {
        let built = for_harness(
            env(&[
                ("Path", r".;C:\Windows\System32;;bin;C:rel"),
                ("nodefaultcurrentdirectoryinexepath", ""),
                ("SystemRoot", r"C:\Windows"),
            ]),
            &[],
            &[],
            true,
        );
        assert_eq!(get(&built, "Path"), [r"C:\Windows\System32"]);
        assert_eq!(get(&built, NO_CURRENT_DIRECTORY), ["1"]);
        assert!(get(&built, "nodefaultcurrentdirectoryinexepath").is_empty());
        assert_eq!(get(&built, "SystemRoot"), [r"C:\Windows"]);

        let nothing = for_harness(env(&[("Path", r".;bin;C:rel")]), &[], &[], true);
        assert_eq!(get(&nothing, "Path"), [""]);
        assert_eq!(get(&nothing, NO_CURRENT_DIRECTORY), ["1"]);
        let none = for_harness(env(&[("KEPT", "1")]), &[], &[], true);
        assert!(
            !none
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("PATH")),
            "{none:?}"
        );
        // Not on Linux, where it means nothing.
        let linux = for_harness(env(&[("PATH", "/usr/bin")]), &[], &[], false);
        assert!(get(&linux, NO_CURRENT_DIRECTORY).is_empty());
    }

    #[test]
    fn a_pattern_with_a_star_covers_every_name_that_starts_so() {
        assert!(matches("SECRET_ONE", "SECRET_*", false));
        assert!(matches("SECRET_", "SECRET_*", false));
        assert!(!matches("SECRETS", "SECRET_*", false));
        assert!(matches("EXACT", "EXACT", false));
        assert!(!matches("EXACTLY", "EXACT", false));
        assert!(!matches("exact", "EXACT", false));
        assert!(matches("exact", "EXACT", true));
    }

    /// One value: a reload's list is what every holder reads next.
    #[test]
    fn the_withheld_list_is_one_value() {
        let withheld = std::sync::Arc::new(Withheld::new(vec!["OLD".to_owned()]));
        let reader = std::sync::Arc::clone(&withheld);
        withheld.set(vec!["NEW".to_owned()]);
        assert_eq!(reader.get(), ["NEW"]);
    }
}
