//! The project profile in an epoch: section 5 of the system prompt, fixed
//! for the epoch whatever happens to the profile, told as a notice when it
//! changes, whole again in the next epoch's prompt, and never the reason a
//! resumed session starts a new epoch
//! (`crates/cortex-agent/src/context/profile.rs` has the rules).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cortex_agent::{
    ContextSettings, EpochReason, NOTICE_LEAD, PROFILE_CHANGED, PROFILE_EMPTY, ProfileSource,
};
use cortex_base::ContentHash;
use cortex_protocol::item::NoticeSource;
use cortex_protocol::{Item, Message, Part, Role, StopReason};
use cortex_testkit::Response;
use serde_json::json;

use crate::transitions::{retry, start};
use crate::{Rig, Setup, World, calls, lock, says};

const ONE: &str = "Project profile: what is known.\n\nCommands:\n- test: `cargo test`";
const TWO: &str = "Project profile: what is known.\n\nCommands:\n- test: `cargo nextest run`";

/// A profile the test sets, and that counts how often it was asked.
#[derive(Default)]
struct Scripted {
    section: Mutex<String>,
    asked: AtomicUsize,
}

impl Scripted {
    fn new(section: &str) -> Arc<Self> {
        let source = Arc::new(Self::default());
        source.set(section);
        source
    }

    fn set(&self, section: &str) {
        section.clone_into(&mut lock(&self.section));
    }
}

impl ProfileSource for Scripted {
    fn section(&self) -> String {
        self.asked.fetch_add(1, Ordering::SeqCst);
        lock(&self.section).clone()
    }
}

fn with_profile(world: &World, source: &Arc<Scripted>) -> Setup {
    let mut setup = Setup::new(world);
    setup.profile = Some(Arc::clone(source) as Arc<dyn ProfileSource>);
    setup
}

fn text(message: &Message) -> String {
    message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

/// The profile notices of the log, in order.
fn told(rig: &Rig) -> Vec<String> {
    rig.store
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::Notice(notice) if notice.source == NoticeSource::Profile => Some(notice.text),
            _ => None,
        })
        .collect()
}

fn systems(world: &World) -> Vec<String> {
    world
        .provider
        .requests()
        .iter()
        .map(|request| request.system.clone())
        .collect()
}

#[test]
fn the_profile_is_section_five_of_the_epochs_prompt_and_its_hash_is_recorded() {
    let world = World::new("profile_in_prompt", [says("One.")]);
    let source = Scripted::new(ONE);
    let rig = with_profile(&world, &source).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    let system = &systems(&world)[0];
    assert!(system.ends_with(&format!("\n\n{ONE}")), "{system}");
    let epochs = rig.store.epochs();
    assert_eq!(epochs.len(), 1);
    assert_eq!(epochs[0].profile, Some(ContentHash::of(ONE.as_bytes())));
    assert_eq!(
        told(&rig),
        Vec::<String>::new(),
        "the prompt has it: nothing to tell"
    );

    // No profile, no section, no hash.
    let world = World::new("profile_none", [says("One.")]);
    let source = Scripted::new("");
    let rig = with_profile(&world, &source).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    assert!(!systems(&world)[0].contains("Project profile"));
    assert_eq!(rig.store.epochs()[0].profile, None);
    assert_eq!(told(&rig), Vec::<String>::new());
}

