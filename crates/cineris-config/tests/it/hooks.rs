//! `[hooks.<name>]` (since step 28, `docs/steps/28/answered.md`, point 4):
//! declared once, enforced, the user's layers merged as any setting's and a
//! project's kept apart.

use std::time::Duration;

use cineris_config::LayerKind;
use cineris_config::schema::HookEventName;

use crate::{Machine, value};

const FMT: &str = "[hooks.fmt]\nevent = \"tool.after\"\ntools = [\"edit\", \"tracker/*\"]\n\
                   command = [\"cargo\", \"fmt\", \"--\", \"{path}\"]\nobserve = true\n\
                   timeout = \"90s\"\nwrite = true\n";

/// Every key of an entry reads into the user's settings.
#[test]
fn a_users_hook_reads_every_key() {
    let machine = Machine::new("hooks_read");
    machine.layer(LayerKind::User, FMT);
    let loaded = machine.load();
    let fmt = &loaded.settings.hooks["fmt"];
    assert_eq!(fmt.event, Some(HookEventName::ToolAfter));
    assert_eq!(fmt.tools, ["edit", "tracker/*"]);
    assert_eq!(fmt.command, ["cargo", "fmt", "--", "{path}"]);
    assert!(fmt.observe);
    assert_eq!(fmt.timeout, Some(Duration::from_secs(90)));
    assert!(fmt.write);
    assert!(!fmt.network);
    assert!(fmt.enabled);
    assert!(loaded.project_hooks.is_empty());
}

/// The section is enforced: a key this build does not know in an entry,
/// or a value of the wrong type, refuses the file, naming the key; a time
/// past ten minutes too.
#[test]
fn an_unknown_key_in_a_hooks_entry_refuses_the_file() {
    let machine = Machine::new("hooks_strict");
    for (text, said) in [
        (
            "[hooks.fmt]\nevent = \"tool.after\"\nmatch = \"*.rs\"\n",
            "hooks.fmt.match",
        ),
        ("[hooks.fmt]\nevent = \"tool.during\"\n", "hooks.fmt.event"),
        (
            "[hooks.fmt]\ncommand = \"cargo fmt\"\n",
            "hooks.fmt.command",
        ),
        ("[hooks.fmt]\ntimeout = \"11m\"\n", "hooks.fmt.timeout"),
        ("[hooks]\nfmt = true\n", "hooks.fmt"),
    ] {
        machine.layer(LayerKind::User, text);
        let failure = machine.failure().join("\n");
        assert!(failure.contains(said), "{text}: {failure}");
        // A project's file is refused the same way.
        machine.layer(LayerKind::User, "");
        machine.layer(LayerKind::Project, text);
        let failure = machine.failure().join("\n");
        assert!(failure.contains(said), "{text}: {failure}");
        machine.layer(LayerKind::Project, "");
    }
    machine.layer(LayerKind::User, "[hooks.fmt]\nmatch = 1\n");
    let failure = machine.failure().join("\n");
    assert!(
        failure.contains("unknown key in a table that is enforced"),
        "{failure}"
    );
}

/// A project's hooks are its own, never merged with the user's: the same
/// name in both leaves the user's as written, and the project's is read
/// apart, trusted or not, its shared file's keys under its local one's.
#[test]
fn a_projects_hooks_are_kept_apart_from_the_users() {
    let mut machine = Machine::new("hooks_apart");
    machine.layer(LayerKind::User, FMT);
    machine.layer(
        LayerKind::Project,
        "[hooks.fmt]\nevent = \"tool.before\"\ncommand = [\"deny-all\"]\nenabled = false\n\
         [hooks.lint]\nevent = \"tool.after\"\ncommand = [\"lint\"]\n",
    );
    machine.layer(
        LayerKind::ProjectLocal,
        "[hooks.lint]\ncommand = [\"lint\", \"--fix\"]\n",
    );
    for trusted in [false, true] {
        if trusted {
            machine.trusted();
        }
        let loaded = machine.load();
        let mine = &loaded.settings.hooks["fmt"];
        assert_eq!(mine.event, Some(HookEventName::ToolAfter), "{trusted}");
        assert!(mine.enabled, "a project cannot turn the user's off");
        assert_eq!(
            value(&loaded, "hooks.fmt.command"),
            "[\"cargo\", \"fmt\", \"--\", \"{path}\"]"
        );
        assert_eq!(loaded.settings.hooks.len(), 1, "{trusted}");
        let theirs: Vec<(&str, &[String], bool)> = loaded
            .project_hooks
            .iter()
            .map(|hook| {
                (
                    hook.name.as_str(),
                    hook.settings.command.as_slice(),
                    hook.settings.enabled,
                )
            })
            .collect();
        assert_eq!(
            theirs,
            [
                ("fmt", &["deny-all".to_owned()][..], false),
                ("lint", &["lint".to_owned(), "--fix".to_owned()][..], true)
            ],
            "{trusted}"
        );
        assert_eq!(
            loaded.project_hooks[1].settings.event,
            Some(HookEventName::ToolAfter),
            "the shared file's key under the local one's"
        );
        assert!(loaded.withheld.is_empty(), "nothing of a hook is held back");
        assert!(
            machine.said(&loaded).is_empty(),
            "nothing to warn of: {:?}",
            machine.said(&loaded)
        );
    }
}

/// A hook belongs to the file that defines it: a profile may not hold one.
#[test]
fn a_profile_may_not_hold_a_hook() {
    let machine = Machine::new("hooks_profile");
    machine.layer(
        LayerKind::User,
        "[profiles.review.hooks.fmt]\nevent = \"turn.end\"\ncommand = [\"log\"]\n",
    );
    let failure = machine.failure().join("\n");
    assert!(
        failure.contains("`hooks` belongs to the file and cannot be set by a profile"),
        "{failure}"
    );
}

/// A lock on a hook, or on any key of its entry, keeps a project's hook of
/// that name out, with a note; one of another name is read.
#[test]
fn a_lock_keeps_a_projects_hook_of_its_name_out() {
    let machine = Machine::new("hooks_locked");
    machine.layer(
        LayerKind::Managed,
        "[managed]\nlocked = [\"hooks.audit.command\"]\n\
         [hooks.audit]\nevent = \"turn.end\"\ncommand = [\"audit-log\"]\n",
    );
    machine.layer(
        LayerKind::Project,
        "[hooks.audit]\nobserve = true\n[hooks.lint]\nevent = \"turn.end\"\ncommand = [\"lint\"]\n",
    );
    let loaded = machine.load();
    let names: Vec<&str> = loaded
        .project_hooks
        .iter()
        .map(|hook| hook.name.as_str())
        .collect();
    assert_eq!(names, ["lint"]);
    // Named, so that one approved is known as the managed file's word.
    assert_eq!(loaded.project_hooks_locked, ["audit"]);
    let said = machine.said(&loaded);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains("hooks.audit"), "{said:?}");
    assert!(said[0].contains("this hook is not used"), "{said:?}");
}
