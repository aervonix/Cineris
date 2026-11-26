//! Custom commands in a session (step 28): `commands/list`, and a prompt
//! that names a command, expanded in the lane before `prompt.submit`
//! (`docs/architecture/12-extensions.md`, "Tier 1: custom commands").
//! One world and one session kept in memory each.

use cineris_protocol::error::code;
use cineris_protocol::item::{CommandSource, PromptCommand};
use cineris_protocol::methods::{
    CommandCall, CommandsListParams, CommandsListResult, Op, OpResult, TurnStartParams,
};
use cineris_protocol::{Item, Part};

use crate::extensions_found::{cache, pin, start, users, write_extension};
use crate::{Front, World, says};

/// Sends `/name args` as `turn/start` takes a command.
async fn send(front: &Front, name: &str, args: &str) -> Result<(), String> {
    front
        .op(Op::TurnStart(TurnStartParams {
            session_id: front.session.clone(),
            input: Vec::new(),
            delivery: None,
            command: Some(CommandCall {
                name: name.to_owned(),
                args: args.to_owned(),
            }),
        }))
        .await
        .map(|_| ())
        .map_err(|error| format!("{}: {}", error.code, error.message))
}

/// The session's `commands/list`.
async fn listed(front: &Front) -> CommandsListResult {
    match front
        .op(Op::CommandsList(CommandsListParams {
            session_id: front.session.clone(),
        }))
        .await
        .unwrap()
    {
        OpResult::CommandsList(result) => result,
        other => panic!("{other:?}"),
    }
}

/// The last prompt recorded: its text and the command it came from.
fn last_prompt(front: &Front) -> (String, Option<PromptCommand>) {
    front
        .items()
        .into_iter()
        .rev()
        .find_map(|item| match item {
            Item::UserMessage(user) => Some((
                user.parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .collect::<String>(),
                user.command,
            )),
            _ => None,
        })
        .expect("a prompt was recorded")
}

/// Commands from files, from an untrusted project: listed with whose each
/// is, the user's taking a name over the project's; expanded and recorded
/// with where they came from; and refused with a reason when unknown, given
/// what they do not take, sent with a prompt beside them, or naming a
/// settings profile, which is judged and never applied (it would widen the
/// level; it is the untrusted project's own; it would widen nothing, and is
/// still not applied in this build).
#[test]
fn file_commands_are_listed_expanded_and_refused_with_reasons() {
    let world = World::new("commands_files", [says("Done.")]);
    world.user_config(
        "[profiles.loose]\npermissions.mode = \"autonomous\"\n\n[profiles.narrow]\n\
         permissions.mode = \"read-only\"\n",
    );
    world.project_config("[profiles.repos]\nverify.mode = \"off\"\n");
    let user = world.dirs().config.unwrap().join("commands");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::write(
        user.join("review.md"),
        "---\ndescription: Review the staged changes\narguments: [focus]\n---\n\
         Review, focusing on {{focus | \"correctness\"}}. Asked: {{args}}\n",
    )
    .unwrap();
    world.put(
        ".cineris/commands/review.md",
        "The project's review, sent as yours.",
    );
    world.put(".cineris/commands/fix.md", "Fix the build.");
    for (name, profile) in [("loose", "loose"), ("own", "repos"), ("narrow", "narrow")] {
        world.put(
            &format!(".cineris/commands/{name}.md"),
            format!("---\nprofile: {profile}\n---\nDo it."),
        );
    }
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let list = listed(&front).await;
        let names: Vec<(&str, CommandSource)> = list
            .commands
            .iter()
            .map(|command| (command.name.as_str(), command.source.clone()))
            .collect();
        assert_eq!(
            names,
            [
                ("fix", CommandSource::Project),
                ("review", CommandSource::User)
            ]
        );
        assert_eq!(list.commands[1].description, "Review the staged changes");
        let not_used = |name: &str| {
            list.not_used
                .iter()
                .find(|left| left.name == name && left.source == CommandSource::Project)
                .map(|left| left.reason.clone())
                .unwrap_or_default()
        };
        assert!(
            not_used("review").contains("a command of this name (`review`)"),
            "{list:?}"
        );
        assert!(
            not_used("loose").contains("sets `permissions.mode`"),
            "{list:?}"
        );
        assert!(not_used("own").contains("this project's, and the project is not trusted"));
        assert!(
            not_used("narrow").contains("not applied in this build"),
            "{list:?}"
        );

        let before = front.ends().len();
        send(&front, "review", "security now").await.unwrap();
        front.turns(before + 1).await;
        let (text, command) = last_prompt(&front);
        assert_eq!(text, "Review, focusing on security. Asked: security now\n");
        assert_eq!(
            command,
            Some(PromptCommand {
                name: "review".to_owned(),
                source: CommandSource::User,
                extension: None,
                args: "security now".to_owned(),
            })
        );

        for (name, args, why) in [
            ("nothing", "", code::UNKNOWN_COMMAND),
            ("fix", "now", code::COMMAND_ARGUMENTS),
            ("loose", "", code::COMMAND_REFUSED),
            ("narrow", "", code::COMMAND_REFUSED),
        ] {
            let error = send(&front, name, args).await.unwrap_err();
            assert!(error.starts_with(why), "/{name}: {error}");
        }
        let error = front
            .op(Op::TurnStart(TurnStartParams {
                session_id: front.session.clone(),
                input: vec![Part::text("and this")],
                delivery: None,
                command: Some(CommandCall {
                    name: "fix".to_owned(),
                    args: String::new(),
                }),
            }))
            .await
            .unwrap_err();
        assert_eq!(error.code, code::INVALID_PROMPT);
        assert_eq!(
            front.ends().len(),
            before + 1,
            "a refused command started a turn"
        );
        front.close().await;
    });
}

