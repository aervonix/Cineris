//! An extension's tools in a session (step 28): found through
//! `tool_search`, called through `tool_call`, and each host call its script
//! makes decided by the session's permission engine after the manifest
//! allowed it. The extension is approved by the test, as the harness's own
//! approval will (`cineris_ext::engine::register`), and handed in through
//! `CoreOptions::extensions`.

use std::path::Path;
use std::sync::Arc;

use cineris_core::ext::ApprovedExtension;
use cineris_ext::folder::Snapshot;
use cineris_protocol::methods::{ApprovalChoice, ApprovalDetail};
use cineris_testkit::{FakeClock, FakeTimer, block_on};
use serde_json::json;

use crate::{Front, World, calls, says};

/// Writes an extension called `probe` in `folder`, with these
/// capabilities and this script, and approves it as it is.
fn approve(folder: &Path, capabilities: &str, script: &str) -> ApprovedExtension {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"A test extension\"\nmain = \"main.js\"\n[capabilities]\n{capabilities}\n"
        ),
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), script).unwrap();
    let snapshot = Arc::new(Snapshot::read(folder).unwrap());
    let timer = FakeTimer::manual(&FakeClock::new(cineris_base::UtcTime::UNIX_EPOCH));
    let (manifest, registration) =
        block_on(cineris_ext::engine::register(Arc::clone(&snapshot), &timer)).unwrap();
    ApprovedExtension {
        folder: folder.to_path_buf(),
        hash: snapshot.hash(),
        manifest,
        registration,
    }
}

/// A call of the extension's tool `name` with `arguments`.
fn call_tool(id: &str, name: &str, arguments: serde_json::Value) -> cineris_testkit::Response {
    calls(&[(
        id,
        "tool_call",
        json!({"tool": format!("probe/{name}"), "arguments": arguments}),
    )])
}

#[test]
fn an_extensions_tool_is_called_through_tool_call_after_the_user_allows_it() {
    let world = World::new(
        "ext_tool_call",
        [call_tool("c1", "echo", json!({"x": 1})), says("Done.")],
    );
    let extension = approve(
        &world.dir.join("ext"),
        "",
        "cineris.tool({ name: 'echo', description: 'Gives its arguments back', \
         input: { type: 'object', properties: { x: { type: 'number' } } }, \
         run: (args) => ({ got: args }) });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    let made = cineris_ext::engine::runtimes_made();
    world.run(async {
        let front = Front::start(&world, &core).await;
        // Opening a session with an extension starts no engine.
        assert_eq!(cineris_ext::engine::runtimes_made(), made);
        front.prompt("echo something").await;
        // A tool from outside asks unless a rule allows it.
        let (id, request) = front.approval().await;
        assert_eq!(request.tool, "tool_call");
        assert!(
            matches!(request.detail, ApprovalDetail::Tool(_)),
            "{:?}",
            request.detail
        );
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert_eq!(results[0].1, "success", "{results:?}");
        // The result is labelled as the extension's, and holds what the
        // script gave.
        assert!(
            results[0].2.contains("a tool of the extension `probe`"),
            "{}",
            results[0].2
        );
        assert!(results[0].2.contains("\"got\""), "{}", results[0].2);
        front.close().await;
    });
    assert!(cineris_ext::engine::runtimes_made() > made);
}

#[test]
fn a_host_read_in_the_workspace_is_allowed_as_the_read_tool_is() {
    let world = World::new(
        "ext_host_read",
        [call_tool("c1", "show", json!({})), says("Done.")],
    );
    world.put("notes.md", "first line\nsecond line\n");
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"**\"]",
        "cineris.tool({ name: 'show', run: async () => cineris.fs.read('notes.md', { offset: 1 }) });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("show the notes").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        // A read in the workspace needs no question of its own.
        front.turns(1).await;
        let results = front.results();
        assert_eq!(results[0].1, "success", "{results:?}");
        assert!(results[0].2.contains("second line"), "{}", results[0].2);
        assert!(!results[0].2.contains("first line"), "{}", results[0].2);
        front.close().await;
    });
}

