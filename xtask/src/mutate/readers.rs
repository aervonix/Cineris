//! Which crates read a file that is not Rust: what `--changed` asks of a
//! data file the diff touches (the model catalog, a fixture, a table a test
//! embeds), so that a change to one chooses the breaks of the crates that
//! can see it, and not every break.
//!
//! A crate reads a file when a string literal of one of its Rust files
//! names it, by at least one name that is not a climb (`..` or `/` alone
//! name nothing). The literal is read three ways:
//!
//! - **from the source file's folder,** as `include_str!` and
//!   `include_bytes!` read their path
//!   (`include_str!("../../../../catalog/models.toml")`);
//! - **from the crate's folder,** as a build script's
//!   `cargo::rerun-if-changed=PATH` and a path joined to
//!   `CARGO_MANIFEST_DIR` are (`Path::new(dir).join("../../catalog/models.toml")`,
//!   `concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures")`);
//! - **from the top of the repository,** as `xtask` names its files
//!   (`"docs/outstanding"`).
//!
//! Each way, it names the file, or a folder the file is in; from the top, a
//! folder only by two names or more (`"docs"` alone is too common a word to
//! read every file under `docs/`). And a literal of two names or more that
//! ends the file's path (`"catalog/models.toml"`) names it from anywhere.
//! A file's name alone (`"request.json"`) does not: many crates name files
//! of that name, and it would make a reader of each.
//!
//! **A reader in its crate's tests only** (a file under the crate's
//! `tests/`, `benches/` or `examples/`, or inside a `#[cfg(test)] mod … {}`)
//! is seen by that crate's tests alone; any other is seen by every crate
//! built on it, or whose tests use it.
//!
//! **What this cannot see,** and what `--changed` then does: a path made at
//! run time from pieces that do not name it by two names (a folder's name
//! joined to the file's: `root.join("corpus").join("PIN")`), or read from
//! the environment or a setting; a file a program the tests start reads by
//! a path of its own. A file outside the folders `--changed` reads one by
//! one that no crate names chooses every break, as before; one inside them
//! (`docs/`, `.github/`, a crate's own folder) is left to the rules that
//! read those (a crate's own files choose its breaks).

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use super::lex;
use crate::workspace::Package;

/// A path in the repository with `.` and `..` resolved and `/` between
/// its parts. `None` when it climbs out of the repository, or is empty.
pub(crate) fn normalise(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The folder a repository path is in: `""` for one at the top.
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(folder, _)| folder)
}

/// `relative` joined to `folder`, resolved.
fn joined(folder: &str, relative: &str) -> Option<String> {
    if folder.is_empty() {
        normalise(relative)
    } else {
        normalise(&format!("{folder}/{relative}"))
    }
}