/// An extension's command, from its script: listed under its extension's
/// name, its text sent under a line naming the extension, its record
/// naming both; one that gives nothing starts no turn. And every command's
/// text is expanded before `prompt.submit`: the extension's own hook sees
/// a file command's body, and refuses it.
#[test]
fn an_extensions_command_runs_and_prompt_submit_sees_the_expanded_text() {
    let world = World::new("commands_scripts", [says("Done.")]);
    let folder = users(&world);
    write_extension(&folder, "hooks = [\"prompt.submit\"]", "unused");
    std::fs::write(
        folder.join("main.js"),
        "cineris.command({ name: 'deploy', description: 'Deploy a preview', arguments: ['env'],\n\
           run(input) {\n\
             if (input.arguments.env === 'none') return null;\n\
             return 'deploy to ' + input.arguments.env + ' [' + input.args + ']';\n\
           } });\n\
         cineris.on('prompt.submit', {}, (e) => {\n\
           if (e.prompt.includes('SECRET')) return { decision: 'deny', reason: 'saw ' + e.prompt };\n\
           return undefined;\n\
         });\n",
    )
    .unwrap();
    pin(&world, &folder);
    cache(&world, &folder);
    world.put(".cineris/commands/ship.md", "Ship the SECRET build.");
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let list = listed(&front).await;
        let deploy = list
            .commands
            .iter()
            .find(|command| command.name == "probe:deploy")
            .unwrap_or_else(|| panic!("{list:?}"));
        assert_eq!(
            (deploy.source.clone(), deploy.extension.as_deref()),
            (CommandSource::User, Some("probe"))
        );

        let before = front.ends().len();
        send(&front, "probe:deploy", "staging").await.unwrap();
        front.turns(before + 1).await;
        let (text, command) = last_prompt(&front);
        assert!(
            text.starts_with(
                "The command `/probe:deploy` of the extension `probe`, written by its author"
            ) && text.ends_with("deploy to staging [staging]"),
            "{text}"
        );
        assert_eq!(
            command.map(|command| (command.name, command.extension)),
            Some(("probe:deploy".to_owned(), Some("probe".to_owned())))
        );

        let error = send(&front, "probe:deploy", "none").await.unwrap_err();
        assert!(
            error.starts_with(code::COMMAND_FAILED) && error.contains("gave no text to send"),
            "{error}"
        );

        // The hook sees the body, not `/ship`: the gate refuses the text.
        send(&front, "ship", "").await.unwrap();
        front.turns(before + 2).await;
        let ended = front.ends().pop().unwrap();
        assert_eq!(
            ended.error.as_ref().map(|error| error.code.as_str()),
            Some(code::PROMPT_DENIED)
        );
        assert!(
            front
                .events()
                .iter()
                .any(|event| format!("{event:?}").contains("saw Ship the SECRET build.")),
            "the hook did not see the expanded text"
        );
        assert_eq!(last_prompt(&front).0, text, "a refused prompt was recorded");
        // Given back with its command, to its sender: `/ship` goes back to
        // the editor, not the text it sent.
        let returned = front
            .events()
            .into_iter()
            .find_map(|event| match event.event {
                cineris_protocol::event::Event::Notice(
                    cineris_protocol::event::Notice::PromptReturned(returned),
                ) => Some(returned),
                _ => None,
            })
            .expect("the prompt was given back");
        assert_eq!(
            returned
                .command
                .map(|command| (command.name, command.source)),
            Some(("ship".to_owned(), CommandSource::Project))
        );
        front.close().await;
    });
}

