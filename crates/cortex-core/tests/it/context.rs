//! Context management through the core: instruction files read from the
//! disk as the agent would read them, a compaction asked for with
//! `session/compact` and undone by a rewind, and a session resumed from a
//! summary.

use cortex_protocol::event::{Event, Notice};
use cortex_protocol::methods::{Op, SessionCompactParams, SessionRef, SessionRewindParams};
use cortex_protocol::{Item, Role};
use serde_json::Value;

use crate::{Front, World, says};

/// What the user was warned of about instruction files, in order.
fn instruction_warnings(front: &Front) -> Vec<String> {
    front
        .events()
        .into_iter()
        .filter_map(|event| match event.event {
            Event::Notice(Notice::Instructions(notice)) => Some(notice.message),
            _ => None,
        })
        .collect()
}

/// Makes a link to a file. On Windows that needs a privilege an ordinary
/// account lacks unless Developer Mode is on: the CI runner has it, and
/// there the link must be made. Elsewhere a test without it says so and
/// stops.
fn link_file(target: &std::path::Path, link: &std::path::Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    match made {
        Ok(()) => true,
        Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
            #[expect(
                clippy::print_stdout,
                reason = "a test that cannot run on this machine says so, where the runner shows it"
            )]
            {
                println!("skipped: cannot make a link to a file here ({error})");
            }
            false
        }
        Err(error) => panic!("the CI runner cannot make a link to a file: {error}"),
    }
}

/// The user's own instruction file in the configuration directory is the
/// user's, and is read as it is: one that is a link, as a dotfile manager
/// (GNU Stow, home-manager, chezmoi's symlink mode) makes it, is followed.
/// The review of step 15 found it left out without a word. On Windows the
/// link is a symbolic link to a file.
#[test]
fn the_users_own_instruction_file_is_read_when_it_is_a_link() {
    let world = World::new("instructions_user_link", [says("Hello.")]);
    let dotfiles = world.home.join("dotfiles");
    std::fs::create_dir_all(&dotfiles).unwrap();
    std::fs::write(
        dotfiles.join("AGENTS.md"),
        "The user's own, through a link.",
    )
    .unwrap();
    let config = world.dirs().config.unwrap();
    std::fs::create_dir_all(&config).unwrap();
    if !link_file(&dotfiles.join("AGENTS.md"), &config.join("AGENTS.md")) {
        return;
    }
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        let warned = instruction_warnings(&front);
        front.close().await;
        let system = world.provider.requests()[0].system.clone();
        assert!(
            system.contains("The user's own, through a link."),
            "the linked file is not in the prompt; the user was told {warned:#?}\n{system}"
        );
        assert!(warned.is_empty(), "{warned:#?}");
    });
}

/// A link of the user's that leads nowhere is not silently taken for no
/// file: the user is told, by the file's name.
#[test]
fn a_users_link_that_leads_nowhere_is_warned_of() {
    let world = World::new("instructions_user_dangling", [says("Hello.")]);
    let config = world.dirs().config.unwrap();
    std::fs::create_dir_all(&config).unwrap();
    if !link_file(
        &world.home.join("gone/AGENTS.md"),
        &config.join("CLAUDE.md"),
    ) {
        return;
    }
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        let warned = instruction_warnings(&front);
        front.close().await;
        assert!(
            warned.iter().any(|message| message.contains("CLAUDE.md")
                && message.contains("a link to a file that is not there")),
            "{warned:#?}"
        );
    });
}

fn kinds(lines: &[Value]) -> Vec<String> {
    lines
        .iter()
        .map(|line| line["kind"].as_str().unwrap_or_default().to_owned())
        .collect()
}

