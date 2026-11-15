//! A made-up month of statistics, for tests and for measuring
//! (`docs/testing-and-ci.md`, "A month of statistics, made up").
//!
//! [`write`] fills a [`StatsDir`] with the events a person using the harness
//! for a month would have left, **through the crate's own [`Recorder`]**: the
//! monthly files are written by the real writer, line by line, and the
//! aggregates are brought up to date by the real sync, so what a reader finds
//! is exactly what a user's directory holds. It returns a [`Summary`] added up
//! from the events as they were made, by code that shares nothing with the
//! aggregates, so a test can compare the two.
//!
//! **Deterministic.** One seed gives one month, byte for byte, on every
//! machine: the only source of numbers is [`SeededRng`], whose output is
//! fixed, and nothing here reads the clock, the zone or the environment. Days
//! are counted in UTC. Only integers and a few exact divisions are used, so no
//! operating system's maths library takes part.
//!
//! **Realistic in proportion, not in content.** A month has working days and
//! days off, sessions in time slots of the day, turns that make a few
//! requests and run a few tool calls each, models and providers by the names
//! of the catalog (`catalog/models.toml`) priced at its prices, a cache that
//! is read on most requests and is lost when the user is away more than five
//! minutes, a context that grows and is compacted by a summary on the `fast`
//! role, a few failed requests, turns that end in each way, and a context
//! record for every request. Nothing in it is taken from a person's use.
//!
//! The module is built for the crate's own tests and, behind the `test-hooks`
//! feature, for the tests of other crates (a dev-dependency turns it on,
//! which `cargo xtask rules` checks). The product never has it.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "it writes statistics files through the recorder, off any core thread, for tests"
)]

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use cortex_base::{FakeClock, LocalDate, Rng, SeededRng, UtcTime, Zone};
use cortex_protocol::event::{TurnOutcome, Verification};
use cortex_protocol::model::ModelRole;
use cortex_protocol::usage::{Billing, Cost, CostBasis, ModelTime, TokenUsage};
use cortex_protocol::{ModelId, ProjectKey, ProviderId, SessionId, Speed, Timestamp, TurnId};

use crate::aggregate::UsageSum;
use crate::record::{
    ContextRecord, Difference, Purpose, Record, RequestRecord, Sources, ToolOutcome, ToolRecord,
    ToolShare, TurnRecord, Variants,
};
use crate::recorder::{Recorder, RecorderOptions};
use crate::store::StatsDir;

#[cfg(test)]
mod tests;

/// How many events may wait for the writer, and so how many are made before
/// the generator waits for it.
#[cfg(not(test))]
const QUEUE: usize = 16_384;

/// In the crate's own tests, small: the months they make are a day or two,
/// and with the queue of a real run the generator would never wait for the
/// writer, so nothing would show that it does.
#[cfg(test)]
const QUEUE: usize = 64;

/// A session is compacted when its context passes this many tokens.
const COMPACT_AT: u64 = 160_000;

/// What the anthropic cache keeps a prefix for, in milliseconds.
const CACHE_LIFETIME_MS: i64 = 300_000;

/// What a user's month looks like: how often they work, and how much.
///
/// The four in [`Scale::ALL`] are the ones tests use. Every number is a
/// range or a percentage the generator draws from, so two months of one scale
/// differ in their figures and not in their shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    /// The scale's name.
    pub name: &'static str,
    /// The chance, in percent, that a weekday is a working day.
    pub weekday_active: u64,
    /// The chance, in percent, that a Saturday or Sunday is.
    pub weekend_active: u64,
    /// Sessions a working day, at least and at most.
    pub sessions: (u64, u64),
    /// Turns a session.
    pub turns: (u64, u64),
    /// Requests to the main model a turn.
    pub requests: (u64, u64),
    /// Tool calls in one round of a turn.
    pub tool_batch: (u64, u64),
    /// How many projects the user works in.
    pub projects: u64,
    /// The main models a session may use, as the catalog names them, with
    /// how likely each is.
    pub models: &'static [(&'static str, u64)],
    /// The chance, in percent, that a session on a model that has a fast
    /// tier runs at it.
    pub fast_speed: u64,
    /// The chance, in percent, that a session on the anthropic provider is
    /// on a subscription, so that its cost is an estimate.
    pub subscription: u64,
    /// Failed requests in every thousand.
    pub failures: u64,
    /// The chance, in percent, that a turn has a judge's request.
    pub judge: u64,
}

impl Scale {
    /// Someone who uses it now and then: about two days in five, a session
    /// or two, short turns. Some 200 requests a month.
    pub const LIGHT: Self = Self {
        name: "light",
        weekday_active: 45,
        weekend_active: 5,
        sessions: (1, 2),
        turns: (3, 8),
        requests: (2, 3),
        tool_batch: (1, 2),
        projects: 2,
        models: &[
            ("claude-sonnet-5-5", 6),
            ("claude-haiku-5-5", 2),
            ("gpt-6-luna", 1),
            ("deepseek-flash", 1),
            ("z-ai/glm-5.3-flash", 1),
        ],
        fast_speed: 0,
        subscription: 0,
        failures: 8,
        judge: 0,
    };

