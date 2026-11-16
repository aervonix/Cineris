//! Whether the tests a break's `caught-by` names exist.
//!
//! A `caught-by` that names a test that was renamed or removed is found
//! when the break is run, and a run is minutes of building. This finds it
//! before, from a list of the tests of the crates the break runs.
//!
//! **A list of tests has to be built, and `cargo xtask check` builds
//! nothing** (`lint-linux` runs clippy and no test binary). So the list is
//! kept, and the check reads it:
//!
//! - `cargo xtask mutate --check --tests` builds the tests of the crates
//!   the breaks run, as `cargo nextest list` does, lists them, keeps each
//!   list in `target/mutate/test-lists.json`, and checks every `caught-by`
//!   against them. At the desk, with the tests built, it is the time of
//!   the listing: a second or two for a crate.
//! - `cargo xtask mutate --check`, and the `breaks` step of `cargo xtask
//!   check`, check against the lists that are kept **and fresh**, and
//!   build nothing. A list is fresh when the files of its crates are what
//!   they were when it was made ([`source_key`]). With no file, or a stale
//!   list, the check says that names were not checked, and says how to
//!   check them. It costs nothing more than it did.
//!
//! Which tests exist depends on what is built together, so a list is made
//! for each set of crates a break runs, as `mutate` judges a break against
//! the tests of exactly the build it runs in.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use super::drive::Fnv;
use super::list::{self, Break, Expect};
use crate::Result;
use crate::workspace;

/// Where the lists of tests are kept: beside what `mutate` keeps in the
/// workspace's own `target` folder, where nextest keeps its reports.
pub(crate) fn lists_path(root: &Path) -> PathBuf {
    root.join("target").join("mutate").join("test-lists.json")
}

/// The tests of one build, as `<binary> <name>`, which is how a test is
/// named by [`crate::junit::TestCase::id`] and in a `caught-by`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Listed {
    /// The tests a run runs.
    pub(crate) tests: Vec<String>,
    /// The tests marked `#[ignore]`: listed, and never run by `mutate`, so
    /// no break can be caught by one.
    pub(crate) ignored: Vec<String>,
}

/// What is kept for one set of crates.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stored {
    /// What the crates' files were when the list was made.
    key: String,
    listed: Listed,
}

/// The lists that are kept: for each set of crates, its tests and the
/// state of the code they were listed for.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Lists {
    sets: BTreeMap<String, Stored>,
}

impl Lists {
    /// What is in the file. A file that is not there, or that cannot be
    /// read, is an empty set of lists: it only means nothing is known.
    pub(crate) fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        Self::parse(&text).unwrap_or_default()
    }

    fn parse(text: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(text).ok()?;
        let strings = |value: Option<&Value>| -> Option<Vec<String>> {
            value?
                .as_array()?
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect()
        };
        let mut sets = BTreeMap::new();
        for (name, stored) in value.get("sets")?.as_object()? {
            sets.insert(
                name.clone(),
                Stored {
                    key: stored.get("key")?.as_str()?.to_owned(),
                    listed: Listed {
                        tests: strings(stored.get("tests"))?,
                        ignored: strings(stored.get("ignored"))?,
                    },
                },
            );
        }
        Some(Self { sets })
    }

    /// Writes the lists to `path`, all or none: the file is written beside
    /// it and moved over it.
    pub(crate) fn save(&self, path: &Path) -> Result<()> {
        let sets: serde_json::Map<String, Value> = self
            .sets
            .iter()
            .map(|(name, stored)| {
                (
                    name.clone(),
                    json!({
                        "key": stored.key,
                        "tests": stored.listed.tests,
                        "ignored": stored.listed.ignored,
                    }),
                )
            })
            .collect();
        let text = Value::Object(
            [("sets".to_owned(), Value::Object(sets))]
                .into_iter()
                .collect(),
        )
        .to_string();
        let failed = |error: std::io::Error| format!("could not write {}: {error}", path.display());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(failed)?;
        }
        let beside = path.with_extension("json.new");
        fs::write(&beside, text).map_err(failed)?;
        fs::rename(&beside, path).map_err(failed)
    }

    /// The tests of `set`, when they were listed for the code as it is.
    fn fresh(&self, set: &str, key: &str) -> Option<&Listed> {
        self.sets
            .get(set)
            .filter(|stored| stored.key == key)
            .map(|stored| &stored.listed)
    }
}