/// [`send`], answering "allow once" to the one question the command puts
/// while it runs, before its `turn/start` has answered.
async fn sent_answering(front: &Front, name: &str) -> Result<(), String> {
    let mut sent = std::pin::pin!(send(front, name, ""));
    let mut approval = std::pin::pin!(front.approval());
    let mut answered = false;
    std::future::poll_fn(|context| {
        if let std::task::Poll::Ready(result) = sent.as_mut().poll(context) {
            return std::task::Poll::Ready(result);
        }
        if !answered && let std::task::Poll::Ready((id, _)) = approval.as_mut().poll(context) {
            answered = true;
            front
                .answer(&id, cineris_protocol::methods::ApprovalChoice::AllowOnce)
                .unwrap();
            context.waker().wake_by_ref();
        }
        std::task::Poll::Pending
    })
    .await
}

/// An extension that keeps a command running until it is stopped, and one
/// that reads a file that looks like a secret, which is asked about.
const RUNS: &str = "cineris.command({ name: 'spin', description: 'Never ends', run() { for (;;) {} } });\n\
     cineris.command({ name: 'peek', description: 'Reads the secrets', async run() {\n\
       await cineris.fs.read('.env');\n\
       return 'peeked';\n\
     } });\n";

/// A running command is stopped by an interrupt, which does not wait behind
/// the `turn/start` it runs in: the early look a lane makes as an operation
/// is sent (`Core::early`), and the interrupt itself, as headless mode sends
/// it beside the start. Each ends `command_failed`, interrupted, with nothing
/// recorded and no turn started. And each run's questions are its own: a
/// person's "allow once" for one run is not another run's, nor a real
/// turn's, and no run's question is put as a turn's (`t_`).
#[test]
fn an_interrupt_stops_a_running_command_and_its_questions_are_its_own() {
    let world = World::new(
        "commands_interrupt",
        [
            says("One."),
            says("Two."),
            crate::calls(&[("r1", "read", serde_json::json!({"path": ".env"}))]),
            says("Done."),
        ],
    );
    let folder = users(&world);
    write_extension(&folder, "read = [\"**\"]", "unused");
    std::fs::write(folder.join("main.js"), RUNS).unwrap();
    pin(&world, &folder);
    cache(&world, &folder);
    world.put(".env", "KEY=1\n");
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        let interrupt = Op::TurnInterrupt(cineris_protocol::methods::TurnInterruptParams::new(
            front.session.clone(),
        ));
        for early in [true, false] {
            let runs = cineris_testkit::Hold::new(cineris_core::A_SCRIPT_COMMAND_RUNS);
            let mut sent = std::pin::pin!(send(&front, "probe:spin", ""));
            runs.drive_until_reached(sent.as_mut()).await;
            runs.release();
            drop(runs);
            if early {
                front.core.early(front.client.id, &interrupt);
            } else {
                front.op(interrupt.clone()).await.unwrap();
            }
            let error = sent.await.unwrap_err();
            assert!(
                error.starts_with(code::COMMAND_FAILED) && error.contains("was interrupted"),
                "{error}"
            );
        }
        assert!(
            front.ends().is_empty(),
            "an interrupted command started a turn"
        );
        assert!(
            !front
                .items()
                .iter()
                .any(|item| matches!(item, Item::UserMessage(_))),
            "an interrupted command recorded a prompt"
        );

        for done in 1..=2 {
            sent_answering(&front, "probe:peek").await.unwrap();
            front.turns(done).await;
        }
        front.answering(crate::Answering::allow_all());
        front.turn("read the secrets").await;
        let asked: Vec<String> = front
            .approvals()
            .iter()
            .map(|request| request.turn_id.as_str().to_owned())
            .collect();
        assert_eq!(asked.len(), 3, "a run took another's answer: {asked:?}");
        assert!(
            asked[0].starts_with("cmd_") && asked[1].starts_with("cmd_") && asked[0] != asked[1],
            "{asked:?}"
        );
        assert!(asked[2].starts_with("t_"), "{asked:?}");
        front.close().await;
    });
}