    /// A daily user: most weekdays, a few sessions of a dozen turns. Some
    /// 1,500 requests a month.
    pub const TYPICAL: Self = Self {
        name: "typical",
        weekday_active: 90,
        weekend_active: 15,
        sessions: (2, 3),
        turns: (5, 12),
        requests: (2, 4),
        tool_batch: (1, 3),
        projects: 4,
        models: &[
            ("claude-sonnet-5-5", 4),
            ("claude-opus-5-5", 3),
            ("claude-haiku-5-5", 1),
            ("gpt-6-luna", 1),
            ("deepseek-v4-pro", 1),
            ("anthropic/claude-haiku-5.5", 1),
        ],
        fast_speed: 5,
        subscription: 20,
        failures: 6,
        judge: 0,
    };

    /// Someone who lives in it: nearly every day, weekends too, many long
    /// sessions, every provider. Some 5,000 requests a month.
    pub const HEAVY: Self = Self {
        name: "heavy",
        weekday_active: 98,
        weekend_active: 50,
        sessions: (4, 5),
        turns: (10, 20),
        requests: (2, 4),
        tool_batch: (1, 3),
        projects: 9,
        models: &[
            ("claude-opus-5-5", 5),
            ("claude-fable-5-2", 3),
            ("claude-sonnet-5-5", 3),
            ("gpt-6-luna", 1),
            ("deepseek-v4-pro", 1),
            ("openai/gpt-6-luna", 1),
            ("z-ai/glm-5.3-flash", 1),
            ("deepseek-v4-flash-0731", 1),
        ],
        fast_speed: 15,
        subscription: 35,
        failures: 5,
        judge: 3,
    };

    /// The extreme the statistics' size was measured at in step 16 (a day of
    /// several thousand requests), for runs of the release build. Not for a
    /// test of the debug build: it makes some 90,000 requests a month.
    pub const EXTREME: Self = Self {
        name: "extreme",
        weekday_active: 100,
        weekend_active: 80,
        sessions: (14, 18),
        turns: (40, 60),
        requests: (3, 8),
        tool_batch: (1, 4),
        projects: 12,
        models: Self::HEAVY.models,
        fast_speed: 15,
        subscription: 35,
        failures: 5,
        judge: 3,
    };

    /// The three scales of ordinary tests, smallest first.
    pub const ALL: [Self; 3] = [Self::LIGHT, Self::TYPICAL, Self::HEAVY];
}

/// What to make: a seed, a scale, and which months.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spec {
    /// The seed. The same seed, scale and months give the same bytes.
    pub seed: u64,
    /// How much of what.
    pub scale: Scale,
    /// The first month, as year and month.
    pub first: (i32, u8),
    /// How many months, from the first. One is a month; twelve is a year of
    /// history (a year or less keeps every day a day bucket and prunes no
    /// file).
    pub months: u32,
    /// How many days to make of each month, from its first. `0`, or more
    /// than the month has: all of them. A few days make a small month, for
    /// the tests that need only its shape.
    pub days: u32,
}

impl Spec {
    /// One month, October 2026.
    #[must_use]
    pub const fn month(seed: u64, scale: Scale) -> Self {
        Self {
            seed,
            scale,
            first: (2026, 10),
            months: 1,
            days: 0,
        }
    }

    /// The same, with only the first `days` days of each month.
    #[must_use]
    pub const fn with_days(self, days: u32) -> Self {
        Self { days, ..self }
    }

    /// The same, with `months` months.
    #[must_use]
    pub const fn with_months(self, months: u32) -> Self {
        Self { months, ..self }
    }

    /// Twelve months, from January 2026.
    #[must_use]
    pub const fn year(seed: u64, scale: Scale) -> Self {
        Self {
            seed,
            scale,
            first: (2026, 1),
            months: 12,
            days: 0,
        }
    }
}

/// What was made, added up from the events as they were made.
///
/// The comparison with what the crate's own reader and aggregates say is the
/// point of it: they are added up by other code, from the files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// The seed.
    pub seed: u64,
    /// The scale's name.
    pub scale: &'static str,
    /// The months written, `YYYY-MM`.
    pub months: Vec<String>,
    /// Days in those months.
    pub days: u32,
    /// Days with a session.
    pub active_days: u32,
    /// Days with none.
    pub days_off: u32,
    /// Sessions.
    pub sessions: u64,
    /// Turns.
    pub turns: u64,
    /// Requests, failed ones and the fast and judge roles' included.
    pub requests: u64,
    /// The requests that failed.
    pub failed_requests: u64,
    /// Tool calls.
    pub tool_calls: u64,
    /// Context records.
    pub contexts: u64,
    /// Lines written to the monthly files.
    pub lines: u64,
    /// Usage by local day and model, as the aggregates keep it by day and
    /// model with the provider, speed, project and billing added up.
    pub usage: BTreeMap<(String, String), UsageSum>,
    /// All the usage.
    pub totals: UsageSum,
    /// Turns by how they ended, by the name the files write.
    pub outcomes: BTreeMap<String, u64>,
    /// Tool calls by tool.
    pub tools: BTreeMap<String, u64>,
    /// Turns' wall time, in milliseconds.
    pub wall_ms: u64,
}

impl Summary {
    fn new(spec: &Spec) -> Self {
        Self {
            seed: spec.seed,
            scale: spec.scale.name,
            months: Vec::new(),
            days: 0,
            active_days: 0,
            days_off: 0,
            sessions: 0,
            turns: 0,
            requests: 0,
            failed_requests: 0,
            tool_calls: 0,
            contexts: 0,
            lines: 0,
            usage: BTreeMap::new(),
            totals: UsageSum::default(),
            outcomes: BTreeMap::new(),
            tools: BTreeMap::new(),
            wall_ms: 0,
        }
    }

