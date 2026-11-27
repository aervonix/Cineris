//! The Linux sandbox, applied for real: commands started through the
//! launcher (this test binary, entered as [`LAUNCH`]) under a policy, and
//! what they can and cannot do (`docs/decisions/0032-what-the-linux-sandbox-needs.md`).
//!
//! Each test makes a workspace, a temporary directory and a directory
//! outside both, in the system's temporary directory, and runs a shell
//! command or this binary as a helper inside the sandbox.

use std::io::Write;
use std::net::{TcpListener, UdpSocket};
use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cineris_sandbox::launch::{FAILED, Launcher, MARK, POLICY_BYTES};
use cineris_sandbox::policy::{Network, Policy, Reach};
use cineris_sandbox::support::{Assessment, detect};
use cineris_testkit::scratch::Scratch;

use crate::{DENY, ENTRY, LAUNCH, NET};

/// The Landlock version this kernel gives the sandbox, or `None` when it
/// would not enforce. On CI it must enforce: a runner without Landlock is a
/// failure here, never a skip, so the tests below cannot be turned off by a
/// change of image.
fn enforcing() -> Option<u8> {
    match detect().assess() {
        Assessment::Enforcing { landlock, .. } => Some(landlock),
        other => {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI must run the sandbox's tests on a kernel where it enforces: {other:?} \
                 ({:?})",
                detect()
            );
            println!("the sandbox does not enforce here ({other:?}): nothing to test");
            None
        }
    }
}

/// A workspace, a temporary directory, and a directory outside both.
struct Place {
    scratch: Scratch,
    /// The command's temporary folder, a write root, when it is not the
    /// scratch's own `tmp`: a short folder of its own ([`Place::short`]).
    short_tmp: Option<ShortFolder>,
}

/// A folder with a short path of its own, for a Unix socket: the kernel
/// holds a socket's path to 108 bytes, and a scratch folder under a long
/// `TMPDIR` (the copy the desk's `rx` builds in) is too long for one. It is
/// under `/tmp`, which every Linux has, named for the test and the process,
/// and removed when dropped.
struct ShortFolder(PathBuf);

impl ShortFolder {
    fn new(test: &str) -> Self {
        let path = PathBuf::from(format!("/tmp/cins-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a short folder under /tmp");
        Self(path)
    }
}

impl Drop for ShortFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Place {
    fn new(test: &str) -> Self {
        let scratch = Scratch::outside("cineris-sandbox-confined", test);
        scratch.file("ws/README", "workspace\n");
        scratch.dir("tmp");
        scratch.file("outside/keep", "keep me\n");
        Self {
            scratch,
            short_tmp: None,
        }
    }

    /// The same, with the command's temporary folder, a write root, a short
    /// folder of its own: a Unix socket bound in it has a path the kernel
    /// takes, whatever `TMPDIR` the test runs under.
    fn short(test: &str) -> Self {
        Self {
            short_tmp: Some(ShortFolder::new(test)),
            ..Self::new(test)
        }
    }

    fn ws(&self) -> PathBuf {
        self.scratch.join("ws")
    }

    fn tmp(&self) -> PathBuf {
        match &self.short_tmp {
            Some(short) => short.0.clone(),
            None => self.scratch.join("tmp"),
        }
    }

    fn outside(&self) -> PathBuf {
        self.scratch.join("outside")
    }

