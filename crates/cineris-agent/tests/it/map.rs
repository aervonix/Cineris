//! The repository map in an epoch: given once, before the first prompt
//! when it is ready, at a safe boundary when it is late, fresh after a
//! compaction, and kept, not built again, by a resume
//! (`crates/cineris-agent/src/context/map.rs` has the rules).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cineris_agent::{ContextSettings, MapAnswer, MapPending, MapSource, RepoMap};
use cineris_protocol::item::NoticeSource;
use cineris_protocol::{Item, Message, Part, Role};
use cineris_testkit::FakeTimer;
use serde_json::json;

use crate::{Rig, Setup, World, calls, lock, says};

/// What the scripted source answers the next request with.
enum Answer {
    /// A map at once.
    Now(&'static str),
    /// No map.
    None,
    /// Later: the test sends it with [`ScriptedMap::send`].
    Later,
    /// Never: the answer is dropped, as a pass that panics drops it.
    Dropped,
}

/// A map source that answers as it is told, and remembers what it was
/// asked.
#[derive(Default)]
struct ScriptedMap {
    answers: Mutex<VecDeque<Answer>>,
    held: Mutex<Vec<MapAnswer>>,
    /// The touched files of each request.
    requests: Mutex<Vec<Vec<String>>>,
    started: AtomicUsize,
    ended: AtomicUsize,
}

impl ScriptedMap {
    fn new(answers: impl IntoIterator<Item = Answer>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().collect()),
            ..Self::default()
        })
    }

    fn send(&self, text: &str) {
        let answer = lock(&self.held).pop().expect("a request is waiting");
        answer.send(Some(text.to_owned()));
    }

    fn requests(&self) -> usize {
        lock(&self.requests).len()
    }
}

impl MapSource for ScriptedMap {
    fn request(&self, touched: Vec<String>) -> MapPending {
        lock(&self.requests).push(touched);
        match lock(&self.answers).pop_front().unwrap_or(Answer::None) {
            Answer::Now(text) => MapPending::ready(Some(text.to_owned())),
            Answer::None => MapPending::ready(None),
            Answer::Later => {
                let (answer, pending) = MapPending::channel();
                lock(&self.held).push(answer);
                pending
            }
            Answer::Dropped => MapPending::channel().1,
        }
    }

    fn turn_started(&self) {
        self.started.fetch_add(1, Ordering::SeqCst);
    }

    fn turn_ended(&self) {
        self.ended.fetch_add(1, Ordering::SeqCst);
    }
}

fn with_map(world: &World, source: &Arc<ScriptedMap>, timer: &FakeTimer, wait: Duration) -> Setup {
    let mut setup = Setup::new(world);
    setup.map = Some(RepoMap {
        source: Arc::clone(source) as Arc<dyn MapSource>,
        timer: Arc::new(timer.clone()),
        wait,
    });
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

fn map_items(rig: &Rig) -> Vec<String> {
    rig.store
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::Notice(notice) if notice.source == NoticeSource::Map => Some(notice.text),
            _ => None,
        })
        .collect()
}

#[test]
fn a_map_ready_in_time_goes_just_before_the_first_prompt_once() {
    let world = World::new("map_before_prompt", [says("One."), says("Two.")]);
    let source = ScriptedMap::new([Answer::Now("MAP")]);
    let timer = FakeTimer::auto(&world.clock);
    let rig = with_map(&world, &source, &timer, Duration::from_millis(150)).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    let items = rig.store.items();
    assert!(
        matches!(&items[0], Item::Notice(notice) if notice.source == NoticeSource::Map),
        "{items:#?}"
    );
    assert!(matches!(&items[1], Item::UserMessage(_)));
    assert_eq!(map_items(&rig), ["MAP"], "given once in the epoch");
    assert_eq!(source.requests(), 1);
    let first = &world.provider.requests()[0];
    assert_eq!(first.messages[0].role, Role::User);
    assert!(text(&first.messages[0]).ends_with("MAP"));
    assert!(text(&first.messages[0]).starts_with(cineris_agent::NOTICE_LEAD));
    assert!(text(&first.messages[1]).contains("first"));
    assert!(
        timer.sleeps().is_empty(),
        "a map that is ready waits for nothing"
    );
    assert_eq!(source.started.load(Ordering::SeqCst), 2);
    assert_eq!(source.ended.load(Ordering::SeqCst), 2);
}

