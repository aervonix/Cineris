//! The day's limits, `[limits.day]`, across every process on the machine
//! (`docs/architecture/18-usage.md`, "Limits on one day").
//!
//! **One purse a day, in each core,** holds what the day has spent: what
//! the statistics held when the core's first day-limited session of the day
//! was put together, then this core's own spending as it happens, then what
//! other processes spent, brought up to date from the monthly files before
//! each request a day-limited session makes. Each session holds the day to
//! its own limits (a project may lower them) and spends from the purse.
//!
//! **What other processes spent is read on a thread of the purse's own,**
//! not on the blocking pool: a read stuck on a stalled disk must not hold
//! the runtime's end, which waits for the pool. A request waits for the
//! read at most [`CATCH_UP_LIMIT`], and then goes on with the purse as it
//! was last read; the log says so once. A read that ends later still adds
//! what it found.
//!
//! **The statistics are needed.** With them off there is nothing to read
//! another process's spending from, and a day's limit would hold per
//! process: a session with one is refused at open instead
//! (`day_limit_needs_stats`, the owner's decision of 2026-11-15).
//!
//! **This process's own requests are not read back:** each request line
//! says which day's purse it was charged to (`charged_to`), and the purse's
//! reader skips exactly its own. Its id is this process's recorder's, which
//! no other process's shares, and the day.
//!
//! **How far a limit can be passed.** A request in flight in another
//! process is not in the files until it ends, so a limit can be passed by
//! the requests other processes have in flight when it is reached: at most
//! two for each session running there (its request and a summary in the
//! background), and the tokens and money those use. A process's own
//! sessions are held exactly, as the ledger holds them. Spending that never
//! reaches the files is not seen by other processes at all: a process killed
//! during a request, events its writer dropped, a line it could not write;
//! nor is what was written while this purse could not read (a stalled disk,
//! a file it may not read), until it can again.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cortex_agent::{Account, CatchUp, Spent};
use cortex_base::{LocalDate, SharedTimer, Zone};
use cortex_protocol::ErrorKind;
use cortex_stats::{Aggregates, DayReader, DaySpent};
use cortex_tools::BoxFuture;

use crate::core::{OpError, error};

/// How long a request waits for what other processes spent. The read is a
/// few lines of a file, microseconds; one that takes a second is a disk
/// that has stalled, and a second is short beside a request's own time.
pub(crate) const CATCH_UP_LIMIT: Duration = Duration::from_secs(1);

/// The day's purse of one core.
pub(crate) struct Day {
    date: LocalDate,
    purse: Account,
}

/// What the purse's reader thread is asked: add what the files gained to
/// the purse, and answer.
struct Ask(tokio::sync::oneshot::Sender<()>);

fn spent(day: DaySpent) -> Spent {
    Spent {
        usd_nanos: day.usd_nanos,
        tokens: day.tokens,
        requests: day.requests,
    }
}

/// The purse's [`CatchUp`]: asks the reader thread, and waits for it at
/// most [`CATCH_UP_LIMIT`] on the core's timer.
struct Catching {
    asks: mpsc::Sender<Ask>,
    /// The purse's id, which the lines of the requests it counted carry.
    id: String,
    timer: SharedTimer,
    /// Whether the log has been told that a read took too long.
    told: AtomicBool,
}

impl std::fmt::Debug for Catching {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catching").finish_non_exhaustive()
    }
}

impl CatchUp for Catching {
    fn catch_up(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let (answer, answered) = tokio::sync::oneshot::channel();
            if self.asks.send(Ask(answer)).is_err() {
                return;
            }
            let sleep = self.timer.sleep(CATCH_UP_LIMIT);
            let mut answered = std::pin::pin!(answered);
            let mut sleep = std::pin::pin!(sleep);
            let in_time = std::future::poll_fn(|context| {
                if answered.as_mut().poll(context).is_ready() {
                    return std::task::Poll::Ready(true);
                }
                if sleep.as_mut().poll(context).is_ready() {
                    return std::task::Poll::Ready(false);
                }
                std::task::Poll::Pending
            })
            .await;
            if !in_time && !self.told.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    limit_ms = CATCH_UP_LIMIT.as_millis(),
                    "the day's limits could not read what other processes spent in time: the \
                     request goes on with the day's spending as last read"
                );
            }
        })
    }

    fn id(&self) -> &str {
        &self.id
    }
}

