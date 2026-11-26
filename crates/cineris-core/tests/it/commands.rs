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
        front.close().await;
    });
}
