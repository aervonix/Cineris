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
//!    `Cargo.lock`, `.config/nextest.toml`, `rust-toolchain.toml`,
//!    `.cargo/config.toml`: what every break's build reads), or a file
//!    outside `crates/`, `xtask/`, `docs/`, `mutations/` and `.github/` that
//!    is not Markdown and that no crate's Rust names (what is not known is
//!    taken to touch every break), or the `Cargo.toml` or `build.rs` of a
//!    crate the break runs or of one it is built on; or
//! 2. adds or changes the break itself: its text, by what decides what it
//!    does ([`Break::definition`]), is not that of any break of the lists at
//!    `REF` (a reworded `rule` is not a change); or
//! 3. touches its `file`; or
//! 4. touches a test its `caught-by` names, as [`placed`] reads it; or
//! 5. touches a file that is not Rust (a data file, such as the model
//!    catalog, wherever it is) that a crate's Rust names, and the break runs
//!    that crate or one built on it ([`super::readers`] has how a reader is
//!    found, and what it cannot see).
//!
//! **Which test a name is** is the hard part: a test's path comes from `mod`
//! lines across files, macros make tests no `#[test]` names, and a `*` ends
//! a name. So a name is read by its last segment (`no_job_starts_a_piece`
//! of `mutate::jobs::tests::no_job_starts_a_piece`), and a test is touched
//! when a file of the crates the break runs that the diff touches (as it is
//! now, or as it was at `REF`: a test that was removed or moved) has that
//! word among its tokens, anywhere (a definition, a macro's argument, a
//! call). That over-chooses (a helper of the same name, a test of the same
//! name in another module). When no file of those crates has the word at all
//! (a test that a macro made from pieces), or the name is a pattern (it ends
//! in `*`, which can end inside a module's name: `open*` is every test of
//! `openai_chat`), the name cannot be placed, and the break is chosen when
//! **any file of those crates** was touched, and counted as such. A file of
//! those crates that is not Rust (a fixture, a snapshot, a file a test reads
//! or `include_str!` takes) chooses the break too, counted the same way.
//! What the rule does not see: a change to Rust code a test calls that is
//! not the break's file nor names a test it names (a helper elsewhere in the
//! crate, the test kit), a break that names no test (it is chosen by its
//! file, its text and the settings only), and a data file read by a path no
//! string of Rust names ([`super::readers`]). A full run is for those.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::budget::Class;
use super::lex;
use super::list::{self, Break};
use super::readers;
use crate::Result;
use crate::workspace::Package;

/// The files whose change touches every break: what each break's build
/// reads.
const SETTINGS: [&str; 7] = [
    "Cargo.toml",
    "Cargo.lock",
    ".config/nextest.toml",
    "rust-toolchain.toml",
    "rust-toolchain",
    ".cargo/config.toml",
    ".cargo/config",
];

/// The folders whose files the rule reads one by one: anything else that is
/// not Markdown is taken to touch every break's build.
const KNOWN: [&str; 5] = ["crates/", "xtask/", "docs/", "mutations/", ".github/"];

/// Whether `path` is outside the folders the rule reads one by one.
fn outside_known(path: &str) -> bool {
    !KNOWN.iter().any(|folder| path.starts_with(folder))
}

/// Whether `path` is Markdown.
fn markdown(path: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension == "md")
}

/// The text of the files the rule reads: now, at the reference, and which
/// Rust files there are.
pub(crate) trait Sources {
    /// The file as it was at the reference; `None` when it was not there.
    fn old(&self, path: &str) -> Option<String>;
    /// The file as it is; `None` when it is gone.
    fn now(&self, path: &str) -> Option<String>;
    /// The Rust files now under `directory`, as paths of the repository.
    fn rust_files(&self, directory: &str) -> Vec<String>;
    /// The workspace's packages, with their dependencies: asked only when
    /// the diff touches a file that is not Rust, or a crate's manifest.
    fn packages(&self) -> Result<Vec<Package>>;
}

/// Why a break was chosen: the first of the rule's reasons that holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// The build's settings changed (the file), every break's or those of a
    /// crate the break runs or is built on.
    Settings(String),
    /// A file no crate's Rust names changed, outside the folders read one by
    /// one: what reads it cannot be known (the file, and why).
    Unread(String, String),
    /// The break is new or changed in its list.
    Itself,
    /// The break's `file` was touched.
    File,
    /// A test its `caught-by` names was touched (the entry).
    Test(String),
    /// A test its `caught-by` names could not be placed in any file, and a
    /// file of the crates it runs was touched (the entry).
    Unplaced(String),
    /// A data file a crate it runs, or one they are built on, reads (the
    /// file).
    Data(String),
}

