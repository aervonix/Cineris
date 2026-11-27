//! The harness's [`Spawner`]: a local server started as the harness starts
//! a program (`docs/steps/30/answered.md`, points 3.1 to 3.3; the
//! maintainers' changes after the design's independent review).
//!
//! 1. **The program** is found as an extension's program is
//!    (`crate::programs`): a bare name in `PATH`'s absolute folders outside
//!    the workspace and the write roots (on Windows `.exe`, `.cmd`, `.bat`
//!    a folder at a time, and nothing else of `PATHEXT`); an absolute path
//!    as named, **refused when it is inside the workspace or a write
//!    root**, where the model could have written it; a relative path never.
//!    On Windows a `.cmd` or `.bat` (`npx.cmd`) runs through
//!    `%SystemRoot%\System32\cmd.exe` with one line built here, and an
//!    argument it cannot carry refuses the start, named; an argument that
//!    holds a look-alike of a quote, a backslash or a hyphen is refused on
//!    every system.
//! 2. **Its own folder,** made for each start and removed when it ends:
//!    a private temporary folder, its `TMPDIR` (`TEMP` and `TMP` on
//!    Windows), and an empty private folder to run in.
//! 3. **Where it runs:** the workspace only when the project is trusted,
//!    and otherwise its empty folder, so that what a repository holds
//!    (`./node_modules/.bin`, a module `python -m` imports, a program
//!    `cmd.exe` finds where it runs) is never run through a server of the
//!    user's in a project that is not trusted. An entry's `cwd` that
//!    resolves inside the workspace counts as the workspace; one outside it
//!    is taken as it is.
//! 4. **Its environment** is the explicit list ([`super::env`]); each value
//!    read from the user's environment is given to the session's secret
//!    filter.
//! 5. **Its confinement,** where the session's sandbox enforces, at every
//!    level, `unrestricted` included, as an extension's program's: the
//!    launcher's live input ([`cineris_sandbox::launch::Launcher::wrap_live`]),
//!    the session's reads, its private temporary folder its only write
//!    root, and the network only with `network = true`. **It writes
//!    nothing else in this release:** an entry that asks to write the
//!    workspace is refused. Where the sandbox does not enforce it runs
//!    unconfined, as a hook command does. **The live mode's rules for its
//!    caller** (the early review of the launcher's live input): the policy
//!    is written whole before any line, since the input's thread writes it
//!    first and the client's lines queue behind it; a launcher's line with
//!    its exit code 125 is only how the server ended, and never a reason to
//!    start it unconfined (a server can print that line itself); and the
//!    environment, set on the launcher's own process before it confines
//!    itself, holds no variable of the system's loader
//!    ([`super::env`]).
//! 6. **Its process** ([`super::process`]): a tree on a thread of its own,
//!    its three pipes, and its staged end.
//!
//! It blocks: it looks at the file system and starts a process. The
//! harness calls it off its core thread.

#![allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "starting a server looks at the file system and starts a process; it is called off \
              the core thread"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use cineris_base::{SecretFilter, SharedTimer};
use cineris_ext::manifest::Program;
use cineris_mcp::{Launch, ServerDefinition, ServerPipes, SpawnError, Spawner};
use cineris_sandbox::launch::Launcher;
use cineris_sandbox::policy::{Network, Policy, Reach};
use cineris_sandbox::tree::Guard;
use cineris_tools::process::{Argument, CommandLine};

use super::env::{EnvParts, server_env};
use super::process::{self, Prepared};
use crate::programs::{self, Search};

/// How a server is confined where the session's sandbox enforces: the
/// launcher to start (the harness's own executable and its hidden
/// subcommand), what the session's commands may read, and the Landlock
/// version the session counted on.
#[derive(Debug, Clone)]
pub struct Confinement {
    /// The launcher's program.
    pub program: PathBuf,
    /// Its arguments before the launcher's own: the hidden subcommand.
    pub prefix: Vec<OsString>,
    /// What the server may read: the session's reads.
    pub read: Reach,
    /// The Landlock version the session's sandbox enforces with.
    pub landlock: u8,
}

