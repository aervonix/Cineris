//! A local MCP server started as the harness starts it
//! (`cineris_core::mcp::ServerSpawner`; `docs/steps/30/answered.md`, points
//! 3.1 to 3.3, and the maintainers' changes after the design's review):
//! the program found and refused as an extension's is, the working folder,
//! the explicit environment, the lines both ways, standard error's end,
//! the staged end on the fake timer, and, on Linux, the confinement.
//!
//! The server is this test binary, entered as [`crate::MCP`], acting as its
//! next argument says ([`serve_and_exit`]). Everything it says on standard
//! output begins `cin-mcp: `, after the test runner's own first lines.

use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::task::Poll;
use std::time::Duration;

use cineris_base::{Secret, SecretFilter};
use cineris_core::mcp::{MAX_LINE, ServerSpawner, SpawnerOptions};
use cineris_mcp::{
    EnvEntry, Expose, Launch, LineError, Origin, ServerDefinition, ServerPipes, SpawnError,
    Spawner, ValueSource,
};
use cineris_testkit::{FakeClock, FakeTimer, Scratch};
use serde_json::Value;

use crate::MCP;

/// What the server says each line of its own with.
const SAYS: &str = "cin-mcp: ";

/// A server's program: this binary as [`crate::MCP`] in `mode`, then `args`.
fn this_server(mode: &str, args: &[&str]) -> (String, Vec<String>) {
    let exe = std::env::current_exe().expect("the test binary has a path");
    let mut arguments: Vec<String> = ["--exact", "launcher_entry", "--nocapture", MCP, mode]
        .map(str::to_owned)
        .to_vec();
    arguments.extend(args.iter().map(|arg| (*arg).to_owned()));
    (exe.display().to_string(), arguments)
}

/// A local server's definition.
fn local(program: String, args: Vec<String>, env: Vec<EnvEntry>) -> ServerDefinition {
    ServerDefinition {
        name: "test".to_owned(),
        origin: Origin::User,
        launch: Launch::Local {
            program,
            args,
            cwd: None,
            env,
            network: false,
            write: false,
        },
        description: None,
        enabled: true,
        expose: Expose::Search,
        tools: None,
        deny_tools: Vec::new(),
        start_timeout: Duration::from_secs(10),
        call_timeout: Duration::from_secs(60),
    }
}

/// A place for one test: a workspace, the servers' folders, and a folder
/// outside both.
struct Place {
    scratch: Scratch,
    timer: FakeTimer,
    filter: Arc<RwLock<SecretFilter>>,
}

impl Place {
    fn new(test: &str) -> Self {
        let scratch = Scratch::outside("cineris-core-mcp", test);
        scratch.dir("work");
        scratch.dir("outside");
        let clock = FakeClock::starting_at("2026-11-27T10:00:00Z").unwrap();
        Self {
            scratch,
            timer: FakeTimer::manual(&clock),
            filter: Arc::new(RwLock::new(SecretFilter::new())),
        }
    }

    fn workspace(&self) -> PathBuf {
        std::fs::canonicalize(self.scratch.join("work")).unwrap()
    }

    /// The options of a spawner in this place, trusted as said, with
    /// `inherited` as the harness's environment.
    fn options(&self, trusted: bool, inherited: Vec<(OsString, OsString)>) -> SpawnerOptions {
        SpawnerOptions {
            workspace: self.workspace(),
            trusted,
            folders: self.scratch.join("state").join("mcp"),
            confine: None,
            write_roots: Vec::new(),
            inherited: Some(inherited),
            withheld: vec!["GATEWAY_KEY".to_owned()],
            filter: Arc::clone(&self.filter),
            guard: None,
            timer: Arc::new(self.timer.clone()),
        }
    }

    fn spawner(&self, trusted: bool) -> ServerSpawner {
        ServerSpawner::new(self.options(trusted, base_env()))
    }
}

/// The environment a test gives a spawner as the harness's: the system's
/// own on Windows, which a program needs to start, `PATH`, and what each
/// test adds.
fn base_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| {
            let name = name.to_string_lossy().to_ascii_uppercase();
            [
                "PATH",
                "SYSTEMROOT",
                "SYSTEMDRIVE",
                "WINDIR",
                "USERPROFILE",
                "HOME",
            ]
            .contains(&name.as_str())
        })
        .collect()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
}

/// A server's output, as its pipes give it.
type Output = tokio::sync::mpsc::Receiver<Result<Vec<u8>, LineError>>;

/// Closes a server's input: its client lets go of its end.
fn close_input(pipes: &mut ServerPipes) {
    drop(std::mem::replace(
        &mut pipes.input,
        tokio::sync::mpsc::channel(1).0,
    ));
}

/// The next thing the server says of its own, skipping the test runner's
/// lines; `None` when its output ends first.
async fn said(output: &mut Output) -> Option<String> {
    while let Some(line) = output.recv().await {
        let line = line.expect("a line within the cap");
        let line = String::from_utf8_lossy(&line).into_owned();
        if let Some(rest) = line.strip_prefix(SAYS) {
            return Some(rest.to_owned());
        }
    }
    None
}

