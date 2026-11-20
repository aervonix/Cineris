//! Child processes for process tests: started on purpose, seen to start,
//! and seen to die (`docs/testing-and-ci.md`, "No real waiting in tests").
//!
//! A test of a process tree needs a child that stays until it is killed, a
//! grandchild, and a child that ignores a polite request to stop. And it
//! needs to know two things without sleeping: that such a process is up,
//! and that it is gone.
//!
//! # The helper is the test binary itself
//!
//! No extra program is built. A test binary gives itself an entry test:
//!
//! ```ignore
//! #[test]
//! fn child_entry() {
//!     cineris_testkit::children::run_if_child();
//! }
//! ```
//!
//! Run by the test runner, that test finds no child specification among its
//! arguments and passes at once. Run again by a test, with the arguments
//! [`ChildSpec::arguments`] gives, the same test becomes the child and
//! never returns.
//!
//! # A child is watched through a socket
//!
//! Each child connects to the test's [`Roll`] on the loopback interface,
//! says its name and its process id, waits for the roll to answer, and
//! keeps the connection until it dies. So [`Roll::next`] returns when a child is up, and
//! [`Present::gone_within`] returns when the system has closed a dead
//! child's connection. Neither is a guess at how long something takes.

#![allow(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "test support for process tests: it starts processes and blocks on sockets"
)]

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

/// The argument that makes a test binary a child.
const MARK: &str = "cin-child=";

/// The argument that makes a test binary print its other arguments.
const ECHO: &str = "cin-echo";

/// What a line of echoed arguments starts with.
const ECHO_LINE: &str = "cin-echo:";

/// The argument that makes a test binary write to its two output streams.
const STREAMS: &str = "cin-streams";

/// What one child does, and the child it starts, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildSpec {
    name: String,
    address: String,
    stubborn: bool,
    no_stdout: bool,
    say: Option<String>,
    complain: Option<String>,
    exit: Option<i32>,
    child: Option<Box<ChildSpec>>,
}

impl ChildSpec {
    /// A child that reports to `roll` under `name` and then stays until it
    /// is killed. A name is letters, digits, `-` and `_`.
    #[must_use]
    pub fn new(roll: &Roll, name: &str) -> Self {
        assert!(
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
            "a child's name is letters, digits, `-` and `_`: {name:?}"
        );
        Self {
            name: name.to_owned(),
            address: roll.address.clone(),
            stubborn: false,
            no_stdout: false,
            say: None,
            complain: None,
            exit: None,
            child: None,
        }
    }

    /// Makes this child one that the polite interrupt does not stop. It
    /// only applies to a child that another child starts.
    ///
    /// - On Unix it is started with `SIGINT` and `SIGTERM` ignored, which
    ///   a process keeps across `exec`.
    /// - On Windows it is started with no console, so that a console event
    ///   never reaches it.
    #[must_use]
    pub fn stubborn(mut self) -> Self {
        self.stubborn = true;
        self
    }

    /// Makes this child start with its standard output thrown away, so
    /// that of the two output streams it holds only the standard error it
    /// inherits. It only applies to a child that another child starts.
    ///
    /// On Windows a child also inherits every handle of its parent's that
    /// may be inherited, which the parent's own standard output is: there
    /// it may still hold that pipe, unused.
    #[must_use]
    pub fn without_stdout(mut self) -> Self {
        self.no_stdout = true;
        self
    }

    /// Makes this child print `text` and a line break on its standard
    /// output once it has reported. Letters, digits, `-` and `_`.
    #[must_use]
    pub fn saying(mut self, text: &str) -> Self {
        assert_word(text);
        self.say = Some(text.to_owned());
        self
    }

    /// Makes this child print `text` and a line break on its standard
    /// error once it has reported, after what it says on its standard
    /// output. Letters, digits, `-` and `_`.
    #[must_use]
    pub fn complaining(mut self, text: &str) -> Self {
        assert_word(text);
        self.complain = Some(text.to_owned());
        self
    }

    /// Makes this child exit with `code` once it has reported and started
    /// its own child, instead of staying.
    #[must_use]
    pub fn exiting(mut self, code: i32) -> Self {
        self.exit = Some(code);
        self
    }

    /// Makes this child start `child` before anything else.
    #[must_use]
    pub fn with_child(mut self, child: ChildSpec) -> Self {
        self.child = Some(Box::new(child));
        self
    }

