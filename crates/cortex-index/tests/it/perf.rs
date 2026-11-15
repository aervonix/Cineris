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

use cortex_testkit::Phases;

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
    Index::new(
        root,
        dir,
        limits,
        Arc::new(OpenGate),
        Arc::new(SystemClock::new()),
    )
}

#[test]
#[ignore = "a measurement, run by hand in a release build: see the module documentation"]
fn perf_a_cold_index_a_warm_one_and_a_warm_map() {
    let phases = Phases::new("perf_a_cold_index_a_warm_one_and_a_warm_map");
    let files = number_from_env("CORTEX_PERF_FILES", 20_000);
    let tree = phases.measure("generate the tree", || generated("perf_cold_warm", files));
    let dir = tree.cache_dir();
    let index = system_index(tree.root.clone(), dir, Limits::default());
    let rss_before = crate::resident_kib("VmRSS:");

    let start = Instant::now();
    let cold = phases.measure("cold pass", || index.pass(&CancelToken::new(), &|| false));
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
        phases.measure("change a file", || {
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
        });
        let start = Instant::now();
        let report = phases.measure("warm pass", || index.pass(&CancelToken::new(), &|| false));
        let passed = start.elapsed();
        let map = phases
            .measure("warm map", || index.map(&MapRequest::new()))
            .expect("a map");
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
    phases.measure("breakdown of a warm pass", || breakdown(&tree));
    phases.mark("tear-down");
    drop(index);
    drop(tree);
    phases.end();
}

/// Where a warm pass's time goes: the listing as a pass makes it, the same
/// listing without looking for ignore files in each directory, and the
/// cache read and written as a pass does (owner-only, by an atomic
/// replace). Each a median of five.
fn breakdown(tree: &Tree) {
    let root = tree.root.to_fs_path();
    let list = |rules: bool| {
        let start = Instant::now();
        let mut builder = ignore::WalkBuilder::new(&root);
        builder.follow_links(false).standard_filters(false);
        if rules {
            builder
                .hidden(true)
                .ignore(true)
                .git_ignore(true)
                .git_exclude(true)
                .parents(true)
                .require_git(false)
                .git_global(false);
        }
        builder.sort_by_file_name(std::ffi::OsStr::cmp);
        let mut found = 0_usize;
        for entry in builder.build().flatten() {
            if entry.metadata().is_ok() {
                found += 1;
            }
        }
        assert!(found > 0);
        ms(start.elapsed())
    };
    let listing = median((0..5).map(|_| list(true)).collect());
    let bare = median((0..5).map(|_| list(false)).collect());
    // The same rules on the threads a pass lists on: half the cores, four
    // at most.
    let threads =
        std::thread::available_parallelism().map_or(1, |cores| (cores.get() / 2).clamp(1, 4));
    let on_threads = median(
        (0..5)
            .map(|_| {
                let start = Instant::now();
                let mut builder = ignore::WalkBuilder::new(&root);
                builder
                    .follow_links(false)
                    .standard_filters(false)
                    .hidden(true)
                    .ignore(true)
                    .git_ignore(true)
                    .git_exclude(true)
                    .parents(true)
                    .require_git(false)
                    .git_global(false)
                    .threads(threads);
                let found = std::sync::atomic::AtomicUsize::new(0);
                builder.build_parallel().run(|| {
                    Box::new(|entry| {
                        if entry.is_ok_and(|entry| entry.metadata().is_ok()) {
                            found.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        ignore::WalkState::Continue
                    })
                });
                assert!(found.into_inner() > 0);
                ms(start.elapsed())
            })
            .collect(),
    );
    let cache = tree.cache_dir().join(cortex_index::CACHE_FILE);
    let reads = (0..5)
        .map(|_| {
            let start = Instant::now();
            let bytes = std::fs::read(&cache).expect("the cache");
            assert!(!bytes.is_empty());
            ms(start.elapsed())
        })
        .collect();
    let bytes = std::fs::read(&cache).expect("the cache");
    let copy = tree.cache_dir().join("copy");
    let options = cortex_base::AtomicWriteOptions {
        sync: false,
        owner_only: true,
        follow_links: false,
        ..cortex_base::AtomicWriteOptions::default()
    };
    let writes = (0..5)
        .map(|_| {
            let start = Instant::now();
            cortex_base::write_atomic(&copy, &bytes, &options, &SystemClock::new())
                .expect("written");
            ms(start.elapsed())
        })
        .collect();
    println!(
        "where it goes: the listing on one thread {listing:.1} ms ({bare:.1} ms without the ignore files), on {threads} threads {on_threads:.1} ms; the cache read {:.1} ms, written {:.1} ms ({} bytes)",
        median(reads),
        median(writes),
        bytes.len()
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
    println!(
        "its map: {:.1} ms, {} bytes, says partial: {}",
        ms(start.elapsed()),
        map.len(),
        map.contains("partial")
    );
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
