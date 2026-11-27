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
//!
//! # A command's live input
//!
//! A program that is spoken to while it runs (an MCP server, step 30) needs
//! a standard input that stays open. [`Launcher::wrap_live`] starts the
//! launcher with the policy's exact length in bytes on its command line,
//! before `--` (`--policy-bytes N`); the launcher reads exactly that many
//! bytes from its standard input, unbuffered, so that nothing after them is
//! taken, reads them as a policy, and becomes the command with the same
//! standard input, which then carries what the harness writes after the
//! policy. **Neither what is written to that input nor a crafted command
//! line can change the policy:**
//!
//! - The length is set only by [`Launcher::wrap_live`], from the policy it
//!   encoded itself, and the harness writes the policy before anything
//!   else on the pipe.
//! - The bytes read are exactly a policy or nothing runs: a policy must end
//!   with its `end` field at its last byte ([`crate::policy`], "The wire
//!   form"), so a length one short or one long refuses it, and a policy
//!   with an `input` field is refused in this mode.
//! - The length must be given once, in decimal digits with no sign and no
//!   leading zero, from 1 to [`MAX_BYTES`]; one missing, not written so,
//!   zero, too large, given twice, or beside any other option before `--`
//!   refuses the launch, as does an input that ends before the length.
//! - Only what comes before `--` is the launcher's: the command's own
//!   arguments are never read as options.
//!
//! Every refusal fails closed, with the [`MARK`] line and [`FAILED`].

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

/// The launcher's one option, before `--`: the policy's length in bytes,
/// read from a standard input that then stays the command's live input
/// ([`Launcher::wrap_live`]).
pub const POLICY_BYTES: &str = "--policy-bytes";

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

    /// What to start in place of `program` with `arguments`, whose standard
    /// input stays open for what the harness writes while it runs (an MCP
    /// server's input): [the module](self), "A command's live input". The
    /// policy's length goes on the launcher's command line, before `--`,
    /// and [`Wrapped::stdin`] holds the policy alone, which the caller
    /// writes first and whole, before anything else on the pipe; what it
    /// writes after it is the command's input.
    #[must_use]
    pub fn wrap_live(&self, program: &OsStr, arguments: &[OsString]) -> Wrapped {
        cineris_base::hook::launched(self.program.as_os_str(), program);
        let mut all = self.prefix.clone();
        all.push(OsString::from(POLICY_BYTES));
        all.push(OsString::from(self.encoded.len().to_string()));
        all.push(OsString::from("--"));
        all.push(program.to_owned());
        all.extend(arguments.iter().cloned());
        Wrapped {
            program: self.program.clone(),
            arguments: all,
            stdin: self.encoded.clone(),
        }
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
    let (live, program, rest) = options(arguments).map_err(Failure::sandbox)?;

    let (policy, input) = match live {
        // The command's input is live: exactly the policy's bytes are read,
        // and the rest of the pipe is the command's.
        Some(length) => {
            let bytes = read_exactly(length).map_err(Failure::sandbox)?;
            match Policy::decode_with_input(&bytes).map_err(Failure::sandbox)? {
                (policy, None) => (policy, Given::Live),
                (_, Some(_)) => {
                    return Err(Failure::sandbox(
                        "the policy carries a command's input, and this command's input is live",
                    ));
                }
            }
        }
        None => {
            let mut bytes = Vec::new();
            std::io::stdin()
                .lock()
                .take(u64::try_from(MAX_BYTES + MAX_INPUT).unwrap_or(u64::MAX) + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| {
                    Failure::sandbox(format!("the policy could not be read: {error}"))
                })?;
            let (policy, input) = Policy::decode_with_input(&bytes).map_err(Failure::sandbox)?;
            drop(bytes);
            // Made before the launcher is confined, which may refuse what
            // making it takes.
            let input = match input {
                Some(input) => Given::Sealed(input_file(&input).map_err(|error| {
                    Failure::sandbox(format!("the command's input could not be given: {error}"))
                })?),
                None => Given::Closed,
            };
            (policy, input)
        }
    };

    confine(&policy).map_err(Failure::sandbox)?;
    become_command(&program, &rest, policy.temp.as_deref(), input)
}

