//! The environment a local MCP server is given: an explicit list, never the
//! harness's own environment (`docs/steps/30/answered.md`, point 3.3).
//!
//! 1. **The base,** each variable taken from the harness's environment
//!    when it is there: `PATH`, the locale (`LANG`, `LC_*`, `TZ`), and on
//!    Unix the home folder and the user's name (`HOME`, `USER`,
//!    `LOGNAME`); on Windows the home folder (`USERPROFILE`, `HOMEDRIVE`,
//!    `HOMEPATH`), the user's names (`USERNAME`, `USERDOMAIN`) and the
//!    system's own (`SystemRoot`, `SystemDrive`, `windir`, `APPDATA`,
//!    `LOCALAPPDATA`, `ProgramData`, `ProgramFiles`, `ProgramFiles(x86)`,
//!    `CommonProgramFiles`, `COMPUTERNAME`, `NUMBER_OF_PROCESSORS`,
//!    `PROCESSOR_ARCHITECTURE`, `OS`). One the session keeps from its
//!    commands (a provider's key, `shell.env_remove`) is not taken.
//! 2. **Then each of the entry's `env`:** a value from the user's variable
//!    of the name it gives, read now (one that is not set refuses the
//!    start, naming it), or a literal as it is written.
//! 3. **Then what the harness sets itself:** the server's private
//!    temporary folder as `TMPDIR` (`TEMP` and `TMP` on Windows); on
//!    Windows `ComSpec` as the system's own `cmd.exe`, `PATHEXT` at its
//!    default, and `NoDefaultCurrentDirectoryInExePath`, so that `cmd.exe`
//!    never runs a program from the folder it runs in. An entry may not
//!    name one of these.
//! 4. **`PATH` keeps its absolute folders only,** with the workspace and
//!    the write roots taken out, whether it came from the base or from an
//!    entry: a program a server finds by its bare name is never one a
//!    command could have planted.
//!
//! **Never given,** at any of those steps: a provider's key
//! (`cineris_base::child_env::KNOWN_KEYS`, and every variable the harness
//! took a credential from), a variable of `shell.env_remove`, or one whose
//! name begins `CINERIS_`; nor, as an entry, one that tells the system's
//! loader what to run (`LD_PRELOAD`, every `LD_` one, `GLIBC_TUNABLES`,
//! `DYLD_…`: `cineris_config::mcp::loads_code`), since the environment is
//! set on the launcher's own process before it confines itself. An entry that names one of them as where its
//! value comes from, or that gives a `CINERIS_` variable, refuses the
//! start, as does a name that is empty, holds `=` or a NUL, or begins with
//! a digit.
//!
//! Each value read from the user's environment comes back beside the list
//! ([`ServerEnv::secrets`]), for the session's secret filter: a server that
//! echoes one has it masked, as a provider's header read from the
//! environment is.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use cineris_base::Secret;
use cineris_base::child_env;
use cineris_mcp::{EnvEntry, SpawnError, ValueSource};

/// The base variables on every system: `PATH` and the locale.
const BASE: &[&str] = &["PATH", "LANG", "LC_*", "TZ"];

/// The base variables on Unix besides [`BASE`]: the home folder and the
/// user's name.
const BASE_UNIX: &[&str] = &["HOME", "USER", "LOGNAME"];

/// The base variables on Windows besides [`BASE`]: the home folder, the
/// user's names and the system's own.
const BASE_WINDOWS: &[&str] = &[
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERNAME",
    "USERDOMAIN",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "CommonProgramFiles",
    "COMPUTERNAME",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
];

/// `PATHEXT` as Windows sets it by default.
pub(crate) const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC";

/// What a server's environment is made from, besides its entry's `env`.
#[derive(Debug, Clone, Copy)]
pub struct EnvParts<'a> {
    /// The harness's environment, which the base and the entries' values
    /// are read from.
    pub inherited: &'a [(OsString, OsString)],
    /// The variables no server is given besides [`child_env::KNOWN_KEYS`]:
    /// those the harness took a credential from, and `shell.env_remove`'s
    /// (a name may end with `*`).
    pub withheld: &'a [String],
    /// Where a program found by name is never taken from: the workspace and
    /// the write roots, taken out of `PATH`.
    pub avoid: &'a [PathBuf],
    /// The server's private temporary folder.
    pub temp: &'a Path,
    /// On Windows, the system's own `cmd.exe`, given as `ComSpec`.
    pub cmd: Option<&'a Path>,
    /// Whether to make it by Windows's rules.
    pub windows: bool,
}