    fn policy(&self, network: Network, landlock: u8) -> Policy {
        Policy {
            read: Reach::All,
            write: vec![resolved(&self.ws()), resolved(&self.tmp())],
            network,
            temp: Some(resolved(&self.tmp())),
            landlock,
        }
    }
}

/// A root resolved to its real path, as the harness resolves the roots
/// when it makes the session's policy (`cineris-core`'s `policy_of`): the
/// launcher then opens them with no link followed. A test that passes a
/// system directory (`/lib`, a symbolic link to `/usr/lib` on a merged-usr
/// system) must resolve it the same way, or the launcher refuses it.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// What a confined command did.
#[derive(Debug)]
struct Ran {
    code: Option<i32>,
    output: String,
}

impl Ran {
    fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// The launcher this test binary is, as the harness would make it. Its
/// role comes after a `--` of the test runner's own, so that the runner
/// does not read the launcher's option (`--policy-bytes`) as one of its.
fn launcher(policy: Policy) -> Launcher {
    Launcher::new(
        std::env::current_exe().expect("the test binary has a path"),
        ["--exact", ENTRY, "--nocapture", "--", LAUNCH],
        policy,
    )
}

/// Runs `program` with `arguments` through the launcher, in `cwd`, with
/// the policy bytes given (normally the launcher's own).
fn run_with(
    launcher: &Launcher,
    stdin: &[u8],
    before: &[&str],
    cwd: &Path,
    program: &str,
    arguments: &[&str],
) -> Ran {
    let arguments: Vec<std::ffi::OsString> = arguments.iter().map(Into::into).collect();
    let wrapped = launcher.wrap(program.as_ref(), &arguments);
    let mut command = match before.split_first() {
        Some((first, rest)) => {
            let mut command = Command::new(first);
            command.args(rest).arg(&wrapped.program);
            command
        }
        None => Command::new(&wrapped.program),
    };
    command
        .args(&wrapped.arguments)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("start the launcher");
    {
        let mut input = child.stdin.take().expect("its input");
        // A launcher that stopped before reading shows as a broken pipe.
        let _ = input.write_all(stdin);
    }
    let output = child.wait_with_output().expect("wait for the launcher");
    Ran {
        code: output.status.code(),
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

fn sh(place: &Place, policy: Policy, script: &str) -> Ran {
    let launcher = launcher(policy);
    let stdin = launcher.policy().encode();
    run_with(
        &launcher,
        &stdin,
        &[],
        &place.ws(),
        "/bin/sh",
        &["-c", script],
    )
}

#[test]
fn sys_the_workspace_and_the_temporary_directory_are_writable_and_nothing_else() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("writes");
    let outside = place.outside();
    // A link in the workspace to a directory outside it: Landlock judges
    // the file written, wherever the name it was reached by is.
    std::os::unix::fs::symlink(&outside, place.ws().join("out-link")).expect("a link");
    let script = format!(
        "echo a > in.txt && mkdir -p sub && mv in.txt sub/ && echo b > \"$TMPDIR/t\" && \
         mv \"$TMPDIR/t\" ./from-tmp && echo quiet > /dev/null && echo err >&2 && \
         echo late > /dev/stderr && printf '%s' \"$TMPDIR\" > tmpdir.txt; \
         echo c > {o}/new; truncate -s 0 {o}/keep; rm -f {o}/keep; mkdir {o}/d; \
         ln -s /etc/passwd {o}/link; mv sub/in.txt {o}/moved; echo through > out-link/through; \
         echo done",
        o = outside.display()
    );
    let ran = sh(&place, place.policy(Network::Off, landlock), &script);
    assert!(ran.output.contains("done"), "{ran:?}");
    // Inside: written, moved, renamed across the two roots.
    assert_eq!(place.scratch.read_text("ws/sub/in.txt"), "a\n");
    assert_eq!(place.scratch.read_text("ws/from-tmp"), "b\n");
    assert_eq!(
        place.scratch.read_text("ws/tmpdir.txt"),
        place.tmp().display().to_string()
    );
    // Outside: nothing made, nothing emptied, nothing removed or moved.
    assert_eq!(place.scratch.listing("outside"), vec!["keep".to_owned()]);
    assert_eq!(place.scratch.read_text("outside/keep"), "keep me\n");
    assert!(ran.output.contains("Permission denied"), "{ran:?}");

    // The model is told which write was refused.
    let line = launcher(place.policy(Network::Off, landlock))
        .explain(Some(1), &ran.output)
        .expect("a refused write is explained");
    assert!(line.contains(&outside.display().to_string()), "{line}");
}

#[test]
fn sys_reading_is_not_restricted_by_default() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("reads");
    let script = format!("cat {}/keep", place.outside().display());
    let ran = sh(&place, place.policy(Network::Off, landlock), &script);
    assert!(ran.ok() && ran.output.contains("keep me"), "{ran:?}");
}

#[test]
fn sys_a_read_policy_keeps_reads_to_its_roots() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("read_roots");
    let mut policy = place.policy(Network::Off, landlock);
    // The system's directories, so that the shell itself can run.
    let mut roots: Vec<PathBuf> = ["/usr", "/bin", "/lib", "/lib64", "/etc", "/proc"]
        .iter()
        .map(|dir| resolved(Path::new(dir)))
        .collect();
    roots.push(resolved(&place.ws()));
    policy.read = Reach::Only(roots);
    let script = format!(
        "cat README; cat {}/keep || echo refused",
        place.outside().display()
    );
    let ran = sh(&place, policy, &script);
    assert!(ran.output.contains("workspace"), "{ran:?}");
    assert!(
        ran.output.contains("refused") && !ran.output.contains("keep me"),
        "{ran:?}"
    );
}

/// Runs this binary as a network helper inside the sandbox.
fn net(place: &Place, network: Network, landlock: u8, what: &[&str]) -> Ran {
    let launcher = launcher(place.policy(network, landlock));
    let stdin = launcher.policy().encode();
    let exe = std::env::current_exe().expect("the test binary has a path");
    let mut arguments = vec!["--exact", ENTRY, "--nocapture", NET];
    arguments.extend_from_slice(what);
    run_with(
        &launcher,
        &stdin,
        &[],
        &place.ws(),
        exe.to_str().expect("the test binary's path is text"),
        &arguments,
    )
}

/// The verdict a network helper printed: `ok`, or the kind of error that
/// refused it. The sandbox refuses with a permission error before anything
/// is sent, which is how its refusal is told from a peer's.
fn verdict(ran: &Ran) -> String {
    ran.output
        .lines()
        .find_map(|line| line.strip_prefix("net: "))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the helper said nothing: {ran:?}"))
}

#[test]
fn sys_with_the_network_off_no_socket_reaches_anything() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::short("network_off");
    let tcp = TcpListener::bind("127.0.0.1:0").expect("listen on loopback");
    let tcp6 = TcpListener::bind("[::1]:0").ok();
    let udp = UdpSocket::bind("127.0.0.1:0").expect("a UDP socket on loopback");
    // In the command's own temporary folder, a write root: the strongest
    // place to refuse a socket from.
    let unix_path = place.tmp().join("listener.sock");
    let _unix = UnixListener::bind(&unix_path).expect("a Unix socket");
    let abstract_name = format!("cineris-sandbox-test-{}", std::process::id());
    let _abstract = UnixListener::bind_addr(
        &std::os::unix::net::SocketAddr::from_abstract_name(abstract_name.as_bytes())
            .expect("an abstract name"),
    )
    .expect("an abstract Unix socket");
    let tcp_at = tcp.local_addr().unwrap().to_string();
    let udp_at = udp.local_addr().unwrap().to_string();
    let unix_at = unix_path.display().to_string();