/// The purse's reader: adds what the files gained to the purse, for each
/// ask, until every asker has gone.
fn read(mut reader: DayReader, purse: &Account, asks: &mpsc::Receiver<Ask>) {
    let mut told = false;
    let mut failed = |failure: std::io::Error| {
        if !std::mem::replace(&mut told, true) {
            tracing::warn!(error = %failure, "the day's limits could not read the statistics");
        }
    };
    while let Ok(first) = asks.recv() {
        // Asks that came meanwhile are answered by the same read.
        let answers: Vec<Ask> = std::iter::once(first).chain(asks.try_iter()).collect();
        match reader.catch_up() {
            Ok(more) => purse.add_spent(spent(more)),
            Err(failure) => failed(failure),
        }
        for Ask(answer) in answers {
            let _ = answer.send(());
        }
    }
}

/// The purse, brought up to date by a reader thread of its own from now on,
/// and its catch-up.
fn reading(
    reader: DayReader,
    purse: Account,
    id: String,
    timer: SharedTimer,
) -> (Account, Arc<Catching>) {
    let (asks, asked) = mpsc::channel();
    let catching = Arc::new(Catching {
        asks,
        id,
        timer,
        told: AtomicBool::new(false),
    });
    // The thread adds to the purse through an account of its own that
    // shares what was spent and has no catch-up, so that it holds no way to
    // ask itself: once the day's last asker is gone its channel closes, and
    // it ends.
    let read_into = purse.clone();
    let purse = purse.catching_up(Arc::clone(&catching) as Arc<dyn CatchUp>);
    let started = std::thread::Builder::new()
        .name("cortex-day".to_owned())
        .spawn(move || read(reader, &read_into, &asked));
    if let Err(failure) = started {
        tracing::warn!(
            error = %failure,
            "the day's limits cannot read what other processes spend: they hold for this process"
        );
    }
    (purse, catching)
}

/// The account of today's limits for one session, `[limits.day]`:
/// requests, tokens and money, across every session of the day in every
/// process. `None` when no day limit is set. Called when the session is put
/// together, on the blocking pool; nothing is read without a day limit.
///
/// A session that runs past midnight goes on drawing on the day it began
/// in; the next session opened begins a new day. `limits.day.time` is not
/// enforced (`docs/outstanding/a-days-time-limit.md`).
///
/// # Errors
///
/// `day_limit_needs_stats` when there are no statistics to count other
/// processes' spending from: they are off, or there is no state directory.
/// `stats_unreadable` when the day's statistics cannot be read.
pub(crate) fn day_account(
    inner: &crate::core::Inner,
    limits: &cortex_config::schema::LimitSet,
) -> Result<Option<Account>, OpError> {
    if limits.requests.is_none() && limits.tokens.is_none() && limits.cost.is_none() {
        return Ok(None);
    }
    let (Some(recorder), Some(dir)) = (&inner.stats, crate::usage::stats_dir(&inner.user())) else {
        let why = if inner.user().settings.stats.enabled {
            "there is no state directory to keep them in"
        } else {
            "they are off (`stats.enabled = false`)"
        };
        return Err(error(
            ErrorKind::Usage,
            "day_limit_needs_stats",
            format!(
                "The day's limits count every process's spending from the statistics, and {why}. \
                 Turn them on, or remove `[limits.day]`."
            ),
        ));
    };
    let clock = Arc::clone(&inner.options.clock);
    let zone = inner.options.zone.clone().unwrap_or_else(Zone::system);
    let today = zone.date_of(clock.now());
    let budget = cortex_protocol::lease::Budget {
        wall_clock_ms: None,
        ..crate::assemble::budget(limits)
    };
    let name = format!("the day {today}");
    let mut day = inner.day.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(open) = day.as_ref()
        && open.date == today
    {
        return Ok(Some(Account::draws_on(name, budget, &open.purse)));
    }
    // The day's first: what the aggregates hold for the day, and what the
    // day's monthly files gained since they were saved; nothing else is
    // read, and nothing is pruned or saved (the writer does both).
    let unreadable = |failure: std::io::Error| {
        error(
            ErrorKind::Internal,
            "stats_unreadable",
            format!("The day's limits could not read what the day has spent: {failure}."),
        )
    };
    let aggregates = Aggregates::load(&dir).map_err(unreadable)?;
    // This process's recorder's id, which no other process's has, and the
    // day: a purse of its own, of this process and this day.
    let id = format!("{}/{today}", recorder.writer());
    let (mut reader, mut before) = DayReader::start(&dir, today, &aggregates, Some(id.clone()));
    before.add(reader.catch_up().map_err(unreadable)?);
    let purse = Account::with_spent(
        name.clone(),
        cortex_protocol::lease::Budget::default(),
        clock,
        spent(before),
    );
    let (purse, _) = reading(reader, purse, id, Arc::clone(&inner.options.timer));
    let account = Account::draws_on(name, budget, &purse);
    *day = Some(Day { date: today, purse });
    Ok(Some(account))
}

