#![allow(
    clippy::unwrap_used,
    clippy::disallowed_methods,
    clippy::print_stdout,
    reason = "a test, which reads the files it checks and prints what it measured"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use cortex_base::LocalDate;
use cortex_testkit::Scratch;

use super::*;
use crate::record::Line;
use crate::store::Aggregates;

fn scratch(test: &str) -> Scratch {
    Scratch::outside("cortex-stats-synthetic", test)
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

#[test]
fn one_seed_gives_the_same_bytes() {
    let scratch = scratch("one_seed_gives_the_same_bytes");
    let spec = Spec::month(7, Scale::TYPICAL);
    let (first, one) = made(&scratch, "a", &spec);
    let (second, two) = made(&scratch, "b", &spec);
    assert_eq!(one, two);
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
    let (first, one) = made(&scratch, "a", &Spec::month(1, Scale::TYPICAL));
    let (second, two) = made(&scratch, "b", &Spec::month(2, Scale::TYPICAL));
    assert_ne!(one.usage, two.usage);
    assert_ne!(one.totals, two.totals);
    assert_ne!(files(&first), files(&second));
    // The shape is the scale's, whatever the seed.
    for summary in [&one, &two] {
        assert!(
            summary.active_days > 15 && summary.active_days < 31,
            "{summary:?}"
        );
    }
}

#[test]
fn the_same_seed_makes_the_same_months_whichever_scale_it_follows() {
    // The seed is the whole of the randomness: nothing of the process's
    // state, the order of tests or the time leaks into the month.
    let scratch = scratch("the_same_seed_makes_the_same_months");
    let spec = Spec::month(3, Scale::LIGHT);
    let (_, before) = made(&scratch, "a", &Spec::month(9, Scale::TYPICAL));
    let (_, one) = made(&scratch, "b", &spec);
    let (_, two) = made(&scratch, "c", &spec);
    assert!(before.requests > one.requests);
    assert_eq!(one, two);
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
fn a_year_is_twelve_files_and_adds_up() {
    let scratch = scratch("a_year_is_twelve_files_and_adds_up");
    let (dir, summary) = made(&scratch, "year", &Spec::year(5, Scale::LIGHT));
    assert_eq!(summary.months.len(), 12);
    assert_eq!(dir.months().unwrap(), summary.months);
    assert_eq!(summary.days, 365);
    assert_summary_matches(&dir, &lines(&dir), &summary);
}

#[test]
fn a_light_and_a_typical_month_are_what_the_crate_reads_back() {
    let scratch = scratch("a_light_and_a_typical_month");
    for (seed, scale) in [(11, Scale::LIGHT), (12, Scale::TYPICAL)] {
        let (dir, summary) = made(&scratch, scale.name, &Spec::month(seed, scale));
        assert_eq!(summary.scale, scale.name);
        assert_summary_matches(&dir, &lines(&dir), &summary);
    }
}

#[test]
fn the_scales_differ_as_they_are_documented() {
    let scratch = scratch("the_scales_differ_as_they_are_documented");
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
        assert_eq!(summary.scale, scale.name);
        by_scale.push(summary);
    }
    let [light, typical, heavy] = &by_scale[..] else {
        panic!("three scales");
    };
    // Each scale is bigger than the one before in everything it counts.
    for (small, big) in [(light, typical), (typical, heavy)] {
        assert!(small.active_days < big.active_days);
        assert!(small.sessions * 3 < big.sessions * 2);
        assert!(small.turns * 2 < big.turns);
        assert!(small.requests * 2 < big.requests);
        assert!(small.tool_calls * 2 < big.tool_calls);
        assert!(small.totals.output * 2 < big.totals.output);
        assert!(small.totals.cache_read * 2 < big.totals.cache_read);
    }
    // And is in the range its documentation gives.
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
    // A light user has days off most weeks; a heavy one almost none.
    assert!(light.days_off >= 12, "{}", light.days_off);
    assert!(heavy.days_off <= 6, "{}", heavy.days_off);
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

#[test]
fn a_months_events_stay_in_its_file_by_the_day_they_fell_on() {
    let scratch = scratch("a_months_events_stay_in_its_file");
    let (dir, _) = made(&scratch, "month", &Spec::month(41, Scale::TYPICAL));
    for line in lines(&dir) {
        assert!(line.day.starts_with("2026-10-"), "{}", line.day);
        assert_eq!(
            &line.record.at().as_str()[..10],
            line.day,
            "the day a line says is the day its time names, in UTC"
        );
    }
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
fn assert_costs_are_priced(read: &[Line]) {
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
    assert!(checked > 4_000);
}

/// A heavy month, made once: what it holds, what it costs, and how long it
/// takes to make, written and brought up to date, in the build this test
/// has.
#[test]
fn a_heavy_month_has_every_kind_priced_and_is_made_in_a_moment() {
    let scratch = scratch("a_heavy_month_has_every_kind");
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
    assert!(took.as_secs() < 3, "{took:?}");
    assert!(Path::new(&dir.events_file("2026-10")).exists());
    let read = lines(&dir);
    assert_every_kind(&read, &summary);
    assert_costs_are_priced(&read);
    assert_summary_matches(&dir, &read, &summary);
}
