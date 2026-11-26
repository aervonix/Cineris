//! Custom commands (step 28): the `/` menu lists them beside the built-ins,
//! a project's labelled and a built-in's name kept by the built-in; one
//! typed goes as the command, with a prompt's delivery, and comes back when
//! it is refused; a held one waits for the list; and a prompt a command
//! made says which, and whose.

use cineris_protocol::error::code;
use cineris_protocol::item::{CommandSource, Delivery as ItemDelivery, PromptCommand};
use cineris_protocol::methods::{
    CommandCall, CommandInfo, CommandNotUsed, CommandsListResult, TurnStartParams,
};

use super::*;

fn command(name: &str, source: CommandSource, extension: Option<&str>, about: &str) -> CommandInfo {
    CommandInfo {
        name: name.to_owned(),
        source,
        extension: extension.map(str::to_owned),
        description: about.to_owned(),
        arguments: Vec::new(),
        path: None,
    }
}

/// The user's `review`, the project's `deploy`, a project extension's
/// `tracker:sync` with no description, and a project's `help` and the
/// user's `Help`, which the built-in keeps.
fn listed() -> OpResult {
    OpResult::CommandsList(CommandsListResult {
        commands: vec![
            command("deploy", CommandSource::Project, None, "Deploy to staging"),
            command("help", CommandSource::Project, None, "Not the built-in"),
            command(
                "review",
                CommandSource::User,
                None,
                "Review for one concern",
            ),
            command("tracker:sync", CommandSource::Project, Some("tracker"), ""),
            command("Help", CommandSource::User, None, "Not the built-in either"),
        ],
        not_used: Vec::new(),
    })
}

/// What was sent since the last look, taken: the `commands/list` among it
/// (at most one, for the session), and the rest.
fn list_asked(link: &Scripted) -> (Option<Ticket>, Vec<(Ticket, Op)>) {
    let (asks, rest): (Vec<_>, Vec<_>) = link
        .take_sent()
        .into_iter()
        .partition(|(_, op)| matches!(op, Op::CommandsList(_)));
    assert!(asks.len() <= 1, "one ask at a time: {asks:#?}");
    let ask = asks.into_iter().next().map(|(ticket, op)| {
        assert!(matches!(op, Op::CommandsList(params) if params.session_id.as_str() == SESSION));
        ticket
    });
    (ask, rest)
}

/// The one `turn/start` sent since the last look, with its ticket.
fn started(link: &Scripted) -> (Ticket, TurnStartParams) {
    let mut starts: Vec<_> = link
        .take_sent()
        .into_iter()
        .filter_map(|(ticket, op)| match op {
            Op::TurnStart(params) => Some((ticket, params)),
            _ => None,
        })
        .collect();
    assert_eq!(starts.len(), 1, "{starts:#?}");
    starts.remove(0)
}

fn call(name: &str, args: &str) -> Option<CommandCall> {
    Some(CommandCall {
        name: name.to_owned(),
        args: args.to_owned(),
    })
}