/// A server's whole environment, and the values read from the user's
/// environment for it.
pub struct ServerEnv {
    /// Every variable it is given, and nothing else.
    pub vars: Vec<(OsString, OsString)>,
    /// The values its entries read from the user's environment: for the
    /// session's secret filter.
    pub secrets: Vec<Secret>,
}

impl std::fmt::Debug for ServerEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The names only: a value may be a credential.
        let names: Vec<_> = self.vars.iter().map(|(name, _)| name).collect();
        f.debug_struct("ServerEnv")
            .field("names", &names)
            .field("secrets", &self.secrets.len())
            .finish()
    }
}

/// Why `name` cannot be a variable's name: empty, holding `=` or a NUL, or
/// beginning with a digit (`cineris_config::mcp::bad_variable_name`, which
/// refuses such an entry as the configuration loads). `None` when it can.
#[must_use]
pub fn bad_name(name: &str) -> Option<&'static str> {
    cineris_config::mcp::bad_variable_name(name)
}

/// Whether `name` is one of the harness's own: it begins `CINERIS_`, in
/// any case.
fn harness_own(name: &str) -> bool {
    name.get(..8)
        .is_some_and(|start| start.eq_ignore_ascii_case("CINERIS_"))
}

/// The variables the harness sets itself, which no entry may name.
fn set_by_the_harness(windows: bool) -> &'static [&'static str] {
    if windows {
        &[
            "TEMP",
            "TMP",
            "TMPDIR",
            "ComSpec",
            "PATHEXT",
            child_env::NO_CURRENT_DIRECTORY,
        ]
    } else {
        &["TMPDIR"]
    }
}

