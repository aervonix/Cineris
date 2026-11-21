//! The order a host call is checked in, and what each place a script runs
//! may do: registering only while it is first evaluated, a question never
//! from a hook, a write only from a tool call.

use std::time::Duration;

use cineris_base::path::{AbsPath, PathStyle};
use cineris_ext::capability::Capabilities;
use cineris_ext::host::{FetchCall, HostCall, HostError, Phase, Scope, serve};
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

fn fetch_call(url: &str) -> FetchCall {
    FetchCall {
        url: url.to_owned(),
        method: "GET".to_owned(),
        headers: Vec::new(),
        body: None,
        timeout_ms: Some(120_000),
    }
}

fn fetch(url: &str) -> HostCall {
    HostCall::Fetch(fetch_call(url))
}

#[test]
fn a_fetch_reaches_the_host_only_for_a_host_the_manifest_names() {
    let host = FakeHost::new();
    let capabilities =
        capabilities("net = [\"api.example.com\", \"*.cdn.example\", \"127.0.0.1\"]");
    for refused in [
        "https://example.com/x",
        "https://evil.com/?q=api.example.com",
        "https://api.example.com.evil.com/",
        "https://cdn.example/",
        "http://127.0.0.2:8080/",
    ] {
        let result = block_on(serve(
            fetch(refused),
            &capabilities,
            &host,
            Phase::Tool,
            Duration::from_secs(5),
        ));
        assert!(
            matches!(result, Err(HostError::Refused(_))),
            "{refused}: {result:?}"
        );
    }
    assert!(host.asked().is_empty(), "{:?}", host.asked());
    let fetched = block_on(serve(
        fetch("https://API.Example.com:443/issues?q=1"),
        &capabilities,
        &host,
        Phase::Tool,
        Duration::from_secs(5),
    ))
    .unwrap();
    assert_eq!(fetched["status"], 200);
    assert_eq!(fetched["body"], "fetched");
    // Only the hosts named literally go with it, and its time is the
    // call's at most.
    assert_eq!(
        host.asked(),
        [
            "fetch GET https://API.Example.com:443/issues?q=1 host=api.example.com literal=api.example.com timeout=5000"
        ]
    );
}

#[test]
fn a_urls_host_is_read_without_its_port_user_or_brackets() {
    use cineris_ext::host::url_host;
    assert_eq!(
        url_host("https://Example.COM:8443/a").unwrap(),
        "example.com"
    );
    assert_eq!(url_host("http://[::1]:80/x").unwrap(), "::1");
    assert_eq!(url_host("https://a.b?x=1").unwrap(), "a.b");
    assert_eq!(url_host("https://a.b#frag").unwrap(), "a.b");
    for refused in [
        "https://user@api.example.com/",
        "https://api.example.com@evil.com/",
        "no-scheme.example.com",
        "https:///path",
        "http://[::1/x",
    ] {
        assert!(url_host(refused).is_err(), "{refused}");
    }
    for (plain, host) in [
        ("https://1.2.3.4/", "1.2.3.4"),
        ("https://[0:0:0:0:0:0:0:1]:8443/", "::1"),
        ("https://[FE80:0::A]/", "fe80::a"),
        ("HTTPS://Example.COM/", "example.com"),
        ("https://under_score-1.example/", "under_score-1.example"),
        ("https://example.com.:443/", "example.com."),
    ] {
        assert_eq!(url_host(plain).unwrap(), host, "{plain}");
    }
}

/// A URL the fetch client would read as another host than the one the
/// manifest and the decision are about never reaches the host, even where
/// the manifest allows any host.
#[test]
fn a_url_the_client_would_read_as_another_host_never_reaches_the_host() {
    let host = FakeHost::new();
    let capabilities = capabilities("net = [\"*\"]");
    for refused in [
        "https://evil.test\\.cdn.example/",
        "https://ex%61mple.com/",
        "https://exa\tmple.com/",
        "https://exa\nmple.com/",
        "https://0x7f.1/",
        "https://2130706433/",
        "https://127.1/",
        "https://0177.0.0.1/",
        "https://127.0.0.01/",
        "https://1.2.3.4./",
        "https://1.2.3.0x4/",
        "https://[::1%25lo]/",
        "https://[::ffff:127.0.0.1]/",
        "https://[::1]evil.test/",
        // The standard skips any run of `/` and `\\` after the scheme, and
        // drops a tab or a line break wherever it is: each of these is
        // `good.test`, not `evil.test`.
        "https:good.test://evil.test/",
        "https:/\\good.test://evil.test/",
        "https:\\\\good.test://evil.test/",
        "https:\t//good.test://evil.test/",
        "https:/\t/good.test://evil.test/",
        "https:@good.test://evil.test/",
        " https://good.test/",
        "https://good.test/\n",
        "ftp://good.test/",
    ] {
        let result = block_on(serve(
            fetch(refused),
            &capabilities,
            &host,
            Phase::Tool,
            Duration::from_secs(5),
        ));
        assert!(
            matches!(result, Err(HostError::Failed(_))),
            "{refused:?}: {result:?}"
        );
    }
    assert!(host.asked().is_empty(), "{:?}", host.asked());
}