/// What a session's servers are started with.
pub struct SpawnerOptions {
    /// The workspace, as the session's paths give it.
    pub workspace: PathBuf,
    /// Whether the user trusts the project: a server runs in the workspace
    /// only then.
    pub trusted: bool,
    /// Where each server's own folder is made: a private folder of the
    /// harness's, never inside the workspace or a write root, where a
    /// command could put something in a server's way.
    pub folders: PathBuf,
    /// How a server is confined, where the session's sandbox enforces.
    pub confine: Option<Confinement>,
    /// The session's write roots besides the workspace: where no program is
    /// taken from, and which `PATH` leaves out.
    pub write_roots: Vec<PathBuf>,
    /// The environment a server's is made from, when it is not this
    /// process's own (a test's).
    pub inherited: Option<Vec<(OsString, OsString)>>,
    /// The variables no server is given besides the providers' known keys:
    /// those the harness took a credential from, and `shell.env_remove`'s.
    pub withheld: Vec<String>,
    /// The session's secret filter: every value read from the user's
    /// environment for a server is masked by it from then on, and a
    /// server's standard error is shown through it.
    pub filter: Arc<RwLock<SecretFilter>>,
    /// The guard of process trees, which kills a server's if the harness
    /// dies.
    pub guard: Option<Guard>,
    /// What a server's end waits on.
    pub timer: SharedTimer,
}

impl std::fmt::Debug for SpawnerOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnerOptions")
            .field("workspace", &self.workspace)
            .field("trusted", &self.trusted)
            .field("folders", &self.folders)
            .field("confine", &self.confine)
            .field("write_roots", &self.write_roots)
            .field("withheld", &self.withheld)
            .finish_non_exhaustive()
    }
}

/// `path` in the plain form a program is given a folder in: on Windows
/// `C:\…` or `\\server\share\…`, never the extended-length `\\?\…` form
/// `canonicalize` gives there, which `cmd.exe` cannot run in (it falls back
/// to the Windows folder) and many programs cannot read. Elsewhere as it is.
fn plain(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    if let Some(text) = path.to_str() {
        if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{share}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\")
            && rest.as_bytes().get(1) == Some(&b':')
        {
            return PathBuf::from(rest);
        }
    }
    path
}

/// Starts a session's local servers, as the [module](self) says.
#[derive(Debug)]
pub struct ServerSpawner {
    options: SpawnerOptions,
    /// The number of the next server folder.
    next: AtomicU64,
}

/// A started server's own folders.
struct Own {
    /// The folder that holds the other two, removed when the server ends.
    root: PathBuf,
    /// Its private temporary folder, its only write root.
    temp: PathBuf,
    /// The empty folder it runs in where the workspace is not given.
    work: PathBuf,
}

impl ServerSpawner {
    /// A spawner with `options`.
    #[must_use]
    pub fn new(options: SpawnerOptions) -> Self {
        Self {
            options,
            next: AtomicU64::new(0),
        }
    }

    /// Where no program is taken from: the workspace and the write roots.
    fn avoid(&self) -> Vec<PathBuf> {
        let mut avoid = vec![self.options.workspace.clone()];
        avoid.extend(self.options.write_roots.iter().cloned());
        avoid
    }

    /// Makes a server's own folders, each open to the user alone.
    fn own_folders(&self) -> Result<Own, SpawnError> {
        let failed = |error: std::io::Error| {
            SpawnError::Failed(format!(
                "the server's own folder could not be made: {error}"
            ))
        };
        cineris_base::private::create_dir_all(&self.options.folders).map_err(failed)?;
        let base = plain(std::fs::canonicalize(&self.options.folders).map_err(failed)?);
        let avoid = cineris_base::program::resolved(&self.avoid());
        if cineris_base::program::passes_through(&base, &avoid) {
            return Err(SpawnError::Failed(
                "the servers' folders are inside the workspace or a writable root, where a \
                 command could put something in a server's way"
                    .to_owned(),
            ));
        }
        let mut tries = 0;
        let root = loop {
            let number = self.next.fetch_add(1, Ordering::Relaxed);
            let root = base.join(format!("{}-{number}", std::process::id()));
            match crate::sandbox::make_private_dir(&root) {
                Ok(()) => break root,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && tries < 64 => {
                    tries += 1;
                }
                Err(error) => return Err(failed(error)),
            }
        };
        let own = Own {
            temp: root.join("tmp"),
            work: root.join("work"),
            root,
        };
        for folder in [&own.temp, &own.work] {
            if let Err(error) = crate::sandbox::make_private_dir(folder) {
                let _ = std::fs::remove_dir_all(&own.root);
                return Err(failed(error));
            }
        }
        Ok(own)
    }