/// The environment of a server whose entry gives `entries`, made from
/// `parts` ([the module](self) says how).
///
/// # Errors
///
/// [`SpawnError::Refused`], naming the variable, when an entry's name
/// cannot be a variable's or is one of the harness's own, or its value
/// comes from a variable that is not set, or from one no server is given.
pub fn server_env(entries: &[EnvEntry], parts: &EnvParts<'_>) -> Result<ServerEnv, SpawnError> {
    let windows = parts.windows;
    let withheld = |name: &OsStr| {
        child_env::is_withheld(name, parts.withheld, windows)
            || name.to_str().is_none_or(harness_own)
    };
    let own_base = if windows { BASE_WINDOWS } else { BASE_UNIX };
    let mut vars: Vec<(OsString, OsString)> = parts
        .inherited
        .iter()
        .filter(|(name, _)| {
            BASE.iter()
                .chain(own_base)
                .any(|pattern| child_env::is_named(name, pattern, windows))
        })
        .filter(|(name, _)| !withheld(name))
        .cloned()
        .collect();

    let mut secrets = Vec::new();
    for entry in entries {
        let name = entry.name.as_str();
        let shown = cineris_base::in_code(name);
        if let Some(why) = bad_name(name) {
            return Err(SpawnError::Refused(format!(
                "the variable `{shown}` {why}, so it cannot be given"
            )));
        }
        if harness_own(name) {
            return Err(SpawnError::Refused(format!(
                "the variable `{shown}` is one of the harness's own, and no server is given one"
            )));
        }
        // Set on the launcher's own process, before it confines itself.
        if cineris_config::mcp::loads_code(name) {
            return Err(SpawnError::Refused(format!(
                "the variable `{shown}` tells the system's loader what to run, which it would do \
                 before the sandbox confines the server, and no server is given one"
            )));
        }
        if set_by_the_harness(windows)
            .iter()
            .any(|set| child_env::matches(name, set, windows))
        {
            return Err(SpawnError::Refused(format!(
                "the variable `{shown}` is set by the harness for every server, and an entry may \
                 not give it"
            )));
        }
        let value = match &entry.value {
            ValueSource::Env(source) => {
                let from = cineris_base::in_code(source);
                if withheld(OsStr::new(source)) {
                    return Err(SpawnError::Refused(format!(
                        "`{shown}` would take its value from `{from}`, a provider's key, one of \
                         the harness's own or one `shell.env_remove` keeps from commands, and no \
                         server is given one"
                    )));
                }
                let found = parts
                    .inherited
                    .iter()
                    .find(|(name, _)| child_env::is_named(name, source, windows))
                    .map(|(_, value)| value.clone());
                let Some(value) = found else {
                    return Err(SpawnError::Refused(format!(
                        "`{shown}` takes its value from `{from}`, which is not set"
                    )));
                };
                if let Some(text) = value.to_str() {
                    secrets.push(Secret::new(text));
                }
                value
            }
            ValueSource::Literal(text) => {
                if text.expose().contains('\0') {
                    return Err(SpawnError::Refused(format!(
                        "the value of `{shown}` holds a NUL, which no variable can"
                    )));
                }
                OsString::from(text.expose())
            }
        };
        vars.retain(|(known, _)| !child_env::is_named(known, name, windows));
        vars.push((OsString::from(name), value));
    }

    // What the harness sets itself, over anything of the same name.
    let temp = parts.temp.as_os_str().to_owned();
    let mut set: Vec<(&str, OsString)> = if windows {
        vec![
            ("TEMP", temp.clone()),
            ("TMP", temp),
            ("PATHEXT", OsString::from(DEFAULT_PATHEXT)),
            (child_env::NO_CURRENT_DIRECTORY, OsString::from("1")),
        ]
    } else {
        vec![("TMPDIR", temp)]
    };
    if windows && let Some(cmd) = parts.cmd {
        set.push(("ComSpec", cmd.as_os_str().to_owned()));
    }
    for (name, value) in set {
        vars.retain(|(known, _)| !child_env::is_named(known, name, windows));
        vars.push((OsString::from(name), value));
    }
    if windows {
        // No `ComSpec` but the system's own: one inherited is not passed on.
        if parts.cmd.is_none() {
            vars.retain(|(known, _)| !child_env::is_named(known, "ComSpec", true));
        }
    }

    // `PATH`: absolute folders only, none in the workspace or a write root.
    child_env::one_absolute_path(&mut vars, windows);
    child_env::keep_out_of(&mut vars, parts.avoid, windows);
    Ok(ServerEnv { vars, secrets })
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

    fn entry(name: &str, value: ValueSource) -> EnvEntry {
        EnvEntry {
            name: name.to_owned(),
            value,
        }
    }

    fn from(variable: &str) -> ValueSource {
        ValueSource::Env(variable.to_owned())
    }

    fn literal(text: &str) -> ValueSource {
        ValueSource::Literal(Secret::new(text))
    }

    fn names(made: &ServerEnv) -> Vec<String> {
        let mut names: Vec<String> = made
            .vars
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn value<'a>(made: &'a ServerEnv, name: &str) -> Option<&'a OsStr> {
        made.vars
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.as_os_str())
    }

    fn inherited() -> Vec<(OsString, OsString)> {
        env(&[
            ("PATH", "/usr/bin:relative:/work/bin:/bin"),
            ("HOME", "/home/u"),
            ("USER", "u"),
            ("LANG", "C.UTF-8"),
            ("LC_ALL", "C"),
            ("TZ", "UTC"),
            ("ANTHROPIC_API_KEY", "sk-ant-key"),
            ("GATEWAY_KEY", "gw-key"),
            ("CINERIS_CONFIG", "/elsewhere"),
            ("AWS_SECRET_ACCESS_KEY", "aws"),
            ("SSH_AUTH_SOCK", "/run/agent"),
            ("LD_PRELOAD", "/x.so"),
            ("TMPDIR", "/tmp"),
            ("DOCS_TOKEN_SOURCE", "docs-token-value"),
            ("REMOVED_ONE", "r"),
        ])
    }

    fn parts<'a>(inherited: &'a [(OsString, OsString)], withheld: &'a [String]) -> EnvParts<'a> {
        EnvParts {
            inherited,
            withheld,
            avoid: &[],
            temp: Path::new("/state/mcp/1/tmp"),
            cmd: None,
            windows: false,
        }
    }

    /// The base on Unix is `PATH`, the locale, the home folder and the
    /// user's name, and `TMPDIR` the server's own: nothing else of the
    /// harness's environment, a provider's key, an agent's socket and a
    /// loader's variable among it.
    #[test]
    fn a_server_is_given_the_base_list_and_nothing_else() {
        let inherited = inherited();
        let made = server_env(&[], &parts(&inherited, &[])).unwrap();
        assert_eq!(
            names(&made),
            ["HOME", "LANG", "LC_ALL", "PATH", "TMPDIR", "TZ", "USER"]
        );
        assert_eq!(value(&made, "TMPDIR"), Some(OsStr::new("/state/mcp/1/tmp")));
        assert_eq!(
            value(&made, "PATH"),
            Some(OsStr::new("/usr/bin:/work/bin:/bin"))
        );
        assert!(made.secrets.is_empty());
    }

    /// Each entry is given: from the user's variable it names, read now,
    /// or as written; a value read from the environment comes back for the
    /// secret filter, and a literal does not.
    #[test]
    fn each_entry_is_given_from_its_source() {
        let inherited = inherited();
        let made = server_env(
            &[
                entry("DOCS_TOKEN", from("DOCS_TOKEN_SOURCE")),
                entry("MODE", literal("fast")),
                entry("LANG", literal("de_DE.UTF-8")),
            ],
            &parts(&inherited, &[]),
        )
        .unwrap();
        assert_eq!(
            value(&made, "DOCS_TOKEN"),
            Some(OsStr::new("docs-token-value"))
        );
        assert_eq!(value(&made, "MODE"), Some(OsStr::new("fast")));
        // An entry of a base variable's name is given in its place, once.
        assert_eq!(value(&made, "LANG"), Some(OsStr::new("de_DE.UTF-8")));
        assert_eq!(
            made.vars.iter().filter(|(name, _)| name == "LANG").count(),
            1
        );
        let secrets: Vec<&str> = made.secrets.iter().map(Secret::expose).collect();
        assert_eq!(secrets, ["docs-token-value"]);
    }

    /// A variable that is not set refuses the start, naming it; so does a
    /// source that is a provider's key, the harness's own, one the session
    /// took a credential from, or one `shell.env_remove` names.
    #[test]
    fn a_source_that_is_absent_or_withheld_refuses_the_start() {
        let inherited = inherited();
        let withheld = ["GATEWAY_KEY".to_owned(), "REMOVED_*".to_owned()];
        for (source, naming) in [
            ("NOT_SET_ANYWHERE", "not set"),
            ("ANTHROPIC_API_KEY", "provider"),
            ("CINERIS_CONFIG", "provider"),
            ("cineris_config", "provider"),
            ("GATEWAY_KEY", "provider"),
            ("REMOVED_ONE", "provider"),
        ] {
            let refused = server_env(
                &[entry("GIVEN", from(source))],
                &parts(&inherited, &withheld),
            )
            .unwrap_err();
            let SpawnError::Refused(why) = &refused else {
                panic!("{source}: {refused:?}");
            };
            assert!(why.contains(source) && why.contains(naming), "{why}");
        }
        // Withheld from the base too.
        let made = server_env(
            &[],
            &parts(
                &env(&[("PATH", "/bin"), ("LANG", "C")]),
                &["LANG".to_owned()],
            ),
        )
        .unwrap();
        assert_eq!(names(&made), ["PATH", "TMPDIR"]);
    }

    /// A name that cannot be a variable's, one of the harness's own, or one
    /// the harness sets itself refuses the start, naming it.
    #[test]
    fn a_name_that_cannot_be_given_refuses_the_start() {
        let inherited = inherited();
        for (name, naming) in [
            ("", "is empty"),
            ("A=B", "holds `=`"),
            ("A\0B", "holds a NUL"),
            ("1ST", "begins with a digit"),
            ("CINERIS_LOG", "the harness's own"),
            ("LD_PRELOAD", "the system's loader"),
            ("GLIBC_TUNABLES", "the system's loader"),
            ("DYLD_INSERT_LIBRARIES", "the system's loader"),
            ("Cineris_x", "the harness's own"),
            ("TMPDIR", "set by the harness"),
        ] {
            let refused =
                server_env(&[entry(name, literal("v"))], &parts(&inherited, &[])).unwrap_err();
            assert!(
                matches!(&refused, SpawnError::Refused(why) if why.contains(naming)),
                "{name:?}: {refused:?}"
            );
        }
        assert_eq!(bad_name("PATH"), None);
        assert_eq!(bad_name("_X1"), None);
        let refused =
            server_env(&[entry("X", literal("a\0b"))], &parts(&inherited, &[])).unwrap_err();
        assert!(matches!(refused, SpawnError::Refused(why) if why.contains("NUL")));
    }

    /// `PATH` keeps its absolute folders, and none inside the workspace or
    /// a write root, whether it came from the base or from an entry.
    #[test]
    fn path_keeps_absolute_folders_outside_the_workspace() {
        let scratch = cineris_testkit::Scratch::outside("cineris-core-mcp-env", "path");
        let work = scratch.dir("work");
        let tools = scratch.dir("tools");
        let path = format!(
            "{}:{}:relative:{}",
            work.join("node_modules/.bin").display(),
            tools.display(),
            work.display()
        );
        let inherited = env(&[("PATH", &path)]);
        let avoid = [work.clone()];
        let made = server_env(
            &[],
            &EnvParts {
                avoid: &avoid,
                ..parts(&inherited, &[])
            },
        )
        .unwrap();
        assert_eq!(value(&made, "PATH"), Some(tools.as_os_str()));
        let made = server_env(
            &[entry("PATH", literal(&path))],
            &EnvParts {
                avoid: &avoid,
                ..parts(&env(&[]), &[])
            },
        )
        .unwrap();
        assert_eq!(value(&made, "PATH"), Some(tools.as_os_str()));
    }

    /// On Windows the base is the system's own list, matched in any case;
    /// `TEMP` and `TMP` are the server's folder, `ComSpec` the system's
    /// `cmd.exe`, `PATHEXT` its default, and `cmd.exe` looks in no folder it
    /// runs in.
    #[test]
    fn on_windows_the_systems_own_and_the_harnesss_settings_are_given() {
        let inherited = env(&[
            ("Path", r"C:\Windows\System32;.;C:\tools"),
            ("SYSTEMROOT", r"C:\Windows"),
            ("USERPROFILE", r"C:\Users\u"),
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
            ("ComSpec", r"C:\evil\cmd.exe"),
            ("PATHEXT", ".PS1;.EXE"),
            ("OPENAI_API_KEY", "sk"),
            ("PSModulePath", r"C:\ps"),
            ("TEMP", r"C:\Users\u\AppData\Local\Temp"),
        ]);
        let made = server_env(
            &[],
            &EnvParts {
                inherited: &inherited,
                withheld: &[],
                avoid: &[],
                temp: Path::new(r"C:\state\mcp\1\tmp"),
                cmd: Some(Path::new(r"C:\Windows\System32\cmd.exe")),
                windows: true,
            },
        )
        .unwrap();
        assert_eq!(
            names(&made),
            [
                "APPDATA",
                "ComSpec",
                "NoDefaultCurrentDirectoryInExePath",
                "PATHEXT",
                "Path",
                "SYSTEMROOT",
                "TEMP",
                "TMP",
                "USERPROFILE"
            ]
        );
        assert_eq!(
            value(&made, "ComSpec"),
            Some(OsStr::new(r"C:\Windows\System32\cmd.exe"))
        );
        assert_eq!(value(&made, "PATHEXT"), Some(OsStr::new(DEFAULT_PATHEXT)));
        assert_eq!(value(&made, "TMP"), Some(OsStr::new(r"C:\state\mcp\1\tmp")));
        assert_eq!(
            value(&made, "Path"),
            Some(OsStr::new(r"C:\Windows\System32;C:\tools"))
        );
        // An entry of a name the harness sets is refused in any case.
        let refused = server_env(
            &[entry("pathext", literal(".PS1"))],
            &EnvParts {
                inherited: &inherited,
                withheld: &[],
                avoid: &[],
                temp: Path::new(r"C:\t"),
                cmd: None,
                windows: true,
            },
        )
        .unwrap_err();
        assert!(matches!(refused, SpawnError::Refused(why) if why.contains("set by the harness")));
    }
}
