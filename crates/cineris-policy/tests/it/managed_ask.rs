//! Whether a question carries an `ask` of the managed configuration
//! (`Question::managed`): the verifier holds back the user's own check by
//! it (the owner's decision of 2026-11-16).

use cineris_policy::{Outcome, RuleSource, SourcedRule};
use cineris_protocol::lease::{Effect, RuleAction};

use crate::{project, rule, shell, user, with_rules};

fn managed(effect: Effect, pattern: &str, action: RuleAction) -> SourcedRule {
    SourcedRule::new(rule(effect, pattern, action), RuleSource::Managed)
}

/// Whether the shell call `command` is asked about with the managed mark.
fn marked(rules: Vec<SourcedRule>, command: &str) -> bool {
    match shell(&with_rules(rules), command) {
        Outcome::Ask(question) => question.managed,
        other => panic!("`{command}` is not asked about: {other:?}"),
    }
}

#[test]
fn an_ask_of_the_managed_configuration_is_marked_whichever_way_it_comes() {
    let ask = RuleAction::Ask;
    for (what, rules, command) in [
        // A rule on the command.
        (
            "an exec rule",
            vec![managed(Effect::Exec, "./check", ask)],
            "./check",
        ),
        // The same, beside another reason to ask that is not the rule's.
        (
            "an exec rule beside a command off the safe list",
            vec![managed(Effect::Exec, "./check", ask)],
            "./check && make",
        ),
        // A rule on a path the command reads, writes or only names.
        (
            "a read rule on a path the command reads",
            vec![managed(Effect::Read, "/work/app/secrets/**", ask)],
            "cat secrets/token",
        ),
        (
            "a read rule on a path the command only names",
            vec![managed(Effect::Read, "/work/app/secrets/**", ask)],
            "./check secrets/token",
        ),
        (
            "a write rule on a path a command removes",
            vec![managed(Effect::Write, "/work/app/deploy/**", ask)],
            "rm deploy/x",
        ),
    ] {
        assert!(marked(rules, command), "{what}: `{command}` is not marked");
    }
}

#[test]
fn a_managed_ask_is_marked_beside_another_layers_ask_in_either_order() {
    let ask = RuleAction::Ask;
    let orders: [(&str, Vec<SourcedRule>); 4] = [
        (
            "managed, then the user's",
            vec![
                managed(Effect::Exec, "./check", ask),
                user(Effect::Exec, "./check", ask),
            ],
        ),
        (
            "the user's, then managed",
            vec![
                user(Effect::Exec, "./check", ask),
                managed(Effect::Exec, "./check", ask),
            ],
        ),
        (
            "managed, then the user's ask of every command",
            vec![
                managed(Effect::Exec, "./check", ask),
                user(Effect::Exec, "*", ask),
            ],
        ),
        (
            "managed, then the project's",
            vec![
                managed(Effect::Exec, "./check", ask),
                project(Effect::Exec, "./check *", ask),
            ],
        ),
    ];
    for (what, rules) in orders {
        assert!(marked(rules, "./check"), "{what}: not marked");
    }
}

#[test]
fn an_ask_of_any_other_layer_is_not_marked() {
    let ask = RuleAction::Ask;
    for (what, rules, command) in [
        (
            "the user's exec rule",
            vec![user(Effect::Exec, "./check", ask)],
            "./check",
        ),
        (
            "the project's path rule",
            vec![project(Effect::Read, "/work/app/secrets/**", ask)],
            "./check secrets/token",
        ),
    ] {
        assert!(!marked(rules, command), "{what}: `{command}` is marked");
    }
}
