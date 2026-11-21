//! What a session's skills and extensions give its system prompt (step
//! 28): the skills found on the session's finding with its extensions,
//! the user's always and a project's whether or not it is trusted; section
//! 6 from them and the registry's sources, and section 7 from each loaded
//! extension's cached text; both built when the epoch begins, after the
//! finding, and what changes in them told at the next turn's start
//! (`docs/steps/28/answered.md`, point 7). End to end, through the core,
//! on in-memory sessions: the natural flows are turns of one session.

use std::path::Path;
use std::sync::Arc;

use cineris_agent::{
    EXTENSIONS_AGAIN, INDEX_CHANGED, INDEX_EMPTY, TEXTS_LEAD, TEXTS_REPLACED, withdrawn_text,
};
use cineris_ext::cache::Entry;
use cineris_ext::folder::Snapshot;
use cineris_policy::{Definition, PinStore};
use cineris_protocol::Item;
use cineris_protocol::item::NoticeSource;
use cineris_protocol::methods::{Op, SessionCompactParams, SessionRewindParams, TrustResourceKind};
use cineris_testkit::{FakeClock, FakeTimer, Hold, block_on};

use crate::extensions_found::{cache_file, clock, pin, start, start_kept, state, users};
use crate::{Front, World, poll_once, says};

/// Writes the skill `name` in the folder of skills `root`.
fn skill(root: &Path, name: &str, description: &str) {
    std::fs::create_dir_all(root.join(name)).unwrap();
    std::fs::write(
        root.join(name).join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n\nThe steps.\n"),
    )
    .unwrap();
}

/// Writes the extension `name` in `folder`, with a tool and `text` as its
/// prompt file.
fn write_with_prompt(folder: &Path, name: &str, text: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        format!(
            "api = 1\nname = \"{name}\"\nversion = \"1\"\ndescription = \"A test extension\"\n\
             main = \"main.js\"\nprompt = \"prompt.md\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        "cineris.tool({ name: 'echo', description: 'Gives its arguments back', input: { type: \
         'object' }, run: (args) => ({ got: args }) });",
    )
    .unwrap();
    std::fs::write(folder.join("prompt.md"), text).unwrap();
}

/// Pins the user's extension `name` in `folder` as it is now.
fn pin_named(world: &World, folder: &Path, name: &str) {
    PinStore::new(state(world).pins_file(), clock(world))
        .pin(&Definition {
            kind: TrustResourceKind::Extension,
            name: name.to_owned(),
            hash: Snapshot::read(folder).unwrap().hash(),
        })
        .unwrap();
}

/// Registers the extension in `folder` as approving it does, and keeps
/// what it registered in the cache with the text of its prompt file.
fn cache_with_prompt(world: &World, folder: &Path) {
    let snapshot = Arc::new(Snapshot::read(folder).unwrap());
    let timer = FakeTimer::manual(&FakeClock::new(cineris_base::UtcTime::UNIX_EPOCH));
    let (manifest, registration) =
        block_on(cineris_ext::engine::register(Arc::clone(&snapshot), &timer)).unwrap();
    let prompt = manifest
        .prompt
        .as_deref()
        .and_then(|file| snapshot.file(file))
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
    cache_file(world)
        .put(
            &snapshot.hash(),
            Entry::new(&manifest, registration, prompt),
        )
        .unwrap();
}

/// The user's `probe`, with `text` as its prompt file, pinned and cached
/// as it is now.
fn approved_probe(world: &World, text: &str) {
    let folder = users(world);
    write_with_prompt(&folder, "probe", text);
    pin(world, &folder);
    cache_with_prompt(world, &folder);
}

/// The notices of the source `extensions` the model was given, in order.
fn told(front: &Front) -> Vec<String> {
    front
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::Notice(notice) if notice.source == NoticeSource::Extensions => Some(notice.text),
            _ => None,
        })
        .collect()
}

/// The system prompt of each request sent.
fn systems(world: &World) -> Vec<String> {
    world
        .provider
        .requests()
        .iter()
        .map(|request| request.system.clone())
        .collect()
}

