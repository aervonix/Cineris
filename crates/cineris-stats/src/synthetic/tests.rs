#![allow(
    clippy::unwrap_used,
    clippy::disallowed_methods,
    clippy::print_stdout,
    reason = "a test, which reads the files it checks and prints what it measured"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use cineris_base::LocalDate;
use cineris_testkit::Scratch;

use super::*;
use crate::record::Line;
use crate::store::Aggregates;

fn scratch(test: &str) -> Scratch {
    Scratch::outside("cineris-stats-synthetic", test)
}

/// Writes a month into a fresh directory of the scratch's, and returns the
/// directory with what was said about it.
fn made(scratch: &Scratch, name: &str, spec: &Spec) -> (StatsDir, Summary) {
    let dir = StatsDir::new(scratch.join(name));
    let summary = write(&dir, spec).unwrap();
    (dir, summary)
}

/// Every file of a directory, by name, with its bytes.
fn files(dir: &StatsDir) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(dir.root())
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&path).unwrap())
        })
        .collect()
}

/// Every line of every monthly file, read as the crate reads them.
fn lines(dir: &StatsDir) -> Vec<Line> {
    let mut all = Vec::new();
    for month in dir.months().unwrap() {
        let text = std::fs::read_to_string(dir.events_file(&month)).unwrap();
        for line in text.lines() {
            all.push(serde_json::from_str::<Line>(line).unwrap());
        }
    }
    all
}

/// A scale that is small and has a little of everything: every day worked,
/// every provider, the fast tier, both billings, failed requests and a
/// judge, at rates far above any real user's, so that a day or two show
/// the roles, speeds, billings, providers, failures and cache reads a month
/// holds, and a test of them does not need a month. The kinds only a whole
/// month shows (every outcome, verification and cache difference, a summary
/// on the fast role, a speed's fallback) are left to the ignored
/// `perf_a_heavy_month_has_every_kind_priced`.
const MIXED: Scale = Scale {
    name: "mixed",
    weekday_active: 100,
    weekend_active: 100,
    sessions: (2, 3),
    turns: (3, 6),
    requests: (2, 4),
    tool_batch: (1, 3),
    projects: 3,
    models: &[
        ("claude-opus-5-5", 3),
        ("claude-fable-5-2", 2),
        ("claude-sonnet-5-5", 2),
        ("gpt-6-luna", 1),
        ("openai/gpt-6-luna", 1),
        ("deepseek-v4-pro", 1),
        ("z-ai/glm-5.3-flash", 1),
    ],
    fast_speed: 50,
    subscription: 40,
    failures: 60,
    judge: 20,
};

#[test]
fn one_seed_gives_the_same_bytes() {
    let scratch = scratch("one_seed_gives_the_same_bytes");
    let spec = Spec::month(7, MIXED).with_days(1);
    let (first, one) = made(&scratch, "a", &spec);
    let (second, two) = made(&scratch, "b", &spec);
    assert_eq!(one, two);
    assert!(one.lines > 50, "{}", one.lines);
    let (one_files, two_files) = (files(&first), files(&second));
    assert_eq!(
        one_files.keys().collect::<Vec<_>>(),
        ["aggregates.json", "events-2026-10.jsonl"]
    );
    assert_eq!(one_files, two_files, "not one byte of a file may differ");
}

#[test]
fn another_seed_gives_another_month() {
    let scratch = scratch("another_seed_gives_another_month");
    let (first, one) = made(&scratch, "a", &Spec::month(1, MIXED).with_days(1));
    let (second, two) = made(&scratch, "b", &Spec::month(2, MIXED).with_days(1));
    assert_ne!(one.usage, two.usage);
    assert_ne!(one.totals, two.totals);
    assert_ne!(files(&first), files(&second));
}