#[test]
fn instruction_files_are_read_as_the_agent_would_read_them() {
    let world = World::new("instructions_disk", [says("Hello.")]);
    world.put("AGENTS.md", "Use tabs.");
    world.put("sub/AGENTS.md", "Nested, not read at the start.");
    let config = world.dirs().config.unwrap();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("AGENTS.md"), "The user's own: be brief.").unwrap();
    #[cfg(unix)]
    {
        // A link to a file outside the workspace is not followed: what it
        // holds would go to the provider with every request.
        let outside = world.dir.join("outside-secret.txt");
        std::fs::write(&outside, "NOT-FOR-THE-PROVIDER").unwrap();
        std::os::unix::fs::symlink(&outside, world.workspace.join("CLAUDE.md")).unwrap();
    }
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    let system = world.provider.requests()[0].system.clone();
    assert!(system.contains("Use tabs."), "{system}");
    assert!(system.contains("The user's own: be brief."));
    assert!(system.find("be brief").unwrap() < system.find("Use tabs.").unwrap());
    assert!(!system.contains("Nested, not read at the start."));
    assert!(!system.contains("NOT-FOR-THE-PROVIDER"));
}

#[cfg(unix)]
#[test]
fn a_claude_file_that_links_to_agents_is_read_once() {
    let world = World::new("instructions_link_inside", [says("Hello.")]);
    world.put("AGENTS.md", "Use tabs.");
    std::os::unix::fs::symlink("AGENTS.md", world.workspace.join("CLAUDE.md")).unwrap();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    let system = world.provider.requests()[0].system.clone();
    assert_eq!(system.matches("Use tabs.").count(), 1, "{system}");
}

#[test]
fn a_compaction_asked_for_is_undone_by_a_rewind_to_before_it() {
    let world = World::new(
        "compact_and_undo",
        [
            says("First answer."),
            says("The summary."),
            says("After the summary."),
            says("After the rewind."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("first").await;
        let first_answer = front
            .items()
            .iter()
            .find_map(|item| match item {
                Item::AssistantMessage(message) => Some(message.id.clone()),
                _ => None,
            })
            .unwrap();
        front
            .op(Op::SessionCompact(SessionCompactParams {
                session_id: front.session.clone(),
                focus: Some("the first answer".to_owned()),
            }))
            .await
            .unwrap();
        front.turns(2).await;
        front.turn("second").await;
        let compacted = world.provider.requests()[2].clone();
        assert_eq!(compacted.messages.len(), 2, "the summary and the prompt");
        let summary = crate::texts(&compacted.messages[0]);
        assert!(summary.contains("The summary.") && summary.contains("first"));

        // Back to before the compaction: the epoch it began is off the
        // branch, and the whole conversation is sent again.
        front
            .op(Op::SessionRewind(SessionRewindParams {
                session_id: front.session.clone(),
                to: first_answer,
                restore_files: false,
            }))
            .await
            .unwrap();
        front.turn("third").await;
        let undone = world.provider.requests()[3].clone();
        let said: Vec<String> = undone.messages.iter().map(crate::texts).collect();
        assert_eq!(said.len(), 3, "{said:#?}");
        assert!(said[0].ends_with("first"));
        assert_eq!(said[1], "First answer.");
        assert_eq!(said[2], "third");
        assert_eq!(undone.messages[1].role, Role::Assistant);

        let lines = front.log(&world);
        let kinds = kinds(&lines);
        assert_eq!(kinds.iter().filter(|kind| *kind == "summary").count(), 1);
        assert_eq!(kinds.iter().filter(|kind| *kind == "epoch").count(), 2);
        let summary = lines.iter().find(|line| line["kind"] == "summary").unwrap();
        assert!(summary["carried"]["plan"].is_object());
        assert_eq!(summary["carried"]["last_user"][0]["text"], "first");
        assert!(
            kinds.iter().any(|kind| kind == "usage"),
            "the summary's request is in the log: {kinds:?}"
        );
        front.close().await;
    });
}

#[test]
fn a_session_resumed_after_a_compaction_begins_from_its_summary() {
    let world = World::new(
        "resume_from_summary",
        [
            says("First answer."),
            says("The summary."),
            says("Resumed."),
        ],
    );
    let core = world.core();
    let session = world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("first").await;
        front
            .op(Op::SessionCompact(SessionCompactParams {
                session_id: front.session.clone(),
                focus: None,
            }))
            .await
            .unwrap();
        front.turns(2).await;
        front.close().await;
        front.session.clone()
    });
    // A second run of the program.
    let core = world.core();
    world.run(async {
        let client = core.connect(cortex_core::ClientInfo::user());
        let opened = core
            .handle(
                client.id,
                Op::SessionOpen(SessionRef {
                    session_id: session.clone(),
                }),
            )
            .await
            .unwrap();
        let cortex_protocol::methods::OpResult::SessionOpen(opened) = opened else {
            panic!("not the result of session/open");
        };
        let front = Front {
            core: core.clone(),
            client,
            session: session.clone(),
            state: opened.session,
            seen: std::cell::RefCell::default(),
            answered: std::cell::RefCell::default(),
            answering: std::cell::RefCell::default(),
        };
        front.turn("again").await;
        front.close().await;
    });
    let resumed = world.provider.requests().pop().unwrap();
    let said: Vec<String> = resumed.messages.iter().map(crate::texts).collect();
    assert_eq!(said.len(), 2, "{said:#?}");
    assert!(said[0].contains("The summary."));
    assert_eq!(said[1], "again");
}

