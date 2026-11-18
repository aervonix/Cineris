//! The map: what it shows, in what order, inside its budget, and that it
//! is the same bytes for the same inputs.

use std::sync::Arc;
use std::time::Duration;

use cineris_index::{Limits, MapRequest, OpenGate};

use super::assert_snapshot;
use super::index::Tree;

fn rust_file(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("pub fn {name}() {{}}\n"))
        .collect()
}

/// A small project of a few directories.
fn project(test: &str) -> Tree {
    let tree = Tree::new(cineris_testkit::scratch!(test));
    tree.write("Cargo.toml", "[package]\nname = \"demo\"\n");
    tree.write("README.md", "# Demo\n\n## Use\n");
    tree.write(
        "src/lib.rs",
        "pub struct Engine;\n\nimpl Engine {\n    pub fn start(&self) {}\n    pub fn stop(&self) {}\n}\n",
    );
    tree.write("src/parse/mod.rs", &rust_file(&["parse", "tokens"]));
    tree.write("src/parse/expr.rs", &rust_file(&["expr"]));
    tree.write("tests/it.rs", &rust_file(&["runs"]));
    tree.write("guides/intro.md", "# Guide\n\n## Install\n");
    tree
}

fn request(tokens: usize) -> MapRequest {
    MapRequest {
        tokens,
        ..MapRequest::new()
    }
}

#[test]
fn a_small_projects_map() {
    let tree = project("a_small_projects_map");
    let index = tree.index();
    tree.pass(&index);
    let map = index.map(&MapRequest::new()).expect("a map");
    assert_snapshot("maps/small.txt", &map);
    // No absolute path, no time: nothing of the machine.
    assert!(!map.contains(tree.root.as_str()), "{map}");
    assert!(!map.contains("2026"), "{map}");
}

#[test]
fn the_map_is_the_same_bytes_for_the_same_inputs() {
    let tree = project("the_map_is_the_same_bytes");
    let index = tree.index();
    tree.pass(&index);
    let touched = vec!["tests/it.rs".to_owned(), "src/parse/expr.rs".to_owned()];
    let first = index
        .map(&MapRequest {
            touched: touched.clone(),
            ..MapRequest::new()
        })
        .expect("a map");
    // Another pass over the same files, another order of the same touched
    // files, another reader's threads: the same map.
    let again = tree
        .index_with(Limits::default(), Arc::new(OpenGate))
        .with_threads(3);
    tree.pass(&again);
    let reversed: Vec<String> = touched.iter().rev().cloned().collect();
    let second = again
        .map(&MapRequest {
            touched: reversed,
            ..MapRequest::new()
        })
        .expect("a map");
    assert_eq!(first, second);
    // Rebuilt from scratch, after the cache is gone.
    std::fs::remove_file(index.cache_path()).expect("remove");
    tree.pass(&index);
    let third = index
        .map(&MapRequest {
            touched,
            ..MapRequest::new()
        })
        .expect("a map");
    assert_eq!(first, third);
}

#[test]
fn touched_files_come_first_then_recent_sessions_then_recent_changes() {
    let tree = project("touched_files_come_first");
    // One file changed a moment ago, ahead of the rest of the tree.
    tree.write("src/hot.rs", &rust_file(&["hot"]));
    tree.date("src/hot.rs", Duration::from_secs(60));
    let index = tree.index();
    tree.pass(&index);
    let map = index
        .map(&MapRequest {
            touched: vec!["tests/it.rs".to_owned()],
            recent: vec!["guides/intro.md".to_owned()],
            ..MapRequest::new()
        })
        .expect("a map");
    let files = map
        .split("Files, most relevant first:\n")
        .nth(1)
        .expect("files");
    let order: Vec<&str> = files
        .lines()
        .map(|line| line.split(':').next().unwrap_or(""))
        .collect();
    assert_eq!(
        &order[..3],
        ["tests/it.rs", "guides/intro.md", "src/hot.rs"],
        "{map}"
    );
    // Then code before documents, each directory's first file before any
    // directory's second, shallower first.
    assert_eq!(
        &order[3..],
        [
            "src/lib.rs",
            "src/parse/mod.rs",
            "src/parse/expr.rs",
            "README.md"
        ],
        "{map}"
    );
}

