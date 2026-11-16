//! Measurements for the rows of `docs/performance-budgets.md`: loading
//! both files, the look for a change, rendering section 5, one change to
//! the store, and what the observer adds to a `shell` call. Run by hand, in a release build:
//! `cargo xtask scratch-test`'s neighbour for a test that is checked in,
//! `cargo test --release -p cortex-knowledge --test it perf_ -- --ignored
//! --nocapture --test-threads 1`, or on a runner with `try`'s
//! `perf -p cortex-knowledge`.

use std::time::{Duration, Instant};

use cortex_knowledge::render;
use cortex_protocol::project::EntryAction;
use cortex_testkit::scratch;

use crate::{World, change};

/// A private store of `entries` entries, about a third of them commands,
/// and a shared file of `shared`, each of a realistic length.
fn fill(world: &World, entries: usize, shared: usize) {
    let mut store = String::from("version = 1\n");
    for index in 0..entries {
        if index % 3 == 0 {
            store.push_str(&format!(
                "[[entry]]\nid = \"k_{n}\"\nkind = \"command\"\ntext = \"The project's test command for part {index}.\"\n\
                 status = \"active\"\nbasis = \"observed\"\nevidence = \"Ran and succeeded twice.\"\n\
                 confirmed = \"2026-11-16T09:00:00Z\"\nexpires = \"2027-02-14T09:00:00Z\"\n\
                 accepted = \"2026-11-16T09:30:00Z\"\nseen = 7\n\
                 learned = {{ session = \"s_01JCX4T9Q2\", at = \"2026-11-16T09:00:00Z\" }}\n\
                 command = {{ purpose = \"test\", run = \"cargo nextest run -p part-{index} --no-fail-fast\", cwd = \"crates/part-{index}\", usual_ms = 4200 }}\n",
                n = index + 1
            ));
        } else {
            store.push_str(&format!(
                "[[entry]]\nid = \"k_{n}\"\nkind = \"convention\"\ntext = \"Convention {index}: every module says what it is for, in a sentence or two, before anything else, and names the page of the docs that specifies it.\"\n\
                 status = \"active\"\nbasis = \"proposed\"\nevidence = \"docs/README.md, line {index}\"\n\
                 confirmed = \"2026-11-16T09:00:00Z\"\nexpires = \"2027-01-15T09:00:00Z\"\n\
                 accepted = \"2026-11-16T09:30:00Z\"\n\
                 learned = {{ session = \"s_01JCX4T9Q2\", at = \"2026-11-16T09:00:00Z\" }}\n",
                n = index + 1
            ));
        }
    }
    world.write_store(&store);
    let mut team = String::from("version = 1\n");
    for index in 0..shared {
        team.push_str(&format!(
            "[[entry]]\nkind = \"fact\"\ntext = \"Shared fact {index}: the service in services/part-{index} owns its own schema, and nothing else writes to its tables.\"\n"
        ));
    }
    world.write_shared(&team);
}

fn timed<T>(runs: usize, mut run: impl FnMut() -> T) -> (Duration, Duration) {
    let mut times: Vec<Duration> = (0..runs)
        .map(|_| {
            let started = Instant::now();
            std::hint::black_box(run());
            started.elapsed()
        })
        .collect();
    times.sort();
    (times[runs / 2], times[runs * 95 / 100])
}

