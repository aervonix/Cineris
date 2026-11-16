//! `cargo xtask mutate --changed REF`: only the breaks a diff can affect.
//!
//! A full run of a big step's breaks is half an hour, and again after each
//! fix. Most of a fix's diff is far from most breaks. `--changed REF` runs
//! the breaks the working tree's diff from `REF` can affect, and says how
//! many it left and why. A full run stays the default, and is what a report
//! rests on.
//!
//! **The rule, and the one that matters most: it never skips a break the
//! diff can affect.** A break is chosen when the diff, the working tree
//! against `REF` with the files git does not know yet in it, and both names
//! of a file that was moved:
//!
//! 1. touches the build's own settings (the workspace's `Cargo.toml`,
//!    `Cargo.lock`, `.config/nextest.toml`, `rust-toolchain.toml`: what every
//!    break's build reads), or the `Cargo.toml` or `build.rs` of a crate the
//!    break runs; or
//! 2. adds or changes the break itself: its text, by what decides what it
//!    does ([`Break::definition`]), is not that of any break of the lists at
//!    `REF` (a reworded `rule` is not a change); or
//! 3. touches its `file`; or
//! 4. touches a test its `caught-by` names, as [`placed`] reads it.
//!
//! **Which test a name is** is the hard part: a test's path comes from `mod`
//! lines across files, macros make tests no `#[test]` names, and a `*` ends
//! a name. So a name is read by its last segment (`no_job_starts_a_piece`
//! of `mutate::jobs::tests::no_job_starts_a_piece`), and a test is touched
//! when a file of the crates the break runs that the diff touches (as it is
//! now, or as it was at `REF`: a test that was removed or moved) has that
//! word among its tokens, anywhere (a definition, a macro's argument, a
//! call). That over-chooses (a helper of the same name, a test of the same
//! name in another module) and under-chooses nothing a token can show. When
//! no file of those crates has the word at all (a test that a macro made
//! from pieces, a name that is a pattern ending in `::*`) the name cannot be
//! placed, and the break is chosen when **any file of those crates** was
//! touched, and counted as such. What the rule does not see: a change to
//! something a test calls or reads (code of the crate that is not the
//! break's file, a fixture), and a break that names no test (it is chosen by
//! its file, its text and the settings only). A full run is for those.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::lex;
use super::list::{self, Break};
use crate::Result;

/// The files whose change touches every break: what each break's build
/// reads.
const SETTINGS: [&str; 5] = [
    "Cargo.toml",
    "Cargo.lock",
    ".config/nextest.toml",
    "rust-toolchain.toml",
    "rust-toolchain",
];

/// The text of the files the rule reads: now, at the reference, and which
/// Rust files there are.
pub(crate) trait Sources {
    /// The file as it was at the reference; `None` when it was not there.
    fn old(&self, path: &str) -> Option<String>;
    /// The file as it is; `None` when it is gone.
    fn new(&self, path: &str) -> Option<String>;
    /// The Rust files now under `directory`, as paths of the repository.
    fn rust_files(&self, directory: &str) -> Vec<String>;
}

/// Why a break was chosen: the first of the rule's four that holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// The build's settings changed (the file).
    Settings(String),
    /// The break is new or changed in its list.
    Itself,
    /// The break's `file` was touched.
    File,
    /// A test its `caught-by` names was touched (the entry).
    Test(String),
    /// A test its `caught-by` names could not be placed in any file, and a
    /// file of the crates it runs was touched (the entry).
    Unplaced(String),
}

impl Reason {
    /// In a sentence, for `--list`.
    fn said(&self) -> String {
        match self {
            Self::Settings(file) => format!("`{file}` changed, and every break's build reads it"),
            Self::Itself => "the break is new or changed in its list".to_owned(),
            Self::File => "its file was touched".to_owned(),
            Self::Test(entry) => format!("a test it names was touched: `{entry}`"),
            Self::Unplaced(entry) => format!(
                "`{entry}` could not be placed in any file, and a file of the crates it runs was \
                 touched"
            ),
        }
    }
}

/// What the rule chose.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    /// The breaks chosen, in the order given, each with why.
    pub(crate) chosen: Vec<(Break, Reason)>,
    /// How many breaks were given.
    pub(crate) given: usize,
    /// How many files the diff touches.
    pub(crate) files: usize,
}

impl Selection {
    /// The breaks, without the reasons.
    pub(crate) fn breaks(&self) -> Vec<Break> {
        self.chosen.iter().map(|(one, _)| one.clone()).collect()
    }

