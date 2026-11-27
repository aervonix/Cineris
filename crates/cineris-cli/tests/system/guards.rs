//! The guards of the harness's own approvals where the review of step 28
//! found a way past them: a harness that is the first process of its
//! namespace (Linux, finding 7), and a raw client of `serve --socket` run by
//! another harness's command (both systems, finding 3). The other findings
//! of that review are shown in `nested` (the subcommands and the variables)
//! and in `cineris-sandbox`'s own tests (the Windows walks' bounds).

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;

use crate::{Machine, shown};

/// The test this binary is run again by to be a raw client of the socket.
const CLIENT_ENTRY: &str = "guards::sys_socket_client_entry";

/// The argument that makes this binary the client.
const CLIENT: &str = "cin-socket-client";

/// This binary as a raw client of a server's socket: a few lines of its own,
/// naming no Cineris, as any program of the user's could be. After [`CLIENT`]:
/// what it does (`hello`: `initialize`, and the first line back; `trust`:
/// `initialize`, then `session/new` for the workspace, answering the trust
/// question it is asked with trusted, to that request's answer), the place,
/// the workspace, and the file to write each line it reads to (`closed` when
/// the server closes the connection first). Passes at once otherwise.
#[test]
fn sys_socket_client_entry() {
    let arguments: Vec<String> = std::env::args().collect();
    let Some(at) = arguments.iter().position(|argument| argument == CLIENT) else {
        return;
    };
    let [mode, place, workspace, log] = &arguments[at + 1..at + 5] else {
        panic!("{arguments:?}");
    };
    let mut log = std::fs::File::create(log).unwrap();
    let (mut writing, reading) = open(place);
    let initialize = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": cineris_testkit::conformance::initialize()});
    let mut send = |message: serde_json::Value| {
        // A refused connection may be closed as it is written to.
        let _ = writeln!(writing, "{message}").and_then(|()| writing.flush());
    };
    send(initialize);
    if mode == "trust" {
        send(
            serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "session/new",
            "params": {"workspace": workspace}}),
        );
    }
    let mut lines = BufReader::new(reading);
    loop {
        let mut line = String::new();
        if lines.read_line(&mut line).unwrap_or(0) == 0 {
            writeln!(log, "closed").unwrap();
            break;
        }
        log.write_all(line.as_bytes()).unwrap();
        let message: serde_json::Value = serde_json::from_str(&line).unwrap();
        if mode == "hello" {
            break;
        }
        if message["method"] == "trust/request" {
            send(serde_json::json!({"jsonrpc": "2.0", "id": message["id"],
                "result": {"trusted": true}}));
        }
        if message["id"] == 2 && (message.get("result").is_some() || message.get("error").is_some())
        {
            break;
        }
    }
    #[expect(clippy::exit, reason = "a client ends with what it did")]
    std::process::exit(0);
}

/// The client's two halves of a connection to `place`.
#[cfg(unix)]
fn open(place: &str) -> (impl std::io::Write, impl std::io::Read) {
    let stream = std::os::unix::net::UnixStream::connect(place).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .unwrap();
    (stream.try_clone().unwrap(), stream)
}

/// The client's two halves of a connection to the pipe `place`: written
/// and read in turn, on one thread.
#[cfg(windows)]
fn open(place: &str) -> (impl std::io::Write, impl std::io::Read) {
    let pipe = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(place)
        .unwrap();
    (pipe.try_clone().unwrap(), pipe)
}

/// The client run again from this binary, `mode` and its words.
fn client_words(mode: &str, place: &str, workspace: &Path, log: &Path) -> Vec<String> {
    vec![
        "--exact".to_owned(),
        CLIENT_ENTRY.to_owned(),
        "--nocapture".to_owned(),
        CLIENT.to_owned(),
        mode.to_owned(),
        place.to_owned(),
        workspace.display().to_string(),
        log.display().to_string(),
    ]
}

/// A server the person started (`serve --socket`), and where it listens.
struct Server {
    child: std::process::Child,
    place: String,
    #[cfg(unix)]
    _gone: Gone,
}

/// A folder removed at the end.
#[cfg(unix)]
struct Gone(std::path::PathBuf);