    /// The folder a server whose entry gives `asked` runs in: the
    /// workspace, or the entry's folder (relative to the workspace, or
    /// under the home folder with a leading `~`); its own empty folder
    /// `work` in place of one inside the workspace while the project is not
    /// trusted.
    fn working_folder(
        &self,
        asked: Option<&str>,
        work: &Path,
        inherited: &[(OsString, OsString)],
    ) -> Result<PathBuf, SpawnError> {
        let workspace = &self.options.workspace;
        let path = match asked {
            None => workspace.clone(),
            Some(text) => {
                let rest = if text == "~" {
                    Some("")
                } else {
                    text.strip_prefix("~/")
                        .or_else(|| cfg!(windows).then(|| text.strip_prefix("~\\")).flatten())
                };
                let named = match rest {
                    Some(rest) => {
                        let home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
                        let home = inherited
                            .iter()
                            .find(|(name, _)| {
                                cineris_base::child_env::is_named(name, home, cfg!(windows))
                            })
                            .map(|(_, value)| PathBuf::from(value))
                            .filter(|home| home.is_absolute())
                            .ok_or_else(|| {
                                SpawnError::Refused(format!(
                                    "its folder `{}` names the home folder, and there is none",
                                    cineris_base::in_code(text)
                                ))
                            })?;
                        home.join(rest)
                    }
                    None => PathBuf::from(text),
                };
                if named.is_absolute() {
                    named
                } else {
                    workspace.join(named)
                }
            }
        };
        let real = std::fs::canonicalize(&path).ok();
        let workspace_real = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.clone());
        let inside = cineris_base::location_within(&path, workspace)
            || real
                .as_deref()
                .is_some_and(|real| cineris_base::location_within(real, &workspace_real));
        if inside && !self.options.trusted {
            return Ok(work.to_path_buf());
        }
        match real {
            Some(real) if real.is_dir() => Ok(plain(real)),
            _ => Err(SpawnError::Refused(format!(
                "its folder `{}` is not a folder",
                path.display()
            ))),
        }
    }

    /// Everything but the start, for `server`, in its own folders `own`.
    fn prepare(
        &self,
        server: &ServerDefinition,
        inherited: &[(OsString, OsString)],
        own: &Own,
    ) -> Result<Prepared, SpawnError> {
        let Launch::Local {
            program,
            args,
            cwd,
            env,
            network,
            write,
        } = &server.launch
        else {
            return Err(SpawnError::Refused(format!(
                "`{}` is a remote server, and is not started as a process",
                cineris_base::in_code(&server.name)
            )));
        };
        let shown = cineris_base::in_code(program).into_owned();
        if *write {
            return Err(SpawnError::Refused(
                "a server is not given the workspace to write in this release: it writes only its \
                 own temporary folder"
                    .to_owned(),
            ));
        }
        let windows = cfg!(windows);
        let avoid = self.avoid();
        let resolved_avoid = cineris_base::program::resolved(&avoid);

        // 1. The program, and on Windows the `cmd.exe` a script needs.
        let named = Program::parse(program.clone())
            .map_err(|(shown, why)| SpawnError::Refused(format!("the command {shown}{why}")))?;
        if let Program::Path(path) = &named
            && cineris_base::program::passes_through(Path::new(path), &resolved_avoid)
        {
            return Err(SpawnError::Refused(format!(
                "the command `{shown}` is inside the workspace or a writable root, where the \
                 model could have written it, so it is not started"
            )));
        }
        let search_path: Vec<PathBuf> = inherited
            .iter()
            .find(|(name, _)| cineris_base::child_env::is_named(name, "PATH", windows))
            .map(|(_, value)| std::env::split_paths(value).collect())
            .unwrap_or_default();
        let found = programs::find(
            &named,
            &Search {
                path: &search_path,
                avoid: &avoid,
                // A path inside one was refused above, confined or not.
                confined_roots: &[],
                windows,
            },
        )
        .map_err(|why| match &named {
            Program::Name(_) => SpawnError::NotFound(why),
            Program::Path(path) if !cineris_base::program::is_program(Path::new(path), windows) => {
                SpawnError::NotFound(why)
            }
            Program::Path(_) => SpawnError::Refused(why),
        })?;
        let cmd = if windows {
            programs::system_cmd(inherited, &avoid)
        } else {
            None
        };
        let line = programs::command_line(&found, args, windows, cmd.as_deref())
            .map_err(SpawnError::Refused)?;

        // 3. Where it runs, and 4. its environment.
        let directory = self.working_folder(cwd.as_deref(), &own.work, inherited)?;
        let made = server_env(
            env,
            &EnvParts {
                inherited,
                withheld: &self.options.withheld,
                avoid: &avoid,
                temp: &own.temp,
                cmd: cmd.as_deref(),
                windows,
            },
        )?;

        // 5. Its confinement.
        let (line, preamble, confined) = match &self.options.confine {
            Some(confine) => {
                let launcher = Launcher::new(
                    confine.program.clone(),
                    confine.prefix.clone(),
                    Policy {
                        read: confine.read.clone(),
                        write: vec![own.temp.clone()],
                        network: if *network {
                            Network::Full
                        } else {
                            Network::Off
                        },
                        temp: Some(own.temp.clone()),
                        landlock: confine.landlock,
                    },
                );
                let arguments = line
                    .arguments
                    .iter()
                    .map(|argument| match argument {
                        Argument::Exact(text) => Ok(OsString::from(text)),
                        // Only a Windows line, where nothing is confined.
                        Argument::Verbatim(_) => Err(SpawnError::Failed(
                            "a command line written for Windows cannot be confined".to_owned(),
                        )),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let wrapped = launcher.wrap_live(line.program.as_os_str(), &arguments);
                let mut confined = CommandLine::new(wrapped.program);
                confined.arguments = wrapped
                    .arguments
                    .into_iter()
                    .map(|argument| {
                        argument.into_string().map(Argument::Exact).map_err(|_| {
                            SpawnError::Failed(
                                "a command line that is not text cannot be confined".to_owned(),
                            )
                        })
                    })
                    .collect::<Result<_, _>>()?;
                (confined, wrapped.stdin, true)
            }
            None => (line, Vec::new(), false),
        };

        // What it was given from the user's environment is masked from now.
        {
            let mut filter = self
                .options
                .filter
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            for secret in &made.secrets {
                filter.add_value(secret);
            }
        }

        let mut command = std::process::Command::new(&line.program);
        for argument in &line.arguments {
            match argument {
                Argument::Exact(text) => {
                    command.arg(text);
                }
                Argument::Verbatim(text) => {
                    #[cfg(windows)]
                    {
                        use std::os::windows::process::CommandExt;
                        command.raw_arg(text);
                    }
                    #[cfg(not(windows))]
                    command.arg(text);
                }
            }
        }
        command.current_dir(&directory).env_clear().envs(made.vars);
        Ok(Prepared {
            command,
            preamble,
            confined,
            shown,
            folder: own.root.clone(),
        })
    }
}

impl Spawner for ServerSpawner {
    fn spawn(&self, server: &ServerDefinition) -> Result<ServerPipes, SpawnError> {
        let inherited = self
            .options
            .inherited
            .clone()
            .unwrap_or_else(|| std::env::vars_os().collect());
        let own = self.own_folders()?;
        let prepared = match self.prepare(server, &inherited, &own) {
            Ok(prepared) => prepared,
            Err(error) => {
                let _ = std::fs::remove_dir_all(&own.root);
                return Err(error);
            }
        };
        process::start(
            prepared,
            self.options.guard.clone(),
            Arc::clone(&self.options.filter),
            Arc::clone(&self.options.timer),
        )
    }
}
