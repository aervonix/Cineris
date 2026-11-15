//! The waits and time limits a repository's file could slow or stall a
//! session with (the owner's decision, 2026-11-15: `docs/steps/09/answered.md`,
//! "Settings a repository could slow a session with"). Each has a range that
//! binds every file, a project's file may shorten the waits and the timeouts
//! and may not lengthen them, and turning the background summary off is the
//! user's alone.

use cortex_config::schema::{self, Kind, Trust};
use cortex_config::{LayerKind, TrustClass};

use crate::{Machine, layer_of, value};

/// Each tighten-only wait, a value shorter than its default and one longer.
const WAITS: [(&str, &str, &str, &str); 8] = [
    // The key, its default as the file writes it, shorter, longer.
    ("retry.backoff_base", "1s", "500ms", "2s"),
    ("retry.backoff_cap", "30s", "10s", "1m"),
    ("retry.max_wait", "2m", "1m", "10m"),
    ("retry.idle_timeout", "1m30s", "45s", "5m"),
    ("retry.first_event_timeout", "1m", "30s", "2m"),
    ("model.speed_wait", "20s", "0s", "1m"),
    ("shell.timeout", "2m", "30s", "10m"),
    ("shell.max_timeout", "30m", "10m", "2h"),
];

/// The waits, one per line, each set to its shorter value or its longer.
fn file(longer: bool) -> String {
    WAITS
        .iter()
        .map(|(key, _, shorter, long)| {
            format!("{key} = \"{}\"\n", if longer { long } else { shorter })
        })
        .collect()
}

#[test]
fn a_project_may_shorten_each_wait_and_time_limit() {
    let machine = Machine::new("waits_shorter");
    machine.layer(LayerKind::Project, &file(false));
    let loaded = machine.load();
    for (key, _, shorter, _) in WAITS {
        assert_eq!(value(&loaded, key), format!("\"{shorter}\""), "{key}");
        assert_eq!(layer_of(&loaded, key), "project", "{key}");
    }
    assert!(loaded.withheld.is_empty());
    assert!(loaded.diagnostics.is_empty(), "{:?}", machine.said(&loaded));
    // What the readers are handed is the shorter value.
    assert_eq!(
        loaded.settings.retry.max_wait,
        std::time::Duration::from_secs(60)
    );
    assert_eq!(
        loaded.settings.retry_for("anthropic").idle_timeout,
        std::time::Duration::from_secs(45)
    );
    assert!(loaded.settings.model.speed_wait.is_zero());
}

#[test]
fn no_project_may_lengthen_a_wait_or_a_time_limit_trusted_or_not() {
    let mut machine = Machine::new("waits_longer");
    machine.layer(LayerKind::Project, &file(true));
    for trusted in [false, true] {
        if trusted {
            machine.trusted();
        }
        let loaded = machine.load();
        for (key, default, _, _) in WAITS {
            assert_eq!(value(&loaded, key), format!("\"{default}\""), "{key}");
            assert_eq!(layer_of(&loaded, key), "default", "{key}");
        }
        // Nothing waits for trust: trust does not change it.
        assert!(loaded.withheld.is_empty());
        // Each is ignored out loud, with what is in effect.
        let said = machine.said(&loaded);
        assert_eq!(said.len(), WAITS.len(), "{said:?}");
        for (line, (key, default, _, longer)) in WAITS.iter().enumerate() {
            let warning = format!(
                "warning: ~/work/app/.cortex/config.toml:{}: {key}: a project's configuration may \
                 lower this and may not raise it: \"{longer}\" is above the \"{default}\" in \
                 effect, so it is ignored. Set it in your own configuration file if you want it",
                line + 1
            );
            assert!(said.contains(&warning), "{warning}\nnot in {said:?}");
        }
    }
}

/// A project's value is compared with the value in effect, not with the
/// default, and as lengths of time, whatever units each is written in.
#[test]
fn a_length_of_time_is_tightened_against_the_one_in_effect_in_any_units() {
    let time = |text: &str| toml::Value::String(text.to_owned());
    let class = |wanted: &str, current: Option<&str>| {
        Trust::TightenOnly.class(&time(wanted), current.map(time).as_ref())
    };
    assert_eq!(class("1m30s", Some("90s")), TrustClass::Preference);
    assert_eq!(class("89999ms", Some("1m30s")), TrustClass::Preference);
    assert_eq!(class("90001ms", Some("1m30s")), TrustClass::UserOnly);
    assert_eq!(class("1h", Some("1d")), TrustClass::Preference);
    assert_eq!(class("1d", Some("1h")), TrustClass::UserOnly);
    // The first limit where there was none.
    assert_eq!(class("1d", None), TrustClass::Preference);

    // The user's own value is the one a project is held to.
    let machine = Machine::new("waits_against_the_user");
    machine.layer(LayerKind::User, "[retry]\nidle_timeout = \"40s\"\n");
    machine.layer(LayerKind::Project, "[retry]\nidle_timeout = \"1m\"\n");
    let loaded = machine.load();
    assert_eq!(value(&loaded, "retry.idle_timeout"), "\"40s\"");
    assert_eq!(layer_of(&loaded, "retry.idle_timeout"), "user");
    let said = machine.said(&loaded);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].contains("\"1m\" is above the \"40s\" in effect"),
        "{said:?}"
    );
    machine.layer(LayerKind::Project, "[retry]\nidle_timeout = \"40000ms\"\n");
    let loaded = machine.load();
    assert_eq!(layer_of(&loaded, "retry.idle_timeout"), "project");
    assert!(loaded.diagnostics.is_empty(), "{:?}", machine.said(&loaded));

    // And a time limit where there was none applies.
    machine.layer(LayerKind::Project, "[limits.turn]\ntime = \"10m\"\n");
    let loaded = machine.load();
    assert_eq!(value(&loaded, "limits.turn.time"), "\"10m\"");
    assert_eq!(layer_of(&loaded, "limits.turn.time"), "project");
}

