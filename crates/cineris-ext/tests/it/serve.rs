//! The order a host call is checked in, and what each place a script runs
//! may do: registering only while it is first evaluated, a question never
//! from a hook, a write only from a tool call.

use std::time::Duration;

use cineris_base::path::{AbsPath, PathStyle};
use cineris_ext::capability::Capabilities;
use cineris_ext::host::{HostCall, HostError, Phase, Scope, serve};
use cineris_ext::manifest::Manifest;
use cineris_testkit::block_on;
use serde_json::json;

use crate::{FakeHost, manifest};

fn capabilities(table: &str) -> Capabilities {
    let parsed = Manifest::parse(&manifest(table)).unwrap();
    let workspace = AbsPath::parse("/work/app", PathStyle::Posix).unwrap();
    Capabilities::compile(&parsed.capabilities, &workspace).unwrap()
}

fn exec(program: &str) -> HostCall {
    HostCall::Exec {
        program: program.to_owned(),
        args: vec!["x".to_owned()],
        cwd: None,
        stdin: None,
        timeout_ms: Some(60_000),
    }
}

#[test]
fn the_manifest_is_asked_before_the_host() {
    let host = FakeHost::new();
    let capabilities = capabilities("exec = [\"gh\"]\nwrite = [\"out/**\"]\nexec_network = true");
    let refused = block_on(serve(
        exec("rm"),
        &capabilities,
        &host,
        Phase::Tool,
        Duration::from_secs(5),
    ));
    assert!(matches!(refused, Err(HostError::Refused(_))), "{refused:?}");
    assert!(host.asked().is_empty());
    let ran = block_on(serve(
        exec("gh"),
        &capabilities,
        &host,
        Phase::Tool,
        Duration::from_secs(5),
    ))
    .unwrap();
    assert_eq!(ran["stdout"], "out");
    // The manifest's network and write roots go with the request.
    assert_eq!(
        host.asked(),
        ["exec gh x network=true roots=1 timeout=5000"]
    );
}

#[test]
fn nothing_but_registering_happens_while_the_main_module_is_first_evaluated() {
    let host = FakeHost::new();
    let capabilities = capabilities("exec = [\"gh\"]");
    for call in [
        exec("gh"),
        HostCall::Log("x".to_owned()),
        HostCall::Notify("x".to_owned()),
    ] {
        let result = block_on(serve(
            call,
            &capabilities,
            &host,
            Phase::Evaluation,
            Duration::from_secs(5),
        ));
        assert!(matches!(result, Err(HostError::NotHere(_))), "{result:?}");
    }
    assert!(host.asked().is_empty());
}

#[test]
fn a_hook_cannot_put_a_question_and_only_a_tool_call_writes() {
    let host = FakeHost::new();
    let capabilities = capabilities("write = [\"**\"]");
    let ask = HostCall::Ask {
        question: "go on?".to_owned(),
        choices: Vec::new(),
    };
    let result = block_on(serve(
        ask.clone(),
        &capabilities,
        &host,
        Phase::Hook,
        Duration::from_secs(5),
    ));
    assert!(matches!(result, Err(HostError::NotHere(_))), "{result:?}");
    assert_eq!(
        block_on(serve(
            ask,
            &capabilities,
            &host,
            Phase::Command,
            Duration::from_secs(5)
        ))
        .unwrap(),
        json!("yes")
    );
    for phase in [Phase::Hook, Phase::Command] {
        let write = HostCall::Write {
            path: "x.txt".to_owned(),
            text: "x".to_owned(),
        };
        let result = block_on(serve(
            write,
            &capabilities,
            &host,
            phase,
            Duration::from_secs(5),
        ));
        assert!(
            matches!(result, Err(HostError::NotHere(_))),
            "{phase:?}: {result:?}"
        );
    }
    assert_eq!(host.asked(), ["ask go on?"]);
}

#[test]
fn a_programs_time_is_never_longer_than_the_call_has_left() {
    // A program asked for a minute, in a call with five seconds left.
    let host = FakeHost::new();
    let capabilities = capabilities("exec = [\"gh\"]");
    block_on(serve(
        exec("gh"),
        &capabilities,
        &host,
        Phase::Tool,
        Duration::from_secs(5),
    ))
    .unwrap();
    assert!(
        host.asked()[0].ends_with("timeout=5000"),
        "{:?}",
        host.asked()
    );
}

#[test]
fn oversized_arguments_and_keys_are_refused_before_the_host() {
    let host = FakeHost::new();
    let capabilities = capabilities("exec = [\"gh\"]");
    let many = HostCall::Exec {
        program: "gh".to_owned(),
        args: vec![String::new(); 257],
        cwd: None,
        stdin: None,
        timeout_ms: None,
    };
    let long = HostCall::Exec {
        program: "gh".to_owned(),
        args: vec!["x".repeat(32 * 1024 + 1)],
        cwd: None,
        stdin: None,
        timeout_ms: None,
    };
    let key = HostCall::StateSet {
        key: "k".repeat(257),
        value: Some(json!(1)),
        scope: Scope::Project,
    };
    for call in [many, long, key] {
        let result = block_on(serve(
            call,
            &capabilities,
            &host,
            Phase::Tool,
            Duration::from_secs(5),
        ));
        assert!(matches!(result, Err(HostError::Failed(_))), "{result:?}");
    }
    assert!(host.asked().is_empty());
}