    let mut cases = vec![
        ("tcp", tcp_at.clone()),
        ("udp", udp_at),
        // TEST-NET-1: refused before anything is sent, so no wait.
        ("tcp", "192.0.2.1:80".to_owned()),
        ("unix", unix_at.clone()),
        ("abstract", abstract_name.clone()),
        ("listen", "127.0.0.1:0".to_owned()),
    ];
    if let Some(tcp6) = &tcp6 {
        cases.push(("tcp", tcp6.local_addr().unwrap().to_string()));
    }
    for (kind, at) in &cases {
        let ran = net(&place, Network::Off, landlock, &[kind, at]);
        assert_eq!(
            verdict(&ran),
            "refused PermissionDenied",
            "{kind} {at}: {ran:?}"
        );
    }
    // A pair of connected Unix sockets reaches only the command's own
    // processes, and netlink only the kernel: both stay.
    for kind in ["pair", "netlink"] {
        let ran = net(&place, Network::Off, landlock, &[kind, "-"]);
        assert_eq!(verdict(&ran), "ok", "{kind}: {ran:?}");
    }

    // Netlink reaches the kernel's interface list, and nothing else: its
    // user protocol, which reaches other processes, is refused.
    let ran = net(&place, Network::Off, landlock, &["netlink-user", "-"]);
    assert_eq!(verdict(&ran), "refused PermissionDenied", "{ran:?}");

    // With the network on, the network is reached, and a Unix socket of
    // anyone else's still is not: the session bus, an agent or the Docker
    // socket would let a command out of the sandbox.
    let ran = net(&place, Network::Full, landlock, &["tcp", &tcp_at]);
    assert_eq!(verdict(&ran), "ok", "tcp with the network on: {ran:?}");
    for (kind, at) in [
        ("unix", unix_at.as_str()),
        ("abstract", abstract_name.as_str()),
    ] {
        let ran = net(&place, Network::Full, landlock, &[kind, at]);
        assert_eq!(
            verdict(&ran),
            "refused PermissionDenied",
            "{kind} {at} with the network on: {ran:?}"
        );
    }
    // Nor netlink's other protocols, nor any family but the internet's.
    let ran = net(&place, Network::Full, landlock, &["netlink-user", "-"]);
    assert_eq!(verdict(&ran), "refused PermissionDenied", "{ran:?}");
    let ran = net(&place, Network::Full, landlock, &["netlink", "-"]);
    assert_eq!(verdict(&ran), "ok", "{ran:?}");
    let ran = net(&place, Network::Full, landlock, &["pair", "-"]);
    assert_eq!(verdict(&ran), "ok", "a pair with the network on: {ran:?}");
}

/// Runs this binary's system-call helper, inside the sandbox or outside it,
/// and returns what it printed after `call: `.
fn call(place: &Place, landlock: u8, name: &str, inside: bool) -> String {
    let exe = std::env::current_exe().expect("the test binary has a path");
    let arguments = ["--exact", ENTRY, "--nocapture", crate::CALL, name];
    let ran = if inside {
        let launcher = launcher(place.policy(Network::Off, landlock));
        let stdin = launcher.policy().encode();
        run_with(
            &launcher,
            &stdin,
            &[],
            &place.ws(),
            exe.to_str().expect("text"),
            &arguments,
        )
    } else {
        let output = Command::new(&exe)
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .expect("run the helper");
        Ran {
            code: output.status.code(),
            output: String::from_utf8_lossy(&output.stdout).into_owned(),
        }
    };
    ran.output
        .lines()
        .find_map(|line| line.strip_prefix("call: "))
        .map(str::to_owned)
        .unwrap_or_else(|| panic!("the helper said nothing: {ran:?}"))
}

