//! Custom commands in a session (step 28): `commands/list`, and a prompt
//! that names a command, expanded in the lane before `prompt.submit`
//! (`docs/architecture/12-extensions.md`, "Tier 1: custom commands").
//! One world and one session each, kept in memory unless a test reads its
//! log.

use std::time::Duration;

use cineris_protocol::error::code;
use cineris_protocol::item::{CommandSource, PromptCommand};
use cineris_protocol::methods::{
    CommandCall, CommandsListParams, CommandsListResult, Op, OpResult, TrustResourceKind,
    TurnStartParams,
};
use cineris_protocol::{Item, Part};

use crate::extensions_found::{cache, clock, pin, start, state, users, write_extension};
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
/// with where they came from, text sent beside one after its expanded text;
/// and refused with a reason when unknown, given what they do not take,
/// sent with an image beside them, or naming a settings profile, which is
/// judged and never applied (it would widen the level; it is the untrusted
/// project's own; it would widen nothing, and is still not applied in this
/// build).
#[test]
fn file_commands_are_listed_expanded_and_refused_with_reasons() {
    let world = World::new("commands_files", [says("Done."), says("Done.")]);
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
        let with = |input: Vec<Part>| {
            Op::TurnStart(TurnStartParams {
                session_id: front.session.clone(),
                input,
                delivery: None,
                command: Some(CommandCall {
                    name: "fix".to_owned(),
                    args: String::new(),
                }),
            })
        };
        let image = Part::Image(cineris_protocol::message::ImagePart {
            media_type: "image/png".to_owned(),
            data: Some("iVBORw0KGgo=".to_owned()),
            artifact: None,
        });
        let error = front
            .op(with(vec![Part::text("and this"), image]))
            .await
            .unwrap_err();
        assert_eq!(error.code, code::INVALID_PROMPT);
        assert_eq!(
            front.ends().len(),
            before + 1,
            "a refused command started a turn"
        );
        // Text sent with a command follows its expanded text, after a blank
        // line, in the one prompt recorded: headless mode's standard input.
        front.op(with(vec![Part::text("and this")])).await.unwrap();
        front.turns(before + 2).await;
        let (text, command) = last_prompt(&front);
        assert_eq!(text, "Fix the build.\n\nand this");
        assert_eq!(
            command.map(|command| (command.name, command.source)),
            Some(("fix".to_owned(), CommandSource::Project))
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
    // Not in the registration cache: registered as the session opens,
    // held there while the list is asked for, which waits for it.
    world.put(".cineris/commands/ship.md", "Ship the SECRET build.");
    let core = world.core();
    world.run(async {
        let registering =
            cineris_testkit::Hold::new(cineris_core::ext::BEFORE_AN_EXTENSION_IS_REGISTERED);
        let front = start(&world, &core).await;
        let mut list = std::pin::pin!(listed(&front));
        registering.drive_until_reached(list.as_mut()).await;
        registering.release();
        drop(registering);
        let list = list.await;
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

/// A command's programs have no write root, whatever its manifest's `write`
/// says (the maintainers, after the early review of the commands): one that
/// writes the workspace fails to, at `supervised` and at `read-only`. And a
/// second interrupt (`now`) stops a command whose program runs, at once: its
/// program ignores the polite signals, so only the kill ends it, and a
/// grace, on the core's timer, which nothing moves here, would never pass
/// (the review of step 28's commands, L5).
#[cfg(target_os = "linux")]
#[test]
fn sys_a_commands_programs_write_nothing_and_an_interrupt_stops_them() {
    if !crate::sandbox::enforcing_here() {
        return;
    }
    let world = World::new("commands_programs", [says("One."), says("Two.")]);
    let probe = crate::ext_exec::extension_running(
        &world,
        "probe",
        "exec = [\"sh\"]\nwrite = [\"**\"]",
        "cineris.command({ name: 'write', description: 'Writes', async run() {\n\
           const ran = await cineris.exec('sh', ['-c', 'echo x > out.txt']);\n\
           return 'wrote with ' + ran.code;\n\
         } });\n\
         cineris.command({ name: 'wait', description: 'Waits', async run() {\n\
           await cineris.exec('sh', ['-c', \"trap '' TERM INT HUP; sleep 30\"], { timeout: 29000 });\n\
           return 'waited';\n\
         } });\n",
    );
    // The runner's grace, of a length nothing else waits: the script's own
    // runtime gives its thread 2 s to stop, as the runner's default does.
    let grace = Duration::from_secs(7);
    let core = world.core_with(|options| {
        options.launcher = Some(crate::sandbox::launcher());
        options.extensions = vec![probe];
        options.runner = options.runner.take().map(|runner| runner.with_grace(grace));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        let mut turns = 0;
        for level in [
            cineris_protocol::lease::Level::Supervised,
            cineris_protocol::lease::Level::ReadOnly,
        ] {
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
            // Refused, or run and failing to write: either way nothing is
            // written.
            if sent_answering(&front, "probe:write").await.is_ok() {
                turns += 1;
                front.turns(turns).await;
                let (text, _) = last_prompt(&front);
                assert!(!text.ends_with("wrote with 0"), "{level:?}: {text}");
            }
            assert_eq!(
                world.read("out.txt"),
                None,
                "a command's program wrote at {level:?}"
            );
        }
        let interrupt = Op::TurnInterrupt(cineris_protocol::methods::TurnInterruptParams {
            session_id: front.session.clone(),
            now: true,
        });
        front
            .op(Op::SessionSet(
                cineris_protocol::methods::SessionSetParams {
                    session_id: front.session.clone(),
                    settings: cineris_protocol::session::SessionSettings {
                        level: Some(cineris_protocol::lease::Level::Supervised),
                        ..Default::default()
                    },
                },
            ))
            .await
            .unwrap();
        // Asked about, allowed, and run: its program's own 29 s are on the
        // timer once the runner drives it.
        let mut sent = std::pin::pin!(sent_answering(&front, "probe:wait"));
        let mut running = std::pin::pin!(world.timer.sleeping_for(Duration::from_secs(29)));
        std::future::poll_fn(|context| {
            assert!(
                sent.as_mut().poll(context).is_pending(),
                "it ended before its program ran"
            );
            running.as_mut().poll(context)
        })
        .await;
        front.core.early(front.client.id, &interrupt);
        let mut grace = std::pin::pin!(world.timer.sleeping_for(grace));
        let error = std::future::poll_fn(|context| {
            if let std::task::Poll::Ready(ended) = sent.as_mut().poll(context) {
                return std::task::Poll::Ready(ended);
            }
            assert!(
                grace.as_mut().poll(context).is_pending(),
                "the program was stopped politely, its grace running: it was not killed at once"
            );
            std::task::Poll::Pending
        })
        .await
        .unwrap_err();
        assert!(error.contains("was interrupted"), "{error}");
        front.close().await;
    });
}

/// An extension that keeps a command running until it is stopped, and one
/// that reads a file that looks like a secret, which is asked about.
const RUNS: &str = "cineris.command({ name: 'spin', description: 'Never ends', run() { for (;;) {} } });\n\
     cineris.command({ name: 'ask', description: 'Asks', async run() {\n\
       await cineris.ask('Go on?', ['yes', 'no']);\n\
       return 'asked';\n\
     } });\n\
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
/// turn's, and no run's question is put as a turn's (`t_`); each begins with
/// the command and whose it is, the user's own here (a question's text, an
/// approval's summary).
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
        let other = front.join(cineris_core::ClientInfo::user()).await;
        // Stopped where it waits before its script (`A_COMMAND_IS_TAKEN`),
        // and where its script loops: by the early look, by the interrupt
        // itself, and by another client's interrupt.
        for (point, how) in [
            (cineris_core::A_COMMAND_IS_TAKEN, "early"),
            (cineris_core::A_SCRIPT_COMMAND_RUNS, "early"),
            (cineris_core::A_SCRIPT_COMMAND_RUNS, "itself"),
            (cineris_core::A_SCRIPT_COMMAND_RUNS, "another"),
        ] {
            let runs = cineris_testkit::Hold::new(point);
            let mut sent = std::pin::pin!(send(&front, "probe:spin", ""));
            runs.drive_until_reached(sent.as_mut()).await;
            runs.release();
            drop(runs);
            match how {
                "early" => front.core.early(front.client.id, &interrupt),
                "itself" => drop(front.op(interrupt.clone()).await.unwrap()),
                _ => other.core.early(other.client.id, &interrupt),
            }
            let error = sent.await.unwrap_err();
            assert!(
                error.starts_with(code::COMMAND_FAILED) && error.contains("was interrupted"),
                "{point}, {how}: {error}"
            );
        }
        // An open question of its script is withdrawn, and a late answer
        // refused.
        let asking = {
            let (core, id, op) = (
                front.core.clone(),
                front.client.id,
                Op::TurnStart(TurnStartParams {
                    session_id: front.session.clone(),
                    input: Vec::new(),
                    delivery: None,
                    command: Some(CommandCall {
                        name: "probe:ask".to_owned(),
                        args: String::new(),
                    }),
                }),
            );
            tokio::spawn(async move { core.handle(id, op).await })
        };
        let question = || {
            front
                .seen
                .borrow()
                .iter()
                .find_map(|outgoing| match outgoing {
                    cineris_core::Outgoing::Request {
                        id,
                        request: cineris_protocol::methods::ServerRequest::Question(_),
                    } => Some(id.clone()),
                    _ => None,
                })
        };
        front.until(|_| question().is_some()).await;
        let text = front
            .seen
            .borrow()
            .iter()
            .find_map(|outgoing| match outgoing {
                cineris_core::Outgoing::Request {
                    request: cineris_protocol::methods::ServerRequest::Question(asked),
                    ..
                } => Some(asked.question.clone()),
                _ => None,
            })
            .unwrap_or_default();
        assert!(
            text.starts_with("The command `/probe:ask` of your extension `probe` asks: Go on?"),
            "{text}"
        );
        front.core.early(front.client.id, &interrupt);
        let error = asking.await.unwrap().unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.kind.clone()),
            (
                code::COMMAND_FAILED,
                cineris_protocol::ErrorKind::Interrupted
            )
        );
        let late = front.core.respond(
            front.client.id,
            &question().unwrap(),
            cineris_protocol::methods::ServerResponse::Question(
                cineris_protocol::methods::QuestionResponse {
                    answer: Some("yes".to_owned()),
                },
            ),
        );
        assert!(late.is_err(), "a late answer was taken");
        // A command has its own time, 30 s from when it is asked, whatever
        // its manifest gives its tools.
        let mut sent = std::pin::pin!(send(&front, "probe:spin", ""));
        let mut sleeping =
            std::pin::pin!(world.timer.sleeping_for(std::time::Duration::from_secs(30)));
        std::future::poll_fn(|context| {
            assert!(
                sent.as_mut().poll(context).is_pending(),
                "it ended before its time"
            );
            sleeping.as_mut().poll(context)
        })
        .await;
        world.timer.advance(std::time::Duration::from_secs(30));
        let error = sent.await.unwrap_err();
        assert!(error.contains("did not end within its 30 s"), "{error}");
        other.close().await;
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
        let summary = &front.approvals()[0].summary;
        assert!(
            summary.starts_with("The command `/probe:peek` of your extension `probe`: "),
            "{summary}"
        );
        front.close().await;
    });
}

