//! What the server adds to a request: its round trip through a connection,
//! over the transport's in-memory ends and over the local socket (a named
//! pipe on Windows), for `docs/performance-budgets.md`, "Latency the
//! harness adds".
//!
//! A measurement, not a blocking test. It is ignored by default and run by
//! hand in a release build:
//!
//! ```text
//! cargo test --release -p cortex-rpc --test it perf_ -- --ignored --nocapture
//! ```
//!
//! or on a runner, `gh workflow run try -f command="perf -p cortex-rpc"`.
//! `CORTEX_PERF_ROUNDS` sets the number of requests (2,000 unless given).

use std::time::{Duration, Instant};

use cortex_protocol::methods::{Empty, Op};
use cortex_rpc::{ListenOptions, Listener, Place, Transport};
use cortex_testkit::conformance::{Carrier, Lines};
use cortex_testkit::numbers::report;

use crate::{CoreThread, Wire, World};

fn rounds() -> usize {
    std::env::var("CORTEX_PERF_ROUNDS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(2_000)
}

/// The median and the 99th percentile of `times`, in microseconds.
fn spread(mut times: Vec<Duration>) -> (f64, f64) {
    times.sort();
    let at = |share: f64| {
        let index = ((times.len() as f64 - 1.0) * share).round() as usize;
        times[index].as_secs_f64() * 1e6
    };
    (at(0.5), at(0.99))
}

/// Each of `rounds` calls of a read (`models/list`), timed from the line's
/// write to its result's read.
fn timed(lines: &mut Lines, rounds: usize) -> Vec<Duration> {
    // Warmed first: the first call builds what the rest reuse.
    for _ in 0..20 {
        lines.call(Op::ModelsList(Empty {})).expect("models/list");
    }
    (0..rounds)
        .map(|_| {
            let began = Instant::now();
            lines.call(Op::ModelsList(Empty {})).expect("models/list");
            began.elapsed()
        })
        .collect()
}

/// A client's lines over a transport a connect gave.
fn lines_over(transport: Transport) -> Lines {
    let Transport { mut input, output } = transport;
    let mut held: Vec<u8> = Vec::new();
    Lines::new(
        move |bytes| {
            let _ = output.blocking_send(bytes);
        },
        move || loop {
            if let Some(end) = held.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = held.drain(..=end).collect();
                return Some(line);
            }
            match input.blocking_recv() {
                Some(Ok(chunk)) => held.extend(chunk),
                Some(Err(_)) | None => return None,
            }
        },
    )
}

/// The place the test serves at: a socket in a folder of its own, or a
/// pipe named for the process.
fn place(world: &World) -> Place {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let folder = world.root.join("perf");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
        Place::Path(folder.join("rpc.sock"))
    }
    #[cfg(windows)]
    {
        let _ = world;
        Place::named(&format!("cortex-harness-perf-{}", std::process::id()))
    }
}

fn serve(core: &CoreThread, listener: Listener, world: &World) -> cortex_base::CancelToken {
    let stop = cortex_base::CancelToken::new();
    let (server, stopping, options) = (core.core.clone(), stop.clone(), world.serve_options());
    core.handle.spawn(async move {
        cortex_rpc::serve_socket(
            server,
            listener,
            options,
            64,
            &stopping,
            &cortex_base::CancelToken::new(),
        )
        .await;
    });
    stop
}

#[test]
#[ignore = "a measurement, run by hand in a release build: see the module documentation"]
fn perf_a_requests_round_trip_through_the_server() {
    let rounds = rounds();
    let world = World::new("perf-round-trip");
    let core = world.core();

    let mut wire = Wire::connect(&core, world.serve_options());
    let (median, high) = spread(timed(&mut wire.lines, rounds));
    report("rpc.round_trip.in_memory.median", median, "us");
    report("rpc.round_trip.in_memory.p99", high, "us");

    let place = place(&world);
    let listener = Listener::bind(&place, &ListenOptions::new(false)).expect("the listener");
    let stop = serve(&core, listener, &world);
    let (connected, reached) = std::sync::mpsc::channel();
    let at = place.clone();
    core.handle.spawn(async move {
        let _ = connected.send(cortex_rpc::socket::connect(&at).await);
    });
    let transport = reached.recv().unwrap().expect("the server is reached");
    let mut lines = lines_over(transport);
    lines
        .initialize(cortex_testkit::conformance::initialize())
        .expect("initialize");
    let (median, high) = spread(timed(&mut lines, rounds));
    report("rpc.round_trip.socket.median", median, "us");
    report("rpc.round_trip.socket.p99", high, "us");
    drop(lines);
    stop.cancel();
}