    /// The program and the arguments that start this child: the running
    /// test binary, and the arguments that select `entry`, the name of the
    /// test that calls [`run_if_child`].
    ///
    /// None of them needs quoting in any shell beyond the program's path:
    /// the others are letters, digits, `-`, `_` and `=`. The specification
    /// is written in hexadecimal for that reason: a comma or a semicolon
    /// means something to one shell or another.
    #[must_use]
    pub fn arguments(&self, entry: &str) -> (String, Vec<String>) {
        let program = std::env::current_exe()
            .expect("the test binary has a path")
            .to_str()
            .expect("the test binary's path is Unicode")
            .to_owned();
        (
            program,
            vec![
                "--exact".to_owned(),
                entry.to_owned(),
                "--nocapture".to_owned(),
                format!("{MARK}{}", hex(&format!("{entry};{}", self.encode()))),
            ],
        )
    }

    /// A command that starts this child directly, with its standard input
    /// closed and its output thrown away.
    #[must_use]
    pub fn command(&self, entry: &str) -> Command {
        let (program, arguments) = self.arguments(entry);
        let mut command = Command::new(program);
        command
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// `name,address,flags,say,complain[+child]`, where flags are letters
    /// with the exit code last.
    fn encode(&self) -> String {
        let mut flags = String::from("f");
        if self.stubborn {
            flags.push('s');
        }
        if self.no_stdout {
            flags.push('o');
        }
        if let Some(code) = self.exit {
            flags.push_str(&format!("x{code}"));
        }
        let mut text = format!(
            "{},{},{flags},{},{}",
            self.name,
            self.address,
            self.say.as_deref().unwrap_or(""),
            self.complain.as_deref().unwrap_or("")
        );
        if let Some(child) = &self.child {
            text.push('+');
            text.push_str(&child.encode());
        }
        text
    }

    fn decode(text: &str) -> Self {
        let (own, child) = match text.split_once('+') {
            Some((own, child)) => (own, Some(child)),
            None => (text, None),
        };
        let mut fields = own.split(',');
        let mut field = || {
            fields
                .next()
                .expect("a child specification has five fields")
        };
        let name = field().to_owned();
        let address = field().to_owned();
        let flags = field();
        let say = field();
        let complain = field();
        let (letters, exit) = match flags.split_once('x') {
            Some((letters, code)) => (letters, Some(code.parse().expect("an exit code"))),
            None => (flags, None),
        };
        Self {
            name,
            address,
            stubborn: letters.contains('s'),
            no_stdout: letters.contains('o'),
            say: (!say.is_empty()).then(|| say.to_owned()),
            complain: (!complain.is_empty()).then(|| complain.to_owned()),
            exit,
            child: child.map(|child| Box::new(Self::decode(child))),
        }
    }
}

/// The program and the first arguments of a command that prints the
/// arguments after them, exactly as the program received them: the running
/// test binary, the arguments that select `entry`, and `--`.
///
/// A test of quoting puts quoted texts after these in a shell's command
/// line, and reads what arrived with [`echoed`].
#[must_use]
pub fn echo_arguments(entry: &str) -> (String, Vec<String>) {
    let program = std::env::current_exe()
        .expect("the test binary has a path")
        .to_str()
        .expect("the test binary's path is Unicode")
        .to_owned();
    (
        program,
        vec![
            "--exact".to_owned(),
            entry.to_owned(),
            "--nocapture".to_owned(),
            ECHO.to_owned(),
            "--".to_owned(),
        ],
    )
}

/// Which of a command's two output streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// The standard output.
    Stdout,
    /// The standard error.
    Stderr,
}

/// The program and the arguments of a command that writes
/// [`stream_bytes`] of `stderr` bytes to its standard error, then of
/// `stdout` bytes to its standard output, and exits with code 0: the
/// running test binary, and the arguments that select `entry`.
///
/// It writes the standard error first, all of it, so that a reader that
/// waits for the standard output to end before it reads the other stream
/// waits for ever once the standard error is more than a pipe holds.
///
/// The test runner prints a line or two of its own on the standard output
/// before the entry test runs, so what such a command writes there ends
/// with its bytes. Its standard error holds its bytes alone.
#[must_use]
pub fn streams_arguments(entry: &str, stdout: usize, stderr: usize) -> (String, Vec<String>) {
    let program = std::env::current_exe()
        .expect("the test binary has a path")
        .to_str()
        .expect("the test binary's path is Unicode")
        .to_owned();
    (
        program,
        vec![
            "--exact".to_owned(),
            entry.to_owned(),
            "--nocapture".to_owned(),
            STREAMS.to_owned(),
            stdout.to_string(),
            stderr.to_string(),
        ],
    )
}