/// The summary against what the crate's own reader and aggregates say of
/// the files: by day and model, in total, and for time, tools and outcomes.
fn assert_summary_matches(dir: &StatsDir, read: &[Line], summary: &Summary) {
    let aggregates = Aggregates::load(dir).unwrap();
    let total = aggregates.total();
    assert_eq!(total.unreadable, 0);
    assert_eq!(total.dropped, 0);

    let mut usage: BTreeMap<(String, String), UsageSum> = BTreeMap::new();
    for (key, sum) in &total.usage {
        let into = usage
            .entry((key.period.clone(), key.model.clone()))
            .or_default();
        into.requests += sum.requests;
        into.input += sum.input;
        into.cache_read += sum.cache_read;
        into.cache_write += sum.cache_write;
        into.output += sum.output;
        into.reasoning += sum.reasoning;
        into.metered_nanos += sum.metered_nanos;
        into.estimate_nanos += sum.estimate_nanos;
        into.recorded_nanos += sum.recorded_nanos;
        into.unpriced += sum.unpriced;
    }
    assert_eq!(usage, summary.usage, "usage by day and model");

    let turns: u64 = total.time.values().map(|time| time.turns).sum();
    let wall: u64 = total.time.values().map(|time| time.wall_ms).sum();
    assert_eq!((turns, wall), (summary.turns, summary.wall_ms));

    let mut tools: BTreeMap<String, u64> = BTreeMap::new();
    for (key, tool) in &total.tools {
        *tools.entry(key.tool.clone()).or_default() += tool.calls;
    }
    tools.retain(|_, calls| *calls > 0);
    assert_eq!(tools, summary.tools, "tool calls by tool");

    // Turns are the outcomes with no tool; calls are the others.
    let mut outcomes: BTreeMap<String, u64> = BTreeMap::new();
    for (key, counts) in total.outcomes.iter().filter(|(key, _)| key.tool.is_empty()) {
        let _ = key;
        for (outcome, count) in counts {
            *outcomes.entry(outcome.clone()).or_default() += count;
        }
    }
    assert_eq!(outcomes, summary.outcomes, "turns by outcome");

    // The lines are what the summary counted, kind by kind.
    assert_eq!(read.len() as u64, summary.lines);
    let count =
        |want: fn(&Record) -> bool| read.iter().filter(|line| want(&line.record)).count() as u64;
    assert_eq!(
        count(|record| matches!(record, Record::Request(_))),
        summary.requests
    );
    assert_eq!(
        count(|record| matches!(record, Record::Tool(_))),
        summary.tool_calls
    );
    assert_eq!(
        count(|record| matches!(record, Record::Turn(_))),
        summary.turns
    );
    assert_eq!(
        count(|record| matches!(record, Record::Context(_))),
        summary.contexts
    );
    assert_eq!(
        count(|record| matches!(record, Record::Request(request) if request.error.is_some())),
        summary.failed_requests
    );
    for line in read {
        assert!(
            summary
                .months
                .iter()
                .any(|month| line.day.starts_with(month)),
            "{} is in no month written",
            line.day
        );
        assert_eq!(
            &line.record.at().as_str()[..10],
            line.day,
            "the day a line says is the day its time names, in UTC"
        );
    }
    let days: BTreeSet<&str> = read.iter().map(|line| line.day.as_str()).collect();
    let summed: BTreeSet<&str> = summary.usage.keys().map(|(day, _)| day.as_str()).collect();
    assert!(days.is_superset(&summed));
    assert_eq!(
        days.len() as u32,
        summary.active_days,
        "a line on every active day and on no other"
    );
    assert_eq!(summary.days, summary.active_days + summary.days_off);

    // And the aggregates are what a rebuild from the files gives.
    let today = LocalDate::parse(&format!("{}-28", summary.months.last().unwrap())).unwrap();
    assert_eq!(
        aggregates.total(),
        aggregates.rebuild(dir, today).unwrap().total()
    );
}