#[test]
fn a_map_at_exactly_its_budget_is_whole_and_one_byte_less_says_what_it_left_out() {
    let tree = project("a_map_at_exactly_its_budget");
    let index = tree.index();
    tree.pass(&index);
    let whole = index.map(&request(100_000)).expect("a map");
    assert!(!whole.contains("more file"), "{whole}");
    // A budget of exactly its size, in tokens of three bytes: pad the map
    // to a multiple of three by asking for that many tokens.
    let tokens = whole.len().div_ceil(3);
    let exact = index.map(&request(tokens)).expect("a map");
    assert_eq!(exact, whole);
    // Too small for everything: it says how many files it left out, and
    // stays inside the budget.
    let cut = index.map(&request(tokens - 30)).expect("a map");
    assert!(
        cut.len() <= (tokens - 30) * 3,
        "{} > {}",
        cut.len(),
        (tokens - 30) * 3
    );
    assert!(
        cut.contains("more files not shown") || cut.contains("more file not shown"),
        "{cut}"
    );
    // However small the budget, never past it.
    for tokens in [0, 10, 80, 120, 200, 400] {
        if let Some(map) = index.map(&request(tokens)) {
            assert!(map.len() <= tokens * 3, "{tokens}: {} bytes", map.len());
        }
    }
}

#[test]
fn a_fresh_clone_has_no_recent_changes_to_rank_by() {
    let tree = project("a_fresh_clone_has_no_recent_changes");
    let index = tree.index();
    tree.pass(&index);
    let map = index.map(&MapRequest::new()).expect("a map");
    let files = map
        .split("Files, most relevant first:\n")
        .nth(1)
        .expect("files");
    // Every file has one time: the order is the code's first, shallower
    // first, then the documents'.
    let first = files.lines().next().expect("a line");
    assert!(first.starts_with("src/lib.rs"), "{map}");
}

/// The files of a map, in its order.
fn shown_files(map: &str) -> Vec<&str> {
    map.split("Files, most relevant first:\n")
        .nth(1)
        .unwrap_or("")
        .lines()
        .filter(|line| !line.starts_with('['))
        .map(|line| line.split(':').next().unwrap_or(""))
        .collect()
}

/// The independent review of step 18 found this repository's map naming
/// `Cargo.lock`, the licences and the top-level documents, and no source
/// file. A lockfile, a licence, a changelog and a published crate's
/// original manifest are never shown; a code file of such a name is code;
/// and code comes before documents.
#[test]
fn noise_is_never_shown_and_code_comes_before_documents() {
    let tree = Tree::new(cineris_testkit::scratch!(
        "noise_is_never_shown_and_code_comes_before_documents"
    ));
    let paragraphs = "Terms\n  of use\nMore\n  terms\n";
    tree.write(
        "Cargo.lock",
        "[[package]]\nname = \"a\"\ndependencies = [\n  \"b\",\n]\n",
    );
    tree.write("package-lock.json", "{\n  \"name\": \"a\"\n}\n");
    tree.write("LICENSE-APACHE", paragraphs);
    tree.write("COPYING", paragraphs);
    tree.write("CHANGELOG.md", "# Changelog\n\n## 1.0\n");
    tree.write("Cargo.toml.orig", "[package]\n  name = \"a\"\n");
    tree.write("README.md", "# Demo\n\n## Use\n");
    tree.write("deep/down/src/lib.rs", &rust_file(&["deep"]));
    tree.write("src/history.rs", &rust_file(&["record"]));
    let index = tree.index();
    tree.pass(&index);
    let map = index.map(&MapRequest::new()).expect("a map");
    assert_eq!(
        shown_files(&map),
        ["src/history.rs", "deep/down/src/lib.rs", "README.md"],
        "{map}"
    );
    // Left out of the files, not of the counts.
    assert!(map.contains("9 files indexed."), "{map}");
}

/// The last tier is spread over the tree: each directory's first file
/// before any directory's second, so a directory of many files does not
/// take the map from the others. A directory's first file is the one with
/// the most top-level definitions.
#[test]
fn the_map_shows_the_trees_breadth_before_one_directorys_depth() {
    let tree = Tree::new(cineris_testkit::scratch!("the_map_shows_the_trees_breadth"));
    for n in 0..20 {
        tree.write(&format!("big/f{n:02}.rs"), &rust_file(&["a"]));
    }
    tree.write("big/z.rs", &rust_file(&["a", "b", "c"]));
    tree.write("small/one.rs", &rust_file(&["one"]));
    tree.write("small/two.rs", &rust_file(&["two"]));
    tree.write("tiny/only.rs", &rust_file(&["only"]));
    let index = tree.index();
    tree.pass(&index);
    let map = index.map(&MapRequest::new()).expect("a map");
    assert_eq!(
        shown_files(&map)[..6],
        [
            "big/z.rs",
            "small/one.rs",
            "tiny/only.rs",
            "big/f00.rs",
            "small/two.rs",
            "big/f01.rs"
        ],
        "{map}"
    );
}