async fn send(pipes: &ServerPipes, line: &str) {
    pipes
        .input
        .send(line.as_bytes().to_vec())
        .await
        .expect("the server's input is open");
}

/// What this binary does as an MCP server: `MODE` and its arguments.
///
/// - `echo`: says `ready`, then `echo LINE` for each line of its input, and
///   at its end says `eof` and exits with 0.
/// - `env`: says `env` and its whole environment as one JSON object, then
///   reads its input to its end and exits.
/// - `flood BYTES`: says `ready`, writes a line of that many bytes, then
///   reads its input to its end and exits.
/// - `ignore`: starts a child that runs until it is killed, says `child
///   PID`, `self PID` and `ready`, reads its input to its end, says `eof`,
///   and then runs on, its input closed, until it is killed.
/// - `leave`: starts such a child, says `child PID`, `self PID` and
///   `ready`, and exits, leaving the child running.
/// - `stderr`: writes 20 KiB to standard error, then `token: ` and the
///   variable `TOKEN`, then `the end`; says `ready`, and reads its input to
///   its end.
/// - `where`: says `cwd` and the folder it runs in, and `tmp` and its
///   temporary folder, and exits.
/// - `write PATH`: says `write ok` or `write refused`, by whether it could
///   write `PATH` (`@tmp`: a file in its temporary folder), and exits.
/// - `net ADDRESS`: says `net ok` or `net refused`, by whether it could
///   connect to the TCP `ADDRESS`, and exits.
pub(crate) fn serve_and_exit(arguments: &[OsString]) -> ! {
    let words: Vec<String> = arguments
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let mode = words.first().map_or("", String::as_str);
    let arg = words.get(1).map_or("", String::as_str);
    let say = |text: &str| {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{SAYS}{text}");
        let _ = out.flush();
    };
    let to_end = || {
        let mut input = String::new();
        let _ = std::io::Read::read_to_string(&mut std::io::stdin(), &mut input);
    };
    match mode {
        "echo" => {
            say("ready");
            for line in std::io::stdin().lock().lines() {
                match line {
                    Ok(line) => say(&format!("echo {line}")),
                    Err(_) => break,
                }
            }
            say("eof");
        }
        "env" => {
            let env: serde_json::Map<String, Value> = std::env::vars_os()
                .map(|(name, value)| {
                    (
                        name.to_string_lossy().into_owned(),
                        Value::String(value.to_string_lossy().into_owned()),
                    )
                })
                .collect();
            say(&format!("env {}", Value::Object(env)));
            to_end();
        }
        "flood" => {
            say("ready");
            let bytes: usize = arg.parse().unwrap_or(0);
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(&vec![b'x'; bytes]);
            let _ = out.write_all(b"\n");
            let _ = out.flush();
            drop(out);
            to_end();
        }
        "ignore" | "leave" => {
            let exe = std::env::current_exe().expect("the test binary has a path");
            #[expect(
                clippy::zombie_processes,
                reason = "the child runs until its tree is killed, and the server never waits for it"
            )]
            let child = std::process::Command::new(exe)
                .args([
                    "--exact",
                    "launcher_entry",
                    "--nocapture",
                    crate::HOOK,
                    "hang",
                ])
                .stdin(std::process::Stdio::null())
                .spawn()
                .expect("start the child");
            say(&format!("child {}", child.id()));
            say(&format!("self {}", std::process::id()));
            say("ready");
            if mode == "ignore" {
                to_end();
                say("eof");
                crate::hang();
            }
        }
        "stderr" => {
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(&vec![b'.'; 20 * 1024]);
            let _ = writeln!(
                err,
                "\ntoken: {}",
                std::env::var("TOKEN").unwrap_or_default()
            );
            let _ = writeln!(err, "the end");
            let _ = err.flush();
            drop(err);
            say("ready");
            to_end();
        }
        "where" => {
            let cwd = std::env::current_dir().map(|cwd| cwd.display().to_string());
            say(&format!("cwd {}", cwd.unwrap_or_default()));
            say(&format!("tmp {}", std::env::temp_dir().display()));
        }
        "write" => {
            // `@tmp`: a file in its own temporary folder.
            let path = if arg == "@tmp" {
                std::env::temp_dir().join("written")
            } else {
                PathBuf::from(arg)
            };
            let wrote = std::fs::write(path, "written by a server\n").is_ok();
            say(if wrote { "write ok" } else { "write refused" });
        }
        "net" => {
            let reached = std::net::TcpStream::connect(arg).is_ok();
            say(if reached { "net ok" } else { "net refused" });
        }
        _ => say("no such mode"),
    }
    #[expect(clippy::exit, reason = "a server ends when its test is done with it")]
    std::process::exit(0)
}