#[test]
fn a_small_month_is_what_the_crate_reads_back() {
    let scratch = scratch("a_small_month_is_what_the_crate_reads_back");
    // A week of a light user: some days worked, some not.
    let spec = Spec::month(11, Scale::LIGHT).with_days(7);
    let (dir, light) = made(&scratch, "light", &spec);
    assert_eq!(light.days, 7);
    assert!(light.days_off >= 1, "days off: {}", light.days_off);
    assert!(light.active_days >= 1, "days worked: {}", light.active_days);
    assert_summary_matches(&dir, &lines(&dir), &light);
    // And a day of the mixed scale.
    let (dir, mixed) = made(&scratch, "mixed", &Spec::month(12, MIXED).with_days(1));
    assert_eq!((mixed.days, mixed.days_off), (1, 0));
    assert_summary_matches(&dir, &lines(&dir), &mixed);
}

#[test]
fn each_month_is_a_file_of_its_own() {
    let scratch = scratch("each_month_is_a_file_of_its_own");
    let spec = Spec::year(5, MIXED).with_months(2).with_days(1);
    let (dir, summary) = made(&scratch, "months", &spec);
    assert_eq!(summary.months, ["2026-01", "2026-02"]);
    assert_eq!(dir.months().unwrap(), summary.months);
    assert_eq!(summary.days, 2);
    assert_summary_matches(&dir, &lines(&dir), &summary);
}

/// What a small month of the mixed scale must show: the kinds a reader meets
/// that come with the scale's rates, and the cache read on most requests.
#[test]
fn a_mixed_month_has_its_kinds_and_each_request_is_priced() {
    let scratch = scratch("a_mixed_month_has_its_kinds");
    let (dir, summary) = made(&scratch, "mixed", &Spec::month(31, MIXED).with_days(2));
    let read = lines(&dir);
    let mut roles = BTreeSet::new();
    let mut speeds = BTreeSet::new();
    let mut billings = BTreeSet::new();
    let mut providers = BTreeSet::new();
    let (mut hits, mut retried) = (0, 0);
    for line in &read {
        match &line.record {
            Record::Request(request) => {
                roles.insert(request.role.as_str().to_owned());
                speeds.insert(request.speed.as_str().to_owned());
                billings.insert(request.billing.as_str().to_owned());
                providers.insert(request.provider.as_str().to_owned());
                hits += u64::from(request.tokens.cache_read > 0);
                retried += u64::from(request.retries > 0);
            }
            Record::Dropped { .. } => panic!("an event was dropped"),
            _ => {}
        }
    }
    assert_eq!(
        roles,
        ["fast", "judge", "main"].map(str::to_owned).into(),
        "the roles"
    );
    assert_eq!(speeds.len(), 2, "standard and fast: {speeds:?}");
    assert_eq!(billings.len(), 2, "metered and subscription: {billings:?}");
    assert!(providers.len() >= 3, "{providers:?}");
    assert!(summary.failed_requests > 0 && retried > 0);
    assert_eq!(
        summary.totals.unpriced, summary.failed_requests,
        "a failed request is the one kind with no price"
    );
    assert!(
        hits * 100 > summary.requests * 55,
        "the cache is read: {hits} of {}",
        summary.requests
    );
    assert!(summary.totals.cache_read > 5 * summary.totals.input);
    assert!(summary.totals.reasoning > 0 && summary.totals.reasoning < summary.totals.output);
    assert!(summary.totals.metered_nanos > 0 && summary.totals.estimate_nanos > 0);
    assert_costs_are_priced(&read, 50);
}