#[test]
fn sys_the_calls_that_would_undo_or_get_round_the_sandbox_are_refused() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("calls");
    // Each is refused inside the sandbox. Outside it the same call is not
    // refused that way, so the refusal is the filter's; a host that refuses
    // one by itself (`io_uring` turned off, `perf` for every unprivileged
    // process) makes that control prove nothing, and it is said, not failed.
    let mut controls = 0;
    for (name, inside) in [
        ("io_uring", "EPERM"),
        ("keyctl", "EPERM"),
        ("userfaultfd", "EPERM"),
        ("perf", "EPERM"),
        ("tiocsti", "EPERM"),
        // The C library falls back to `clone`, whose flags the filter reads.
        ("clone3", "ENOSYS"),
    ] {
        assert_eq!(call(&place, landlock, name, true), inside, "{name} inside");
        if call(&place, landlock, name, false) == inside {
            println!("this machine refuses {name} by itself: its control proves nothing");
        } else {
            controls += 1;
        }
    }
    // The calls no host refuses by itself are controls everywhere.
    assert!(controls >= 3, "too few controls held: {controls}");
    // A new user namespace: refused inside; outside, not refused that way
    // (`unshare` answers `EINVAL` there in a process with more than one
    // thread, as the test binary's helper is, and `clone` makes the child).
    for name in ["unshare-user", "clone-user"] {
        assert_eq!(call(&place, landlock, name, true), "EPERM", "{name}");
        let outside = call(&place, landlock, name, false);
        assert!(
            outside != "EPERM" || std::env::var_os("CI").is_none(),
            "CI's runners allow user namespaces, so the refusal is the filter's: {name} {outside}"
        );
    }
    // `bpf` is refused to unprivileged processes by most kernels anyway.
    assert_eq!(call(&place, landlock, "bpf", true), "EPERM");
    // Leaving the command's session is answered as done, and nothing
    // moves: the process stays in the tree's session, which dies with it.
    assert_eq!(call(&place, landlock, "setsid", true), "same");
    assert_eq!(call(&place, landlock, "setsid", false), "new");
    // A process group of its own stays allowed (`timeout` needs one): the
    // session still holds it.
    assert_eq!(call(&place, landlock, "setpgid", true), "new");
}

/// Landlock does not govern a change of a file's owner or extended
/// attributes, and seccomp cannot check their path argument, so the sandbox
/// refuses them outright, inside the workspace too; a change of mode, which
/// builds need, stays allowed (found by the third review of step 26, after
/// measuring what cargo, git, npm, pip and make need). A change of mode or
/// time outside the write roots is a stated limit (decision 0032).
#[test]
fn sys_the_owner_and_extended_attributes_cannot_be_changed() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("metadata");
    // A mode change is allowed, inside the sandbox as out: cargo, git
    // checkouts and installers set modes.
    assert_eq!(call(&place, landlock, "chmod", true), "ok");
    // An owner change (to one's own id, which succeeds for a normal user
    // outside the sandbox) is refused inside it.
    assert_eq!(call(&place, landlock, "chown", true), "EPERM");
    assert_ne!(
        call(&place, landlock, "chown", false),
        "EPERM",
        "a normal user can chown a file to themselves: the refusal is the filter's"
    );
    // An extended-attribute change is refused inside the sandbox. Outside
    // it the call is not refused that way (it succeeds, or the file system
    // does not support the attribute, which is not `EPERM`).
    assert_eq!(call(&place, landlock, "setxattr", true), "EPERM");
    assert_ne!(call(&place, landlock, "setxattr", false), "EPERM");

    // `cp -a`, which preserves modes, times, owner and extended attributes,
    // still copies the content and the mode inside the workspace: it carries
    // the mode and time, and passes over the owner it may not set (a normal
    // user's `cp` does not fail on that). Where a file carries an extended
    // attribute — a POSIX ACL, which a default ACL on the directory gives
    // every file under it on some systems, is one — `cp -a` reports that it
    // could not preserve it and exits non-zero, but the content, mode and
    // time are copied all the same. The reads run under `;`, so they run
    // whatever `cp` returned, and the test asks only that the copy carried
    // the content and the mode (found on `ubuntu-22.04`, whose runner's
    // workspace has a default ACL, by the branch's try run).
    let ran = sh(
        &place,
        place.policy(Network::Off, landlock),
        "mkdir -p a/sub && echo hi > a/sub/f && chmod 0640 a/sub/f && cp -a a b; cat b/sub/f; \
         stat -c %a b/sub/f",
    );
    assert!(
        ran.output.contains("hi") && ran.output.contains("640"),
        "{ran:?}"
    );
}

#[test]
fn sys_a_signal_to_a_process_outside_the_sandbox_is_refused() {
    let Some(landlock) = enforcing() else { return };
    if landlock < 6 {
        println!("Landlock {landlock} does not scope signals: the gap the status names");
        return;
    }
    let place = Place::new("signals");
    let mut outsider = Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("a process outside the sandbox");
    let script = format!(
        "kill -0 {} && echo reached || echo refused; sleep 30 & kill $! && echo own",
        outsider.id()
    );
    let ran = sh(&place, place.policy(Network::Off, landlock), &script);
    let _ = outsider.kill();
    let _ = outsider.wait();
    assert!(
        ran.output.contains("refused") && !ran.output.contains("reached"),
        "{ran:?}"
    );
    // Its own processes it can still signal.
    assert!(ran.output.contains("own"), "{ran:?}");
}