/// Lines go both ways, one at a time; a line that holds a line end of its
/// own is never written, as it would be two messages; and closing the
/// input ends a server that ends with it, which is said in words.
#[test]
fn sys_a_server_answers_each_line_and_ends_with_its_input() {
    let place = Place::new("lines");
    let (program, args) = this_server("echo", &[]);
    let mut pipes = place
        .spawner(true)
        .spawn(&local(program, args, Vec::new()))
        .expect("the server starts");
    // What waits for the client of its output is at most 16 MiB of lines
    // at the cap (the full review: 64 such lines were 256 MiB).
    assert!(pipes.output.max_capacity() * MAX_LINE <= cineris_core::mcp::QUEUED_BYTES);
    runtime().block_on(async {
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        for line in ["one", "{\"jsonrpc\":\"2.0\",\"id\":1}"] {
            send(&pipes, line).await;
            assert_eq!(said(&mut pipes.output).await, Some(format!("echo {line}")));
        }
        send(&pipes, "two\nlines").await;
        send(&pipes, "three").await;
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("echo three"));
        close_input(&mut pipes);
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("eof"));
        assert_eq!(pipes.process.exited().await, "exited with code 0");
    });
}

/// A line longer than 4 MiB ends the output with the error that says so.
#[test]
fn sys_a_line_over_the_cap_ends_the_output() {
    let place = Place::new("flood");
    let (program, args) = this_server("flood", &[&(MAX_LINE + 1).to_string()]);
    let mut pipes = place
        .spawner(true)
        .spawn(&local(program, args, Vec::new()))
        .expect("the server starts");
    runtime().block_on(async {
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        let mut last = None;
        while let Some(line) = pipes.output.recv().await {
            last = Some(line);
        }
        assert_eq!(last, Some(Err(LineError::TooLong(MAX_LINE))));
        pipes.process.shutdown(Duration::ZERO).await;
    });
    // At the cap exactly, a line is passed on.
    let (program, args) = this_server("flood", &[&MAX_LINE.to_string()]);
    let mut pipes = place
        .spawner(true)
        .spawn(&local(program, args, Vec::new()))
        .expect("the server starts");
    runtime().block_on(async {
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        let line = pipes.output.recv().await.expect("a line");
        assert_eq!(line.map(|line| line.len()), Ok(MAX_LINE));
        close_input(&mut pipes);
        pipes.process.exited().await;
    });
}

/// The environment is exactly the list: the base, the entries, the
/// server's own temporary folder; never a provider's key or anything else
/// of the harness's. A value read from the environment is given to the
/// secret filter. An entry whose variable is not set, or is a provider's
/// key, refuses the start, naming it.
#[test]
fn sys_a_server_is_given_exactly_its_list() {
    let place = Place::new("environment");
    let mut inherited = base_env();
    for (name, value) in [
        ("LANG", "C.UTF-8"),
        ("TZ", "UTC"),
        ("ANTHROPIC_API_KEY", "sk-ant-never"),
        ("GATEWAY_KEY", "gateway-never"),
        ("UNLISTED", "never"),
        ("CINERIS_CONFIG", "never"),
        ("DOCS_SOURCE", "docs-token-0123456789"),
    ] {
        inherited.push((name.into(), value.into()));
    }
    let spawner = ServerSpawner::new(place.options(true, inherited.clone()));
    let (program, args) = this_server("env", &[]);
    let entries = vec![
        EnvEntry {
            name: "DOCS_TOKEN".to_owned(),
            value: ValueSource::Env("DOCS_SOURCE".to_owned()),
        },
        EnvEntry {
            name: "MODE".to_owned(),
            value: ValueSource::Literal(Secret::new("fast")),
        },
    ];
    let mut pipes = spawner
        .spawn(&local(program.clone(), args.clone(), entries))
        .expect("the server starts");
    let given =
        runtime().block_on(async { said(&mut pipes.output).await.expect("its environment") });
    let given: Value = serde_json::from_str(given.strip_prefix("env ").unwrap()).unwrap();
    let given = given.as_object().unwrap();
    let mut names: Vec<String> = given.keys().map(|name| name.to_ascii_uppercase()).collect();
    names.sort();
    let mut expected: Vec<String> = inherited
        .iter()
        .map(|(name, _)| name.to_string_lossy().to_ascii_uppercase())
        .filter(|name| {
            [
                "PATH",
                "SYSTEMROOT",
                "SYSTEMDRIVE",
                "WINDIR",
                "USERPROFILE",
                "HOME",
                "LANG",
                "TZ",
            ]
            .contains(&name.as_str())
        })
        .collect();
    expected.extend(["DOCS_TOKEN", "MODE"].map(str::to_owned));
    if cfg!(windows) {
        expected.extend(
            [
                "TEMP",
                "TMP",
                "PATHEXT",
                "NODEFAULTCURRENTDIRECTORYINEXEPATH",
                "COMSPEC",
            ]
            .map(str::to_owned),
        );
    } else {
        expected.push("TMPDIR".to_owned());
    }
    expected.sort();
    expected.dedup();
    assert_eq!(names, expected);
    assert_eq!(given["DOCS_TOKEN"], "docs-token-0123456789");
    assert_eq!(given["MODE"], "fast");
    assert!(
        place
            .filter
            .read()
            .unwrap()
            .knows(&Secret::new("docs-token-0123456789")),
        "a value read from the environment is masked from now"
    );
    drop(pipes);

    for (source, naming) in [
        ("NOT_SET_ANYWHERE", "not set"),
        ("ANTHROPIC_API_KEY", "provider"),
        ("GATEWAY_KEY", "provider"),
    ] {
        let refused = spawner
            .spawn(&local(
                program.clone(),
                args.clone(),
                vec![EnvEntry {
                    name: "GIVEN".to_owned(),
                    value: ValueSource::Env(source.to_owned()),
                }],
            ))
            .unwrap_err();
        assert!(
            matches!(&refused, SpawnError::Refused(why) if why.contains(source) && why.contains(naming)),
            "{source}: {refused:?}"
        );
    }
}