impl Reason {
    /// In a sentence, for `--list`.
    fn said(&self) -> String {
        match self {
            Self::Settings(file) => {
                format!("`{file}` changed, and the build of the crates it runs reads it")
            }
            Self::Unread(file, why) => format!("`{file}` changed: {why}"),
            Self::Itself => "the break is new or changed in its list".to_owned(),
            Self::File => "its file was touched".to_owned(),
            Self::Test(entry) => format!("a test it names was touched: `{entry}`"),
            Self::Unplaced(entry) => format!(
                "`{entry}` could not be placed in any file, and a file of the crates it runs was \
                 touched"
            ),
            Self::Data(file) => {
                format!("`{file}` changed, and a crate it runs, or one they are built on, reads it")
            }
        }
    }
}

/// What the rule chose.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    /// The breaks chosen, in the order given, each with why.
    pub(crate) chosen: Vec<(Break, Reason)>,
    /// How relevant each chosen break is, for `--budget`: by its id.
    pub(crate) classes: BTreeMap<String, Class>,
    /// How many breaks were given.
    pub(crate) given: usize,
    /// How many files the diff touches.
    pub(crate) files: usize,
    /// What was found of the data files the diff touches: who reads each.
    pub(crate) notes: Vec<String>,
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
    /// left and why; then what was found of each data file.
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
                    count(|r| matches!(r, Reason::Unread(..))),
                    "by a file whose reader cannot be known, chosen to be safe",
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
                (
                    count(|r| matches!(r, Reason::Data(_))),
                    "by a data file a crate they run reads",
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
             their text, nor the build's settings, nor a data file a crate they run reads. A \
             full run (no `--changed`) is what a report rests on."
        ));
        for note in &self.notes {
            said.push_str(&format!("\n  {note}"));
        }
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
    let settings = touched
        .iter()
        .find(|file| SETTINGS.contains(&file.as_str()));
    let data = Data::read(&touched, sources);
    let every = data.every.clone();
    let before = definitions_before(known, &touched, sources);
    let mut crates = Crates::new(&touched, sources);
    let mut classes = BTreeMap::new();
    let chosen = given
        .iter()
        .filter_map(|one| {
            let itself = !before.contains(&one.definition());
            let file = touched.contains(&one.file);
            let near = crates.touched_by(one);
            let seen = data.seen_by(one);
            let reason = if let Some(file) = settings {
                Some(Reason::Settings(file.clone()))
            } else if let Some((file, why)) = &every {
                Some(Reason::Unread(file.clone(), why.clone()))
            } else if itself {
                Some(Reason::Itself)
            } else if file {
                Some(Reason::File)
            } else {
                near.clone().or_else(|| seen.clone())
            };
            let reason = reason?;
            let runs_a_crate = one.test_packages().is_ok();
            let class = match (&near, &seen) {
                _ if itself || file => Class::Touched,
                (Some(Reason::Test(_)), _) => Class::Test,
                (Some(_), _) | (None, Some(_)) if runs_a_crate => Class::Crate,
                _ => Class::Rest,
            };
            classes.insert(one.id.clone(), class);
            Some((one.clone(), reason))
        })
        .collect();
    Selection {
        chosen,
        classes,
        given: given.len(),
        files: touched.len(),
        notes: data.notes,
    }
}

/// What the files of the diff that are not Rust, and the crates' manifests,
/// come to: the crates that can see each.
#[derive(Debug, Default)]
struct Data {
    /// A file that chooses every break: no crate names it and it is outside
    /// the folders read one by one, or the workspace's crates could not be
    /// read. The file, and why.
    every: Option<(String, String)>,
    /// For each file a crate reads, or crate manifest, the crates that can
    /// see it: those that read it (or whose manifest it is), and every
    /// crate built on them or whose tests use them. With the reason a break
    /// of one of them is given.
    seen: Vec<(BTreeSet<String>, Reason)>,
    /// What was found, to be said.
    notes: Vec<String>,
}

/// Whether `path` is Rust that a crate holds as data and does not build: a
/// `.rs` file under a crate's folder (`crates/<name>/` or `xtask/`) but in
/// none of the folders cargo builds from (`src/`, `tests/`, `benches/`,
/// `examples/`) and not its `build.rs`, such as `cineris-index`'s
/// `fixtures/outlines/sample.rs` or `xtask`'s `fixtures/rules/*.rs`. Such a
/// file is read like any data file: its readers are found by the paths that
/// name it.
pub(crate) fn rust_as_data(path: &str) -> bool {
    let within = if let Some(rest) = path.strip_prefix("crates/") {
        rest.split_once('/').map(|(_, within)| within)
    } else {
        path.strip_prefix("xtask/")
    };
    let Some(within) = within else {
        return false;
    };
    let first = within.split('/').next().unwrap_or(within);
    within.contains('/')
        && path.ends_with(".rs")
        && !["src", "tests", "benches", "examples"].contains(&first)
}

