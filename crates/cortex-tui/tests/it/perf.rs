//! Measurements, ignored in ordinary runs: run by hand, optimised, with
//! `cargo nextest run -p cortex-tui --release --run-ignored only -E
//! 'test(/measure_/)' --no-capture`. The numbers are in
//! `docs/performance-budgets.md`.

use std::time::{Duration, Instant};

use cortex_testkit::{Rng, SeededRng};
use cortex_tui::complete::{Candidates, Filter, KEPT};

/// `count` paths shaped like a large workspace's: a few levels of folders
/// from a vocabulary, then a file name with an extension.
fn workspace(count: usize) -> Vec<String> {
    const WORDS: [&str; 24] = [
        "src", "lib", "core", "tests", "docs", "util", "parser", "lexer", "render", "net",
        "config", "model", "view", "store", "io", "cache", "ffi", "macros", "bench", "scan",
        "number", "token", "session", "agent",
    ];
    const EXTENSIONS: [&str; 6] = ["rs", "md", "toml", "json", "ts", "py"];
    let mut rng = SeededRng::new(21);
    let mut pick = |n: usize| usize::try_from(rng.next_u64() % n as u64).unwrap();
    (0..count)
        .map(|index| {
            let depth = 1 + pick(5);
            let mut path = String::new();
            for _ in 0..depth {
                path.push_str(WORDS[pick(WORDS.len())]);
                path.push('/');
            }
            path.push_str(WORDS[pick(WORDS.len())]);
            path.push('_');
            path.push_str(WORDS[pick(WORDS.len())]);
            path.push_str(&format!("{index}."));
            path.push_str(EXTENSIONS[pick(EXTENSIONS.len())]);
            path
        })
        .collect()
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    let rank = (p * (sorted.len() - 1) as f64).round();
    sorted[rank as usize]
}

#[test]
#[ignore = "a measurement: run by hand, optimised"]
fn measure_fuzzy_matching_on_100k_paths() {
    let paths = workspace(100_000);
    let started = Instant::now();
    let candidates = Candidates::new(paths);
    let prepared = started.elapsed();
    let queries = ["scannum", "srclexer", "docsmodel", "agentsess", "ffimacro"];
    let mut keystrokes = Vec::new();
    let mut first = Vec::new();
    for _ in 0..5 {
        for query in queries {
            let mut filter = Filter::new();
            for end in 1..=query.len() {
                let started = Instant::now();
                let hits = filter.run(&candidates, &query[..end], KEPT);
                let took = started.elapsed();
                assert!(hits.len() <= KEPT);
                if end == 1 {
                    first.push(took);
                }
                keystrokes.push(took);
            }
        }
    }
    // A whole query typed at once, with nothing to narrow from.
    let mut whole = Vec::new();
    for query in queries {
        let started = Instant::now();
        let _ = Filter::new().run(&candidates, query, KEPT);
        whole.push(started.elapsed());
    }
    keystrokes.sort();
    first.sort();
    whole.sort();
    println!(
        "100,000 paths: prepared in {prepared:?}; a keystroke median {:?}, 99th percentile {:?}, \
         most {:?}; the first character median {:?}, most {:?}; a whole query from nothing most {:?}",
        percentile(&keystrokes, 0.5),
        percentile(&keystrokes, 0.99),
        keystrokes.last().unwrap(),
        percentile(&first, 0.5),
        first.last().unwrap(),
        whole.last().unwrap(),
    );
}

// ------------------------------------------------------------ key latency

use cortex_testkit::VTerm;
use cortex_testkit::pty::Session;

use crate::demo::STATUS_MARK;

/// What starts the input box's first row: its border, a space, `◆` and a
/// space; in the ASCII glyph set, which the old console host and ConPTY
/// get, `| > `.
const PROMPTS: [&str; 2] = ["\u{2502} \u{25C6} ", "| > "];

const COLS: u16 = 120;
const ROWS: u16 = 40;
const LIMIT: Duration = Duration::from_secs(30);