    /// Why each chosen break was, by its id, for `--list`.
    pub(crate) fn reasons(&self) -> BTreeMap<String, String> {
        self.chosen
            .iter()
            .map(|(one, reason)| (one.id.clone(), reason.said()))
            .collect()
    }

    /// What to say of it: how many were chosen, by what, and how many were
    /// left and why.
    pub(crate) fn summary(&self, reference: &str) -> String {
        let count = |wanted: fn(&Reason) -> bool| {
            self.chosen
                .iter()
                .filter(|(_, reason)| wanted(reason))
                .count()
        };
        let skipped = self.given - self.chosen.len();
        let mut said = format!(
            "--changed {reference}: the diff touches {} file(s). {} of {} break(s) chosen",
            self.files,
            self.chosen.len(),
            self.given
        );
        if !self.chosen.is_empty() {
            let parts = [
                (
                    count(|r| matches!(r, Reason::Settings(_))),
                    "by a change to the build's settings",
                ),
                (
                    count(|r| *r == Reason::Itself),
                    "new or changed in their lists",
                ),
                (count(|r| *r == Reason::File), "by their file"),
                (
                    count(|r| matches!(r, Reason::Test(_))),
                    "by a test they name",
                ),
                (
                    count(|r| matches!(r, Reason::Unplaced(_))),
                    "whose test names could not be placed in any file, chosen to be safe",
                ),
            ];
            let parts: Vec<String> = parts
                .iter()
                .filter(|(number, _)| *number > 0)
                .map(|(number, what)| format!("{number} {what}"))
                .collect();
            said.push_str(&format!(" ({})", parts.join(", ")));
        }
        said.push_str(&format!(
            "; {skipped} skipped: the diff touches neither their file, nor a test they name, nor \
             their text, nor the build's settings. A full run (no `--changed`) is what a \
             report rests on."
        ));
        said
    }
}

/// The breaks of `given` the diff can affect (see the module's rule).
///
/// `known` is every break of `mutations/` as it is now, which with the lists
/// the diff touches (their text at the reference, from `sources`) tells
/// which breaks are new or changed. `changed` is the files the diff touches,
/// every name of a file that was moved.
pub(crate) fn select(
    known: &[Break],
    given: &[Break],
    changed: &[String],
    sources: &dyn Sources,
) -> Selection {
    let touched: BTreeSet<String> = changed.iter().map(|path| path.replace('\\', "/")).collect();
    let settings = SETTINGS.iter().find(|file| touched.contains(**file));
    let before = definitions_before(known, &touched, sources);
    let mut crates = Crates::new(&touched, sources);
    let chosen = given
        .iter()
        .filter_map(|one| {
            let reason = if let Some(file) = settings {
                Some(Reason::Settings((*file).to_owned()))
            } else if !before.contains(&one.definition()) {
                Some(Reason::Itself)
            } else if touched.contains(&one.file) {
                Some(Reason::File)
            } else {
                crates.touched_by(one)
            };
            reason.map(|reason| (one.clone(), reason))
        })
        .collect();
    Selection {
        chosen,
        given: given.len(),
        files: touched.len(),
    }
}

/// What decides each break of the lists at the reference: the breaks of the
/// lists the diff does not touch (the same as at the reference), and those
/// of the touched lists as they were.
fn definitions_before(
    known: &[Break],
    touched: &BTreeSet<String>,
    sources: &dyn Sources,
) -> HashSet<String> {
    let mut before: HashSet<String> = known
        .iter()
        .filter(|one| !touched.contains(&one.list))
        .map(Break::definition)
        .collect();
    for path in touched {
        let is_list = path
            .strip_prefix(list::DIRECTORY)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|name| name.ends_with(".breaks") && !name.contains('/'));
        if !is_list {
            continue;
        }
        // A list that cannot be read as it was (it is new, or was not a
        // list) has no breaks before.
        if let Some(text) = sources.old(path)
            && let Ok(old) = list::parse(path, &text)
        {
            before.extend(old.iter().map(Break::definition));
        }
    }
    before
}

/// The crates the breaks run, and what the diff touches of each.
struct Crates<'a> {
    touched: &'a BTreeSet<String>,
    sources: &'a dyn Sources,
    /// By directory: the words of its Rust files and where they are.
    words: HashMap<String, HashMap<String, Vec<String>>>,
}