/// A session started below the workspace root (the binaries find the root
/// by walking up since the follow-ups of step 23) says where its commands
/// start, reads the instruction files from the root down to there, and a
/// resumed session keeps both; one started at the root has no such line. A
/// `cwd` outside the workspace is refused.
#[test]
fn a_session_started_below_the_root_says_where_commands_start_and_keeps_it() {
    let world = World::new("start_below", [says("One."), says("Two."), says("Three.")]);
    world.put("AGENTS.md", "Root instructions.");
    world.put("src/deep/AGENTS.md", "Deep instructions.");
    let core = world.core();
    let session = world.run(async {
        let front = Front::start_in(&world, &core, &world.workspace.join("src/deep")).await;
        front.turn("go").await;
        front.close().await;
        front.session.clone()
    });
    let line = "- Commands start in: src/deep (relative to the workspace;";
    let system = world.provider.requests()[0].system.clone();
    assert!(system.contains(line), "{system}");
    assert!(system.contains("Root instructions."), "{system}");
    assert!(system.contains("Deep instructions."), "{system}");
    // The shell says where its commands run.
    let first = world.provider.requests()[0].clone();
    let shell = first
        .tools
        .iter()
        .find(|tool| tool.name == "shell")
        .expect("the shell tool");
    assert!(
        shell
            .description
            .contains("runs in `src/deep` (relative to the workspace root)"),
        "{}",
        shell.description
    );
    // A second run of the program resumes it: the same.
    let core = world.core();
    world.run(async {
        let client = core.connect(cortex_core::ClientInfo::user());
        let opened = core
            .handle(
                client.id,
                Op::SessionOpen(SessionRef {
                    session_id: session.clone(),
                }),
            )
            .await
            .unwrap();
        let cortex_protocol::methods::OpResult::SessionOpen(opened) = opened else {
            panic!("not the result of session/open");
        };
        let front = Front {
            core: core.clone(),
            client,
            session: session.clone(),
            state: opened.session,
            seen: std::cell::RefCell::default(),
            answered: std::cell::RefCell::default(),
            answering: std::cell::RefCell::default(),
        };
        front.turn("again").await;
        front.close().await;
    });
    let resumed = world.provider.requests().pop().unwrap();
    assert!(resumed.system.contains(line), "{}", resumed.system);
    // Started at the root: no such line, and the deeper file is not read.
    world.run(async {
        let front = Front::start(&world, &world.core()).await;
        front.turn("root").await;
        front.close().await;
    });
    let at_root = world.provider.requests().pop().unwrap();
    assert!(
        !at_root.system.contains("Commands start in"),
        "{}",
        at_root.system
    );
    assert!(!at_root.system.contains("Deep instructions."));
    // Outside the workspace: refused.
    let outside = world.run(async {
        let core = world.core();
        let client = core.connect(cortex_core::ClientInfo::user());
        core.handle(
            client.id,
            Op::SessionNew(cortex_protocol::methods::SessionNewParams {
                cwd: Some(world.home.to_str().unwrap().to_owned()),
                workspace: world.workspace.to_str().unwrap().to_owned(),
                settings: cortex_protocol::session::SessionSettings::default(),
                title: None,
                config: serde_json::Map::new(),
                ephemeral: true,
            }),
        )
        .await
    });
    assert_eq!(outside.expect_err("refused").code, "invalid_cwd");
}