/// The commands of the project's extension `probe`: one reads a file that
/// looks like a secret, which is asked about; one says something; one keeps
/// a value of the session's; one writes a file its manifest lets it write.
/// And a tool that keeps its runtime until it is stopped.
const STOPS: &str = "cineris.command({ name: 'peek', description: 'Reads the secrets', async run() {\n\
       await cineris.fs.read('.env');\n\
       return 'peeked';\n\
     } });\n\
     cineris.command({ name: 'ok', description: 'Says something', run() { return 'go on'; } });\n\
     cineris.command({ name: 'keep', description: 'Keeps a value', async run() {\n\
       await cineris.state.set('kept', 1);\n\
       return null;\n\
     } });\n\
     cineris.command({ name: 'write', description: 'Writes', async run() {\n\
       await cineris.fs.write('out.txt', 'x');\n\
       return 'wrote';\n\
     } });\n\
     cineris.tool({ name: 'spin', description: 'Never ends', input: { type: 'object' },\n\
       run() { for (;;) {} } });\n";

/// Writes the user's extension `gate`, a gate on `prompt.submit`, pinned as
/// it was before it changed: its gate is held as the session opens, and
/// nothing of it is evaluated.
fn a_changed_gate(world: &World) {
    let folder = world.dirs().config.unwrap().join("extensions").join("gate");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"gate\"\nversion = \"1\"\ndescription = \"A gate\"\nmain = \
         \"main.js\"\n[capabilities]\nhooks = [\"prompt.submit\"]\n",
    )
    .unwrap();
    let gate = "cineris.on('prompt.submit', () => undefined);\n";
    std::fs::write(folder.join("main.js"), gate).unwrap();
    cineris_policy::PinStore::new(state(world).pins_file(), clock(world))
        .pin(&cineris_policy::Definition {
            kind: TrustResourceKind::Extension,
            name: "gate".to_owned(),
            hash: cineris_ext::folder::Snapshot::read(&folder).unwrap().hash(),
        })
        .unwrap();
    std::fs::write(folder.join("main.js"), format!("// changed\n{gate}")).unwrap();
}