impl<'a> Crates<'a> {
    fn new(touched: &'a BTreeSet<String>, sources: &'a dyn Sources) -> Self {
        Self {
            touched,
            sources,
            words: HashMap::new(),
        }
    }

    /// The reason a test or a crate's own files make `one` chosen, if any
    /// do: rule 1's crate files and rule 4.
    fn touched_by(&mut self, one: &Break) -> Option<Reason> {
        // A break that runs no crate can be placed nowhere: chosen.
        let Ok(packages) = one.test_packages() else {
            return Some(Reason::Unplaced(one.file.clone()));
        };
        let touched = self.touched;
        let directories: Vec<String> = packages.iter().map(|name| directory_of(name)).collect();
        for directory in &directories {
            for file in ["Cargo.toml", "build.rs"] {
                let path = format!("{directory}/{file}");
                if self.touched.contains(&path) {
                    return Some(Reason::Settings(path));
                }
            }
        }
        let in_directories = |path: &String| {
            directories
                .iter()
                .any(|directory| path.starts_with(&format!("{directory}/")))
        };
        if !touched.iter().any(in_directories) {
            return None;
        }
        for entry in &one.caught_by {
            match placed(entry) {
                Placed::Nowhere => return Some(Reason::Unplaced(entry.clone())),
                Placed::Word { word, prefix } => {
                    let mut found = false;
                    for directory in &directories {
                        let words = self.words_of(directory);
                        let files: Vec<&String> = if prefix {
                            words
                                .iter()
                                .filter(|(known, _)| known.starts_with(&word))
                                .flat_map(|(_, files)| files)
                                .collect()
                        } else {
                            words.get(&word).into_iter().flatten().collect()
                        };
                        found |= !files.is_empty();
                        if files.iter().any(|file| touched.contains(*file)) {
                            return Some(Reason::Test(entry.clone()));
                        }
                    }
                    if !found {
                        // No file of the crates has the word: made by a
                        // macro from pieces, or a name that is gone.
                        return Some(Reason::Unplaced(entry.clone()));
                    }
                }
            }
        }
        None
    }

    /// Each word of the Rust files under `directory` (now, and as the ones
    /// the diff touches were), with the files that have it.
    fn words_of(&mut self, directory: &str) -> &HashMap<String, Vec<String>> {
        let (touched, sources) = (self.touched, self.sources);
        self.words.entry(directory.to_owned()).or_insert_with(|| {
            let mut words: HashMap<String, Vec<String>> = HashMap::new();
            let mut add = |path: &str, text: &str| {
                let code = lex::code_tokens(text);
                for word in code
                    .iter()
                    .map(|token| token.text(text))
                    .filter(|text| is_word(text))
                    .collect::<HashSet<_>>()
                {
                    let files = words.entry(word.to_owned()).or_default();
                    if !files.iter().any(|file| file == path) {
                        files.push(path.to_owned());
                    }
                }
            };
            let mut now = sources.rust_files(directory);
            for path in touched {
                if path.starts_with(&format!("{directory}/"))
                    && path.ends_with(".rs")
                    && !now.contains(path)
                {
                    // Gone now, and there before.
                    now.push(path.clone());
                }
            }
            for path in &now {
                if let Some(text) = sources.new(path) {
                    add(path, &text);
                }
                if touched.contains(path)
                    && let Some(text) = sources.old(path)
                {
                    add(path, &text);
                }
            }
            words
        })
    }
}

/// Whether a token is a name: letters, digits and `_`, and not a number.
fn is_word(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The directory a crate is in: `crates/<name>`, and `xtask`.
fn directory_of(package: &str) -> String {
    if package == "xtask" {
        "xtask".to_owned()
    } else {
        format!("crates/{package}")
    }
}

/// What a `caught-by` entry says about where its test is.
#[derive(Debug, PartialEq, Eq)]
enum Placed {
    /// A word of the test's name: its last segment, or its beginning when
    /// the entry ends in `*`.
    Word { word: String, prefix: bool },
    /// Nothing to look for: the entry ends in `::*` or is only `*`, any test
    /// of a module.
    Nowhere,
}

/// The word to look for of a `caught-by` entry: the last segment of the
/// test's name, after the binary if the entry names one
/// (`cortex-llm::it openai::endpoints::*`).
fn placed(entry: &str) -> Placed {
    let name = entry
        .split_once(char::is_whitespace)
        .map_or(entry, |(_, name)| name.trim());
    let (name, prefix) = match name.strip_suffix('*') {
        Some(name) => (name, true),
        None => (name, false),
    };
    let last = name.rsplit("::").next().unwrap_or(name);
    if last.is_empty() {
        Placed::Nowhere
    } else {
        Placed::Word {
            word: last.to_owned(),
            prefix,
        }
    }
}

/// The repository at `root`, and the commit a diff is from.
pub(crate) struct Git<'a> {
    root: &'a Path,
    commit: String,
    rust: std::cell::OnceCell<Vec<String>>,
}

