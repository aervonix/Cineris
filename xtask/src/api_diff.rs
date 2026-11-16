//! `cargo xtask api-diff`: the public items added, removed and changed
//! between a ref and the working tree (`docs/testing-and-ci.md`, "The public
//! API between two trees").
//!
//! A fix session is asked to list the public signatures it changed, for the
//! step after it, and wrote the list by hand. This writes it.
//!
//! **How the API is read: rustdoc's JSON output**, which is exact: it has
//! the re-exports, the macro-made and generated items, the `cfg`s, and every
//! signature as the compiler understood it, where reading the `pub` items
//! with a lexer misses all of those. It needs `-Z unstable-options`, which
//! the pinned stable toolchain accepts with `RUSTC_BOOTSTRAP=1` (no second
//! toolchain to install, and the format is the pinned toolchain's own; a
//! file of a format this code was not written for is refused, see
//! [`read::SUPPORTED_FORMATS`]).
//!
//! **Nothing of the session's own is touched.** The other tree is a clone
//! under the build directory (`target/api-diff/base`), made with
//! `git clone --shared` as `combine` makes its own: it reads the repository
//! and writes nothing to it, no branch, ref or worktree entry. Each tree has
//! a build directory of its own (`target/api-diff/head-target` and
//! `base-target`), kept between runs, so that the next run documents only
//! what changed. The working tree is documented where it is, uncommitted
//! changes and new files included.

mod diff;
mod read;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde_json::Value;

use self::diff::CrateDeltas;
use crate::Result;
use crate::combine::{git, remove_tree};
use crate::workspace::{self, Package};

const USAGE: &str = "\
usage: cargo xtask api-diff [--against REF] [-p CRATE]... [--features CRATE/FEATURE,...]
                            [--format text|markdown]

The public items added, removed and changed between REF (`main` when not
given) and the working tree, by crate, each with its path and its signature
before and after. -p names a crate, and may be given more than once; without
it, every crate of the workspace that has a library.

A signature changed is its generics, bounds, arguments, return type, the
fields of a public struct, the variants of an enum, a trait's items and the
`impl`s of traits for the crate's types; not its docs, not its body, not the
names of its arguments.

The API is read from rustdoc's JSON output, which the pinned toolchain writes
with RUSTC_BOOTSTRAP=1, in two build directories under target/api-diff/ kept
between runs. REF is checked out in a clone there; the repository, its refs and
this worktree are not touched.

  --format markdown   One table for each crate, for a report.
  --features LIST     Features to turn on when documenting, `CRATE/FEATURE`
                      separated by commas (a bare FEATURE when one crate is
                      named): the items behind them, such as cortex-tui's
                      `test-kit` or cortex-policy's `test-support`, are then
                      in the API compared. A feature the ref does not have
                      is left off there, and said.";

/// How the report is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Text,
    Markdown,
}

/// What `cargo xtask api-diff` was asked for.
#[derive(Debug, PartialEq, Eq)]
struct Request {
    against: Option<String>,
    packages: Vec<String>,
    format: Format,
    /// `--features`: each feature with its crate.
    features: Vec<(String, String)>,
}

impl Request {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Request {
            against: None,
            packages: Vec::new(),
            format: Format::Text,
            features: Vec::new(),
        };
        let mut bare = Vec::new();
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = |name: &str| -> Result<String> {
                let value = words
                    .next()
                    .ok_or_else(|| format!("`{name}` needs a value\n\n{USAGE}"))?;
                if value.starts_with('-') {
                    return Err(format!("`{name} {value}`: not a value\n\n{USAGE}"));
                }
                Ok(value.clone())
            };
            match word.as_str() {
                "--against" => request.against = Some(value("--against")?),
                "--features" => bare.push(value("--features")?),
                "-p" | "--package" => request.packages.push(value(word)?),
                "--format" => {
                    request.format = match value("--format")?.as_str() {
                        "text" => Format::Text,
                        "markdown" | "md" => Format::Markdown,
                        other => {
                            return Err(format!(
                                "`--format {other}`: it is `text` or `markdown`\n\n{USAGE}"
                            ));
                        }
                    };
                }
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other => {
                    if let Some(package) = other.strip_prefix("--against=") {
                        request.against = Some(package.to_owned());
                    } else if let Some(package) = other.strip_prefix("--package=") {
                        request.packages.push(package.to_owned());
                    } else if let Some(list) = other.strip_prefix("--features=") {
                        bare.push(list.to_owned());
                    } else {
                        return Err(format!("unknown argument `{other}`\n\n{USAGE}"));
                    }
                }
            }
        }
        let mut packages = request.packages.clone();
        packages.sort();
        packages.dedup();
        for list in &bare {
            if list.split([',', ' ']).all(str::is_empty) {
                return Err(format!("`--features` names no feature\n\n{USAGE}"));
            }
            for feature in list.split([',', ' ']).filter(|word| !word.is_empty()) {
                let named = match feature.split_once('/') {
                    Some((package, feature)) if !package.is_empty() && !feature.is_empty() => {
                        (package.to_owned(), feature.to_owned())
                    }
                    Some(_) => {
                        return Err(format!(
                            "`--features {feature}`: a feature is `CRATE/FEATURE`\n\n{USAGE}"
                        ));
                    }
                    None => match packages.as_slice() {
                        [package] => (package.clone(), feature.to_owned()),
                        _ => {
                            return Err(format!(
                                "`--features {feature}`: which crate's? Write \
                                 `CRATE/{feature}`, or name one crate with -p\n\n{USAGE}"
                            ));
                        }
                    },
                };
                if !request.features.contains(&named) {
                    request.features.push(named);
                }
            }
        }
        Ok(request)
    }
}

