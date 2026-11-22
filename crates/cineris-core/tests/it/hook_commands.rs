//! A hook command's start (`docs/steps/28/answered.md`, "The processes",
//! the hook command's column; point 6), in an open session, through
//! `Core::run_hook_command` until the loop's points call hooks (the step's
//! next part makes these turns).
//!
//! The programs are this test binary, entered as [`crate::HOOK`], which
//! reads the event and reports, fails or runs on as its first argument
//! says; and, for a hook that answers, a script that starts it and then
//! answers (a `.cmd` on Windows, started through `cmd.exe` as `exec`
//! starts one): the test runner writes lines of its own on the binary's
//! standard output, which no answer read strictly could carry. The tests
//! that start one are named `sys_…`, and are few: one session runs every
//! start shape in turn (one that gates, one that replaces the arguments,
//! one that fails, one that runs past its time, one whose turn is
//! interrupted), so that the system tier pays for one session's start. The
//! hooks that start nothing are tested in `hook_wiring`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use cineris_core::ext::{HookOutcome, HookRun};
use cineris_core::{ClientInfo, Outgoing};
use cineris_protocol::event::TurnOutcome;
use cineris_protocol::lease::{Budget, Effect, HardLimits, Lease, Level, NetworkAccess, PathSet};
use cineris_protocol::lease::{Rule, RuleAction, SandboxPolicy};
use cineris_protocol::methods::{
    ApprovalChoice, ApprovalResponse, ClientRole, Op, OpResult, ServerResponse, SessionRef,
};
use serde_json::{Value, json};

use crate::{Front, HOOK, World};

/// This test binary as a hook's program in `mode`, then `args`.
pub(crate) fn this_binary(mode: &str, args: &[&str]) -> Vec<String> {
    let program = std::env::current_exe().expect("the test binary has a path");
    std::iter::once(program.display().to_string())
        .chain(
            ["--exact", "launcher_entry", "--nocapture", HOOK, mode]
                .iter()
                .chain(args)
                .map(|word| (*word).to_owned()),
        )
        .collect()
}

/// The `[hooks.<name>]` entry of a hook on `event` that runs `command`,
/// with `extra` keys.
pub(crate) fn entry(name: &str, event: &str, command: &[String], extra: &str) -> String {
    let words: Vec<String> = command.iter().map(|word| format!("'{word}'")).collect();
    format!(
        "[hooks.{name}]\nevent = \"{event}\"\ncommand = [{}]\n{extra}\n",
        words.join(", ")
    )
}