/// Whether `named` is `path`, or a folder `path` is in.
fn names_or_holds(named: &str, path: &str) -> bool {
    named == path
        || path
            .strip_prefix(named)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether the string literal `text`, of the Rust file `source` of the
/// crate in `crate_folder`, names `path` (a repository path): see the
/// module's rules.
pub(crate) fn literal_names(text: &str, source: &str, crate_folder: &str, path: &str) -> bool {
    // A build script's `cargo::rerun-if-changed=PATH` (or `cargo:`) is a
    // path from the crate's folder.
    let (text, crate_only) = match text.split_once("rerun-if-changed=") {
        Some((_, rest)) => (rest, true),
        None => (text, false),
    };
    let text = text.trim();
    // A format string's hole, or more than one line, is no path.
    if text.is_empty() || text.contains(['{', '}', '\n', '\0']) {
        return false;
    }
    let parts: Vec<&str> = text
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    let names = parts.iter().filter(|part| **part != "..").count();
    // `..`, `../..`, `/`: a climb names no file, and would hold every file
    // above the source.
    if names == 0 {
        return false;
    }
    let rooted = text.starts_with(['/', '\\']);
    // `text` from `folder`, when it names something there: not the folder
    // itself or one above it (`a/../..` climbs past the name it gives).
    let from = |folder: &str| joined(folder, text).filter(|named| !names_or_holds(named, folder));
    if let Some(named) = from(crate_folder)
        && names_or_holds(&named, path)
    {
        return true;
    }
    if crate_only || rooted {
        return false;
    }
    if let Some(named) = from(folder_of(source))
        && names_or_holds(&named, path)
    {
        return true;
    }
    // From the top: the file itself, or a folder of two names or more.
    if let Some(named) = normalise(text)
        && (named == path || (named.contains('/') && names_or_holds(&named, path)))
    {
        return true;
    }
    // The end of the file's path, of two names or more, its climbs taken
    // off: `../catalog/models.toml`.
    let tail: Vec<&str> = parts
        .iter()
        .copied()
        .skip_while(|part| *part == "..")
        .collect();
    if tail.len() < 2 || tail.contains(&"..") {
        return false;
    }
    let tail = tail.join("/");
    path == tail || path.ends_with(&format!("/{tail}"))
}

/// Each string literal of `source`: where it starts, and its text between
/// its quotes, as it is written (escapes are left as they are: a path has
/// none).
pub(crate) fn literals(source: &str) -> Vec<(usize, &str)> {
    lex::code_tokens(source)
        .into_iter()
        .filter_map(|token| {
            let literal = token.text(source);
            lex::string_content(literal).map(|content| (token.start, &literal[content]))
        })
        .collect()
}

/// Where each `#[cfg(test)] mod NAME { … }` of `source` is: what only the
/// crate's own tests build. Any other `cfg(test)` (on a function, a `use`,
/// a module in a file of its own) is not counted: the reader is then taken
/// to be seen by the crates built on it too, which chooses more and never
/// less.
pub(crate) fn test_modules(source: &str) -> Vec<Range<usize>> {
    let tokens = lex::code_tokens(source);
    let text = |index: usize| tokens.get(index).map_or("", |token| token.text(source));
    let mut found = Vec::new();
    let mut at = 0;
    while at < tokens.len() {
        let attribute = ["#", "[", "cfg", "(", "test", ")", "]"];
        if !(0..attribute.len()).all(|offset| text(at + offset) == attribute[offset]) {
            at += 1;
            continue;
        }
        let mut next = at + attribute.len();
        if text(next) == "pub" {
            next += 1;
            if text(next) == "(" {
                while next < tokens.len() && text(next) != ")" {
                    next += 1;
                }
                next += 1;
            }
        }
        if text(next) != "mod" || text(next + 2) != "{" {
            at += 1;
            continue;
        }
        let open = next + 2;
        let mut depth = 0usize;
        let mut close = None;
        for index in open..tokens.len() {
            match text(index) {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(index);
                        break;
                    }
                }
                _ => {}
            }
        }
        // An unclosed module is not counted.
        let Some(close) = close else {
            break;
        };
        found.push(tokens[at].start..tokens[close].end);
        at = close + 1;
    }
    found
}

/// Whether a Rust file is built only for its crate's own tests: one under
/// the crate's `tests/`, `benches/` or `examples/`.
fn tests_only_file(path: &str, crate_folder: &str) -> bool {
    path.strip_prefix(crate_folder)
        .and_then(|rest| rest.strip_prefix('/'))
        .is_some_and(|rest| {
            ["tests/", "benches/", "examples/"]
                .iter()
                .any(|folder| rest.starts_with(folder))
        })
}

/// The folder of each package, from its manifest: `crates/cineris-llm`,
/// `xtask`. A package at the top of the repository has none, and is left
/// out: its files are every file.
pub(crate) fn folders(packages: &[Package]) -> Vec<(String, String)> {
    packages
        .iter()
        .filter_map(|package| {
            let folder = folder_of(&package.manifest.replace('\\', "/")).to_owned();
            (!folder.is_empty()).then(|| (package.name.clone(), folder))
        })
        .collect()
}

/// The crates that read a file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Readers {
    /// Those whose code built for others reads it: the crates built on them
    /// see it too.
    pub(crate) built: BTreeSet<String>,
    /// Those whose own tests alone read it.
    pub(crate) tests: BTreeSet<String>,
}

impl Readers {
    pub(crate) fn is_empty(&self) -> bool {
        self.built.is_empty() && self.tests.is_empty()
    }
}