/// Refuses a feature asked for of a crate not compared, or one the working
/// tree's crate does not have: before anything is built.
fn check_features(
    features: &[(String, String)],
    chosen: &[String],
    head: &BTreeMap<String, Library>,
) -> Result<()> {
    for (package, feature) in features {
        if !chosen.contains(package) {
            return Err(format!(
                "`--features {package}/{feature}`: `{package}` is not among the crates compared"
            ));
        }
        let known = head
            .get(package)
            .is_some_and(|library| library.features.iter().any(|known| known == feature));
        if !known {
            return Err(format!(
                "`--features {package}/{feature}`: the working tree's `{package}` has no such \
                 feature"
            ));
        }
    }
    Ok(())
}

/// One call of `cargo doc`: its crates, and its `--features` arguments.
type DocCall = (Vec<String>, Vec<String>);

/// The calls of `cargo doc` that document `libraries` with the features
/// asked for: one for each crate that has a feature asked for, given as
/// its bare names, and one for the rest. A crate with a dev-dependency on
/// itself that asks for the feature (as `cortex-tui` and `cortex-policy`
/// have, for their tests) takes `--features CRATE/FEATURE` as the
/// dev-dependency's, and documents its library without it: the tenth
/// tooling step found `cortex-tui`'s kit missing so. The bare names of a
/// call of one crate are its own. Each call
/// is its crates and its `--features` arguments. Also the features asked
/// for of these crates that this tree's crate does not have, to be said:
/// a feature the ref does not have yet is left off there.
fn doc_calls(libraries: &[&Library], features: &[(String, String)]) -> (Vec<DocCall>, Vec<String>) {
    let mut calls = Vec::new();
    let mut rest = Vec::new();
    let mut missing = Vec::new();
    for library in libraries {
        let mut on = Vec::new();
        for (package, feature) in features
            .iter()
            .filter(|(package, _)| *package == library.package)
        {
            if library.features.iter().any(|known| known == feature) {
                on.push(feature.clone());
            } else {
                missing.push(format!("{package}/{feature}"));
            }
        }
        if on.is_empty() {
            rest.push(library.package.clone());
        } else {
            calls.push((
                vec![library.package.clone()],
                vec!["--features".to_owned(), on.join(",")],
            ));
        }
    }
    if !rest.is_empty() {
        calls.insert(0, (rest, Vec::new()));
    }
    (calls, missing)
}

/// A workspace crate with a library, in one tree.
#[derive(Debug, Clone)]
struct Library {
    package: String,
    /// The name of the JSON file rustdoc writes: the library's, with `_`.
    json: String,
    features: Vec<String>,
}

/// The libraries of a tree's packages, by package name.
fn libraries(packages: &[Package]) -> BTreeMap<String, Library> {
    packages
        .iter()
        .filter_map(|package| {
            let target = package.targets.iter().find(|target| {
                target
                    .kinds
                    .iter()
                    .any(|kind| matches!(kind.as_str(), "lib" | "rlib" | "proc-macro"))
            })?;
            Some((
                package.name.clone(),
                Library {
                    package: package.name.clone(),
                    json: target.name.replace('-', "_"),
                    features: package
                        .features
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect(),
                },
            ))
        })
        .collect()
}

