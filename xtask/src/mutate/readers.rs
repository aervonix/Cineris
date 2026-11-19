//! Which crates read a file that is not Rust: what `--changed` asks of a
//! data file the diff touches (the model catalog, a fixture, a table a test
//! embeds), so that a change to one chooses the breaks of the crates that
//! can see it, and not every break.
//!
//! A crate reads a file when a string literal of one of its Rust files
//! names it. The literal is read three ways, and any of them that names the
//! file, or a folder the file is in, counts:
//!
//! - **from the source file's folder,** as `include_str!` and
//!   `include_bytes!` read their path
//!   (`include_str!("../../../../catalog/models.toml")`);
//! - **from the crate's folder,** as a build script's
//!   `cargo::rerun-if-changed=PATH` and a path joined to
//!   `CARGO_MANIFEST_DIR` are (`Path::new(dir).join("../../catalog/models.toml")`,
//!   `concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures")`);
//! - **from the top of the repository,** as `xtask` names its files
//!   (`"docs/outstanding"`). A literal of one word names a folder this way
//!   only with a `/` in it: `"docs"` alone is too common a word to be a
//!   reader of every file under `docs/`.
//!
//! And a literal whose path ends the file's (`"models.toml"`,
//! `"catalog/models.toml"`) names it, wherever it is: a path joined from
//! pieces (`root.join("corpus").join("PIN")`) still has its last one.
//!
//! **What this cannot see,** and what `--changed` then does: a path made at
//! run time from pieces none of which is the file's name, or read from the
//! environment or a setting; a file a program the tests start reads by a
//! path of its own; a folder walked whose name is one word from the top of
//! the repository. A file outside the folders `--changed` reads one by one
//! that no crate names chooses every break, as before; one inside them
//! (`docs/`, `.github/`, a crate's own folder) is left to the rules that
//! read those (a crate's own files choose its breaks).

use std::collections::{BTreeMap, BTreeSet};

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
    let rooted = text.starts_with(['/', '\\']);
    if let Some(named) = joined(crate_folder, text)
        && names_or_holds(&named, path)
    {
        return true;
    }
    if crate_only || rooted {
        return false;
    }
    if let Some(named) = joined(folder_of(source), text)
        && names_or_holds(&named, path)
    {
        return true;
    }
    // From the top: the file itself, or a folder of two parts or more.
    if let Some(named) = normalise(text)
        && (named == path || (named.contains('/') && names_or_holds(&named, path)))
    {
        return true;
    }
    // The end of the file's path, its climbs taken off: `models.toml`,
    // `../catalog/models.toml`.
    let tail: Vec<&str> = text
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .skip_while(|part| matches!(*part, "." | ".."))
        .collect();
    if tail.is_empty() || tail.iter().any(|part| matches!(*part, "." | "..")) {
        return false;
    }
    let tail = tail.join("/");
    path == tail || path.ends_with(&format!("/{tail}"))
}

/// The text of each string literal of `source`, between its quotes, as it
/// is written (escapes are left as they are: a path has none).
pub(crate) fn literals(source: &str) -> Vec<&str> {
    lex::code_tokens(source)
        .into_iter()
        .filter_map(|token| {
            let literal = token.text(source);
            lex::string_content(literal).map(|content| &literal[content])
        })
        .collect()
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

/// For each of `wanted` (repository paths), the crates whose Rust files
/// name it. `files` is each Rust file of the crates, as `(crate, crate's
/// folder, path, text)`: as it is, and as it was at the reference when the
/// diff touches it, so that a reader the diff removed still counts.
pub(crate) fn readers<'a>(
    wanted: &[String],
    files: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str)>,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut found: BTreeMap<String, BTreeSet<String>> = wanted
        .iter()
        .map(|path| (path.clone(), BTreeSet::new()))
        .collect();
    for (name, folder, source, text) in files {
        for literal in literals(text) {
            for path in wanted {
                if literal_names(literal, source, folder, path) {
                    found
                        .get_mut(path)
                        .map(|crates| crates.insert(name.to_owned()));
                }
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{literal_names, literals, normalise, readers};

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
    fn the_end_of_a_path_names_the_file_wherever_it_is() {
        let (source, folder) = ("xtask/src/corpus.rs", "xtask");
        // `root.join("corpus").join("STAMP")`: the last piece.
        assert!(literal_names("STAMP", source, folder, "pins/STAMP"));
        assert!(literal_names(
            "rates.toml",
            source,
            folder,
            "tables/rates.toml"
        ));
        assert!(literal_names(
            "../tables/rates.toml",
            "crates/x/src/a.rs",
            "crates/x",
            "tables/rates.toml"
        ));
        // Part of a name is not its end.
        assert!(!literal_names(
            "els.toml",
            source,
            folder,
            "tables/rates.toml"
        ));
        assert!(!literal_names("", source, folder, "tables/rates.toml"));
        assert!(!literal_names("..", source, folder, "tables/rates.toml"));
    }

    #[test]
    fn the_literals_of_a_file_are_its_strings_and_not_its_comments() {
        let source = "// \"not/this.toml\"\nconst A: &str = \"one\";\nfn f() { g(r#\"two\"#) }\n";
        assert_eq!(literals(source), ["one", "two"]);
    }

    #[test]
    fn each_wanted_file_gets_the_crates_that_name_it() {
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
                "crates/demo-stats/src/synthetic/tests.rs",
                "const T: &str = include_str!(\"../../../../tables/rates.toml\");",
            ),
            (
                "demo-base",
                "crates/demo-base",
                "crates/demo-base/src/lib.rs",
                "// tables/rates.toml\nconst X: &str = \"tables\";",
            ),
        ];
        let wanted = vec!["tables/rates.toml".to_owned(), "pins/STAMP".to_owned()];

        let found = readers(&wanted, files);

        assert_eq!(
            found["tables/rates.toml"]
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["demo-llm", "demo-stats"]
        );
        assert!(found["pins/STAMP"].is_empty());
    }
}