/// A script in `dir` called `name` that hands its arguments and its input
/// to this binary, which writes them to `report` (when there is one), with
/// nothing of the binary's on the script's standard output; and then
/// answers `answer`. A `.cmd` on Windows, a shell script elsewhere. Its
/// path.
pub(crate) fn script(dir: &Path, name: &str, report: Option<&Path>, answer: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let exe = std::env::current_exe().expect("the test binary has a path");
    #[cfg(windows)]
    {
        let path = dir.join(format!("{name}.cmd"));
        let mut text = String::from("@echo off\r\n");
        match report {
            Some(report) => text.push_str(&format!(
                "\"{}\" --exact launcher_entry --nocapture {HOOK} report \"{}\" %* >nul 2>&1\r\n",
                exe.display(),
                report.display()
            )),
            None => text.push_str("more >nul\r\n"),
        }
        text.push_str(&format!("echo {answer}\r\n"));
        std::fs::write(&path, text).unwrap();
        path
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        let mut text = String::from("#!/bin/sh\n");
        match report {
            Some(report) => text.push_str(&format!(
                "'{}' --exact launcher_entry --nocapture {HOOK} report '{}' \"$@\" >/dev/null \
                 2>&1\n",
                exe.display(),
                report.display()
            )),
            None => text.push_str("cat >/dev/null\n"),
        }
        text.push_str(&format!("printf '%s\\n' '{answer}'\n"));
        std::fs::write(&path, text).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}

/// A hook's program that runs on until it is stopped, and that a test can
/// tell has started: on Linux a script that writes its process's id, waits
/// on a pipe the test opens, and then becomes this binary's `hang`; on
/// other systems this binary's `hang` itself.
pub(crate) struct Lingering {
    /// The entry's `command`.
    pub(crate) command: Vec<String>,
    #[cfg(target_os = "linux")]
    pid: PathBuf,
    #[cfg(target_os = "linux")]
    ready: PathBuf,
}

impl Lingering {
    /// One called `name`, its files in `dir`.
    pub(crate) fn new(dir: &Path, name: &str) -> Self {
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::create_dir_all(dir).unwrap();
            let pid = dir.join(format!("{name}.pid"));
            let ready = dir.join(format!("{name}.ready"));
            let made = std::process::Command::new("mkfifo")
                .arg(&ready)
                .status()
                .expect("mkfifo runs");
            assert!(made.success());
            let path = dir.join(name);
            let exe = std::env::current_exe().expect("the test binary has a path");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\necho $$ > '{}'\ncat '{}' > /dev/null\nexec '{}' --exact \
                     launcher_entry --nocapture {HOOK} hang\n",
                    pid.display(),
                    ready.display(),
                    exe.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self {
                command: vec![path.display().to_string()],
                pid,
                ready,
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (dir, name);
            Self {
                command: this_binary("hang", &[]),
            }
        }
    }

    /// Once its start is known to be made (its time runs on the session's
    /// clock): on Linux, waits until it has written its id, with no wait of
    /// real time (opening the pipe to write blocks until the script opens
    /// it to read), and gives it.
    pub(crate) fn started(&self) -> Option<u32> {
        #[cfg(target_os = "linux")]
        {
            drop(
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(&self.ready)
                    .expect("the pipe opens"),
            );
            let pid = std::fs::read_to_string(&self.pid).unwrap();
            Some(pid.trim().parse().unwrap())
        }
        #[cfg(not(target_os = "linux"))]
        None
    }

    /// Fails the test, with `what`, when the process `started` named is
    /// still there.
    pub(crate) fn assert_gone(pid: Option<u32>, what: &str) {
        if let Some(pid) = pid {
            assert!(
                !Path::new(&format!("/proc/{pid}")).exists(),
                "{what}: its process {pid} is still there"
            );
        }
    }
}

/// Completes when `count` waits of `length` run on the world's timer: for
/// a hook, the loop's wait for it and its run's own time, which starts once
/// its program has.
async fn waits_of(world: &World, length: Duration, count: usize) {
    while world
        .timer
        .running()
        .iter()
        .filter(|wait| **wait == length)
        .count()
        < count
    {
        tokio::task::yield_now().await;
    }
}

/// A supervisor's grant `id` that allows reading everything, at
/// `autonomous`.
fn a_grant_to_read_everything(id: &str) -> Lease {
    let everything = PathSet {
        all: true,
        roots: Vec::new(),
    };
    cineris_policy::standing_lease(
        cineris_protocol::ids::LeaseId::new(id),
        cineris_policy::StandingTerms {
            level: Level::Autonomous,
            rules: vec![Rule {
                effect: Effect::Read,
                pattern: "**".to_owned(),
                action: RuleAction::Allow,
            }],
            sandbox: SandboxPolicy {
                read: everything.clone(),
                write: everything,
                protect: PathSet::default(),
                network: NetworkAccess::Off,
            },
            models: Vec::new(),
            budget: Budget::default(),
            limits: HardLimits::default(),
        },
        cineris_policy::SandboxState::Enforcing,
    )
    .expect("a lease")
}

/// A run of a tool event's hook for `edit`, naming `paths`.
pub(crate) fn for_edit(paths: &[&str]) -> HookRun {
    HookRun {
        input: json!({"event": "tool.before", "tool": "edit", "arguments": {"path": "src/a.rs"}}),
        tool: Some("edit".to_owned()),
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        turn: None,
    }
}

/// Every start shape, as turns of one session, the loop running each hook
/// at its `tool.before` point:
///
/// - a hook that gates is given its placeholders as whole arguments
///   (`{path}` once for each file by place, resolved and plain), the event
///   on its standard input and no provider's key, and its `deny` refuses the
///   call;
/// - one that replaces the arguments, here with a read outside the
///   workspace, which the engine asks about with rules it could keep, has
///   its replacement put to the user offering "allow once" and "deny"
///   alone; it is a person's to answer, so neither a supervisor that had
///   the session open nor one that opens it while the question is open is
///   put it, and neither's allow is taken (the review of step 28's second
///   pull request, M6); answered "allow for the session", it runs once and
///   keeps no rule;
/// - one that exits with an error has failed, which refuses the call;
/// - one that runs past its time on the session's clock is killed, and
///   refuses the call;
/// - a replacement that would write a protected path (`.git/config`) is
///   refused before anyone is asked (`check_replacement`);
/// - one that asks about a call the rules allow has the question put all
///   the same, saying which hook asked;
/// - an extension's hook (a script's, beside the commands) runs a program
///   a rule allows, and is refused one the engine would ask about, naming
///   the rule that would allow it: a hook puts no question, so at
///   `supervised` it runs only what the engine allows outright
///   (`docs/steps/28/answered.md`, point 2);
/// - one whose turn is interrupted while it runs, its own time far off, is
///   stopped, its process with it (M3).
#[test]
fn sys_a_hook_command_gates_replaces_fails_and_runs_past_its_time() {
    let world = World::new(
        "hook_shapes",
        [
            crate::calls(&[(
                "c1",
                "edit",
                json!({"path": "src/a.rs", "edits": [{"old": "fn", "new": "pub fn"}]}),
            )]),
            crate::says("Done."),
            crate::calls(&[("c2", "read", json!({"path": "src/a.rs"}))]),
            crate::says("Done."),
            crate::calls(&[("c3", "list", json!({"path": "."}))]),
            crate::says("Done."),
            crate::calls(&[("c4", "search", json!({"pattern": "main"}))]),
            crate::says("Done."),
            crate::calls(&[("c5", "write", json!({"path": "notes.txt", "content": "x"}))]),
            crate::says("Done."),
            crate::calls(&[("c6", "outline", json!({"path": "src/a.rs"}))]),
            crate::says("Done."),
            crate::calls(&[("c8", "tool_search", json!({"query": "anything"}))]),
            crate::says("Done."),
            crate::calls(&[("c7", "shell", json!({"command": "echo never"}))]),
        ],
    );
    // The stuck hook's time, which nothing reaches.
    const STUCK: Duration = Duration::from_secs(50);
    let tools = world.dir.join("hooks");
    let report = world.dir.join("report.json");
    let gate = script(
        &tools,
        "gate",
        Some(&report),
        r#"{"decision":"deny","reason":"generated"}"#,
    );
    let outside = world.dir.join("outside.txt");
    std::fs::write(&outside, "the replacement's file\n").unwrap();
    let swap = script(
        &tools,
        "swap",
        None,
        &format!(
            r#"{{"arguments":{{"path":{}}}}}"#,
            serde_json::to_string(&outside.display().to_string()).unwrap()
        ),
    );
    let asks = script(
        &tools,
        "asks",
        None,
        r#"{"decision":"ask","reason":"look first"}"#,
    );
    let stuck = Lingering::new(&tools, "stuck");
    let to_config = script(
        &tools,
        "to-config",
        None,
        r#"{"arguments":{"path":".git/config","content":"[core]"}}"#,
    );
    let gate_command: Vec<String> = [gate.display().to_string()]
        .into_iter()
        .chain(["{path}", "{tool}", "{workspace}", "{event}"].map(str::to_owned))
        .collect();
    world.user_config(
        &[
            entry("gate", "tool.before", &gate_command, "tools = [\"edit\"]"),
            entry(
                "swap",
                "tool.before",
                &[swap.display().to_string()],
                "tools = [\"read\"]",
            ),
            entry(
                "broken",
                "tool.before",
                &this_binary("fail", &[]),
                "tools = [\"list\"]",
            ),
            entry(
                "slow",
                "tool.before",
                &this_binary("hang", &[]),
                "tools = [\"search\"]\ntimeout = \"2s\"",
            ),
            entry(
                "to-config",
                "tool.before",
                &[to_config.display().to_string()],
                "tools = [\"write\"]",
            ),
            entry(
                "asks",
                "tool.before",
                &[asks.display().to_string()],
                "tools = [\"outline\"]",
            ),
            entry(
                "stuck",
                "tool.before",
                &stuck.command,
                "tools = [\"shell\"]\ntimeout = \"50s\"",
            ),
            // An extension's hook's programs: allowed by these rules alone.
            "[[permissions.rules]]\neffect = \"exec\"\nmatch = \"probe-echo *\"\naction = \
             \"allow\"\n"
                .to_owned(),
            format!(
                "[[permissions.rules]]\neffect = \"exec\"\nmatch = {}\naction = \"allow\"\n",
                serde_json::to_string(&format!("{} *", std::env::current_exe().unwrap().display()))
                    .unwrap()
            ),
        ]
        .concat(),
    );
    world.put("src/a.rs", "fn main() {}\n");
    // An extension whose hook runs two programs on `PATH`: one the rule
    // above allows, and one the engine would ask about.
    let programs = world.dir.join("programs");
    crate::ext_exec::place_echo(&programs, "probe-echo");
    crate::ext_exec::place_echo(&programs, "probe-asked");
    let words = |given: &str| serde_json::to_string(&crate::ext_exec::echo_args(&[given])).unwrap();
    // And a `tool.after` hook whose program writes the workspace.
    let writes = this_binary("write", &["hooked.txt"]);
    let gatekeeper = crate::ext_exec::extension_running(
        &world,
        "gatekeeper",
        &format!(
            "exec = [\"probe-echo\", \"probe-asked\", {}]\nwrite = [\"**\"]\nhooks = \
             [\"tool.before\", \"tool.after\"]",
            serde_json::to_string(&writes[0]).unwrap()
        ),
        &format!(
            "cineris.on('tool.after', {{ tools: ['outline'] }}, async () => {{ \
             await cineris.exec({}, {}); }});\n\
             cineris.on('tool.before', {{ tools: ['tool_search'] }}, async () => {{\n\
               const allowed = await cineris.exec('probe-echo', {});\n\
               let asked;\n\
               try {{ await cineris.exec('probe-asked', {}); asked = 'ran'; }}\n\
               catch (err) {{ asked = String(err); }}\n\
               const reported = allowed.stdout.includes('{}') ? ' and reported' : '';\n\
               return {{ decision: 'deny', reason: 'allowed ran with ' + allowed.code + reported \
             + '; asked: ' + asked }};\n\
             }});\n",
            serde_json::to_string(&writes[0]).unwrap(),
            serde_json::to_string(&writes[1..]).unwrap(),
            words("allowed"),
            words("asked"),
            crate::ECHO_MARK,
        ),
    );
    let env = crate::ext_exec::path_with(
        &world,
        &[&programs],
        &[("ANTHROPIC_API_KEY", "sk-test-not-for-hooks")],
    );
    let core = world.core_with(|options| {
        options.shell_env = Some(env);
        options.extensions = vec![gatekeeper];
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.answering(crate::Answering::allow_all());

        // One that gates. The edit names one file, given once, by place.
        front.turn("edit it").await;
        let results = front.results();
        let (tool, outcome, text) = &results[0];
        assert_eq!(tool, "edit");
        assert_eq!(*outcome, "denied", "{text}");
        assert!(text.contains("generated"), "{text}");
        assert_eq!(world.read("src/a.rs").as_deref(), Some("fn main() {}\n"));
        let given: Value =
            serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
        let args: Vec<&str> = given["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap())
            .collect();
        assert_eq!(args.len(), 4, "{args:?}");
        assert!(!args[0].starts_with(r"\\?\"), "a plain path: {}", args[0]);
        assert!(
            cineris_base::same_location(
                Path::new(args[0]),
                &world.workspace.join("src").join("a.rs")
            ),
            "{}",
            args[0]
        );
        assert_eq!(args[1], "edit");
        assert!(!args[2].starts_with(r"\\?\"), "{}", args[2]);
        assert!(cineris_base::same_location(
            Path::new(args[2]),
            &world.workspace
        ));
        assert_eq!(args[3], "tool.before");
        let stdin: Value = serde_json::from_str(given["stdin"].as_str().unwrap()).unwrap();
        assert_eq!(stdin["event"], "tool.before");
        assert_eq!(stdin["tool"], "edit");
        assert_eq!(stdin["arguments"]["path"], "src/a.rs");
        assert_eq!(
            given["key"],
            Value::Null,
            "no provider's key reaches a hook"
        );

        // One that replaces the arguments: a person's to answer, offered
        // once or never, and run with the hook's.
        let supervisor = |grant: &str| {
            core.connect_with_grant(
                ClientInfo {
                    role: ClientRole::Supervisor,
                    ..ClientInfo::user()
                },
                Some(a_grant_to_read_everything(grant)),
            )
        };
        let opens = |client: &cineris_core::Client| {
            core.handle(
                client.id,
                Op::SessionOpen(SessionRef {
                    session_id: front.session.clone(),
                }),
            )
        };
        let early = supervisor("l_early");
        let opened = opens(&early).await;
        assert!(opened.is_ok(), "{opened:?}");
        front.prompt("read it").await;
        let (id, asked) = front.approval().await;
        assert!(asked.replaced.is_some(), "{asked:?}");
        assert_eq!(
            asked.choices,
            [ApprovalChoice::AllowOnce, ApprovalChoice::Deny]
        );
        assert!(
            asked.reasons.iter().any(|line| line.contains("swap")),
            "{:?}",
            asked.reasons
        );
        let not_put = |client: &cineris_core::Client, which: &str| {
            let mut put = false;
            while let Some(next) = client.queue.try_next() {
                put |= matches!(next, Outgoing::Request { .. });
            }
            assert!(!put, "a replacement was put to a supervisor that {which}");
            let taken = core.respond(
                client.id,
                &id,
                ServerResponse::Approval(ApprovalResponse {
                    choice: ApprovalChoice::AllowOnce,
                    note: None,
                }),
            );
            assert!(
                taken.is_err(),
                "the allow of a supervisor that {which} was taken"
            );
        };
        not_put(&early, "had the session open");
        // A session has one supervisor at a time: the next opens it while
        // the question is open.
        core.disconnect(early.id).await;
        let late = supervisor("l_late");
        let opened = opens(&late).await;
        assert!(opened.is_ok(), "{opened:?}");
        not_put(&late, "opened it late");
        front.answer(&id, ApprovalChoice::AllowSession).unwrap();
        front.turns(2).await;
        let results = front.results();
        let (tool, outcome, text) = &results[1];
        assert_eq!(tool, "read");
        assert_eq!(*outcome, "success", "{text}");
        assert!(text.contains("the replacement's file"), "{text}");
        let OpResult::PermissionsList(listed) = front
            .op(Op::PermissionsList(SessionRef {
                session_id: front.session.clone(),
            }))
            .await
            .unwrap()
        else {
            panic!("not the result of permissions/list");
        };
        assert!(
            listed.approvals.is_empty(),
            "a replacement's answer kept a rule: {:?}",
            listed.approvals
        );

        // One that fails: the call is refused, the failure named.
        front.turn("list it").await;
        let results = front.results();
        let (_, outcome, text) = &results[2];
        assert_eq!(*outcome, "denied", "{text}");
        assert!(text.contains("the hook went wrong"), "{text}");

        // One that runs past its time: killed when the session's clock
        // reaches it, and the call refused.
        let mut turn = std::pin::pin!(front.turn("search it"));
        let mut its_time = std::pin::pin!(async {
            world.timer.sleeping_for(Duration::from_secs(2)).await;
            world.timer.advance(Duration::from_secs(2));
        });
        let mut advanced = false;
        std::future::poll_fn(|context| {
            if !advanced {
                advanced = its_time.as_mut().poll(context).is_ready();
            }
            turn.as_mut().poll(context).map(|_| ())
        })
        .await;
        let results = front.results();
        let (_, outcome, text) = &results[3];
        assert_eq!(*outcome, "denied", "{text}");
        assert!(text.contains("slow"), "{text}");

        // A replacement that would write `.git/config`: refused, unasked.
        let asked_before = front.approvals().len();
        front.turn("write it").await;
        let results = front.results();
        let (tool, outcome, text) = &results[4];
        assert_eq!(tool, "write");
        assert_eq!(*outcome, "denied", "{text}");
        assert!(text.contains("protected path"), "{text}");
        assert_eq!(front.approvals().len(), asked_before, "nobody was asked");
        assert!(world.read(".git/config").is_none());
        assert!(world.read("notes.txt").is_none());

        // One that asks about a call the rules allow: the question is put
        // all the same, saying which hook asked.
        front.turn("outline it").await;
        let approvals = front.approvals();
        let asked = approvals.last().expect("a question");
        assert_eq!(asked.tool, "outline", "{approvals:?}");
        assert!(
            asked.reasons.iter().any(|line| line.contains("asks")),
            "{:?}",
            asked.reasons
        );
        let results = front.results();
        let (tool, outcome, text) = &results[5];
        assert_eq!(tool, "outline");
        // Allowed, it ran: this session has no index to outline from.
        assert_ne!(*outcome, "denied", "{text}");
        // Its `tool.after` hook's program wrote the workspace, and what it
        // wrote is not in the round's checkpoint, as a hook command's is
        // not: undo does not put it back.
        assert_eq!(
            world.read("hooked.txt").as_deref(),
            Some("written by a hook\n")
        );
        assert!(
            !front
                .log(&world)
                .iter()
                .any(|entry| entry["kind"] == "checkpoint"
                    && entry.to_string().contains("hooked.txt")),
            "a hook's program's writes were recorded in the round's checkpoint"
        );

        // An extension's hook runs the program a rule allows, and is
        // refused the one the engine would ask about, with the rule that
        // would allow it: nobody is asked.
        let asked_before = front.approvals().len();
        front.turn("search the tools").await;
        let results = front.results();
        let (tool, outcome, text) = &results[6];
        assert_eq!(tool, "tool_search");
        assert_eq!(*outcome, "denied", "{text}");
        assert!(text.contains("`gatekeeper/hook.tool.before`"), "{text}");
        assert!(text.contains("allowed ran with 7 and reported"), "{text}");
        assert!(
            text.contains("asked: Error: a hook may not put a question")
                && text.contains("effect = \"exec\"")
                && text.contains("probe-asked"),
            "{text}"
        );
        assert_eq!(front.approvals().len(), asked_before, "nobody was asked");

        // One whose turn is interrupted while it runs: stopped, though its
        // time is far off.
        let started = std::cell::Cell::new(None);
        let mut turn = std::pin::pin!(front.turn("run it"));
        let mut interrupt = std::pin::pin!(async {
            // The loop's wait for it, and its run's own time.
            waits_of(&world, STUCK, 2).await;
            started.set(stuck.started());
            front.interrupt(false).await;
        });
        let mut interrupted = false;
        let ended = std::future::poll_fn(|context| {
            if !interrupted {
                interrupted = interrupt.as_mut().poll(context).is_ready();
            }
            turn.as_mut().poll(context)
        })
        .await;
        assert_eq!(ended.outcome, TurnOutcome::Interrupted, "{ended:?}");

        // The same file named three ways, and on Windows in another case,
        // is one file to `{path}`, given once, by place.
        let mut named = vec!["src/a.rs", "./src/a.rs", "src/../src/a.rs"];
        if cfg!(windows) {
            named.push("SRC/A.RS");
        }
        let ran = core
            .run_hook_command(
                &front.session,
                "gate",
                HookRun {
                    input: json!({"event": "tool.before"}),
                    tool: Some("edit".to_owned()),
                    paths: named.iter().map(|path| (*path).to_owned()).collect(),
                    turn: None,
                },
            )
            .await
            .unwrap();
        assert!(matches!(ran, HookOutcome::Answered(_)), "{ran:?}");
        let given: Value =
            serde_json::from_str(&std::fs::read_to_string(&report).unwrap()).unwrap();
        assert_eq!(given["args"].as_array().unwrap().len(), 4, "{given}");
        front.close().await;
        Lingering::assert_gone(started.get(), "the interrupted hook ran on");
    });
}

#[cfg(target_os = "linux")]
mod confined {
    //! Where the sandbox enforces: a launcher of the hook's own.

    use super::*;
    use crate::sandbox::{enforcing_here, launcher};
    use cineris_protocol::Part;
    use cineris_protocol::event::{Event, Notice};
    use cineris_protocol::item::Delivery;
    use cineris_protocol::methods::TurnStartParams;

    /// A hook without `write` writes nothing, and fails when it tries; one
    /// with it writes the workspace, and a protected path it changes is
    /// found by the look after it, which puts it back as it was and stops
    /// the turn (the look is `cineris.exec`'s, since the third security fix
    /// one that puts back). A hook of the stopped turn then starts nothing,
    /// and is no stop of its own (the review of step 28's second pull
    /// request, M2). A stop made by a hook of no turn (a `session.start`
    /// one's) while a turn runs stops that turn too: the call whose
    /// question was open does not run, whatever the answer. One made late
    /// by a hook of an earlier turn leaves the running turn going, and
    /// refuses the prompt queued behind it, which goes back to its sender.
    /// An extension's hook's program has no write root, whatever its
    /// manifest says (the maintainers' call on the early review of the
    /// script hooks, M2): at `supervised` it runs confined with none, writes
    /// nothing, and no look is made of it, so a protected path changed by
    /// hand stops nothing; at `read-only` it is refused before it runs, as
    /// one the engine would ask about, and writes nothing either.
    #[test]
    fn sys_a_hooks_writes_are_confined_and_looked_at() {
        if !enforcing_here() {
            return;
        }
        let world = World::new(
            "hook_confined",
            [
                crate::says("Hi."),
                crate::calls(&[("c1", "shell", json!({"command": "touch made.txt"}))]),
                crate::calls(&[("c2", "shell", json!({"command": "touch late.txt"}))]),
                crate::says("Done."),
                crate::says("Fine."),
            ],
        );
        world.put(".github/README", "ci\n");
        world.user_config(
            &[
                entry(
                    "reader",
                    "tool.after",
                    &this_binary("write", &["out.txt"]),
                    "",
                ),
                entry(
                    "scribble",
                    "tool.after",
                    &this_binary("write", &[".github/workflows/x.yml"]),
                    "write = true",
                ),
                // The extension's hook's program: allowed by this rule.
                format!(
                    "[[permissions.rules]]\neffect = \"exec\"\nmatch = {}\naction = \
                     \"allow\"\n",
                    serde_json::to_string(&format!(
                        "{} *",
                        std::env::current_exe().unwrap().display()
                    ))
                    .unwrap()
                ),
            ]
            .concat(),
        );
        // An extension whose hook, on an event no turn reaches, runs a
        // program with a write root.
        let writes = this_binary("write", &["guarded.txt"]);
        let guard = crate::ext_exec::extension_running(
            &world,
            "guard",
            &format!(
                "exec = [{}]\nwrite = [\"**\"]\nhooks = [\"compaction.before\"]",
                serde_json::to_string(&writes[0]).unwrap()
            ),
            &format!(
                "cineris.on('compaction.before', async () => {{ const ran = await \
                 cineris.exec({}, {}); throw new Error('ran: ' + JSON.stringify(ran)); }});\n",
                serde_json::to_string(&writes[0]).unwrap(),
                serde_json::to_string(&writes[1..]).unwrap()
            ),
        );
        let core = world.core_with(|options| {
            options.launcher = Some(launcher());
            options.extensions = vec![guard];
        });
        world.run(async {
            let front = Front::start(&world, &core).await;
            let after = || HookRun {
                input: json!({"event": "tool.after"}),
                tool: Some("edit".to_owned()),
                ..HookRun::default()
            };
            let reader = core
                .run_hook_command(&front.session, "reader", after())
                .await
                .unwrap();
            let HookOutcome::Failed(why) = &reader else {
                panic!("{reader:?}");
            };
            assert!(why.contains("exited with 4"), "{why}");
            assert!(world.read("out.txt").is_none());
            let scribble = core
                .run_hook_command(&front.session, "scribble", after())
                .await
                .unwrap();
            let HookOutcome::Stopped(note) = &scribble else {
                panic!("{scribble:?}");
            };
            assert!(note.contains(".github/workflows"), "{note}");
            assert!(note.contains("put back"), "{note}");
            assert!(
                world.read(".github/workflows/x.yml").is_none(),
                "the hook's protected change is put back"
            );
            // The stop holds back one prompt.
            let held = front.turn("first").await;
            assert_eq!(held.outcome, TurnOutcome::Denied);
            front.drain();
            // The turn is stopped: no more of its hooks starts, and one
            // that does not is no stop of its own (the review of the second
            // pull request, M2, and its re-check): it is not told as one,
            // and holds back no prompt, now that the first stop is used.
            let again = core
                .run_hook_command(&front.session, "reader", after())
                .await
                .unwrap();
            let HookOutcome::Failed(why) = &again else {
                panic!("{again:?}");
            };
            assert!(why.contains("its turn was stopped"), "{why}");
            front.drain();
            assert!(
                !front
                    .notices()
                    .iter()
                    .any(|notice| notice.contains("The hook `reader` was stopped")),
                "{:?}",
                front.notices()
            );
            let sent = front.turn("second").await;
            assert_eq!(
                sent.outcome,
                TurnOutcome::Completed,
                "a hook not started for a stopped turn held back a prompt"
            );
            // In the next turn, a protected path changed by hand since the
            // last look refuses the next start that has a write root.
            world.put(".github/workflows/new.yml", "on: push\n");
            let next = core
                .run_hook_command(
                    &front.session,
                    "scribble",
                    HookRun {
                        turn: Some("t_9".to_owned()),
                        ..after()
                    },
                )
                .await
                .unwrap();
            let HookOutcome::Stopped(note) = &next else {
                panic!("{next:?}");
            };
            assert!(note.contains("changed since the last command"), "{note}");
            assert_eq!(front.turn("third").await.outcome, TurnOutcome::Denied);
            front.drain();
            // A stop of no turn's while a turn runs, its question open.
            front.prompt("make it").await;
            let (id, asked) = front.approval().await;
            assert_eq!(asked.tool, "shell");
            let outside = core
                .run_hook_command(
                    &front.session,
                    "scribble",
                    HookRun {
                        turn: Some("t_0".to_owned()),
                        ..after()
                    },
                )
                .await
                .unwrap();
            assert!(matches!(outside, HookOutcome::Stopped(_)), "{outside:?}");
            front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
            front.turns(front.ends().len() + 1).await;
            assert!(
                world.read("made.txt").is_none(),
                "the running turn's call ran after a hook's stop"
            );
            // The stop holds back the next prompt.
            assert_eq!(front.turn("late").await.outcome, TurnOutcome::Denied);
            // A stop made late, by a hook of an earlier turn, while a turn
            // runs: that turn goes on, and the prompt queued behind it, sent
            // before the stop, is refused by it, and goes back whole to its
            // sender, as every prompt has its ticket (the re-check of step
            // 28's second pull request).
            front.prompt("go").await;
            let (id, _) = front.approval().await;
            front
                .op(Op::TurnStart(TurnStartParams {
                    session_id: front.session.clone(),
                    input: vec![Part::text("next")],
                    delivery: Some(Delivery::Queue),
                }))
                .await
                .unwrap();
            let late = core
                .run_hook_command(
                    &front.session,
                    "scribble",
                    HookRun {
                        turn: Some("t_7".to_owned()),
                        ..after()
                    },
                )
                .await
                .unwrap();
            assert!(matches!(late, HookOutcome::Stopped(_)), "{late:?}");
            front.answer(&id, ApprovalChoice::Deny).unwrap();
            let ended = front.ends().len();
            front.turns(ended + 2).await;
            let ends = front.ends();
            assert_eq!(ends[ended].outcome, TurnOutcome::Completed, "{ends:?}");
            assert_eq!(ends[ended + 1].outcome, TurnOutcome::Denied, "{ends:?}");
            let returned = front
                .events()
                .into_iter()
                .rev()
                .find_map(|event| match event.event {
                    Event::Notice(Notice::PromptReturned(returned)) => Some(returned),
                    _ => None,
                })
                .expect("the queued prompt given back");
            assert_eq!(
                returned.parts,
                [Part::text("next")],
                "a prompt refused by a later stop did not come back to its sender"
            );
            // An extension's hook's program has no write root, at
            // `supervised` and at `read-only`: it runs confined with none,
            // and fails to write; no look is made of it, so a protected path
            // changed by hand stops nothing, and no prompt is held back.
            world.put(".github/workflows/by-hand.yml", "on: push\n");
            for level in [None, Some(cineris_protocol::lease::Level::ReadOnly)] {
                if let Some(level) = &level {
                    front
                        .op(Op::SessionSet(
                            cineris_protocol::methods::SessionSetParams {
                                session_id: front.session.clone(),
                                settings: cineris_protocol::session::SessionSettings {
                                    level: Some(level.clone()),
                                    ..Default::default()
                                },
                            },
                        ))
                        .await
                        .unwrap();
                }
                // Of a turn no stop has reached: the first was stopped.
                let said = core
                    .hook_runs_in_turn(&front.session, "guard/hook.compaction.before", "t_99", 1)
                    .await
                    .unwrap();
                // At `read-only` the engine refuses the program before any
                // write root matters: it asks about a program it does not
                // know to only read, which no hook runs, so the hook is
                // refused, and nobody asked. What the level pins is that
                // the hook still writes nothing.
                let failed = if level.is_some() {
                    "the level `read-only` asks"
                } else {
                    "the write failed"
                };
                assert!(
                    said[0].as_deref().is_some_and(|why| why.contains(failed)
                        && !why.contains("changed since the last command")),
                    "{level:?}: {said:?}"
                );
                assert!(
                    world.read("guarded.txt").is_none(),
                    "{level:?}: a hook's program wrote"
                );
            }
            front.drain();
            assert!(
                !front
                    .notices()
                    .iter()
                    .any(|notice| notice.contains("was stopped by the look")
                        && notice.contains("guard")),
                "{:?}",
                front.notices()
            );
            assert_eq!(
                front.turn("after the guard").await.outcome,
                TurnOutcome::Completed
            );
            front.close().await;
        });
    }

    /// A protected path a hook's start changed, put back, and then changed
    /// again by something else (here the test's own hand) is a change from
    /// outside: the next start with a write root is refused, and a session
    /// that closes before another start says so in its log. The early
    /// review of the hook commands found that a path the look had just
    /// recorded counted as the harness's own until the next look after,
    /// so that a change to it passed both checks unreported.
    #[test]
    fn sys_a_protected_path_changed_again_after_a_hook_is_found_by_the_next_start_and_the_close() {
        if !enforcing_here() {
            return;
        }
        let world = World::new(
            "hook_changed_again",
            [crate::says("Hi."), crate::says("Hi.")],
        );
        world.put(".github/README", "ci\n");
        world.user_config(&entry(
            "scribble",
            "tool.after",
            &this_binary("write", &[".github/workflows/x.yml"]),
            "write = true",
        ));
        let after = |turn: &str| HookRun {
            input: json!({"event": "tool.after"}),
            tool: Some("edit".to_owned()),
            turn: Some(turn.to_owned()),
            ..HookRun::default()
        };
        // The next start is refused.
        let core = world.core_with(|options| options.launcher = Some(launcher()));
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.turn("hello").await;
            let first = core
                .run_hook_command(&front.session, "scribble", after("t_2"))
                .await
                .unwrap();
            assert!(matches!(first, HookOutcome::Stopped(_)), "{first:?}");
            assert!(world.read(".github/workflows/x.yml").is_none());
            world.put(".github/workflows/x.yml", "changed again\n");
            let next = core
                .run_hook_command(&front.session, "scribble", after("t_3"))
                .await
                .unwrap();
            let HookOutcome::Stopped(note) = &next else {
                panic!("the next start was not refused: {next:?}");
            };
            assert!(
                note.contains("changed since the last command") && note.contains("x.yml"),
                "{note}"
            );
            assert_eq!(
                world.read(".github/workflows/x.yml").as_deref(),
                Some("changed again\n")
            );
            front.close().await;
        });
        // The close says so.
        std::fs::remove_file(world.workspace.join(".github/workflows/x.yml")).unwrap();
        let core = world.core_with(|options| options.launcher = Some(launcher()));
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.turn("hello").await;
            let first = core
                .run_hook_command(&front.session, "scribble", after("t_2"))
                .await
                .unwrap();
            assert!(matches!(first, HookOutcome::Stopped(_)), "{first:?}");
            world.put(".github/workflows/x.yml", "changed again\n");
            front.close().await;
        });
        let logs = world.logs();
        let last = logs
            .iter()
            .map(|log| std::fs::read_to_string(log).unwrap())
            .find(|log| log.contains("changed since the last command"));
        let Some(last) = last else {
            panic!("no session's log says the change at its close: {logs:?}");
        };
        assert!(last.contains("x.yml"), "{last}");
    }
}
