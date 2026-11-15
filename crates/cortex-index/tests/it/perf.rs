//! The index's budgets of `docs/performance-budgets.md`, "Data paths": a
//! warm map in at most 150 ms, and a cold index of 20,000 files in at most
//! 2 s of background time per pass; with the caps on a tree of 100,000
//! files, the extractors' speed per language, the cache's size and the
//! memory a pass uses.
//!
//! These are measurements, not blocking tests: making the trees takes
//! longer than a test may. They are ignored by default and run by hand, in
//! a release build, outside nextest:
//!
//! ```text
//! cargo test --release -p cortex-index --test it perf_ -- --ignored --nocapture --test-threads 1
//! ```
//!
//! `CORTEX_PERF_TREE` names a real tree to index as well (this repository,
//! say), and `CORTEX_PERF_FILES` the size of the generated one (20,000
//! unless given). The numbers it prints go into the page above.

use std::sync::Arc;
use std::time::{Duration, Instant};

use cortex_base::{AbsPath, CancelToken, PathOptions, PathResolver, SystemClock};
use cortex_index::{Index, Language, Limits, MapRequest, OpenGate, extract};

use super::fixtures;
use super::index::Tree;

fn number_from_env(variable: &str, default: usize) -> usize {
    std::env::var(variable)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

/// A source file of about a kilobyte in one of four languages.
fn source(n: usize) -> (String, String) {
    match n % 4 {
        0 => (
            format!("f{n}.rs"),
            format!(
                "/// Item {n}.\npub struct S{n} {{\n    a: u32,\n}}\n\nimpl S{n} {{\n    pub fn new() -> Self {{ Self {{ a: {n} }} }}\n\n    pub fn get(&self) -> u32 {{\n        self.a\n    }}\n}}\n\npub fn helper_{n}(x: u32) -> u32 {{\n    x + {n}\n}}\n{}",
                "// padding line to make the file look like real code\n".repeat(12)
            ),
        ),
        1 => (
            format!("f{n}.py"),
            format!(
                "class C{n}:\n    \"\"\"Item {n}.\"\"\"\n\n    def __init__(self):\n        self.a = {n}\n\n    def get(self):\n        return self.a\n\n\ndef helper_{n}(x):\n    return x + {n}\n{}",
                "# padding line to make the file look like real code\n".repeat(14)
            ),
        ),
        2 => (
            format!("f{n}.ts"),
            format!(
                "/** Item {n}. */\nexport class C{n} {{\n  private a = {n};\n\n  get(): number {{\n    return this.a;\n  }}\n}}\n\nexport function helper{n}(x: number): number {{\n  return x + {n};\n}}\n{}",
                "// padding line to make the file look like real code\n".repeat(14)
            ),
        ),
        _ => (
            format!("f{n}.go"),
            format!(
                "package p\n\n// T{n} is an item.\ntype T{n} struct {{\n\ta int\n}}\n\nfunc (t *T{n}) Get() int {{\n\treturn t.a\n}}\n\nfunc Helper{n}(x int) int {{\n\treturn x + {n}\n}}\n{}",
                "// padding line to make the file look like real code\n".repeat(14)
            ),
        ),
    }
}

/// A tree of `files` source files, a hundred to a directory, ten
/// directories to a parent.
fn generated(test: &str, files: usize) -> Tree {
    let tree = Tree::new(cortex_testkit::scratch!(test));
    for n in 0..files {
        let (name, text) = source(n);
        let dir = n / 100;
        let path = tree
            .scratch
            .file(format!("ws/d{:03}/s{dir:04}/{name}", dir / 10), text);
        // Settled: modified well before the pass, as a tree at rest is.
        std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| {
                file.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
            })
            .unwrap();
    }
    tree
}

fn system_index(root: AbsPath, dir: std::path::PathBuf, limits: Limits) -> Index {
    Index::new(root, dir, limits, Arc::new(OpenGate), Arc::new(SystemClock::new()))
}

