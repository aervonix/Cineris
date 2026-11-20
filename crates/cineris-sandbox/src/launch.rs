//! The launcher: the harness's own executable, under a hidden subcommand,
//! which applies the policy to itself and then becomes the command
//! (`docs/architecture/06-permissions-and-sandbox.md`, "How it is applied").
//!
//! - [`Launcher`] is the harness's side: what to start in place of a
//!   command, and the bytes of the policy for that process's standard
//!   input.
//! - [`run`] is the launcher's side, the body of the hidden subcommand. It
//!   reads the policy to its end, confines itself, gives the command a
//!   closed input (or the input that followed the policy, [`input`
//!   below](#a-commands-input)) and the session's temporary directory, and
//!   replaces itself with the command.
//!
//! **It fails closed.** Once the harness has said the sandbox enforces, a
//! policy that cannot be read, a Landlock rule the kernel refuses, a
//! seccomp filter that does not load, or a build without the backends
//! stops the launcher before the command runs: it writes one line that
//! begins with [`MARK`] and exits with [`FAILED`]. Nothing is run
//! unconfined in its place.
//!
//! The launcher is a fresh, single-threaded process, so nothing runs
//! between `fork` and `exec` of a multi-threaded harness. The process tree
//! (its session, its group, the death signal, the guard) is set up by the
//! runner before the launcher starts, so the launcher does not touch it.
//!
//! # A command's input
//!
//! The launcher's standard input carries the policy, so a command it
//! starts cannot be given another. A policy with an `input` field
//! ([`crate::policy`], "The wire form") is followed by the command's input,
//! at most [`MAX_INPUT`] bytes: an extension's `cineris.exec` with `stdin`,
//! and a hook command's event (step 28). The launcher puts those bytes in
//! an anonymous memory file **before it confines itself**, made
//! close-on-exec, written, sealed against every write, growth and shrink
//! and against further seals, and rewound, and the command is given it as
//! its standard input in place of the closed one. The command can read it
//! and cannot change it; nothing else of it is left open. Without `input`
//! the command's input is closed, as it always was.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::explain;
use crate::policy::{MAX_BYTES, MAX_INPUT, Policy, PolicyError};

/// What every line the launcher writes about itself begins with.
pub const MARK: &str = "cineris-sandbox:";

/// The exit code of a launcher that did not start the command because the
/// sandbox could not be applied. The same code `env` and `timeout` use for
/// a failure of their own.
pub const FAILED: i32 = 125;

/// The harness's side of the launcher: one per session, holding the
/// session's policy.
#[derive(Debug, Clone)]
pub struct Launcher {
    program: PathBuf,
    prefix: Vec<OsString>,
    policy: Policy,
    encoded: Vec<u8>,
}

/// What to start in place of a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wrapped {
    /// The program: the harness's own executable.
    pub program: PathBuf,
    /// Its arguments: the hidden subcommand, `--`, and the command.
    pub arguments: Vec<OsString>,
    /// The policy, for the program's standard input.
    pub stdin: Vec<u8>,
}

impl Launcher {
    /// A launcher that starts `program` with `prefix` (the harness's own
    /// executable and its hidden subcommand, whose body is [`run`]) to
    /// apply `policy`.
    #[must_use]
    pub fn new(
        program: impl Into<PathBuf>,
        prefix: impl IntoIterator<Item = impl Into<OsString>>,
        policy: Policy,
    ) -> Self {
        let encoded = policy.encode();
        Self {
            program: program.into(),
            prefix: prefix.into_iter().map(Into::into).collect(),
            policy,
            encoded,
        }
    }

    /// The policy it applies.
    #[must_use]
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// What to start in place of `program` with `arguments`.
    #[must_use]
    pub fn wrap(&self, program: &OsStr, arguments: &[OsString]) -> Wrapped {
        // The launcher's start is this program's: the launcher becomes it,
        // and its own process is given no environment of the test's to say
        // so itself.
        cineris_base::hook::launched(self.program.as_os_str(), program);
        let mut all = self.prefix.clone();
        all.push(OsString::from("--"));
        all.push(program.to_owned());
        all.extend(arguments.iter().cloned());
        Wrapped {
            program: self.program.clone(),
            arguments: all,
            stdin: self.encoded.clone(),
        }
    }