    /// Adds one event as it is made, by the UTC day its time names.
    fn count(&mut self, record: &Record) {
        let day = record.at().as_str().get(..10).unwrap_or_default();
        self.lines += 1;
        match record {
            Record::Request(request) => {
                self.requests += 1;
                if request.error.is_some() {
                    self.failed_requests += 1;
                }
                let sum = self
                    .usage
                    .entry((day.to_owned(), request.model.as_str().to_owned()))
                    .or_default();
                sum.requests += 1;
                sum.input += request.tokens.input;
                sum.cache_read += request.tokens.cache_read;
                sum.cache_write += request.tokens.cache_write;
                sum.output += request.tokens.output;
                sum.reasoning += request.tokens.reasoning.unwrap_or(0);
                match &request.cost {
                    None => sum.unpriced += 1,
                    Some(cost) => match cost.basis {
                        CostBasis::Metered => sum.metered_nanos += cost.usd_nanos,
                        CostBasis::Recorded => sum.recorded_nanos += cost.usd_nanos,
                        _ => sum.estimate_nanos += cost.usd_nanos,
                    },
                }
            }
            Record::Tool(tool) => {
                self.tool_calls += 1;
                *self.tools.entry(tool.tool.clone()).or_default() += 1;
            }
            Record::Turn(turn) => {
                self.turns += 1;
                self.wall_ms += turn.wall_ms;
                *self
                    .outcomes
                    .entry(turn.outcome.as_str().to_owned())
                    .or_default() += 1;
            }
            Record::Context(_) => self.contexts += 1,
            Record::Dropped { .. } => {}
        }
    }

    fn finish(&mut self) {
        let mut totals = UsageSum::default();
        for sum in self.usage.values() {
            totals.requests += sum.requests;
            totals.input += sum.input;
            totals.cache_read += sum.cache_read;
            totals.cache_write += sum.cache_write;
            totals.output += sum.output;
            totals.reasoning += sum.reasoning;
            totals.metered_nanos += sum.metered_nanos;
            totals.estimate_nanos += sum.estimate_nanos;
            totals.recorded_nanos += sum.recorded_nanos;
            totals.unpriced += sum.unpriced;
        }
        self.totals = totals;
    }
}

/// A price card, in ten-thousandths of a dollar for a million tokens, so
/// that the catalog's prices (`0.0399`) are whole numbers.
#[derive(Debug, Clone, Copy)]
struct Card {
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
}

/// A model of the catalog, with what the generator needs of it.
#[derive(Debug, Clone, Copy)]
struct Model {
    provider: &'static str,
    id: &'static str,
    /// The model's context window.
    window: u64,
    card: Card,
    /// The card above 100,000 input tokens, where the catalog has one.
    above: Option<Card>,
    /// The card at the fast tier, where the model has one.
    fast: Option<Card>,
    /// The levels of reasoning the model names, least first. Empty: none.
    levels: &'static [&'static str],
    /// The model that does the `fast` role's work for it.
    utility: &'static str,
    /// Output tokens a second at standard speed, at least and at most.
    rate: (u64, u64),
    /// The tool schema and prompt style the catalog's variants give it.
    tool_schema: &'static str,
    prompt_style: &'static str,
}

const FIVE: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const SIX: &[&str] = &["none", "low", "medium", "high", "xhigh", "max"];
const FOUR: &[&str] = &["none", "low", "high", "max"];
const THREE: &[&str] = &["low", "high", "max"];