#[test]
fn a_host_write_of_a_protected_path_asks_once_only_and_keeps_no_rule() {
    let world = World::new(
        "ext_host_write",
        [
            call_tool("c1", "ci", json!({})),
            call_tool("c2", "ci", json!({})),
            says("Done."),
        ],
    );
    let extension = approve(
        &world.dir.join("ext"),
        "write = [\".github/**\"]",
        "cineris.tool({ name: 'ci', run: async () => { \
           try { await cineris.fs.write('.github/workflows/ci.yml', 'on: push\\n'); return 'wrote' } \
           catch (e) { return 'refused: ' + e.message } } });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("set up CI").await;
        // The call of the tool, then the host call's own question.
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        let (id, request) = front.approval().await;
        assert_eq!(request.tool, "probe/ci");
        assert_eq!(
            request.choices,
            [ApprovalChoice::AllowOnce, ApprovalChoice::Deny],
            "a host call's question keeps no rule"
        );
        assert!(request.adds.is_empty());
        let ApprovalDetail::Diff(change) = &request.detail else {
            panic!("not a change to a file: {:?}", request.detail);
        };
        assert!(change.diff.contains("+on: push"), "{}", change.diff);
        // A client that answers with a choice it was not offered keeps no
        // rule by it: the next write is asked about again.
        front.answer(&id, ApprovalChoice::AllowSession).unwrap();
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        let (id, request) = front.approval().await;
        assert_eq!(request.tool, "probe/ci");
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("wrote"), "{results:?}");
        assert!(results[1].2.contains("refused"), "{results:?}");
        front.close().await;
    });
    assert_eq!(
        world.read(".github/workflows/ci.yml").as_deref(),
        Some("on: push\n")
    );
}

#[test]
fn a_script_overwrites_a_file_only_after_reading_it_as_it_is() {
    let world = World::new(
        "ext_write_after_read",
        [
            call_tool("c1", "blind", json!({})),
            call_tool("c2", "careful", json!({})),
            says("Done."),
        ],
    );
    world.put("config.txt", "old\n");
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"**\"]\nwrite = [\"**\"]",
        "cineris.tool({ name: 'blind', run: async () => { \
           try { await cineris.fs.write('config.txt', 'blind\\n'); return 'wrote' } \
           catch (e) { return 'refused: ' + e.message } } });\n\
         cineris.tool({ name: 'careful', run: async () => { \
           const old = await cineris.fs.read('config.txt'); \
           await cineris.fs.write('config.txt', old + 'new\\n'); return 'wrote' } });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("change the config").await;
        // Each call of the tool asks; a write in the workspace does not.
        for _ in 0..2 {
            let (id, _) = front.approval().await;
            front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        }
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("refused"), "{results:?}");
        assert!(results[1].2.contains("wrote"), "{results:?}");
        front.close().await;
    });
    assert_eq!(world.read("config.txt").as_deref(), Some("old\nnew\n"));
}

#[test]
fn a_rule_that_denies_a_path_refuses_the_scripts_read_of_it() {
    let world = World::new(
        "ext_host_rule",
        [call_tool("c1", "peek", json!({})), says("Done.")],
    );
    world.user_config(
        "[[permissions.rules]]\neffect = \"read\"\nmatch = \"private/**\"\naction = \"deny\"\n",
    );
    world.put("private/plan.md", "secret plan");
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"**\"]",
        "cineris.tool({ name: 'peek', run: async () => { \
           try { return await cineris.fs.read('private/plan.md') } \
           catch (e) { return 'refused: ' + e.message } } });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("peek").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("refused"), "{results:?}");
        assert!(!results[0].2.contains("secret plan"), "{results:?}");
        front.close().await;
    });
}