#[test]
fn a_late_map_follows_at_the_next_boundary_after_a_round_never_inside_it() {
    let world = World::new(
        "map_late",
        [
            calls(&[("c1", "read", json!({"path": "a", "gate": "tool"}))]),
            says("Done."),
        ],
    );
    let source = ScriptedMap::new([Answer::Later]);
    let timer = FakeTimer::auto(&world.clock);
    let rig = with_map(&world, &source, &timer, Duration::from_millis(150)).start(&world);
    world.run(async {
        rig.prompt("go");
        world.log.has("start read a").await;
        // The map comes while the round runs.
        source.send("LATE MAP");
        world.gates.open("tool");
        rig.events.turns(1).await;
    });
    // The epoch's first chance waited its 150 ms, once, and gave up.
    assert_eq!(timer.sleeps(), [Duration::from_millis(150)]);
    let requests = world.provider.requests();
    assert!(
        requests[0]
            .messages
            .iter()
            .all(|message| !text(message).contains("LATE MAP"))
    );
    let second = &requests[1].messages;
    let roles: Vec<&Role> = second.iter().map(|message| &message.role).collect();
    assert_eq!(
        roles,
        [&Role::User, &Role::Assistant, &Role::Tool, &Role::User]
    );
    assert!(text(&second[3]).ends_with("LATE MAP"));
    crate::assert_valid_history(second);
}

#[test]
fn headless_waits_for_nothing_and_takes_the_map_when_it_is_ready() {
    let world = World::new("map_no_wait", [says("One."), says("Two.")]);
    let source = ScriptedMap::new([Answer::Later]);
    let timer = FakeTimer::manual(&world.clock);
    let rig = with_map(&world, &source, &timer, Duration::ZERO).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    assert!(timer.sleeps().is_empty());
    assert!(map_items(&rig).is_empty());
    source.send("MAP");
    world.run(async {
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    // Given at the next turn's start, before its prompt: one request asked.
    assert_eq!(map_items(&rig), ["MAP"]);
    assert_eq!(source.requests(), 1);
}

#[test]
fn a_source_with_no_map_is_not_asked_again_in_the_epoch() {
    let world = World::new("map_none", [says("One."), says("Two.")]);
    let source = ScriptedMap::new([Answer::None]);
    let timer = FakeTimer::auto(&world.clock);
    let rig = with_map(&world, &source, &timer, Duration::from_millis(150)).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    assert_eq!(source.requests(), 1);
    assert!(map_items(&rig).is_empty());
}

#[test]
fn a_compaction_gives_a_fresh_map_and_a_resume_keeps_it_as_the_history_was_sent() {
    let world = World::new(
        "map_compaction_resume",
        [
            says("One."),
            says("Two."),
            says("A summary."),
            says("Three."),
            says("After the resume."),
        ],
    );
    let source = ScriptedMap::new([Answer::Now("FIRST MAP"), Answer::Now("SECOND MAP")]);
    let timer = FakeTimer::auto(&world.clock);
    let mut setup = with_map(&world, &source, &timer, Duration::from_millis(150));
    setup.context = ContextSettings {
        keep_recent_tokens: 10_000,
        ..ContextSettings::default()
    };
    let rig = setup.start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    rig.handle.compact(None).expect("a compaction");
    world.run(async {
        rig.events.turns(3).await;
        rig.prompt("third");
        rig.events.turns(4).await;
    });
    assert_eq!(rig.store.summaries().len(), 1);
    assert_eq!(map_items(&rig), ["FIRST MAP", "SECOND MAP"]);
    let requests = world.provider.requests();
    let last_before = requests.last().expect("a request").clone();
    // The epoch after the compaction has its own map and not the old one,
    // though the turns it replays held it.
    let sent: String = last_before.messages.iter().map(text).collect();
    assert!(
        sent.contains("SECOND MAP") && !sent.contains("FIRST MAP"),
        "{sent}"
    );

    // Resumed: the history read back is the one sent, and no map is built.
    let resumed_source = ScriptedMap::new([Answer::Now("NOT ASKED")]);
    let mut again = with_map(&world, &resumed_source, &timer, Duration::from_millis(150));
    again.memory = rig.store.clone();
    let resumed = again.start(&world);
    world.run(async {
        resumed.prompt("fourth");
        resumed.events.turns(1).await;
    });
    assert_eq!(
        resumed_source.requests(),
        0,
        "the epoch's history holds its map"
    );
    let after = world.provider.requests().last().expect("a request").clone();
    assert!(after.messages.len() > last_before.messages.len());
    assert_eq!(
        &after.messages[..last_before.messages.len()],
        &last_before.messages[..],
        "the resumed history begins with the one sent before"
    );
}

#[test]
fn the_map_is_asked_for_with_the_files_the_sessions_tools_touched() {
    let world = World::new(
        "map_touched",
        [
            calls(&[("c1", "read", json!({"path": "src/a.rs", "record": true}))]),
            says("Read."),
            says("A summary."),
            says("Next."),
        ],
    );
    let source = ScriptedMap::new([Answer::Now("FIRST"), Answer::Now("SECOND")]);
    let timer = FakeTimer::auto(&world.clock);
    let rig = with_map(&world, &source, &timer, Duration::from_millis(150)).start(&world);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    rig.handle.compact(None).expect("a compaction");
    world.run(async {
        rig.events.turns(2).await;
        rig.prompt("next");
        rig.events.turns(3).await;
    });
    let requests = lock(&source.requests).clone();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert!(requests[0].is_empty());
    assert_eq!(requests[1], ["src/a.rs"]);
}

/// The independent review of step 18: the loop kept what it knew of the map
/// by the epoch's number, which comes round again after a rewind past a
/// compaction. Epoch 2's source said it had no map; after a rewind to epoch
/// 1 and another compaction, the new epoch 2 was taken to have none too. A
/// change of history forgets it all.
#[test]
fn a_rewind_past_a_compaction_forgets_what_the_epoch_of_its_number_knew() {
    let world = World::new(
        "map_rewind",
        [
            says("One."),
            says("A summary."),
            says("Two."),
            says("A summary again."),
            says("Three."),
        ],
    );
    let source = ScriptedMap::new([
        Answer::Now("FIRST MAP"),
        Answer::None,
        Answer::Now("THIRD MAP"),
    ]);
    let timer = FakeTimer::auto(&world.clock);
    let rig = with_map(&world, &source, &timer, Duration::from_millis(150)).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
    });
    // Where going back to the end of the first turn leaves the history.
    let back_to = rig.store.records().len();
    rig.handle.compact(None).expect("a compaction");
    world.run(async {
        rig.events.turns(2).await;
        rig.prompt("second");
        rig.events.turns(3).await;
    });
    assert_eq!(source.requests(), 2, "epoch 2's source had no map");
    assert_eq!(map_items(&rig), ["FIRST MAP"]);
    // Back in epoch 1, whose history holds its map.
    rig.store.keep_first(back_to);
    let mut change = rig.handle.change_history().expect("idle");
    change.made();
    drop(change);
    rig.handle.compact(None).expect("a compaction again");
    world.run(async {
        rig.events.turns(4).await;
        rig.prompt("third");
        rig.events.turns(5).await;
    });
    assert_eq!(
        source.requests(),
        3,
        "the new epoch 2 is asked for its own map"
    );
    assert_eq!(map_items(&rig), ["FIRST MAP", "THIRD MAP"]);
}