const fn card(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Card {
    Card {
        input,
        output,
        cache_read,
        cache_write,
    }
}

/// The models of `catalog/models.toml` and their standard prices there. A
/// test reads the catalog and checks each is in it.
const MODELS: &[Model] = &[
    Model {
        provider: "anthropic",
        id: "claude-fable-5-2",
        window: 1_000_000,
        card: card(80_000, 400_000, 2_000, 100_000),
        above: None,
        fast: Some(card(160_000, 800_000, 4_000, 200_000)),
        levels: FIVE,
        utility: "claude-haiku-5-5",
        rate: (45, 75),
        tool_schema: "strict",
        prompt_style: "concise",
    },
    Model {
        provider: "anthropic",
        id: "claude-opus-5-5",
        window: 1_000_000,
        card: card(40_000, 200_000, 2_000, 50_000),
        above: None,
        fast: Some(card(80_000, 400_000, 4_000, 100_000)),
        levels: FIVE,
        utility: "claude-haiku-5-5",
        rate: (50, 80),
        tool_schema: "strict",
        prompt_style: "concise",
    },
    Model {
        provider: "anthropic",
        id: "claude-sonnet-5-5",
        window: 1_000_000,
        card: card(20_000, 100_000, 1_000, 25_000),
        above: None,
        fast: None,
        levels: FIVE,
        utility: "claude-haiku-5-5",
        rate: (60, 100),
        tool_schema: "strict",
        prompt_style: "concise",
    },
    Model {
        provider: "anthropic",
        id: "claude-haiku-5-5",
        window: 1_000_000,
        card: card(1_000, 5_000, 100, 1_250),
        above: Some(card(5_000, 25_000, 500, 6_250)),
        fast: None,
        levels: FIVE,
        utility: "claude-haiku-5-5",
        rate: (120, 220),
        tool_schema: "strict",
        prompt_style: "concise",
    },
    Model {
        provider: "openai",
        id: "gpt-6-luna",
        window: 1_050_000,
        card: card(1_000, 5_000, 100, 0),
        above: None,
        fast: Some(card(2_000, 10_000, 200, 0)),
        levels: SIX,
        utility: "gpt-6-luna",
        rate: (90, 160),
        tool_schema: "strict",
        prompt_style: "concise",
    },
    Model {
        provider: "openrouter",
        id: "z-ai/glm-5.3-flash",
        window: 1_048_576,
        card: card(399, 784, 300, 0),
        above: None,
        fast: None,
        levels: THREE,
        utility: "z-ai/glm-5.3-flash",
        rate: (80, 140),
        tool_schema: "loose",
        prompt_style: "guided",
    },
    Model {
        provider: "openrouter",
        id: "openai/gpt-6-luna",
        window: 1_050_000,
        card: card(1_000, 5_000, 100, 0),
        above: None,
        fast: Some(card(2_000, 10_000, 200, 0)),
        levels: SIX,
        utility: "openai/gpt-6-luna",
        rate: (90, 160),
        tool_schema: "strict",
        prompt_style: "concise",
    },
    Model {
        provider: "openrouter",
        id: "anthropic/claude-haiku-5.5",
        window: 1_000_000,
        card: card(1_000, 5_000, 100, 0),
        above: None,
        fast: None,
        levels: SIX,
        utility: "anthropic/claude-haiku-5.5",
        rate: (120, 220),
        tool_schema: "strict",
        prompt_style: "concise",
    },
    Model {
        provider: "deepseek",
        id: "deepseek-flash",
        window: 1_000_000,
        card: card(3_000, 12_000, 60, 0),
        above: None,
        fast: None,
        levels: FOUR,
        utility: "deepseek-flash",
        rate: (70, 130),
        tool_schema: "loose",
        prompt_style: "guided",
    },
    Model {
        provider: "deepseek",
        id: "deepseek-v4-pro",
        window: 1_000_000,
        card: card(13_200, 39_600, 440, 0),
        above: None,
        fast: None,
        levels: FOUR,
        utility: "deepseek-flash",
        rate: (40, 80),
        tool_schema: "strict",
        prompt_style: "concise",
    },
    Model {
        provider: "venice",
        id: "deepseek-v4-flash-0731",
        window: 1_000_000,
        card: card(1_300, 2_600, 280, 0),
        above: None,
        fast: Some(card(3_500, 7_000, 875, 0)),
        levels: FOUR,
        utility: "deepseek-v4-flash-0731",
        rate: (70, 130),
        tool_schema: "loose",
        prompt_style: "guided",
    },
];

/// The models the generator draws from, as the catalog names them.
#[must_use]
pub fn model_ids() -> Vec<&'static str> {
    MODELS.iter().map(|model| model.id).collect()
}

fn model(id: &str) -> &'static Model {
    MODELS
        .iter()
        .find(|model| model.id == id)
        .unwrap_or(&MODELS[0])
}

/// What a request cost, in nano-dollars, at a model's prices.
fn price(model: &Model, speed: &Speed, tokens: &TokenUsage) -> u64 {
    let total_input = tokens.input + tokens.cache_read + tokens.cache_write;
    let standard = match model.above {
        Some(above) if total_input > 100_000 => above,
        _ => model.card,
    };
    let card = match (speed, model.fast) {
        (Speed::Fast, Some(fast)) => fast,
        _ => standard,
    };
    (tokens.input * card.input
        + tokens.cache_read * card.cache_read
        + tokens.cache_write * card.cache_write
        + tokens.output * card.output)
        / 10
}

/// The tools of the built-in set and how long each runs, in milliseconds,
/// at least and at most, with how likely each is.
const TOOLS: &[(&str, (u64, u64), u64)] = &[
    ("read", (3, 60), 40),
    ("search", (20, 400), 20),
    ("list", (4, 50), 8),
    ("edit", (8, 90), 14),
    ("write", (8, 90), 3),
    ("shell", (200, 30_000), 12),
    ("plan", (1, 6), 2),
    ("outline", (5, 60), 1),
];

fn range(rng: &mut SeededRng, (low, high): (u64, u64)) -> u64 {
    low + rng.below(high.saturating_sub(low) + 1)
}

/// A number in the range that is small more often than large: the lower of
/// two draws, which is how sizes of files and of answers fall.
fn skewed(rng: &mut SeededRng, (low, high): (u64, u64)) -> u64 {
    let first = range(rng, (low, high));
    first.min(range(rng, (low, high)))
}

fn chance(rng: &mut SeededRng, percent: u64) -> bool {
    rng.below(100) < percent
}

fn stamp(millis: i64) -> Timestamp {
    UtcTime::from_unix_millis(millis)
        .unwrap_or(UtcTime::UNIX_EPOCH)
        .to_timestamp()
}

/// A session as it goes on.
struct Session {
    id: SessionId,
    project: ProjectKey,
    main: &'static Model,
    utility: &'static Model,
    billing: Billing,
    speed: Speed,
    variants: Variants,
    reasons: bool,
    /// What the context holds, by source.
    sources: Sources,
    epoch: u64,
    /// Whether the next request begins an epoch: nothing is cached yet.
    fresh: bool,
    /// When the last request ended, for the cache's lifetime.
    last_end: i64,
    turn: u64,
}

impl Session {
    fn context(&self) -> u64 {
        let sources = &self.sources;
        sources.system
            + sources.tools
            + sources.summary
            + sources.user
            + sources.assistant
            + sources.tool_results
            + sources.notices
    }
}

struct Generator {
    rng: SeededRng,
    recorder: Recorder,
    summary: Summary,
    scale: Scale,
    projects: Vec<ProjectKey>,
    /// Events made since the writer was last waited for.
    pending: usize,
}

