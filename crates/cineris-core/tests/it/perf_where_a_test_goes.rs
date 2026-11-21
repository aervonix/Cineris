//! A throwaway measurement, not to be merged: where an extension test's
//! time goes on a hosted Windows runner.

use std::time::{Duration, Instant};

use cineris_core::{ClientInfo, Outgoing};
use cineris_protocol::methods::{Op, OpResult, SessionNewParams};
use cineris_protocol::session::SessionSettings;

use crate::extensions_found::{cache, pin, searches_times, users, write_extension};
use crate::{Answering, Front, World};

fn median(mut times: Vec<Duration>) -> f64 {
    if times.is_empty() {
        return 0.0;
    }
    times.sort();
    times[times.len() / 2].as_secs_f64() * 1e3
}

async fn open_ephemeral(world: &World, core: &cineris_core::Core) -> Front {
    let client = core.connect(ClientInfo::user());
    let result = core
        .handle(
            client.id,
            Op::SessionNew(SessionNewParams {
                cwd: None,
                workspace: world.workspace.to_str().unwrap().to_owned(),
                settings: SessionSettings::default(),
                title: None,
                config: serde_json::Map::new(),
                ephemeral: true,
            }),
        )
        .await;
    let Ok(OpResult::SessionNew(result)) = result else {
        panic!("{result:?}");
    };
    let read: Vec<Outgoing> = Vec::new();
    Front {
        core: core.clone(),
        client,
        session: result.session.session_id.clone(),
        state: result.session,
        seen: std::cell::RefCell::new(read),
        answered: std::cell::RefCell::new(Vec::new()),
        answering: std::cell::RefCell::new(Answering::default()),
    }
}

#[test]
#[ignore = "a measurement"]
fn perf_where_an_extension_tests_time_goes() {
    let names = [
        "world",
        "write_extension",
        "pin",
        "cache",
        "core",
        "open",
        "turn_1",
        "turn_2",
        "close",
        "drop_world",
        "first_session",
    ];
    for variant in ["logged", "ephemeral", "no_extension", "same_core", "reset"] {
        let mut times: Vec<Vec<Duration>> = vec![Vec::new(); names.len()];
        for round in 0..7 {
            let mut at = Instant::now();
            let mut lap = |index: usize, times: &mut Vec<Vec<Duration>>| {
                times[index].push(at.elapsed());
                at = Instant::now();
            };
            let world = World::new(&format!("perf_where_{variant}_{round}"), searches_times(4));
            if variant == "reset" {
                // What putting a world back as it was costs: everything in
                // it removed, and its home, workspace and configuration
                // made again.
                write_extension(&users(&world), "", "Gives its arguments back");
                pin(&world, &users(&world));
                let start = Instant::now();
                for entry in std::fs::read_dir(&world.dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        std::fs::remove_dir_all(&path).unwrap();
                    } else {
                        std::fs::remove_file(&path).unwrap();
                    }
                }
                std::fs::create_dir_all(&world.home).unwrap();
                std::fs::create_dir_all(&world.workspace).unwrap();
                world.user_config("");
                times[0].push(start.elapsed());
                continue;
            }
            lap(0, &mut times);
            if variant != "no_extension" {
                write_extension(&users(&world), "", "Gives its arguments back");
            }
            lap(1, &mut times);
            if variant != "no_extension" {
                pin(&world, &users(&world));
            }
            lap(2, &mut times);
            if variant != "no_extension" {
                cache(&world, &users(&world));
            }
            lap(3, &mut times);
            let core = world.core();
            lap(4, &mut times);
            if variant == "same_core" {
                // A first session of the core, unmeasured; then the second.
                world.run(async {
                    let front = open_ephemeral(&world, &core).await;
                    front.turn("look for the probe").await;
                    front.close().await;
                });
                lap(10, &mut times);
            }
            world.run(async {
                let front = if variant == "ephemeral" || variant == "same_core" {
                    open_ephemeral(&world, &core).await
                } else {
                    Front::start(&world, &core).await
                };
                lap(5, &mut times);
                front.turn("look for the probe").await;
                lap(6, &mut times);
                front.turn("look for the probe").await;
                lap(7, &mut times);
                front.close().await;
                lap(8, &mut times);
            });
            drop(core);
            drop(world);
            lap(9, &mut times);
        }
        for (name, took) in names.iter().zip(times) {
            cineris_testkit::numbers::report(&format!("where.{variant}.{name}"), median(took), "ms");
        }
    }
}
