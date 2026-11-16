//! The commands step 24 adds that need no panel: `/reload`, `/profile`,
//! `/verify`, `/context`, and `/config` with where each value came from
//! (`docs/steps/24/answered.md`, "The design, before building", point 7).

use super::*;
use cortex_protocol::event::ConfigChanged;
use cortex_protocol::methods::{
    ConfigGetResult, ConfigLayer, ConfigOrigin, ConfigReloadParams, ConfigReloadResult, SessionRef,
};
use cortex_protocol::usage::{ContextBreakdown, SessionUsage};

/// The operation the last command sent, with its ticket.
fn sent(link: &Scripted, pick: impl Fn(&Op) -> bool) -> (Ticket, Op) {
    link.take_sent()
        .into_iter()
        .find(|(_, op)| pick(op))
        .expect("the operation is sent")
}

fn event(seq: u64, event: Event) -> Inbound {
    Inbound::Event(SessionEvent {
        session_id: SessionId::from(SESSION),
        seq,
        turn_id: None,
        event,
    })
}

/// `/reload` reads the files again, for the core and every session of this
/// client, and says what it did once: from its result, not again from the
/// `config.changed` the same reload sent.
#[test]
fn reload_says_what_the_reload_did_once() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    run_command(&mut app, "/reload", now);
    let (ticket, op) = sent(&link, |op| matches!(op, Op::ConfigReload(_)));
    assert_eq!(op, Op::ConfigReload(ConfigReloadParams::default()));
    let changed = ConfigChanged {
        applied: vec!["retry.attempts".to_owned()],
        pending: vec!["model.main".to_owned()],
        errors: Vec::new(),
        notes: vec!["`model.main` takes effect at this session's next context epoch.".to_owned()],
    };
    link.give(event(40, Event::ConfigChanged(changed.clone())));
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::ConfigReload(ConfigReloadResult {
            applied: changed.applied,
            pending: changed.pending,
            errors: Vec::new(),
            notes: changed.notes,
        })),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert_eq!(
        all.matches("`retry.attempts` in force now").count(),
        1,
        "{all}"
    );
    assert_eq!(all.matches("next context epoch").count(), 1, "{all}");
}

/// What another client's reload, or the core's look before a turn,
/// changed is said as it comes.
#[test]
fn a_change_found_elsewhere_is_said() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    link.give(event(
        40,
        Event::ConfigChanged(ConfigChanged {
            applied: vec!["limits.turn.cost".to_owned()],
            errors: vec![
                "error: ~/.config/cortex-harness/config.toml:3: is not valid TOML".to_owned(),
            ],
            ..ConfigChanged::default()
        }),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert!(all.contains("`limits.turn.cost` in force now"), "{all}");
    assert!(all.contains("config.toml:3"), "{all}");
}

/// `/profile <name>` loads this session with the profile, and `/profile`
/// lists the profiles its files define, the one in force marked.
#[test]
fn profile_loads_one_and_lists_them() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    run_command(&mut app, "/profile careful", now);
    let (ticket, op) = sent(&link, |op| matches!(op, Op::ConfigReload(_)));
    assert_eq!(
        op,
        Op::ConfigReload(ConfigReloadParams {
            session_id: Some(SessionId::from(SESSION)),
            profile: Some("careful".to_owned()),
            changed_only: false,
        })
    );
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::ConfigReload(ConfigReloadResult {
            applied: vec!["retry.attempts".to_owned()],
            ..ConfigReloadResult::default()
        })),
    ));
    drain(&mut app, now);

    run_command(&mut app, "/profile", now);
    let (ticket, op) = sent(&link, |op| matches!(op, Op::ConfigGet(_)));
    let Op::ConfigGet(params) = op else {
        unreachable!()
    };
    assert_eq!(params.keys, ["profile", "profiles"]);
    let mut config = serde_json::Map::new();
    config.insert("profiles.careful.retry.attempts".to_owned(), 1.into());
    config.insert("profiles.daily.limits.day.requests".to_owned(), 5.into());
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::ConfigGet(ConfigGetResult {
            config,
            origins: Default::default(),
        })),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert!(all.contains("careful  (in force)"), "{all}");
    assert!(all.contains("daily"), "{all}");
    assert!(!all.contains("daily  (in force)"), "{all}");
}