/// Writes a month of events, or several, into `dir`, and says what it wrote.
///
/// The files are written by a [`Recorder`] on the writer thread it always
/// has, and the aggregates are brought up to date when it is done. A
/// directory that already holds files is added to, not replaced: give it an
/// empty one.
///
/// # Errors
///
/// When the writer cannot be started, or the recorder dropped an event
/// because the writer fell behind (it is waited for often enough that this
/// does not happen).
pub fn write(dir: &StatsDir, spec: &Spec) -> io::Result<Summary> {
    let mut first = LocalDate::new(spec.first.0, spec.first.1, 1)
        .ok_or_else(|| io::Error::other("not a month"))?;
    let zone = Zone::utc();
    // The writer's own "today" is the last day written, so that nothing is
    // folded or pruned in a year or less.
    let mut end = first;
    for _ in 0..spec.months {
        end = next_month(end);
    }
    let clock = Arc::new(FakeClock::new(zone.start_of(end.add_days(-1))));
    let mut options = RecorderOptions::new(dir.clone());
    options.zone = Some(zone.clone());
    options.queue = QUEUE;
    options.retention = spec.months.saturating_add(12);
    let recorder = Recorder::new(options, clock);
    recorder.start()?;

    let mut rng = SeededRng::new(spec.seed);
    let projects = (0..spec.scale.projects.max(1))
        .map(|_| ProjectKey::new(format!("{:016x}{:016x}", rng.next_u64(), rng.next_u64())))
        .collect();
    let mut generator = Generator {
        rng,
        recorder,
        summary: Summary::new(spec),
        scale: spec.scale,
        projects,
        pending: 0,
    };
    for _ in 0..spec.months {
        generator.summary.months.push(first.month_text());
        let next = next_month(first);
        let mut date = first;
        let mut made = 0;
        while date < next && (spec.days == 0 || made < spec.days) {
            generator.make_day(date, &zone);
            date = date.add_days(1);
            made += 1;
        }
        first = next;
    }
    generator.recorder.flush();
    if generator.recorder.dropped() > 0 {
        return Err(io::Error::other("the recorder dropped events"));
    }
    generator.summary.finish();
    Ok(generator.summary)
}

fn next_month(first: LocalDate) -> LocalDate {
    let (year, month) = if first.month() == 12 {
        (first.year() + 1, 1)
    } else {
        (first.year(), first.month() + 1)
    };
    LocalDate::new(year, month, 1).unwrap_or(first.add_days(31))
}

impl Generator {
    fn make_day(&mut self, date: LocalDate, zone: &Zone) {
        self.summary.days += 1;
        let weekend = date.weekday() >= 5;
        let percent = if weekend {
            self.scale.weekend_active
        } else {
            self.scale.weekday_active
        };
        if !chance(&mut self.rng, percent) {
            self.summary.days_off += 1;
            return;
        }
        self.summary.active_days += 1;
        let day_start = zone.start_of(date).unix_millis();
        // Sessions take turns at the day, between 06:00 and 23:30.
        let sessions = range(&mut self.rng, self.scale.sessions);
        let window = 17 * 3_600_000 + 1_800_000;
        let slot = window / i64::try_from(sessions).unwrap_or(1);
        for number in 0..i64::try_from(sessions).unwrap_or(0) {
            let begins = day_start + 6 * 3_600_000 + number * slot;
            self.make_session(begins, begins + slot);
        }
    }

    fn make_session(&mut self, begins: i64, ends: i64) {
        self.summary.sessions += 1;
        let weights: u64 = self.scale.models.iter().map(|(_, weight)| weight).sum();
        let mut pick = self.rng.below(weights.max(1));
        let mut chosen = self.scale.models[0].0;
        for (id, weight) in self.scale.models {
            if pick < *weight {
                chosen = id;
                break;
            }
            pick -= weight;
        }
        let main = model(chosen);
        let utility = model(main.utility);
        let anthropic = main.provider == "anthropic";
        let billing = if anthropic && chance(&mut self.rng, self.scale.subscription) {
            Billing::Subscription
        } else {
            Billing::Metered
        };
        let speed = if main.fast.is_some() && chance(&mut self.rng, self.scale.fast_speed) {
            Speed::Fast
        } else {
            Speed::Standard
        };
        // The effort is the model's default more often than not.
        let effort = if main.levels.is_empty() {
            None
        } else if chance(&mut self.rng, 60) {
            Some("high")
        } else {
            let at = usize::try_from(self.rng.below(main.levels.len() as u64)).unwrap_or(0);
            main.levels.get(at).copied()
        };
        let project = usize::try_from(self.rng.below(self.projects.len() as u64)).unwrap_or(0);
        let start = begins + i64::try_from(self.rng.below(600_000)).unwrap_or(0);
        let random = self.rng.next_u64() >> 16;
        let mut session = Session {
            id: SessionId::new(format!("s_{:010x}{random:012x}", start / 1000)),
            project: self.projects[project].clone(),
            main,
            utility,
            billing,
            speed,
            variants: Variants {
                edit_format: "replace".to_owned(),
                tool_schema: main.tool_schema.to_owned(),
                prompt_style: main.prompt_style.to_owned(),
                effort: effort.map(str::to_owned),
            },
            reasons: effort.is_some_and(|level| level != "none"),
            sources: Sources {
                system: range(&mut self.rng, (4_000, 8_000)),
                tools: range(&mut self.rng, (3_000, 6_000)),
                ..Sources::default()
            },
            epoch: 1,
            fresh: true,
            last_end: start,
            turn: 0,
        };
        let turns = range(&mut self.rng, self.scale.turns);
        let mut cursor = start;
        for _ in 0..turns {
            if cursor + 60_000 > ends {
                break;
            }
            session.turn += 1;
            let finished = self.make_turn(&mut session, cursor);
            let away = if chance(&mut self.rng, 8) {
                range(&mut self.rng, (300_000, 1_500_000))
            } else {
                skewed(&mut self.rng, (3_000, 240_000))
            };
            cursor = finished + i64::try_from(away).unwrap_or(0);
        }
    }

