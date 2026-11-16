//! `cortex serve --stdio`: the RPC server, for the process that started it
//! (`docs/architecture/11-rpc-protocol.md`, "Transports";
//! `docs/steps/27/answered.md`, point 4).
//!
//! **Threads.** The core runs on a thread of its own, on a current-thread
//! Tokio runtime with its timer and its I/O driver on, as the other modes'
//! do, and the connection's reader, lane and writer are tasks of it
//! (`cortex_rpc`). Standard input and output are read and written by two
//! threads of their own that are not joined, so a pipe that never closes
//! cannot hold the end. The main thread watches for signals.
//!
//! **Standard output carries only the protocol.** Nothing in this mode
//! prints: the log goes to standard error (at `warn`, off with `--quiet`)
//! and to the log file, and a failure to start is said on standard error.
//!
//! **The end.** When standard input ends, when the client asks for
//! `shutdown`, or at the first signal, the server disconnects its client,
//! shuts the core down, waits for the requests still running and writes
//! what is queued, each for a bounded time, and exits: 0, or 130 or 143 for
//! a signal. A second signal ends the process at once; the guard of process
//! trees and the Job Object end its commands. This is headless mode's table
//! of signals with its second step left out: a server has no one turn to
//! interrupt now.

use std::future::Future;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cortex_base::{CancelToken, SharedTimer};
use cortex_config::{Env, LoadOptions};
use cortex_core::{Core, TokioTimer};
use cortex_rpc::{ServeOptions, Transport};
use tokio::sync::{mpsc, oneshot};

use crate::args::Serve;
use crate::exit;
use crate::signals::{self, Interrupt};

/// How long the core thread is given from the start of the server's end:
/// its own waits are each bounded, and `Core::shutdown` waits for each
/// session's close, which is not. Past it the process ends without it, as
/// the interactive mode's end does.
const END: Duration = Duration::from_secs(5);

/// How long the end waits for the last lines to reach standard output.
const LAST_LINES: Duration = Duration::from_secs(1);

/// Runs the server. Returns the exit code.
pub(crate) fn run(serve: Serve, name: &'static str) -> u8 {
    if let Some(flag) = serve.run_only() {
        eprintln!("{name}: {flag} is for a run; `serve` does not take it.");
        return exit::USAGE;
    }
    let env = Env::from_process();
    let log = crate::headless::start_log(&serve.run, &env, name);
    let mut load = LoadOptions::new(env.clone());
    load.profile.clone_from(&serve.run.common.profile);
    load.overrides = crate::headless::overrides(&serve.run);
    let loaded = match cortex_config::load(&load) {
        Ok(loaded) => loaded,
        Err(failure) => {
            eprintln!("{name}: {}", crate::visible(&failure.render(&env)));
            return exit::USAGE;
        }
    };
    let mut options =
        match crate::interactive::core_options(&serve.run, &env, &loaded, log.as_ref()) {
            Ok(options) => options,
            Err(message) => {
                eprintln!("{name}: {}", crate::visible(&message));
                return exit::USAGE;
            }
        };
    let rpc = &loaded.settings.rpc;
    // `"0s"` is no limit: the approval waits for as long as it takes.
    options.approval_timeout = (!rpc.approval_timeout.is_zero()).then_some(rpc.approval_timeout);
    let stalled_client = rpc.stalled_client;

    let stop = CancelToken::new();
    let ending = CancelToken::new();
    let (done, finished) = oneshot::channel::<u8>();
    let (core_stop, core_ending) = (stop.clone(), ending.clone());
    // Not joined: past its bound the process ends without it.
    let core_thread = std::thread::Builder::new()
        .name("cortex-core".to_owned())
        .spawn(move || {
            let code = core_thread(options, stalled_client, &core_stop, &core_ending, name);
            let _ = done.send(code);
        });
    if let Err(error) = core_thread {
        eprintln!("{name}: the core thread could not be started: {error}");
        return exit::INTERNAL;
    }

    // The main thread watches for signals until the core thread is done.
    let core_done = Arc::new(AtomicBool::new(false));
    let (interrupts, interrupted) = mpsc::unbounded_channel::<Interrupt>();
    let signalled = std::sync::Mutex::new(None);
    let code = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map(|runtime| {
            runtime.block_on(watch(
                finished,
                interrupts,
                interrupted,
                &core_done,
                (&stop, &ending),
                &signalled,
            ))
        });
    if let Some(log) = log {
        log.flush();
    }
    let signalled = *signalled
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match (code, signalled) {
        (_, Some(code)) => code,
        (Ok(Some(code)), None) => code,
        // The core thread ended without a code: it panicked.
        (Ok(None), None) => exit::INTERNAL,
        (Err(error), None) => {
            tracing::warn!(%error, "signals are not watched: their runtime did not start");
            exit::INTERNAL
        }
    }
}