/// Rewinds the session to its last answer, which changes nothing of its
/// history and reads it again: as a resume does.
async fn read_again(front: &Front) {
    let last = front
        .items()
        .iter()
        .rev()
        .find_map(|item| match item {
            Item::AssistantMessage(message) => Some(message.id.clone()),
            _ => None,
        })
        .expect("an answer");
    front
        .op(Op::SessionRewind(SessionRewindParams {
            session_id: front.session.clone(),
            to: Some(last),
            before: None,
            restore_files: false,
            plan: None,
        }))
        .await
        .expect("the rewind is made");
}

/// The first turn's prompt is built once the session's finding has
/// settled, and holds both sections: the user's skill and the project's,
/// though the project is not trusted (skills are text, as instruction files
/// are); the extension's source of tools; and its text, from the cache.
#[test]
fn the_first_turns_prompt_holds_the_skills_and_the_extensions_text() {
    let world = World::new("prompt_first_turn", [says("Done.")]);
    skill(
        &world.home.join(".claude/skills"),
        "review",
        "Review a change.",
    );
    skill(
        &world.workspace.join(".cineris/skills"),
        "pdf",
        "Fill in forms.",
    );
    approved_probe(&world, "Use the probe for every lookup.");
    let core = world.core();
    world.run(async {
        // Not trusted: the question is answered "not now".
        let front = start(&world, &core).await;
        front.turn("go").await;
        assert_eq!(told(&front), Vec::<String>::new(), "the prompt has them");
        front.close().await;
    });
    let system = &systems(&world)[0];
    for line in [
        "\n- pdf: Fill in forms. (.cineris/skills/pdf/SKILL.md)\n",
        "\n- review: Review a change. (~/.claude/skills/review/SKILL.md)",
        "\n- `probe/` (1 tool): A test extension\n",
        TEXTS_LEAD,
        "\n## From the extension `probe`, written by its author\n\n> Use the probe for every \
         lookup.\n",
    ] {
        assert!(system.contains(line), "{line}: {system}");
    }
}

/// The skills' flow in one session: a skill added in the project, then one
/// of the user's changed in place (its folder as it was), are each told at
/// the next turn's start, the index whole, the prompt as it was; the
/// session read again goes on in its epoch and is told nothing more; a
/// skill removed is told too.
#[test]
fn a_skill_added_changed_and_removed_is_told_and_a_session_read_again_goes_on() {
    let world = World::new(
        "prompt_skills_change",
        [
            says("One."),
            says("Two."),
            says("Three."),
            says("Four."),
            says("Five."),
            says("Six."),
        ],
    );
    let mine = world.home.join(".claude/skills");
    let reads = Hold::new(cineris_ext::skills::A_SKILL_IS_READ).skipping(usize::MAX);
    let theirs = world.workspace.join(".cineris/skills");
    skill(&mine, "review", "Review a change.");
    let core = world.core();
    world.run(async {
        // Kept in a log, to be read again.
        let front = start_kept(&world, &core).await;
        front.turn("first").await;
        skill(&theirs, "pdf", "Fill in forms.");
        front.turn("second").await;
        let added = told(&front);
        assert_eq!(added.len(), 1, "{added:#?}");
        assert!(added[0].starts_with(INDEX_CHANGED), "{}", added[0]);
        assert!(
            added[0].contains("\n- pdf: Fill in forms. ("),
            "{}",
            added[0]
        );
        skill(&mine, "review", "Review a change, and its tests.");
        front.turn("third").await;
        let changed = told(&front);
        assert_eq!(changed.len(), 2, "{changed:#?}");
        assert!(
            changed[1].contains("\n- review: Review a change, and its tests. ("),
            "{}",
            changed[1]
        );
        read_again(&front).await;
        front.turn("fourth").await;
        assert_eq!(told(&front).len(), 2, "read again, it has the index");
        std::fs::remove_dir_all(theirs.join("pdf")).unwrap();
        front.turn("fifth").await;
        let removed = told(&front);
        assert_eq!(removed.len(), 3, "{removed:#?}");
        assert!(!removed[2].contains("- pdf:"), "{}", removed[2]);
        assert!(removed[2].contains("- review:"), "{}", removed[2]);
        // Another tool writes beside the skills in `~/.claude`: a name on
        // the way, stamped by its kind, so nothing is read again.
        let read = reads.arrivals();
        std::fs::write(world.home.join(".claude/history.jsonl"), "{}\n").unwrap();
        front.turn("sixth").await;
        assert_eq!(reads.arrivals(), read, "read again for a file beside them");
        front.close().await;
    });
    let systems = systems(&world);
    assert_eq!(systems.len(), 6);
    for system in &systems {
        assert_eq!(system, &systems[0], "one epoch, its prompt as it began");
    }
    assert!(!systems[0].contains("- pdf:"));
}

