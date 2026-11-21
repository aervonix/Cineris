//! How the harness starts git, wherever it starts it
//! (`docs/architecture/07-sessions-and-context.md`, "The base state";
//! `docs/architecture/06-permissions-and-sandbox.md`, "The harness's own
//! git"; `docs/security.md`, section 1).
//!
//! Two crates ask git about a workspace: the session store, for the base
//! state and a file's earlier content, and the shell tool, for what a
//! command changed. Neither can see the other, and until step 13 each held
//! its own list of the flags that keep git from writing to the repository
//! or running a program the repository's configuration names. The lists
//! had drifted apart: one turned hooks off and the other pinned the
//! prefixes of a patch. This is the one list, with everything either had,
//! and since the fixes of step 26 the one way to start that git: [`start`].
//!
//! **What no flag turns off** is a program that the repository's
//! configuration names for git to run: a filter for a kind of file, which
//! runs on `git diff` and `git status`; the transport of a fetch that a
//! partial clone makes when an object is missing; and more besides. That
//! is why both callers ask git about a file's content only in a project
//! the user trusts. And trust is not enough once commands are confined: a
//! confined command may write the repository's configuration, which lies
//! in the workspace, after the user trusted it. So [`start`] takes a
//! [`Confinement`], and in a session that confines commands git runs inside
//! the sandbox, reading only, with no network: whatever the configuration
//! makes it start is confined as the command was, and can change nothing
//! (found by the third review of step 26).
//!
//! **What such a program sees** is git's environment, so that is built, not
//! inherited ([`crate::child_env::for_harness`]): no provider key variable,
//! and a `PATH` of absolute directories only, so that a filter named by a
//! bare name is never a workspace file found through `.` or an empty entry.
//! Where nothing confines commands (no sandbox, `unrestricted`) the filter
//! runs with the user's rights, and before the harness's own processes'
//! fixes it ran with the keys too.

#![allow(
    clippy::disallowed_types,
    reason = "starts the harness's own git, which its callers run on the blocking pool"
)]

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// What every git command the harness starts begins with, before the
/// subcommand.
///
/// - `--no-optional-locks`: git writes nothing it does not have to, so a
///   look at the repository never refreshes its index behind the user.
/// - `core.fsmonitor=false`: no file-system monitor, which a repository's
///   configuration could name as any program.
/// - `core.hooksPath`: nowhere, so that no hook of the repository runs.
/// - `core.quotepath=off`: names come out as they are, not quoted.
/// - `diff.noprefix=false` and `diff.mnemonicPrefix=false`: a patch has its
///   usual `a/` and `b/` prefixes whatever the user's configuration says,
///   so that it applies.
///
/// A caller also sets [`GIT_ENV`] and closes standard input.
pub const GIT_HARDENING: [&str; 11] = [
    "--no-optional-locks",
    "-c",
    "core.fsmonitor=false",
    "-c",
    if cfg!(windows) {
        "core.hooksPath=NUL"
    } else {
        "core.hooksPath=/dev/null"
    },
    "-c",
    "core.quotepath=off",
    "-c",
    "diff.noprefix=false",
    "-c",
    "diff.mnemonicPrefix=false",
];

/// The environment every git command the harness starts is given: git
/// never asks for a password, takes no optional lock even where an alias
/// or a wrapper drops the flag, and never fetches a missing object of a
/// partial clone on its own (a lazy fetch would start the clone's
/// transport, which the repository's configuration names; the network is
/// off inside the sandbox, but this refuses it outright, before the
/// socket).
pub const GIT_ENV: [(&str, &str); 3] = [
    ("GIT_TERMINAL_PROMPT", "0"),
    ("GIT_OPTIONAL_LOCKS", "0"),
    ("GIT_NO_LAZY_FETCH", "1"),
];