/// The program and arguments that run the editor demo with `options`:
/// `CORTEX_PERF_EDITOR_DEMO` names an optimised build of it (`cargo xtask
/// perf` sets it), and otherwise this test binary is the demo.
fn demo(options: &[&str]) -> (String, Vec<String>) {
    match std::env::var("CORTEX_PERF_EDITOR_DEMO") {
        Ok(program) if !program.is_empty() => {
            (program, options.iter().map(|o| (*o).to_owned()).collect())
        }
        _ => (
            crate::children::this_binary(),
            crate::children::child_arguments(crate::DEMO, &options.join(",")),
        ),
    }
}

fn start(options: &[&str]) -> Session {
    let (program, arguments) = demo(options);
    Session::start(&program, &arguments, COLS, ROWS).answering()
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn quantile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let rank = (p / 100.0 * (sorted.len() - 1) as f64).round();
    sorted[(rank as usize).min(sorted.len() - 1)]
}

fn sorted(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(f64::total_cmp);
    values
}

/// The text on the input box's first row.
fn editor(term: &VTerm) -> Option<String> {
    term.screen().into_iter().find_map(|row| {
        let rest = PROMPTS.iter().find_map(|prompt| row.strip_prefix(prompt))?;
        if rest.starts_with(cortex_tui::view::input::PLACEHOLDER) {
            return Some(String::new());
        }
        Some(
            rest.trim_end()
                .trim_end_matches(['\u{2502}', '|'])
                .trim_end()
                .to_owned(),
        )
    })
}

/// The demo's own key timings: from reading a key to having written the
/// frame that shows it, in milliseconds.
fn demo_keys(path: &std::path::Path) -> Vec<f64> {
    let text = std::fs::read_to_string(path).expect("the demo's statistics");
    let keys = text.split("\"keys\":[").nth(1).expect("keys");
    let keys = &keys[..keys.find(']').expect("the end of the keys")];
    keys.split(',')
        .filter(|n| !n.is_empty())
        .map(|n| n.parse::<f64>().expect("a number") / 1000.0)
        .collect()
}

fn quit(session: &mut Session) {
    session.send(b"\x03");
    session.send(b"\x04");
    session.wait_exit(LIMIT);
}

/// Types `count` keys one at a time after `prefix`, each when the last has
/// shown: the latencies, and the demo's own share.
fn idle_keys(
    count: usize,
    prefix: &str,
    options: &[&str],
    stats: &std::path::Path,
) -> (Vec<f64>, Vec<f64>) {
    let stats_option = format!("stats={}", stats.display());
    let mut all = vec!["rate=400", stats_option.as_str()];
    all.extend_from_slice(options);
    let mut session = start(&all);
    session.wait_for(LIMIT, "the first frame", |t| t.shows(STATUS_MARK));
    let mut typed = String::new();
    for c in prefix.chars() {
        typed.push(c);
        session.send(c.to_string().as_bytes());
        session.wait_for(LIMIT, "the prefix", |t| {
            editor(t).as_deref() == Some(typed.as_str())
        });
    }
    let mut latencies = Vec::new();
    for index in 0..count {
        // The first row holds the text the reader looks at: start it again
        // before it fills.
        if typed.len() >= 90 {
            session.send(b"\x15");
            session.wait_for(LIMIT, "the prompt cleared", |t| {
                editor(t).as_deref() == Some("")
            });
            typed.clear();
        }
        let c = char::from(b'a' + u8::try_from(index % 26).unwrap_or(0));
        typed.push(c);
        let sent = session.send(c.to_string().as_bytes());
        let shown = session.wait_for(LIMIT, "a key's echo", |t| {
            editor(t).as_deref() == Some(typed.as_str())
        });
        latencies.push(millis(shown.saturating_duration_since(sent)));
    }
    quit(&mut session);
    let mut own = demo_keys(stats);
    own.drain(..prefix.chars().count().min(own.len()));
    (latencies, own)
}