    /// What to start in place of `program` with `arguments`, with `input`
    /// as the command's standard input: it follows the policy on the
    /// launcher's own ([`crate::policy`], "The wire form").
    ///
    /// # Errors
    ///
    /// When `input` is longer than [`MAX_INPUT`].
    pub fn wrap_with_input(
        &self,
        program: &OsStr,
        arguments: &[OsString],
        input: &[u8],
    ) -> Result<Wrapped, PolicyError> {
        let stdin = self.policy.encode_with_input(input)?;
        Ok(Wrapped {
            stdin,
            ..self.wrap(program, arguments)
        })
    }

    /// One line for the model about what the sandbox did to a command that
    /// has ended, read from its exit code and its output: which operation
    /// it blocked, or why the command did not run. `None` when there is
    /// nothing to say.
    #[must_use]
    pub fn explain(&self, code: Option<i32>, output: &str) -> Option<String> {
        explain::explain(&self.policy, code, output)
    }
}

/// The body of the hidden subcommand: `arguments` are what follows it,
/// `--` and then the command.
///
/// It returns only when the command could not be started, with the exit
/// code to end with, after writing why on its standard error, which is the
/// command's output.
#[must_use]
pub fn run(arguments: Vec<OsString>) -> i32 {
    match launch(arguments) {
        Ok(never) => match never {},
        Err(Failure { code, message }) => {
            // The command's output is this process's standard error: the
            // line reaches the result.
            #[expect(clippy::print_stderr, reason = "the launcher's one line is its output")]
            {
                eprintln!("{MARK} {message}");
            }
            code
        }
    }
}

/// Why the launcher did not become the command.
struct Failure {
    code: i32,
    message: String,
}

impl Failure {
    fn sandbox(message: impl std::fmt::Display) -> Self {
        Self {
            code: FAILED,
            message: format!(
                "the sandbox could not be applied, so the command was not run: {message}"
            ),
        }
    }
}

/// The uninhabited type of a launch that succeeded: the process is the
/// command now.
enum Never {}

fn launch(arguments: Vec<OsString>) -> Result<Never, Failure> {
    let mut arguments = arguments.into_iter();
    if arguments.next().as_deref() != Some(OsStr::new("--")) {
        return Err(Failure::sandbox("the launcher was not given a command"));
    }
    let Some(program) = arguments.next() else {
        return Err(Failure::sandbox("the launcher was not given a command"));
    };
    let rest: Vec<OsString> = arguments.collect();

    let mut bytes = Vec::new();
    std::io::stdin()
        .lock()
        .take(u64::try_from(MAX_BYTES + MAX_INPUT).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Failure::sandbox(format!("the policy could not be read: {error}")))?;
    let (policy, input) = Policy::decode_with_input(&bytes).map_err(Failure::sandbox)?;
    drop(bytes);
    // Made before the launcher is confined, which may refuse what making
    // it takes.
    let input = input
        .map(|input| input_file(&input))
        .transpose()
        .map_err(|error| {
            Failure::sandbox(format!("the command's input could not be given: {error}"))
        })?;

    confine(&policy).map_err(Failure::sandbox)?;
    become_command(&program, &rest, policy.temp.as_deref(), input)
}