/// The scales are what their documentation says, from their parameters: a
/// month of each is not made. The expected requests of a month are the days
/// worked, by the sessions a day, the turns a session and the requests a
/// turn, at the middle of each range.
#[test]
fn the_scales_differ_as_they_are_documented() {
    let mean = |(low, high): (u64, u64)| (low + high) as f64 / 2.0;
    // A 31-day month of the calendar has 22 weekdays and 9 days of weekend.
    let expected = |scale: &Scale| {
        let days =
            22.0 * scale.weekday_active as f64 / 100.0 + 9.0 * scale.weekend_active as f64 / 100.0;
        days * mean(scale.sessions) * mean(scale.turns) * mean(scale.requests)
    };
    let [light, typical, heavy] = Scale::ALL;
    for (small, big) in [(light, typical), (typical, heavy), (heavy, Scale::EXTREME)] {
        assert!(small.weekday_active <= big.weekday_active);
        assert!(small.weekend_active < big.weekend_active);
        assert!(small.projects < big.projects);
        assert!(mean(small.tool_batch) <= mean(big.tool_batch));
        assert!(expected(&small) * 2.0 < expected(&big));
        assert!(small.models.len() <= big.models.len());
    }
    for (scale, low, high) in [
        (&light, 150.0, 300.0),
        (&typical, 1_000.0, 2_000.0),
        (&heavy, 4_000.0, 6_500.0),
        (&Scale::EXTREME, 50_000.0, 150_000.0),
    ] {
        let requests = expected(scale);
        assert!(
            (low..high).contains(&requests),
            "{}: {requests}",
            scale.name
        );
    }
    // A light user has days off most weeks; a heavy one almost none; the
    // fast tier, subscriptions and the judge are the heavy scales' alone.
    assert!(light.weekday_active < 50 && heavy.weekday_active > 95);
    assert_eq!(
        (light.fast_speed, light.subscription, light.judge),
        (0, 0, 0)
    );
    assert!(heavy.fast_speed > 0 && heavy.subscription > 0 && heavy.judge > 0);
    assert_eq!(
        Scale::ALL.map(|scale| scale.name),
        ["light", "typical", "heavy"]
    );
}

/// A heavy month holds every kind of event a reader must meet.
fn assert_every_kind(read: &[Line], summary: &Summary) {
    let mut roles = BTreeSet::new();
    let mut purposes = BTreeSet::new();
    let mut providers = BTreeSet::new();
    let mut speeds = BTreeSet::new();
    let mut billings = BTreeSet::new();
    let mut tool_outcomes = BTreeSet::new();
    let mut turn_outcomes = BTreeSet::new();
    let mut differences = BTreeSet::new();
    let mut verified = BTreeSet::new();
    let mut projects = BTreeSet::new();
    let (mut priced, mut hits, mut retried, mut fell_back) = (0, 0, 0, 0);
    for line in read {
        match &line.record {
            Record::Request(request) => {
                roles.insert(request.role.as_str().to_owned());
                purposes.insert(format!("{:?}", request.purpose));
                providers.insert(request.provider.as_str().to_owned());
                speeds.insert(request.speed.as_str().to_owned());
                billings.insert(request.billing.as_str().to_owned());
                projects.insert(request.project.clone().unwrap());
                priced += u64::from(request.cost.is_some());
                hits += u64::from(request.tokens.cache_read > 0);
                retried += u64::from(request.retries > 0);
                fell_back += u64::from(request.speed_requested.is_some());
            }
            Record::Tool(tool) => {
                tool_outcomes.insert(tool.outcome.as_str());
            }
            Record::Turn(turn) => {
                turn_outcomes.insert(turn.outcome.as_str().to_owned());
                verified.insert(turn.verification.as_ref().map(|v| v.as_str().to_owned()));
            }
            Record::Context(context) => {
                differences.insert(
                    serde_json::to_value(&context.difference).unwrap()["at"]
                        .as_str()
                        .unwrap()
                        .to_owned(),
                );
            }
            Record::Dropped { .. } => panic!("an event was dropped"),
        }
    }
    assert_eq!(
        roles,
        ["fast", "judge", "main"].map(str::to_owned).into(),
        "the roles"
    );
    assert_eq!(
        purposes.len(),
        2,
        "a turn's requests and summaries: {purposes:?}"
    );
    assert_eq!(
        providers.len(),
        5,
        "every provider of the catalog: {providers:?}"
    );
    assert_eq!(speeds.len(), 2, "standard and fast: {speeds:?}");
    assert_eq!(billings.len(), 2, "metered and subscription: {billings:?}");
    assert_eq!(
        tool_outcomes.len(),
        6,
        "every way a call ends: {tool_outcomes:?}"
    );
    assert_eq!(
        turn_outcomes.len(),
        5,
        "every way a turn ends: {turn_outcomes:?}"
    );
    assert_eq!(
        verified.len(),
        4,
        "passed, failed, skipped, and none: {verified:?}"
    );
    assert_eq!(differences.len(), 6, "every place a cache can differ");
    assert_eq!(projects.len(), 9);
    assert!(summary.failed_requests > 20, "{}", summary.failed_requests);
    assert_eq!(priced, summary.requests - summary.failed_requests);
    assert_eq!(summary.totals.unpriced, summary.failed_requests);
    assert!(retried > 0 && fell_back > 0);
    // Most requests read the cache: a conversation grows by its tail.
    assert!(
        hits * 100 > summary.requests * 75,
        "{hits} of {}",
        summary.requests
    );
    assert!(summary.totals.cache_read > 10 * summary.totals.input);
    assert!(summary.totals.reasoning > 0 && summary.totals.reasoning < summary.totals.output);
    assert!(summary.totals.metered_nanos > 0 && summary.totals.estimate_nanos > 0);
    assert_eq!(summary.totals.recorded_nanos, 0);
}