/// Streams 2,000 tokens a second for `seconds`, typing a key every 137 ms:
/// the latencies, the demo's own share, and the keys never shown.
fn streaming_keys(seconds: u64, stats: &std::path::Path) -> (Vec<f64>, Vec<f64>, usize) {
    let turn = format!("turn-ms={}", seconds * 1000);
    let stats_option = format!("stats={}", stats.display());
    let mut session = start(&["rate=2000", "auto", "quit-after-turn", &turn, &stats_option]);
    session.wait_for(LIMIT, "the first frame", |t| t.shows(STATUS_MARK));
    let every = Duration::from_millis(137);
    let mut next = Instant::now() + every;
    let mut typed = String::new();
    let mut sent: Vec<Instant> = Vec::new();
    let mut shown: Vec<Option<Instant>> = Vec::new();
    let end = Instant::now() + Duration::from_secs(seconds);
    while !session.ended() && Instant::now() < end + Duration::from_secs(5) {
        let now = Instant::now();
        if now >= next && now < end && typed.len() < 100 {
            let c = char::from(b'a' + u8::try_from(typed.len() % 26).unwrap_or(0));
            typed.push(c);
            sent.push(session.send(c.to_string().as_bytes()));
            shown.push(None);
            next += every;
        }
        let wait = next
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(1));
        if let Some(at) = session.step(wait)
            && let Some(text) = editor(session.screen())
        {
            for (index, slot) in shown.iter_mut().enumerate() {
                if slot.is_none() && text.len() > index {
                    *slot = Some(at);
                }
            }
        }
    }
    session.wait_exit(LIMIT);
    let keys = sent
        .iter()
        .zip(&shown)
        .filter_map(|(sent, shown)| {
            shown.map(|shown| millis(shown.saturating_duration_since(*sent)))
        })
        .collect();
    let lost = shown.iter().filter(|s| s.is_none()).count();
    (keys, demo_keys(stats), lost)
}

fn row(out: &mut String, name: &str, values: &[f64], budget: &str) {
    let values = sorted(values.to_vec());
    out.push_str(&format!(
        "| {name} | {:.2} | {:.2} | {:.2} | {budget} |\n",
        quantile(&values, 50.0),
        quantile(&values, 99.0),
        values.last().copied().unwrap_or(f64::NAN),
    ));
}

#[test]
#[ignore = "a measurement: `cargo xtask perf` runs it, optimised"]
fn perf_the_editor_against_the_key_budgets() {
    let scratch = cortex_testkit::Scratch::outside("cortex-tui-perf", "keys");
    let seconds = std::env::var("CORTEX_PERF_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let (idle, idle_own) = idle_keys(200, "", &[], &scratch.join("idle.json"));
    let (stream, stream_own, lost) = streaming_keys(seconds, &scratch.join("stream.json"));
    let (menu, menu_own) = idle_keys(24, "@", &["files=100000"], &scratch.join("menu.json"));
    let mut out = String::from(
        "| Key to frame (ms) | median | 99th | most | budget |\n|---|---|---|---|---|\n",
    );
    row(&mut out, "idle", &idle, "2 at the 99th");
    row(&mut out, "idle, the harness's share", &idle_own, "");
    row(
        &mut out,
        "streaming 2,000 tokens/s",
        &stream,
        "4 at the 99th",
    );
    row(&mut out, "streaming, the harness's share", &stream_own, "");
    row(&mut out, "in an @ menu over 100,000 paths", &menu, "");
    row(&mut out, "@ menu, the harness's share", &menu_own, "");
    out.push_str(&format!("\nkeys never shown while streaming: {lost}\n"));
    println!("{out}");
    if let Ok(path) = std::env::var("CORTEX_PERF_JSON") {
        std::fs::write(path, &out).expect("write the results");
    }
    assert_eq!(lost, 0, "a key typed while streaming never showed");
}