/// What the backoff is multiplied by lengthens every later wait, so a
/// project's file may lower it and may not raise it (a twin found by the
/// review of this fix), compared as a number.
#[test]
fn a_project_may_lower_the_backoff_factor_and_not_raise_it() {
    let mut machine = Machine::new("waits_factor");
    machine.layer(LayerKind::Project, "[retry]\nbackoff_factor = 1.5\n");
    let loaded = machine.load();
    assert_eq!(value(&loaded, "retry.backoff_factor"), "1.5");
    assert!(loaded.diagnostics.is_empty(), "{:?}", machine.said(&loaded));
    machine.layer(LayerKind::Project, "[retry]\nbackoff_factor = 10\n");
    for trusted in [false, true] {
        if trusted {
            machine.trusted();
        }
        let loaded = machine.load();
        assert_eq!(value(&loaded, "retry.backoff_factor"), "2.0");
        assert_eq!(layer_of(&loaded, "retry.backoff_factor"), "default");
        let said = machine.said(&loaded);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("10 is above the 2.0 in effect"),
            "{said:?}"
        );
    }
}

/// Shortening stops at the floor: a project's file that sets a timeout a
/// request could never meet is refused, as the user's own would be.
#[test]
fn a_project_cannot_shorten_a_wait_past_its_floor() {
    let machine = Machine::new("waits_floor");
    machine.layer(
        LayerKind::Project,
        "[retry]\nfirst_event_timeout = \"1ms\"\nbackoff_base = \"0s\"\n",
    );
    let failure = machine.failure();
    for expected in [
        "retry.first_event_timeout: `1ms` is out of range: expected 30s to 1h",
        "retry.backoff_base: `0s` is out of range: expected 100ms to 1m",
    ] {
        assert!(
            failure.iter().any(|line| line.contains(expected)),
            "{expected}\nnot in {failure:?}"
        );
    }
}

#[test]
fn only_the_user_turns_the_background_summary_off() {
    let mut machine = Machine::new("waits_background");
    machine.layer(LayerKind::Project, "[context]\nbackground = false\n");
    for trusted in [false, true] {
        if trusted {
            machine.trusted();
        }
        let loaded = machine.load();
        assert!(loaded.settings.context.background, "trusted: {trusted}");
        assert!(loaded.withheld.is_empty());
        let said = machine.said(&loaded);
        assert_eq!(
            said,
            [
                "warning: ~/work/app/.cortex/config.toml:2: context.background: a project's \
                 configuration may not set this; it is ignored. Set it in your own configuration \
                 file if you want it"
            ]
        );
    }
    machine.layer(LayerKind::Project, "");
    machine.layer(LayerKind::User, "[context]\nbackground = false\n");
    assert!(!machine.load().settings.context.background);
}

/// The keys that hold a length of time or a number, may be set from a
/// project's file, and have no range, each with the reason it needs none.
/// Anything else of the kind has a range: a project's file applies its value
/// without trust (or, tighten-only, any lower one), so a key with no bound
/// is a key a repository can slow, stall or break a session with.
const UNBOUNDED: [(&str, &str); 4] = [
    // A limit stops a turn or a session on purpose, and says why, as
    // `requests = 1` does: a project's lower one is a restriction the design
    // lets it add (08-configuration, "A limit can be tightened by anyone").
    // A higher one is the user's alone.
    ("limits.turn.time", "a limit stops on purpose and says so"),
    (
        "limits.session.time",
        "a limit stops on purpose and says so",
    ),
    ("limits.day.time", "a limit stops on purpose and says so"),
    // The core holds it to at least 250 ms where it starts the index
    // (`cortex-core`'s `repo_map`), and a pass that runs out carries on when
    // the session is idle: no request waits for it.
    (
        "index.pass_time",
        "held to 250 ms at least where it is read",
    ),
];

#[test]
fn every_number_a_project_can_set_has_a_range() {
    fn counted(kind: &Kind) -> bool {
        match kind {
            Kind::Duration | Kind::Integer | Kind::Number => true,
            Kind::List(item) | Kind::Map(item) => counted(item),
            _ => false,
        }
    }
    let keys = schema::keys();
    // A setting declared as a named collection of values (a `BTreeMap` of
    // lengths of time, say) has no kind of its own here, and would pass
    // the check unseen. There is none: one that comes is given a kind, or
    // the check is taught to look inside it.
    let unread: Vec<&str> = keys
        .iter()
        .filter(|key| key.effect.is_some() && key.kind.is_none())
        .map(|key| key.path.as_str())
        .collect();
    assert!(
        unread.is_empty(),
        "settings this check cannot read: {unread:?}"
    );
    let mut flagged: Vec<String> = keys
        .into_iter()
        .filter(|key| key.trust != Trust::UserOnly && key.range.is_none())
        .filter(|key| key.kind.as_ref().is_some_and(counted))
        .map(|key| key.path)
        .collect();
    flagged.sort();
    let mut listed: Vec<&str> = UNBOUNDED.iter().map(|(key, _)| *key).collect();
    listed.sort_unstable();
    assert_eq!(
        flagged, listed,
        "a key that a project's file can set and that holds a length of time or a number needs \
         a range, or a place with its reason in `UNBOUNDED`; a key listed there that has a range \
         now, or is gone, leaves the list"
    );
}