impl<'a> Git<'a> {
    /// Opens the repository at `root` for a diff from `reference`. Refuses a
    /// tree with no `.git` of its own (the desk's copy has none), and a
    /// reference that is no commit.
    pub(crate) fn open(root: &'a Path, reference: &str) -> Result<Self> {
        if !root.join(".git").exists() {
            return Err(format!(
                "`--changed {reference}` needs git's history, and {} has no `.git`. At the \
                 desk, `rx` copies the worktree without one: run `rx --git -- cargo xtask mutate \
                 --changed {reference} ...`, which gives the copy a `.git` from a bundle of \
                 every ref.",
                root.display()
            ));
        }
        if reference.is_empty() || reference.starts_with('-') {
            return Err(format!(
                "`--changed {reference}`: a branch, a tag or a commit, not an option"
            ));
        }
        let commit = crate::owner_guard::git(
            root,
            &["rev-parse", "--verify", "--quiet", &format!("{reference}^{{commit}}")],
        )
        .map_err(|why| {
            format!(
                "`--changed {reference}`: `{reference}` is not a commit of this repository ({why}). \
                 Fetch it, or name one that is here: this is never read as \"nothing changed\"."
            )
        })?;
        let commit = commit.trim().to_owned();
        if commit.is_empty() {
            return Err(format!(
                "`--changed {reference}`: `{reference}` is not a commit of this repository."
            ));
        }
        Ok(Self {
            root,
            commit,
            rust: std::cell::OnceCell::new(),
        })
    }

    /// The files the working tree differs in from the reference, new files
    /// git does not track yet in them, and both names of a file that was
    /// moved.
    pub(crate) fn changed_files(&self) -> Result<Vec<String>> {
        let mut files = BTreeSet::new();
        for arguments in [
            vec!["diff", "--name-only", "--no-renames", "-z", &self.commit],
            vec!["ls-files", "--others", "--exclude-standard", "-z"],
        ] {
            let listing = crate::owner_guard::git(self.root, &arguments)?;
            files.extend(
                listing
                    .split('\0')
                    .map(str::trim)
                    .filter(|file| !file.is_empty())
                    .map(str::to_owned),
            );
        }
        Ok(files.into_iter().collect())
    }
}