/// The first `count` bytes a command of [`streams_arguments`] writes to
/// `stream`: numbered lines that name the stream (`out 000001`, or
/// `err 000001`), so that a byte from the other stream, a byte lost, or a
/// cut in the wrong place shows.
#[must_use]
pub fn stream_bytes(stream: Stream, count: usize) -> Vec<u8> {
    let name = match stream {
        Stream::Stdout => "out",
        Stream::Stderr => "err",
    };
    let mut bytes = Vec::with_capacity(count + 16);
    let mut line = 0u64;
    while bytes.len() < count {
        line += 1;
        bytes.extend_from_slice(format!("{name} {line:06}\n").as_bytes());
    }
    bytes.truncate(count);
    bytes
}

/// The arguments each echoing child received, in the order their lines
/// appear in `output`.
#[must_use]
pub fn echoed(output: &str) -> Vec<Vec<String>> {
    output
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix(ECHO_LINE))
        .map(|json| serde_json::from_str(json).expect("an echoed line is a list of strings"))
        .collect()
}

/// Becomes the child that the arguments describe, and never returns. With
/// no child specification among the arguments it returns at once, which is
/// what happens when the test runner runs the entry test.
pub fn run_if_child() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.iter().any(|argument| argument == ECHO) {
        let received: Vec<&String> = arguments
            .iter()
            .skip_while(|argument| *argument != "--")
            .skip(1)
            .collect();
        let line = serde_json::to_string(&received).expect("a list of strings is JSON");
        let mut out = std::io::stdout().lock();
        // On a line of its own, whatever the test harness printed before.
        writeln!(out, "\n{ECHO_LINE}{line}").expect("write to standard output");
        out.flush().expect("write to standard output");
        drop(out);
        #[expect(clippy::exit, reason = "a child process ends when it has answered")]
        std::process::exit(0);
    }
    if let Some(at) = arguments.iter().position(|argument| argument == STREAMS) {
        let count = |at: usize| -> usize {
            arguments
                .get(at)
                .and_then(|count| count.parse().ok())
                .expect("a writer of streams is given two counts")
        };
        let (stdout, stderr) = (count(at + 1), count(at + 2));
        let mut err = std::io::stderr().lock();
        err.write_all(&stream_bytes(Stream::Stderr, stderr))
            .expect("write to standard error");
        err.flush().expect("write to standard error");
        drop(err);
        let mut out = std::io::stdout().lock();
        out.write_all(&stream_bytes(Stream::Stdout, stdout))
            .expect("write to standard output");
        out.flush().expect("write to standard output");
        drop(out);
        #[expect(clippy::exit, reason = "a child process ends when it has written")]
        std::process::exit(0);
    }
    let Some(spec) = arguments
        .iter()
        .find_map(|argument| argument.strip_prefix(MARK).map(unhex))
    else {
        return;
    };
    let (entry, spec) = spec
        .split_once(';')
        .expect("a child specification names its entry test");
    let spec = ChildSpec::decode(spec);

    // Its own child first, so that a child that reports has started it.
    let grandchild = spec.child.as_ref().map(|child| {
        let (program, arguments) = child.arguments(entry);
        let mut command = if child.stubborn && cfg!(unix) {
            // An ignored signal stays ignored across `exec`.
            let mut shell = Command::new("sh");
            shell
                .arg("-c")
                .arg("trap '' INT TERM; exec \"$0\" \"$@\"")
                .arg(program)
                .args(arguments);
            shell
        } else {
            let mut plain = Command::new(program);
            plain.args(arguments);
            plain
        };
        command.stdin(Stdio::null());
        if child.no_stdout {
            command.stdout(Stdio::null());
        }
        #[cfg(windows)]
        if child.stubborn {
            use std::os::windows::process::CommandExt;
            /// `DETACHED_PROCESS`: the new process has no console.
            const DETACHED: u32 = 0x0000_0008;
            command.creation_flags(DETACHED);
            // With no console there is nothing to inherit the standard
            // streams for, except what was redirected: keep those.
        }
        cineris_base::hook::started(command.get_program());
        command.spawn().expect("start the child's own child")
    });

    let mut stream = TcpStream::connect(&spec.address).expect("reach the test's roll");
    writeln!(stream, "{} {}", spec.name, std::process::id()).expect("report to the test");
    stream.flush().expect("report to the test");
    // The roll answers when it has the line. A child that exits at once
    // would otherwise race its own report: Windows may throw away what a
    // process has sent and the other side has not read when the process
    // ends.
    let mut heard = [0u8; 1];
    stream.read_exact(&mut heard).expect("be heard by the test");

    if let Some(text) = &spec.say {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{text}").expect("write to standard output");
        out.flush().expect("write to standard output");
    }
    if let Some(text) = &spec.complain {
        let mut err = std::io::stderr().lock();
        writeln!(err, "{text}").expect("write to standard error");
        err.flush().expect("write to standard error");
    }
    if let Some(code) = spec.exit {
        // The grandchild is left running, on purpose: that is the case of
        // a command that exits and leaves something behind.
        drop(grandchild);
        #[expect(
            clippy::exit,
            reason = "a child process ends with the exit code its test asked for"
        )]
        std::process::exit(code);
    }
    // Stay until killed. The connection stays open until then.
    loop {
        std::thread::park();
    }
}