/// The variables that tell git where a repository is, its index, its
/// objects and its command-line settings, which the harness's own git is
/// never given: what `git rev-parse --local-env-vars` lists (git's own
/// `local_repo_env`, which git takes out itself before it runs a command in
/// another repository, a submodule's). A harness started inside a git hook,
/// a `rebase --exec` or a `submodule foreach` inherits them from that git,
/// and its own git would then answer about that repository and index, not
/// the one the harness found by walking up from the workspace (the harness's
/// own processes' fixes; the third security fix). The user's own settings,
/// `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_NOSYSTEM`, `HOME` and
/// `XDG_CONFIG_HOME`, are not among them and pass as they came.
///
/// Written here, not asked of git at every start, which would cost a process
/// before each one; `git::tests::sys_the_local_variables_are_the_ones_git_lists`
/// compares it with the installed git's list.
pub const LOCAL_ENV_VARS: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// Whether `name` is one of [`LOCAL_ENV_VARS`], by the system's rule for a
/// variable's name (case folded on Windows).
fn is_local(name: &std::ffi::OsStr) -> bool {
    LOCAL_ENV_VARS
        .iter()
        .any(|local| crate::child_env::is_named(name, local, cfg!(windows)))
}

/// The setting that turns git's file-system monitor off, as a name and a
/// value: for a command the harness does not start itself and so cannot
/// give [`GIT_HARDENING`] (a shell command the model wrote), where it goes
/// into the environment as `GIT_CONFIG_KEY_<n>` and `GIT_CONFIG_VALUE_<n>`.
pub const NO_FSMONITOR: (&str, &str) = ("core.fsmonitor", "false");

/// Where the harness's own git runs: as it is, or inside the session's
/// sandbox. The core implements it over the sandbox's launcher; [`AsItIs`]
/// is for a harness whose commands are not confined.
pub trait Confinement: Send + Sync + std::fmt::Debug {
    /// Whether git started now runs inside the sandbox, so that a caller
    /// can say why git failed.
    fn confines(&self) -> bool;

    /// The git program the harness starts: the one found on `PATH`
    /// ([`crate::program::git`]), or, for a session that confines, one
    /// outside its write roots, so that a git a confined command planted
    /// inside a write root is never the one the harness runs (found by the
    /// third review of step 26). `None` when there is no git to run.
    fn git_program(&self) -> Option<PathBuf> {
        crate::program::git().map(Path::to_path_buf)
    }

    /// The variables git, and every program it starts, is not given,
    /// beside [`crate::child_env::KNOWN_KEYS`]: the providers' key
    /// variables as they are now ([`crate::child_env::Withheld`]). Required,
    /// so that every confinement says what it withholds: a filter a
    /// repository's configuration names runs with what git is given.
    fn withheld(&self) -> Vec<String>;

    /// The directories git, and every program it starts, is never found
    /// in: the workspace, and the write roots while commands are confined.
    /// Required for the same reason as [`Confinement::withheld`]: a filter
    /// git finds by a bare name is found on git's `PATH`, which leaves
    /// these out ([`crate::child_env::for_harness`]).
    fn avoid(&self) -> Vec<PathBuf>;

    /// The environment git's is built from, before the withheld variables
    /// go and `PATH` keeps its absolute directories
    /// ([`crate::child_env::for_harness`]): this process's own, unless a
    /// test gives one of its own.
    fn inherited(&self) -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }

    /// What to start in place of `git` with `arguments`.
    ///
    /// # Errors
    ///
    /// When git cannot be started as it must be. Then it is not started
    /// at all: an answer git did not give is one the caller does without.
    fn start(&self, git: &Path, arguments: Vec<OsString>) -> Result<Started, String>;
}

/// A program to start in place of git, and what it reads first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    /// The program: git itself, or the sandbox's launcher.
    pub program: PathBuf,
    /// Its arguments.
    pub arguments: Vec<OsString>,
    /// Bytes for its standard input, which then ends: the launcher reads
    /// its policy there. `None`: a closed input.
    pub stdin: Option<Vec<u8>>,
}

/// Git as it is: where commands are not confined (no sandbox on the
/// system, the `unrestricted` level), so nothing a command wrote got past a
/// boundary that the harness's git would then cross.
#[derive(Debug, Clone, Copy, Default)]
pub struct AsItIs;

impl Confinement for AsItIs {
    fn confines(&self) -> bool {
        false
    }

    /// No list beyond the known keys: a harness with a configuration gives
    /// its own confinement, which holds the configured keys.
    fn withheld(&self) -> Vec<String> {
        Vec::new()
    }