/// A set of crates, as it is named in the lists: sorted, once each, the
/// same set whatever order a break gives them in.
pub(crate) fn set_name(packages: &[String]) -> String {
    let set: BTreeSet<&str> = packages.iter().map(String::as_str).collect();
    set.into_iter().collect::<Vec<_>>().join(",")
}

/// What the code a set of crates is listed from is, as one text: the name
/// and content of every file of those crates, the manifests of the
/// workspace, the lock file and nextest's configuration. A list made for
/// another is stale, and is not used.
///
/// A change in a crate the set depends on is not in it: a test's name is in
/// its own crate's files. What a dependency changes is which features are
/// on, and a manifest that does that is in it.
///
/// `memo` holds what a folder's files hash to, so that a crate that is in
/// many sets is read once.
fn source_key(
    root: &Path,
    files: &[String],
    set: &str,
    memo: &mut BTreeMap<String, u64>,
) -> Result<String> {
    let mut hash = Fnv::new();
    for folder in std::iter::once("").chain(set.split(',')) {
        if !memo.contains_key(folder) {
            let one = folder_hash(root, files, folder)?;
            memo.insert(folder.to_owned(), one);
        }
        hash.add(folder.as_bytes());
        hash.add(&memo[folder].to_le_bytes());
    }
    Ok(format!("{:016x}", hash.0))
}

/// The name and content of the files of one crate (`xtask/` for `xtask`), as
/// one number; for no crate (`""`), the workspace's own manifest, lock file
/// and nextest's configuration.
fn folder_hash(root: &Path, files: &[String], package: &str) -> Result<u64> {
    let folder = match package {
        "" => String::new(),
        "xtask" => "xtask/".to_owned(),
        package => format!("crates/{package}/"),
    };
    let mut hash = Fnv::new();
    for file in files {
        let wanted = if folder.is_empty() {
            matches!(
                file.as_str(),
                "Cargo.toml" | "Cargo.lock" | ".config/nextest.toml"
            )
        } else {
            file.starts_with(&folder)
        };
        if !wanted {
            continue;
        }
        let path = root.join(file);
        let content = fs::read(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        hash.add(file.as_bytes());
        hash.add(&content);
    }
    Ok(hash.0)
}

/// The tests in nextest's `list --message-format json`.
pub(crate) fn parse_listing(json: &str) -> Result<Listed> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| format!("nextest's list is no JSON: {error}"))?;
    let suites = value
        .get("rust-suites")
        .and_then(Value::as_object)
        .ok_or("nextest's list has no `rust-suites`")?;
    let mut listed = Listed::default();
    for (binary, suite) in suites {
        let Some(cases) = suite.get("testcases").and_then(Value::as_object) else {
            continue;
        };
        for (name, case) in cases {
            let id = format!("{binary} {name}");
            if case.get("ignored").and_then(Value::as_bool) == Some(true) {
                listed.ignored.push(id);
            } else {
                listed.tests.push(id);
            }
        }
    }
    if listed.tests.is_empty() && listed.ignored.is_empty() {
        return Err("nextest's list has no tests in it".to_owned());
    }
    listed.tests.sort();
    listed.ignored.sort();
    Ok(listed)
}

/// Lists the tests of `packages` in the tree at `root`: builds them as `cargo
/// nextest run -p …` does (a build already there is used) and asks the
/// binaries for their tests. cargo's progress goes to the caller's
/// terminal: a first build is minutes.
pub(crate) fn nextest_list(root: &Path, packages: &[String]) -> Result<Listed> {
    let mut command = Command::new(workspace::cargo());
    command
        .args(["nextest", "list", "--locked", "--message-format", "json"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .env("CARGO_TERM_COLOR", "never");
    for package in packages {
        command.args(["-p", package]);
    }
    let output = command
        .output()
        .map_err(|error| format!("could not run cargo nextest list: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo nextest list -p {}` failed ({}): the tests of those crates do not build, so \
             what a `caught-by` names cannot be looked for.",
            packages.join(" -p "),
            output.status
        ));
    }
    parse_listing(&String::from_utf8_lossy(&output.stdout))
}

/// What a check of the names came to.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Checked {
    /// One message for each `caught-by` name that no test has.
    pub(crate) problems: Vec<String>,
    /// How many names were looked for.
    pub(crate) names: usize,
    /// The sets of crates with no fresh list of tests, and so no check.
    pub(crate) unchecked: Vec<String>,
    /// How many names of breaks of the other system's code were looked for
    /// in the words of the source ([`by_source`]).
    pub(crate) by_source: usize,
}