/// What the command's standard input is.
#[allow(
    clippy::disallowed_types,
    reason = "the launcher is a process of its own, with one thread and no runtime: the file is \
              made before it becomes the command"
)]
enum Given {
    /// Closed: it sees its end at once.
    Closed,
    /// The sealed memory file of the input that followed the policy.
    Sealed(
        #[cfg_attr(
            not(unix),
            expect(
                dead_code,
                reason = "only Unix has a launcher that becomes the command"
            )
        )]
        std::fs::File,
    ),
    /// The launcher's own, past the policy: what the harness writes while
    /// the command runs.
    Live,
}

/// The launcher's arguments read: the policy's length when the command's
/// input is live, the command, and its arguments. Only what comes before
/// `--` is the launcher's, and the one option it takes is
/// [`POLICY_BYTES`], once, with a length ([`parse_length`]).
///
/// # Errors
///
/// What is wrong, for the launcher's line.
fn options(arguments: Vec<OsString>) -> Result<(Option<usize>, OsString, Vec<OsString>), String> {
    let mut arguments = arguments.into_iter();
    let mut live = None;
    loop {
        let Some(argument) = arguments.next() else {
            return Err("the launcher was not given a command".to_owned());
        };
        if argument == "--" {
            break;
        }
        if argument != POLICY_BYTES {
            return Err(format!(
                "the launcher was given `{}`, which it does not take",
                argument.to_string_lossy()
            ));
        }
        if live.is_some() {
            return Err(format!("`{POLICY_BYTES}` is given twice"));
        }
        live = Some(parse_length(arguments.next().as_deref())?);
    }
    let Some(program) = arguments.next() else {
        return Err("the launcher was not given a command".to_owned());
    };
    Ok((live, program, arguments.collect()))
}

/// The policy's length after [`POLICY_BYTES`]: decimal digits, no sign,
/// no leading zero, from 1 to [`MAX_BYTES`]. Read strictly: it decides
/// where the policy ends and the command's input begins.
fn parse_length(value: Option<&OsStr>) -> Result<usize, String> {
    let Some(value) = value else {
        return Err(format!("`{POLICY_BYTES}` is not given a length"));
    };
    let text = value.to_str().unwrap_or("");
    let shown = value.to_string_lossy();
    let written = !text.is_empty()
        && text.len() <= MAX_BYTES.to_string().len()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && !text.starts_with('0');
    let length = written
        .then(|| text.parse::<usize>().ok())
        .flatten()
        .ok_or_else(|| format!("the policy's length `{shown}` is not written as a length is"))?;
    if length > MAX_BYTES {
        return Err(format!(
            "the policy's length `{shown}` is more than {MAX_BYTES} bytes"
        ));
    }
    Ok(length)
}