#[test]
fn sys_a_new_user_namespace_is_refused() {
    let Some(landlock) = enforcing() else { return };
    if Command::new("unshare").arg("--version").output().is_err() {
        assert!(std::env::var_os("CI").is_none(), "CI has util-linux");
        return;
    }
    // Outside the sandbox it works, or the refusal inside proves nothing.
    let outside = Command::new("unshare")
        .args(["--user", "true"])
        .status()
        .is_ok_and(|status| status.success());
    if !outside {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI's runners allow user namespaces"
        );
        println!("this machine refuses user namespaces itself: nothing to test");
        return;
    }
    let place = Place::new("user_namespace");
    let ran = sh(
        &place,
        place.policy(Network::Off, landlock),
        "unshare --user true && echo made || echo refused",
    );
    assert!(
        ran.output.contains("refused") && !ran.output.contains("made"),
        "{ran:?}"
    );
}

#[test]
fn sys_a_policy_that_is_not_understood_runs_nothing() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("garbled");
    let launcher = launcher(place.policy(Network::Off, landlock));
    let mut stdin = launcher.policy().encode();
    let at = stdin.len() - 4;
    stdin.splice(at..at, b"share\0/\0".iter().copied());
    let ran = run_with(
        &launcher,
        &stdin,
        &[],
        &place.ws(),
        "/bin/sh",
        &["-c", "echo ran > ran"],
    );
    assert_eq!(ran.code, Some(FAILED), "{ran:?}");
    assert!(
        ran.output.contains(MARK) && ran.output.contains("`share`"),
        "{ran:?}"
    );
    assert!(!place.ws().join("ran").exists());
    let line = launcher.explain(ran.code, &ran.output).expect("explained");
    assert!(line.contains("not run"), "{line}");
}

/// A command started with input after its policy (step 28) reads exactly
/// those bytes, NULs and lines and all, and then the end of its input; it
/// cannot write them (the memory file is sealed), so what a second reader
/// would find is what was given.
#[test]
fn sys_a_confined_command_reads_exactly_its_input_and_cannot_change_it() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("input");
    let launcher = launcher(place.policy(Network::Off, landlock));
    let input = b"first line\nzwei\0drei\n\xc3\xa9 end";
    let stdin = launcher
        .wrap_with_input("/bin/sh".as_ref(), &[], input)
        .expect("an input within the limit")
        .stdin;
    // The write is tried first, at the input's start, where the size does
    // not change: only the seal against writing refuses it there (the
    // early review of step 28: tried at the end, the seal against growing
    // refused it alone).
    let script = "if printf x >&0; then echo wrote; else echo refused; fi; cat > got; \
                  exec 0<&-; echo done";
    let ran = run_with(
        &launcher,
        &stdin,
        &[],
        &place.ws(),
        "/bin/sh",
        &["-c", script],
    );
    assert!(ran.ok() && ran.output.contains("done"), "{ran:?}");
    assert!(ran.output.contains("refused"), "{ran:?}");
    assert_eq!(
        std::fs::read(place.ws().join("got")).expect("what it read"),
        input
    );

    // The most input there may be reaches the command whole: the launcher
    // reads it with its policy.
    let most: Vec<u8> = (0..cineris_sandbox::policy::MAX_INPUT)
        .map(|at| u8::try_from(at % 251).expect("a byte"))
        .collect();
    let stdin = launcher
        .wrap_with_input("/bin/sh".as_ref(), &[], &most)
        .expect("an input at the limit")
        .stdin;
    let ran = run_with(
        &launcher,
        &stdin,
        &[],
        &place.ws(),
        "/bin/sh",
        &["-c", "cat > most"],
    );
    assert!(ran.ok(), "{ran:?}");
    assert!(
        std::fs::read(place.ws().join("most")).expect("what it read") == most,
        "the input at the limit did not arrive whole"
    );
}

/// With no input after its policy, a confined command's input is closed,
/// as it always was: a reader sees its end at once.
#[test]
fn sys_a_confined_command_with_no_input_reads_its_end_at_once() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("no_input");
    // Closed, not the launcher's own input at its end (the early review of
    // step 30's live input): it is the null device.
    let ran = sh(
        &place,
        place.policy(Network::Off, landlock),
        "echo \"input: $(readlink /proc/self/fd/0)\"; cat > got; echo done",
    );
    assert!(ran.ok() && ran.output.contains("done"), "{ran:?}");
    assert!(ran.output.contains("input: /dev/null"), "{ran:?}");
    assert_eq!(place.scratch.read_text("ws/got"), "");
}

/// An input whose length does not match the bytes after the policy runs
/// nothing: the launcher refuses the whole policy before it starts the
/// command.
#[test]
fn sys_an_input_of_the_wrong_length_runs_nothing() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("input_length");
    let launcher = launcher(place.policy(Network::Off, landlock));
    let mut stdin = launcher.policy().encode_with_input(b"abc").expect("small");
    stdin.push(b'd');
    let ran = run_with(
        &launcher,
        &stdin,
        &[],
        &place.ws(),
        "/bin/sh",
        &["-c", "echo ran > ran"],
    );
    assert_eq!(ran.code, Some(FAILED), "{ran:?}");
    assert!(
        ran.output.contains(MARK) && ran.output.contains("`input`"),
        "{ran:?}"
    );
    assert!(!place.ws().join("ran").exists());
}

