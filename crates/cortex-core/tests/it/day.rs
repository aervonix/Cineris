//! The day's limits across processes (found by the independent review of
//! step 16: the day's account counted its own core's spending only), and
//! refused with statistics off (the owner's decision, 2026-11-15).

use cortex_core::ClientInfo;
use cortex_protocol::event::TurnOutcome;
use cortex_protocol::session::SessionSettings;

use crate::{Front, World, says};

/// A request line of the world's day, as a build before the writer was
/// recorded writes one: it has no `writer`, so it is another process's.
const OLD_BUILDS_LINE: &str = r#"{"day":"2026-11-13","kind":"request","at":"2026-11-13T09:00:00Z","session":"s_old","turn":"t_1","provider":"anthropic","model":"opus","role":"main","variants":{"edit_format":"replace","tool_schema":"strict","prompt_style":"concise"},"tokens":{"input":10,"output":5},"billing":"metered","stop":"end"}"#;

fn stats_dir(world: &World) -> std::path::PathBuf {
    world.dirs().state.unwrap().join("stats")
}

fn utc(world: &World) -> cortex_core::Core {
    world.core_with(|options| options.zone = Some(cortex_base::Zone::utc()))
}

/// Two cores on one state directory are two processes. Each opens its
/// session before the other spends anything, and the second is still held
/// to the day's one request by the first's.
#[test]
fn two_processes_together_hold_the_day_to_its_limit() {
    let world = World::new("two_processes_day", [says("One."), says("Not sent.")]);
    world.user_config("[limits.day]\nrequests = 1\n");
    let first = utc(&world);
    let second = utc(&world);
    world.run(async {
        let a = Front::start(&world, &first).await;
        let b = Front::start(&world, &second).await;
        let one = a.turn("one").await;
        assert_eq!(one.outcome, TurnOutcome::Completed, "{:?}", one.error);
        // The first process ends, its statistics written.
        a.close().await;
        first.shutdown().await;
        let two = b.turn("two").await;
        assert_eq!(
            two.outcome,
            TurnOutcome::Limit,
            "a day's limit of one request let a second through, from the other process"
        );
        assert_eq!(two.error.unwrap().code, "request_limit");
        b.close().await;
    });
    assert_eq!(world.provider.requests().len(), 1);
}

/// Another process's line is counted though it came from a build that did
/// not say which recorder wrote it.
#[test]
fn a_line_an_older_build_wrote_counts_as_another_processs() {
    let world = World::new("old_builds_line", [says("Not sent.")]);
    world.user_config("[limits.day]\nrequests = 1\n");
    let core = utc(&world);
    world.run(async {
        let front = Front::start(&world, &core).await;
        let dir = stats_dir(&world);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("events-2026-11.jsonl"),
            format!("{OLD_BUILDS_LINE}\n"),
        )
        .unwrap();
        let end = front.turn("go").await;
        assert_eq!(end.outcome, TurnOutcome::Limit, "{:?}", end.error);
        front.close().await;
    });
    assert_eq!(world.provider.requests().len(), 0);
}

/// This process's own spending is in its purse as it happens, and its own
/// lines of it are not read back: two requests of a limit of two are made.
#[test]
fn a_processs_own_spending_is_not_counted_twice() {
    let world = World::new("own_not_twice", [says("One."), says("Two.")]);
    world.user_config("[limits.day]\nrequests = 2\n");
    let core = utc(&world);
    let file = stats_dir(&world).join("events-2026-11.jsonl");
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        // The writer has the first request's line on the disk before the
        // second request reads the files.
        // (The writer is a thread of its own, which this waits for; a
        // writer that never wrote it is stopped by the test's time limit.)
        while !std::fs::read_to_string(&file).is_ok_and(|text| text.contains(r#""kind":"request""#))
        {
            std::thread::yield_now();
        }
        let two = front.turn("two").await;
        assert_eq!(two.outcome, TurnOutcome::Completed, "{:?}", two.error);
        front.close().await;
    });
    assert_eq!(world.provider.requests().len(), 2);
}

/// With statistics off there is nothing to count another process's
/// spending from: a session with a day's limit, the user's or a project's,
/// is refused at open, and nothing is read, pruned or written (the
/// independent review of step 16 found the old monthly file deleted and
/// the aggregates written).
#[test]
fn a_days_limit_with_statistics_off_is_refused_at_open() {
    let world = World::new("day_stats_off", []);
    let dir = stats_dir(&world);
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("events-2025-01.jsonl");
    std::fs::write(&old, format!("{OLD_BUILDS_LINE}\n")).unwrap();
    for (user, project) in [
        (
            "[stats]\nenabled = false\nretention = 1\n\n[limits.day]\nrequests = 5\n",
            "",
        ),
        (
            "[stats]\nenabled = false\nretention = 1\n",
            "[limits.day]\ncost = 1.0\n",
        ),
    ] {
        world.user_config(user);
        world.project_config(project);
        let core = utc(&world);
        let refused = world.run(Front::start_as(
            &world,
            &core,
            ClientInfo::user(),
            SessionSettings::default(),
        ));
        let Err(error) = refused else {
            panic!("a session with a day's limit opened with statistics off");
        };
        assert_eq!(error.code, "day_limit_needs_stats");
        assert!(error.message.contains("stats.enabled"), "{}", error.message);
        // `cortex stats` still works.
        world.run(core.stats_summary()).unwrap();
    }
    assert!(old.exists(), "nothing was pruned");
    assert!(!dir.join("aggregates.json").exists(), "nothing was written");
}

/// A request waits for what other processes spent at most a second: a
/// disk that does not answer (here a FIFO with nobody at its other end
/// stands in for one) holds it that long, and it then goes on with the
/// day's spending as last read.
#[cfg(target_os = "linux")]
#[test]
fn a_stalled_disk_holds_a_days_request_at_most_a_second() {
    use std::time::Duration;

    use cortex_testkit::join;

    let world = World::new("day_stalled", [says("Sent.")]);
    world.user_config("[limits.day]\nrequests = 5\n");
    let core = utc(&world);
    world.run(async {
        let front = Front::start(&world, &core).await;
        let dir = stats_dir(&world);
        std::fs::create_dir_all(&dir).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(dir.join("events-2026-11.jsonl"))
            .status()
            .unwrap();
        assert!(made.success());
        let (end, ()) = join(front.turn("go"), async {
            while !world.timer.running().contains(&Duration::from_secs(1)) {
                world.timer.sleeping().await;
                tokio::task::yield_now().await;
            }
            world.timer.advance(Duration::from_secs(1));
        })
        .await;
        assert_eq!(end.outcome, TurnOutcome::Completed, "{:?}", end.error);
        assert!(world.timer.sleeps().contains(&Duration::from_secs(1)));
        front.close().await;
    });
    assert_eq!(world.provider.requests().len(), 1);
}