/// The menu: asked for at the open and as it opens, the last list kept
/// through a failed one, the project's noted, `help` the built-in's. Typed:
/// `/review x` goes as the command; refused, it comes back with the core's
/// sentence; sent again under a turn, it steers it. `/help` lists them too,
/// and a `/` word that is none of them is a prompt, as before.
#[test]
fn custom_commands_join_the_menu_and_go_as_commands() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let (asked, _) = list_asked(&link);
    link.give(Inbound::Done(
        asked.expect("asked at the open"),
        Ok(listed()),
    ));
    drain(&mut app, now);

    type_text(&mut app, "/", now);
    let _ = app.frame(now);
    let (again, _) = list_asked(&link);
    let again = again.expect("asked again as the menu opens");
    let _ = app.frame(now);
    assert_eq!(list_asked(&link).0, None, "not again while it stays open");
    // A list that fails leaves the last one, and is not said.
    link.give(Inbound::Done(
        again,
        Err(ErrorData::new(ErrorKind::Internal, "x", "No list here.")),
    ));
    drain(&mut app, now);
    let menu = app.input().menu().cloned().expect("the command menu");
    let detail = |name: &str| -> Vec<String> {
        menu.rows
            .iter()
            .filter(|row| row.label == name)
            .map(|row| row.detail.clone())
            .collect()
    };
    assert_eq!(detail("review"), ["Review for one concern"]);
    assert_eq!(detail("deploy"), ["(project's) Deploy to staging"]);
    assert_eq!(detail("tracker:sync"), ["(project's)"]);
    assert_eq!(
        detail("help"),
        ["The commands and the keys"],
        "the built-in's"
    );
    assert!(detail("Help").is_empty(), "the built-in's, in any case");

    type_text(&mut app, "review x", now);
    key(&mut app, "enter", now);
    let (ticket, sent) = started(&link);
    assert!(sent.input.is_empty(), "{sent:#?}");
    assert_eq!(sent.command, call("review", "x"));
    assert_eq!(sent.delivery, None);
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(
            ErrorKind::Usage,
            code::COMMAND_ARGUMENTS,
            "/review takes one argument at most.",
        )),
    ));
    drain(&mut app, now);
    assert_eq!(app.input().editor().text(), "/review x");
    assert!(!app.running(), "the turn it began did not begin");
    let all = notices(&mut app, now);
    assert!(all.contains("/review takes one argument at most."), "{all}");
    assert!(!all.contains("No list here."), "{all}");

    link.give(event(Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    key(&mut app, "enter", now);
    let (_, sent) = started(&link);
    assert_eq!(sent.command, call("review", "x"));
    assert_eq!(sent.delivery, Some(ItemDelivery::Steer));

    run_command(&mut app, "/Help", now);
    assert!(turn_start(&link.take_sent()).is_empty(), "the built-in ran");
    let all = notices(&mut app, now);
    assert!(
        all.contains("/deploy       (project's) Deploy to staging"),
        "{all}"
    );
    assert!(!all.contains("Not the built-in"), "{all}");

    type_text(&mut app, "/etc/hosts is wrong", now);
    key(&mut app, "enter", now);
    let (_, sent) = started(&link);
    assert_eq!(sent.command, None);
    assert_eq!(sent.input, vec![Part::text("/etc/hosts is wrong")]);
}

/// The command line's `/review security` waits for the session's list,
/// then goes as the command; its prompt names the command and whose it is,
/// what the core recorded escaped. An interrupt while one waits gives it
/// back, unsent.
#[test]
fn a_held_command_waits_for_the_list_and_its_prompt_says_whose() {
    let now = Instant::now();
    let (mut app, link) = app_with(now, Some("/review security"));
    opened(&mut app, &link, now);
    let (asked, rest) = list_asked(&link);
    assert!(turn_start(&rest).is_empty(), "it waits: {rest:#?}");
    assert!(app.running());
    link.give(Inbound::Done(
        asked.expect("asked at the open"),
        Ok(listed()),
    ));
    drain(&mut app, now);
    let (_, sent) = started(&link);
    assert!(sent.input.is_empty());
    assert_eq!(sent.command, call("review", "security"));
    assert_eq!(sent.delivery, None);

    let message = |id: &str, text: &str, command: PromptCommand| {
        event(Event::ItemCompleted(ItemCompleted {
            item: Item::UserMessage(UserMessageItem {
                id: ItemId::from(id),
                parts: vec![Part::text(text)],
                delivery: None,
                command: Some(command),
                created_at: None,
            }),
        }))
    };
    link.give(message(
        "i_1",
        "Review the change for security.",
        PromptCommand {
            name: "review".to_owned(),
            source: CommandSource::User,
            extension: None,
            args: "security".to_owned(),
        },
    ));
    link.give(message(
        "i_2",
        "Sync the tracker.",
        PromptCommand {
            name: "tracker:sync".to_owned(),
            source: CommandSource::Project,
            extension: Some("tracker".to_owned()),
            args: "now\u{1b}[2J".to_owned(),
        },
    ));
    drain(&mut app, now);
    let mut all = Vec::new();
    let live = screen(&mut app, now, &mut all);
    all.extend(live);
    let at = |what: &str| {
        all.iter()
            .position(|line| line.contains(what))
            .unwrap_or_else(|| panic!("{what}: {all:#?}"))
    };
    assert_eq!(
        at("/review security · your command") + 1,
        at("Review the change for security.")
    );
    assert!(at("· the project's extension tracker's command") < at("Sync the tracker."));
    assert!(!all.iter().any(|line| line.contains('\u{1b}')), "{all:#?}");

    let (mut app, link) = app_with(now, Some("/review again"));
    opened(&mut app, &link, now);
    let (asked, _) = list_asked(&link);
    key(&mut app, "esc", now);
    assert_eq!(app.input().editor().text(), "/review again");
    assert!(!app.running());
    link.give(Inbound::Done(asked.expect("asked"), Ok(listed())));
    drain(&mut app, now);
    assert!(link.take_sent().is_empty(), "nothing is sent of it");
}

/// A command's prompt the core hands back comes back as it was typed:
/// refused by a hook (`prompt_returned`), or undelivered at an interrupt,
/// beside a plain one and in order. The interrupt key while a command's
/// start is on its way (an extension's command still running) interrupts,
/// once; the start's refusal (`command_failed`, `interrupted`) gives `/name
/// args` back with nothing said, and the spinner stops. A command that
/// failed of itself is said.
#[test]
fn a_commands_prompt_comes_back_as_it_was_typed() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let (asked, _) = list_asked(&link);
    link.give(Inbound::Done(
        asked.expect("asked at the open"),
        Ok(listed()),
    ));
    drain(&mut app, now);
    let review = PromptCommand {
        name: "review".to_owned(),
        source: CommandSource::User,
        extension: None,
        args: "security".to_owned(),
    };
    link.give(event(Event::Notice(
        cineris_protocol::event::Notice::PromptReturned(
            cineris_protocol::event::PromptReturnedNotice {
                message: "A hook refused the prompt.".to_owned(),
                parts: vec![Part::text("Review the change for security.")],
                delivery: None,
                hooks: Vec::new(),
                command: Some(review.clone()),
                ticket: None,
            },
        ),
    )));
    drain(&mut app, now);
    assert_eq!(app.input().editor().text(), "/review security");

    key(&mut app, "enter", now);
    let (ticket, sent) = started(&link);
    assert_eq!(sent.command, call("review", "security"));
    assert!(app.running());
    key(&mut app, "esc", now);
    let interrupts = link
        .take_sent()
        .into_iter()
        .filter(|(_, op)| matches!(op, Op::TurnInterrupt(_)))
        .count();
    assert_eq!(interrupts, 1, "one key, one interrupt");
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(
            ErrorKind::Interrupted,
            code::COMMAND_FAILED,
            "`/review` was interrupted, so nothing was sent and no turn started.",
        )),
    ));
    drain(&mut app, now);
    assert_eq!(app.input().editor().text(), "/review security");
    assert!(!app.running(), "the spinner stops");
    let all = notices(&mut app, now);
    assert!(!all.contains("was interrupted"), "nothing said: {all}");
    key(&mut app, "enter", now);
    let (ticket, _) = started(&link);
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(
            ErrorKind::Tool,
            code::COMMAND_FAILED,
            "`/review` failed: its script exited 1.",
        )),
    ));
    drain(&mut app, now);
    assert_eq!(app.input().editor().text(), "/review security");
    let all = notices(&mut app, now);
    assert!(
        all.contains("`/review` failed: its script exited 1."),
        "{all}"
    );

    key(&mut app, "enter", now);
    let _ = started(&link);
    link.give(event(Event::TurnStarted(TurnStarted {})));
    drain(&mut app, now);
    key(&mut app, "esc", now);
    let (ticket, _) = link.take_sent().pop().expect("the interrupt");
    let deploy = PromptCommand {
        name: "deploy".to_owned(),
        source: CommandSource::Project,
        extension: None,
        args: String::new(),
    };
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::TurnInterrupt(TurnInterruptResult {
            interrupted: true,
            undelivered: vec![
                vec![Part::text("Review the change for security.")],
                vec![Part::text("and the docs")],
                vec![Part::text("Deploy to staging.")],
            ],
            undelivered_commands: vec![Some(review), None, Some(deploy)],
        })),
    ));
    drain(&mut app, now);
    assert_eq!(
        app.input().editor().text(),
        "/review security\nand the docs\n/deploy"
    );
}