/// A rule that lets the agent read a directory outside the workspace does
/// not make a link to it a project's instruction file: the project's are
/// the project's own.
#[cfg(unix)]
#[test]
fn a_link_out_of_the_workspace_is_not_followed_where_a_rule_allows_reading_there() {
    let world = World::new("instructions_link_allowed", [says("Hello.")]);
    let outside = world.dir.join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("notes.md"), "OUTSIDE-NOTES").unwrap();
    std::os::unix::fs::symlink(outside.join("notes.md"), world.workspace.join("AGENTS.md"))
        .unwrap();
    world.user_config(&format!(
        "[[permissions.rules]]\neffect = \"read\"\nmatch = \"{}/**\"\naction = \"allow\"\n",
        outside.display()
    ));
    let core = world.core();
    let warned = world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        let warned = instruction_warnings(&front);
        front.close().await;
        warned
    });
    let system = world.provider.requests()[0].system.clone();
    assert!(!system.contains("OUTSIDE-NOTES"), "{system}");
    // It stays refused, and the user is told which file, and why (since
    // the review of step 15: it was dropped without a word).
    assert!(
        warned
            .iter()
            .any(|message| message.contains("AGENTS.md")
                && message.contains("outside the workspace")),
        "{warned:#?}"
    );
}

/// The same on Windows, with a symbolic link to a file: a project's
/// instruction file that links outside the workspace is refused, and the
/// user is told.
#[cfg(windows)]
#[test]
fn on_windows_a_link_out_of_the_workspace_is_refused_and_warned_of() {
    let world = World::new("instructions_link_out_windows", [says("Hello.")]);
    let outside = world.dir.join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("notes.md"), "OUTSIDE-NOTES").unwrap();
    if !link_file(
        &outside.join("notes.md"),
        &world.workspace.join("AGENTS.md"),
    ) {
        return;
    }
    let core = world.core();
    let warned = world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        let warned = instruction_warnings(&front);
        front.close().await;
        warned
    });
    let system = world.provider.requests()[0].system.clone();
    assert!(!system.contains("OUTSIDE-NOTES"), "{system}");
    assert!(
        warned
            .iter()
            .any(|message| message.contains("AGENTS.md")
                && message.contains("outside the workspace")),
        "{warned:#?}"
    );
}

/// `[compat]` governs readers of other tools' files only
/// (`docs/architecture/08-configuration.md`): with `compat.instructions`
/// off, `AGENTS.md` and `CLAUDE.md` in the workspace are not read, and the
/// names the user configured, and the user's own file in the configuration
/// directory, still are. The review of step 15, its fourth note, found
/// every instruction file turned off with it, without a word.
#[test]
fn with_compat_instructions_off_only_other_tools_names_are_not_read() {
    let world = World::new("instructions_compat_off", [says("Hello.")]);
    world.put("AGENTS.md", "OTHER-TOOLS-NAME");
    world.put("CLAUDE.md", "OTHER-TOOLS-NAME-TOO");
    world.put("CORTEX.md", "The project's, under a name of ours.");
    let config = world.dirs().config.unwrap();
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("AGENTS.md"), "The user's own.").unwrap();
    world.user_config(
        "[compat]\ninstructions = false\n\n[context]\ninstruction_names = [\"CORTEX.md\"]\n",
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    let system = world.provider.requests()[0].system.clone();
    assert!(!system.contains("OTHER-TOOLS-NAME"), "{system}");
    assert!(
        system.contains("The project's, under a name of ours."),
        "{system}"
    );
    assert!(system.contains("The user's own."), "{system}");
}

#[test]
fn an_instruction_file_a_rule_denies_is_not_read() {
    let world = World::new("instructions_denied", [says("Hello.")]);
    world.put("AGENTS.md", "DENIED-TEXT");
    world.put("CLAUDE.md", "Allowed text.");
    world.user_config(
        "[[permissions.rules]]\neffect = \"read\"\nmatch = \"AGENTS.md\"\naction = \"deny\"\n",
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    let system = world.provider.requests()[0].system.clone();
    assert!(!system.contains("DENIED-TEXT"), "{system}");
    assert!(system.contains("Allowed text."));
}