impl Data {
    fn read(touched: &BTreeSet<String>, sources: &dyn Sources) -> Self {
        let mut data = Data::default();
        let mut wanted = Vec::new();
        let mut manifests = Vec::new();
        for path in touched {
            if SETTINGS.contains(&path.as_str()) {
                continue;
            }
            let name = path.rsplit('/').next().unwrap_or(path);
            if path.ends_with(".rs") && name != "build.rs" && !rust_as_data(path) {
                // Rust outside the crates is in no break's build that the
                // rule can tell of.
                if outside_known(path) && data.every.is_none() {
                    data.every = Some((
                        path.clone(),
                        "it is Rust in no crate the rule reads, so every break is chosen"
                            .to_owned(),
                    ));
                }
                continue;
            }
            if name == "Cargo.toml" || name == "build.rs" {
                manifests.push(path.clone());
            } else {
                wanted.push(path.clone());
            }
        }
        if wanted.is_empty() && manifests.is_empty() {
            return data;
        }
        let packages = match sources.packages() {
            Ok(packages) => packages,
            Err(error) => {
                // Without the crates, a file no folder rule covers is taken
                // to touch every break, as it always was.
                if data.every.is_none()
                    && let Some(path) = wanted
                        .iter()
                        .find(|path| outside_known(path) && !markdown(path))
                {
                    data.every = Some((
                        path.clone(),
                        format!(
                            "the workspace's crates could not be read ({error}), so who reads \
                             it cannot be told, and every break is chosen"
                        ),
                    ));
                }
                return data;
            }
        };
        let folders = readers::folders(&packages);
        let crate_of = |path: &str| {
            folders
                .iter()
                .filter(|(_, folder)| {
                    path.strip_prefix(folder.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
                })
                .max_by_key(|(_, folder)| folder.len())
                .map(|(name, folder)| (name.clone(), folder.clone()))
        };
        let seen_from = |readers: &BTreeSet<String>| -> BTreeSet<String> {
            let names: Vec<String> = readers.iter().cloned().collect();
            crate::dependents::dependents(&packages, &names)
                .map(|all| all.into_iter().collect())
                .unwrap_or_else(|_| readers.clone())
        };
        for manifest in &manifests {
            if let Some((name, folder)) = crate_of(manifest)
                && folder == readers::normalise(&format!("{manifest}/..")).unwrap_or_default()
            {
                let seen = seen_from(&BTreeSet::from([name]));
                data.seen.push((seen, Reason::Settings(manifest.clone())));
            } else {
                wanted.push(manifest.clone());
            }
        }
        if wanted.is_empty() {
            return data;
        }
        // Every Rust file of the crates, as it is, and as it was where the
        // diff touches it: a reader the diff took away still saw the file.
        let mut texts: Vec<(String, String, String, String)> = Vec::new();
        for (name, folder) in &folders {
            let mut files = sources.rust_files(folder);
            for path in touched {
                if path.ends_with(".rs") && !files.contains(path) {
                    files.push(path.clone());
                }
            }
            for path in files {
                // A file of a crate inside this one's folder is that crate's.
                if crate_of(&path).is_none_or(|(owner, _)| &owner != name) {
                    continue;
                }
                if let Some(text) = sources.now(&path) {
                    texts.push((name.clone(), folder.clone(), path.clone(), text));
                }
                if touched.contains(&path)
                    && let Some(text) = sources.old(&path)
                {
                    texts.push((name.clone(), folder.clone(), path.clone(), text));
                }
            }
        }
        let found = readers::readers(
            &wanted,
            texts.iter().map(|(name, folder, path, text)| {
                (name.as_str(), folder.as_str(), path.as_str(), text.as_str())
            }),
        );
        for (path, readers) in found {
            if readers.is_empty() {
                if outside_known(&path) && !markdown(&path) && data.every.is_none() {
                    data.every = Some((
                        path.clone(),
                        "no string of any crate's Rust names it, so what reads it cannot be \
                         known, and every break is chosen"
                            .to_owned(),
                    ));
                }
                continue;
            }
            let mut seen = if readers.built.is_empty() {
                BTreeSet::new()
            } else {
                seen_from(&readers.built)
            };
            seen.extend(readers.tests.iter().cloned());
            let list = |names: &BTreeSet<String>| {
                names
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let mut by = Vec::new();
            if !readers.built.is_empty() {
                by.push(format!(
                    "the code of {} (and so every crate built on it)",
                    list(&readers.built)
                ));
            }
            if !readers.tests.is_empty() {
                by.push(format!("the tests alone of {}", list(&readers.tests)));
            }
            data.notes.push(format!(
                "`{path}` is named by {}: the breaks that run one of these {} crate(s) can see \
                 it: {}",
                by.join(", and "),
                seen.len(),
                list(&seen)
            ));
            data.seen.push((seen, Reason::Data(path)));
        }
        data
    }

    /// Why `one` can see a file of the diff that is not Rust, or a crate's
    /// manifest, if it can: a crate it runs is among those that can.
    fn seen_by(&self, one: &Break) -> Option<Reason> {
        let packages = one.test_packages().ok()?;
        self.seen
            .iter()
            .find(|(crates, _)| packages.iter().any(|name| crates.contains(name)))
            .map(|(_, reason)| reason.clone())
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
        // A file of the crates that is not Rust (a fixture, a snapshot, what
        // a test reads), or is Rust the crate holds as data and does not build
        // (a fixture of Rust), can change what a test sees: chosen to be safe,
        // whether or not a literal names it.
        if let Some(file) = touched
            .iter()
            .find(|path| in_directories(path) && (!path.ends_with(".rs") || rust_as_data(path)))
        {
            return Some(Reason::Unplaced(file.clone()));
        }
        for entry in &one.caught_by {
            match placed(entry) {
                Placed::Nowhere => return Some(Reason::Unplaced(entry.clone())),
                Placed::Word(word) => {
                    let mut found = false;
                    for directory in &directories {
                        let words = self.words_of(directory);
                        let files: Vec<&String> = words.get(&word).into_iter().flatten().collect();
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
                for word in lex::names_in(text) {
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
                if let Some(text) = sources.now(path) {
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
    /// A word of the test's name: its last segment.
    Word(String),
    /// Nothing to look for: the entry ends in `*`, a pattern that can end
    /// inside a module's name.
    Nowhere,
}

/// The word to look for of a `caught-by` entry: the last segment of the
/// test's name, after the binary if the entry names one
/// (`cineris-llm::it openai::endpoints::*`).
fn placed(entry: &str) -> Placed {
    let name = entry
        .split_once(char::is_whitespace)
        .map_or(entry, |(_, name)| name.trim());
    // A pattern can end inside a module's name (`open*` is every test of
    // `openai_chat`), whose tests no word of their own names: placed
    // nowhere, so any file of the crates chooses the break.
    if name.ends_with('*') {
        return Placed::Nowhere;
    }
    let last = name.rsplit("::").next().unwrap_or(name);
    if last.is_empty() {
        Placed::Nowhere
    } else {
        Placed::Word(last.to_owned())
    }
}

/// The breaks of `chosen` the working tree's diff from `reference` can
/// affect, in the repository at `root`: what `--changed` does. `known` is
/// every break of `mutations/`.
pub(crate) fn since(
    root: &Path,
    reference: &str,
    known: &[Break],
    chosen: &[Break],
) -> Result<Selection> {
    let git = Git::open(root, reference)?;
    let files = git.changed_files()?;
    Ok(select(known, chosen, &files, &git))
}

/// The repository at `root`, and the commit a diff is from.
pub(crate) struct Git<'a> {
    root: &'a Path,
    commit: String,
    rust: std::cell::OnceCell<Vec<String>>,
}

impl<'a> Git<'a> {
    /// Opens the repository at `root` for a diff from `reference`. Refuses a
    /// tree with no `.git` of its own (a copy made without it has none), and a
    /// reference that is no commit.
    pub(crate) fn open(root: &'a Path, reference: &str) -> Result<Self> {
        if !root.join(".git").exists() {
            return Err(format!(
                "`--changed {reference}` needs git's history, and {} has no `.git`: run it in a \
                 git checkout (a copy of the tree made without its `.git` cannot say what \
                 changed).",
                root.display()
            ));
        }
        if reference.is_empty() {
            return Err(
                "`--changed` needs a branch, a tag or a commit, not an empty text".to_owned(),
            );
        }
        if reference.starts_with('-') {
            return Err(format!(
                "`--changed {reference}`: a branch, a tag or a commit, not an option"
            ));
        }
        // The `.git` must be this tree's own: a copy whose `.git` is a
        // worktree's pointer to a folder that is not there, or a tree inside
        // another repository, would give another tree's diff, or none.
        let top = crate::owner_guard::git(root, &["rev-parse", "--show-toplevel"]).map_err(|why| {
            format!(
                "`--changed {reference}` needs git's history, and git cannot read the `.git` of \
                 {} ({why}). A copy of a working tree carries its `.git` as a pointer to the \
                 folder it came from, which is not in the copy: run it in the git checkout \
                 itself.",
                root.display()
            )
        })?;
        // Both read by the file system, which spells them alike: git gives
        // `C:/…` on Windows.
        let real = |path: &Path| std::fs::canonicalize(path).ok();
        if real(Path::new(top.trim())).is_none() || real(Path::new(top.trim())) != real(root) {
            return Err(format!(
                "`--changed {reference}`: git's work tree is {}, not {}: the diff would be \
                 another tree's.",
                top.trim(),
                root.display()
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

    fn now(&self, path: &str) -> Option<String> {
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

    fn packages(&self) -> Result<Vec<Package>> {
        crate::workspace::metadata(self.root).map(|metadata| metadata.packages)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{Placed, Reason, Sources, placed, select};
    use crate::mutate::budget::Class;
    use crate::mutate::list::{self, Break};
    use crate::workspace::{DepKind, Dependency, Package};

    /// Files as they are, and as they were, in memory.
    #[derive(Default)]
    struct Files {
        now: BTreeMap<String, String>,
        was: BTreeMap<String, String>,
        /// The workspace's crates; none: they cannot be read.
        packages: Option<Vec<Package>>,
    }

    /// A crate in `crates/<name>`, built on `normal` and whose tests use
    /// `dev`.
    fn package(name: &str, normal: &[&str], dev: &[&str]) -> Package {
        let edge = |to: &&str, kind| Dependency {
            name: (*to).to_owned(),
            kind,
            in_workspace: true,
            default_features: true,
            features: Vec::new(),
            optional: false,
        };
        Package {
            name: name.to_owned(),
            manifest: format!("crates/{name}/Cargo.toml"),
            publish_disabled: true,
            dependencies: normal
                .iter()
                .map(|to| edge(to, DepKind::Normal))
                .chain(dev.iter().map(|to| edge(to, DepKind::Dev)))
                .collect(),
            targets: Vec::new(),
            features: Vec::new(),
        }
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

        fn now(&self, path: &str) -> Option<String> {
            self.now.get(path).cloned()
        }

        fn rust_files(&self, directory: &str) -> Vec<String> {
            self.now
                .keys()
                .filter(|path| path.starts_with(&format!("{directory}/")) && path.ends_with(".rs"))
                .cloned()
                .collect()
        }

        fn packages(&self) -> crate::Result<Vec<Package>> {
            self.packages
                .clone()
                .ok_or_else(|| "no workspace in this test".to_owned())
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
        assert_eq!(chosen(&tree(), &["README.md"], &all()), []);
        assert_eq!(chosen(&tree(), &[".github/workflows/ci.yml"], &all()), []);
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
        // A file deleted, nothing of its name left anywhere: it is found by
        // where it was.
        let deleted = Files::default()
            .same("mutations/demo.breaks", &format!("{A}\n{B}"))
            .edited(
                "crates/demo/src/old_tests.rs",
                Some("fn first_test() {}"),
                None,
            )
            .same("crates/demo/src/other.rs", "fn second_test() {}");
        assert_eq!(
            chosen(&deleted, &["crates/demo/src/old_tests.rs"], &all()),
            [(
                "demo/a".to_owned(),
                Reason::Test("tests::first_test".to_owned())
            )]
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
    fn a_name_that_ends_in_a_star_is_placed_nowhere_and_chosen_by_any_file_of_its_crates() {
        assert_eq!(placed("mutate::tests::sys_*"), Placed::Nowhere);
        // A pattern that ends inside a module's name.
        assert_eq!(placed("cineris-llm::it openai*"), Placed::Nowhere);
        assert_eq!(placed("tests::*"), Placed::Nowhere);
        assert_eq!(placed("*"), Placed::Nowhere);
        // The binary of an entry is not part of the name.
        assert_eq!(
            placed("cineris-core::it sys_x"),
            Placed::Word("sys_x".to_owned())
        );
        assert_eq!(
            placed("cineris-llm::it openai_chat::endpoints::an_url"),
            Placed::Word("an_url".to_owned())
        );
        assert_eq!(placed("cineris-config dirs::tests::*"), Placed::Nowhere);
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
                Reason::Unplaced("tests::first_*".to_owned())
            )]
        );
        // The review's case: `open*` names the tests of a module `openai`,
        // whose file is touched and has no word that begins `open`.
        let list = A.replace("tests::first_test", "open*");
        let files = tree()
            .same("crates/demo/src/lib.rs", "mod openai;")
            .same("crates/demo/src/openai.rs", "#[test]\nfn first() {}\n");
        assert_eq!(
            chosen(
                &files,
                &["crates/demo/src/openai.rs"],
                &lists(&[("demo", &list)])
            ),
            [("demo/a".to_owned(), Reason::Unplaced("open*".to_owned()))]
        );
    }

    #[test]
    fn a_file_of_a_breaks_crate_that_is_not_rust_chooses_it() {
        assert_eq!(
            chosen(&tree(), &["crates/demo/tests/data.json"], &all()),
            [
                (
                    "demo/a".to_owned(),
                    Reason::Unplaced("crates/demo/tests/data.json".to_owned())
                ),
                (
                    "demo/b".to_owned(),
                    Reason::Unplaced("crates/demo/tests/data.json".to_owned())
                ),
            ]
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
        for file in [
            "Cargo.lock",
            "Cargo.toml",
            ".config/nextest.toml",
            ".cargo/config.toml",
        ] {
            let got = chosen(&tree(), &[file], &all());
            assert_eq!(got.len(), 3, "{file}");
            assert!(
                got.iter()
                    .all(|(_, reason)| matches!(reason, Reason::Settings(_)))
            );
        }
        // A file outside the folders read one by one, when the crates that
        // could read it cannot be read: every break.
        for file in ["tables/rates.toml", "rustfmt.toml", "fuzz/src/lib.rs"] {
            let got = chosen(&tree(), &[file], &all());
            assert_eq!(got.len(), 3, "{file}");
            assert!(
                got.iter()
                    .all(|(_, reason)| matches!(reason, Reason::Unread(..))),
                "{got:?}"
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

    /// A workspace of four crates: `demo` is built on `base`, `tester`'s
    /// tests use `base`, and `other` stands alone. `base`'s build script
    /// reads `tables/rates.toml`; each crate has a break.
    fn data_tree() -> (Files, Vec<Break>) {
        const BASE: &str = "[break] base\nrule: r\nfile: crates/base/src/lib.rs\nfind: x\n\
                            replace: y\n";
        const TESTER: &str = "[break] tester\nrule: r\nfile: crates/tester/src/lib.rs\nfind: x\n\
                              replace: y\n";
        let text = format!("{A}\n{B}\n{BASE}\n{TESTER}");
        let files = tree()
            .same("mutations/demo.breaks", &text)
            .same(
                "crates/base/build.rs",
                "fn main() { let p = dir.join(\"../../tables/rates.toml\"); }",
            )
            .same("crates/base/src/lib.rs", "pub fn base() {}")
            .same("crates/tester/src/lib.rs", "pub fn tester() {}")
            .same("tables/rates.toml", "rate = 1\n");
        let files = Files {
            packages: Some(vec![
                package("base", &[], &[]),
                package("demo", &["base"], &[]),
                package("tester", &[], &["base"]),
                package("other", &[], &[]),
            ]),
            ..files
        };
        let breaks = lists(&[("demo", &text), ("other", OTHER)]);
        (files, breaks)
    }

    #[test]
    fn a_data_file_chooses_the_breaks_of_the_crates_that_read_it_and_those_built_on_them() {
        let (files, breaks) = data_tree();
        let changed = ["tables/rates.toml".to_owned()];

        let got = select(&breaks, &breaks, &changed, &files);

        let ids: Vec<&str> = got.chosen.iter().map(|(one, _)| one.id.as_str()).collect();
        // `base` reads it, `demo` is built on it, `tester`'s tests use it;
        // `other` cannot see it.
        assert_eq!(ids, ["demo/a", "demo/b", "demo/base", "demo/tester"]);
        assert!(
            got.chosen
                .iter()
                .all(|(_, reason)| *reason == Reason::Data("tables/rates.toml".to_owned()))
        );
        assert!(got.classes.values().all(|class| *class == Class::Crate));
        let said = got.summary("main");
        assert!(
            said.contains("4 by a data file a crate they run reads"),
            "{said}"
        );
        assert!(
            said.contains("`tables/rates.toml` is named by the code of base (and so every crate built on it): the breaks that run one of these 3 crate(s) can see it: base, demo, tester"),
            "{said}"
        );
    }

    #[test]
    fn rust_a_crate_holds_as_data_is_read_as_data() {
        use super::rust_as_data;
        for data in [
            "crates/demo/fixtures/outlines/sample.rs",
            "xtask/fixtures/rules/a.rs",
        ] {
            assert!(rust_as_data(data), "{data}");
        }
        for built in [
            "crates/demo/src/lib.rs",
            "crates/demo/tests/it/main.rs",
            "crates/demo/benches/b.rs",
            "crates/demo/examples/e.rs",
            "crates/demo/build.rs",
            "xtask/src/main.rs",
            "fuzz/fuzz_targets/a.rs",
            "crates/demo/fixtures/a.txt",
        ] {
            assert!(!rust_as_data(built), "{built}");
        }
        // Read by another crate's code, a Rust fixture chooses that crate's
        // breaks and those built on it, as any data file does.
        let (files, breaks) = data_tree();
        let files = files
            .same(
                "crates/base/src/lib.rs",
                "pub const OUTLINE: &str = include_str!(\"../../other/fixtures/outline.rs\");",
            )
            .same("crates/other/fixtures/outline.rs", "fn shown() {}\n");
        let changed = ["crates/other/fixtures/outline.rs".to_owned()];
        let got = select(&breaks, &breaks, &changed, &files);
        let by_data: Vec<&str> = got
            .chosen
            .iter()
            .filter(|(_, reason)| matches!(reason, Reason::Data(_)))
            .map(|(one, _)| one.id.as_str())
            .collect();
        assert_eq!(by_data, ["demo/a", "demo/b", "demo/base", "demo/tester"]);
    }

    #[test]
    fn a_crates_own_rust_fixture_chooses_its_breaks_though_no_literal_names_it() {
        let (files, breaks) = data_tree();
        // `crates/base/fixtures/sample.rs`: named by no literal anywhere.
        let files = files.same("crates/base/fixtures/sample.rs", "fn shown() {}\n");
        let changed = ["crates/base/fixtures/sample.rs".to_owned()];
        let got = select(&breaks, &breaks, &changed, &files);
        let ids: Vec<&str> = got.chosen.iter().map(|(one, _)| one.id.as_str()).collect();
        assert_eq!(
            ids,
            ["demo/base"],
            "base's own break, as for any file of it"
        );
        assert!(matches!(
            &got.chosen[0].1,
            Reason::Unplaced(file) if file == "crates/base/fixtures/sample.rs"
        ));
    }

    #[test]
    fn a_file_no_crate_names_chooses_every_break_outside_the_folders_read_one_by_one() {
        let (files, breaks) = data_tree();
        // Outside them, and not Markdown: every break, with why.
        let got = select(&breaks, &breaks, &["assets/logo.png".to_owned()], &files);
        assert_eq!(got.chosen.len(), breaks.len());
        assert!(got.chosen.iter().all(|(_, reason)| matches!(
            reason,
            Reason::Unread(file, why) if file == "assets/logo.png" && why.contains("no string")
        )));
        assert!(got.classes.values().all(|class| *class == Class::Rest));
        // Markdown, or a file of the folders read one by one: nothing by
        // itself.
        for file in ["NOTES.md", "docs/table.csv", ".github/data.json"] {
            assert_eq!(chosen(&files, &[file], &breaks), [], "{file}");
        }
        // Unless a crate names it: then that crate's breaks.
        let files = files.same(
            "crates/other/src/c.rs",
            "pub fn c() { read(\"docs/tables\"); }",
        );
        assert_eq!(
            chosen(&files, &["docs/tables/one.csv"], &breaks),
            [(
                "other/c".to_owned(),
                Reason::Data("docs/tables/one.csv".to_owned())
            )]
        );
    }

    #[test]
    fn a_reader_the_diff_took_away_still_counts() {
        let (files, breaks) = data_tree();
        // Only `tester` names the file, and the diff takes that away.
        let files = files.same("crates/base/build.rs", "fn main() {}").edited(
            "crates/tester/src/data.rs",
            Some("const T: &str = include_str!(\"../../../tables/rates.toml\");"),
            Some("const T: &str = \"\";"),
        );
        assert_eq!(
            chosen(
                &files,
                &["tables/rates.toml", "crates/tester/src/data.rs"],
                &breaks
            ),
            [(
                "demo/tester".to_owned(),
                Reason::Data("tables/rates.toml".to_owned())
            )]
        );
        // Not taken away by the diff, and named by no crate now: unread.
        let files = files.same("crates/tester/src/data.rs", "const T: &str = \"\";");
        let got = chosen(&files, &["tables/rates.toml"], &breaks);
        assert_eq!(got.len(), breaks.len());
        assert!(
            got.iter()
                .all(|(_, reason)| matches!(reason, Reason::Unread(..))),
            "{got:?}"
        );
    }

    #[test]
    fn a_crates_manifest_chooses_the_breaks_of_the_crates_built_on_it() {
        let (files, breaks) = data_tree();
        let got = chosen(&files, &["crates/base/Cargo.toml"], &breaks);
        assert_eq!(
            got.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            ["demo/a", "demo/b", "demo/base", "demo/tester"]
        );
        assert!(
            got.iter()
                .all(|(_, reason)| *reason == Reason::Settings("crates/base/Cargo.toml".to_owned()))
        );
    }

    #[test]
    fn each_chosen_break_is_classed_by_how_near_the_diff_came_to_it() {
        let (files, breaks) = data_tree();
        // `demo/a`'s file; `demo/b`'s test (in a file of its own); the data
        // file for the rest.
        let files = files
            .same("crates/demo/src/tests.rs", "fn first_test() {}\n")
            .same("crates/demo/src/more.rs", "fn second_test() {}\n");
        let changed: Vec<String> = [
            "crates/demo/src/a.rs",
            "crates/demo/src/more.rs",
            "tables/rates.toml",
        ]
        .map(str::to_owned)
        .to_vec();

        let got = select(&breaks, &breaks, &changed, &files);

        let classes: Vec<(&str, Class)> = got
            .classes
            .iter()
            .map(|(id, class)| (id.as_str(), *class))
            .collect();
        assert_eq!(
            classes,
            [
                ("demo/a", Class::Touched),
                ("demo/b", Class::Test),
                ("demo/base", Class::Crate),
                ("demo/tester", Class::Crate),
            ]
        );
        // The build's settings: every break, of the last class unless the
        // diff came nearer.
        let changed = ["Cargo.lock".to_owned(), "crates/demo/src/a.rs".to_owned()];
        let got = select(&breaks, &breaks, &changed, &files);
        assert_eq!(got.classes["demo/a"], Class::Touched);
        assert_eq!(got.classes["other/c"], Class::Rest);
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
        assert_eq!(
            opened.old("crates/demo/src/a.rs").unwrap(),
            "pub fn a() {}\n"
        );
        assert_eq!(opened.old("crates/demo/src/new.rs"), None);
        assert_eq!(opened.now("crates/demo/src/b.rs"), None);
        assert!(
            opened
                .rust_files("crates/demo")
                .contains(&"crates/demo/src/new.rs".to_owned())
        );
        let known = lists(&[("demo", &format!("{A}\n{B}"))]);
        let got = super::since(&root, "HEAD", &known, &known).unwrap();
        assert_eq!(got, select(&known, &known, &files, &opened));
        assert_eq!(
            got.chosen
                .iter()
                .map(|(one, reason)| (one.id.as_str(), reason.clone()))
                .collect::<Vec<_>>(),
            [("demo/a", Reason::File), ("demo/b", Reason::File)]
        );

        // A reference that is no commit is an error, never "nothing changed".
        for (bad, said) in [
            ("no-such-branch", "is not a commit of this repository"),
            ("-x", "not an option"),
            ("", "not an empty text"),
        ] {
            let error = super::Git::open(&root, bad).err().unwrap();
            assert!(error.contains("--changed"), "{bad}: {error}");
            assert!(error.contains(said), "{bad}: {error}");
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
    fn sys_a_git_that_points_nowhere_is_refused_and_not_read_as_no_change() {
        let root = crate::mutate::drive::tests::tree("changed-dangling-git", &[("a.txt", "x")]);
        // A worktree's `.git` is a file that names its folder in the main
        // repository: in a copy, that folder is not there.
        std::fs::write(root.join(".git"), "gitdir: /nowhere/.git/worktrees/gone\n").unwrap();

        let error = super::Git::open(&root, "main").err().unwrap();

        assert!(error.contains("git cannot read the `.git`"), "{error}");
        assert!(error.contains("in the git checkout itself"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sys_a_tree_inside_another_repository_is_refused_and_not_read_as_its_diff() {
        let outer = crate::mutate::drive::tests::tree("changed-outer", &[("b.txt", "y")]);
        git(&outer, &["init", "-q"]);
        // A copy with an empty `.git` of its own, inside the outer
        // repository: git reads the outer one.
        let root = outer.join("copy");
        std::fs::create_dir_all(root.join(".g".to_owned() + "it")).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();

        let error = super::Git::open(&root, "HEAD").err().unwrap();

        assert!(
            error.contains("git's work tree is") && error.contains("another tree's"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(&outer);
    }

    #[test]
    fn a_refused_diff_has_an_exit_status_of_its_own() {
        let (status, said) = super::super::conclusion(
            &Err(format!("{}no `.git` here", super::super::NO_DIFF_MARK)),
            false,
        );
        assert_eq!(status, 3);
        let said = said.unwrap();
        assert!(
            said.contains("nothing was run") && said.contains("no `.git` here"),
            "{said}"
        );
        // Any other failure is 1, as before.
        assert_eq!(
            super::super::conclusion(&Err("boom".to_owned()), false).0,
            1
        );
    }

    #[test]
    fn a_tree_with_no_git_is_told_how_to_have_one() {
        let root = crate::mutate::drive::tests::tree("changed-no-git", &[("a.txt", "x")]);

        let error = super::Git::open(&root, "origin/main").err().unwrap();

        assert!(error.contains("has no `.git`"), "{error}");
        assert!(error.contains("run it in a git checkout"), "{error}");
        assert!(error.contains("`--changed origin/main`"), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