/// The catalog's own file, as text.
fn catalog() -> &'static str {
    include_str!("../../../../catalog/models.toml")
}

/// A model's table in the catalog: the lines from its `[[model]]` to the
/// next.
fn catalog_block(id: &str) -> &'static str {
    catalog()
        .split("[[model]]")
        .find(|block| block.contains(&format!("\nid = \"{id}\"")))
        .unwrap_or_else(|| panic!("{id} is not in the catalog"))
}

/// A price in a block's `[model.prices]` table, in the ten-thousandths of a
/// dollar the generator keeps.
fn catalog_price(block: &str, key: &str) -> u64 {
    let prices = block
        .split("[model.prices]")
        .nth(1)
        .unwrap()
        .split("\n[")
        .next()
        .unwrap();
    let line = prices
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key} = ")))
        .unwrap_or_else(|| panic!("no {key}"));
    let dollars: f64 = line.trim().parse().unwrap();
    (dollars * 10_000.0).round() as u64
}

#[test]
fn the_models_and_prices_are_the_catalogs() {
    for model in MODELS {
        let block = catalog_block(model.id);
        let provider = block
            .lines()
            .find_map(|line| line.strip_prefix("provider = \""))
            .unwrap();
        assert!(
            provider.starts_with(model.provider),
            "{}: {provider}",
            model.id
        );
        let prices = &model.card;
        assert_eq!(catalog_price(block, "input"), prices.input, "{}", model.id);
        assert_eq!(
            catalog_price(block, "output"),
            prices.output,
            "{}",
            model.id
        );
        assert_eq!(
            catalog_price(block, "cache_read"),
            prices.cache_read,
            "{}",
            model.id
        );
        let window: u64 = block
            .lines()
            .find_map(|line| line.strip_prefix("context_window = "))
            .unwrap()
            .replace('_', "")
            .trim()
            .parse()
            .unwrap();
        assert_eq!(window, model.window, "{}", model.id);
        assert_eq!(
            model.fast.is_some(),
            block.contains("name = \"fast\""),
            "{}",
            model.id
        );
        assert!(model_ids().contains(&model.utility));
    }
    for scale in [Scale::LIGHT, Scale::TYPICAL, Scale::HEAVY, Scale::EXTREME] {
        for (id, weight) in scale.models {
            assert!(*weight > 0 && model_ids().contains(id), "{id}");
        }
    }
}