/// Panics unless `text` is letters, digits, `-` and `_`: what a child's
/// specification can carry, and what any shell reads as one word.
fn assert_word(text: &str) {
    assert!(
        text.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
        "what a child says is letters, digits, `-` and `_`: {text:?}"
    );
}

/// Text as hexadecimal digits: two for each byte.
fn hex(text: &str) -> String {
    text.bytes().map(|byte| format!("{byte:02x}")).collect()
}

/// The text [`hex`] was given.
fn unhex(digits: &str) -> String {
    let bytes: Vec<u8> = (0..digits.len() / 2)
        .map(|at| u8::from_str_radix(&digits[at * 2..at * 2 + 2], 16).expect("two hex digits"))
        .collect();
    String::from_utf8(bytes).expect("a child specification is text")
}

/// Where children report: a listener on the loopback interface, on a port
/// the system picks.
#[derive(Debug)]
pub struct Roll {
    address: String,
    arrivals: Receiver<Present>,
}

/// A child that has reported, and is alive for as long as its connection
/// is open.
#[derive(Debug)]
pub struct Present {
    name: String,
    pid: u32,
    stream: TcpStream,
}

impl Default for Roll {
    fn default() -> Self {
        Self::new()
    }
}

impl Roll {
    /// Starts listening.
    #[must_use]
    pub fn new() -> Self {
        crate::watch::arm();
        let listener = TcpListener::bind("127.0.0.1:0").expect("listen on the loopback interface");
        let address = listener
            .local_addr()
            .expect("the listener has an address")
            .to_string();
        let (sender, arrivals) = mpsc::channel();
        // The thread ends when the test's process does. It reads each
        // child's one line itself, so that a slow child delays nobody.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let sender = sender.clone();
                std::thread::spawn(move || {
                    let mut line = String::new();
                    let mut reader = BufReader::new(&stream);
                    if reader.read_line(&mut line).is_err() {
                        return;
                    }
                    let mut words = line.split_whitespace();
                    let (Some(name), Some(pid)) = (words.next(), words.next()) else {
                        return;
                    };
                    let Ok(pid) = pid.parse() else { return };
                    // For the timeout report: a child of the test's own.
                    cineris_base::hook::spawned(
                        pid,
                        cineris_base::hook::describe_line(
                            &format!("(the roll's child `{name}`)"),
                            &[],
                        ),
                    );
                    // Heard: the child goes on only now.
                    if (&stream).write_all(b"k").is_err() {
                        return;
                    }
                    let _ = sender.send(Present {
                        name: name.to_owned(),
                        pid,
                        stream,
                    });
                });
            }
        });
        Self { address, arrivals }
    }

    /// Waits for the next child to report, in the order they did. Panics
    /// if none does within `limit`, which is a bound on a mistake and not
    /// a delay: a child that reports is returned at once.
    #[must_use]
    pub fn next(&self, limit: Duration) -> Present {
        let _doing = crate::watch::doing(format!(
            "waits for a child to report to the roll at {}, up to {limit:?}",
            self.address
        ));
        self.arrivals
            .recv_timeout(limit)
            .unwrap_or_else(|_| panic!("no child reported within {limit:?}"))
    }

    /// Waits for the children with these names, in whatever order they
    /// report, and returns them in the order asked for.
    #[must_use]
    pub fn all(&self, names: &[&str], limit: Duration) -> Vec<Present> {
        let mut found: Vec<Option<Present>> = names.iter().map(|_| None).collect();
        while found.iter().any(Option::is_none) {
            let present = self.next(limit);
            let place = names
                .iter()
                .position(|name| *name == present.name)
                .unwrap_or_else(|| panic!("an unexpected child reported: {}", present.name));
            found[place] = Some(present);
        }
        found.into_iter().flatten().collect()
    }

    /// Whether a child has reported that [`next`](Self::next) has not
    /// returned yet. For asserting that nothing more was started.
    #[must_use]
    pub fn try_next(&self) -> Option<Present> {
        self.arrivals.try_recv().ok()
    }
}