/// What lists the tests of a set of crates: `cargo nextest list`, or a test's own.
pub(crate) type Lister<'a> = dyn FnMut(&[String]) -> Result<Listed> + 'a;

/// Checks the `caught-by` of `breaks` against the lists of tests.
///
/// With `list` the lists are made (and kept) for every set of crates that
/// has none that is fresh. Without, a set with no fresh list is left
/// unchecked and named in the result, and nothing is built.
pub(crate) fn check(
    root: &Path,
    breaks: &[Break],
    mut list: Option<&mut Lister<'_>>,
) -> Result<Checked> {
    let mut checked = Checked::default();
    // The breaks that name tests, by the set of crates they run.
    let mut by_set: BTreeMap<String, Vec<&Break>> = BTreeMap::new();
    let mut elsewhere: Vec<&Break> = Vec::new();
    for one in breaks {
        if one.expect == Expect::Survives || one.caught_by.is_empty() {
            continue;
        }
        // A break of the other system's code has no tests built here: its
        // names are looked for in the words of the source, and where it
        // runs, in the list of what is built.
        if !one.runs_here() {
            elsewhere.push(one);
            continue;
        }
        // A break that runs no crate has said so in the checks before.
        if let Ok(packages) = one.test_packages() {
            by_set.entry(set_name(&packages)).or_default().push(one);
        }
    }
    if !elsewhere.is_empty() {
        let files = workspace::repository_files(root)?;
        let (names, problems) = by_source(root, &files, &elsewhere);
        checked.by_source = names;
        checked.problems.extend(problems);
    }
    if by_set.is_empty() {
        return Ok(checked);
    }
    let path = lists_path(root);
    let mut lists = Lists::load(&path);
    // Without a file there is nothing to be fresh: the files of the tree
    // are not read.
    if list.is_none() && lists.sets.is_empty() {
        checked.unchecked = by_set.into_keys().collect();
        return Ok(checked);
    }
    let files = workspace::repository_files(root)?;
    let mut changed = false;
    let mut memo = BTreeMap::new();
    for (set, breaks) in by_set {
        let key = source_key(root, &files, &set, &mut memo)?;
        if lists.fresh(&set, &key).is_none() {
            let Some(list) = list.as_deref_mut() else {
                checked.unchecked.push(set);
                continue;
            };
            let packages: Vec<String> = set.split(',').map(str::to_owned).collect();
            let listed = list(&packages)?;
            lists.sets.insert(
                set.clone(),
                Stored {
                    key: key.clone(),
                    listed,
                },
            );
            changed = true;
        }
        if let Some(listed) = lists.fresh(&set, &key) {
            for one in breaks {
                checked.names += one.caught_by.len();
                checked.problems.extend(missing(one, &set, listed));
            }
        }
    }
    if changed {
        lists.save(&path)?;
    }
    Ok(checked)
}