    /// No directory: where nothing is confined, and nothing is configured.
    fn avoid(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn start(&self, git: &Path, arguments: Vec<OsString>) -> Result<Started, String> {
        Ok(Started {
            program: git.to_owned(),
            arguments,
            stdin: None,
        })
    }
}

/// Why the harness's git was not started.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// There is no git in an absolute directory of `PATH`.
    #[error("git is not in an absolute directory of PATH")]
    NotInstalled,
    /// It could not be started as it must be.
    #[error("git could not be started inside the sandbox: {0}")]
    NotConfined(String),
    /// The system did not start it.
    #[error("git could not be started: {0}")]
    Io(#[from] std::io::Error),
}

/// Starts the harness's own git with `arguments`, in `dir`, inside
/// `confinement`: the one way every crate starts it.
///
/// Git is the one the confinement finds ([`Confinement::git_program`]),
/// started by that path, with [`GIT_HARDENING`] before `arguments`. Its
/// environment is built, not inherited: the confinement's
/// ([`Confinement::inherited`]) without the providers' key variables and
/// with a `PATH` of absolute directories only
/// ([`crate::child_env::for_harness`]), without the variables that name
/// another repository's place ([`LOCAL_ENV_VARS`]), then [`GIT_ENV`]. A program git
/// starts in turn (a filter a repository's configuration names) is given
/// the same, so it never sees a key, and never finds a workspace file by a
/// bare name through an empty or relative `PATH` entry. A launcher that
/// confines git passes its environment on as it was given. Its output is a
/// pipe for the caller to read; its standard error is thrown away; its
/// input is closed, after what the confinement gives it to read.
///
/// # Errors
///
/// When there is no git, when the confinement refuses, or when the system
/// cannot start it.
pub fn start(
    confinement: &dyn Confinement,
    dir: &Path,
    arguments: &[&str],
) -> Result<Child, StartError> {
    let git = confinement.git_program().ok_or(StartError::NotInstalled)?;
    let all: Vec<OsString> = GIT_HARDENING
        .iter()
        .chain(arguments)
        .map(OsString::from)
        .collect();
    let started = confinement
        .start(&git, all)
        .map_err(StartError::NotConfined)?;
    let mut env = crate::child_env::for_harness(
        confinement.inherited(),
        &confinement.withheld(),
        &confinement.avoid(),
        cfg!(windows),
    );
    // Git finds the repository from `dir`, as the harness did, never from
    // a place an outer git left in the environment.
    env.retain(|(name, _)| !is_local(name));
    let mut command = Command::new(&started.program);
    command
        .args(&started.arguments)
        .current_dir(dir)
        .env_clear()
        .envs(env)
        .envs(GIT_ENV)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(if started.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    crate::hook::started(started.program.as_os_str());
    let described = crate::hook::describe(&command);
    let mut child = command.spawn()?;
    crate::hook::spawned(child.id(), described);
    if let (Some(bytes), Some(mut input)) = (started.stdin, child.stdin.take()) {
        // A launcher reads its policy to the end before anything else, and
        // a policy is far smaller than a pipe holds. One that is gone
        // already has failed, and its exit says so.
        let _ = input.write_all(&bytes);
    }
    Ok(child)
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "the tests make a repository of their own in a scratch directory"
)]
mod tests {
    use super::*;

    /// A confinement that records what git it was asked to start, and
    /// starts git with arguments and an input of its own in its place:
    /// asked to hash what it reads, so that its answer shows the input
    /// arrived.
    #[derive(Debug, Default)]
    struct Recording {
        asked: std::sync::Mutex<Vec<Vec<OsString>>>,
        refuse: bool,
    }

    impl Confinement for Recording {
        fn confines(&self) -> bool {
            true
        }

        fn withheld(&self) -> Vec<String> {
            Vec::new()
        }

        fn avoid(&self) -> Vec<PathBuf> {
            Vec::new()
        }

        fn start(&self, git: &Path, arguments: Vec<OsString>) -> Result<Started, String> {
            if self.refuse {
                return Err("refused".to_owned());
            }
            self.asked.lock().unwrap().push(arguments);
            Ok(Started {
                program: git.to_owned(),
                arguments: ["hash-object", "--stdin"].map(OsString::from).to_vec(),
                stdin: Some(b"hello\n".to_vec()),
            })
        }
    }