/// The crates to compare: those named, or every one that has a library in
/// either tree.
fn select(
    requested: &[String],
    head: &BTreeMap<String, Library>,
    base: &BTreeMap<String, Library>,
    packages: &[Package],
) -> Result<Vec<String>> {
    if requested.is_empty() {
        let mut all: Vec<String> = head.keys().chain(base.keys()).cloned().collect();
        all.sort();
        all.dedup();
        return Ok(all);
    }
    let mut chosen = Vec::new();
    for name in requested {
        if head.contains_key(name) || base.contains_key(name) {
            if !chosen.contains(name) {
                chosen.push(name.clone());
            }
        } else if packages.iter().any(|package| &package.name == name) {
            return Err(format!(
                "`{name}` has no library: there is no API to compare"
            ));
        } else {
            return Err(format!("no crate named `{name}` in the workspace"));
        }
    }
    chosen.sort();
    Ok(chosen)
}

/// The commit `reference` names: `main` falls back to `origin/main`, which
/// is all a checkout of a branch from the remote has.
fn resolve(root: &Path, reference: Option<&str>) -> Result<String> {
    let spec = |name: &str| format!("{name}^{{commit}}");
    let name = reference.unwrap_or("main");
    match git(root, &["rev-parse", "--verify", "--quiet", &spec(name)]) {
        Ok(commit) if !commit.is_empty() => Ok(commit),
        _ if reference.is_none() => git(
            root,
            &["rev-parse", "--verify", "--quiet", &spec("origin/main")],
        )
        .map_err(|_| {
            "no `main` or `origin/main` to compare with: give a ref with --against".to_owned()
        }),
        _ => Err(format!("`{name}` is not a ref of this repository")),
    }
}

/// Puts `commit` in the clone at `base`, making the clone when it is not
/// there (or no longer works).
///
/// A clone that is damaged is not a repository, and git then looks in the
/// folders above it: `base` is under the build directory of the session's own
/// tree, so a checkout there would be a checkout of the session's tree. It is
/// only made where git says the repository is `base`'s own.
fn prepare_base(root: &Path, base: &Path, commit: &str) -> Result<()> {
    let checkout =
        |dir: &Path| git(dir, &["checkout", "--quiet", "--force", "--detach", commit]).map(|_| ());
    let its_own = git(base, &["rev-parse", "--git-dir"]).is_ok_and(|dir| dir == ".git");
    if base.join(".git").exists() && its_own && checkout(base).is_ok() {
        return Ok(());
    }
    remove_tree(base)?;
    let parent = base.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not make {}: {error}", parent.display()))?;
    git(
        parent,
        &[
            "clone",
            "--quiet",
            "--shared",
            "--no-checkout",
            &root.to_string_lossy(),
            &base.to_string_lossy(),
        ],
    )?;
    checkout(base)
}

/// Takes the work for this run, waiting out a moment's hold by a child
/// another thread forked ([`crate::workspace::lock_for_one_run`]); the
/// system's lock goes when the run does.
fn lock(directory: &Path, pause: &mut dyn FnMut(std::time::Duration)) -> Result<File> {
    crate::workspace::lock_for_one_run(directory, pause, || {
        "another `cargo xtask api-diff` is running: wait for it to end".to_owned()
    })
}

/// The flags rustdoc is given: its JSON, which is unstable.
const RUSTDOCFLAGS: &str = "-Z unstable-options --output-format json";

/// Documents `libraries` of the tree at `tree` as JSON, building in
/// `target`. The files are `target/doc/<library>.json`.
fn document(
    tree: &Path,
    target: &Path,
    libraries: &[&Library],
    features: &[(String, String)],
    label: &str,
) -> Result<()> {
    let (commands, missing) = doc_commands(libraries, features);
    for feature in &missing {
        eprintln!("the {label} tree has no feature `{feature}`: documented without it there");
    }
    for (count, arguments) in &commands {
        document_once(tree, target, *count, arguments, label)?;
    }
    Ok(())
}

/// The arguments of each call of `cargo doc` that documents `libraries`
/// with the features asked for ([`doc_calls`]), with how many crates the
/// call documents; and the features asked for that this tree lacks.
fn doc_commands(
    libraries: &[&Library],
    features: &[(String, String)],
) -> (Vec<(usize, Vec<String>)>, Vec<String>) {
    let (calls, missing) = doc_calls(libraries, features);
    let commands = calls
        .iter()
        .map(|(names, asked)| {
            let sandbox = libraries.iter().any(|library| {
                names.contains(&library.package)
                    && library.package == "cortex-sandbox"
                    && library.features.iter().any(|feature| feature == "backends")
            });
            let mut arguments: Vec<String> = [
                "doc",
                "--no-deps",
                "--lib",
                "--locked",
                "--quiet",
                "--color",
                "never",
            ]
            .map(str::to_owned)
            .to_vec();
            for name in names {
                arguments.extend(["-p".to_owned(), name.clone()]);
            }
            if sandbox {
                arguments.extend(workspace::implied_features(names));
            }
            arguments.extend(asked.iter().cloned());
            (names.len(), arguments)
        })
        .collect();
    (commands, missing)
}