/// The commands the list says are not used are said once a session, one
/// line each, whose and why, and `/help` lists them, marked. One typed goes
/// as the command, for the core to refuse with why, never to the model as
/// `/name args`.
#[test]
fn commands_not_used_are_said_once_and_typed_go_as_commands() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let with_not_used = || {
        let OpResult::CommandsList(mut list) = listed() else {
            unreachable!("a list");
        };
        list.not_used = vec![
            CommandNotUsed {
                name: "broken".to_owned(),
                source: CommandSource::Project,
                extension: None,
                path: Some(".cineris/commands/broken.md".to_owned()),
                reason: "Its front matter is not TOML.".to_owned(),
            },
            CommandNotUsed {
                name: String::new(),
                source: CommandSource::User,
                extension: None,
                path: Some("~/.cineris/commands/.md".to_owned()),
                reason: "It has no name.".to_owned(),
            },
        ];
        OpResult::CommandsList(list)
    };
    let words = |all: &str| all.split_whitespace().collect::<Vec<_>>().join(" ");
    let (asked, _) = list_asked(&link);
    link.give(Inbound::Done(
        asked.expect("asked at the open"),
        Ok(with_not_used()),
    ));
    drain(&mut app, now);
    let all = words(&notices(&mut app, now));
    assert!(
        all.contains("/broken (the project's command) is not used: Its front matter is not TOML."),
        "{all}"
    );
    assert!(
        all.contains("~/.cineris/commands/.md (your command) is not used: It has no name."),
        "{all}"
    );
    type_text(&mut app, "/", now);
    let _ = app.frame(now);
    let (again, _) = list_asked(&link);
    link.give(Inbound::Done(
        again.expect("asked again as the menu opens"),
        Ok(with_not_used()),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert!(!all.contains("is not used"), "said once: {all}");
    key(&mut app, "backspace", now);

    run_command(&mut app, "/help", now);
    let all = notices(&mut app, now);
    assert!(
        all.contains("/broken       (project's) not used: Its front matter is not TOML."),
        "{all}"
    );
    assert!(
        all.contains("~/.cineris/commands/.md not used: It has no name."),
        "{all}"
    );
    run_command(&mut app, "/broken now", now);
    let (_, sent) = started(&link);
    assert!(sent.input.is_empty(), "{sent:#?}");
    assert_eq!(sent.command, call("broken", "now"));
}
