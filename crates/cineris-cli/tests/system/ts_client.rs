//! A client written in TypeScript, which knows the protocol only from the
//! generated declarations (`docs/protocol/ts/cineris-protocol.d.ts`), runs a
//! turn against `cineris serve --stdio` (`docs/steps/27/answered.md`, point
//! 7). The client is `ts_client/turn.mts`, and Node runs it as it is,
//! stripping its types: Node 22.18 or later (23.6 or later in the 23 line).
//!
//! Node checks no types. The jobs that run this test check them first, with
//! `tsc` and `ts_client/tsconfig.json`, so that a change to the protocol that
//! the client no longer fits fails there (`docs/testing-and-ci.md`, "The
//! TypeScript client"). The client also checks each field it reads when it
//! reads it, so a field renamed or removed fails this test as well.
//!
//! Where it runs, by [`ASKED`] and `CI`:
//!
//! | | Node new enough on `PATH` | Not |
//! |---|---|---|
//! | `CINERIS_TS_CLIENT` set | runs | fails |
//! | `CI` set, and not `CINERIS_TS_CLIENT` | says it was not run | says it was not run |
//! | Neither | runs | says it was not run |
//!
//! The blocking test jobs set `CI` and not the variable, so they never
//! depend on Node; the jobs that set it cannot pass without running it.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

use crate::{CINERIS, Machine, Run, shown};

/// The variable that asks for this test: where it is set, the test must run.
const ASKED: &str = "CINERIS_TS_CLIENT";

/// The client.
const CLIENT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/system/ts_client/turn.mts"
);

/// A file the agent may not write without asking, so that its write is put
/// to the client.
const PROTECTED: &str = ".github/workflows/ci.yml";

/// The client opens a session, sends a prompt, answers the approval the
/// turn asks for, and prints the turn's outcome and the assistant's text;
/// its answer reached the server, which made the write it allowed.
#[test]
fn sys_a_typescript_client_runs_a_turn() {
    let Some(node) = node() else {
        return;
    };
    let machine = Machine::with_config("ts-client", "index.map = false\n");
    machine.reply_tool(
        "c1",
        "write",
        &json!({"path": PROTECTED, "content": "on: push\n"}),
    );
    machine.reply_text("Written, as you asked.");
    let workspace = machine.workspace.to_str().unwrap();
    let run = Run::of(
        machine
            .command_as(
                &node,
                &[
                    // Type stripping is experimental in the 22 line, and says
                    // so on standard error.
                    "--disable-warning=ExperimentalWarning",
                    CLIENT,
                    CINERIS,
                    workspace,
                    "set up CI",
                    "allow_once",
                ],
            )
            .output()
            .expect("node starts"),
    );
    run.expect_code(0);
    assert_eq!(run.stdout.lines().count(), 1, "one line\n{}", shown(&run));
    let printed: Value = serde_json::from_str(&run.stdout)
        .unwrap_or_else(|error| panic!("not JSON ({error})\n{}", shown(&run)));
    assert_eq!(printed["outcome"], "completed", "{printed}");
    assert_eq!(printed["stop_reason"], "end", "{printed}");
    assert_eq!(printed["text"], "Written, as you asked.", "{printed}");
    assert_eq!(printed["approvals"], json!(["allow_once"]), "{printed}");
    assert!(
        printed["turn_id"]
            .as_str()
            .is_some_and(|turn| !turn.is_empty()),
        "{printed}"
    );
    assert_eq!(
        std::fs::read_to_string(machine.workspace.join(PROTECTED)).unwrap(),
        "on: push\n",
        "the write the client allowed"
    );
}

/// The Node to run the client with, or `None` when the test is not to run
/// here, having said why on standard error.
fn node() -> Option<PathBuf> {
    let asked = std::env::var_os(ASKED).is_some_and(|value| !value.is_empty());
    match here(asked, std::env::var_os("CI").is_some(), usable_node) {
        Here::Run(node) => Some(node),
        Here::NotRun(why) => {
            eprintln!("not run: {why}");
            None
        }
        Here::Wanting(problem) => panic!("{ASKED} is set, and {problem}"),
    }
}

/// What the test does on a machine.
#[derive(Debug, PartialEq)]
enum Here {
    /// It runs, with this Node.
    Run(PathBuf),
    /// It is not run here, for this reason, and passes.
    NotRun(String),
    /// It was asked for and cannot run, for this reason: it fails.
    Wanting(String),
}

/// What the test does where [`ASKED`] is set or not, and `CI` is set or not,
/// with `node` looking for the Node to run, which is not done where the
/// answer does not depend on it (the module's table).
fn here(asked: bool, ci: bool, node: impl FnOnce() -> Result<PathBuf, String>) -> Here {
    if ci && !asked {
        return Here::NotRun(format!(
            "in CI only a job that sets {ASKED} runs it (`ts-client`, in ci.yml and the nightly)"
        ));
    }
    match node() {
        Ok(node) => Here::Run(node),
        Err(problem) if asked => Here::Wanting(problem),
        Err(problem) => Here::NotRun(problem),
    }
}

/// `node` on `PATH`, when it is one that runs a `.mts` file as it is.
fn usable_node() -> Result<PathBuf, String> {
    let output = Command::new("node")
        .arg("--version")
        .output()
        .map_err(|error| format!("there is no `node` on PATH ({error})"))?;
    let said = String::from_utf8_lossy(&output.stdout);
    let said = said.trim();
    let version = version(said).ok_or_else(|| format!("`node --version` said {said:?}"))?;
    if strips_types(version) {
        Ok(PathBuf::from("node"))
    } else {
        Err(format!(
            "Node {said} does not run TypeScript as it is: 22.18 or later is needed"
        ))
    }
}

/// The major and minor numbers of a version as `node --version` writes it:
/// `v22.18.0`.
fn version(said: &str) -> Option<(u32, u32)> {
    let mut parts = said.strip_prefix('v')?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// Whether a Node of this version strips a file's types by default: from
/// 22.18 in the 22 line, and from 23.6 on.
fn strips_types((major, minor): (u32, u32)) -> bool {
    match major {
        22 => minor >= 18,
        23 => minor >= 6,
        _ => major > 23,
    }
}

/// The module's table, and the oldest Node of each line that strips types.
/// Not a system test: it starts nothing, so it runs in the blocking jobs.
#[test]
fn it_runs_where_it_is_asked_for_and_never_in_ci_unasked() {
    let found = || Ok(PathBuf::from("node"));
    let missing = || Err("no node".to_owned());
    assert!(matches!(
        here(false, true, || panic!(
            "Node is looked for in a blocking job"
        )),
        Here::NotRun(_)
    ));
    assert_eq!(here(true, true, found), Here::Run(PathBuf::from("node")));
    assert_eq!(
        here(true, true, missing),
        Here::Wanting("no node".to_owned())
    );
    assert_eq!(
        here(true, false, missing),
        Here::Wanting("no node".to_owned())
    );
    assert_eq!(here(false, false, found), Here::Run(PathBuf::from("node")));
    assert_eq!(
        here(false, false, missing),
        Here::NotRun("no node".to_owned())
    );

    let strips = |said: &str| strips_types(version(said).unwrap());
    for old in ["v20.19.5", "v22.17.1", "v23.5.0"] {
        assert!(!strips(old), "{old}");
    }
    for new in ["v22.18.0", "v22.20.1", "v23.6.0", "v24.0.0", "v26.13.0"] {
        assert!(strips(new), "{new}");
    }
    assert_eq!(version("22.18.0"), None);
    assert_eq!(version("vx.1"), None);
}