/// For each of `wanted` (repository paths), the crates whose Rust files
/// name it. `files` is each Rust file of the crates, as `(crate, crate's
/// folder, path, text)`: as it is, and as it was at the reference when the
/// diff touches it, so that a reader the diff removed still counts.
pub(crate) fn readers<'a>(
    wanted: &[String],
    files: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str)>,
) -> BTreeMap<String, Readers> {
    let mut found: BTreeMap<String, Readers> = wanted
        .iter()
        .map(|path| (path.clone(), Readers::default()))
        .collect();
    for (name, folder, source, text) in files {
        let whole_file = tests_only_file(source, folder);
        let modules = if whole_file {
            Vec::new()
        } else {
            test_modules(text)
        };
        for (at, literal) in literals(text) {
            let tests_only = whole_file || modules.iter().any(|module| module.contains(&at));
            for path in wanted {
                if literal_names(literal, source, folder, path)
                    && let Some(readers) = found.get_mut(path)
                {
                    if tests_only {
                        readers.tests.insert(name.to_owned());
                    } else {
                        readers.built.insert(name.to_owned());
                    }
                }
            }
        }
    }
    for readers in found.values_mut() {
        let built = readers.built.clone();
        readers.tests.retain(|name| !built.contains(name));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{literal_names, literals, normalise, readers, test_modules};

    #[test]
    fn a_path_is_resolved_and_one_that_climbs_out_is_none() {
        assert_eq!(
            normalise("crates/a/../../tables/rates.toml").as_deref(),
            Some("tables/rates.toml")
        );
        assert_eq!(normalise("./a\\b/./c").as_deref(), Some("a/b/c"));
        assert_eq!(normalise("../outside"), None);
        assert_eq!(normalise(""), None);
    }

    #[test]
    fn include_str_reads_from_its_own_files_folder() {
        let source = "crates/demo-stats/src/synthetic/tests.rs";
        assert!(literal_names(
            "../../../../tables/rates.toml",
            source,
            "crates/demo-stats",
            "tables/rates.toml"
        ));
        // One climb fewer is another file.
        assert!(!literal_names(
            "../../../tables/rates.toml",
            "crates/x/src/deep/more/tests.rs",
            "crates/x",
            "tables/other.toml"
        ));
    }

    #[test]
    fn a_build_scripts_paths_are_from_the_crates_folder() {
        let crate_folder = "crates/demo-llm";
        let build = "crates/demo-llm/build.rs";
        // Joined to CARGO_MANIFEST_DIR.
        assert!(literal_names(
            "../../tables/rates.toml",
            build,
            crate_folder,
            "tables/rates.toml"
        ));
        assert!(literal_names(
            "cargo::rerun-if-changed=../../tables/rates.toml",
            build,
            crate_folder,
            "tables/rates.toml"
        ));
        assert!(literal_names(
            "cargo:rerun-if-changed=fixtures",
            build,
            crate_folder,
            "crates/demo-llm/fixtures/a/b.json"
        ));
        // A hole is filled at run time: no path.
        assert!(!literal_names(
            "cargo::rerun-if-changed={}",
            build,
            crate_folder,
            "tables/rates.toml"
        ));
        // `concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures")`.
        assert!(literal_names(
            "/fixtures",
            "crates/demo-llm/tests/it/main.rs",
            crate_folder,
            "crates/demo-llm/fixtures/openai/x.json"
        ));
        assert!(!literal_names(
            "/fixtures",
            "crates/other/tests/it/main.rs",
            "crates/other",
            "crates/demo-llm/fixtures/openai/x.json"
        ));
    }

    #[test]
    fn a_path_from_the_top_names_a_file_or_a_folder_of_two_parts() {
        let (source, folder) = ("xtask/src/rules.rs", "xtask");
        assert!(literal_names(
            "docs/outstanding",
            source,
            folder,
            "docs/outstanding/an-item.md"
        ));
        assert!(literal_names("NOTICE-X", source, folder, "NOTICE-X"));
        // One word is not a folder of everything under it.
        assert!(!literal_names("docs", source, folder, "docs/vision.md"));
        // A folder is not a file whose name starts with it.
        assert!(!literal_names(
            "docs/out",
            source,
            folder,
            "docs/outstanding/x.md"
        ));
    }

    #[test]
    fn the_end_of_a_path_of_two_names_names_the_file_wherever_it_is() {
        let (source, folder) = ("xtask/src/corpus.rs", "xtask");
        assert!(literal_names(
            "tables/rates.toml",
            source,
            folder,
            "data/tables/rates.toml"
        ));
        assert!(literal_names(
            "../tables/rates.toml",
            "crates/x/src/deep/a.rs",
            "crates/x",
            "tables/rates.toml"
        ));
        // A file's name alone is too common to make a reader.
        assert!(!literal_names("STAMP", source, folder, "pins/STAMP"));
        assert!(!literal_names(
            "rates.toml",
            source,
            folder,
            "tables/rates.toml"
        ));
        // Part of a name is not its end.
        assert!(!literal_names(
            "s/rates.toml",
            source,
            folder,
            "tables/rates.toml"
        ));
        assert!(!literal_names("", source, folder, "tables/rates.toml"));
    }

    #[test]
    fn a_literal_that_only_climbs_names_nothing() {
        // `../..` from a source two folders down would hold every file of
        // the crates: it names none.
        for climb in [
            "..",
            "../..",
            "../../",
            "/",
            ".",
            "./..",
            "a/../..",
            "/../..",
            "a/b/../..",
        ] {
            assert!(
                !literal_names(
                    climb,
                    "crates/x/src/deep/a.rs",
                    "crates/x",
                    "crates/y/fixtures/a.json"
                ),
                "{climb}"
            );
        }
    }

    #[test]
    fn the_literals_of_a_file_are_its_strings_and_not_its_comments() {
        let source = "// \"not/this.toml\"\nconst A: &str = \"one\";\nfn f() { g(r#\"two\"#) }\n";
        let found: Vec<&str> = literals(source).into_iter().map(|(_, text)| text).collect();
        assert_eq!(found, ["one", "two"]);
        assert_eq!(&source[literals(source)[0].0..][..5], "\"one\"");
    }

    #[test]
    fn a_test_module_is_found_from_its_attribute_to_its_closing_brace() {
        let source = "fn a() { \"x\"; }\n#[cfg(test)]\nmod tests {\n    fn b() { { \"y\"; } }\n}\n\
                      fn c() { \"z\"; }\n#[cfg(test)]\nfn d() {}\n#[cfg(test)]\nmod outside;\n";

        let modules = test_modules(source);

        assert_eq!(modules.len(), 1);
        let module = &source[modules[0].clone()];
        assert!(module.starts_with("#[cfg(test)]\nmod tests {"), "{module}");
        assert!(module.ends_with("}\n}"), "{module}");
        assert!(!module.contains('z'), "{module}");
        // `pub(crate) mod` too.
        let source = "#[cfg(test)]\npub(crate) mod kit { const P: &str = \"q\"; }";
        assert_eq!(test_modules(source).len(), 1);
    }

    #[test]
    fn each_wanted_file_gets_the_crates_that_name_it_and_whose_tests_alone_do() {
        let files = [
            (
                "demo-llm",
                "crates/demo-llm",
                "crates/demo-llm/build.rs",
                "fn main() { let p = dir.join(\"../../tables/rates.toml\"); }",
            ),
            (
                "demo-stats",
                "crates/demo-stats",
                "crates/demo-stats/src/synthetic.rs",
                "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    const T: &str = \
                 include_str!(\"../../../tables/rates.toml\");\n}\n",
            ),
            (
                "demo-tools",
                "crates/demo-tools",
                "crates/demo-tools/tests/it/main.rs",
                "const T: &str = include_str!(\"../../../../tables/rates.toml\");",
            ),
            (
                "demo-base",
                "crates/demo-base",
                "crates/demo-base/src/lib.rs",
                "// tables/rates.toml\nconst X: &str = \"tables\"; const Y: &str = \"rates.toml\";",
            ),
            // Built and tested both: built.
            (
                "demo-llm",
                "crates/demo-llm",
                "crates/demo-llm/tests/it/main.rs",
                "const T: &str = include_str!(\"../../../../tables/rates.toml\");",
            ),
        ];
        let wanted = vec!["tables/rates.toml".to_owned(), "pins/STAMP".to_owned()];

        let found = readers(&wanted, files);

        let names = |set: &std::collections::BTreeSet<String>| {
            set.iter().map(String::as_str).collect::<Vec<_>>().join(",")
        };
        assert_eq!(names(&found["tables/rates.toml"].built), "demo-llm");
        assert_eq!(
            names(&found["tables/rates.toml"].tests),
            "demo-stats,demo-tools"
        );
        assert!(found["pins/STAMP"].is_empty());
    }
}