/// Exactly `length` bytes of this process's standard input, read with no
/// buffer of the standard library's in between, so that not one byte after
/// them is taken from the command's input.
#[cfg(unix)]
#[allow(
    clippy::disallowed_types,
    reason = "the launcher is a process of its own, with one thread and no runtime"
)]
fn read_exactly(length: usize) -> Result<Vec<u8>, String> {
    use std::os::fd::AsFd;

    // A second descriptor of the same pipe, close-on-exec, read with plain
    // `read` calls each asking for no more than is left of the policy.
    let input = std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map_err(|error| format!("the policy could not be read: {error}"))?;
    let mut input = std::fs::File::from(input);
    let mut bytes = vec![0u8; length];
    input.read_exact(&mut bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            format!("the input ended before the policy's {length} bytes")
        } else {
            format!("the policy could not be read: {error}")
        }
    })?;
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_exactly(_length: usize) -> Result<Vec<u8>, String> {
    Err("there is no sandbox launcher on this system".to_owned())
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

/// Replaces this process with the command: its input as `input` says
/// (closed, the sealed input, or the launcher's own, live), the session's
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
    input: Given,
) -> Result<Never, Failure> {
    use std::os::unix::process::CommandExt;

    close_on_exec();
    let mut command = std::process::Command::new(program);
    command.args(arguments).stdin(match input {
        Given::Sealed(input) => std::process::Stdio::from(input),
        Given::Closed => std::process::Stdio::null(),
        Given::Live => std::process::Stdio::inherit(),
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
fn become_command(
    _program: &OsStr,
    _arguments: &[OsString],
    _temp: Option<&Path>,
    _input: Given,
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

    fn policy() -> Policy {
        Policy {
            read: Reach::All,
            write: Vec::new(),
            network: Network::Off,
            temp: None,
            landlock: 6,
        }
    }

    /// A live wrap gives the policy's exact length before `--`, the policy
    /// alone as the bytes to write first, and every argument of the
    /// command after `--`, one that looks like the launcher's option among
    /// them.
    #[test]
    fn a_live_wrap_gives_the_policys_exact_length_before_the_marker() {
        let launcher = Launcher::new("/opt/cineris", ["__sandbox"], policy());
        let wrapped = launcher.wrap_live(
            OsStr::new("server"),
            &[OsString::from(POLICY_BYTES), OsString::from("1")],
        );
        let length = wrapped.stdin.len().to_string();
        assert_eq!(
            wrapped.arguments,
            [
                "__sandbox",
                POLICY_BYTES,
                &length,
                "--",
                "server",
                POLICY_BYTES,
                "1"
            ]
            .map(OsString::from)
            .to_vec()
        );
        assert_eq!(Policy::decode(&wrapped.stdin).unwrap(), policy());
        // What the launcher reads back of its own arguments.
        let (live, program, rest) = options(wrapped.arguments[1..].to_vec()).unwrap();
        assert_eq!(live, Some(wrapped.stdin.len()));
        assert_eq!(program, "server");
        assert_eq!(rest, [POLICY_BYTES, "1"].map(OsString::from).to_vec());
    }

    /// Every length that is not one written as the harness writes it, and
    /// every other argument before `--`, refuses the launch, naming why.
    #[test]
    fn a_policy_length_not_written_as_the_harness_writes_it_is_refused() {
        let read = |before: &[&str]| {
            let mut arguments: Vec<OsString> = before.iter().map(OsString::from).collect();
            arguments.extend(["--", "/bin/sh"].map(OsString::from));
            options(arguments)
        };
        assert_eq!(read(&[]).unwrap().0, None);
        assert_eq!(read(&[POLICY_BYTES, "300"]).unwrap().0, Some(300));
        let most = MAX_BYTES.to_string();
        assert_eq!(read(&[POLICY_BYTES, &most]).unwrap().0, Some(MAX_BYTES));
        let past = (MAX_BYTES + 1).to_string();
        for (before, naming) in [
            (vec![POLICY_BYTES], "not written as a length"),
            (vec![POLICY_BYTES, "abc"], "not written as a length"),
            (vec![POLICY_BYTES, "+3"], "not written as a length"),
            (vec![POLICY_BYTES, "-3"], "not written as a length"),
            (vec![POLICY_BYTES, "03"], "not written as a length"),
            (vec![POLICY_BYTES, " 3"], "not written as a length"),
            (vec![POLICY_BYTES, "3 "], "not written as a length"),
            (vec![POLICY_BYTES, "0x3"], "not written as a length"),
            (vec![POLICY_BYTES, ""], "not written as a length"),
            (vec![POLICY_BYTES, "0"], "not written as a length"),
            (
                vec![POLICY_BYTES, "99999999999999999999"],
                "not written as a length",
            ),
            (vec![POLICY_BYTES, &past], "more than"),
            (vec![POLICY_BYTES, "3", POLICY_BYTES, "3"], "given twice"),
            (vec![POLICY_BYTES, "3", POLICY_BYTES, "4"], "given twice"),
            (vec!["--policy-bytes=3"], "does not take"),
            (vec!["-x"], "does not take"),
            (vec!["3"], "does not take"),
        ] {
            let error = read(&before).unwrap_err();
            assert!(error.contains(naming), "{before:?}: {error}");
        }
        // A length with no `--` after it, or nothing after `--`.
        let error = options([POLICY_BYTES, "3"].map(OsString::from).to_vec()).unwrap_err();
        assert!(error.contains("not given a command"), "{error}");
        let error = options([POLICY_BYTES, "3", "--"].map(OsString::from).to_vec()).unwrap_err();
        assert!(error.contains("not given a command"), "{error}");
    }
}