/// With no profile defined, `/profile` says how to define one.
#[test]
fn profile_with_none_defined_says_how_to_make_one() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    run_command(&mut app, "/profile", now);
    let (ticket, _) = sent(&link, |op| matches!(op, Op::ConfigGet(_)));
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::ConfigGet(ConfigGetResult {
            config: serde_json::Map::new(),
            origins: Default::default(),
        })),
    ));
    drain(&mut app, now);
    assert!(notices(&mut app, now).contains("[profiles.<name>]"));
}

/// `/verify` asks the core for a turn that only verifies; when no check
/// could run, the core's reason is said.
#[test]
fn verify_asks_for_the_checks_and_says_why_none_could_run() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    run_command(&mut app, "/verify", now);
    let (ticket, op) = sent(&link, |op| matches!(op, Op::VerifyRun(_)));
    assert_eq!(
        op,
        Op::VerifyRun(SessionRef {
            session_id: SessionId::from(SESSION)
        })
    );
    link.give(Inbound::Done(
        ticket,
        Err(ErrorData::new(
            cortex_protocol::ErrorKind::Usage,
            "no_checks",
            "No check could be chosen for this project.",
        )),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert!(all.contains("No check could be chosen"), "{all}");
    assert!(!all.contains("not in this build"), "{all}");
}

/// `/context` shows the last request's parts as estimates, against the
/// request's limit and the window; before any request it says there is
/// none.
#[test]
fn context_shows_the_last_request_by_part() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    run_command(&mut app, "/context", now);
    assert!(notices(&mut app, now).contains("No request has been made"));
    link.give(event(
        40,
        Event::UsageUpdated(cortex_protocol::event::UsageUpdated {
            usage: SessionUsage {
                context: Some(ContextBreakdown {
                    system: 2_000,
                    tools: 3_000,
                    summary: 0,
                    user: 1_000,
                    assistant: 2_500,
                    tool_results: 1_500,
                    notices: 0,
                    estimate: 10_000,
                    limit: 150_000,
                    window: 200_000,
                }),
                ..SessionUsage::default()
            },
        }),
    ));
    drain(&mut app, now);
    run_command(&mut app, "/context", now);
    let all = notices(&mut app, now);
    assert!(all.contains("(estimates)"), "{all}");
    assert!(all.contains("tools"), "{all}");
    assert!(all.contains("30.0%"), "{all}");
    assert!(
        all.contains("10.0k  of the 150.0k a request may hold (6.7%)"),
        "{all}"
    );
    assert!(all.contains("in a window of 200.0k (5.0%)"), "{all}");
}

/// `/config` says where each value came from: its layer, file and line, or
/// that it is the built-in default.
#[test]
fn config_says_where_each_value_came_from() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    run_command(&mut app, "/config", now);
    let (ticket, _) = sent(&link, |op| matches!(op, Op::ConfigGet(_)));
    let mut config = serde_json::Map::new();
    config.insert("retry.attempts".to_owned(), 3.into());
    config.insert("tui.theme".to_owned(), "auto".into());
    let mut origins = std::collections::BTreeMap::new();
    origins.insert(
        "retry.attempts".to_owned(),
        ConfigOrigin {
            layer: ConfigLayer::User,
            file: Some("~/.config/cortex-harness/config.toml".to_owned()),
            line: Some(12),
        },
    );
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::ConfigGet(ConfigGetResult { config, origins })),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert!(
        all.contains("retry.attempts = 3  (user, ~/.config/cortex-harness/config.toml:12)"),
        "{all}"
    );
    assert!(all.contains("tui.theme = \"auto\"  (default)"), "{all}");
}