impl Present {
    /// The name the child reported under.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The child's process id.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether the child is still alive: its connection is open and silent.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.stream
            .set_nonblocking(true)
            .expect("change the socket's mode");
        let mut byte = [0u8; 1];
        let alive = matches!(
            (&self.stream).read(&mut byte),
            Err(error) if error.kind() == ErrorKind::WouldBlock
        );
        self.stream
            .set_nonblocking(false)
            .expect("change the socket's mode");
        alive
    }

    /// Waits until the child is gone, which the system shows by closing
    /// its connection, and returns `true`. Returns `false` if it is still
    /// alive after `limit`, which bounds a failure and is never waited out
    /// by a test that passes.
    #[must_use]
    pub fn gone_within(&self, limit: Duration) -> bool {
        let _doing = crate::watch::doing(format!(
            "waits for the child `{}` (process {}) to be gone, up to {limit:?}",
            self.name, self.pid
        ));
        self.stream
            .set_read_timeout(Some(limit))
            .expect("set the socket's time limit");
        let mut byte = [0u8; 1];
        match (&self.stream).read(&mut byte) {
            // Closed, or reset: a killed process's connection ends either
            // way. A child never writes after its first line, so a byte
            // would be a mistake in the child, and is not "gone".
            Ok(count) => count == 0,
            Err(error) => !matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
        }
    }
}

/// Waits until `condition` holds, asking again and again, and panics if it
/// does not within `limit`. `what` names what is waited for, for the
/// failure message.
///
/// For a condition nothing announces: a file that appears, a process the
/// system has finished with. Where there is something to wait on (a child
/// of a [`Roll`], a [`Gate`](crate::Gate)), wait on that instead.
pub fn until(limit: Duration, what: &str, mut condition: impl FnMut() -> bool) {
    let _doing = crate::watch::doing(format!("waits for {what}, up to {limit:?}"));
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < limit,
            "waited {limit:?} for {what}, and it did not happen"
        );
        std::thread::yield_now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_specification_survives_its_own_encoding() {
        let roll = Roll::new();
        let spec = ChildSpec::new(&roll, "parent")
            .saying("ready")
            .exiting(3)
            .with_child(ChildSpec::new(&roll, "grand-child_1").stubborn());
        assert_eq!(ChildSpec::decode(&spec.encode()), spec);
        let streams = ChildSpec::new(&roll, "both")
            .saying("out")
            .complaining("err")
            .exiting(-2)
            .with_child(ChildSpec::new(&roll, "holder").without_stdout().stubborn());
        assert_eq!(ChildSpec::decode(&streams.encode()), streams);
        let plain = ChildSpec::new(&roll, "one");
        assert_eq!(ChildSpec::decode(&plain.encode()), plain);

        let (_, arguments) = spec.arguments("child_entry");
        assert_eq!(&arguments[..3], ["--exact", "child_entry", "--nocapture"]);
        // Nothing a shell would read as anything but a word.
        assert!(
            arguments[3]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_=".contains(&byte))
        );
        assert_eq!(
            unhex(&hex("child_entry;a,127.0.0.1:9,f,,+b")),
            "child_entry;a,127.0.0.1:9,f,,+b"
        );
    }

    #[test]
    fn the_bytes_of_each_stream_are_numbered_lines_of_their_own() {
        assert_eq!(stream_bytes(Stream::Stdout, 0), b"");
        assert_eq!(
            stream_bytes(Stream::Stdout, 22),
            b"out 000001\nout 000002\n"
        );
        assert_eq!(stream_bytes(Stream::Stderr, 14), b"err 000001\nerr");
        let long = stream_bytes(Stream::Stderr, 100_000);
        assert_eq!(long.len(), 100_000);
        // A cut keeps the beginning.
        assert_eq!(&long[..14], stream_bytes(Stream::Stderr, 14));

        let (_, arguments) = streams_arguments("child_entry", 5, 7);
        assert_eq!(
            arguments,
            ["--exact", "child_entry", "--nocapture", STREAMS, "5", "7"]
        );
    }
}
