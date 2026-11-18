//! The count of narrowings (`Engine::narrowed`): it moves when a rule that
//! let calls run is taken back, and only then, so that the loop decides
//! again a call decided before
//! (`docs/architecture/03-agent-loop.md`, "Tool rounds").

use cineris_policy::command::ShellSyntax;
use cineris_policy::exact_command_rule;
use cineris_protocol::ids::ApprovalId;
use cineris_protocol::lease::{Effect, Rule, RuleAction};
use cineris_protocol::methods::ApprovalChoice;

use cineris_policy::ApprovalStore;

use crate::{clock, engine, engine_with, question, rule, shell};

fn exact(command: &str) -> Rule {
    exact_command_rule(command, ShellSyntax::Posix)
        .unwrap_or_else(|| panic!("no rule for {command:?}"))
}

#[test]
fn revoking_an_approval_in_force_narrows_and_revoking_nothing_does_not() {
    let engine = engine();
    assert_eq!(engine.narrowed(), 0);
    let asked = question(shell(&engine, "cargo build"));
    engine
        .record(&asked, &ApprovalChoice::AllowSession)
        .unwrap();
    assert_eq!(engine.narrowed(), 0, "an approval widens");

    assert_eq!(
        engine.revoke(&[ApprovalId::new("nothing-here")]).unwrap(),
        0
    );
    assert_eq!(engine.narrowed(), 0, "nothing was taken back");

    let id = engine.approvals()[0].id.clone();
    assert_eq!(engine.revoke(&[id]).unwrap(), 1);
    assert_eq!(engine.narrowed(), 1);
}

/// The same for an approval stored for the project, which the engine
/// takes back from its file and from the rules in force.
#[test]
fn revoking_an_approval_stored_for_the_project_narrows() {
    let scratch = cineris_testkit::scratch!("narrowed_stored");
    let store = ApprovalStore::new(scratch.join("approvals").join("aaaa1111.toml"), clock());
    let engine = engine_with(|parts| parts.store = Some(store));
    let asked = question(shell(&engine, "cargo build"));
    engine
        .record(&asked, &ApprovalChoice::AllowProject)
        .unwrap();
    assert_eq!(engine.narrowed(), 0, "an approval widens");
    let id = engine.approvals()[0].id.clone();
    assert_eq!(engine.revoke(&[id]).unwrap(), 1);
    assert_eq!(engine.narrowed(), 1);
}

#[test]
fn taking_a_learned_command_off_narrows_and_adding_one_does_not() {
    let engine = engine();
    engine.set_learned(vec![exact("cargo test")]).unwrap();
    assert_eq!(engine.narrowed(), 0, "a command learned widens");
    engine
        .set_learned(vec![exact("cargo test"), exact("make")])
        .unwrap();
    assert_eq!(engine.narrowed(), 0, "the same, and one more");
    engine.set_learned(vec![exact("make")]).unwrap();
    assert_eq!(engine.narrowed(), 1, "`cargo test` was taken off");
    engine.set_learned(Vec::new()).unwrap();
    assert_eq!(engine.narrowed(), 2, "`make` was taken off");
    engine.set_learned(Vec::new()).unwrap();
    assert_eq!(engine.narrowed(), 2, "nothing was there to take off");
}

/// A list that cannot be used leaves no learned rule in force: if there
/// were some, that is a narrowing.
#[test]
fn a_learned_list_refused_whole_narrows_when_it_takes_rules_away() {
    let engine = engine();
    let wrong = rule(Effect::Write, "x", RuleAction::Allow);
    assert!(engine.set_learned(vec![wrong.clone()]).is_err());
    assert_eq!(engine.narrowed(), 0, "there was nothing to take away");
    engine.set_learned(vec![exact("cargo test")]).unwrap();
    assert!(
        engine
            .set_learned(vec![exact("cargo test"), wrong])
            .is_err()
    );
    assert_eq!(engine.narrowed(), 1);
    assert!(engine.learned().is_empty());
}