/// The core's purse of the day, while there is one.
pub(crate) type DaySlot = Mutex<Option<Day>>;

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use cortex_base::{FakeClock, LocalDate};
    use cortex_stats::{Aggregates, DayReader, StatsDir};

    use super::{Account, CatchUp, reading};

    /// One request line of another process, and the four other lines a
    /// turn writes beside it, of the day `2026-11-15`.
    fn a_requests_lines(n: u64) -> String {
        let request = format!(
            r#"{{"day":"2026-11-15","kind":"request","charged_to":"w_other/2026-11-15","at":"2026-11-15T10:00:00Z","session":"s_{n}","turn":"t_1","provider":"anthropic","model":"opus","role":"main","variants":{{"edit_format":"replace","tool_schema":"strict","prompt_style":"concise"}},"tokens":{{"input":1200,"cache_read":40000,"output":600}},"cost":{{"usd_nanos":47000000,"basis":"metered"}},"billing":"metered","stop":"tool_calls"}}"#
        );
        let other = format!(
            r#"{{"day":"2026-11-15","kind":"tool","at":"2026-11-15T10:00:00Z","session":"s_{n}","turn":"t_1","tool":"read","model":"opus","variants":{{"edit_format":"replace","tool_schema":"strict","prompt_style":"concise"}},"outcome":"ok","running_ms":12}}"#
        );
        format!("{request}\n{other}\n{other}\n{other}\n{other}\n")
    }

    /// What bringing a day's purse up to date costs a request: the ask to
    /// the reader thread, its read of what one request of another process
    /// added (five lines), and the answer. Run by hand: `cargo test
    /// --release -p cortex-core --lib perf_catching_up -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "a measurement, run by hand"]
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "a measurement that writes and removes its own directory"
    )]
    fn perf_catching_up_before_a_request() {
        use std::io::Write as _;
        let root = std::env::temp_dir().join(format!("cortex-perf-day-{}", std::process::id()));
        let dir = StatsDir::new(root.join("stats"));
        std::fs::create_dir_all(dir.root()).unwrap();
        let path = dir.events_file("2026-11");
        // A heavy day already in the file: 3,000 requests.
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        for n in 0..3_000 {
            file.write_all(a_requests_lines(n).as_bytes()).unwrap();
        }
        let day = LocalDate::parse("2026-11-15").unwrap();
        let mut aggregates = Aggregates::load(&dir).unwrap();
        aggregates.sync(&dir, day, "2026-11", None).unwrap();
        let (reader, _) =
            DayReader::start(&dir, day, &aggregates, Some("w_mine/2026-11-15".to_owned()));
        let clock = std::sync::Arc::new(FakeClock::starting_at("2026-11-15T10:00:00Z").unwrap());
        let purse = Account::new("the day", cortex_protocol::lease::Budget::default(), clock);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let timer: cortex_base::SharedTimer = std::sync::Arc::new(cortex_llm::TokioTimer);
        let (purse, catching) = reading(reader, purse, "w_mine/2026-11-15".to_owned(), timer);
        let mut times: Vec<Duration> = Vec::with_capacity(2_000);
        for n in 0..2_000 {
            file.write_all(a_requests_lines(n).as_bytes()).unwrap();
            let started = Instant::now();
            runtime.block_on(catching.catch_up());
            times.push(started.elapsed());
        }
        assert_eq!(purse.spent().requests, 2_000);
        times.sort();
        println!(
            "catching a day's purse up before a request: median {:?}, 95th {:?}, 99th {:?}, most {:?}",
            times[times.len() / 2],
            times[times.len() * 95 / 100],
            times[times.len() * 99 / 100],
            times[times.len() - 1],
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