impl Sources for Git<'_> {
    fn old(&self, path: &str) -> Option<String> {
        crate::owner_guard::git(self.root, &["show", &format!("{}:{path}", self.commit)]).ok()
    }

    fn new(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(path)).ok()
    }

    fn rust_files(&self, directory: &str) -> Vec<String> {
        let all = self.rust.get_or_init(|| {
            crate::workspace::repository_files(self.root)
                .unwrap_or_default()
                .into_iter()
                .filter(|file| file.ends_with(".rs"))
                .collect()
        });
        let within = format!("{directory}/");
        all.iter()
            .filter(|file| file.starts_with(&within))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Placed, Reason, Sources, placed, select};
    use crate::mutate::list::{self, Break};

    /// Files as they are, and as they were, in memory.
    #[derive(Default)]
    struct Files {
        now: BTreeMap<String, String>,
        was: BTreeMap<String, String>,
    }

    impl Files {
        /// A file that is the same now and at the reference.
        fn same(mut self, path: &str, text: &str) -> Self {
            self.now.insert(path.to_owned(), text.to_owned());
            self.was.insert(path.to_owned(), text.to_owned());
            self
        }

        /// A file that was `before` and is `after` (`None`: not there).
        fn edited(mut self, path: &str, before: Option<&str>, after: Option<&str>) -> Self {
            if let Some(text) = before {
                self.was.insert(path.to_owned(), text.to_owned());
            }
            if let Some(text) = after {
                self.now.insert(path.to_owned(), text.to_owned());
            }
            self
        }
    }

    impl Sources for Files {
        fn old(&self, path: &str) -> Option<String> {
            self.was.get(path).cloned()
        }

        fn new(&self, path: &str) -> Option<String> {
            self.now.get(path).cloned()
        }

        fn rust_files(&self, directory: &str) -> Vec<String> {
            self.now
                .keys()
                .filter(|path| path.starts_with(&format!("{directory}/")) && path.ends_with(".rs"))
                .cloned()
                .collect()
        }
    }

    const A: &str = "[break] a\nrule: r\nfile: crates/demo/src/a.rs\nfind: x\nreplace: y\n\
                     caught-by:\n    tests::first_test\n";
    const B: &str = "[break] b\nrule: r\nfile: crates/demo/src/b.rs\nfind: x\nreplace: y\n\
                     caught-by:\n    tests::second_test\n";
    const OTHER: &str = "[break] c\nrule: r\nfile: crates/other/src/c.rs\nfind: x\nreplace: y\n\
                         caught-by:\n    tests::third_test\n";

    fn lists(text: &[(&str, &str)]) -> Vec<Break> {
        text.iter()
            .flat_map(|(name, body)| {
                list::parse(&format!("mutations/{name}.breaks"), body).unwrap()
            })
            .collect()
    }

    /// The ids chosen, with why, for a diff over a tree of two lists.
    fn chosen(files: &Files, changed: &[&str], breaks: &[Break]) -> Vec<(String, Reason)> {
        let changed: Vec<String> = changed.iter().map(|path| (*path).to_owned()).collect();
        select(breaks, breaks, &changed, files)
            .chosen
            .into_iter()
            .map(|(one, reason)| (one.id, reason))
            .collect()
    }

    /// A tree whose lists are as they were at the reference, with two
    /// crates whose tests are in their own files.
    fn tree() -> Files {
        Files::default()
            .same("mutations/demo.breaks", &format!("{A}\n{B}"))
            .same("mutations/other.breaks", OTHER)
            .same("crates/demo/src/a.rs", "pub fn a() {}")
            .same("crates/demo/src/b.rs", "pub fn b() {}")
            .same(
                "crates/demo/src/tests.rs",
                "#[test]\nfn first_test() {}\n#[test]\nfn second_test() {}\n",
            )
            .same("crates/other/src/c.rs", "pub fn c() {}")
            .same("crates/other/src/tests.rs", "#[test]\nfn third_test() {}\n")
    }

    fn all() -> Vec<Break> {
        lists(&[("demo", &format!("{A}\n{B}")), ("other", OTHER)])
    }

    #[test]
    fn nothing_changed_chooses_nothing() {
        assert_eq!(chosen(&tree(), &[], &all()), []);
        // A file no break or test is in.
        assert_eq!(chosen(&tree(), &["docs/testing-and-ci.md"], &all()), []);
    }

    #[test]
    fn a_break_is_chosen_by_its_file_and_only_that_break() {
        assert_eq!(
            chosen(&tree(), &["crates/demo/src/a.rs"], &all()),
            [("demo/a".to_owned(), Reason::File)]
        );
        // A name of a file that was moved counts as much as the other.
        assert_eq!(
            chosen(
                &tree(),
                &["crates/other/src/c.rs", "crates/other/src/moved.rs"],
                &all()
            ),
            [("other/c".to_owned(), Reason::File)]
        );
    }

    #[test]
    fn a_break_is_chosen_by_a_test_it_names_and_not_by_another() {
        let files = tree().edited(
            "crates/demo/src/tests.rs",
            Some("#[test]\nfn first_test() {}\n#[test]\nfn second_test() {}\n"),
            Some("#[test]\nfn first_test() { assert!(true) }\n#[test]\nfn second_test() {}\n"),
        );
        // Both tests are in the one file the diff touches: both are touched.
        let both = chosen(&files, &["crates/demo/src/tests.rs"], &all());
        assert_eq!(
            both,
            [
                (
                    "demo/a".to_owned(),
                    Reason::Test("tests::first_test".to_owned())
                ),
                (
                    "demo/b".to_owned(),
                    Reason::Test("tests::second_test".to_owned())
                ),
            ]
        );
        // A crate's test file is not another crate's.
        assert!(both.iter().all(|(id, _)| id != "other/c"));
        // A test in a file of its own is touched alone.
        let files = tree()
            .same("crates/demo/src/tests.rs", "fn second_test() {}\n")
            .same("crates/demo/src/more.rs", "fn first_test() {}\n");
        assert_eq!(
            chosen(&files, &["crates/demo/src/more.rs"], &all()),
            [(
                "demo/a".to_owned(),
                Reason::Test("tests::first_test".to_owned())
            )]
        );
    }

    #[test]
    fn a_test_that_was_removed_or_moved_is_touched_by_where_it_was() {
        // Removed: the file no longer has the word, and had it.
        let removed = tree().edited(
            "crates/demo/src/tests.rs",
            Some("#[test]\nfn first_test() {}\n#[test]\nfn second_test() {}\n"),
            Some("#[test]\nfn second_test() {}\n"),
        );
        assert!(
            chosen(&removed, &["crates/demo/src/tests.rs"], &all())
                .iter()
                .any(|(id, reason)| id == "demo/a" && matches!(reason, Reason::Test(_)))
        );
        // Moved with its file: the old name's text is the old one's.
        let moved = Files::default()
            .same("mutations/demo.breaks", &format!("{A}\n{B}"))
            .edited(
                "crates/demo/src/old_tests.rs",
                Some("fn first_test() {}"),
                None,
            )
            .edited(
                "crates/demo/src/new_tests.rs",
                None,
                Some("fn first_test() {}"),
            )
            .same("crates/demo/src/other.rs", "fn second_test() {}");
        assert_eq!(
            chosen(
                &moved,
                &[
                    "crates/demo/src/old_tests.rs",
                    "crates/demo/src/new_tests.rs"
                ],
                &all()
            ),
            [(
                "demo/a".to_owned(),
                Reason::Test("tests::first_test".to_owned())
            )]
        );
    }

    #[test]
    fn a_name_no_file_has_is_chosen_by_any_file_of_the_crates_and_said_so() {
        // `first_test` is made by a macro from pieces: no file has the word.
        let files = Files::default()
            .same("mutations/demo.breaks", &format!("{A}\n{B}"))
            .same(
                "crates/demo/src/tests.rs",
                "fn second_test() {}\nmake!(first, test);",
            )
            .same("crates/demo/src/lib.rs", "pub fn lib() {}")
            .same("crates/other/src/lib.rs", "pub fn lib() {}");
        let breaks = lists(&[("demo", &format!("{A}\n{B}"))]);

        assert_eq!(
            chosen(&files, &["crates/demo/src/lib.rs"], &breaks),
            [(
                "demo/a".to_owned(),
                Reason::Unplaced("tests::first_test".to_owned())
            )]
        );
        // A file of another crate leaves it.
        assert_eq!(chosen(&files, &["crates/other/src/lib.rs"], &breaks), []);
        // A pattern of a whole module cannot be placed either.
        let list = A.replace("tests::first_test", "tests::*");
        let breaks = lists(&[("demo", &list)]);
        let files = Files::default()
            .same("mutations/demo.breaks", &list)
            .same("crates/demo/src/lib.rs", "pub fn lib() {}");
        assert_eq!(
            chosen(&files, &["crates/demo/src/lib.rs"], &breaks),
            [("demo/a".to_owned(), Reason::Unplaced("tests::*".to_owned()))]
        );
    }

    #[test]
    fn a_name_that_ends_in_a_star_is_a_beginning() {
        assert_eq!(
            placed("mutate::tests::sys_*"),
            Placed::Word {
                word: "sys_".to_owned(),
                prefix: true
            }
        );
        assert_eq!(placed("tests::*"), Placed::Nowhere);
        assert_eq!(placed("*"), Placed::Nowhere);
        // The binary of an entry is not part of the name.
        assert_eq!(
            placed("cortex-llm::it openai_chat::endpoints::an_url"),
            Placed::Word {
                word: "an_url".to_owned(),
                prefix: false
            }
        );
        assert_eq!(placed("cortex-config dirs::tests::*"), Placed::Nowhere);
        let list = A.replace("tests::first_test", "tests::first_*");
        let files = tree().same("crates/demo/src/extra.rs", "fn first_other() {}");
        assert_eq!(
            chosen(
                &files,
                &["crates/demo/src/extra.rs"],
                &lists(&[("demo", &list)])
            ),
            [(
                "demo/a".to_owned(),
                Reason::Test("tests::first_*".to_owned())
            )]
        );
    }

    #[test]
    fn a_break_is_chosen_when_it_is_new_or_changed_in_its_list_and_not_when_only_its_rule_is() {
        // At the reference: `a` only, and `b` with another find.
        let was = format!("{A}\n{}", B.replace("find: x", "find: z"));
        let files = tree().edited(
            "mutations/demo.breaks",
            Some(&was),
            Some(&format!("{A}\n{B}")),
        );
        let new_c = lists(&[("demo", &format!("{A}\n{B}")), ("other", OTHER)]);
        let got = chosen(&files, &["mutations/demo.breaks"], &new_c);
        assert_eq!(got, [("demo/b".to_owned(), Reason::Itself)]);

        // A reworded rule is no change.
        let was = format!("{}\n{B}", A.replace("rule: r", "rule: an older sentence"));
        let files = tree().edited(
            "mutations/demo.breaks",
            Some(&was),
            Some(&format!("{A}\n{B}")),
        );
        assert_eq!(chosen(&files, &["mutations/demo.breaks"], &new_c), []);

        // A list that is new: every break of it is. A list that was
        // renamed: its breaks are what they were.
        let files = Files::default()
            .same("mutations/other.breaks", OTHER)
            .edited("mutations/demo.breaks", Some(&format!("{A}\n{B}")), None)
            .edited("mutations/step.breaks", None, Some(&format!("{A}\n{B}")));
        let moved = lists(&[("step", &format!("{A}\n{B}")), ("other", OTHER)]);
        assert_eq!(
            chosen(
                &files,
                &["mutations/demo.breaks", "mutations/step.breaks"],
                &moved
            ),
            []
        );
        let files = Files::default()
            .same("mutations/other.breaks", OTHER)
            .edited("mutations/demo.breaks", None, Some(&format!("{A}\n{B}")));
        assert_eq!(
            chosen(&files, &["mutations/demo.breaks"], &new_c)
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            ["demo/a", "demo/b"]
        );
    }

    #[test]
    fn a_break_that_is_in_no_list_of_the_reference_is_new_wherever_it_is_read_from() {
        // A reviewer's list, given with `--file`: not in `mutations/`.
        let known = all();
        let given = lists(&[("review", &A.replace("[break] a", "[break] fresh"))]);
        let files = tree();
        let got = select(&known, &given, &["docs/x.md".to_owned()], &files);
        // Its text is that of `a`, which the reference has: not new.
        assert!(got.chosen.is_empty(), "{:?}", got.chosen);
        let other = lists(&[("review", &A.replace("find: x", "find: new text"))]);
        let got = select(&known, &other, &["docs/x.md".to_owned()], &files);
        assert_eq!(got.chosen[0].1, Reason::Itself);
    }

    #[test]
    fn a_change_to_the_builds_settings_chooses_every_break() {
        for file in ["Cargo.lock", "Cargo.toml", ".config/nextest.toml"] {
            let got = chosen(&tree(), &[file], &all());
            assert_eq!(got.len(), 3, "{file}");
            assert!(
                got.iter()
                    .all(|(_, reason)| matches!(reason, Reason::Settings(_)))
            );
        }
        // A crate's manifest chooses the breaks that run that crate.
        assert_eq!(
            chosen(&tree(), &["crates/other/Cargo.toml"], &all()),
            [(
                "other/c".to_owned(),
                Reason::Settings("crates/other/Cargo.toml".to_owned())
            )]
        );
    }

    #[test]
    fn the_packages_a_break_names_are_the_crates_it_is_judged_in() {
        // `a` runs `other`'s tests too: a touched file of `other` that has
        // the test's name chooses it.
        let list = A.replace("replace: y\n", "replace: y\npackages: demo, other\n");
        let files = tree().same("crates/other/src/tests.rs", "fn first_test() {}");
        let breaks = lists(&[("demo", &list)]);
        assert_eq!(
            chosen(&files, &["crates/other/src/tests.rs"], &breaks),
            [(
                "demo/a".to_owned(),
                Reason::Test("tests::first_test".to_owned())
            )]
        );
    }

    #[test]
    fn a_break_that_names_no_test_is_chosen_by_its_file_alone() {
        let list = "[break] a\nrule: r\nfile: crates/demo/src/a.rs\nfind: x\nreplace: y\n";
        let breaks = lists(&[("demo", list)]);
        let files = Files::default()
            .same("mutations/demo.breaks", list)
            .same("crates/demo/src/a.rs", "")
            .same("crates/demo/src/tests.rs", "");
        assert_eq!(chosen(&files, &["crates/demo/src/tests.rs"], &breaks), []);
        assert_eq!(
            chosen(&files, &["crates/demo/src/a.rs"], &breaks),
            [("demo/a".to_owned(), Reason::File)]
        );
    }

    #[test]
    fn the_summary_says_how_many_were_chosen_by_what_and_how_many_left() {
        let changed = ["crates/demo/src/a.rs".to_owned()];
        let got = select(&all(), &all(), &changed, &tree());

        let said = got.summary("origin/main");

        assert!(
            said.starts_with(
                "--changed origin/main: the diff touches 1 file(s). 1 of 3 break(s) chosen (1 by their file)"
            ),
            "{said}"
        );
        assert!(said.contains("2 skipped"), "{said}");
        assert_eq!(got.reasons()["demo/a"], "its file was touched");
    }

    /// `git` in `root`, as a commit of a made-up identity.
    fn git(root: &std::path::Path, arguments: &[&str]) -> String {
        let mut all = vec![
            "-c",
            "user.name=tester",
            "-c",
            "user.email=tester@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
        ];
        all.extend(arguments);
        crate::owner_guard::git(root, &all).unwrap()
    }

    #[test]
    fn sys_a_real_repository_gives_the_files_a_diff_touches_with_both_names_of_a_move() {
        let root = crate::mutate::drive::tests::tree(
            "changed-git",
            &[
                ("mutations/demo.breaks", &format!("{A}\n{B}")),
                ("crates/demo/Cargo.toml", "[package]\n"),
                ("crates/demo/src/a.rs", "pub fn a() {}\n"),
                ("crates/demo/src/b.rs", "pub fn b() {}\n"),
                (
                    "crates/demo/src/tests.rs",
                    "#[test]\nfn first_test() {}\n#[test]\nfn second_test() {}\n",
                ),
            ],
        );
        git(&root, &["init", "-q"]);
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "base"]);
        // A change to a file, a file moved (staged), a new file that git
        // has not been told of, and a list's break that is new.
        std::fs::write(root.join("crates/demo/src/a.rs"), "pub fn a() { 1; }\n").unwrap();
        git(
            &root,
            &["mv", "crates/demo/src/b.rs", "crates/demo/src/bb.rs"],
        );
        std::fs::write(root.join("crates/demo/src/new.rs"), "pub fn new() {}\n").unwrap();

        let opened = super::Git::open(&root, "HEAD").unwrap();
        let files = opened.changed_files().unwrap();

        assert_eq!(
            files,
            [
                "crates/demo/src/a.rs",
                "crates/demo/src/b.rs",
                "crates/demo/src/bb.rs",
                "crates/demo/src/new.rs",
            ]
        );
        // The text at the reference, and now; the Rust files are those of
        // the tree, a new one among them.
        assert_eq!(opened.old("crates/demo/src/a.rs").unwrap(), "pub fn a() {}\n");
        assert_eq!(opened.old("crates/demo/src/new.rs"), None);
        assert_eq!(opened.new("crates/demo/src/b.rs"), None);
        assert!(
            opened
                .rust_files("crates/demo")
                .contains(&"crates/demo/src/new.rs".to_owned())
        );
        let known = lists(&[("demo", &format!("{A}\n{B}"))]);
        let got = select(&known, &known, &files, &opened);
        assert_eq!(
            got.chosen
                .iter()
                .map(|(one, reason)| (one.id.as_str(), reason.clone()))
                .collect::<Vec<_>>(),
            [("demo/a", Reason::File), ("demo/b", Reason::File)]
        );

        // A reference that is no commit is an error, never "nothing changed".
        for bad in ["no-such-branch", "-x", ""] {
            let error = super::Git::open(&root, bad).err().unwrap();
            assert!(error.contains("--changed"), "{bad}: {error}");
        }
        let error = super::Git::open(&root, "no-such-branch").err().unwrap();
        assert!(
            error.contains("is not a commit of this repository"),
            "{error}"
        );
        // After a commit, the reference is the older commit.
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "second"]);
        let opened = super::Git::open(&root, "HEAD").unwrap();
        assert_eq!(opened.changed_files().unwrap(), Vec::<String>::new());
        let opened = super::Git::open(&root, "HEAD~1").unwrap();
        assert_eq!(opened.changed_files().unwrap().len(), 4);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tree_with_no_git_is_told_how_to_have_one() {
        let root = crate::mutate::drive::tests::tree("changed-no-git", &[("a.txt", "x")]);

        let error = super::Git::open(&root, "origin/main").err().unwrap();

        assert!(error.contains("has no `.git`"), "{error}");
        assert!(
            error.contains("rx --git -- cargo xtask mutate --changed origin/main"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
