//! What extensions give the system prompt in an epoch: the index of
//! section 6 and the extensions' texts of section 7, fixed for the epoch;
//! a change to the index told whole at the next turn's start; a text the
//! user took back withdrawn by a notice naming its extension; and the epoch
//! rule on a resume, where section 7 is inside `base` and section 6 is not
//! (`crates/cineris-agent/src/context/extensions.rs` has the rules).

use std::sync::{Arc, Mutex};

use cineris_agent::{
    ContextSettings, EXTENSIONS_AGAIN, EpochReason, ExtensionPrompt, ExtensionSections,
    ExtensionsSource, INDEX_CHANGED, INDEX_EMPTY, NOTICE_LEAD, TEXTS_END, TEXTS_LEAD,
    TEXTS_REPLACED, render_index, withdrawn_text,
};
use cineris_base::ContentHash;
use cineris_protocol::event::Notice;
use cineris_protocol::item::NoticeSource;
use cineris_protocol::{Item, Message, Part, Role};
use cineris_tools::SourceSummary;
use serde_json::json;

use crate::{Rig, Setup, World, calls, lock, says};

const SKILLS: &str = "Skills: instructions for particular kinds of task.\n- pdf: Fill in PDF \
    forms. (.cineris/skills/pdf/SKILL.md)";

const NOTES: &str = "## From the extension `notes`, written by its author\n\n> Write notes.";
const TRACKER: &str =
    "## From the extension `tracker`, written by its author\n\n> Use the tracker.";

/// Sections the test sets; `None` makes the source fail, as a panic does.
struct Scripted {
    sections: Mutex<Option<ExtensionSections>>,
}

impl Scripted {
    fn new(sections: ExtensionSections) -> Arc<Self> {
        Arc::new(Self {
            sections: Mutex::new(Some(sections)),
        })
    }

    fn fail(&self) {
        *lock(&self.sections) = None;
    }

    fn set(&self, sections: ExtensionSections) {
        *lock(&self.sections) = Some(sections);
    }
}

impl ExtensionsSource for Scripted {
    #[expect(
        clippy::panic,
        reason = "a source that fails, as the loop must take one: it answers nothing"
    )]
    fn sections(&self) -> ExtensionSections {
        let sections = lock(&self.sections).clone();
        sections.unwrap_or_else(|| panic!("the source fails"))
    }
}

/// The text of every `info` notice the session sent its front ends.
fn infos(rig: &Rig) -> Vec<String> {
    rig.events
        .notices()
        .into_iter()
        .filter_map(|notice| match notice {
            Notice::Info(info) => Some(info.message),
            _ => None,
        })
        .collect()
}

fn tool_source(name: &str, tools: usize, description: &str) -> SourceSummary {
    SourceSummary {
        name: name.to_owned(),
        description: description.to_owned(),
        tools,
    }
}

fn prompt(name: &str, text: &str) -> ExtensionPrompt {
    ExtensionPrompt {
        name: name.to_owned(),
        text: text.to_owned(),
    }
}

/// A skill, and two extensions with tools and texts.
fn two() -> ExtensionSections {
    ExtensionSections {
        skills: SKILLS.to_owned(),
        sources: vec![
            tool_source("notes", 1, "Keeps notes"),
            tool_source("tracker", 3, "Issue tracker tools"),
        ],
        prompts: vec![
            prompt("notes", "Write notes."),
            prompt("tracker", "Use the tracker."),
        ],
    }
}

/// [`two`] with another list of skills: only section 6 differs.
fn other_skills() -> ExtensionSections {
    ExtensionSections {
        skills:
            "Skills: instructions.\n- review: Review a change. (~/.claude/skills/review/SKILL.md)"
                .to_owned(),
        ..two()
    }
}

/// [`two`] with `tracker`'s text changed: only section 7 differs.
fn tracker_changed() -> ExtensionSections {
    ExtensionSections {
        prompts: vec![
            prompt("notes", "Write notes."),
            prompt("tracker", "Use the tracker for every task."),
        ],
        ..two()
    }
}

/// [`two`] with `notes` disabled: its text, and its tools, gone.
fn without_notes() -> ExtensionSections {
    ExtensionSections {
        sources: vec![tool_source("tracker", 3, "Issue tracker tools")],
        prompts: vec![prompt("tracker", "Use the tracker.")],
        ..two()
    }
}

fn with_extensions(world: &World, source: &Arc<Scripted>) -> Setup {
    let mut setup = Setup::new(world);
    setup.extensions = Some(Arc::clone(source) as Arc<dyn ExtensionsSource>);
    setup
}