/// Each request's cost is its tokens at the prices of its model and speed.
fn assert_costs_are_priced(read: &[Line], at_least: u64) {
    let mut checked = 0;
    for line in read {
        let Record::Request(request) = &line.record else {
            continue;
        };
        let Some(cost) = request.cost.clone() else {
            assert!(
                request.error.is_some(),
                "only a failed request has no price"
            );
            continue;
        };
        let model = model(request.model.as_str());
        let tokens = &request.tokens;
        let card = match (&request.speed, model.fast) {
            (Speed::Fast, Some(fast)) => fast,
            _ if model.above.is_some()
                && tokens.input + tokens.cache_read + tokens.cache_write > 100_000 =>
            {
                model.above.unwrap()
            }
            _ => model.card,
        };
        // Dollars a million tokens in ten-thousandths: a nano-dollar is a
        // tenth of a unit of token times price.
        let by_hand = tokens.input * card.input / 10
            + tokens.cache_read * card.cache_read / 10
            + tokens.cache_write * card.cache_write / 10
            + tokens.output * card.output / 10;
        // Each product is cut to a whole nano-dollar on its own by hand and
        // the sum is by the generator: they differ by under 4 units.
        assert!(by_hand.abs_diff(cost.usd_nanos) < 4, "{request:?}");
        let expected = if request.billing == Billing::Metered {
            CostBasis::Metered
        } else {
            CostBasis::Estimate
        };
        assert_eq!(cost.basis, expected);
        checked += 1;
    }
    assert!(checked >= at_least, "{checked} requests priced");
}

// The measurements. Each makes whole months, which is what costs: they are
// ignored in every ordinary run, and run by hand with
// `cargo xtask test -p cineris-stats --run-ignored only perf_ --no-capture`,
// or on a runner with `try -f command="perf -p cineris-stats"`. They print
// their times and assert on none of them.

/// Light, typical and heavy months, made and timed, each read back and
/// compared with its summary.
#[test]
#[ignore = "a measurement, run by hand"]
fn perf_the_scales_made_and_timed() {
    let scratch = scratch("perf_the_scales_made_and_timed");
    let mut by_scale = Vec::new();
    for scale in Scale::ALL {
        let started = Instant::now();
        let (dir, summary) = made(&scratch, scale.name, &Spec::month(21, scale));
        let took = started.elapsed();
        println!(
            "{:8} {:>3} days worked, {:>4} sessions, {:>5} turns, {:>6} requests ({} failed), \
             {:>6} tool calls, {:>6} contexts, {:>6} lines, {:>6} KB, ${:.2} metered, ${:.2} estimated, made in {} ms",
            scale.name,
            summary.active_days,
            summary.sessions,
            summary.turns,
            summary.requests,
            summary.failed_requests,
            summary.tool_calls,
            summary.contexts,
            summary.lines,
            dir.size() / 1000,
            summary.totals.metered_nanos as f64 / 1e9,
            summary.totals.estimate_nanos as f64 / 1e9,
            took.as_millis(),
        );
        assert_summary_matches(&dir, &lines(&dir), &summary);
        by_scale.push(summary);
    }
    let [light, typical, heavy] = &by_scale[..] else {
        panic!("three scales");
    };
    for (small, big) in [(light, typical), (typical, heavy)] {
        assert!(small.active_days < big.active_days);
        assert!(small.sessions * 3 < big.sessions * 2);
        assert!(small.turns * 2 < big.turns);
        assert!(small.requests * 2 < big.requests);
        assert!(small.tool_calls * 2 < big.tool_calls);
        assert!(small.totals.output * 2 < big.totals.output);
        assert!(small.totals.cache_read * 2 < big.totals.cache_read);
    }
    assert!((120..600).contains(&light.requests), "{}", light.requests);
    assert!(
        (900..2_500).contains(&typical.requests),
        "{}",
        typical.requests
    );
    assert!(
        (3_500..8_000).contains(&heavy.requests),
        "{}",
        heavy.requests
    );
    assert!(light.days_off >= 12, "{}", light.days_off);
    assert!(heavy.days_off <= 6, "{}", heavy.days_off);
}

/// A heavy month, made once: every kind in it, every cost, and how long it
/// takes to make, written and brought up to date, in the build this test
/// has.
#[test]
#[ignore = "a measurement, run by hand"]
fn perf_a_heavy_month_has_every_kind_priced() {
    let scratch = scratch("perf_a_heavy_month_has_every_kind_priced");
    let dir = StatsDir::new(scratch.join("heavy"));
    let started = Instant::now();
    let summary = write(&dir, &Spec::month(31, Scale::HEAVY)).unwrap();
    let took = started.elapsed();
    println!(
        "heavy month: {} lines, {} requests, {} KB of events, in {} ms",
        summary.lines,
        summary.requests,
        dir.size() / 1000,
        took.as_millis()
    );
    assert!(Path::new(&dir.events_file("2026-10")).exists());
    let read = lines(&dir);
    assert_every_kind(&read, &summary);
    assert_costs_are_priced(&read, 4_000);
    assert_summary_matches(&dir, &read, &summary);
}