#[cfg(unix)]
impl Drop for Gone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Server {
    fn start(machine: &Machine) -> Self {
        #[cfg(unix)]
        let (named, place, gone) = {
            use std::os::unix::fs::PermissionsExt as _;
            // Short, as a socket's path must be.
            let folder =
                std::path::PathBuf::from(format!("/tmp/cin-guards-{}", std::process::id()));
            std::fs::create_dir_all(&folder).unwrap();
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
            let socket = folder.join("rpc.sock").display().to_string();
            (socket.clone(), socket, Gone(folder))
        };
        #[cfg(windows)]
        let (named, place) = {
            let name = format!("cineris-guards-{}", std::process::id());
            let place = format!(r"\\.\pipe\{name}");
            (name, place)
        };
        let mut command = machine.command(&["serve", "--socket", &named]);
        command.stdin(std::process::Stdio::piped());
        let mut child = command.spawn().expect("the server starts");
        let mut said = String::new();
        BufReader::new(child.stderr.take().unwrap())
            .read_line(&mut said)
            .unwrap();
        assert!(said.contains("serving at"), "{said}");
        Self {
            child,
            place,
            #[cfg(unix)]
            _gone: gone,
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A command of another harness of the person's (a headless run at
/// `unrestricted`, where nothing confines its commands) that reaches the
/// person's own server, `serve --socket`, as a raw client naming no Cineris,
/// is refused: before the review of step 28 (finding 3) the socket refused
/// only the server's own commands, and this one opened a session and
/// answered the trust question itself, trusting the project. The control:
/// the same client run by the test, under no harness, is served.
#[test]
fn sys_another_harnesss_command_is_refused_at_the_socket() {
    let machine = Machine::with_config(
        "sys_another_harnesss_command_is_refused_at_the_socket",
        "[permissions]\nmode = \"unrestricted\"\n",
    );
    // Something for the trust question to be asked about.
    machine.project_config(
        "[[permissions.rules]]\neffect = \"exec\"\nmatch = \"make test\"\naction = \"allow\"\n",
    );
    let server = Server::start(&machine);
    let own = std::env::current_exe().unwrap();
    let hello = machine.workspace.join("hello.txt");
    let control = std::process::Command::new(&own)
        .args(client_words(
            "hello",
            &server.place,
            &machine.workspace,
            &hello,
        ))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    let answered = std::fs::read_to_string(&hello).unwrap_or_default();
    assert!(
        answered.contains(r#""id":1"#) && answered.contains(r#""result""#),
        "the control, under no harness, was not served: {answered}\n{}",
        String::from_utf8_lossy(&control.stderr)
    );
    let log = machine.workspace.join("rpc.txt");
    let words = client_words("trust", &server.place, &machine.workspace, &log);
    let (name, text, command) = if cfg!(windows) {
        let words: Vec<String> = words.iter().map(|word| format!("\"{word}\"")).collect();
        (
            "client.cmd",
            format!(
                "@echo off\r\n\"{}\" {} > client.out 2>&1\r\n",
                own.display(),
                words.join(" ")
            ),
            "cmd /c .\\client.cmd",
        )
    } else {
        let words: Vec<String> = words.iter().map(|word| format!("'{word}'")).collect();
        (
            "client.sh",
            format!(
                "'{}' {} > client.out 2>&1\n",
                own.display(),
                words.join(" ")
            ),
            "sh client.sh",
        )
    };
    std::fs::write(machine.workspace.join(name), text).unwrap();
    machine.reply_tool(
        "toolu_1",
        "shell",
        &serde_json::json!({"command": command, "timeout_ms": 60_000}),
    );
    machine.reply_text("Done.");
    let run = machine.run(&["-p", "go", "--output", "json"]);
    let after = machine.run(&["permissions", "list"]);
    drop(server);
    let said = std::fs::read_to_string(&log).unwrap_or_default();
    let first = after.stdout.lines().next().unwrap_or_default().to_owned();
    assert!(
        first.ends_with(": not trusted")
            && !said.contains(r#""result""#)
            && said.contains("closed"),
        "another harness's command was served at the socket: {first}\n--- the client \
         read:\n{said}\n--- it said:\n{}\n{}",
        std::fs::read_to_string(machine.workspace.join("client.out")).unwrap_or_default(),
        shown(&run)
    );
}

/// Whether an unprivileged process may make a user, a process and a mount
/// namespace here, as a container does.
#[cfg(target_os = "linux")]
fn namespaces_allowed() -> bool {
    std::process::Command::new("unshare")
        .args(["-Urpf", "--mount-proc", "true"])
        .output()
        .is_ok_and(|out| out.status.success())
}

/// A harness that is the first process of its namespace of processes (a
/// container's entry point: `docker run image cineris -p …` without
/// `--init`, here `unshare -Urpf --mount-proc`) is seen by the approving
/// subcommands its command runs from a script: before the review of step 28
/// (finding 7) the walk up the line of parents stopped at process 1 without
/// reading it, and `ext add` printed its hash, approved with it and pinned,
/// and `permissions trust` trusted. The script also says what process 1 is,
/// so that the test shows the harness was it. It needs an unprivileged
/// user namespace, which Ubuntu 24.04 (the hosted `ubuntu-latest`) refuses
/// through AppArmor (`kernel.apparmor_restrict_unprivileged_userns`): there
/// it says it was not run, and a runner of Ubuntu 22.04 runs it (`try`'s
/// `os=linux-22.04`); elsewhere on CI it must run. The walk's rule itself is
/// held on every system by `cineris-sandbox`'s
/// `session::tests::a_harness_that_is_process_1_is_seen_and_the_line_ends_at_process_0`.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_harness_that_is_process_1_is_seen() {
    if !namespaces_allowed() {
        let restricted = std::fs::read_to_string(
            "/proc/sys/kernel/apparmor_restrict_unprivileged_userns",
        )
        .is_ok_and(|value| value.trim() == "1");
        assert!(
            restricted || std::env::var_os("CI").is_none(),
            "this runner refuses user namespaces, and AppArmor's restriction is not why"
        );
        eprintln!("not run: this machine refuses unprivileged user namespaces");
        return;
    }
    let machine = Machine::with_config(
        "sys_a_harness_that_is_process_1_is_seen",
        "[permissions]\nmode = \"unrestricted\"\n",
    );
    let folder = machine.home.join("src").join("tracker");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"tracker\"\nversion = \"1\"\ndescription = \"A test \
         extension\"\nmain = \"main.js\"\n",
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), "// nothing\n").unwrap();
    let binary = Path::new(crate::CINERIS).display().to_string();
    let script = format!(
        "tr '\\0' ' ' < /proc/1/cmdline > first.txt\n'{binary}' ext add '{}' > add.txt 2>&1\n\
         '{binary}' permissions trust > trust.txt 2>&1\nANTHROPIC_API_KEY='{}' '{binary}' -p hi \
         > nested.out 2> nested.err\n",
        folder.display(),
        crate::KEY
    );
    std::fs::write(machine.workspace.join("p1.sh"), script).unwrap();
    machine.reply_tool(
        "toolu_1",
        "shell",
        &serde_json::json!({"command": "sh p1.sh", "timeout_ms": 60_000}),
    );
    // The nested run's answer, then the outer run's.
    machine.reply_text("Nested.");
    machine.reply_text("Done.");
    let run = crate::Run::of(
        machine
            .command_as(
                Path::new("unshare"),
                &[
                    "-Urpf",
                    "--mount-proc",
                    crate::CINERIS,
                    "-p",
                    "go",
                    "--output",
                    "json",
                ],
            )
            .output()
            .expect("unshare runs"),
    );
    let read =
        |name: &str| std::fs::read_to_string(machine.workspace.join(name)).unwrap_or_default();
    let first = read("first.txt");
    assert!(
        first.starts_with(crate::CINERIS) && first.contains("-p go"),
        "the harness was not process 1: {first}\n{}",
        shown(&run)
    );
    let (add, trust) = (read("add.txt"), read("trust.txt"));
    let pinned = machine.dirs().state.unwrap().join("pins.toml").exists();
    assert!(
        add.contains("`cineris ext add` was refused")
            && trust.contains("`cineris permissions trust` was refused")
            && !pinned,
        "approved under a harness that is process 1 (pins.toml written: {pinned}):\n--- ext \
         add:\n{add}\n--- trust:\n{trust}\n{}",
        shown(&run)
    );
    // A nested run under it is held to `read-only`, its stores found the
    // session's: the harness, process 1, is found on its line for them too.
    let nested = read("nested.err");
    assert!(
        nested.contains("held to `read-only`") && !nested.contains("was refused"),
        "a nested run under a harness that is process 1:\n{nested}\n{}",
        shown(&run)
    );
}

/// A nested headless run whose line of parents cannot be read (a process of
/// it named in bytes that are not UTF-8, which leaves the line not known,
/// and so under a session's command) with a variable that moves its stores
/// is refused: before the review of step 28 the check found no session's
/// process to compare the stores with, compared only the `CINERIS_`
/// variables, and ran it, capped, reading the stores `XDG_STATE_HOME` named.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_nested_run_whose_line_cannot_be_read_is_refused_with_its_stores_moved() {
    let machine = Machine::with_config(
        "sys_a_nested_run_whose_line_cannot_be_read_is_refused_with_its_stores_moved",
        "[permissions]\nmode = \"unrestricted\"\n",
    );
    let binary = Path::new(crate::CINERIS).display().to_string();
    let moved = machine.workspace.join("moved");
    let script = format!(
        "ln -s /bin/sh \"$(printf 'cin\\377')\"\n\"./$(printf 'cin\\377')\" -c \
         \"XDG_STATE_HOME='{}' ANTHROPIC_API_KEY='{}' '{binary}' -p hi > unread.out 2> \
         unread.err; true\"\n",
        moved.display(),
        crate::KEY
    );
    std::fs::write(machine.workspace.join("unread.sh"), script).unwrap();
    machine.reply_tool(
        "toolu_1",
        "shell",
        &serde_json::json!({"command": "sh unread.sh", "timeout_ms": 60_000}),
    );
    // The outer run's: the nested run is refused before it asks.
    machine.reply_text("Done.");
    let run = machine.run(&["-p", "go", "--output", "json"]);
    let said = std::fs::read_to_string(machine.workspace.join("unread.err")).unwrap_or_default();
    assert!(
        said.contains("was refused") && said.contains("whose process was not found"),
        "{said}\n{}",
        shown(&run)
    );
}

/// A nested headless run's stores are compared with every harness of its
/// line, and not with the nearest alone: a copy of a shell named `cineris`
/// (a harness by its name and its file's), run with a moved `HOME`, between
/// the session and the nested run, does not make the moved `HOME` the
/// session's (the early review of step 28's fixes: the run was compared
/// with the copy, passed, and read the stores the moved `HOME` named).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_shell_named_cineris_on_the_line_does_not_pass_moved_stores() {
    let machine = Machine::with_config(
        "sys_a_shell_named_cineris_on_the_line_does_not_pass_moved_stores",
        "[permissions]\nmode = \"unrestricted\"\n",
    );
    let binary = Path::new(crate::CINERIS).display().to_string();
    let moved = machine.workspace.join("moved");
    let script = format!(
        "mkdir -p fake\ncp /bin/sh fake/cineris\nHOME='{moved}' ./fake/cineris -c \"HOME='{moved}' \
         ANTHROPIC_API_KEY='{}' '{binary}' -p hi > fake.out 2> fake.err; true\"\n",
        crate::KEY,
        moved = moved.display()
    );
    std::fs::write(machine.workspace.join("fake.sh"), script).unwrap();
    machine.reply_tool(
        "toolu_1",
        "shell",
        &serde_json::json!({"command": "sh fake.sh", "timeout_ms": 60_000}),
    );
    // The outer run's: the nested run is refused before it asks.
    machine.reply_text("Done.");
    let run = machine.run(&["-p", "go", "--output", "json"]);
    let said = std::fs::read_to_string(machine.workspace.join("fake.err")).unwrap_or_default();
    assert!(
        said.contains("HOME is set otherwise") && said.contains("was refused"),
        "{said}\n{}",
        shown(&run)
    );
}
