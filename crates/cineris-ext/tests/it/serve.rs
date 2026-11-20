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

/// A refusal of a path names it as the script gave it and is the same
/// however the path failed, and a manifest that grants no reading refuses
/// before the path is resolved at all: a script learns nothing of where a
/// path leads, or whether it leads anywhere (the review of step 28).
#[test]
fn a_refused_path_tells_the_script_nothing_of_where_it_leads() {
    let read = |path: &str| HostCall::Read {
        path: path.to_owned(),
        offset: 0,
        limit: None,
    };
    let refusal = |call: HostCall, table: &str, host: &FakeHost| {
        let refused = block_on(serve(
            call,
            &capabilities(table),
            host,
            Phase::Tool,
            Duration::from_secs(5),
        ));
        let Err(HostError::Refused(refusal)) = refused else {
            panic!("not refused as the manifest refuses: {refused:?}");
        };
        refusal.detail
    };
    // A host with a workspace, whose resolver would answer.
    let scratch = cineris_testkit::Scratch::for_test(
        env!("CARGO_TARGET_TMPDIR"),
        "cineris-ext-it",
        "refused_path",
    );
    scratch.dir("ws");
    let host = FakeHost::in_workspace(&scratch.join("ws"), None);
    // No `read` glob: the path is never resolved.
    assert_eq!(
        refusal(read("~/.ssh/id_ed25519"), "", &host),
        "reading `~/.ssh/id_ed25519`"
    );
    assert_eq!(host.resolves.load(std::sync::atomic::Ordering::SeqCst), 0);
    // A path that leads outside is named as given, not where it leads.
    assert_eq!(
        refusal(read("../elsewhere/x"), "read = [\"**\"]", &host),
        "reading `../elsewhere/x`"
    );
    // A path that does not resolve is refused just as one that leads
    // outside: this host has no workspace, and its resolver fails.
    assert_eq!(
        refusal(read("src/x"), "read = [\"**\"]", &FakeHost::new()),
        "reading `src/x`"
    );
    // So is a write with no `write` glob.
    assert_eq!(
        refusal(
            HostCall::Write {
                path: "out/a".to_owned(),
                text: String::new(),
            },
            "read = [\"**\"]",
            &host,
        ),
        "writing `out/a`"
    );
    assert!(host.asked().is_empty());
}

/// A question offers a bounded number of choices, each shown cut and with
/// nothing that acts unseen, and an answer that is not one of them is no
/// answer.
#[test]
fn a_question_is_bounded_and_its_answer_is_one_of_its_choices() {
    let host = FakeHost::new();
    let ask = |choices: &[&str]| HostCall::Ask {
        question: "Go on?\u{1b}[2J".to_owned(),
        choices: choices.iter().map(|choice| (*choice).to_owned()).collect(),
    };
    let served = |call: HostCall| {
        block_on(serve(
            call,
            &capabilities(""),
            &host,
            Phase::Tool,
            Duration::from_secs(5),
        ))
    };
    // The fake host answers "yes", which is not offered here.
    assert_eq!(served(ask(&["a", "b"])).unwrap(), json!(null));
    assert_eq!(host.asked(), ["ask Go on?\\u{1b}[2J"]);
    assert_eq!(served(ask(&["yes", "no"])).unwrap(), json!("yes"));
    let refused = served(ask(&["1", "2", "3", "4", "5", "6", "7", "8", "9"]));
    assert!(
        matches!(&refused, Err(HostError::Failed(text)) if text.contains("at most 8")),
        "{refused:?}"
    );
    assert_eq!(host.asked().len(), 2);
}

/// What a script tells the user or the log cannot draw what the harness
/// draws: a control character is shown as an escape.
#[test]
fn a_notice_and_a_line_of_the_log_show_nothing_unseen() {
    let host = FakeHost::new();
    for call in [
        HostCall::Notify("done\u{1b}[1A\rCineris: allowed".to_owned()),
        HostCall::Progress("half\u{7}".to_owned()),
        HostCall::Log("line\nforged".to_owned()),
    ] {
        block_on(serve(
            call,
            &capabilities(""),
            &host,
            Phase::Tool,
            Duration::from_secs(5),
        ))
        .unwrap();
    }
    let noted = host.asked();
    assert_eq!(noted.len(), 3);
    for noted in noted {
        assert!(!noted.chars().any(char::is_control), "{noted:?}");
    }
}