/// A year of a light user: twelve files, adding up.
#[test]
#[ignore = "a measurement, run by hand"]
fn perf_a_year_is_twelve_files_and_adds_up() {
    let scratch = scratch("perf_a_year_is_twelve_files_and_adds_up");
    let started = Instant::now();
    let (dir, summary) = made(&scratch, "year", &Spec::year(5, Scale::LIGHT));
    println!(
        "a light year: {} lines in {} ms",
        summary.lines,
        started.elapsed().as_millis()
    );
    assert_eq!(summary.months.len(), 12);
    assert_eq!(dir.months().unwrap(), summary.months);
    assert_eq!(summary.days, 365);
    assert_summary_matches(&dir, &lines(&dir), &summary);
}

/// What step 24's usage panel's year is made of: every model of the
/// generator (eleven, of five providers), twenty projects, and the fast
/// tier where a model has one, on a heavy user's days.
const PANEL_YEAR: Scale = Scale {
    name: "panel-year",
    projects: 20,
    models: &[
        ("claude-opus-5-5", 5),
        ("claude-fable-5-2", 3),
        ("claude-sonnet-5-5", 3),
        ("claude-haiku-5-5", 2),
        ("gpt-6-luna", 1),
        ("z-ai/glm-5.3-flash", 1),
        ("openai/gpt-6-luna", 1),
        ("anthropic/claude-haiku-5.5", 1),
        ("deepseek-flash", 1),
        ("deepseek-v4-pro", 1),
        ("deepseek-v4-flash-0731", 1),
    ],
    ..Scale::HEAVY
};