/// The command's input, in an anonymous memory file: close-on-exec,
/// written whole, sealed against writing, growing, shrinking and further
/// seals, and rewound to its start. The command is given a copy of it as
/// its standard input; this one closes when the launcher becomes the
/// command.
#[cfg(target_os = "linux")]
#[allow(
    unsafe_code,
    reason = "decision 0030: an anonymous memory file and its seals are only reachable through \
              system calls; cineris-sandbox is where such calls live"
)]
#[allow(
    clippy::disallowed_types,
    reason = "the launcher is a process of its own, with one thread and no runtime: the file is \
              made before it becomes the command"
)]
fn input_file(input: &[u8]) -> std::io::Result<std::fs::File> {
    use std::io::{Seek, Write};
    use std::os::fd::FromRawFd;

    // SAFETY: the name is a NUL-terminated string that lives for the call,
    // and the flags are the kernel's.
    let fd = unsafe {
        libc::memfd_create(
            c"cineris-input".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` was just made by the call above and nothing else owns
    // it: the file closes it when dropped.
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(input)?;
    let seals = libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL;
    // SAFETY: the call takes the file's descriptor, which the file owns
    // and keeps open for the call, and two integers; it touches no memory
    // of this process.
    if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, seals) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    file.rewind()?;
    Ok(file)
}

#[cfg(not(target_os = "linux"))]
#[allow(
    clippy::disallowed_types,
    reason = "the type of the file the launcher would give the command"
)]
fn input_file(_input: &[u8]) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::other(
        "there is no sandbox launcher on this system",
    ))
}

/// Applies the policy to this process and everything it will start.
#[cfg(all(target_os = "linux", feature = "backends"))]
fn confine(policy: &Policy) -> Result<(), String> {
    crate::confine::apply(policy)
}

#[cfg(not(all(target_os = "linux", feature = "backends")))]
fn confine(_policy: &Policy) -> Result<(), String> {
    Err("this build of Cineris has no sandbox".to_owned())
}

/// Replaces this process with the command: a closed input, the session's
/// temporary directory, and nothing of the launcher's left open.
#[cfg(unix)]
#[allow(
    clippy::disallowed_types,
    reason = "the launcher is a process of its own, with one thread and no runtime, and this is \
              where it becomes the command"
)]
fn become_command(
    program: &OsStr,
    arguments: &[OsString],
    temp: Option<&Path>,
    input: Option<std::fs::File>,
) -> Result<Never, Failure> {
    use std::os::unix::process::CommandExt;

    close_on_exec();
    let mut command = std::process::Command::new(program);
    command.args(arguments).stdin(match input {
        Some(input) => std::process::Stdio::from(input),
        None => std::process::Stdio::null(),
    });
    if let Some(temp) = temp {
        command.env("TMPDIR", temp);
    }
    let error = command.exec();
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => 127,
        _ => 126,
    };
    Err(Failure {
        code,
        message: format!(
            "`{}` could not be started inside the sandbox: {error}",
            Path::new(program).display()
        ),
    })
}

#[cfg(not(unix))]
#[allow(
    clippy::disallowed_types,
    reason = "the type of the file the launcher would give the command"
)]
fn become_command(
    _program: &OsStr,
    _arguments: &[OsString],
    _temp: Option<&Path>,
    _input: Option<std::fs::File>,
) -> Result<Never, Failure> {
    Err(Failure::sandbox(
        "there is no sandbox launcher on this system",
    ))
}

/// Marks every descriptor above the standard three to be closed when the
/// command starts, so that nothing the launcher inherited by mistake (a
/// socket above all) reaches the command.
#[cfg(unix)]
#[allow(
    unsafe_code,
    reason = "decision 0030: marking descriptors is only reachable through a system call; \
              cineris-sandbox is where such calls live"
)]
fn close_on_exec() {
    #[cfg(target_os = "linux")]
    {
        /// `CLOSE_RANGE_CLOEXEC`: mark, do not close.
        const CLOEXEC: libc::c_uint = 1 << 2;
        // SAFETY: the call takes three integers and touches no memory of
        // this process. It marks descriptors; it closes none, so no handle
        // the standard library holds is invalidated.
        let _ = unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, CLOEXEC) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Network, Reach};

    #[test]
    fn a_wrapped_command_keeps_every_argument_after_the_marker() {
        let policy = Policy {
            read: Reach::All,
            write: Vec::new(),
            network: Network::Off,
            temp: None,
            landlock: 6,
        };
        let launcher = Launcher::new("/opt/cineris", ["__sandbox"], policy.clone());
        let wrapped = launcher.wrap(
            OsStr::new("bash"),
            &[OsString::from("-c"), OsString::from("echo -- 'x y'")],
        );
        assert_eq!(wrapped.program, PathBuf::from("/opt/cineris"));
        assert_eq!(
            wrapped.arguments,
            ["__sandbox", "--", "bash", "-c", "echo -- 'x y'"]
                .map(OsString::from)
                .to_vec()
        );
        assert_eq!(Policy::decode(&wrapped.stdin).unwrap(), policy);
    }
}