/// Runs `echo ran > ran` through the launcher under a filter, set up by
/// this binary before it becomes the launcher, that refuses `call`.
fn under_a_filter_that_refuses(place: &Place, landlock: u8, call: &str) -> Ran {
    let launcher = launcher(place.policy(Network::Off, landlock));
    let stdin = launcher.policy().encode();
    let exe = std::env::current_exe().expect("the test binary has a path");
    let exe = exe.to_str().expect("text");
    run_with(
        &launcher,
        &stdin,
        &[exe, "--exact", ENTRY, "--nocapture", DENY, call, "--"],
        &place.ws(),
        "/bin/sh",
        &["-c", "echo ran > ran"],
    )
}

#[test]
fn sys_landlock_that_cannot_be_applied_runs_nothing() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("landlock_refused");
    let ran = under_a_filter_that_refuses(&place, landlock, "landlock_create_ruleset");
    assert_eq!(ran.code, Some(FAILED), "{ran:?}");
    assert!(ran.output.contains("Landlock"), "{ran:?}");
    assert!(!place.ws().join("ran").exists());
}

#[test]
fn sys_a_seccomp_filter_that_does_not_load_runs_nothing() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("seccomp_refused");
    let ran = under_a_filter_that_refuses(&place, landlock, "seccomp");
    assert_eq!(ran.code, Some(FAILED), "{ran:?}");
    assert!(ran.output.contains("seccomp"), "{ran:?}");
    assert!(!place.ws().join("ran").exists());
}

#[test]
fn sys_a_command_that_cannot_be_found_is_said_so() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("not_found");
    let launcher = launcher(place.policy(Network::Off, landlock));
    let stdin = launcher.policy().encode();
    let ran = run_with(&launcher, &stdin, &[], &place.ws(), "/nowhere/sh", &[]);
    assert_eq!(ran.code, Some(127), "{ran:?}");
    assert!(ran.output.contains("could not be started"), "{ran:?}");
}

/// Finding 6 of the third review of step 26: a write root that lies inside
/// another write root (the session's temporary directory inside a
/// `sandbox.write` root) can be replaced with a link by one confined
/// command. The next command's launcher opened the root by its path,
/// followed the link, and granted writes wherever it led. Now the launcher
/// opens each root with no link followed, so a root that became a link is
/// refused and the command does not run.
#[test]
fn sys_a_write_root_swapped_for_a_link_refuses_the_next_command() {
    let Some(landlock) = enforcing() else { return };
    let scratch = Scratch::outside("cineris-sandbox-confined", "swapped_root");
    scratch.file("ws/README", "workspace\n");
    scratch.dir("shared");
    scratch.dir("shared/cineris-s1");
    scratch.file("home/keep", "the user's\n");
    let ws = resolved(&scratch.join("ws"));
    let shared = resolved(&scratch.join("shared"));
    let temp = shared.join("cineris-s1");
    let home = resolved(&scratch.join("home"));
    let policy = Policy {
        read: Reach::All,
        // The temporary directory lies inside a `sandbox.write` root, so a
        // command may rename it: its parent is writable.
        write: vec![ws.clone(), temp.clone(), shared.clone()],
        network: Network::Off,
        temp: Some(temp.clone()),
        landlock,
    };
    // Control: the home is not writable.
    let before = sh_in(
        policy.clone(),
        &ws,
        &format!("echo x > '{}/before'", home.display()),
    );
    assert!(!home.join("before").exists(), "{before:?}");

    // One command swaps its temporary directory for a link to the home.
    let swapped = sh_in(
        policy.clone(),
        &ws,
        &format!(
            "mv \"$TMPDIR\" \"$TMPDIR.old\" && ln -s '{}' \"$TMPDIR\" && echo swapped",
            home.display()
        ),
    );
    assert!(swapped.ok(), "{swapped:?}");

    // The next command's launcher opens the root with no link followed: a
    // root that is a link now is refused, and the command does not run.
    let next = sh_in(
        policy,
        &ws,
        &format!("echo pwned > '{}/after'", home.display()),
    );
    assert_eq!(next.code, Some(FAILED), "{next:?}");
    assert!(next.output.contains(MARK), "{next:?}");
    assert!(
        !home.join("after").exists(),
        "the next command wrote outside every write root: {next:?}"
    );
}

/// Runs `script` with `/bin/sh -c` through the launcher under `policy`, in
/// `cwd`.
fn sh_in(policy: Policy, cwd: &Path, script: &str) -> Ran {
    let launcher = launcher(policy);
    let stdin = launcher.policy().encode();
    run_with(&launcher, &stdin, &[], cwd, "/bin/sh", &["-c", script])
}