/// Only the host itself goes with a fetch as one that may be outside the
/// internet, and only when the manifest names it literally: a name `*` or a
/// domain's pattern matched takes nothing, whatever else the manifest names.
#[test]
fn only_a_host_named_literally_may_be_outside_the_internet() {
    let host = FakeHost::new();
    let capabilities = capabilities("net = [\"*\", \"localhost\", \"10.0.0.5\", \"[0:0::1]\"]");
    for url in [
        "https://rebind.attacker.example/",
        "https://localhost:8443/",
        "http://10.0.0.5:8080/",
        "http://[0:0::1]/",
    ] {
        block_on(serve(
            fetch(url),
            &capabilities,
            &host,
            Phase::Tool,
            Duration::from_secs(5),
        ))
        .unwrap();
    }
    assert_eq!(
        host.asked(),
        [
            "fetch GET https://rebind.attacker.example/ host=rebind.attacker.example literal= timeout=5000",
            "fetch GET https://localhost:8443/ host=localhost literal=localhost timeout=5000",
            "fetch GET http://10.0.0.5:8080/ host=10.0.0.5 literal=10.0.0.5 timeout=5000",
            "fetch GET http://[0:0::1]/ host=::1 literal=::1 timeout=5000",
        ]
    );
}

/// A fetch takes 30 s at most: less when the script asks for less, and
/// never more than its call has left.
#[test]
fn a_fetch_takes_thirty_seconds_at_most_and_never_more_than_its_call() {
    let host = FakeHost::new();
    let capabilities = capabilities("net = [\"api.example.com\"]");
    for (asked, left, given) in [
        (Some(120_000), 300, 30_000),
        (Some(10_000), 300, 10_000),
        (None, 300, 30_000),
        (Some(120_000), 5, 5_000),
    ] {
        let call = HostCall::Fetch(FetchCall {
            timeout_ms: asked,
            ..fetch_call("https://api.example.com/")
        });
        block_on(serve(
            call,
            &capabilities,
            &host,
            Phase::Tool,
            Duration::from_secs(left),
        ))
        .unwrap();
        let asked_last = host.asked().last().cloned().unwrap_or_default();
        assert!(
            asked_last.ends_with(&format!("timeout={given}")),
            "{asked_last}"
        );
    }
}

/// What a fetch call prints shows no header value, no body and no query.
#[test]
fn a_fetch_call_prints_without_what_may_be_a_credential() {
    let call = FetchCall {
        headers: vec![("authorization".to_owned(), "Bearer s3cret".to_owned())],
        body: Some("s3cret".to_owned()),
        ..fetch_call("https://api.example.com/x?token=s3cret")
    };
    let printed = format!("{call:?}");
    assert!(!printed.contains("s3cret"), "{printed}");
    assert!(printed.contains("authorization"), "{printed}");
    assert!(!printed.contains("/x"), "{printed}");
    assert!(printed.contains("https://api.example.com…"), "{printed}");
    let output = cineris_ext::host::FetchOutput {
        status: 200,
        headers: vec![("set-cookie".to_owned(), "session=s3cret".to_owned())],
        body: b"s3cret".to_vec(),
        cut: false,
    };
    let printed = format!("{output:?}");
    assert!(!printed.contains("s3cret"), "{printed}");
    assert!(printed.contains("set-cookie"), "{printed}");
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

fn state_get(key: &str, scope: Scope) -> HostCall {
    HostCall::StateGet {
        key: key.to_owned(),
        scope,
    }
}

fn state_set(key: &str, value: Option<serde_json::Value>, scope: Scope) -> HostCall {
    HostCall::StateSet {
        key: key.to_owned(),
        value,
        scope,
    }
}

/// A stored value needs no capability and is served in a tool call, a hook
/// and a command alike: it touches only the extension's own values. A key
/// with no value gives `null`, and a set to `null` reaches the host as a
/// removal, so a stored value is never `null`.
#[test]
fn a_value_needs_no_capability_and_a_set_to_null_removes_it() {
    let host = FakeHost::new();
    let capabilities = capabilities("");
    let served = |call: HostCall, phase: Phase| {
        block_on(serve(
            call,
            &capabilities,
            &host,
            phase,
            Duration::from_secs(5),
        ))
    };
    for phase in [Phase::Tool, Phase::Hook, Phase::Command] {
        assert_eq!(
            served(state_get("n", Scope::Session), phase).unwrap(),
            json!(null)
        );
        served(state_set("n", Some(json!({"a": 1})), Scope::Session), phase).unwrap();
        assert_eq!(
            served(state_get("n", Scope::Session), phase).unwrap(),
            json!({"a": 1})
        );
        assert_eq!(
            served(state_get("n", Scope::Project), phase).unwrap(),
            json!(null),
            "the project's values are another store"
        );
        served(state_set("n", Some(json!(null)), Scope::Session), phase).unwrap();
        assert_eq!(
            served(state_get("n", Scope::Session), phase).unwrap(),
            json!(null)
        );
        assert!(
            host.state.lock().unwrap().is_empty(),
            "a set to `null` removes the value"
        );
    }
}

/// A key is 1 to 256 bytes, counted in bytes: at 256 it is served, empty
/// or past it the call is refused before the host, in either scope.
#[test]
fn a_key_is_one_to_256_bytes() {
    let host = FakeHost::new();
    let capabilities = capabilities("");
    let served = |call: HostCall| {
        block_on(serve(
            call,
            &capabilities,
            &host,
            Phase::Tool,
            Duration::from_secs(5),
        ))
    };
    for scope in [Scope::Session, Scope::Project] {
        // 256 bytes of 128 characters, and of 256.
        for key in ["é".repeat(128), "k".repeat(256)] {
            served(state_set(&key, Some(json!(1)), scope)).unwrap();
            assert_eq!(served(state_get(&key, scope)).unwrap(), json!(1));
        }
        for key in [String::new(), "é".repeat(129), "k".repeat(257)] {
            for call in [
                state_set(&key, Some(json!(1)), scope),
                state_get(&key, scope),
            ] {
                let refused = served(call);
                assert!(
                    matches!(&refused, Err(HostError::Failed(why)) if why.contains("1 to 256 bytes")),
                    "{refused:?}"
                );
            }
        }
    }
    assert_eq!(host.state.lock().unwrap().len(), 4);
}