#[test]
#[ignore = "a measurement, run by hand"]
fn perf_loading_looking_and_rendering() {
    for (name, entries, shared) in [("typical", 40, 10), ("at the caps", 500, 200)] {
        let world = World::new(scratch!("perf_knowledge"));
        fill(&world, entries, shared);
        let project = world.project();
        let snapshot = project.load(None);
        assert_eq!(snapshot.problems(), Vec::<String>::new());
        assert_eq!(snapshot.entries().len(), entries + shared);
        let bytes = world.store_text().len() + world.shared_text().len();

        let (median, high) = timed(200, || project.load(None));
        println!(
            "{name}: load both files ({} KB, {} entries): median {median:?}, 95th {high:?}",
            bytes / 1024,
            entries + shared
        );
        let (median, high) = timed(2_000, || project.is_current(&snapshot));
        println!(
            "{name}: the look for a change, nothing changed: median {median:?}, 95th {high:?}"
        );
        let now = world.now();
        let (median, high) = timed(2_000, || render(&snapshot, now, 1_500));
        let section = render(&snapshot, now, 1_500);
        println!(
            "{name}: render section 5 at 1,500 tokens ({} bytes, {} shown, {} left out): median {median:?}, 95th {high:?}",
            section.text.len(),
            section.shown,
            section.left_out
        );
        let (median, high) = timed(200, || render(&snapshot, now, u32::MAX));
        println!("{name}: render every entry: median {median:?}, 95th {high:?}");
        let mut flip = false;
        let (median, high) = timed(40, || {
            flip = !flip;
            let action = if flip {
                EntryAction::Disable
            } else {
                EntryAction::Accept
            };
            project.update(&[change("k_2", action)], &[], None).unwrap()
        });
        println!(
            "{name}: one change (lock, read, write flushed, good copy, read back): median {median:?}, 95th {high:?}"
        );
    }
}

/// What the observer adds to a `shell` call, on the call's own path: the
/// watch's look at the line, and the snapshot's answer to "could this
/// change the store". Storing what was learned is off that path.
#[test]
#[ignore = "a measurement, run by hand"]
fn perf_observing_a_command() {
    use cortex_knowledge::{Ending, Run, Watch};

    for (name, entries, shared) in [("typical", 40, 10), ("at the caps", 500, 200)] {
        let world = World::new(scratch!("perf_observe"));
        fill(&world, entries, shared);
        let project = world.project();
        let snapshot = project.load(None);
        let now = world.now();
        let run = |command: &str| Run {
            command: command.to_owned(),
            cwd: Some("crates/part-0".to_owned()),
            ending: Ending::Succeeded,
            duration_ms: 4_000,
            changed: false,
            confined: true,
            read_only: false,
        };
        for (what, line) in [
            (
                "a line that is not a build, test or check",
                "git status --short",
            ),
            (
                "a line the observer does not take",
                "grep -rn \"fn main\" src | head -5",
            ),
            (
                "a command the store knows and confirmed today",
                "cargo nextest run -p part-0 --no-fail-fast",
            ),
            (
                "a command the store does not know",
                "cargo clippy --workspace --all-targets",
            ),
        ] {
            let run = run(line);
            let mut watch = Watch::new();
            let (median, high) = timed(20_000, || {
                watch
                    .see(&run, None)
                    .iter()
                    .filter(|learnt| snapshot.wants(learnt, now))
                    .count()
            });
            println!("{name}: observe {what}: median {median:?}, 95th {high:?}");
        }
    }
}

/// How large a file the cap on a file's size lets through, and what its
/// load costs: the cap is the size that still loads within the budget.
#[test]
#[ignore = "a measurement, run by hand"]
fn perf_a_store_at_the_size_cap() {
    let world = World::new(scratch!("perf_knowledge_cap"));
    // The longest entries the store takes, as many as fit under the cap.
    let mut store = String::from("version = 1\n");
    let mut count = 0;
    loop {
        let entry = format!(
            "[[entry]]\nid = \"k_{}\"\nkind = \"fact\"\ntext = \"{}\"\nstatus = \"active\"\nevidence = \"{}\"\n",
            count + 1,
            "t".repeat(600),
            "e".repeat(400)
        );
        if count == 500 || store.len() + entry.len() > 256 * 1024 {
            break;
        }
        store.push_str(&entry);
        count += 1;
    }
    world.write_store(&store);
    let project = world.project();
    assert_eq!(project.load(None).entries().len(), count);
    let (median, high) = timed(100, || project.load(None));
    println!(
        "a store of {} KB ({count} entries of the longest text): load median {median:?}, 95th {high:?}",
        store.len() / 1024
    );
}