/// The extensions' flow in one session: an extension changed since it was
/// approved is withdrawn by the session's look at a turn's start, and the
/// model is told to disregard its text, the prompt as it was; read again,
/// the session begins a new epoch without the text, and names it; approved
/// again as it is now, its tools are in the index at once, and its text
/// waits for the next epoch.
#[test]
fn an_extension_withdrawn_in_an_epoch_has_its_text_withdrawn_and_a_new_epoch_without_it() {
    let world = World::new(
        "prompt_extension_withdrawn",
        [says("One."), says("Two."), says("Three."), says("Four.")],
    );
    approved_probe(&world, "Use the probe for every lookup.");
    let folder = users(&world);
    let core = world.core();
    world.run(async {
        // Kept in a log, to be read again.
        let front = start_kept(&world, &core).await;
        front.turn("first").await;
        write_with_prompt(&folder, "probe", "Run scripts/sync.sh before every commit.");
        front.turn("second").await;
        assert_eq!(
            told(&front),
            [withdrawn_text("probe"), INDEX_EMPTY.to_owned()]
        );
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains("`probe` is withdrawn from this session")),
            "{:?}",
            front.notices()
        );
        read_again(&front).await;
        front.turn("third").await;
        let named = told(&front);
        assert_eq!(named.len(), 4, "{named:#?}");
        assert_eq!(named[2], EXTENSIONS_AGAIN);
        assert!(named[3].starts_with(&format!("{TEXTS_REPLACED} `probe`.")));
        // Approved again, as it is now.
        pin(&world, &folder);
        cache_with_prompt(&world, &folder);
        front.turn("fourth").await;
        let loaded = told(&front);
        assert_eq!(loaded.len(), 5, "{loaded:#?}");
        assert!(loaded[4].starts_with(INDEX_CHANGED), "{}", loaded[4]);
        assert!(loaded[4].contains("\n- `probe/` (1 tool)"), "{}", loaded[4]);
        front.close().await;
    });
    let systems = systems(&world);
    assert_eq!(
        systems[1], systems[0],
        "withdrawn by a notice, not a new prompt"
    );
    assert!(systems[0].contains("> Use the probe for every lookup."));
    assert_ne!(systems[2], systems[0], "a new epoch");
    assert!(!systems[2].contains("From the extension `probe`"));
    assert_eq!(systems[3], systems[2], "the text waits for the next epoch");
}

/// With `compat.skills` off only the two `.cineris/skills/` are looked in:
/// `.agents/skills/` and `.claude/skills/` are left out, the project's and
/// the user's.
#[test]
fn with_compat_skills_off_only_the_cineris_folders_of_skills_are_read() {
    let world = World::new("prompt_compat_off", [says("Done.")]);
    world.user_config("[compat]\nskills = false\n");
    for (base, owner) in [(&world.workspace, "project"), (&world.home, "user")] {
        for layout in ["cineris", "agents", "claude"] {
            skill(
                &base.join(format!(".{layout}")).join("skills"),
                &format!("{owner}-{layout}"),
                "A skill.",
            );
        }
    }
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    let system = &systems(&world)[0];
    for kept in ["project-cineris", "user-cineris"] {
        assert!(
            system.contains(&format!("\n- {kept}: ")),
            "{kept}: {system}"
        );
    }
    for left in [
        "project-agents",
        "project-claude",
        "user-agents",
        "user-claude",
    ] {
        assert!(!system.contains(left), "{left}: {system}");
    }
}