/// One call of `cargo doc`, of `count` crates, with `arguments`.
fn document_once(
    tree: &Path,
    target: &Path,
    count: usize,
    arguments: &[String],
    label: &str,
) -> Result<()> {
    let mut command = Command::new(workspace::cargo());
    command.args(arguments);
    command
        .current_dir(tree)
        .env("CARGO_TARGET_DIR", target)
        .env("RUSTC_BOOTSTRAP", "1")
        .env("RUSTDOCFLAGS", RUSTDOCFLAGS)
        .env_remove("CARGO_BUILD_TARGET_DIR")
        .env_remove("CARGO_ENCODED_RUSTDOCFLAGS");
    eprintln!(
        "documenting {count} crate(s) of the {label} tree ({})",
        tree.display()
    );
    let started = Instant::now();
    let output = command
        .output()
        .map_err(|error| format!("could not run cargo: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(40).collect();
        return Err(format!(
            "cargo doc failed in the {label} tree:\n{}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n")
        ));
    }
    eprintln!("  done in {:.1} s", started.elapsed().as_secs_f64());
    Ok(())
}

/// The public items of a library, from the JSON rustdoc wrote under
/// `target`.
fn api_of(target: &Path, library: &Library) -> Result<read::Api> {
    let path = target.join("doc").join(format!("{}.json", library.json));
    let bytes = fs::read(&path).map_err(|error| {
        format!(
            "rustdoc wrote no {} for `{}`: {error}",
            path.display(),
            library.package
        )
    })?;
    let json: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not JSON: {error}", path.display()))?;
    read::read(&json).map_err(|error| format!("{} ({}): {error}", library.package, path.display()))
}

/// Everything `api-diff` does: the report, as text to print.
fn report(root: &Path, request: &Request) -> Result<String> {
    if !root.join(".git").exists() {
        return Err(
            "this copy of the tree has no .git (an rx copy?): run it with `rx --git -- cargo \
             xtask api-diff …`, which gives the copy one"
                .to_owned(),
        );
    }
    let commit = resolve(root, request.against.as_deref())?;
    let head_metadata = workspace::metadata(root)?;
    let work = head_metadata.target_directory.join("api-diff");
    let _lock = lock(&work, &mut std::thread::sleep)?;
    let base = work.join("base");
    prepare_base(root, &base, &commit)?;
    let base_metadata = workspace::metadata(&base)?;

    let head_libraries = libraries(&head_metadata.packages);
    let base_libraries = libraries(&base_metadata.packages);
    let chosen = select(
        &request.packages,
        &head_libraries,
        &base_libraries,
        &head_metadata.packages,
    )?;

    let head_target = work.join("head-target");
    let base_target = work.join("base-target");
    let in_head: Vec<&Library> = chosen
        .iter()
        .filter_map(|name| head_libraries.get(name))
        .collect();
    let in_base: Vec<&Library> = chosen
        .iter()
        .filter_map(|name| base_libraries.get(name))
        .collect();
    check_features(&request.features, &chosen, &head_libraries)?;
    document(root, &head_target, &in_head, &request.features, "working")?;
    document(&base, &base_target, &in_base, &request.features, "base")?;

    let mut crates = Vec::new();
    for name in &chosen {
        let before = match base_libraries.get(name) {
            Some(library) => api_of(&base_target, library)?,
            None => read::Api::new(),
        };
        let after = match head_libraries.get(name) {
            Some(library) => api_of(&head_target, library)?,
            None => read::Api::new(),
        };
        let note = match (
            base_libraries.contains_key(name),
            head_libraries.contains_key(name),
        ) {
            (false, _) => Some("a crate the ref does not have".to_owned()),
            (_, false) => Some("a crate the working tree no longer has".to_owned()),
            _ => None,
        };
        crates.push(CrateDeltas {
            name: name.clone(),
            note,
            deltas: diff::compare(&before, &after),
        });
    }
    let short: String = commit.chars().take(10).collect();
    let name = request.against.as_deref().unwrap_or("main");
    let heading = format!("Public API, `{name}` ({short}) against the working tree");
    Ok(match request.format {
        Format::Text => diff::text(&heading, &crates),
        Format::Markdown => diff::markdown(&heading, &crates),
    })
}

/// `cargo xtask api-diff …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    let text = report(&workspace::root(), &request)?;
    print!("{text}");
    Ok(())
}

#[cfg(test)]
mod tests;