    /// Makes one turn from `begins` and returns when it ended.
    fn make_turn(&mut self, session: &mut Session, begins: i64) -> i64 {
        let turn = TurnId::new(format!("t_{}", session.turn));
        let mut cursor = begins;
        let mut requests = range(&mut self.rng, self.scale.requests);
        // How the turn ends: stopping before its last request needs a round
        // of tool calls to stop at.
        let roll = self.rng.below(100);
        let ending = match roll {
            0..=2 => TurnOutcome::Interrupted,
            3 => TurnOutcome::Limit,
            4 => TurnOutcome::Denied,
            _ => TurnOutcome::Completed,
        };
        if ending != TurnOutcome::Completed && requests < 2 {
            requests = 2;
        }
        let stops_at = self.rng.below(requests - 1);
        let mut outcome = TurnOutcome::Completed;
        let mut model_time = ModelTime::default();
        let mut tools_ms = 0;
        let mut approval_ms = 0;
        let mut calls = 0;
        let mut edited = false;
        let mut shares: BTreeMap<&'static str, u64> = BTreeMap::new();
        let mut made = 0;

        if session.turn == 1 {
            cursor = self.utility_request(session, &turn, cursor, Purpose::Turn, ModelRole::Fast);
        }
        for number in 0..requests {
            if session.context() > COMPACT_AT {
                cursor = self.compact(session, &turn, cursor);
            }
            made += 1;
            let first_of_turn = number == 0;
            let (end, failed) =
                self.main_request(session, &turn, cursor, first_of_turn, &mut model_time);
            cursor = end;
            if failed {
                outcome = TurnOutcome::Failed;
                break;
            }
            if number + 1 == requests {
                break;
            }
            // The round of tool calls this response asked for.
            let stopping = ending != TurnOutcome::Completed && number == stops_at;
            let round = self.tool_round(session, &turn, cursor, stopping.then_some(ending.clone()));
            cursor += i64::try_from(round.wall).unwrap_or(0);
            tools_ms += round.tools_ms;
            approval_ms += round.approval_ms;
            calls += round.calls;
            edited |= round.edited;
            *shares.entry(round.slowest).or_default() += round.tools_ms;
            if stopping {
                outcome = ending.clone();
                break;
            }
        }
        if chance(&mut self.rng, self.scale.judge) {
            cursor = self.judge_request(session, &turn, cursor);
        }
        let verification = if outcome == TurnOutcome::Completed && edited {
            Some(match self.rng.below(100) {
                0..=69 => Verification::Passed,
                70..=77 => Verification::Failed,
                _ => Verification::Skipped,
            })
        } else {
            None
        };
        let verification_ms = match verification {
            Some(Verification::Passed | Verification::Failed) => {
                range(&mut self.rng, (2_000, 60_000))
            }
            _ => 0,
        };
        let harness_ms = range(&mut self.rng, (5, 80));
        cursor += i64::try_from(verification_ms + harness_ms).unwrap_or(0);
        let wall_ms = u64::try_from(cursor - begins).unwrap_or(0);
        self.emit(Record::Turn(TurnRecord {
            at: stamp(cursor),
            session: session.id.clone(),
            turn: turn.clone(),
            project: Some(session.project.clone()),
            provider: ProviderId::new(session.main.provider),
            model: ModelId::new(session.main.id),
            variants: session.variants.clone(),
            requests: made,
            tool_calls: calls,
            outcome,
            verification,
            wall_ms,
            model_time,
            tools_ms,
            approval_ms,
            verification_ms,
            harness_ms,
            by_tool: shares
                .into_iter()
                .filter(|(_, ms)| *ms > 0)
                .map(|(tool, ms)| ToolShare {
                    tool: tool.to_owned(),
                    ms,
                })
                .collect(),
        }));
        cursor
    }