/// A session of another process, over the log of `rig`, with `sections`.
fn again(world: &World, rig: &Rig, sections: ExtensionSections) -> Setup {
    let mut setup = with_extensions(world, &Scripted::new(sections));
    setup.memory = rig.store.clone();
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

/// The notices of the source `extensions` in the log, in order.
fn told(rig: &Rig) -> Vec<String> {
    rig.store
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::Notice(notice) if notice.source == NoticeSource::Extensions => Some(notice.text),
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

fn changed(sections: &ExtensionSections) -> String {
    format!("{INDEX_CHANGED}\n\n{}", render_index(sections))
}

/// Runs one turn of `rig` with `prompt`, its `turns`th.
fn turn(world: &World, rig: &Rig, prompt: &str, turns: usize) {
    world.run(async {
        rig.prompt(prompt);
        rig.events.turns(turns).await;
    });
}

#[test]
fn both_sections_are_in_the_epochs_prompt_and_the_index_is_recorded() {
    let world = World::new("extensions_in_prompt", [says("One."), says("Two.")]);
    let rig = with_extensions(&world, &Scripted::new(two())).start(&world);
    turn(&world, &rig, "first", 1);
    let system = &systems(&world)[0];
    let index = render_index(&two());
    assert!(
        system.ends_with(&format!(
            "\n\n{index}\n\n{TEXTS_LEAD}\n\n{NOTES}\n\n{TRACKER}\n\n{TEXTS_END}"
        )),
        "{system}"
    );
    let epochs = rig.store.epochs();
    assert_eq!(epochs.len(), 1);
    assert_eq!(epochs[0].index, Some(ContentHash::of(index.as_bytes())));
    assert_eq!(
        told(&rig),
        Vec::<String>::new(),
        "the prompt has both: nothing to tell"
    );

    // With nothing to give, no section and no hash.
    let empty = with_extensions(&world, &Scripted::new(ExtensionSections::default()));
    let bare = empty.start(&world);
    turn(&world, &bare, "first", 1);
    assert!(!systems(&world)[1].contains("Skills:"));
    assert!(!systems(&world)[1].contains("## From the extension"));
    assert_eq!(bare.store.epochs()[0].index, None);
}

/// Whatever the index does during an epoch, through a tool round, every
/// request has the prompt the first had, byte for byte, and the change
/// follows whole, as one notice at the next turn's start.
#[test]
fn a_change_to_the_index_is_told_whole_at_the_next_turns_start() {
    let world = World::new(
        "extensions_index_changes",
        [
            calls(&[("c1", "read", json!({"path": "a", "gate": "tool"}))]),
            says("Read."),
            says("Two."),
            says("Three."),
            says("Four."),
        ],
    );
    let source = Scripted::new(two());
    let rig = with_extensions(&world, &source).start(&world);
    // A skill, a source of tools and an extension's text added while a tool
    // runs.
    let more = ExtensionSections {
        sources: vec![
            tool_source("notes", 1, "Keeps notes"),
            tool_source("tracker", 3, "Issue tracker tools"),
            tool_source("zed", 2, "Editor tools"),
        ],
        prompts: vec![
            prompt("notes", "Write notes."),
            prompt("tracker", "Use the tracker."),
            prompt("zed", "Use the editor."),
        ],
        ..other_skills()
    };
    world.run(async {
        rig.prompt("first");
        world.log.has("start read a").await;
        source.set(more.clone());
        world.gates.open("tool");
        rig.events.turns(1).await;
    });
    assert_eq!(
        told(&rig),
        Vec::<String>::new(),
        "nothing is told inside a turn"
    );
    turn(&world, &rig, "second", 2);
    // The index emptied, the texts as they were.
    source.set(ExtensionSections {
        skills: String::new(),
        sources: Vec::new(),
        ..more.clone()
    });
    turn(&world, &rig, "third", 3);
    turn(&world, &rig, "fourth", 4);

    let systems = systems(&world);
    assert_eq!(systems.len(), 5);
    for (at, system) in systems.iter().enumerate() {
        assert_eq!(system, &systems[0], "request {at} has another prompt");
    }
    assert!(
        !systems[0].contains("Use the editor."),
        "a text added waits for the next epoch"
    );
    assert_eq!(rig.store.epochs().len(), 1, "one epoch throughout");
    assert_eq!(
        told(&rig),
        [changed(&more), INDEX_EMPTY.to_owned()],
        "one notice a change, the section whole, and none where nothing changed"
    );
    // The notice is the harness's, just before the turn's own prompt.
    let items = rig.store.items();
    let at = items
        .iter()
        .position(|item| matches!(item, Item::Notice(notice) if notice.source == NoticeSource::Extensions))
        .unwrap();
    assert!(matches!(&items[at + 1], Item::UserMessage(_)));
    let request = &world.provider.requests()[2];
    let notice = request
        .messages
        .iter()
        .find(|message| text(message).contains(INDEX_CHANGED))
        .expect("the notice is sent");
    assert_eq!(notice.role, Role::User);
    assert!(text(notice).starts_with(NOTICE_LEAD));
    assert!(text(notice).ends_with(&render_index(&more)));
}

/// An extension the model has the text of, and that is removed, disabled
/// or changed, is withdrawn at the next turn's start by a notice that names
/// it as code; one that is as it was is not, and one given back during the
/// epoch is not told.
#[test]
fn a_text_removed_disabled_or_changed_is_withdrawn_by_name() {
    let world = World::new(
        "extensions_withdrawn",
        [
            says("One."),
            says("Two."),
            says("Three."),
            says("Four."),
            says("Five."),
        ],
    );
    let odd = prompt("x`y", "Odd text.");
    let three = ExtensionSections {
        prompts: vec![
            prompt("notes", "Write notes."),
            prompt("tracker", "Use the tracker."),
            odd.clone(),
        ],
        ..two()
    };
    let source = Scripted::new(three.clone());
    let rig = with_extensions(&world, &source).start(&world);
    turn(&world, &rig, "first", 1);
    // `notes` is disabled and `tracker` changed; `x\`y` is as it was. The
    // sources stay, so the index is unchanged.
    source.set(ExtensionSections {
        prompts: vec![prompt("tracker", "Use the tracker, always."), odd],
        ..three.clone()
    });
    turn(&world, &rig, "second", 2);
    assert_eq!(
        told(&rig),
        [withdrawn_text("notes"), withdrawn_text("tracker")]
    );
    turn(&world, &rig, "third", 3);
    assert_eq!(told(&rig).len(), 2, "each is withdrawn once");
    // `x\`y` is removed, and `notes` given back as it was.
    source.set(ExtensionSections {
        prompts: vec![
            prompt("notes", "Write notes."),
            prompt("tracker", "Use the tracker, always."),
        ],
        ..three
    });
    turn(&world, &rig, "fourth", 4);
    turn(&world, &rig, "fifth", 5);
    let notices = told(&rig);
    assert_eq!(notices.len(), 3, "{notices:#?}");
    assert!(
        notices[2].starts_with("The extension `x\\`y` was removed, disabled or changed"),
        "{}",
        notices[2]
    );
    let systems = systems(&world);
    for system in &systems {
        assert_eq!(system, &systems[0], "the prompt is fixed for the epoch");
    }
    assert_eq!(rig.store.epochs().len(), 1);
    let last = world.provider.requests().last().unwrap().clone();
    let sent: Vec<String> = last
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .map(text)
        .filter(|text| text.contains("was removed, disabled or changed"))
        .collect();
    assert_eq!(sent.len(), 3);
    assert!(sent.iter().all(|text| text.starts_with(NOTICE_LEAD)));
}

/// Section 7 is inside `base`: a resume whose extensions' texts differ (one
/// changed, one disabled) begins a new epoch, whose prompt holds them as
/// they are now. Nothing needs telling: the prompt has them.
#[test]
fn a_resume_whose_texts_differ_begins_a_new_epoch() {
    let world = World::new(
        "extensions_resume_texts",
        [says("One."), says("Two."), says("Three."), says("Four.")],
    );
    let rig = with_extensions(&world, &Scripted::new(two())).start(&world);
    turn(&world, &rig, "first", 1);

    let changed = again(&world, &rig, tracker_changed()).start(&world);
    turn(&world, &changed, "second", 1);
    let epochs = changed.store.epochs();
    assert_eq!(epochs.len(), 2);
    assert_eq!(epochs[1].reason, Some(EpochReason::Resume));
    assert!(systems(&world)[1].contains("Use the tracker for every task."));
    assert!(!systems(&world)[1].contains(TRACKER));

    let disabled = again(&world, &changed, without_notes()).start(&world);
    turn(&world, &disabled, "third", 1);
    assert_eq!(disabled.store.epochs().len(), 3);
    assert!(!systems(&world)[2].contains("Write notes."));

    // As they were when it was read: it goes on.
    let same = again(&world, &disabled, without_notes()).start(&world);
    turn(&world, &same, "fourth", 1);
    assert_eq!(same.store.epochs().len(), 3);
    assert_eq!(systems(&world)[3], systems(&world)[2]);
    // Each new prompt named what went or changed since the epoch before:
    // `tracker`'s text, then `notes`' and `tracker`'s (back as it was
    // first), after the notice that the prompt is as it is, the history
    // holding a notice of the source by then. The resume that goes on adds
    // nothing.
    let named = told(&same);
    assert_eq!(named.len(), 3, "{named:#?}");
    assert!(named[0].starts_with(&format!("{TEXTS_REPLACED} `tracker`.")));
    assert_eq!(named[1], EXTENSIONS_AGAIN);
    assert!(named[2].starts_with(&format!("{TEXTS_REPLACED} `notes`, `tracker`.")));
}

/// Section 6 is outside `base`: a resume whose index differs goes on in its
/// epoch, with the prompt it had, and is told the index whole. Read again,
/// what the model has is the notice's: nothing more is told.
#[test]
fn a_resume_whose_index_differs_goes_on_in_its_epoch_and_is_told() {
    let world = World::new(
        "extensions_resume_index",
        [says("One."), says("Two."), says("Three."), says("Four.")],
    );
    let rig = with_extensions(&world, &Scripted::new(two())).start(&world);
    turn(&world, &rig, "first", 1);
    let before = world.provider.requests()[0].clone();

    let resumed = again(&world, &rig, other_skills()).start(&world);
    turn(&world, &resumed, "second", 1);
    turn(&world, &resumed, "third", 2);
    assert_eq!(
        resumed.store.epochs().len(),
        1,
        "no new epoch for a changed index"
    );
    assert_eq!(told(&resumed), [changed(&other_skills())], "told once");
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

    let quiet = again(&world, &resumed, other_skills()).start(&world);
    turn(&world, &quiet, "fourth", 1);
    assert_eq!(quiet.store.epochs().len(), 1);
    assert_eq!(told(&quiet).len(), 1);
}

/// An epoch that withdrew a text is not gone on with when it is read
/// again, even when the extension is back as it was and the prompt would
/// be the epoch's to the byte: the model was told to disregard that text.
/// The new epoch's prompt holds it, and one notice says the prompt is as
/// it is now, after the withdrawal the history keeps.
#[test]
fn a_resume_after_a_withdrawal_begins_a_new_epoch_and_says_the_prompt_is_current() {
    let world = World::new(
        "extensions_resume_withdrawn",
        [
            says("One."),
            says("Two."),
            says("Three."),
            says("Four."),
            says("Five."),
            says("A summary."),
        ],
    );
    let source = Scripted::new(two());
    let rig = with_extensions(&world, &source).start(&world);
    turn(&world, &rig, "first", 1);
    source.set(ExtensionSections {
        prompts: vec![prompt("tracker", "Use the tracker.")],
        ..two()
    });
    turn(&world, &rig, "second", 2);
    assert_eq!(told(&rig), [withdrawn_text("notes")]);

    // `notes` is back, as it was.
    let resumed = again(&world, &rig, two()).start(&world);
    turn(&world, &resumed, "third", 1);
    turn(&world, &resumed, "fourth", 2);
    let epochs = resumed.store.epochs();
    assert_eq!(epochs.len(), 2);
    assert_eq!(epochs[1].reason, Some(EpochReason::Resume));
    assert_eq!(
        told(&resumed),
        [withdrawn_text("notes"), EXTENSIONS_AGAIN.to_owned()],
        "said once, as the epoch began"
    );
    let last = world.provider.requests().last().unwrap().clone();
    assert!(last.system.contains(NOTES));
    let sent: Vec<String> = last.messages.iter().map(text).collect();
    let withdrawn = sent
        .iter()
        .position(|text| *text == format!("{NOTICE_LEAD}{}", withdrawn_text("notes")))
        .expect("the withdrawal is in the history that goes on");
    let current = sent
        .iter()
        .position(|text| *text == format!("{NOTICE_LEAD}{EXTENSIONS_AGAIN}"))
        .expect("the notice is sent");
    assert!(
        withdrawn < current,
        "the last word is that the prompt is as it is"
    );

    // Read again, the epoch withdrew nothing: it goes on.
    let third = again(&world, &resumed, two()).start(&world);
    turn(&world, &third, "fifth", 1);
    assert_eq!(third.store.epochs().len(), 2);
    assert_eq!(told(&third).len(), 2);
    // Its summary is still asked to leave out what `notes`' text asked for
    // while it was withdrawn, read back from the withdrawal: the turns of
    // then went on without it. The text the prompt gives now applies.
    third.handle.compact(None).expect("a compaction");
    world.run(async {
        third.events.turns(2).await;
    });
    let summary = world.provider.requests().last().unwrap().clone();
    let asked = summary.messages.last().map(text).unwrap_or_default();
    assert!(
        asked.contains("The text of the extensions `notes` was withdrawn"),
        "{asked}"
    );
}

/// A resume that begins an epoch for another reason (its tools changed)
/// keeps its history, and a notice of an older index in it would stand
/// after the new prompt as the last word: one notice follows, which says
/// the prompt holds both sections as they are.
#[test]
fn a_resume_into_a_new_epoch_after_an_index_notice_says_the_prompt_is_current() {
    let world = World::new(
        "extensions_resume_new_epoch",
        [
            says("One."),
            says("Two."),
            says("Three."),
            says("Four."),
            says("Five."),
        ],
    );
    let source = Scripted::new(two());
    let rig = with_extensions(&world, &source).start(&world);
    turn(&world, &rig, "first", 1);
    source.set(other_skills());
    turn(&world, &rig, "second", 2);
    assert_eq!(told(&rig), [changed(&other_skills())]);

    // Another process, with other tools, and the index as it first was.
    let mut setup = again(&world, &rig, two());
    setup.toolbox = Some(world.toolbox().with(cineris_tools::ReadTool::new()));
    let resumed = setup.start(&world);
    turn(&world, &resumed, "third", 1);
    turn(&world, &resumed, "fourth", 2);
    let epochs = resumed.store.epochs();
    assert_eq!(epochs.len(), 2);
    assert_eq!(epochs[1].reason, Some(EpochReason::Resume));
    assert_eq!(
        epochs[1].index,
        Some(ContentHash::of(render_index(&two()).as_bytes()))
    );
    assert_eq!(
        told(&resumed),
        [changed(&other_skills()), EXTENSIONS_AGAIN.to_owned()]
    );

    // Read again, the session knows what it said: nothing more is told.
    let mut setup = again(&world, &resumed, two());
    setup.toolbox = Some(world.toolbox().with(cineris_tools::ReadTool::new()));
    let third = setup.start(&world);
    turn(&world, &third, "fifth", 1);
    assert_eq!(third.store.epochs().len(), 2);
    assert_eq!(told(&third).len(), 2);
}

/// The next epoch's prompt holds both sections as they are, and no notice
/// of the source from the old epoch is replayed after the summary, live or
/// read again: an older index or a withdrawal there would stand over the
/// new prompt.
#[test]
fn a_compaction_builds_both_sections_as_they_are_and_replays_no_older_notice() {
    let world = World::new(
        "extensions_compaction",
        [
            says("One."),
            says("Two."),
            says("A summary."),
            says("Three."),
            says("After the resume."),
        ],
    );
    let source = Scripted::new(two());
    let mut setup = with_extensions(&world, &source);
    setup.context = ContextSettings {
        keep_recent_tokens: 10_000,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    turn(&world, &rig, "first", 1);
    source.set(other_skills());
    turn(&world, &rig, "second", 2);
    assert_eq!(told(&rig).len(), 1);
    // By the time of the compaction, `notes` is disabled too.
    let third = ExtensionSections {
        skills: other_skills().skills,
        ..without_notes()
    };
    source.set(third.clone());
    rig.handle.compact(None).expect("a compaction");
    world.run(async {
        rig.events.turns(3).await;
        rig.prompt("third");
        rig.events.turns(4).await;
    });
    let epochs = rig.store.epochs();
    assert_eq!(epochs.len(), 2);
    assert_eq!(epochs[1].reason, Some(EpochReason::Compaction));
    assert_eq!(
        epochs[1].index,
        Some(ContentHash::of(render_index(&third).as_bytes()))
    );
    let last = world.provider.requests().last().unwrap().clone();
    assert!(
        last.system.ends_with(&format!(
            "{}\n\n{TEXTS_LEAD}\n\n{TRACKER}\n\n{TEXTS_END}",
            render_index(&third)
        )),
        "{}",
        last.system
    );
    let sent: String = last.messages.iter().map(text).collect();
    assert!(sent.contains("second"), "the turn is replayed: {sent}");
    assert!(
        !sent.contains(INDEX_CHANGED) && !sent.contains("was removed, disabled or changed"),
        "an older notice after the new prompt: {sent}"
    );
    // The turn that compacted began as every turn does, and was told the
    // change there, in the old epoch. The new epoch is told nothing.
    assert_eq!(
        told(&rig),
        [
            changed(&other_skills()),
            withdrawn_text("notes"),
            changed(&third)
        ]
    );

    // Read again by another process: the history is the one sent, and
    // the epoch withdrew nothing.
    let mut setup = again(&world, &rig, third);
    setup.context = ContextSettings {
        keep_recent_tokens: 10_000,
        ..ContextSettings::default()
    };
    let resumed = setup.start(&world);
    turn(&world, &resumed, "fourth", 1);
    assert_eq!(resumed.store.epochs().len(), 2);
    assert_eq!(told(&resumed).len(), 3, "nothing is told again");
    let after = world.provider.requests().last().unwrap().clone();
    assert_eq!(after.system, last.system);
    assert_eq!(
        &after.messages[..last.messages.len()],
        &last.messages[..],
        "the resumed history begins with the one sent before"
    );
}

/// Section 7's cap is in all: an extension added during the epoch would
/// push another past it in the prompt the harness would build now, which
/// is no reason to withdraw that one, as it is as it was. One whose text
/// changed is withdrawn, cut by the cap or not.
#[test]
fn a_text_another_pushes_past_the_cap_is_not_withdrawn() {
    let world = World::new(
        "extensions_past_the_cap",
        [says("One."), says("Two."), says("Three.")],
    );
    // Four texts of 4 KiB: the cap, all whole.
    let full = |letter: char| letter.to_string().repeat(cineris_agent::EXTENSION_TEXT_CAP);
    let four: Vec<ExtensionPrompt> = ['b', 'c', 'd', 'e']
        .into_iter()
        .map(|letter| prompt(&letter.to_string(), &full(letter)))
        .collect();
    let sections = |prompts: Vec<ExtensionPrompt>| ExtensionSections {
        skills: SKILLS.to_owned(),
        sources: Vec::new(),
        prompts,
    };
    let source = Scripted::new(sections(four.clone()));
    let rig = with_extensions(&world, &source).start(&world);
    turn(&world, &rig, "first", 1);
    assert!(systems(&world)[0].contains(&format!("> {}", full('e'))));
    // `a` comes first by name: `e` would be left out now.
    let mut five = vec![prompt("a", &full('a'))];
    five.extend(four.clone());
    source.set(sections(five));
    turn(&world, &rig, "second", 2);
    assert_eq!(told(&rig), Vec::<String>::new());
    // `e` changed.
    let mut changed = four;
    changed[3] = prompt("e", "Another text.");
    source.set(sections(changed));
    turn(&world, &rig, "third", 3);
    assert_eq!(told(&rig), [withdrawn_text("e")]);
}

/// What a withdrawal is measured against is the prompt of the epoch in
/// force: after a compaction, the new prompt's texts (`notes`, withdrawn in
/// the old epoch and not in the new prompt, is not withdrawn again); after
/// a resume that goes on in its epoch, the epoch's.
#[test]
fn a_withdrawal_is_of_a_text_the_epoch_in_force_holds() {
    let world = World::new(
        "extensions_withdrawn_after",
        [
            says("One."),
            says("Two."),
            says("A summary."),
            says("Three."),
            says("Four."),
            says("Five."),
            says("Six."),
        ],
    );
    let source = Scripted::new(two());
    let mut setup = with_extensions(&world, &source);
    setup.context = ContextSettings {
        keep_recent_tokens: 10_000,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    turn(&world, &rig, "first", 1);
    turn(&world, &rig, "second", 2);
    source.set(without_notes());
    rig.handle.compact(None).expect("a compaction");
    world.run(async {
        rig.events.turns(3).await;
    });
    assert_eq!(rig.store.epochs().len(), 2);
    // Told as the compaction's turn began, in the old epoch.
    assert_eq!(
        told(&rig),
        [withdrawn_text("notes"), changed(&without_notes())]
    );
    // `tracker`'s text is gone in the new epoch.
    source.set(ExtensionSections {
        prompts: Vec::new(),
        ..without_notes()
    });
    turn(&world, &rig, "third", 4);
    assert_eq!(rig.store.epochs().len(), 2);
    assert_eq!(
        told(&rig)[2..],
        [withdrawn_text("tracker")],
        "{:#?}",
        told(&rig)
    );

    // A session read again goes on in its epoch, and knows its texts.
    let other = with_extensions(&world, &Scripted::new(two())).start(&world);
    turn(&world, &other, "first", 1);
    let source = Scripted::new(two());
    let mut setup = with_extensions(&world, &source);
    setup.memory = other.store.clone();
    let resumed = setup.start(&world);
    turn(&world, &resumed, "second", 1);
    assert_eq!(resumed.store.epochs().len(), 1);
    source.set(ExtensionSections {
        prompts: vec![prompt("tracker", "Use the tracker.")],
        ..two()
    });
    turn(&world, &resumed, "third", 2);
    assert_eq!(told(&resumed), [withdrawn_text("notes")]);
}

/// What section 7's cap leaves out of a prompt is told to the user, not to
/// the model, once an epoch, as it begins: not at the turns after, and
/// again at a compaction's new prompt; not by a resume that goes on in its
/// epoch, whose prompt was told when it began.
#[test]
fn what_the_cap_leaves_out_is_told_to_the_user_once_an_epoch() {
    let world = World::new(
        "extensions_cap_told",
        [
            says("One."),
            says("Two."),
            says("A summary."),
            says("Three."),
        ],
    );
    let full = |letter: char| letter.to_string().repeat(cineris_agent::EXTENSION_TEXT_CAP);
    let sections = ExtensionSections {
        skills: SKILLS.to_owned(),
        sources: Vec::new(),
        prompts: ['b', 'c', 'd', 'e', 'f']
            .into_iter()
            .map(|letter| prompt(&letter.to_string(), &full(letter)))
            .collect(),
    };
    let source = Scripted::new(sections.clone());
    let mut setup = with_extensions(&world, &source);
    setup.context = ContextSettings {
        keep_recent_tokens: 10_000,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    turn(&world, &rig, "first", 1);
    turn(&world, &rig, "second", 2);
    // Four texts of 4 KiB are the cap: the fifth is left out.
    let told = [
        "The text the extension `f` gives the system prompt is left out, to keep the \
         extensions' part of the system prompt within 16 KiB: the model is not given it."
            .to_owned(),
    ];
    assert_eq!(infos(&rig), told, "once, as it began");
    let sent: String = world
        .provider
        .requests()
        .iter()
        .flat_map(|request| request.messages.iter().map(text))
        .collect();
    assert!(!sent.contains("is left out"), "the model is not told");
    rig.handle.compact(None).expect("a compaction");
    world.run(async {
        rig.events.turns(3).await;
    });
    assert_eq!(rig.store.epochs().len(), 2);
    assert_eq!(infos(&rig), [told[0].clone(), told[0].clone()]);

    let mut setup = with_extensions(&world, &Scripted::new(sections));
    setup.memory = rig.store.clone();
    setup.context = ContextSettings {
        keep_recent_tokens: 10_000,
        ..ContextSettings::default()
    };
    let resumed = setup.start(&world);
    turn(&world, &resumed, "third", 1);
    assert_eq!(resumed.store.epochs().len(), 2, "it goes on");
    assert!(infos(&resumed).is_empty());
}

/// Within a turn, a text taken back is withdrawn before the next request
/// (the early review's M1): a headless run is one turn. The extension is
/// disabled while a tool of the turn runs, and the request after the round
/// carries the withdrawal; the index waits for the next turn's start.
#[test]
fn a_text_taken_back_during_a_turn_is_withdrawn_before_its_next_request() {
    let world = World::new(
        "extensions_mid_turn",
        [
            calls(&[("c1", "read", json!({"path": "a", "gate": "tool"}))]),
            says("Done."),
        ],
    );
    let source = Scripted::new(two());
    let rig = with_extensions(&world, &source).start(&world);
    world.run(async {
        rig.prompt("first");
        world.log.has("start read a").await;
        source.set(without_notes());
        world.gates.open("tool");
        rig.events.turns(1).await;
    });
    assert_eq!(told(&rig), [withdrawn_text("notes")], "no index notice yet");
    let requests = world.provider.requests();
    assert_eq!(requests.len(), 2);
    let notice = format!("{NOTICE_LEAD}{}", withdrawn_text("notes"));
    assert!(
        !requests[0]
            .messages
            .iter()
            .any(|message| text(message) == notice)
    );
    assert_eq!(
        requests[1].messages.last().map(text),
        Some(notice),
        "the withdrawal is the last word before the request"
    );
    assert_eq!(requests[1].system, requests[0].system);
}

/// A resume that begins an epoch because section 7 changed between two
/// processes names the extensions whose text went or changed, from what
/// the epoch before recorded of its texts (the early review's L2), though
/// its history holds no notice of the source.
#[test]
fn a_resume_names_the_texts_that_went_or_changed_since_its_epoch() {
    let world = World::new(
        "extensions_resume_named",
        [
            says("One."),
            says("Two."),
            says("Three."),
            says("A summary."),
        ],
    );
    let rig = with_extensions(&world, &Scripted::new(two())).start(&world);
    turn(&world, &rig, "first", 1);
    let epochs = rig.store.epochs();
    assert_eq!(
        epochs[0]
            .texts
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["notes", "tracker"]
    );
    // `notes` disabled, `tracker` changed.
    let changed = ExtensionSections {
        prompts: vec![prompt("tracker", "Use the tracker for every task.")],
        ..without_notes()
    };
    let resumed = again(&world, &rig, changed.clone()).start(&world);
    turn(&world, &resumed, "second", 1);
    assert_eq!(resumed.store.epochs().len(), 2);
    let named = told(&resumed);
    assert_eq!(named.len(), 1, "{named:#?}");
    assert!(
        named[0].starts_with(&format!("{TEXTS_REPLACED} `notes`, `tracker`.")),
        "{}",
        named[0]
    );
    // Read again, it is no withdrawal: the epoch goes on.
    let third = again(&world, &resumed, changed).start(&world);
    turn(&world, &third, "third", 1);
    assert_eq!(third.store.epochs().len(), 2);
    assert_eq!(told(&third).len(), 1);
    // And the summary is asked to leave out what their text asked for, as
    // for a text withdrawn during a session (the review of step 28's
    // sections), read back from the notice.
    third.handle.compact(None).expect("a compaction");
    world.run(async {
        third.events.turns(2).await;
    });
    let summary = world.provider.requests().last().unwrap().clone();
    let asked = summary.messages.last().map(text).unwrap_or_default();
    assert!(
        asked.contains("The text of the extensions `notes`, `tracker` was withdrawn"),
        "{asked}"
    );
}

/// A resume whose new prompt cuts a text the epoch before held whole names
/// it too (the review of step 28's sections): its mark, of the text
/// standing alone, is as it was, and the rest of it no longer applies.
#[test]
fn a_resume_whose_prompt_cuts_a_text_names_it() {
    let world = World::new(
        "extensions_resume_cut",
        [says("One."), says("Two."), says("A summary.")],
    );
    let full = |letter: char| letter.to_string().repeat(cineris_agent::EXTENSION_TEXT_CAP);
    let mut prompts: Vec<ExtensionPrompt> = ['b', 'c', 'd']
        .into_iter()
        .map(|letter| prompt(&letter.to_string(), &full(letter)))
        .collect();
    prompts.push(prompt("e", &"e".repeat(4000)));
    let sections = |prompts: Vec<ExtensionPrompt>| ExtensionSections {
        skills: SKILLS.to_owned(),
        sources: Vec::new(),
        prompts,
    };
    let rig = with_extensions(&world, &Scripted::new(sections(prompts.clone()))).start(&world);
    turn(&world, &rig, "first", 1);
    assert!(systems(&world)[0].contains(&format!("> {}", "e".repeat(4000))));
    // `a`, first by name, leaves `e` less than its whole.
    prompts.push(prompt("a", &"a".repeat(100)));
    let resumed = again(&world, &rig, sections(prompts)).start(&world);
    turn(&world, &resumed, "second", 1);
    assert_eq!(resumed.store.epochs().len(), 2);
    let named = told(&resumed);
    assert_eq!(named.len(), 1, "{named:#?}");
    assert!(
        named[0].starts_with(&format!("{TEXTS_REPLACED} `e`.")),
        "{}",
        named[0]
    );
    // The summary is asked to leave out what the rest of it asked for.
    resumed.handle.compact(None).expect("a compaction");
    world.run(async {
        resumed.events.turns(2).await;
    });
    let summary = world.provider.requests().last().unwrap().clone();
    let asked = summary.messages.last().map(text).unwrap_or_default();
    assert!(
        asked.contains("The text of the extensions `e` was withdrawn"),
        "{asked}"
    );
}

/// A turn interrupted as it begins still has its epoch's prompt built
/// whole (the review of step 28's sections): the source is asked to its
/// end whatever the turn's token says, so the next turn's requests hold the
/// extensions' texts.
#[test]
fn an_epoch_begun_by_an_interrupted_turn_holds_both_sections() {
    let world = World::new("extensions_interrupted_start", [says("Two.")]);
    let rig = with_extensions(&world, &Scripted::new(two())).start(&world);
    world.run(async {
        // Interrupted before the loop takes it up.
        rig.prompt("first");
        let _ = rig.handle.interrupt();
        rig.events.turns(1).await;
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    let systems = systems(&world);
    assert_eq!(systems.len(), 1, "the first turn sent nothing");
    assert!(
        systems[0].contains(NOTES) && systems[0].contains(TRACKER),
        "{}",
        systems[0]
    );
    assert_eq!(rig.store.epochs().len(), 1);
}

/// A compaction's summary is asked to leave out what a text withdrawn in
/// the history it covers asked for (the early review's L3): live, and in a
/// session read again before it compacts, which reads the withdrawals back
/// from the history since the last summary; and the summary after that,
/// of a history that withdrew nothing more, is not.
#[test]
fn a_summary_is_asked_to_leave_out_a_withdrawn_text() {
    let world = World::new(
        "extensions_summary_leaves_out",
        [
            says("One."),
            says("Two."),
            says("A summary."),
            says("Three."),
            says("A second summary."),
            says("A third summary."),
        ],
    );
    let source = Scripted::new(two());
    let rig = with_extensions(&world, &source).start(&world);
    turn(&world, &rig, "first", 1);
    source.set(without_notes());
    turn(&world, &rig, "second", 2);
    assert_eq!(told(&rig)[0], withdrawn_text("notes"));
    rig.handle.compact(None).expect("a compaction");
    world.run(async {
        rig.events.turns(3).await;
    });
    // `tracker`'s text taken back in the new epoch.
    let none = ExtensionSections {
        prompts: Vec::new(),
        ..without_notes()
    };
    source.set(none.clone());
    turn(&world, &rig, "third", 4);
    // Read again by another process, which compacts twice.
    let resumed = again(&world, &rig, none).start(&world);
    for turns in [1, 2] {
        resumed.handle.compact(None).expect("a compaction");
        world.run(async {
            resumed.events.turns(turns).await;
        });
    }
    let requests = world.provider.requests();
    assert_eq!(requests.len(), 6);
    let asked =
        |request: &cineris_llm::ModelRequest| request.messages.last().map(text).unwrap_or_default();
    let live = asked(&requests[2]);
    assert!(
        live.contains(
            "The text of the extensions `notes` was withdrawn from the system prompt in this \
             conversation: leave out of the summary what that text asked for"
        ),
        "{live}"
    );
    let read_again = asked(&requests[4]);
    assert!(
        read_again.contains("The text of the extensions `tracker` was withdrawn"),
        "{read_again}"
    );
    assert!(
        !read_again.contains("`notes`"),
        "summarised already: {read_again}"
    );
    assert!(
        !asked(&requests[5]).contains("was withdrawn"),
        "{}",
        asked(&requests[5])
    );
}

/// A source that fails while an epoch is in force holds no text (the early
/// review's L6): what the model has of section 7 is withdrawn, failing
/// closed, and the index is left as it is.
#[test]
fn a_source_that_fails_withdraws_the_texts_the_model_has() {
    let world = World::new("extensions_failed", [says("One."), says("Two.")]);
    let source = Scripted::new(two());
    let rig = with_extensions(&world, &source).start(&world);
    turn(&world, &rig, "first", 1);
    source.fail();
    turn(&world, &rig, "second", 2);
    assert_eq!(
        told(&rig),
        [withdrawn_text("notes"), withdrawn_text("tracker")]
    );
}