/// The `caught-by` names of breaks of the other system's code, looked for
/// in the words of the source: a name passes when its last segment (or, for
/// an entry that ends in `*`, its beginning) is a word of some Rust file of
/// the crates the break runs. How many names were looked for, and a message
/// for each that no file has.
///
/// A list of tests is of what the build here builds, and a test of Windows
/// only (`#[cfg(windows)]`) is not in it, so on Linux nothing checked
/// these names until the break was run on Windows: a test that was renamed
/// or removed was found by a run there. This finds that, on any system, from
/// the files alone. It cannot tell a test from a helper of its name, a
/// module path that is wrong, or a test that is `#[ignore]`d, and a name
/// that a macro makes from pieces is not a word of any file (an entry that
/// ends in `::*` is not looked for at all). The list of tests that
/// `--check --tests` keeps for a set of crates is what such a check needs
/// to be exact, and it is built by the system that builds the code.
fn by_source(root: &Path, files: &[String], breaks: &[&Break]) -> (usize, Vec<String>) {
    let mut names = 0;
    let mut problems = Vec::new();
    // The words of each crate's Rust files, read once.
    let mut words: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for one in breaks {
        let Ok(packages) = one.test_packages() else {
            continue;
        };
        for entry in &one.caught_by {
            let name = entry
                .split_once(char::is_whitespace)
                .map_or(entry.as_str(), |(_, name)| name.trim());
            let (name, prefix) = match name.strip_suffix('*') {
                Some(name) => (name, true),
                None => (name, false),
            };
            let word = name.rsplit("::").next().unwrap_or(name);
            if word.is_empty() {
                continue;
            }
            names += 1;
            let found = packages.iter().any(|package| {
                let of_crate = words.entry(package.clone()).or_insert_with(|| {
                    let folder = if package == "xtask" {
                        "xtask/".to_owned()
                    } else {
                        format!("crates/{package}/")
                    };
                    let mut all = BTreeSet::new();
                    for file in files {
                        if file.starts_with(&folder)
                            && file.ends_with(".rs")
                            && let Ok(text) = fs::read_to_string(root.join(file))
                        {
                            all.extend(super::lex::names_in(&text).into_iter().map(str::to_owned));
                        }
                    }
                    all
                });
                if prefix {
                    of_crate.iter().any(|known| known.starts_with(word))
                } else {
                    of_crate.contains(word)
                }
            });
            if !found {
                let system = one.os.map_or("the other system", |os| os.name());
                problems.push(format!(
                    "{}: break `{}` is of code only {system} builds, and its `caught-by` names                      `{entry}`, whose name (`{word}`) is a word of no Rust file of {}.\n  There \
                     is no list of the tests of {system}'s build here, so the names are looked \
                     for among the words of the source. The test was renamed or removed: \
                     correct the name. (A test a macro makes from pieces is no word of any \
                     file: name it by the beginning of a word that is there, with a `*`.)",
                    one.place(),
                    one.id,
                    packages.join(", ")
                ));
            }
        }
    }
    (names, problems)
}