/// The done criterion: the profile section is stable within an epoch.
/// Whatever the profile does, through a tool round, a steer, a retry and
/// an interrupt, every request of the epoch has the prompt the first had,
/// byte for byte, and the change follows as one notice at the next turn's
/// start.
#[test]
fn the_prompt_is_fixed_for_the_epoch_and_a_change_follows_as_a_notice() {
    let retried = Response::new()
        .text("Half an ans")
        .event(retry(2))
        .event(start())
        .text("The whole answer.")
        .end(StopReason::End);
    let world = World::new(
        "profile_fixed",
        [
            calls(&[("c1", "read", json!({"path": "a", "gate": "tool"}))]),
            says("Changed course."),
            retried,
            Response::events([]).stall(),
            says("After the interrupt."),
            says("Last."),
        ],
    );
    let source = Scripted::new(ONE);
    let rig = with_profile(&world, &source).start(&world);
    world.run(async {
        rig.prompt("first");
        world.log.has("start read a").await;
        // The profile changes while a tool runs, and the user steers.
        source.set(TWO);
        rig.steer("use b instead");
        world.gates.open("tool");
        rig.events.turns(1).await;
    });
    assert_eq!(
        told(&rig),
        Vec::<String>::new(),
        "nothing is told inside a turn"
    );
    world.run(async {
        // The next turn is told, once, and its response is retried.
        rig.prompt("second");
        rig.events.turns(2).await;
        // A turn that is interrupted before anything comes back.
        rig.prompt("third");
        while world.provider.requests().len() < 4 {
            tokio::task::yield_now().await;
        }
        let _ = rig.handle.interrupt();
        rig.events.turns(3).await;
        // The profile changes back and forth between two turns: what is
        // told is what it is when the turn starts.
        source.set(ONE);
        source.set("");
        rig.prompt("fourth");
        rig.events.turns(4).await;
        rig.prompt("fifth");
        rig.events.turns(5).await;
    });

    let systems = systems(&world);
    assert_eq!(systems.len(), 6);
    assert!(systems[0].ends_with(ONE));
    for (index, system) in systems.iter().enumerate() {
        assert_eq!(system, &systems[0], "request {index} has another prompt");
    }
    assert_eq!(rig.store.epochs().len(), 1, "one epoch throughout");
    assert_eq!(
        told(&rig),
        [
            format!("{PROFILE_CHANGED}\n\n{TWO}"),
            PROFILE_EMPTY.to_owned()
        ],
        "one notice a change, and none where nothing changed"
    );
    // The notice is the harness's, just before the turn's own prompt.
    let items = rig.store.items();
    let at = items
        .iter()
        .position(
            |item| matches!(item, Item::Notice(notice) if notice.source == NoticeSource::Profile),
        )
        .unwrap();
    assert!(
        matches!(&items[at + 1], Item::UserMessage(user) if text_of_user(user) == "second"),
        "{:#?}",
        &items[at..=at + 1]
    );
    let second = &world.provider.requests()[2];
    let notice = second
        .messages
        .iter()
        .find(|message| text(message).contains(PROFILE_CHANGED))
        .expect("the notice is sent");
    assert_eq!(notice.role, Role::User);
    assert!(text(notice).starts_with(NOTICE_LEAD));
    assert!(text(notice).ends_with(TWO));
}