    /// One request to the session's main model. Returns when it ended and
    /// whether it failed.
    fn main_request(
        &mut self,
        session: &mut Session,
        turn: &TurnId,
        begins: i64,
        first_of_turn: bool,
        into: &mut ModelTime,
    ) -> (i64, bool) {
        let new = if first_of_turn {
            skewed(&mut self.rng, (150, 3_500))
        } else {
            skewed(&mut self.rng, (300, 9_000))
        };
        let output = skewed(&mut self.rng, (60, 2_800));
        let before = session.context();
        let expired = begins - session.last_end > CACHE_LIFETIME_MS;
        let cold = session.fresh || expired;
        let writes = session.main.provider == "anthropic";
        let mut tokens = TokenUsage::default();
        if cold {
            if writes {
                tokens.cache_write = before + new;
                tokens.input = range(&mut self.rng, (3, 60));
            } else {
                tokens.input = before + new;
            }
        } else {
            tokens.cache_read = before;
            if writes {
                tokens.cache_write = new;
                tokens.input = range(&mut self.rng, (3, 60));
            } else {
                tokens.input = new;
            }
        }
        tokens.output = output;
        if session.reasons {
            tokens.reasoning = Some(output * range(&mut self.rng, (10, 45)) / 100);
        }

        let failed = self.rng.below(1000) < self.scale.failures;
        let fast = session.speed == Speed::Fast;
        let retried = if failed || chance(&mut self.rng, 4) {
            range(&mut self.rng, (1, 3))
        } else {
            0
        };
        let (low, high) = session.main.rate;
        let rate = range(&mut self.rng, (low, high)) * if fast { 5 } else { 2 } / 2;
        let mut time = ModelTime {
            connect_ms: range(&mut self.rng, (60, 600)),
            first_byte_ms: range(&mut self.rng, (150, 2_500)),
            streaming_ms: output * 1000 / rate.max(1),
            retry_ms: retried * range(&mut self.rng, (800, 4_000)),
            summary_ms: 0,
        };
        if failed {
            time.streaming_ms = 0;
        }
        let spent = time.connect_ms + time.first_byte_ms + time.streaming_ms + time.retry_ms;
        let end = begins + i64::try_from(spent).unwrap_or(0);
        into.connect_ms += time.connect_ms;
        into.first_byte_ms += time.first_byte_ms;
        into.streaming_ms += time.streaming_ms;
        into.retry_ms += time.retry_ms;

        // A fast session falls back to standard now and then.
        let ran = if fast && chance(&mut self.rng, 5) {
            Speed::Standard
        } else {
            session.speed.clone()
        };
        let requested = (ran != session.speed).then(|| session.speed.clone());
        let (cost, tokens, error) = if failed {
            (None, TokenUsage::default(), Some(self.failure_code()))
        } else {
            let nanos = price(session.main, &ran, &tokens);
            let basis = if session.billing == Billing::Metered {
                CostBasis::Metered
            } else {
                CostBasis::Estimate
            };
            (
                Some(Cost {
                    usd_nanos: nanos,
                    basis,
                }),
                tokens,
                None,
            )
        };
        let stop = if failed {
            "error"
        } else if first_of_turn && self.rng.below(100) < 8 {
            "length"
        } else {
            "end"
        };
        self.emit(Record::Request(RequestRecord {
            at: stamp(end),
            session: session.id.clone(),
            turn: turn.clone(),
            started_in: None,
            project: Some(session.project.clone()),
            provider: ProviderId::new(session.main.provider),
            model: ModelId::new(session.main.id),
            role: ModelRole::Main,
            purpose: Purpose::Turn,
            speed: ran,
            speed_requested: requested,
            variants: session.variants.clone(),
            tokens: tokens.clone(),
            cost,
            catalog_version: Some("2026-11-12".to_owned()),
            billing: session.billing.clone(),
            ttft_ms: (!failed).then_some(time.connect_ms + time.first_byte_ms),
            tokens_per_second: (!failed).then_some(rate as f64),
            time,
            stop: stop.to_owned(),
            error,
            retries: u32::try_from(retried).unwrap_or(0),
            charged_to: None,
        }));
        if failed {
            session.last_end = end;
            return (end, true);
        }

        // What the request held, and what it did to the context.
        let state = &mut session.sources;
        let difference = if session.fresh {
            Difference::First
        } else {
            match self.rng.below(100) {
                0..=2 => Difference::Message {
                    index: self.rng.below(200),
                },
                3..=4 => Difference::Tools,
                5..=6 => Difference::System,
                7 => Difference::Settings,
                _ => Difference::None,
            }
        };
        if first_of_turn {
            state.user += new;
        } else {
            state.tool_results += new;
        }
        let notice = if chance(&mut self.rng, 6) {
            range(&mut self.rng, (20, 200))
        } else {
            0
        };
        state.notices += notice;
        state.assistant += output;
        let estimate = session.context();
        self.emit(Record::Context(ContextRecord {
            at: stamp(end),
            session: session.id.clone(),
            turn: turn.clone(),
            project: Some(session.project.clone()),
            model: ModelId::new(session.main.id),
            epoch: session.epoch,
            sources: session.sources,
            estimate,
            limit: 200_000,
            window: session.main.window,
            difference,
            tokens: Some(tokens),
        }));
        session.fresh = false;
        session.last_end = end;
        (end, false)
    }

    /// A summary that starts a new epoch, on the `fast` role.
    fn compact(&mut self, session: &mut Session, turn: &TurnId, begins: i64) -> i64 {
        let end = self.utility_request(session, turn, begins, Purpose::Summary, ModelRole::Fast);
        let summary = range(&mut self.rng, (3_000, 6_000));
        session.sources = Sources {
            system: session.sources.system,
            tools: session.sources.tools,
            summary,
            ..Sources::default()
        };
        session.epoch += 1;
        session.fresh = true;
        end
    }