#[test]
fn a_host_calls_question_names_what_it_touches_not_the_scripts_arguments() {
    let world = World::new(
        "ext_host_question",
        [call_tool("c1", "secret", json!({})), says("Done.")],
    );
    world.put(".env", "TOKEN=abc");
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"**\"]",
        "cineris.tool({ name: 'secret', run: async () => { \
           try { return await cineris.fs.read('.env') } \
           catch (e) { return 'refused: ' + e.message } } });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("read the env").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        // A file that looks like a secret is asked about, as the `read`
        // tool's would be, and the question names the file.
        let (id, request) = front.approval().await;
        assert_eq!(request.tool, "probe/secret");
        let ApprovalDetail::Target(target) = &request.detail else {
            panic!("not a target: {:?}", request.detail);
        };
        assert_eq!(target.effect, cineris_protocol::lease::Effect::Read);
        assert!(
            target.target.replace('\\', "/").ends_with("/.env"),
            "{}",
            target.target
        );
        assert_eq!(
            request.choices,
            [ApprovalChoice::AllowOnce, ApprovalChoice::Deny]
        );
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("refused"), "{results:?}");
        assert!(!results[0].2.contains("TOKEN"), "{results:?}");
        front.close().await;
    });
}

#[test]
fn an_extension_changed_since_it_was_approved_is_not_run() {
    let world = World::new(
        "ext_changed",
        [call_tool("c1", "echo", json!({})), says("Done.")],
    );
    let folder = world.dir.join("ext");
    let extension = approve(
        &folder,
        "",
        "cineris.tool({ name: 'echo', run: () => 'approved' });",
    );
    // Changed after it was approved, in the same tool and the same size.
    std::fs::write(
        folder.join("main.js"),
        "cineris.tool({ name: 'echo', run: () => 'tampered' });",
    )
    .unwrap();
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("echo").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert_eq!(results[0].1, "error", "{results:?}");
        assert!(
            results[0].2.contains("changed since they were approved"),
            "{results:?}"
        );
        assert!(!results[0].2.contains("tampered"), "{results:?}");
        front.close().await;
    });
}

/// "Allow once" on a host call allows that call alone: the next host call
/// of the same tool call that asks the same question is asked again, with
/// its own diff (the review of step 28: every host call of a tool call is
/// planned under its item, and an answer remembered for one allowed the
/// next unasked).
#[test]
fn allowing_a_host_write_once_allows_that_write_alone() {
    let world = World::new(
        "ext_once",
        [call_tool("c1", "twice", json!({})), says("Done.")],
    );
    world.user_config(
        "[[permissions.rules]]\neffect = \"write\"\nmatch = \"src/**\"\naction = \"ask\"\n",
    );
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"src/**\"]\nwrite = [\"src/**\"]",
        "cineris.tool({ name: 'twice', run: async () => { \
           await cineris.fs.write('src/a.rs', 'harmless\\n'); \
           const now = await cineris.fs.read('src/a.rs'); \
           try { await cineris.fs.write('src/a.rs', now + 'malicious\\n'); return 'wrote both' } \
           catch (e) { return 'refused: ' + e.message } } });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("write twice").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        let diff = |detail: &ApprovalDetail| match detail {
            ApprovalDetail::Diff(change) => change.diff.clone(),
            other => panic!("not a change to a file: {other:?}"),
        };
        let (id, request) = front.approval().await;
        assert!(diff(&request.detail).contains("+harmless"));
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        let (id, request) = front.approval().await;
        assert!(diff(&request.detail).contains("+malicious"));
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("refused"), "{results:?}");
        front.close().await;
    });
    assert_eq!(world.read("src/a.rs").as_deref(), Some("harmless\n"));
}

/// The capabilities a session holds for an extension are the approved
/// manifest's, and that manifest must be the one in the snapshot whose hash
/// was checked: one that differs is not run (the review of step 28).
#[test]
fn an_approved_manifest_other_than_the_one_in_the_folder_is_not_run() {
    let world = World::new(
        "ext_manifest",
        [call_tool("c1", "echo", json!({})), says("Done.")],
    );
    let mut extension = approve(
        &world.dir.join("ext"),
        "",
        "cineris.tool({ name: 'echo', run: () => 'ran' });",
    );
    extension.manifest.capabilities.read = vec!["**".to_owned()];
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("echo").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert_eq!(results[0].1, "error", "{results:?}");
        assert!(results[0].2.contains("not the one approved"), "{results:?}");
        front.close().await;
    });
}