/// The core's half of the usage panel's first frame with a year of
/// history (`docs/performance-budgets.md`: 16 ms from the aggregates, from
/// the key to the frame with the rows): `usage/report` as the core makes it
/// (`cineris-core`'s `usage::report`, on the blocking pool): the aggregates
/// read, brought up to date with the files, the limits read, and the
/// report made, for each range and grouping the panel asks for. The
/// application's half is `cineris-tui`'s
/// `perf_the_usage_panels_first_frame_with_a_year`. A measurement, in a
/// release build:
///
/// ```text
/// cargo test --release -p cineris-stats --lib perf_a_years_report -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, run by hand"]
fn perf_a_years_report_for_the_usage_panel() {
    use cineris_protocol::usage::{UsageGroup, UsageRange};

    let scratch = scratch("perf_a_years_report_for_the_usage_panel");
    let started = Instant::now();
    let (dir, summary) = made(&scratch, "year", &Spec::year(24, PANEL_YEAR));
    println!(
        "a year for the panel: {} days, {} requests, {} KB of events, made in {} s",
        summary.days,
        summary.requests,
        dir.size() / 1000,
        started.elapsed().as_secs()
    );
    // The end of the year made, as the core's clock would have it.
    let clock = FakeClock::starting_at("2026-12-31T18:00:00Z").unwrap();
    let now = cineris_base::Clock::now(&clock);
    let zone = Zone::utc();
    // The first report brings the aggregates up to date with every month;
    // the writer keeps them so, and the panel's reports after it find them
    // current.
    let mut first = Aggregates::load_for_saving(&dir, &clock).unwrap();
    first
        .sync(&dir, zone.date_of(now), &crate::month_of(now), Some(400))
        .unwrap();
    first.save(&dir, &clock).unwrap();
    let ranges = [
        UsageRange::Today,
        UsageRange::Week,
        UsageRange::Month,
        UsageRange::All,
    ];
    let groups: [&[UsageGroup]; 4] = [
        &[UsageGroup::Provider, UsageGroup::Model],
        &[UsageGroup::Project],
        &[UsageGroup::Speed],
        &[UsageGroup::Project, UsageGroup::Provider],
    ];
    let mut times = Vec::new();
    // Where each report's time goes: the aggregates read, brought up to
    // date with the files, and the report made (the limits read with it).
    let (mut loads, mut syncs, mut makes) = (Vec::new(), Vec::new(), Vec::new());
    let mut most_rows = 0;
    for _ in 0..20 {
        for range in &ranges {
            for group in groups {
                let began = Instant::now();
                let mut aggregates = Aggregates::load_for_saving(&dir, &clock).unwrap();
                let loaded = Instant::now();
                aggregates
                    .sync(&dir, zone.date_of(now), &crate::month_of(now), Some(400))
                    .unwrap();
                let synced = Instant::now();
                let total = aggregates.total();
                let limits = crate::LimitsFile::load(&dir);
                let query = crate::Query {
                    range: range.clone(),
                    group_by: group.to_vec(),
                    project: None,
                    week_start: 0,
                    rolling: false,
                };
                let report = crate::report(&total, &query, &zone, now, &limits.providers, true);
                let ended = Instant::now();
                let ms = |from: Instant, to: Instant| (to - from).as_secs_f64() * 1000.0;
                times.push(ms(began, ended));
                loads.push(ms(began, loaded));
                syncs.push(ms(loaded, synced));
                makes.push(ms(synced, ended));
                most_rows = most_rows.max(report.rows.len());
            }
        }
    }
    times.sort_by(f64::total_cmp);
    for part in [&mut loads, &mut syncs, &mut makes] {
        part.sort_by(f64::total_cmp);
    }
    let median = |part: &[f64]| part[part.len() / 2];
    println!(
        "  the aggregates read: median {:.3} ms   brought up to date: median {:.3} ms   \
         the report made, the limits read: median {:.3} ms",
        median(&loads),
        median(&syncs),
        median(&makes)
    );
    let at = |share: usize| times[(times.len() * share / 100).min(times.len() - 1)];
    println!(
        "a report from the year's aggregates, {} reports of up to {most_rows} rows, aggregates of {} KB: \
         median {:.3} ms   p99 {:.3} ms   most {:.3} ms",
        times.len(),
        std::fs::metadata(dir.aggregates_file()).map_or(0, |meta| meta.len() / 1000),
        at(50),
        at(99),
        times[times.len() - 1]
    );
    cineris_testkit::numbers::report("usage_panel.core_report_p99", at(99), "ms");

    // As the core reports since step 24's third part: the aggregates kept
    // from the last report, read again only when another process saved the
    // file (`cineris-core`'s `usage::AggregatesCache`; telling the file
    // apart is one `stat`).
    let mut kept = Aggregates::load_for_saving(&dir, &clock).unwrap();
    let mut kept_times = Vec::new();
    for _ in 0..20 {
        for range in &ranges {
            for group in groups {
                let began = Instant::now();
                let _ = std::fs::metadata(dir.aggregates_file());
                kept.sync(&dir, zone.date_of(now), &crate::month_of(now), Some(400))
                    .unwrap();
                let total = kept.total();
                let limits = crate::LimitsFile::load(&dir);
                let query = crate::Query {
                    range: range.clone(),
                    group_by: group.to_vec(),
                    project: None,
                    week_start: 0,
                    rolling: false,
                };
                let _ = crate::report(&total, &query, &zone, now, &limits.providers, true);
                kept_times.push((Instant::now() - began).as_secs_f64() * 1000.0);
            }
        }
    }
    kept_times.sort_by(f64::total_cmp);
    let kept_at =
        |share: usize| kept_times[(kept_times.len() * share / 100).min(kept_times.len() - 1)];
    println!(
        "a report from the aggregates kept: median {:.3} ms   p99 {:.3} ms   most {:.3} ms",
        kept_at(50),
        kept_at(99),
        kept_times[kept_times.len() - 1]
    );
    cineris_testkit::numbers::report("usage_panel.core_report_kept_p99", kept_at(99), "ms");
}