    /// A request of the utility model: a title, or a summary.
    fn utility_request(
        &mut self,
        session: &mut Session,
        turn: &TurnId,
        begins: i64,
        purpose: Purpose,
        role: ModelRole,
    ) -> i64 {
        let utility = session.utility;
        let (input, output) = if purpose == Purpose::Summary {
            (session.context(), range(&mut self.rng, (1_500, 4_000)))
        } else {
            (
                range(&mut self.rng, (300, 1_500)),
                range(&mut self.rng, (8, 30)),
            )
        };
        let tokens = TokenUsage {
            input,
            output,
            ..TokenUsage::default()
        };
        let (low, high) = utility.rate;
        let rate = range(&mut self.rng, (low, high));
        let time = ModelTime {
            connect_ms: range(&mut self.rng, (60, 400)),
            first_byte_ms: range(&mut self.rng, (100, 900)),
            streaming_ms: output * 1000 / rate.max(1),
            ..ModelTime::default()
        };
        let end = begins
            + i64::try_from(time.connect_ms + time.first_byte_ms + time.streaming_ms).unwrap_or(0);
        let basis = if session.billing == Billing::Metered {
            CostBasis::Metered
        } else {
            CostBasis::Estimate
        };
        let nanos = price(utility, &Speed::Standard, &tokens);
        self.emit(Record::Request(RequestRecord {
            at: stamp(end),
            session: session.id.clone(),
            turn: turn.clone(),
            started_in: None,
            project: Some(session.project.clone()),
            provider: ProviderId::new(utility.provider),
            model: ModelId::new(utility.id),
            role,
            purpose,
            speed: Speed::Standard,
            speed_requested: None,
            variants: Variants {
                effort: None,
                ..session.variants.clone()
            },
            tokens,
            cost: Some(Cost {
                usd_nanos: nanos,
                basis,
            }),
            catalog_version: Some("2026-11-12".to_owned()),
            billing: session.billing.clone(),
            ttft_ms: Some(time.connect_ms + time.first_byte_ms),
            tokens_per_second: Some(rate as f64),
            time,
            stop: "end".to_owned(),
            error: None,
            retries: 0,
            charged_to: None,
        }));
        end
    }

    /// A judge's request: a model-graded comparison.
    fn judge_request(&mut self, session: &mut Session, turn: &TurnId, begins: i64) -> i64 {
        let judge = model("claude-sonnet-5-5");
        let saved = session.utility;
        session.utility = judge;
        let end = self.utility_request(session, turn, begins, Purpose::Turn, ModelRole::Judge);
        session.utility = saved;
        end
    }

    fn failure_code(&mut self) -> String {
        const CODES: &[&str] = &[
            "rate_limited",
            "overloaded",
            "timeout",
            "server_error",
            "connection_reset",
        ];
        let at = usize::try_from(self.rng.below(CODES.len() as u64)).unwrap_or(0);
        CODES[at].to_owned()
    }

    /// The tool calls one response asked for, run together.
    fn tool_round(
        &mut self,
        session: &Session,
        turn: &TurnId,
        begins: i64,
        stopped_by: Option<TurnOutcome>,
    ) -> Round {
        let calls = range(&mut self.rng, self.scale.tool_batch);
        let weights: u64 = TOOLS.iter().map(|(_, _, weight)| weight).sum();
        let mut round = Round {
            wall: 0,
            tools_ms: 0,
            approval_ms: 0,
            calls,
            edited: false,
            slowest: "read",
        };
        let mut longest = 0;
        let mut waited = 0;
        for call in 0..calls {
            let mut pick = self.rng.below(weights);
            let mut chosen = TOOLS[0];
            for entry in TOOLS {
                if pick < entry.2 {
                    chosen = *entry;
                    break;
                }
                pick -= entry.2;
            }
            let (name, span, _) = chosen;
            let running = skewed(&mut self.rng, span);
            let queued = if call == 0 {
                0
            } else {
                range(&mut self.rng, (0, 30))
            };
            let approval =
                if matches!(name, "shell" | "edit" | "write") && chance(&mut self.rng, 10) {
                    range(&mut self.rng, (2_000, 60_000))
                } else {
                    0
                };
            let outcome = match (&stopped_by, call) {
                (Some(TurnOutcome::Interrupted), 0) => ToolOutcome::Interrupted,
                (Some(TurnOutcome::Denied), 0) => ToolOutcome::Denied,
                _ => match self.rng.below(1000) {
                    0..=29 => ToolOutcome::Failed,
                    30..=44 => ToolOutcome::InvalidArguments,
                    45..=47 => ToolOutcome::UnknownTool,
                    48..=62 => ToolOutcome::Denied,
                    63..=69 => ToolOutcome::Interrupted,
                    _ => ToolOutcome::Ok,
                },
            };
            if matches!(name, "edit" | "write") && outcome == ToolOutcome::Ok {
                round.edited = true;
            }
            let span_ms = queued + running;
            if span_ms >= longest {
                longest = span_ms;
                round.slowest = name;
            }
            waited = waited.max(approval);
            self.emit(Record::Tool(ToolRecord {
                at: stamp(begins + i64::try_from(approval + span_ms).unwrap_or(0)),
                session: session.id.clone(),
                turn: turn.clone(),
                project: Some(session.project.clone()),
                tool: name.to_owned(),
                model: ModelId::new(session.main.id),
                variants: session.variants.clone(),
                outcome,
                running_ms: running,
                queued_ms: queued,
                approval_ms: approval,
            }));
        }
        round.tools_ms = longest;
        round.approval_ms = waited;
        round.wall = longest + waited;
        round
    }

    fn emit(&mut self, record: Record) {
        self.summary.count(&record);
        self.recorder.record(record);
        self.pending += 1;
        if self.pending >= QUEUE / 2 {
            self.recorder.flush();
            self.pending = 0;
        }
    }
}

/// What a round of tool calls took.
struct Round {
    wall: u64,
    tools_ms: u64,
    approval_ms: u64,
    calls: u64,
    edited: bool,
    /// The tool that took longest, whose time the round's is counted to.
    slowest: &'static str,
}