#[test]
#[ignore = "a measurement, run by hand in a release build: see the module documentation"]
fn perf_a_cold_index_a_warm_one_and_a_warm_map() {
    let files = number_from_env("CORTEX_PERF_FILES", 20_000);
    let tree = generated("perf_cold_warm", files);
    let dir = tree.cache_dir();
    let index = system_index(tree.root.clone(), dir, Limits::default());
    let rss_before = crate::resident_kib("VmRSS:");

    let start = Instant::now();
    let cold = index.pass(&CancelToken::new(), &|| false);
    let cold_ms = ms(start.elapsed());
    println!(
        "cold pass of {files} files: {cold_ms:.0} ms, read {}, indexed {}, partial {:?}, cache {} bytes ({:.1} KiB a file)",
        cold.read,
        cold.indexed,
        cold.partial,
        cold.cache_bytes,
        cold.cache_bytes as f64 / 1024.0 / cold.indexed.max(1) as f64
    );
    if let (Some(before), Some(peak)) = (rss_before, crate::resident_kib("VmHWM:")) {
        println!(
            "resident: {before} KiB before, at most {peak} KiB during it, {:?} KiB after",
            crate::resident_kib("VmRSS:")
        );
    }

    // A warm start: few changes, then the map, as a session's first prompt
    // would have them.
    let mut warm = Vec::new();
    let mut maps = Vec::new();
    for round in 0..15 {
        let changed = tree.scratch.file(
            format!("ws/d000/s0000/f{round}.rs"),
            format!("pub fn changed_{round}() {{}}\n"),
        );
        std::fs::File::options()
            .write(true)
            .open(&changed)
            .and_then(|file| {
                file.set_modified(std::time::SystemTime::now() - Duration::from_secs(60))
            })
            .unwrap();
        let start = Instant::now();
        let report = index.pass(&CancelToken::new(), &|| false);
        let passed = start.elapsed();
        let map = index.map(&MapRequest::new()).expect("a map");
        warm.push(ms(start.elapsed()));
        maps.push(ms(start.elapsed() - passed));
        assert!(report.read <= 1, "{report:?}");
        assert!(map.len() <= cortex_index::DEFAULT_TOKENS * 3);
    }
    println!(
        "warm pass with one change and the map: median {:.1} ms (map alone {:.1} ms), slowest {:.1} ms",
        median(warm.clone()),
        median(maps),
        warm.iter().copied().fold(0.0, f64::max)
    );
}

#[test]
#[ignore = "a measurement, run by hand in a release build: see the module documentation"]
fn perf_the_caps_hold_on_a_tree_of_100000_files() {
    let tree = generated("perf_caps", 100_000);
    let index = system_index(tree.root.clone(), tree.cache_dir(), Limits::default());
    let start = Instant::now();
    let report = index.pass(&CancelToken::new(), &|| false);
    println!(
        "100,000 files: {:.0} ms, found {}, read {}, indexed {}, partial {:?}",
        ms(start.elapsed()),
        report.found,
        report.read,
        report.indexed,
        report.partial
    );
    assert!(report.indexed <= 20_000);
    assert!(!report.complete());
    let start = Instant::now();
    let map = index.map(&MapRequest::new()).expect("a map");
    println!("its map: {:.1} ms, {} bytes, says partial: {}", ms(start.elapsed()), map.len(), map.contains("partial"));
    assert!(map.contains("The index is partial"));
}

#[test]
#[ignore = "a measurement, run by hand in a release build: see the module documentation"]
fn perf_extraction_speed_per_language() {
    let dir = fixtures().join("outlines");
    let mut entries: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(entry.path()).unwrap();
        // About four megabytes of the fixture, repeated.
        let copies = (4 * 1024 * 1024 / text.len().max(1)).max(1);
        let big = text.repeat(copies);
        let language = Language::detect(&name, &text);
        let mut speeds = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            let outline = extract(language, &name, &big);
            std::hint::black_box(&outline);
            speeds.push(big.len() as f64 / 1_048_576.0 / start.elapsed().as_secs_f64());
        }
        println!(
            "{:<12} {:>7.0} MiB/s  ({} bytes, {})",
            name,
            median(speeds),
            big.len(),
            language.map_or("fallback", Language::name)
        );
    }
}

#[test]
#[ignore = "a measurement, run by hand in a release build: see the module documentation"]
fn perf_a_real_tree() {
    let Ok(path) = std::env::var("CORTEX_PERF_TREE") else {
        println!("CORTEX_PERF_TREE is not set: no real tree measured");
        return;
    };
    let path = std::env::current_dir().unwrap().join(path);
    let root = PathResolver::new(&path, PathOptions::new())
        .expect("the tree")
        .root()
        .clone();
    let scratch = cortex_testkit::scratch!("perf_a_real_tree");
    let index = system_index(root, scratch.join("index"), Limits::default());
    let start = Instant::now();
    let cold = index.pass(&CancelToken::new(), &|| false);
    println!(
        "real tree cold: {:.0} ms, found {}, read {}, partial {:?}, cache {} bytes",
        ms(start.elapsed()),
        cold.found,
        cold.read,
        cold.partial,
        cold.cache_bytes
    );
    let mut warm = Vec::new();
    let mut map = String::new();
    for _ in 0..15 {
        let start = Instant::now();
        let _ = index.pass(&CancelToken::new(), &|| false);
        map = index.map(&MapRequest::new()).expect("a map");
        warm.push(ms(start.elapsed()));
    }
    println!("real tree warm pass and map: median {:.1} ms", median(warm));
    println!("--- the map ---\n{map}--- end ---");
}