/// Waits for the core thread to be done, watching for signals meanwhile:
/// the first stops the server, a second ends the process at once. Gives
/// the core thread at most [`END`] once the server's end has begun (`stop`
/// or `ending`), and then the exit code it would have given.
async fn watch(
    mut finished: oneshot::Receiver<u8>,
    interrupts: mpsc::UnboundedSender<Interrupt>,
    mut interrupted: mpsc::UnboundedReceiver<Interrupt>,
    core_done: &Arc<AtomicBool>,
    (stop, ending): (&CancelToken, &CancelToken),
    signalled: &std::sync::Mutex<Option<u8>>,
) -> Option<u8> {
    let mut watch = std::pin::pin!(signals::watch(interrupts, Arc::clone(core_done)));
    let mut watching = true;
    let mut began = std::pin::pin!(async {
        poll_either(stop.cancelled(), ending.cancelled()).await;
        tokio::time::sleep(END).await;
    });
    std::future::poll_fn(|context| {
        if let std::task::Poll::Ready(code) = std::pin::Pin::new(&mut finished).poll(context) {
            core_done.store(true, Ordering::SeqCst);
            return std::task::Poll::Ready(code.ok());
        }
        if began.as_mut().poll(context).is_ready() {
            tracing::warn!("the core did not end in time: the process ends without it");
            return std::task::Poll::Ready(Some(exit::COMPLETED));
        }
        while let std::task::Poll::Ready(Some(interrupt)) = interrupted.poll_recv(context) {
            let code = interrupt.signal.code();
            let mut first = signalled
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if first.is_some() {
                // The second signal: the end it asked for is taking too
                // long for the person who sent it.
                tracing::warn!(code, "exiting at once, on a second signal");
                #[expect(
                    clippy::exit,
                    reason = "a second signal ends the server at once, as documented; the guard \
                              and the Job Object end its commands"
                )]
                std::process::exit(i32::from(code));
            }
            *first = Some(code);
            stop.cancel();
        }
        if watching && watch.as_mut().poll(context).is_ready() {
            watching = false;
        }
        std::task::Poll::Pending
    })
    .await
}

/// Waits for either of two futures.
async fn poll_either(first: impl Future<Output = ()>, second: impl Future<Output = ()>) {
    let (mut first, mut second) = (std::pin::pin!(first), std::pin::pin!(second));
    std::future::poll_fn(|context| {
        if first.as_mut().poll(context).is_ready() || second.as_mut().poll(context).is_ready() {
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    })
    .await;
}

/// The core thread: a runtime, the core, and the one connection. Returns
/// the exit code.
fn core_thread(
    options: cortex_core::CoreOptions,
    stalled_client: Duration,
    stop: &CancelToken,
    ending: &CancelToken,
    name: &str,
) -> u8 {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("{name}: the core thread's runtime did not start: {error}");
            return exit::INTERNAL;
        }
    };
    runtime.block_on(async move {
        let core = match Core::start(options) {
            Ok(core) => core,
            Err(error) => {
                eprintln!("{name}: {}", crate::visible(&error.message));
                return exit::for_outcome(
                    &exit::outcome_for_kind(&error.kind),
                    Some(&error.kind),
                    None,
                );
            }
        };
        let (transport, written) = match Transport::blocking(io::stdin(), io::stdout()) {
            Ok(transport) => transport,
            Err(error) => {
                eprintln!("{name}: standard I/O could not be served: {error}");
                core.shutdown().await;
                return exit::INTERNAL;
            }
        };
        let timer: SharedTimer = Arc::new(TokioTimer);
        let mut options = ServeOptions::new(timer);
        options.stalled_client = stalled_client;
        let ended = cortex_rpc::serve_one(core, transport, options, stop, ending).await;
        tracing::info!(?ended, "the server ended");
        // The last lines (a `shutdown`'s answer) are out before the process
        // ends, unless the reader stopped reading: the server's end has its
        // own bound, which this is inside.
        if !written.wait(LAST_LINES) {
            tracing::warn!("standard output was not read to the end");
        }
        exit::COMPLETED
    })
}