/// A script's listing walks through the session's `list` tool, every entry
/// gated by the manifest over the session's own gate.
#[test]
fn a_scripts_listing_shows_only_what_its_manifest_reads() {
    let world = World::new(
        "ext_list",
        [call_tool("c1", "ls", json!({})), says("Done.")],
    );
    world.put("src/a.rs", "a");
    world.put("src/b.txt", "b");
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"src\", \"src/*.rs\"]",
        "cineris.tool({ name: 'ls', run: async () => (await cineris.fs.list('src')).text });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("list").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert_eq!(results[0].1, "success", "{results:?}");
        assert!(results[0].2.contains("a.rs"), "{results:?}");
        assert!(!results[0].2.contains("b.txt"), "{results:?}");
        front.close().await;
    });
}

/// A rule about the extension reaches each of its host calls: they carry
/// an `ext` effect beside their own, so a plain read in the workspace asks
/// under a rule that says to ask about the extension.
#[test]
fn a_rule_about_the_extension_reaches_its_host_calls() {
    let world = World::new(
        "ext_rule",
        [call_tool("c1", "show", json!({})), says("Done.")],
    );
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"ask\"\n",
    );
    world.put("notes.md", "plain notes");
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"**\"]",
        "cineris.tool({ name: 'show', run: async () => { \
           try { return await cineris.fs.read('notes.md') } \
           catch (e) { return 'refused: ' + e.message } } });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("show").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        let (id, request) = front.approval().await;
        assert_eq!(request.tool, "probe/show");
        front.answer(&id, ApprovalChoice::Deny).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("refused"), "{results:?}");
        assert!(!results[0].2.contains("plain notes"), "{results:?}");
        front.close().await;
    });
}

/// A read decided while a person was asked opens the file only if its path
/// still leads where it was judged to: a folder above it made a link while
/// the question was open is refused (the review of step 28).
#[test]
fn a_read_whose_path_changed_while_it_was_asked_about_is_refused() {
    let world = World::new(
        "ext_read_swap",
        [call_tool("c1", "secret", json!({})), says("Done.")],
    );
    world.put("conf/.env", "TOKEN=inside");
    let outside = world.dir.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join(".env"), "TOKEN=outside").unwrap();
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"**\"]",
        "cineris.tool({ name: 'secret', run: async () => { \
           try { return await cineris.fs.read('conf/.env') } \
           catch (e) { return 'refused: ' + e.message } } });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    let mut linked = true;
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("read the env").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        // The read asks: it looks like a secret.
        let (id, _) = front.approval().await;
        let conf = world.workspace.join("conf");
        std::fs::rename(&conf, world.workspace.join("conf-was")).unwrap();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&outside, &conf);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(&outside, &conf);
        if made.is_err() {
            println!("skipped: this machine cannot make a link");
            linked = false;
            std::fs::rename(world.workspace.join("conf-was"), &conf).unwrap();
        }
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        let results = front.results();
        if linked {
            assert!(results[0].2.contains("changed while"), "{results:?}");
            assert!(!results[0].2.contains("TOKEN"), "{results:?}");
        }
        front.close().await;
    });
}

/// A file with a NUL byte is not text, however well it decodes.
#[test]
fn a_file_with_a_nul_byte_is_not_read_as_text() {
    let world = World::new(
        "ext_nul",
        [call_tool("c1", "show", json!({})), says("Done.")],
    );
    world.put("data.bin", b"head\0tail");
    let extension = approve(
        &world.dir.join("ext"),
        "read = [\"**\"]",
        "cineris.tool({ name: 'show', run: async () => { \
           try { return await cineris.fs.read('data.bin') } \
           catch (e) { return 'refused: ' + e.message } } });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("show").await;
        let (id, _) = front.approval().await;
        front.answer(&id, ApprovalChoice::AllowOnce).unwrap();
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("not text"), "{results:?}");
        assert!(!results[0].2.contains("tail"), "{results:?}");
        front.close().await;
    });
}