/// A message for each `caught-by` entry of `one` that no test of `listed`
/// has: with the break's list and line, and the name.
fn missing(one: &Break, set: &str, listed: &Listed) -> Vec<String> {
    let mut problems = Vec::new();
    for entry in &one.caught_by {
        if listed.tests.iter().any(|id| list::names_id(entry, id)) {
            continue;
        }
        let packages = set.replace(',', ", ");
        let why = if let Some(id) = listed.ignored.iter().find(|id| list::names_id(entry, id)) {
            format!(
                "The only test it names is `{id}`, which is marked `#[ignore]`: a run of the \
                 break does not run it, so it can catch nothing."
            )
        } else {
            let near = similar(entry, &listed.tests);
            let hint = if near.is_empty() {
                String::new()
            } else {
                format!(
                    " Tests it may mean: {}.",
                    near.iter()
                        .map(|id| format!("`{id}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            format!(
                "No test of {packages} is called that ({} tests listed).{hint}",
                listed.tests.len()
            )
        };
        problems.push(format!(
            "{}: break `{}` names a test that does not exist in its `caught-by`: `{entry}`.\n  \
             {why}\n  The test was renamed or removed: correct the name. `cargo xtask mutate \
             --check --tests {}` lists the tests again.",
            one.place(),
            one.id,
            one.id
        ));
    }
    problems
}

/// A few of the tests that `entry` may have meant: one of the same last name
/// (a test that moved to another module), then one whose last name holds
/// `entry`'s or is held by it (a prefix added or dropped: `sys_`), or begins
/// as it does for sixteen characters or more (a name reworded at its end).
/// None for an entry that ends in `*`.
fn similar(entry: &str, tests: &[String]) -> Vec<String> {
    let name = entry
        .split_once(char::is_whitespace)
        .map_or(entry, |(_, name)| name.trim());
    if name.ends_with('*') {
        return Vec::new();
    }
    let last = name.rsplit("::").next().unwrap_or(name);
    if last.is_empty() {
        return Vec::new();
    }
    let kin = |other: &str| {
        let shared = last
            .chars()
            .zip(other.chars())
            .take_while(|(a, b)| a == b)
            .count();
        other.contains(last) || last.contains(other) || shared >= 16
    };
    // Each test with its own last name.
    let named: Vec<(&String, &str)> = tests
        .iter()
        .filter_map(|id| {
            let (_, test) = id.split_once(' ')?;
            Some((id, test.rsplit("::").next().unwrap_or(test)))
        })
        .collect();
    let same = named.iter().filter(|(_, other)| *other == last);
    let like = named
        .iter()
        .filter(|(_, other)| *other != last && kin(other));
    same.chain(like)
        .take(3)
        .map(|(id, _)| (*id).clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::drive::tests::tree;
    use super::super::list::parse;
    use super::{Checked, Listed, Lists, check, lists_path, parse_listing, set_name};

    const LISTING: &str = r#"{
        "rust-build-meta": {},
        "test-count": 4,
        "rust-suites": {
            "demo": { "binary-id": "demo", "testcases": {
                "budget::tests::over_is_over": { "kind": "test", "ignored": false },
                "budget::tests::slow_one": { "kind": "test", "ignored": true }
            }},
            "demo::it": { "binary-id": "demo::it", "testcases": {
                "limits::a_limit_holds": { "kind": "test", "ignored": false },
                "sys_a_process": { "kind": "test", "ignored": false }
            }}
        }
    }"#;

    fn listed() -> Listed {
        parse_listing(LISTING).unwrap()
    }

    fn breaks(body: &str) -> Vec<super::Break> {
        parse(
            "mutations/demo.breaks",
            &format!(
                "[break] b\nrule: r\nfile: crates/demo/src/lib.rs\nfind: x\nreplace: y\n{body}"
            ),
        )
        .unwrap()
    }

    /// A tree with one crate, and a list of its tests kept for it.
    fn kept(test: &str) -> PathBuf {
        let root = tree(
            test,
            &[
                ("crates/demo/Cargo.toml", "[package]\n"),
                ("crates/demo/src/lib.rs", "pub fn x() {}\n"),
                ("Cargo.toml", "[workspace]\n"),
                ("Cargo.lock", ""),
            ],
        );
        let mut asked = 0;
        let mut list = |_: &[String]| {
            asked += 1;
            Ok(listed())
        };
        let made = check(
            &root,
            &breaks("caught-by:\n    budget::tests::over_is_over\n"),
            Some(&mut list),
        );
        assert_eq!(made.unwrap().problems, [] as [String; 0]);
        assert_eq!(asked, 1);
        root
    }

    #[test]
    fn nextests_list_is_read_with_the_ignored_tests_apart() {
        let listed = listed();

        assert_eq!(
            listed.tests,
            [
                "demo budget::tests::over_is_over",
                "demo::it limits::a_limit_holds",
                "demo::it sys_a_process",
            ]
        );
        assert_eq!(listed.ignored, ["demo budget::tests::slow_one"]);
        assert!(parse_listing("{}").is_err());
        assert!(parse_listing(r#"{"rust-suites": {}}"#).is_err());
    }

    #[test]
    fn a_set_of_crates_is_the_same_in_any_order() {
        let set =
            |names: &[&str]| set_name(&names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>());
        assert_eq!(set(&["b", "a", "b"]), "a,b");
        assert_eq!(set(&["a", "b"]), set(&["b", "a"]));
    }

    #[test]
    fn a_caught_by_that_names_no_test_is_refused_with_its_list_line_and_name() {
        let root = kept("names_missing");
        let list = "caught-by:\n    budget::tests::over_is_over\n    budget::tests::renamed\n    demo::it limits::a_limit_*\n";
        let mut again =
            |_: &[String]| -> super::Result<Listed> { unreachable!("the list is fresh") };

        let checked = check(&root, &breaks(list), Some(&mut again)).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(checked.names, 3);
        assert_eq!(checked.problems.len(), 1, "{:#?}", checked.problems);
        let problem = &checked.problems[0];
        assert!(
            problem.starts_with(
                "mutations/demo.breaks:1: break `demo/b` names a test that does not exist"
            ),
            "{problem}"
        );
        assert!(problem.contains("`budget::tests::renamed`"), "{problem}");
        assert!(
            problem.contains("No test of demo is called that (3 tests listed)"),
            "{problem}"
        );
    }

    #[test]
    fn a_test_that_moved_is_offered_by_its_last_name() {
        let root = kept("names_moved");
        let mut never = |_: &[String]| -> super::Result<Listed> { unreachable!() };

        let checked = check(
            &root,
            &breaks("caught-by:\n    old_module::a_limit_holds\n"),
            Some(&mut never),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert!(
            checked.problems[0].contains("Tests it may mean: `demo::it limits::a_limit_holds`"),
            "{:#?}",
            checked.problems
        );
    }

    #[test]
    fn a_test_renamed_at_its_start_or_end_is_offered_by_the_name_it_kept() {
        let root = kept("names_renamed");
        let mut never = |_: &[String]| -> super::Result<Listed> { unreachable!() };
        let mut said = |entry: &str| {
            let list = format!("caught-by:\n    {entry}\n");
            check(&root, &breaks(&list), Some(&mut never))
                .unwrap()
                .problems
                .remove(0)
        };

        // A `sys_` put in front of a name.
        assert!(
            said("demo::it a_process").contains("Tests it may mean: `demo::it sys_a_process`"),
            "a prefix"
        );
        // A name made longer, and one cut short.
        assert!(
            said("limits::a_limit_holds_for_good").contains("`demo::it limits::a_limit_holds`")
        );
        assert!(said("limits::a_limit_holds_up").contains("`demo::it limits::a_limit_holds`"));
        // Nothing like it: no guess.
        assert!(!said("limits::unrelated").contains("may mean"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_ignored_test_is_not_one_that_can_catch() {
        let root = kept("names_ignored");
        let mut never = |_: &[String]| -> super::Result<Listed> { unreachable!() };

        let checked = check(
            &root,
            &breaks("caught-by:\n    budget::tests::slow_one\n"),
            Some(&mut never),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert!(
            checked.problems[0].contains("marked `#[ignore]`"),
            "{:#?}",
            checked.problems
        );
    }

    #[test]
    fn the_check_alone_reads_a_fresh_list_and_builds_nothing() {
        let root = kept("names_cached");

        let ok = check(
            &root,
            &breaks("caught-by:\n    budget::tests::over_is_over\n"),
            None,
        )
        .unwrap();
        let bad = check(
            &root,
            &breaks("caught-by:\n    budget::tests::gone\n"),
            None,
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(
            ok,
            Checked {
                problems: vec![],
                names: 1,
                unchecked: vec![],
                by_source: 0
            }
        );
        assert_eq!(bad.problems.len(), 1, "{:#?}", bad.problems);
    }

    #[test]
    fn with_no_list_the_check_alone_says_what_it_did_not_check() {
        let root = tree("names_none", &[("crates/demo/src/lib.rs", "")]);

        let checked = check(&root, &breaks("caught-by:\n    anything\n"), None).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(
            checked,
            Checked {
                problems: vec![],
                names: 0,
                unchecked: vec!["demo".to_owned()],
                by_source: 0
            }
        );
    }

    #[test]
    fn a_list_made_for_other_code_is_not_used() {
        let root = kept("names_stale");
        std::fs::write(root.join("crates/demo/src/lib.rs"), "pub fn x() { 1; }\n").unwrap();

        let checked = check(
            &root,
            &breaks("caught-by:\n    budget::tests::gone\n"),
            None,
        )
        .unwrap();

        // A change to another crate does not make it stale.
        std::fs::create_dir_all(root.join("crates/other/src")).unwrap();
        std::fs::write(root.join("crates/other/src/lib.rs"), "").unwrap();
        std::fs::write(root.join("crates/demo/src/lib.rs"), "pub fn x() {}\n").unwrap();
        let after = check(
            &root,
            &breaks("caught-by:\n    budget::tests::gone\n"),
            None,
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(checked.problems, [] as [String; 0]);
        assert_eq!(checked.unchecked, ["demo"]);
        assert_eq!(after.problems.len(), 1, "{:#?}", after.problems);
    }

    #[test]
    fn a_break_that_is_to_survive_or_names_no_test_has_no_names_to_check() {
        let root = tree("names_survives", &[("crates/demo/src/lib.rs", "")]);
        let mut never = |_: &[String]| -> super::Result<Listed> { unreachable!() };

        let none = check(&root, &breaks(""), Some(&mut never)).unwrap();
        let survives = check(&root, &breaks("expect: survives\n"), Some(&mut never)).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(none, Checked::default());
        assert_eq!(survives, Checked::default());
    }

    #[test]
    fn a_break_of_the_other_systems_code_has_no_tests_built_here_and_is_not_checked_for_names() {
        let root = kept("names_other_system");
        let mut never = |_: &[String]| -> super::Result<Listed> { unreachable!() };
        let (other, here) = if cfg!(windows) {
            ("linux", "windows")
        } else {
            ("windows", "linux")
        };
        let gone = "caught-by:\n    budget::tests::gone\n";

        let elsewhere = check(
            &root,
            &breaks(&format!("os: {other}\n{gone}")),
            Some(&mut never),
        )
        .unwrap();
        let local = check(&root, &breaks(&format!("os: {here}\n{gone}")), None).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        // No list of its tests is read, and none is built for it: its names
        // are looked for in the words of the source, and `gone` is in none.
        assert_eq!(elsewhere.by_source, 1);
        assert_eq!(elsewhere.names, 0);
        assert_eq!(elsewhere.unchecked, [] as [String; 0]);
        assert_eq!(elsewhere.problems.len(), 1, "{:#?}", elsewhere.problems);
        assert_eq!(local.problems.len(), 1, "{:#?}", local.problems);
    }

    /// The problems with the names of a break of the other system's code,
    /// against a crate whose source has a test of Windows only.
    fn by_words(test: &str, names: &str) -> (usize, Vec<String>) {
        let root = tree(
            test,
            &[
                ("crates/demo/Cargo.toml", "[package]\n"),
                (
                    "crates/demo/src/lib.rs",
                    "pub fn x() {}\n// fn only_in_a_comment() {}\n\
                     #[cfg(windows)]\nmod windows_only {\n    #[test]\n    fn follows_a_link() {}\n}\n",
                ),
                (
                    "crates/demo/tests/it/main.rs",
                    "#[test]\nfn sys_opens_a_pipe() {}\n",
                ),
            ],
        );
        let (other, _) = if cfg!(windows) {
            ("linux", "windows")
        } else {
            ("windows", "linux")
        };
        let mut never = |_: &[String]| -> super::Result<Listed> { unreachable!() };
        let checked = check(
            &root,
            &breaks(&format!("os: {other}\ncaught-by:\n{names}")),
            Some(&mut never),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&root);
        (checked.by_source, checked.problems)
    }

    #[test]
    fn the_names_of_a_break_of_the_other_systems_code_are_looked_for_in_the_words_of_the_source() {
        // A test of the other system's only is a word of the source, in a
        // module or an integration binary, however the entry names it.
        let (names, problems) = by_words(
            "names_by_words_found",
            "    windows_only::follows_a_link\n    demo::it sys_opens_a_pipe\n    sys_opens*\n    \
             windows_only::*\n",
        );

        assert_eq!(problems, [] as [String; 0]);
        // The entry that ends in `::*` has no word to look for.
        assert_eq!(names, 3);
    }

    #[test]
    fn a_renamed_test_of_the_other_systems_code_is_found_without_a_windows_build() {
        for gone in [
            "windows_only::follows_a_link_to_a_file",
            "only_in_a_comment",
            "follows_b_*",
            "demo::it sys_opens_a_socket",
        ] {
            let (names, problems) = by_words("names_by_words_gone", &format!("    {gone}\n"));

            assert_eq!(names, 1, "{gone}");
            assert_eq!(problems.len(), 1, "{gone}: {problems:#?}");
            assert!(
                problems[0].contains("is a word of no Rust file of demo"),
                "{}",
                problems[0]
            );
            assert!(
                problems[0].contains("only Windows builds"),
                "{}",
                problems[0]
            );
        }
    }

    #[test]
    fn the_lists_are_kept_and_read_back_and_a_damaged_file_is_no_lists() {
        let root = tree("names_file", &[("crates/demo/src/lib.rs", "")]);
        let path = lists_path(&root);
        let mut lists = Lists::default();
        lists.sets.insert(
            "demo".to_owned(),
            super::Stored {
                key: "k".to_owned(),
                listed: listed(),
            },
        );

        lists.save(&path).unwrap();
        let read = Lists::load(&path);
        std::fs::write(&path, "{\"sets\": [").unwrap();
        let damaged = Lists::load(&path);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(read, lists);
        assert_eq!(damaged, Lists::default());
    }
}