/// A program named by a path inside the workspace is never started, nor
/// one by a relative path, nor a server that asks to write the workspace.
#[test]
fn a_program_inside_the_workspace_is_never_started() {
    let place = Place::new("inside");
    // As the workspace is named, not resolved: a program's path is never
    // a verbatim one.
    let planted = place.scratch.join("work").join(if cfg!(windows) {
        "server.exe"
    } else {
        "server"
    });
    std::fs::write(&planted, "#!/bin/sh\necho planted\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&planted, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let spawner = place.spawner(true);
    for program in [planted.display().to_string(), "./server".to_owned()] {
        let refused = spawner
            .spawn(&local(program.clone(), Vec::new(), Vec::new()))
            .unwrap_err();
        assert!(
            matches!(refused, SpawnError::Refused(_)),
            "{program}: {refused:?}"
        );
    }
    // By its bare name, through a `PATH` that leads into the workspace: not
    // found there.
    let mut inherited = base_env();
    inherited.retain(|(name, _)| !name.eq_ignore_ascii_case("PATH"));
    inherited.push(("PATH".into(), place.scratch.join("work").into_os_string()));
    let found = ServerSpawner::new(place.options(true, inherited))
        .spawn(&local("server".to_owned(), Vec::new(), Vec::new()))
        .unwrap_err();
    assert!(matches!(found, SpawnError::NotFound(_)), "{found:?}");
    let (program, args) = this_server("echo", &[]);
    let mut writing = local(program, args, Vec::new());
    if let Launch::Local { write, .. } = &mut writing.launch {
        *write = true;
    }
    let refused = spawner.spawn(&writing).unwrap_err();
    assert!(
        matches!(&refused, SpawnError::Refused(why) if why.contains("own temporary folder")),
        "{refused:?}"
    );
    // A server's own folders are removed when its start is refused.
    let left: Vec<_> = std::fs::read_dir(place.scratch.join("state").join("mcp"))
        .map(|entries| entries.collect())
        .unwrap_or_default();
    assert!(left.is_empty(), "{left:?}");
}

/// A server runs in the workspace only in a trusted project, a `cwd` that
/// resolves inside it counting as the workspace; otherwise in an empty
/// folder of its own. A `cwd` outside the workspace is taken as it is.
#[test]
fn sys_the_workspace_is_a_servers_folder_only_in_a_trusted_project() {
    let place = Place::new("folder");
    let workspace = place.workspace();
    std::fs::create_dir_all(workspace.join("sub")).unwrap();
    let outside = std::fs::canonicalize(place.scratch.join("outside")).unwrap();
    let (program, args) = this_server("where", &[]);
    let run = |trusted: bool, cwd: Option<String>| -> (PathBuf, PathBuf) {
        let mut server = local(program.clone(), args.clone(), Vec::new());
        if let Launch::Local { cwd: asked, .. } = &mut server.launch {
            *asked = cwd;
        }
        let mut pipes = place.spawner(trusted).spawn(&server).expect("it starts");
        runtime().block_on(async {
            let cwd = said(&mut pipes.output).await.expect("its folder");
            let tmp = said(&mut pipes.output).await.expect("its temporary folder");
            (
                PathBuf::from(cwd.strip_prefix("cwd ").unwrap()),
                PathBuf::from(tmp.strip_prefix("tmp ").unwrap()),
            )
        })
    };
    let same = |one: &Path, other: &Path| cineris_base::same_location(one, other);
    for cwd in [
        None,
        Some("sub".to_owned()),
        Some(workspace.display().to_string()),
    ] {
        let (folder, tmp) = run(false, cwd.clone());
        assert!(
            !cineris_base::location_within(&folder, &workspace),
            "{cwd:?}: {}",
            folder.display()
        );
        assert!(folder.ends_with("work"), "{}", folder.display());
        assert!(tmp.ends_with("tmp"), "{}", tmp.display());
        assert!(!same(&folder, &tmp));
    }
    let (folder, _) = run(true, None);
    assert!(same(&folder, &workspace), "{}", folder.display());
    let (folder, _) = run(true, Some("sub".to_owned()));
    assert!(
        same(&folder, &workspace.join("sub")),
        "{}",
        folder.display()
    );
    for trusted in [false, true] {
        let (folder, _) = run(trusted, Some(outside.display().to_string()));
        assert!(same(&folder, &outside), "{}", folder.display());
    }
}

/// A server that does not end when its input closes is given its grace on
/// the fake timer, and then its tree is stopped, the child it started with
/// it; the end of its output then comes.
#[test]
fn sys_a_server_that_ignores_its_closed_input_is_killed_after_its_grace() {
    const GRACE: Duration = Duration::from_secs(2);
    let place = Place::new("grace");
    let (program, args) = this_server("ignore", &[]);
    let mut pipes = place
        .spawner(true)
        .spawn(&local(program, args, Vec::new()))
        .expect("the server starts");
    runtime().block_on(async {
        let child = said(&mut pipes.output).await.expect("its child");
        let child: u32 = child.strip_prefix("child ").unwrap().parse().unwrap();
        said(&mut pipes.output).await.expect("its own id");
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        let mut shutdown = std::pin::pin!(pipes.process.shutdown(GRACE));
        // Polled once: the input is closed and the grace begins.
        std::future::poll_fn(|context| {
            assert!(shutdown.as_mut().poll(context).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("eof"));
        assert_eq!(place.timer.running(), [GRACE]);
        let mut exited = std::pin::pin!(pipes.process.exited());
        std::future::poll_fn(|context| {
            assert!(
                exited.as_mut().poll(context).is_pending(),
                "it runs on within its grace"
            );
            Poll::Ready(())
        })
        .await;
        place.timer.advance(GRACE);
        shutdown.await;
        let how = exited.await;
        assert!(!how.contains("code 0"), "{how}");
        // The output ends: nothing of its tree holds it any more.
        while pipes.output.recv().await.is_some() {}
        #[cfg(target_os = "linux")]
        {
            let stat = std::fs::read_to_string(format!("/proc/{child}/stat"));
            let gone = stat.as_ref().map_or(true, |stat| {
                stat.rsplit(')')
                    .next()
                    .is_some_and(|rest| rest.trim_start().starts_with('Z'))
            });
            assert!(gone, "its child is still running: {stat:?}");
        }
        #[cfg(not(target_os = "linux"))]
        let _ = child;
    });
}

/// The end may be asked for twice, from two tasks at once (the client stops
/// a server that misbehaves, and the lifecycle then shuts the client down):
/// the second waits for the first, which alone gives the grace and stops
/// the tree, and both end; a third, once the server has ended, returns at
/// once.
#[test]
fn sys_a_servers_end_asked_for_twice_at_once_ends_it_once() {
    const GRACE: Duration = Duration::from_secs(2);
    let place = Place::new("twice");
    let (program, args) = this_server("ignore", &[]);
    let mut pipes = place
        .spawner(true)
        .spawn(&local(program, args, Vec::new()))
        .expect("the server starts");
    runtime().block_on(async {
        said(&mut pipes.output).await.expect("its child");
        said(&mut pipes.output).await.expect("its own id");
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        let mut first = std::pin::pin!(pipes.process.shutdown(GRACE));
        let mut second = std::pin::pin!(pipes.process.shutdown(GRACE));
        std::future::poll_fn(|context| {
            assert!(first.as_mut().poll(context).is_pending());
            assert!(second.as_mut().poll(context).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("eof"));
        assert_eq!(place.timer.running(), [GRACE], "one grace, the first's");
        place.timer.advance(GRACE);
        let (mut one, mut two) = (false, false);
        std::future::poll_fn(|context| {
            one = one || first.as_mut().poll(context).is_ready();
            two = two || second.as_mut().poll(context).is_ready();
            if one && two {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
        let how = pipes.process.exited().await;
        assert!(!how.contains("code 0"), "{how}");
        // Once it has ended, at once, with no wait of its own.
        let mut third = std::pin::pin!(pipes.process.shutdown(GRACE));
        std::future::poll_fn(|context| {
            assert!(third.as_mut().poll(context).is_ready(), "it waited");
            Poll::Ready(())
        })
        .await;
    });
}

/// Standard error is drained whatever it holds, its end kept and shown
/// through the session's secret filter, where a value the server was given
/// from the environment is masked.
#[test]
fn sys_a_servers_standard_error_is_kept_to_its_end_and_masked() {
    let place = Place::new("stderr");
    let mut inherited = base_env();
    inherited.push(("TOKEN_SOURCE".into(), "token-value-0123456789".into()));
    let spawner = ServerSpawner::new(place.options(true, inherited));
    let (program, args) = this_server("stderr", &[]);
    let mut pipes = spawner
        .spawn(&local(
            program,
            args,
            vec![EnvEntry {
                name: "TOKEN".to_owned(),
                value: ValueSource::Env("TOKEN_SOURCE".to_owned()),
            }],
        ))
        .expect("the server starts");
    runtime().block_on(async {
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        pipes.process.shutdown(Duration::from_secs(2)).await;
    });
    let tail = pipes.process.stderr_tail();
    assert!(
        tail.len() <= cineris_core::mcp::STDERR_KEPT + 64,
        "{}",
        tail.len()
    );
    assert!(tail.trim_end().ends_with("the end"), "{tail}");
    assert!(!tail.contains("token-value-0123456789"), "{tail}");
    assert!(
        tail.contains(&format!("token: {}", cineris_base::redact::MASK)),
        "{tail}"
    );
}

/// On Linux, where the sandbox enforces, a server is confined at every
/// level: it writes its own temporary folder and nothing else, the
/// workspace included, and reaches the network only with `network`.
#[cfg(all(target_os = "linux", feature = "sandbox"))]
#[test]
fn sys_a_confined_server_writes_only_its_own_folder_and_reaches_the_network_only_when_told() {
    use cineris_sandbox::support::{Assessment, detect};
    let landlock = match detect().assess() {
        Assessment::Enforcing { landlock, .. } => landlock,
        other => {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI must run the sandbox's tests on a kernel where it enforces: {other:?}"
            );
            return;
        }
    };
    let place = Place::new("confined");
    let exe = std::env::current_exe().expect("the test binary has a path");
    let mut options = place.options(true, base_env());
    options.confine = Some(cineris_core::mcp::Confinement {
        program: exe,
        // The launcher's role after a `--` of the test runner's, which would
        // otherwise read the launcher's option as its own.
        prefix: [
            "--exact",
            "launcher_entry",
            "--nocapture",
            "--",
            crate::LAUNCH,
        ]
        .map(OsString::from)
        .to_vec(),
        read: cineris_sandbox::policy::Reach::All,
        landlock,
    });
    let spawner = ServerSpawner::new(options);
    let ask = |mode: &str, arg: &str, network: bool| -> String {
        let (program, args) = this_server(mode, &[arg]);
        let mut server = local(program, args, Vec::new());
        if let Launch::Local {
            network: allowed, ..
        } = &mut server.launch
        {
            *allowed = network;
        }
        let mut pipes = spawner.spawn(&server).expect("it starts");
        runtime().block_on(async { said(&mut pipes.output).await.unwrap_or_default() })
    };
    let written = place.workspace().join("written");
    assert_eq!(
        ask("write", &written.display().to_string(), false),
        "write refused"
    );
    assert!(!written.exists());
    assert_eq!(ask("write", "@tmp", false), "write ok");
    // Not the system's temporary folder outside its own (the full review).
    for outside in [
        std::env::temp_dir().join(format!("cin-mcp-outside-{}", std::process::id())),
        PathBuf::from(format!("/tmp/cin-mcp-outside-{}", std::process::id())),
    ] {
        assert_eq!(
            ask("write", &outside.display().to_string(), false),
            "write refused",
            "{}",
            outside.display()
        );
        assert!(!outside.exists());
    }

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    assert_eq!(ask("net", &address, false), "net refused");
    assert_eq!(ask("net", &address, true), "net ok");
}

/// On Windows a server behind a `.cmd` shim is started through the
/// system's `cmd.exe`, an argument the shim cannot carry refuses the start,
/// naming it, and a program the shim runs by its bare name is never one
/// the workspace holds, even where it runs in the workspace.
#[cfg(windows)]
#[test]
fn sys_a_cmd_shim_is_started_and_never_runs_a_program_from_the_workspace() {
    let place = Place::new("shim");
    let workspace = place.workspace();
    let tools = place.scratch.dir("tools");
    let exe = std::env::current_exe().expect("the test binary has a path");
    let shim = tools.join("echo-mcp.cmd");
    std::fs::write(
        &shim,
        format!(
            "@echo off\r\n\"{}\" --exact launcher_entry --nocapture {MCP} echo %*\r\n",
            exe.display()
        ),
    )
    .unwrap();
    let spawner = place.spawner(true);
    let mut pipes = spawner
        .spawn(&local(shim.display().to_string(), Vec::new(), Vec::new()))
        .expect("the shim starts");
    runtime().block_on(async {
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        send(&pipes, "through cmd").await;
        assert_eq!(
            said(&mut pipes.output).await.as_deref(),
            Some("echo through cmd")
        );
    });
    drop(pipes);
    let refused = spawner
        .spawn(&local(
            shim.display().to_string(),
            vec!["ok".to_owned(), "a&b".to_owned()],
            Vec::new(),
        ))
        .unwrap_err();
    assert!(
        matches!(&refused, SpawnError::Refused(why) if why.contains("argument 2")),
        "{refused:?}"
    );

    // A program planted in the workspace, which `cmd.exe` would find where
    // it runs.
    let marker = place.scratch.join("outside").join("planted-ran");
    std::fs::write(
        workspace.join("cin-planted.cmd"),
        format!("@echo off\r\necho ran > \"{}\"\r\n", marker.display()),
    )
    .unwrap();
    let probe = tools.join("probe-mcp.cmd");
    std::fs::write(
        &probe,
        "@echo off\r\ncall cin-planted\r\necho cin-mcp: done\r\n",
    )
    .unwrap();
    let mut pipes = spawner
        .spawn(&local(probe.display().to_string(), Vec::new(), Vec::new()))
        .expect("the probe starts");
    runtime().block_on(async {
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("done"));
    });
    assert!(!marker.exists(), "a program the workspace holds was run");
}

/// The environment a test's server is given beside the base: a provider's
/// key, a configured provider's and one of the harness's own, each a value
/// no server may see.
fn with_secrets() -> Vec<(OsString, OsString)> {
    let mut inherited = base_env();
    for (name, value) in [
        ("ANTHROPIC_API_KEY", "sk-ant-never-0123456789"),
        ("GATEWAY_KEY", "gateway-never-0123456789"),
        ("CINERIS_SECRET", "cineris-never-0123456789"),
    ] {
        inherited.push((name.into(), value.into()));
    }
    inherited
}

/// A value is read from the environment by a variable's exact name, and
/// the name found is the one judged: a source written as a pattern is
/// refused, never read as a prefix (the full review: `ANTHROPIC_API_KE*`
/// gave a server the key).
#[test]
fn a_source_written_as_a_pattern_is_never_read() {
    let inherited = with_secrets();
    let withheld = ["GATEWAY_KEY".to_owned()];
    for source in [
        "ANTHROPIC_API_KE*",
        "GATEWAY*",
        "CINERIS*",
        "*",
        "ANTHROPIC_API_KE?",
    ] {
        let made = cineris_core::mcp::server_env(
            &[EnvEntry {
                name: "GIVEN".to_owned(),
                value: ValueSource::Env(source.to_owned()),
            }],
            &cineris_core::mcp::EnvParts {
                inherited: &inherited,
                withheld: &withheld,
                avoid: &[],
                temp: Path::new("/state/mcp/1/tmp"),
                cmd: None,
                windows: cfg!(windows),
            },
        );
        assert!(
            matches!(&made, Err(SpawnError::Refused(why)) if why.contains("pattern")),
            "`{source}`: {:?}",
            made.as_ref().map(|made| made.vars.len())
        );
    }
}

/// End to end: a server whose entry takes `ANTHROPIC_API_KE*` is not
/// started, and so never given the key.
#[test]
fn sys_a_server_is_never_given_a_providers_key_through_a_pattern() {
    let place = Place::new("pattern");
    let spawner = ServerSpawner::new(place.options(true, with_secrets()));
    let (program, args) = this_server("env", &[]);
    let mut server = local(
        program,
        args,
        vec![EnvEntry {
            name: "GIVEN".to_owned(),
            value: ValueSource::Env("ANTHROPIC_API_KE*".to_owned()),
        }],
    );
    server.origin = Origin::ProjectConfig;
    let refused = spawner.spawn(&server).map(drop).unwrap_err();
    assert!(matches!(refused, SpawnError::Refused(_)), "{refused:?}");
}

/// A `cwd` that leads into the workspace by another way than its text
/// counts as the workspace: a link on Unix; on Windows another case and a
/// trailing dot, which Windows takes off.
#[test]
fn sys_a_folder_that_leads_into_the_workspace_counts_as_the_workspace() {
    let place = Place::new("folder_link");
    let workspace = place.workspace();
    let (program, args) = this_server("where", &[]);
    #[cfg(unix)]
    let ways = {
        let link = place.scratch.join("outside").join("to-work");
        std::os::unix::fs::symlink(&workspace, &link).unwrap();
        vec![link]
    };
    #[cfg(windows)]
    let ways = {
        let plain = place.scratch.join("work");
        let text = plain.display().to_string();
        vec![
            PathBuf::from(text.to_uppercase()),
            PathBuf::from(format!("{text}.")),
        ]
    };
    for way in ways {
        let mut server = local(program.clone(), args.clone(), Vec::new());
        if let Launch::Local { cwd, .. } = &mut server.launch {
            *cwd = Some(way.display().to_string());
        }
        let mut pipes = place.spawner(false).spawn(&server).expect("it starts");
        let folder =
            runtime().block_on(async { said(&mut pipes.output).await.unwrap_or_default() });
        let folder = PathBuf::from(folder.strip_prefix("cwd ").unwrap());
        assert!(
            !cineris_base::location_within(&folder, &workspace),
            "{}: {}",
            way.display(),
            folder.display()
        );
        assert!(folder.ends_with("work"), "{}", folder.display());
    }
}

/// The servers' own folders are never made inside the workspace or a write
/// root, where a command could put something in a server's way.
#[test]
fn a_servers_folders_inside_the_workspace_are_refused() {
    let place = Place::new("folders_inside");
    let mut options = place.options(true, base_env());
    options.folders = place.workspace().join(".state");
    let (program, args) = this_server("echo", &[]);
    let refused = ServerSpawner::new(options)
        .spawn(&local(program, args, Vec::new()))
        .map(drop)
        .unwrap_err();
    assert!(
        matches!(&refused, SpawnError::Failed(why) if why.contains("inside the workspace")),
        "{refused:?}"
    );
}

/// Whether the process `id` has ended (Linux): it is gone, or a zombie that
/// is no longer run.
#[cfg(target_os = "linux")]
fn ended(id: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{id}/stat")).map_or(true, |stat| {
        stat.rsplit(')')
            .next()
            .is_some_and(|rest| rest.trim_start().starts_with('Z'))
    })
}

/// Waits, for up to half a minute, until the process `id` has ended
/// (Linux): a kill takes milliseconds, so this bounds a failure.
#[cfg(target_os = "linux")]
fn ends_soon(id: u32) -> bool {
    let limit = std::time::Instant::now() + Duration::from_secs(30);
    while !ended(id) {
        if std::time::Instant::now() > limit {
            return false;
        }
        std::thread::yield_now();
    }
    true
}

/// A server whose handle is dropped while it runs, its input closed and
/// ignored, is killed, its tree with it.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_server_dropped_while_it_runs_is_killed() {
    let place = Place::new("dropped");
    let (program, args) = this_server("ignore", &[]);
    let mut pipes = place
        .spawner(true)
        .spawn(&local(program, args, Vec::new()))
        .expect("the server starts");
    let (child, own) = runtime().block_on(async {
        let child = said(&mut pipes.output).await.expect("its child");
        let own = said(&mut pipes.output).await.expect("its own id");
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        (
            child
                .strip_prefix("child ")
                .unwrap()
                .parse::<u32>()
                .unwrap(),
            own.strip_prefix("self ").unwrap().parse::<u32>().unwrap(),
        )
    });
    assert!(!ended(own));
    drop(pipes.process);
    assert!(ends_soon(own), "the server runs on after its handle went");
    assert!(ends_soon(child), "its child runs on after its handle went");
}

/// A server that exits by itself and leaves a child running has its tree
/// ended with it: the child is killed by the time its end is told, and its
/// output then ends; and its own folder is gone by then.
#[test]
fn sys_a_server_that_ends_by_itself_leaves_nothing_running_and_no_folder() {
    let place = Place::new("leave");
    let (program, args) = this_server("leave", &[]);
    let mut pipes = place
        .spawner(true)
        .spawn(&local(program, args, Vec::new()))
        .expect("the server starts");
    runtime().block_on(async {
        let child = said(&mut pipes.output).await.expect("its child");
        let child: u32 = child.strip_prefix("child ").unwrap().parse().unwrap();
        said(&mut pipes.output).await.expect("its own id");
        assert_eq!(said(&mut pipes.output).await.as_deref(), Some("ready"));
        assert_eq!(pipes.process.exited().await, "exited with code 0");
        #[cfg(target_os = "linux")]
        assert!(ended(child), "its child runs on after it ended");
        #[cfg(not(target_os = "linux"))]
        let _ = child;
        while pipes.output.recv().await.is_some() {}
    });
    // Its own folder went before its end was told.
    let folders: Vec<_> = std::fs::read_dir(place.scratch.join("state").join("mcp"))
        .map(|entries| entries.collect())
        .unwrap_or_default();
    assert!(folders.is_empty(), "{folders:?}");
}

/// A `command` or a `cwd` through `/proc` is refused: `/proc/self/cwd` is
/// the harness's folder where it is checked, and the server's where it
/// starts.
#[cfg(target_os = "linux")]
#[test]
fn sys_a_command_or_folder_through_proc_is_refused() {
    let place = Place::new("proc");
    let spawner = place.spawner(true);
    let refused = spawner
        .spawn(&local(
            "/proc/self/cwd/server".to_owned(),
            Vec::new(),
            Vec::new(),
        ))
        .map(drop)
        .unwrap_err();
    assert!(
        matches!(&refused, SpawnError::Refused(why) if why.contains("/proc")),
        "{refused:?}"
    );
    let (program, args) = this_server("where", &[]);
    let mut server = local(program, args, Vec::new());
    if let Launch::Local { cwd, .. } = &mut server.launch {
        *cwd = Some("/proc/self/cwd".to_owned());
    }
    let refused = spawner.spawn(&server).map(drop).unwrap_err();
    assert!(
        matches!(&refused, SpawnError::Refused(why) if why.contains("/proc")),
        "{refused:?}"
    );
    // Nor is a `PATH` entry there given: the server would find a program
    // in its own folder through it, the workspace in a trusted project.
    let mut inherited = base_env();
    inherited.retain(|(name, _)| name != "PATH");
    inherited.push(("PATH".into(), "/proc/self/cwd/bin:/usr/bin:/bin".into()));
    let (program, args) = this_server("env", &[]);
    let mut pipes = ServerSpawner::new(place.options(true, inherited))
        .spawn(&local(program, args, Vec::new()))
        .expect("it starts");
    let given =
        runtime().block_on(async { said(&mut pipes.output).await.expect("its environment") });
    let given: Value = serde_json::from_str(given.strip_prefix("env ").unwrap()).unwrap();
    assert_eq!(given["PATH"], "/usr/bin:/bin");
}