/// A source that drops its answer (a pass that panicked) has not said
/// there is no map: a later boundary asks again, and the epoch has its map
/// (the independent review of step 18).
#[test]
fn a_dropped_answer_is_asked_again_at_a_later_boundary() {
    let world = World::new("map_dropped", [says("One."), says("Two.")]);
    let source = ScriptedMap::new([Answer::Dropped, Answer::Now("THE MAP")]);
    let timer = FakeTimer::auto(&world.clock);
    let rig = with_map(&world, &source, &timer, Duration::from_millis(150)).start(&world);
    world.run(async {
        rig.prompt("first");
        rig.events.turns(1).await;
        rig.prompt("second");
        rig.events.turns(2).await;
    });
    assert_eq!(source.requests(), 2);
    assert_eq!(map_items(&rig), ["THE MAP"]);
}

/// A source that drops every answer is asked a few times in an epoch, not
/// at every boundary for ever.
#[test]
fn a_source_that_drops_every_answer_is_asked_a_bounded_number_of_times() {
    let world = World::new(
        "map_dropped_always",
        [says("One."), says("Two."), says("Three."), says("Four.")],
    );
    let source = ScriptedMap::new((0..10).map(|_| Answer::Dropped));
    let timer = FakeTimer::auto(&world.clock);
    let rig = with_map(&world, &source, &timer, Duration::from_millis(150)).start(&world);
    world.run(async {
        for (turn, prompt) in ["one", "two", "three", "four"].into_iter().enumerate() {
            rig.prompt(prompt);
            rig.events.turns(turn + 1).await;
        }
    });
    assert_eq!(source.requests(), 3);
    assert!(map_items(&rig).is_empty());
}