/// Starts the launcher with `before` (its options before `--`), then
/// `/bin/sh -c script` in the workspace, writes `stdin` and closes it, and
/// waits: what a launcher given a crafted command line, or a crafted input,
/// does.
fn launched_with(place: &Place, before: &[String], stdin: &[u8], script: &str) -> Ran {
    let exe = std::env::current_exe().expect("the test binary has a path");
    let mut command = Command::new(exe);
    command
        .args(["--exact", ENTRY, "--nocapture", "--", LAUNCH])
        .args(before)
        .args(["--", "/bin/sh", "-c", script])
        .current_dir(place.ws())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("start the launcher");
    {
        let mut input = child.stdin.take().expect("its input");
        // A launcher that stopped before reading shows as a broken pipe.
        let _ = input.write_all(stdin);
    }
    let output = child.wait_with_output().expect("wait for the launcher");
    Ran {
        code: output.status.code(),
        output: format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    }
}

/// A confined command started with a live input (step 30: an MCP server's)
/// reads each line as it is written and answers it before the next is
/// sent, under the policy it was started with; its own arguments that look
/// like the launcher's option are its own.
#[test]
fn sys_a_confined_command_reads_its_live_input_line_by_line() {
    use std::io::BufRead;

    let Some(landlock) = enforcing() else { return };
    let place = Place::new("live");
    let launcher = launcher(place.policy(Network::Off, landlock));
    let outside = place.outside().join("written");
    let script = format!(
        "echo \"args: $0 $1\"; while IFS= read -r line; do echo \"got $line\"; done; \
         if echo x > '{}' 2>/dev/null; then echo wrote; else echo refused; fi",
        outside.display()
    );
    let wrapped = launcher.wrap_live(
        "/bin/sh".as_ref(),
        &["-c", &script, POLICY_BYTES, "1"].map(std::ffi::OsString::from),
    );
    let mut child = Command::new(&wrapped.program)
        .args(&wrapped.arguments)
        .current_dir(place.ws())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start the launcher");
    let mut input = child.stdin.take().expect("its input");
    let mut output = std::io::BufReader::new(child.stdout.take().expect("its output"));
    input
        .write_all(&wrapped.stdin)
        .expect("the policy is written");
    // The test runner's own first lines come before the command's.
    let mut line = String::new();
    while !line.starts_with("args: ") {
        line.clear();
        let read = output.read_line(&mut line).expect("a line");
        assert!(read > 0, "the command said nothing");
    }
    assert_eq!(line, format!("args: {POLICY_BYTES} 1\n"));
    // Each line is answered before the next is written: the input is live.
    for word in ["one", "two", "end"] {
        input
            .write_all(format!("{word}\n").as_bytes())
            .expect("a line is written");
        input.flush().expect("flushed");
        line.clear();
        output.read_line(&mut line).expect("a line");
        assert_eq!(line, format!("got {word}\n"));
    }
    drop(input);
    line.clear();
    output.read_line(&mut line).expect("a line");
    assert_eq!(line, "refused\n", "the policy was not applied");
    let status = child.wait().expect("it ends");
    assert!(status.success(), "{status:?}");
    assert!(!outside.exists());
}

/// A policy length that is missing, not written as a length is, zero, past
/// the most a policy may take, given twice or beside another option, and an
/// input that ends before the length, all run nothing: the launcher writes
/// its line and ends with `FAILED`.
#[test]
fn sys_a_live_policy_length_that_is_wrong_runs_nothing() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("live_length");
    let policy = launcher(place.policy(Network::Off, landlock))
        .policy()
        .encode();
    let length = policy.len().to_string();
    let past = (cineris_sandbox::policy::MAX_BYTES + 1).to_string();
    let cases: Vec<(Vec<&str>, &[u8], &str)> = vec![
        (vec![POLICY_BYTES], &policy, "length"),
        (vec![POLICY_BYTES, "abc"], &policy, "length"),
        (vec![POLICY_BYTES, "0"], &policy, "length"),
        (vec![POLICY_BYTES, "+1"], &policy, "length"),
        (vec![POLICY_BYTES, &past], &policy, "more than"),
        (
            vec![POLICY_BYTES, &length, POLICY_BYTES, &length],
            &policy,
            "twice",
        ),
        (
            vec![POLICY_BYTES, &length, POLICY_BYTES, "1"],
            &policy,
            "twice",
        ),
        (vec!["--policy-bytes=1"], &policy, "does not take"),
        // The input ends before the length.
        (
            vec![POLICY_BYTES, &length],
            &policy[..policy.len() / 2],
            "ended before",
        ),
    ];
    for (before, stdin, naming) in cases {
        let before: Vec<String> = before.iter().map(|one| (*one).to_owned()).collect();
        let ran = launched_with(&place, &before, stdin, "echo ran > ran");
        assert_eq!(ran.code, Some(FAILED), "{before:?}: {ran:?}");
        assert!(
            ran.output.contains(MARK) && ran.output.contains(naming),
            "{before:?}: {ran:?}"
        );
        assert!(!place.ws().join("ran").exists(), "{before:?}");
    }
}

