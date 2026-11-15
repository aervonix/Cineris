//! The repository index through the core: never on the start-up path, the
//! map in an epoch, and the outlines behind `outline` and `read`
//! (`docs/architecture/15-efficiency.md`, "Repository map").

use std::sync::Arc;
use std::time::Duration;

use cortex_base::AbsPath;
use cortex_protocol::Item;
use cortex_protocol::item::NoticeSource;
use cortex_testkit::Untouched;
use serde_json::json;

use crate::{Front, World, calls, says, texts};

/// How long a test waits for the index's first touch: a bound on a
/// mistake, which no test that passes waits out.
const LIMIT: Duration = Duration::from_secs(5);

fn map_notices(front: &Front) -> Vec<String> {
    front
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::Notice(notice) if notice.source == NoticeSource::Map => Some(notice.text),
            _ => None,
        })
        .collect()
}

/// The start-up file-access check for the index: a gated index has done
/// nothing when the session opens, and its first request goes without a
/// map it would have had to wait for.
#[test]
fn the_index_is_not_touched_until_the_session_is_open() {
    let world = World::new("index_untouched", [says("Done.")]);
    world.put("src/lib.rs", "pub fn hello() {}\n");
    let watch = Arc::new(Untouched::new());
    let touch = Arc::clone(&watch);
    let core = world.core_with(|options| {
        options.repo_index = true;
        options.map_wait = Duration::ZERO;
        options.index_gate = Some(Arc::new(move |_: &AbsPath| {
            touch.touch();
            true
        }));
    });
    world.run(async {
        // Opening the session would wait at the watch if it used the index.
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        let first = world.provider.requests().remove(0);
        assert!(
            first
                .messages
                .iter()
                .all(|message| !texts(message).contains("Repository map")),
            "the first request did not wait for a map: {:#?}",
            first.messages
        );
    });
    watch.release();
    assert!(
        watch.wait_touched(LIMIT),
        "the index started after the session opened"
    );
    assert_eq!(watch.early(), 0, "the start-up path touched the index");
}

#[test]
fn the_first_prompt_waits_for_the_map_and_it_comes_just_before_the_prompt() {
    let world = World::new("index_map_first", [says("Done.")]);
    world.put("src/lib.rs", "pub struct Engine;\npub fn start() {}\n");
    world.put("README.md", "# App\n");
    let core = world.core_with(|options| options.repo_index = true);
    world.run(async {
        let front = Front::start(&world, &core).await;
        // The wait is on the fake timer, which nothing moves: the map comes
        // because the pass ends, not because the wait does.
        front.turn("go").await;
        let notices = map_notices(&front);
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(
            notices[0].contains("src/lib.rs: struct Engine, fn start"),
            "{}",
            notices[0]
        );
        // Nothing of the machine: no absolute path.
        assert!(!notices[0].contains(world.workspace.to_str().unwrap()));
        let first = world.provider.requests().remove(0);
        assert!(texts(&first.messages[0]).contains("Repository map"));
        assert!(texts(&first.messages[1]).ends_with("go"));
    });
    // The cache is the project's, in the state directory, and private.
    let state = world.dirs().state.unwrap();
    let projects: Vec<_> = std::fs::read_dir(state.join("projects"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(projects.len(), 1);
    let cache = projects[0]
        .path()
        .join("index")
        .join(cortex_index::CACHE_FILE);
    assert!(cortex_base::private::is_private(&cache).unwrap());
}

#[test]
fn the_outline_tool_and_a_large_read_have_the_indexs_outlines() {
    let big: String = (0..2000).map(|n| format!("fn f{n}() {{}}\n")).collect();
    let world = World::new(
        "index_outline",
        [
            calls(&[
                ("c1", "outline", json!({"path": "src/lib.rs"})),
                ("c2", "outline", json!({"path": "src"})),
                ("c3", "read", json!({"path": "src/big.rs"})),
            ]),
            says("Done."),
        ],
    );
    world.put(
        "src/lib.rs",
        "pub struct Engine;\n\nimpl Engine {\n    pub fn start(&self) {}\n}\n",
    );
    world.put("src/big.rs", &big);
    let core = world.core_with(|options| {
        options.repo_index = true;
        options.map_wait = Duration::ZERO;
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("look").await;
        let results = front.results();
        assert_eq!(results.len(), 3, "{results:#?}");
        assert!(
            results.iter().all(|(_, outcome, _)| outcome == "success"),
            "{results:#?}"
        );
        // The three ran together: found by what they say, not by order.
        let said = |start: &str| {
            results
                .iter()
                .find(|(_, _, text)| text.starts_with(start))
                .map(|(_, _, text)| text.clone())
                .unwrap_or_else(|| panic!("no result starts with {start:?}: {results:#?}"))
        };
        assert_eq!(
            said("src/lib.rs ("),
            "src/lib.rs (rust, 3 definitions; approximate):\n1  pub struct Engine\n3  impl \
             Engine\n4    pub fn start(&self)"
        );
        let directory = said("Outlines of the files in `src`");
        assert!(
            directory.contains("src/lib.rs:\n  1  pub struct Engine"),
            "{directory}"
        );
        assert!(
            directory.contains("[971 more definitions: outline the file itself.]"),
            "{directory}"
        );
        let read = said("src/big.rs (2,000 lines,");
        assert!(
            read.contains("Its outline, with line numbers:\n   1  fn f0()"),
            "{read}"
        );
    });
}

/// With no map, a session makes nothing ahead: no background pass, no
/// cache (the review of step 18).
#[test]
fn with_the_map_off_no_pass_runs_in_the_background() {
    let world = World::new("index_map_off", [says("Done.")]);
    world.put("src/lib.rs", "pub fn hello() {}\n");
    world.user_config("[index]\nmap = false\n");
    let watch = Arc::new(Untouched::new());
    let touch = Arc::clone(&watch);
    let core = world.core_with(|options| {
        options.repo_index = true;
        options.map_wait = Duration::ZERO;
        options.index_gate = Some(Arc::new(move |_: &AbsPath| {
            touch.touch();
            true
        }));
    });
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        assert!(map_notices(&front).is_empty());
        front.close().await;
    });
    watch.release();
    assert_eq!(watch.touches(), 0, "nothing was indexed");
    let state = world.dirs().state.unwrap();
    assert!(
        !state.join("projects").exists(),
        "no cache was written under {}",
        state.display()
    );
}