    #[test]
    fn sys_git_is_started_as_its_confinement_says() {
        if crate::program::git().is_none() {
            assert!(std::env::var_os("CI").is_none(), "CI has git");
            return;
        }
        let recording = Recording::default();
        let child = start(&recording, &std::env::temp_dir(), &["status", "-z"]).unwrap();
        // Kept for the timeout report, by its id and the command line that
        // was started: the confinement's.
        let pid = child.id();
        let kept = crate::hook::spawned_children();
        let mine = kept.iter().find(|kept| kept.pid == pid).unwrap();
        assert!(mine.command.ends_with("hash-object --stdin"), "{mine:?}");
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success());
        // What the confinement started, with the input it gave.
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
        // What it was asked for: the hardening, then the arguments.
        let wanted: Vec<OsString> = GIT_HARDENING
            .iter()
            .chain(&["status", "-z"])
            .map(OsString::from)
            .collect();
        assert_eq!(*recording.asked.lock().unwrap(), [wanted]);

        // One that refuses: nothing starts.
        let refusing = Recording {
            refuse: true,
            ..Recording::default()
        };
        assert!(matches!(
            start(&refusing, &std::env::temp_dir(), &["status"]),
            Err(StartError::NotConfined(_))
        ));
        assert!(!AsItIs.confines());
    }

    /// Git as it is, from an environment the test gives, withholding what
    /// the test names: the core's confinement, without a sandbox.
    #[derive(Debug)]
    struct Given {
        inherited: Vec<(OsString, OsString)>,
        withheld: Vec<String>,
    }

    impl Confinement for Given {
        fn confines(&self) -> bool {
            false
        }

        fn withheld(&self) -> Vec<String> {
            self.withheld.clone()
        }

        fn avoid(&self) -> Vec<PathBuf> {
            Vec::new()
        }

        fn inherited(&self) -> Vec<(OsString, OsString)> {
            self.inherited.clone()
        }

        fn start(&self, git: &Path, arguments: Vec<OsString>) -> Result<Started, String> {
            AsItIs.start(git, arguments)
        }
    }