/// With `fail_on_denial`, a host call nobody can be asked about stops the
/// turn: the script's call ends there, nothing more of it runs, a call the
/// model wrote after it is not run though a rule allows it, and the turn
/// ends denied without the model being asked again.
#[test]
fn a_host_call_nobody_can_answer_with_fail_on_denial_ends_the_scripts_call() {
    let world = World::new(
        "ext_stop",
        [
            calls(&[
                (
                    "c1",
                    "tool_call",
                    json!({"tool": "probe/ci", "arguments": {}}),
                ),
                (
                    "c2",
                    "write",
                    json!({"path": "after.txt", "content": "after\n"}),
                ),
            ]),
            says("never sent"),
        ],
    );
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n\n\
         [[permissions.rules]]\neffect = \"write\"\nmatch = \"after.txt\"\naction = \"allow\"\n",
    );
    let extension = approve(
        &world.dir.join("ext"),
        "write = [\".github/**\"]",
        "cineris.tool({ name: 'ci', run: async () => { \
           try { await cineris.fs.write('.github/workflows/ci.yml', 'on: push\\n') } catch (e) { } \
           return 'went on' } });",
    );
    let core = world.core_with(|options| {
        options.extensions = vec![extension];
        options.fail_on_denial = true;
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            cineris_core::ClientInfo::headless(),
            cineris_protocol::session::SessionSettings::default(),
        )
        .await
        .unwrap();
        let end = front.turn("set up CI").await;
        assert_eq!(
            end.outcome,
            cineris_protocol::event::TurnOutcome::Denied,
            "{end:?}"
        );
        let results = front.results();
        assert_eq!(results[0].1, "denied", "{results:?}");
        assert!(results[0].2.contains("stopped"), "{results:?}");
        assert_eq!(core.permission_denials(&front.session).len(), 1);
        assert_eq!(world.provider.remaining(), 1, "the model was asked again");
        front.close().await;
    });
    assert_eq!(world.read(".github/workflows/ci.yml"), None);
    assert_eq!(world.read("after.txt"), None);
}

/// An extension whose script did not stop when its time was up, and whose
/// thread was left running, is not started again in this process: its next
/// call fails at once, saying why and how it is started again (the review
/// of step 28: each call made a new thread that would not stop either).
#[test]
fn an_extension_whose_thread_would_not_stop_is_not_started_again() {
    use std::time::Duration;

    let world = World::new(
        "ext_abandoned",
        [
            call_tool("c1", "spin", json!({})),
            call_tool("c2", "spin", json!({})),
            says("Done."),
        ],
    );
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/*\"\naction = \"allow\"\n",
    );
    let extension = approve(
        &world.dir.join("ext"),
        "\n[limits]\ntime = 5",
        "cineris.tool({ name: 'spin', run: () => Array.prototype.reverse.call({ length: 2 ** 53 - 1 }) });",
    );
    let core = world.core_with(|options| options.extensions = vec![extension]);
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("spin twice").await;
        // The call's time, then the grace its thread is given.
        world.timer.sleeping_for(Duration::from_secs(5)).await;
        world.timer.advance(Duration::from_secs(5));
        world.timer.sleeping_for(cineris_ext::engine::GRACE).await;
        let made = cineris_ext::engine::runtimes_made();
        world.timer.advance(cineris_ext::engine::GRACE);
        front.turns(1).await;
        let results = front.results();
        assert!(results[0].2.contains("ran out of its time"), "{results:?}");
        assert!(
            results[1].2.contains("is disabled") && results[1].2.contains("restarts"),
            "{results:?}"
        );
        assert_eq!(cineris_ext::engine::runtimes_made(), made);
        front.close().await;
    });
}