/// A length one short of the policy, or past it, runs nothing: the bytes
/// read are a policy whole or nothing. So does a policy that carries a
/// command's input (an `input` field) in the live mode.
#[test]
fn sys_a_live_length_that_is_not_the_policys_runs_nothing() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("live_mismatch");
    let launcher = launcher(place.policy(Network::Off, landlock));
    let policy = launcher.policy().encode();
    let mut longer = policy.clone();
    longer.extend_from_slice(b"read\0all\0");
    let with_input = launcher
        .policy()
        .encode_with_input(b"abc")
        .expect("a small input");
    // An `input` field of no bytes is an `input` field all the same.
    let with_no_input = launcher
        .policy()
        .encode_with_input(b"")
        .expect("an empty input");
    for (length, stdin) in [
        (policy.len() - 1, &policy),
        (policy.len() + 1, &longer),
        (longer.len(), &longer),
        (with_input.len(), &with_input),
        (with_no_input.len(), &with_no_input),
    ] {
        let ran = launched_with(
            &place,
            &[POLICY_BYTES.to_owned(), length.to_string()],
            stdin,
            "echo ran > ran",
        );
        assert_eq!(ran.code, Some(FAILED), "{length}: {ran:?}");
        assert!(ran.output.contains(MARK), "{length}: {ran:?}");
        assert!(!place.ws().join("ran").exists(), "{length}");
    }
}

/// What follows the policy on a live input is the command's, never a
/// policy: bytes that are a whole policy of their own, one that would let
/// the command write anywhere and reach the network, reach the command as
/// its input, and the command is held to the policy it was started with.
#[test]
fn sys_a_policy_written_after_the_live_policy_is_only_input() {
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("live_after");
    let launcher = launcher(place.policy(Network::Off, landlock));
    let wider = Policy {
        read: Reach::All,
        write: vec![PathBuf::from("/")],
        network: Network::Full,
        temp: None,
        landlock,
    }
    .encode();
    let policy = launcher.wrap_live("/bin/sh".as_ref(), &[]).stdin;
    let mut stdin = policy.clone();
    stdin.extend_from_slice(&wider);
    let outside = place.outside().join("written");
    let ran = launched_with(
        &place,
        &[POLICY_BYTES.to_owned(), policy.len().to_string()],
        &stdin,
        &format!(
            "cat > got; if echo x > '{}' 2>/dev/null; then echo wrote; else echo refused; fi",
            outside.display()
        ),
    );
    assert!(ran.ok() && ran.output.contains("refused"), "{ran:?}");
    assert!(!outside.exists());
    assert_eq!(
        std::fs::read(place.ws().join("got")).expect("what it read"),
        wider
    );
}

/// A descriptor the launcher inherited by mistake never reaches the
/// command, which with a live input may run all session (the early review
/// of step 30's live input): the launcher marks every one above the
/// standard three to close as it becomes the command. Without the launcher
/// the same descriptor reaches the shell, so the test can see one.
#[test]
#[allow(
    unsafe_code,
    reason = "a test of the launcher leaves a descriptor open on purpose, which only `dup2` can"
)]
fn sys_a_descriptor_the_launcher_inherited_never_reaches_the_command() {
    use std::io::BufRead;
    use std::os::fd::AsRawFd;

    /// The descriptor left open, without close-on-exec.
    const STRAY: i32 = 77;
    let Some(landlock) = enforcing() else { return };
    let place = Place::new("stray_descriptor");
    let file = std::fs::File::open(place.ws().join("README")).expect("a file to leave open");
    // SAFETY: `dup2` takes two descriptors and touches no memory; 77 is
    // free in a test process of its own, and stays open until it exits.
    let made = unsafe { libc::dup2(file.as_raw_fd(), STRAY) };
    assert_eq!(made, STRAY, "{}", std::io::Error::last_os_error());
    let check = format!("if [ -e /proc/self/fd/{STRAY} ]; then echo leaked; else echo closed; fi");

    // The control: started directly, the shell has it.
    let direct = Command::new("/bin/sh")
        .args(["-c", &check])
        .output()
        .expect("the shell runs");
    assert_eq!(String::from_utf8_lossy(&direct.stdout), "leaked\n");

    let launcher = launcher(place.policy(Network::Off, landlock));
    let script = format!("{check}; IFS= read -r line; echo \"got $line\"");
    let wrapped = launcher.wrap_live(
        "/bin/sh".as_ref(),
        &["-c", &script].map(std::ffi::OsString::from),
    );
    let mut child = Command::new(&wrapped.program)
        .args(&wrapped.arguments)
        .current_dir(place.ws())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start the launcher");
    let mut input = child.stdin.take().expect("its input");
    input
        .write_all(&wrapped.stdin)
        .expect("the policy is written");
    input.write_all(b"hello\n").expect("a line is written");
    drop(input);
    let output = std::io::BufReader::new(child.stdout.take().expect("its output"));
    let said: Vec<String> = output
        .lines()
        .map_while(Result::ok)
        .filter(|line| line == "closed" || line == "leaked" || line.starts_with("got "))
        .collect();
    assert_eq!(said, ["closed", "got hello"]);
    assert!(child.wait().expect("it ends").success());
}