fn text_of_user(user: &cortex_protocol::item::UserMessageItem) -> String {
    user.parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

/// The owner's decision of 2026-11-16: a profile that changed between two
/// processes does not make a resume a new epoch, which would drop the
/// reasoning of the whole history. The resumed session goes on in its
/// epoch, with the prompt it had, and is told the change.
#[test]
fn a_resume_after_the_profile_changed_goes_on_in_its_epoch_and_is_told() {
    let world = World::new(
        "profile_resume",
        [
            says("One."),
            says("Two."),
            says("Three."),
            says("Four."),
            says("Five."),
        ],
    );
    let source = Scripted::new(ONE);
    let rig = with_profile(&world, &source).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    let before = world.provider.requests()[0].clone();

    // Another process: the profile is as it was. Nothing is told.
    let mut again = with_profile(&world, &Scripted::new(ONE));
    again.memory = rig.store.clone();
    let same = again.start(&world);
    world.run(async {
        same.prompt("second");
        same.events.turns(1).await;
    });
    assert_eq!(told(&same), Vec::<String>::new());
    assert_eq!(same.store.epochs().len(), 1);

    // And another, after a command was learned.
    let mut again = with_profile(&world, &Scripted::new(TWO));
    again.memory = same.store.clone();
    let changed = again.start(&world);
    world.run(async {
        changed.prompt("third");
        changed.events.turns(1).await;
        changed.prompt("fourth");
        changed.events.turns(2).await;
    });
    let epochs = changed.store.epochs();
    assert_eq!(epochs.len(), 1, "no new epoch for a changed profile");
    assert_eq!(epochs[0].reason, Some(EpochReason::Start));
    assert_eq!(
        told(&changed),
        [format!("{PROFILE_CHANGED}\n\n{TWO}")],
        "told once"
    );
    for request in world.provider.requests() {
        assert_eq!(
            request.system, before.system,
            "the prompt the epoch began with"
        );
    }
    let last = world.provider.requests().last().unwrap().clone();
    assert_eq!(
        &last.messages[..before.messages.len()],
        &before.messages[..],
        "the history goes on as it was sent"
    );

    // Read again, what the model has is the notice's profile: a fourth
    // process with the same profile tells nothing more.
    let mut again = with_profile(&world, &Scripted::new(TWO));
    again.memory = changed.store.clone();
    let quiet = again.start(&world);
    world.run(async {
        quiet.prompt("fifth");
        quiet.events.turns(1).await;
    });
    assert_eq!(told(&quiet).len(), 1);
    assert_eq!(quiet.store.epochs().len(), 1);
}

/// A session from before the profile existed has an epoch with no profile:
/// it goes on in that epoch, and is given the profile as a notice.
#[test]
fn a_session_whose_epoch_has_no_profile_is_given_it_as_a_notice() {
    let world = World::new("profile_old_epoch", [says("One."), says("Two.")]);
    let rig = Setup::new(&world).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    assert_eq!(rig.store.epochs()[0].profile, None);
    let mut again = with_profile(&world, &Scripted::new(ONE));
    again.memory = rig.store.clone();
    let resumed = again.start(&world);
    world.run(async {
        resumed.prompt("second");
        resumed.events.turns(1).await;
    });
    assert_eq!(resumed.store.epochs().len(), 1);
    assert_eq!(told(&resumed), [format!("{PROFILE_CHANGED}\n\n{ONE}")]);
}

/// The next epoch's prompt carries the whole current profile, and the old
/// epoch's notices of it are not replayed after the summary, live or read
/// again: one replayed there would put an old profile over the new prompt.
#[test]
fn a_compaction_builds_the_prompt_with_the_profile_as_it_is_and_replays_no_older_one() {
    let world = World::new(
        "profile_compaction",
        [
            says("One."),
            says("Two."),
            says("A summary."),
            says("Three."),
            says("After the resume."),
        ],
    );
    let source = Scripted::new(ONE);
    let mut setup = with_profile(&world, &source);
    setup.context = ContextSettings {
        keep_recent_tokens: 10_000,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        source.set(TWO);
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    assert_eq!(told(&rig).len(), 1);
    // A third profile by the time of the compaction.
    let three = TWO.replace("nextest run", "nextest run --workspace");
    source.set(&three);
    rig.handle.compact(None).expect("a compaction");
    world.run(async {
        rig.events.turns(3).await;
        rig.prompt("third");
        rig.events.turns(4).await;
    });
    let epochs = rig.store.epochs();
    assert_eq!(epochs.len(), 2);
    assert_eq!(epochs[1].reason, Some(EpochReason::Compaction));
    assert_eq!(epochs[1].profile, Some(ContentHash::of(three.as_bytes())));
    let last = world.provider.requests().last().unwrap().clone();
    assert!(last.system.ends_with(&three), "{}", last.system);
    let sent: String = last.messages.iter().map(text).collect();
    assert!(sent.contains("second"), "the turn is replayed: {sent}");
    assert!(
        !sent.contains(PROFILE_CHANGED) && !sent.contains("cargo test"),
        "an older profile after the new prompt: {sent}"
    );
    // The turn that compacted began as every turn does, and was told the
    // third profile there, in the old epoch. The new epoch is told nothing:
    // its prompt has it.
    assert_eq!(told(&rig).len(), 2);
    assert!(told(&rig)[1].ends_with(&three));

    // Read again by another process: the history is the one sent, and the
    // profile the model has is the new prompt's.
    let mut again = with_profile(&world, &Scripted::new(&three));
    again.memory = rig.store.clone();
    again.context = ContextSettings {
        keep_recent_tokens: 10_000,
        ..ContextSettings::default()
    };
    let resumed = again.start(&world);
    world.run(async {
        resumed.prompt("fourth");
        resumed.events.turns(1).await;
    });
    assert_eq!(told(&resumed).len(), 2, "nothing is told again");
    let after = world.provider.requests().last().unwrap().clone();
    assert_eq!(after.system, last.system);
    assert_eq!(
        &after.messages[..last.messages.len()],
        &last.messages[..],
        "the resumed history begins with the one sent before"
    );
}