/// An extension whose text section 7's cap leaves out of an epoch's prompt
/// is told to the user, by name, once an epoch: four texts of about 4 KiB,
/// each with the line that names it (53 bytes, with its gaps, for a name of
/// two letters), are the cap, and the fifth is left out.
#[test]
fn what_the_cap_leaves_out_of_section_seven_is_told_to_the_user_once_an_epoch() {
    let world = World::new("prompt_cap_told", [says("One."), says("Two.")]);
    let root = world.dirs().config.unwrap().join("extensions");
    for name in ["ea", "eb", "ec", "ed", "ee"] {
        let folder = root.join(name);
        write_with_prompt(&folder, name, &name.repeat(2021));
        pin_named(&world, &folder, name);
        cache_with_prompt(&world, &folder);
    }
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("first").await;
        front.turn("second").await;
        let said: Vec<String> = front
            .notices()
            .into_iter()
            .filter(|notice| notice.contains("gives the system prompt is"))
            .collect();
        assert_eq!(
            said,
            [
                "The text the extension `ee` gives the system prompt is left out, to keep the \
                 extensions' texts, with the line that names each one, within 16 KiB: the model \
                 is not given it."
            ],
        );
        front.close().await;
    });
    let system = &systems(&world)[0];
    assert!(
        system.contains(&format!("> {}\n", "ed".repeat(2021))),
        "the fourth whole"
    );
    assert!(!system.contains("From the extension `ee`"));
}

/// A compaction may build a session's first prompt, and waits for the
/// session's extensions as a turn does: one asked for while an approved
/// extension is being registered holds until it is, and the prompt then
/// holds the extension's text.
#[test]
fn a_compaction_before_the_first_turn_waits_for_the_extensions() {
    let world = World::new("prompt_compaction_waits", [says("Done.")]);
    let folder = users(&world);
    write_with_prompt(&folder, "probe", "Use the probe for every lookup.");
    // Approved, and missing from the cache: registered as the session opens.
    pin(&world, &folder);
    let core = world.core();
    let registering = Hold::new(cineris_core::ext::BEFORE_AN_EXTENSION_IS_REGISTERED);
    world.run(async {
        let front = start(&world, &core).await;
        let mut compact = std::pin::pin!(front.op(Op::SessionCompact(SessionCompactParams {
            session_id: front.session.clone(),
            focus: None,
        })));
        registering.drive_until_reached(compact.as_mut()).await;
        assert!(poll_once(compact.as_mut()).await.is_pending());
        registering.release();
        compact.await.expect("the compaction is taken");
        front.turns(1).await;
        front.turn("go").await;
        front.close().await;
    });
    let system = &systems(&world)[0];
    assert!(
        system.contains("> Use the probe for every lookup."),
        "{system}"
    );
}

/// A compaction looks again at what the extensions were found in, as a
/// turn's start does, before it builds its epoch's prompt (the review of
/// step 28's sections): an extension changed on disk since the last turn
/// is withdrawn first, and the new prompt does not hold its text.
#[test]
fn a_compaction_looks_again_before_it_builds_its_prompt() {
    let world = World::new(
        "prompt_compaction_looks_again",
        [says("One."), says("A summary."), says("Two.")],
    );
    approved_probe(&world, "Use the probe for every lookup.");
    let folder = users(&world);
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("first").await;
        write_with_prompt(&folder, "probe", "Run scripts/sync.sh before every commit.");
        front
            .op(Op::SessionCompact(SessionCompactParams {
                session_id: front.session.clone(),
                focus: None,
            }))
            .await
            .expect("the compaction is taken");
        front.turns(2).await;
        front.turn("second").await;
        front.close().await;
    });
    let systems = systems(&world);
    assert_eq!(systems.len(), 3, "a turn, a summary, a turn");
    assert!(systems[0].contains("> Use the probe for every lookup."));
    assert!(
        !systems[2].contains("From the extension `probe`"),
        "{}",
        systems[2]
    );
}