    /// A repository of its own, with `a.txt` sent through the filter `x`,
    /// whose clean command is `clean`. `None` when the system has no git,
    /// which only a desk without one may lack.
    fn filtered_repository(test: &str, clean: &str) -> Option<PathBuf> {
        use crate::rng::{Rng, SystemRng};
        let Some(git) = crate::program::git() else {
            assert!(std::env::var_os("CI").is_none(), "CI has git");
            return None;
        };
        let dir = std::env::temp_dir().join(format!(
            "cineris-base-git-{test}-{:016x}",
            SystemRng::new().next_u64()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let run = |arguments: &[&str]| {
            let status = Command::new(git)
                .args(arguments)
                .current_dir(&dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "git {arguments:?}");
        };
        run(&["init", "--quiet"]);
        run(&["config", "filter.x.clean", clean]);
        std::fs::write(dir.join(".gitattributes"), "a.txt filter=x\n").unwrap();
        std::fs::write(dir.join("a.txt"), "content\n").unwrap();
        Some(dir)
    }

    /// Asks the harness's git to hash `a.txt`, which runs its clean filter.
    fn hash_through_the_filter(confinement: &Given, dir: &Path) {
        let child = start(confinement, dir, &["hash-object", "a.txt"]).unwrap();
        // The answer does not matter: a filter that is not found or fails
        // leaves the content as it is.
        let _ = child.wait_with_output().unwrap();
    }

    /// A variable of this process's own that the test leaves out of what it
    /// gives git: one cargo or nextest sets for every test. A child that
    /// sees it was given this process's environment on top of the one
    /// built for it.
    fn a_variable_of_the_process() -> Option<String> {
        std::env::vars_os()
            .filter_map(|(name, _)| name.into_string().ok())
            .find(|name| name.starts_with("NEXTEST") || name.starts_with("CARGO_"))
    }

    /// A key variable in the harness's environment never reaches a filter
    /// driver the repository's configuration names (found by the shell
    /// environment's fixes: `git::start` passed the harness's whole
    /// environment). The same filter, given the variable unwithheld, does
    /// see it: so the filter ran, and it is the list that kept it out.
    #[test]
    fn sys_a_filter_driver_is_given_no_key_variable() {
        const KEY: &str = "CINERIS_TEST_PROVIDER_KEY";
        let process = a_variable_of_the_process();
        let extra = process.as_deref().unwrap_or("CINERIS_TEST_NOT_SET");
        let clean = format!(
            "printf '%s %s %s\\n' \"${{{KEY}-unset}}\" \"${{ANTHROPIC_API_KEY-unset}}\" \
             \"${{{extra}-unset}}\" > .git/seen; cat"
        );
        let Some(dir) = filtered_repository("keys", &clean) else {
            return;
        };
        let mut inherited: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| name.to_str() != Some(extra))
            .collect();
        inherited.push((KEY.into(), "sk-configured-key".into()));
        inherited.push(("ANTHROPIC_API_KEY".into(), "sk-known-key".into()));
        let seen = |withheld: &[&str]| {
            let _ = std::fs::remove_file(dir.join(".git").join("seen"));
            let given = Given {
                inherited: inherited.clone(),
                withheld: withheld.iter().map(|name| (*name).to_owned()).collect(),
            };
            hash_through_the_filter(&given, &dir);
            std::fs::read_to_string(dir.join(".git").join("seen"))
                .unwrap_or_else(|_| panic!("the filter did not run"))
        };
        assert_eq!(
            seen(&[KEY]).trim(),
            "unset unset unset",
            "a key variable, or a variable of the harness the environment given to git left \
             out, reached a filter git ran"
        );
        // Not withheld, the configured key reaches it: the filter ran, and
        // saw what it was given. The known one goes whatever the list.
        assert_eq!(seen(&[]).trim(), "sk-configured-key unset unset");
        if process.is_none() {
            assert!(std::env::var_os("CI").is_none(), "{extra}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A filter named by a bare name is never found through an empty or
    /// relative `PATH` entry, which names the directory git runs in: the
    /// workspace. The same program, through an absolute entry, runs.
    #[cfg(unix)]
    #[test]
    fn sys_a_relative_path_entry_does_not_find_a_filter() {
        use std::os::unix::fs::PermissionsExt;
        let name = "cineris-planted-filter";
        let Some(dir) = filtered_repository("relative", name) else {
            return;
        };
        let planted = dir.join(name);
        std::fs::write(&planted, "#!/bin/sh\ntouch .git/planted\ncat\n").unwrap();
        std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).unwrap();
        let marker = dir.join(".git").join("planted");
        let real = std::env::var("PATH").unwrap_or_default();
        let ran = |path: String| {
            let _ = std::fs::remove_file(&marker);
            let mut inherited: Vec<(OsString, OsString)> = std::env::vars_os()
                .filter(|(name, _)| name != "PATH")
                .collect();
            inherited.push(("PATH".into(), path.into()));
            let given = Given {
                inherited,
                withheld: Vec::new(),
            };
            hash_through_the_filter(&given, &dir);
            marker.exists()
        };
        for path in [
            format!(".:{real}"),
            format!(":{real}"),
            format!("{real}:"),
            format!("./:{real}"),
            ".".to_owned(),
            String::new(),
        ] {
            assert!(
                !ran(path.clone()),
                "git found the workspace's {name} through `PATH={path}`"
            );
        }
        // Named by an absolute entry, it runs: the filter is one git runs.
        assert!(ran(format!("{}:{real}", dir.display())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A harness started inside an outer git (a hook, a `rebase --exec`)
    /// inherits that git's place: `GIT_DIR` and `GIT_WORK_TREE` name another
    /// repository, `GIT_INDEX_FILE` its index, `GIT_CONFIG_PARAMETERS` a
    /// setting given on that git's command line. The harness's git answers
    /// about the repository it is started in all the same, and still reads
    /// the user's own configuration, which `GIT_CONFIG_GLOBAL` names (the
    /// third security fix; `outstanding`'s
    /// `the-harness-git-reads-the-repository-git-names-in-the-environment`).
    #[test]
    fn sys_git_answers_about_its_own_repository_whatever_the_environment_names() {
        use crate::rng::{Rng, SystemRng};
        let Some(git) = crate::program::git() else {
            assert!(std::env::var_os("CI").is_none(), "CI has git");
            return;
        };
        let base = std::env::temp_dir().join(format!(
            "cineris-base-git-place-{:016x}",
            SystemRng::new().next_u64()
        ));
        let ours = base.join("ours");
        let other = base.join("other");
        for dir in [&ours, &other] {
            std::fs::create_dir_all(dir).unwrap();
            let status = Command::new(git)
                .args(["init", "--quiet"])
                .current_dir(dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap();
            assert!(status.success());
        }
        let global = base.join("global.gitconfig");
        std::fs::write(&global, "[cineris]\n\tmark = global\n").unwrap();
        let other_git = other.join(".git");
        let mut inherited: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| !is_local(name) && name != "GIT_CONFIG_GLOBAL")
            .collect();
        inherited.extend([
            ("GIT_DIR".into(), other_git.clone().into_os_string()),
            ("GIT_WORK_TREE".into(), other.clone().into_os_string()),
            (
                "GIT_INDEX_FILE".into(),
                other_git.join("index").into_os_string(),
            ),
            (
                "GIT_CONFIG_PARAMETERS".into(),
                "'cineris.mark=parameters'".into(),
            ),
            ("GIT_CONFIG_GLOBAL".into(), global.clone().into_os_string()),
        ]);
        let given = Given {
            inherited,
            withheld: Vec::new(),
        };
        let answer = |arguments: &[&str]| -> String {
            let output = start(&given, &ours, arguments)
                .unwrap()
                .wait_with_output()
                .unwrap();
            assert!(output.status.success(), "git {arguments:?}");
            String::from_utf8(output.stdout).unwrap().trim().to_owned()
        };
        let top = PathBuf::from(answer(&["rev-parse", "--show-toplevel"]));
        assert!(
            crate::same_location(
                &std::fs::canonicalize(&top).unwrap(),
                &std::fs::canonicalize(&ours).unwrap()
            ),
            "the harness's git answered about {} for a start in {}",
            top.display(),
            ours.display()
        );
        assert_eq!(
            answer(&["rev-parse", "--git-path", "index"]),
            ".git/index",
            "the harness's git took the index the environment named"
        );
        assert_eq!(
            answer(&["config", "--get", "cineris.mark"]),
            "global",
            "an outer git's command-line setting reached the harness's git, or the user's own \
             configuration did not"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// [`LOCAL_ENV_VARS`] holds every variable the installed git lists as
    /// one of a repository's place: a git that adds one fails here, naming
    /// it. A name the list has that this git does not list is said, not
    /// failed: an older git lists fewer.
    #[test]
    fn sys_the_local_variables_are_the_ones_git_lists() {
        let Some(git) = crate::program::git() else {
            assert!(std::env::var_os("CI").is_none(), "CI has git");
            return;
        };
        let output = Command::new(git)
            .args(["rev-parse", "--local-env-vars"])
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        let listed: std::collections::BTreeSet<&str> = text.lines().map(str::trim).collect();
        let is_variable = |name: &&str| {
            name.starts_with("GIT_")
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte == b'_')
        };
        if !output.status.success() || listed.is_empty() || !listed.iter().all(is_variable) {
            eprintln!(
                "skipped: this git ({}) is too old to list the variables of a repository's \
                 place (`git rev-parse --local-env-vars` printed {text:?}), so LOCAL_ENV_VARS \
                 was not compared with it",
                git.display()
            );
            assert!(
                std::env::var_os("CI").is_none(),
                "CI's git lists the variables of a repository's place"
            );
            return;
        }
        let ours: std::collections::BTreeSet<&str> = LOCAL_ENV_VARS.into_iter().collect();
        let missing: Vec<&&str> = listed.difference(&ours).collect();
        let extra: Vec<&&str> = ours.difference(&listed).collect();
        assert!(
            missing.is_empty(),
            "git lists variables of a repository's place that LOCAL_ENV_VARS leaves out, so the \
             harness's git would still take them from its environment: missing {missing:?}; \
             extra (in LOCAL_ENV_VARS, not listed by this git) {extra:?}"
        );
        if !extra.is_empty() {
            eprintln!(
                "LOCAL_ENV_VARS has variables this git does not list (an older git): {extra:?}"
            );
        }
    }

    #[test]
    fn the_list_is_one_flag_and_then_settings() {
        assert_eq!(GIT_HARDENING[0], "--no-optional-locks");
        let settings: Vec<&str> = GIT_HARDENING[1..]
            .chunks(2)
            .map(|pair| {
                assert_eq!(pair[0], "-c", "every setting is given with -c");
                pair[1]
            })
            .collect();
        for wanted in [
            "core.fsmonitor=false",
            "core.quotepath=off",
            "diff.noprefix=false",
            "diff.mnemonicPrefix=false",
        ] {
            assert!(settings.contains(&wanted), "{wanted}");
        }
        assert!(
            settings
                .iter()
                .any(|setting| setting.starts_with("core.hooksPath=")),
            "hooks are turned off"
        );
        // The monitor is off by the same name in a command's environment.
        assert_eq!(
            format!("{}={}", NO_FSMONITOR.0, NO_FSMONITOR.1),
            "core.fsmonitor=false"
        );
    }
}