/// Nothing of an extension's command runs where the prompt it would send
/// would be refused, each refused `command_refused` with why; and a
/// command's own stop is the session's. The project's extension `probe` is
/// handed in, as `ext_state` hands one, to a session nobody can be asked
/// in, whose policy fails on a denial:
/// - (b) a gate held on `prompt.submit` (the user's `gate`, changed since it
///   was approved) refuses it; its pin taken away, nothing is held;
/// - (d) a command of an extension found and not loaded is refused with why,
///   as the session was told it: `gate` changed since it was approved, then,
///   its pin taken away, not approved (the review of step 28's commands);
/// - a command's read of a secret, which nobody can be asked about, is
///   answered with a stop: the command fails, the user is told, and the
///   turn running then is stopped; the call's summary names the project's
///   extension;
/// - (a) that stop, left for the next prompt, refuses the next command, and
///   is taken by it;
/// - (c) the running turn stopped, the command after is refused too; the
///   turn ends denied at its next call, and then a command runs;
/// - a command's session value is recorded as of no turn, and its
///   `cineris.fs.write` is refused, though its manifest names the file;
/// - a command waiting behind its extension's tool (one call at a time a
///   runtime) spends its own 30 s from when it was asked, and fails at
///   their end, never having started, the tool going on;
/// - a command sent through a lane, held before its script as the core shuts
///   down, ends interrupted: the shutdown reaches its run (the review of step
///   28's commands, M2).
#[test]
fn a_command_runs_nothing_where_its_prompt_would_be_refused_and_its_stop_is_the_sessions() {
    let held = cineris_testkit::Gate::new();
    let world = World::new(
        "commands_stops",
        [
            // The first turn: held until the test lets it go, then a call
            // that the turn's stop refuses.
            cineris_testkit::Response::new()
                .wait(&held)
                .tool_call("c1", "list", &serde_json::json!({"path": "."}))
                .end(cineris_protocol::StopReason::ToolCalls),
            says("Done."),
            crate::calls(&[(
                "c2",
                "tool_call",
                serde_json::json!({"tool": "probe/spin", "arguments": {}}),
            )]),
        ],
    );
    world.user_config(
        "[[permissions.rules]]\neffect = \"ext\"\nmatch = \"probe/spin\"\naction = \"allow\"\n",
    );
    world.put(".env", "KEY=1\n");
    a_changed_gate(&world);
    let mut probe = crate::ext_exec::extension_running(
        &world,
        "probe",
        "read = [\"**\"]\nwrite = [\"out.txt\"]\n\n[limits]\ntime = 97",
        STOPS,
    );
    probe.source = cineris_ext::source::Source::Project;
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
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
        .expect("the session starts");
        let refused = |error: &str, words: &str| {
            assert!(
                error.starts_with(&format!("{}: ", code::COMMAND_REFUSED)) && error.contains(words),
                "{words}: {error}"
            );
        };
        // (b)
        let error = send(&front, "probe:ok", "").await.unwrap_err();
        refused(
            &error,
            "`/probe:ok` was not run: the extension `gate` holds a gate on `prompt.submit`",
        );
        // (d)
        let error = send(&front, "gate:go", "").await.unwrap_err();
        refused(&error, "`/gate:go` is not sent: your extension `gate` (`");
        refused(
            &error,
            "has changed since you approved it, so it is not loaded.",
        );
        cineris_policy::PinStore::new(state(&world).pins_file(), clock(&world))
            .unpin(&TrustResourceKind::Extension, "gate")
            .unwrap();

        // A turn runs, held at its response.
        front.prompt("first").await;
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(
                        outgoing,
                        cineris_core::Outgoing::Event(cineris_protocol::SessionEvent {
                            event: cineris_protocol::event::Event::TurnStarted(_),
                            ..
                        })
                    )
                })
            })
            .await;
        // (d) Its pin taken away, seen as the turn started.
        let error = send(&front, "gate:go", "").await.unwrap_err();
        refused(&error, "is not loaded: you have not approved it.");
        let error = send(&front, "probe:peek", "").await.unwrap_err();
        assert!(
            error.starts_with(code::COMMAND_FAILED)
                && error.contains("`/probe:peek` was stopped: "),
            "{error}"
        );
        let denials = core.permission_denials(&front.session);
        assert!(
            denials.len() == 1
                && denials[0]
                    .summary
                    .starts_with("The command `/probe:peek` of the project's extension `probe`: "),
            "{denials:?}"
        );
        // (a)
        let error = send(&front, "probe:ok", "").await.unwrap_err();
        refused(&error, ": The command `/probe:peek` was stopped: ");
        assert!(
            error.contains("The prompt was not sent, so that you can look first."),
            "{error}"
        );
        front.drain();
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.starts_with("The command `/probe:peek` was stopped: ")),
            "{:?}",
            front.notices()
        );
        // (c)
        let error = send(&front, "probe:ok", "").await.unwrap_err();
        refused(
            &error,
            "`/probe:ok` was not run: the running turn was stopped.",
        );
        held.open();
        front.turns(1).await;
        assert_eq!(
            front.ends()[0].outcome,
            cineris_protocol::event::TurnOutcome::Denied,
            "{:?}",
            front.ends()
        );
        send(&front, "probe:ok", "").await.unwrap();
        front.turns(2).await;
        assert!(last_prompt(&front).0.ends_with("\n\ngo on"));

        let error = send(&front, "probe:keep", "").await.unwrap_err();
        assert!(error.contains("gave no text to send"), "{error}");
        let error = send(&front, "probe:write", "").await.unwrap_err();
        assert!(
            error.starts_with(code::COMMAND_FAILED)
                && error.contains("`cineris.fs.write` works only in a tool call"),
            "{error}"
        );

        // Behind the extension's tool, its runtime's one call at a time.
        front.prompt("spin").await;
        world.timer.sleeping_for(Duration::from_secs(97)).await;
        let runs = cineris_testkit::Hold::new(cineris_core::A_SCRIPT_COMMAND_RUNS);
        let mut sent = std::pin::pin!(send(&front, "probe:ok", ""));
        runs.drive_until_reached(sent.as_mut()).await;
        runs.release();
        drop(runs);
        let own = Duration::from_secs(30);
        for _ in 0..64 {
            if world.timer.running().contains(&own) {
                break;
            }
            assert!(
                crate::poll_once(sent.as_mut()).await.is_pending(),
                "it ended before its time"
            );
        }
        assert!(
            world.timer.running().contains(&own),
            "its time did not run from when it was asked: {:?}",
            world.timer.running()
        );
        world.timer.advance(own);
        let error = sent.await.unwrap_err();
        assert!(
            error.contains("`/probe:ok` did not end within its 30 s"),
            "{error}"
        );
        // Withdrawn, never started: the tool goes on until it is stopped.
        assert!(world.timer.running().contains(&Duration::from_secs(97)));
        front.interrupt(false).await;
        front.turns(3).await;
        // A command sent through a lane, held before its script, as the core
        // shuts down: the shutdown reaches its run, which ends interrupted.
        let ok = Op::TurnStart(TurnStartParams {
            session_id: front.session.clone(),
            input: Vec::new(),
            delivery: None,
            command: Some(CommandCall {
                name: "probe:ok".to_owned(),
                args: String::new(),
            }),
        });
        front.core.early(front.client.id, &ok);
        let runs = cineris_testkit::Hold::new(cineris_core::A_SCRIPT_COMMAND_RUNS);
        let mut sent = std::pin::pin!(front.core.handle(front.client.id, ok));
        runs.drive_until_reached(sent.as_mut()).await;
        core.shutdown().await;
        runs.release();
        drop(runs);
        let error = sent
            .await
            .expect_err("a command ran on after the core shut down");
        assert_eq!(
            (error.code.as_str(), error.kind.clone()),
            (
                code::COMMAND_FAILED,
                cineris_protocol::ErrorKind::Interrupted
            ),
            "{}",
            error.message
        );
        let records: Vec<serde_json::Value> = front
            .log(&world)
            .into_iter()
            .filter(|entry| entry["kind"] == "ext")
            .collect();
        assert!(
            records.len() == 1
                && records[0]["data"]["state"]["key"] == "kept"
                && records[0]["turn"].is_null(),
            "{records:?}"
        );
    });
    assert_eq!(world.read("out.txt"), None);
}
