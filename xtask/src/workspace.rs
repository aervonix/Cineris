//! What the commands need to know about the repository: where it is, which
//! packages it has (from `cargo metadata`), and which files it holds.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::Result;

/// The repository the command works on: the directory that holds the
/// workspace `Cargo.toml`.
///
/// It is the one `--root DIR` named, or else the checkout the current
/// directory is in ([`find_root`]), chosen once by [`choose_root`] before the
/// command runs. It is not where this binary was built ([`built_in`]): with
/// one target folder shared by several checkouts, cargo runs one checkout's
/// build of xtask from all of them, and a binary built in one tree and run in
/// a copy of it would otherwise check, and write to, the first tree. A test
/// that never went through `main` gets the checkout its process runs in,
/// which is the crate's own.
pub(crate) fn root() -> PathBuf {
    ROOT.get_or_init(|| {
        std::env::current_dir()
            .ok()
            .and_then(|here| find_root(&here))
            .unwrap_or_else(built_in)
    })
    .clone()
}

/// The root [`root`] gives, once chosen.
static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// The checkout this binary was built from: the parent of xtask's own
/// manifest folder, as it was when it was compiled. It may be another
/// checkout than [`root`], or one that is gone.
pub(crate) fn built_in() -> PathBuf {
    let xtask = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask.parent().unwrap_or(xtask).to_path_buf()
}

/// Whether `dir` is a checkout of this workspace: a `Cargo.toml` with a
/// `[workspace]` table, beside the `xtask` crate. A plain folder, a crate of
/// the workspace, or another project's workspace (the fuzz targets', a test's
/// fixture) is not.
pub(crate) fn is_checkout(dir: &Path) -> bool {
    dir.join("xtask").join("Cargo.toml").is_file()
        && fs::read_to_string(dir.join("Cargo.toml"))
            .is_ok_and(|text| text.lines().any(|line| line.trim() == "[workspace]"))
}

/// The checkout `here` is in: `here` itself or the nearest folder above it
/// that [`is_checkout`]. This is git's top level in a checkout, and it needs
/// no `.git`: a copy of the tree (`rx`'s, a `mutate` copy) has none.
pub(crate) fn find_root(here: &Path) -> Option<PathBuf> {
    here.ancestors()
        .find(|dir| is_checkout(dir))
        .map(Path::to_path_buf)
}

/// Chooses the root for this run, from `--root DIR` (`given`) or the current
/// directory (`here`), and keeps it for [`root`]. Returns a note to print
/// when it is not the checkout this binary was built from (`built`).
pub(crate) fn choose_root(
    given: Option<&Path>,
    here: &Path,
    built: &Path,
) -> Result<Option<String>> {
    let chosen = pick_root(given, here)?;
    let note = (normalised(&chosen) != normalised(built)).then(|| {
        format!(
            "xtask: working on {}; this xtask was built from the source in {}",
            chosen.display(),
            built.display()
        )
    });
    // Chosen once, before any command asks.
    if ROOT.set(chosen.clone()).is_err() && ROOT.get() != Some(&chosen) {
        return Err("the repository's root was chosen twice".to_owned());
    }
    Ok(note)
}

/// What [`choose_root`] chooses, without keeping it.
pub(crate) fn pick_root(given: Option<&Path>, here: &Path) -> Result<PathBuf> {
    match given {
        Some(dir) => {
            // Absolute, and not canonical: a canonical path on Windows is
            // `\\?\C:\…`, which compares unequal to every plain path.
            let dir = normalised(&here.join(dir));
            if is_checkout(&dir) {
                Ok(dir)
            } else {
                Err(format!(
                    "--root {}: not a checkout of the workspace (no Cargo.toml with [workspace] \
                     beside xtask/)",
                    dir.display()
                ))
            }
        }
        None => find_root(here).ok_or_else(|| {
            format!(
                "{} is not inside a checkout of the workspace (no Cargo.toml with [workspace] \
                 beside xtask/ in it or above it): run xtask from inside one, or name one with \
                 --root DIR",
                here.display()
            )
        }),
    }
}

/// `path` with its `.` and `..` taken out by reading it, not the file
/// system: on Linux `std::path::absolute` keeps them, so `../ab-base` from
/// a checkout would otherwise read as inside it.
pub(crate) fn normalised(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// Takes a leading `--root DIR` (or `--root=DIR`) off the arguments, before
/// the command's name: the option belongs to xtask, not to any command.
pub(crate) fn take_root_option(arguments: &mut Vec<String>) -> Result<Option<PathBuf>> {
    let Some(first) = arguments.first() else {
        return Ok(None);
    };
    if let Some(dir) = first.strip_prefix("--root=") {
        let dir = PathBuf::from(dir);
        arguments.remove(0);
        return Ok(Some(dir));
    }
    if first != "--root" {
        return Ok(None);
    }
    if arguments.len() < 2 {
        return Err("--root needs a directory: cargo xtask --root DIR <command>".to_owned());
    }
    let dir = PathBuf::from(arguments.remove(1));
    arguments.remove(0);
    Ok(Some(dir))
}

/// The features a build of `packages` alone must turn on, or it builds a
/// crate's tests without what they test.
///
/// `cineris-sandbox`'s confinement tests are compiled only with its
/// `backends` feature, on Linux. A build of the whole workspace turns it on
/// (through `cineris-core`); a build of the crate alone does not, and its
/// tests then pass having confined nothing. Two sessions were misled by
/// such a green run, so `test`, `stress` and `ci --tiers` of the crate turn
/// it on themselves, and say so.
pub(crate) fn implied_features(packages: &[String]) -> Vec<String> {
    if cfg!(target_os = "linux") && packages.iter().any(|package| package == "cineris-sandbox") {
        vec!["--features".to_owned(), SANDBOX_BACKENDS.to_owned()]
    } else {
        Vec::new()
    }
}

/// What [`implied_features`] turns on.
pub(crate) const SANDBOX_BACKENDS: &str = "cineris-sandbox/backends";

/// Says that a feature was turned on for the packages named.
pub(crate) fn say_implied(features: &[String]) {
    if !features.is_empty() {
        println!(
            "note: {SANDBOX_BACKENDS} is on: a build of cineris-sandbox alone compiles its \
             confinement tests out without it"
        );
    }
}

/// The packages `-p`, `--package`, `-pNAME` and `--package=NAME` name in
/// arguments for cargo, before a `--`.
pub(crate) fn packages_named(arguments: &[String]) -> Vec<String> {
    let mut packages = Vec::new();
    let mut words = arguments.iter().take_while(|word| *word != "--");
    while let Some(word) = words.next() {
        if word == "-p" || word == "--package" {
            if let Some(package) = words.next() {
                packages.push(package.clone());
            }
        } else if let Some(package) = word.strip_prefix("--package=") {
            packages.push(package.to_owned());
        } else if let Some(package) = word.strip_prefix("-p").filter(|_| !word.starts_with("--")) {
            packages.push(package.to_owned());
        }
    }
    packages
}

/// Where a run of nextest in `profile` leaves its JUnit report, for the
/// workspace at `root`.
///
/// nextest keeps its files in `target/nextest` under the workspace root
/// (its `store.dir` setting), and not in cargo's target directory: a build
/// that is sent elsewhere with `CARGO_TARGET_DIR` still reports here.
pub(crate) fn nextest_report(root: &Path, profile: &str) -> PathBuf {
    root.join("target")
        .join("nextest")
        .join(profile)
        .join("junit.xml")
}

/// Removes the report of an earlier run, so that a run that fails before it
/// writes one is not followed by a reading of the last run's.
pub(crate) fn forget_report(report: &Path) -> Result<()> {
    match fs::remove_file(report) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not remove {}: {error}", report.display())),
    }
}

/// Removes a directory and what is in it; one that is not there is fine.
pub(crate) fn forget_dir(directory: &Path) -> Result<()> {
    match fs::remove_dir_all(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not remove {}: {error}", directory.display())),
    }
}

/// The cargo that is running this command, so every nested call uses the
/// same toolchain.
pub(crate) fn cargo() -> OsString {
    std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into())
}

/// The kind of a dependency edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DepKind {
    Normal,
    Dev,
    Build,
}

/// One dependency declared in a manifest.
#[derive(Clone, Debug)]
pub(crate) struct Dependency {
    /// The package name (not a rename).
    pub(crate) name: String,
    pub(crate) kind: DepKind,
    /// A path dependency inside this repository.
    pub(crate) in_workspace: bool,
    pub(crate) default_features: bool,
    pub(crate) features: Vec<String>,
    /// Only built when a feature of the package asks for it.
    pub(crate) optional: bool,
}

/// One build target of a package.
#[derive(Clone, Debug)]
pub(crate) struct Target {
    pub(crate) name: String,
    /// Cargo's target kinds: `lib`, `bin`, `test`, ...
    pub(crate) kinds: Vec<String>,
    pub(crate) doctest: bool,
}

/// One workspace package, reduced to what the rules read.
#[derive(Clone, Debug)]
pub(crate) struct Package {
    pub(crate) name: String,
    /// The manifest, relative to the repository root, with `/` separators.
    pub(crate) manifest: String,
    /// `publish = false`.
    pub(crate) publish_disabled: bool,
    pub(crate) dependencies: Vec<Dependency>,
    pub(crate) targets: Vec<Target>,
    /// The package's own features, each with what it turns on, in name
    /// order: `("default", ["std"])`.
    pub(crate) features: Vec<(String, Vec<String>)>,
}

/// The workspace as `cargo metadata` describes it.
#[derive(Clone, Debug)]
pub(crate) struct Metadata {
    pub(crate) target_directory: PathBuf,
    pub(crate) packages: Vec<Package>,
}

/// Runs `cargo metadata` for the workspace. It reads manifests only: no
/// dependency resolution, no network.
pub(crate) fn metadata(root: &Path) -> Result<Metadata> {
    let output = Command::new(cargo())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("could not run cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed:\n{}",
            String::from_utf8_lossy(&output.stderr).trim_end()
        ));
    }
    let json: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cargo metadata did not print JSON: {error}"))?;
    parse_metadata(&json)
}

fn parse_metadata(json: &Value) -> Result<Metadata> {
    let workspace_root = PathBuf::from(text(json, "workspace_root")?);
    let target_directory = PathBuf::from(text(json, "target_directory")?);
    let mut packages = Vec::new();
    for package in list(json, "packages")? {
        let manifest_path = PathBuf::from(text(package, "manifest_path")?);
        let manifest = manifest_path
            .strip_prefix(&workspace_root)
            .unwrap_or(&manifest_path)
            .to_string_lossy()
            .replace('\\', "/");

        let mut dependencies = Vec::new();
        for dependency in list(package, "dependencies")? {
            let kind = match dependency.get("kind").and_then(Value::as_str) {
                None => DepKind::Normal,
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                Some(other) => return Err(format!("unknown dependency kind `{other}`")),
            };
            dependencies.push(Dependency {
                name: text(dependency, "name")?.to_owned(),
                kind,
                // Registry dependencies have a source; path dependencies have none.
                in_workspace: dependency.get("source").is_none_or(Value::is_null),
                default_features: dependency
                    .get("uses_default_features")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                features: strings(dependency, "features")?,
                optional: dependency
                    .get("optional")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }

        let mut targets = Vec::new();
        for target in list(package, "targets")? {
            targets.push(Target {
                name: text(target, "name")?.to_owned(),
                kinds: strings(target, "kind")?,
                doctest: target
                    .get("doctest")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            });
        }

        let mut features: Vec<(String, Vec<String>)> = package
            .get("features")
            .and_then(Value::as_object)
            .map(|features| {
                features
                    .iter()
                    .map(|(name, turns_on)| {
                        let turns_on = turns_on.as_array().map_or_else(Vec::new, |list| {
                            list.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        });
                        (name.clone(), turns_on)
                    })
                    .collect()
            })
            .unwrap_or_default();
        features.sort();

        packages.push(Package {
            name: text(package, "name")?.to_owned(),
            manifest,
            // `publish = false` is reported as an empty list of registries.
            publish_disabled: package
                .get("publish")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty),
            dependencies,
            targets,
            features,
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Metadata {
        target_directory,
        packages,
    })
}

fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("cargo metadata: `{key}` is missing or is not a string"))
}

fn list<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("cargo metadata: `{key}` is missing or is not a list"))
}

fn strings(value: &Value, key: &str) -> Result<Vec<String>> {
    Ok(list(value, key)?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect())
}

/// Every file that belongs to the repository, relative to its root, with `/`
/// separators: what git tracks plus new files it does not ignore.
///
/// Outside a git checkout (a copy of the tree made for a remote build, for
/// example) it falls back to walking the directory, skipping `.git` and
/// every `target` directory.
///
/// Git is asked only when `root` is the top of a checkout. A directory
/// inside one that git ignores (a copy of the tree under `target/`) is not a
/// repository of its own: git would answer for the checkout around it, and
/// say that the directory holds no file at all.
pub(crate) fn repository_files(root: &Path) -> Result<Vec<String>> {
    listed(root, true)
}

/// The files git tracks, relative to the root, with `/` separators: what a
/// pull request holds, and what the repo rules scan. A scratch folder in a
/// worktree, with a build of its own full of absolute paths, is not the
/// repository's (it failed the rules with 3,963 findings once). A new file
/// counts once it is added.
///
/// Outside a git checkout it is [`repository_files`]: there is nothing to
/// tell tracked from untracked by.
pub(crate) fn tracked_files(root: &Path) -> Result<Vec<String>> {
    listed(root, false)
}

fn listed(root: &Path, untracked: bool) -> Result<Vec<String>> {
    let checkout = root.join(".git").exists();
    let mut files = match checkout.then(|| git_files(root, untracked)).flatten() {
        Some(files) => files,
        None => {
            let mut files = Vec::new();
            walk(root, Path::new(""), &mut files)?;
            files
        }
    };
    // A tracked file that was deleted and not yet committed is still listed.
    files.retain(|file| root.join(file).is_file());
    files.sort();
    Ok(files)
}

fn git_files(root: &Path, untracked: bool) -> Option<Vec<String>> {
    let mut arguments = vec!["ls-files", "-z", "--cached"];
    if untracked {
        arguments.extend(["--others", "--exclude-standard"]);
    }
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8(output.stdout).ok()?;
    Some(
        listing
            .split('\0')
            .filter(|file| !file.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

fn walk(root: &Path, relative: &Path, files: &mut Vec<String>) -> Result<()> {
    let directory = root.join(relative);
    let entries = fs::read_dir(&directory)
        .map_err(|error| format!("could not list {}: {error}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("could not list {}: {error}", directory.display()))?;
        let name = entry.file_name();
        let path = relative.join(&name);
        let file_type = entry
            .file_type()
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        if file_type.is_dir() {
            // Any build directory, not only the top one: a scratch folder's
            // build holds absolute paths, and is no file of the repository's.
            if name == "target" || (relative.as_os_str().is_empty() && name == ".git") {
                continue;
            }
            walk(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(path.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

/// How many times a lock that is held is tried again, and how long apart,
/// before another run is said to hold it: 20 times, 10 ms apart, 200 ms in
/// all.
const LOCK_TRIES: u32 = 20;
pub(crate) const LOCK_EVERY: std::time::Duration = std::time::Duration::from_millis(10);

/// Takes `directory` for one run of a command (`mutate`'s copy,
/// `scratch-test`'s, `api-diff`'s work), for as long as the file returned
/// is held, through a file `lock` in it. The lock is the operating
/// system's, so a run that is killed lets go of it. `busy` says what to do
/// when another run holds it.
///
/// **A lock that is held is tried again for a short while** (`pause` waits
/// between the tries). Where one process has threads that start processes
/// (`cargo test`, which runs `xtask`'s tests as threads of one), a process
/// forked by another thread holds the descriptor of the lock a moment, from
/// its fork to its `exec`, after the run that took the lock has let go of
/// it: the next run found it held. Descriptors are not inherited by what
/// runs (the standard library opens every file close-on-exec, and on
/// Windows a handle is not inheritable), so the moment is that of the fork
/// alone, and a few tens of milliseconds are more than enough. Another run
/// that really holds it is still told so, 200 ms later.
pub(crate) fn lock_for_one_run(
    directory: &Path,
    pause: &mut dyn FnMut(std::time::Duration),
    busy: impl FnOnce() -> String,
) -> Result<std::fs::File> {
    use std::fs::{OpenOptions, TryLockError};
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let path = directory.join("lock");
    let file = OpenOptions::new()
        // For reading and writing: Windows locks no file that is open only
        // for appending.
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    let mut tries = 0;
    let taken = loop {
        match file.try_lock() {
            Err(TryLockError::WouldBlock) if tries < LOCK_TRIES => {
                tries += 1;
                pause(LOCK_EVERY);
            }
            taken => break taken,
        }
    };
    match taken {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(busy()),
        // Not taken, and not because another run has it. Going on would be
        // going on without knowing whether another run is there.
        Err(TryLockError::Error(error)) => Err(format!(
            "could not lock {}: {error}. One run at a time may use it, and without the lock \
             there is no telling whether another is using it.",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DepKind, parse_metadata};

    #[test]
    fn metadata_is_reduced_to_what_the_rules_read() {
        let json = json!({
            "workspace_root": "/repo",
            "target_directory": "/repo/target",
            "packages": [{
                "name": "cineris-agent",
                "manifest_path": "/repo/crates/cineris-agent/Cargo.toml",
                "publish": [],
                "dependencies": [
                    {
                        "name": "cineris-llm", "source": null, "kind": null,
                        "uses_default_features": false, "features": [],
                        "path": "/repo/crates/cineris-llm"
                    },
                    {
                        "name": "serde_json",
                        "source": "registry+https://github.com/rust-lang/crates.io-index",
                        "kind": "dev", "uses_default_features": true, "features": ["std"]
                    }
                ],
                "targets": [
                    { "name": "cineris_agent", "kind": ["lib"], "doctest": false },
                    { "name": "it", "kind": ["test"], "doctest": false }
                ],
                "features": { "test-hooks": [], "default": ["std"] }
            }]
        });

        let metadata = parse_metadata(&json).unwrap();
        let package = &metadata.packages[0];

        assert_eq!(package.name, "cineris-agent");
        assert_eq!(package.manifest, "crates/cineris-agent/Cargo.toml");
        assert!(package.publish_disabled);
        assert!(package.dependencies[0].in_workspace);
        assert!(!package.dependencies[0].default_features);
        assert_eq!(package.dependencies[0].kind, DepKind::Normal);
        assert!(!package.dependencies[1].in_workspace);
        assert_eq!(package.dependencies[1].kind, DepKind::Dev);
        assert_eq!(package.dependencies[1].features, ["std"]);
        assert_eq!(package.targets[1].kinds, ["test"]);
        assert_eq!(
            package.features,
            [
                ("default".to_owned(), vec!["std".to_owned()]),
                ("test-hooks".to_owned(), vec![])
            ]
        );
    }

    /// The name starts with `sys_` because it starts git.
    #[test]
    fn sys_a_directory_that_a_checkout_ignores_is_listed_by_walking_it() {
        let outer =
            std::env::temp_dir().join(format!("cineris-xtask-files-{}", std::process::id()));
        if outer.exists() {
            std::fs::remove_dir_all(&outer).unwrap();
        }
        let copy = outer.join("target").join("copy");
        std::fs::create_dir_all(copy.join("src")).unwrap();
        std::fs::write(outer.join(".gitignore"), "/target/\n").unwrap();
        std::fs::write(outer.join("kept.txt"), "tracked").unwrap();
        std::fs::write(copy.join("src").join("lib.rs"), "// a copy").unwrap();
        std::fs::write(copy.join("README.md"), "# A copy").unwrap();
        // A build of the copy's own, which a walk passes over.
        std::fs::create_dir_all(copy.join("scratch").join("target")).unwrap();
        std::fs::write(copy.join("scratch").join("target").join("built"), "built").unwrap();
        let git = |arguments: &[&str]| {
            let status = std::process::Command::new("git")
                .args(arguments)
                .current_dir(&outer)
                .status()
                .unwrap();
            assert!(status.success(), "git {arguments:?}");
        };
        git(&["init", "--quiet"]);
        git(&["add", ".gitignore"]);
        // A scratch folder nobody added.
        std::fs::create_dir_all(outer.join("scratch")).unwrap();
        std::fs::write(outer.join("scratch").join("notes.txt"), "untracked").unwrap();

        // The checkout itself: what git does not ignore, and of it, what git
        // tracks.
        assert_eq!(
            super::repository_files(&outer).unwrap(),
            [".gitignore", "kept.txt", "scratch/notes.txt"]
        );
        assert_eq!(super::tracked_files(&outer).unwrap(), [".gitignore"]);
        // The copy inside it. Git, asked there, lists nothing.
        assert_eq!(
            super::repository_files(&copy).unwrap(),
            ["README.md", "src/lib.rs"]
        );
        assert_eq!(
            super::tracked_files(&copy).unwrap(),
            ["README.md", "src/lib.rs"]
        );

        std::fs::remove_dir_all(&outer).unwrap();
    }

    #[test]
    fn an_old_report_is_removed_and_no_report_is_no_error() {
        let directory =
            std::env::temp_dir().join(format!("cineris-xtask-report-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let report = directory.join("junit.xml");
        std::fs::write(&report, "<testsuites/>").unwrap();

        super::forget_report(&report).unwrap();
        assert!(!report.exists());
        super::forget_report(&report).unwrap();

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_publishable_package_is_seen_as_one() {
        let json = json!({
            "workspace_root": "/repo",
            "target_directory": "/repo/target",
            "packages": [{
                "name": "cineris-base",
                "manifest_path": "/repo/crates/cineris-base/Cargo.toml",
                "publish": null,
                "dependencies": [],
                "targets": []
            }]
        });

        assert!(!parse_metadata(&json).unwrap().packages[0].publish_disabled);
    }

    /// A checkout of the workspace, as small as `is_checkout` takes, at
    /// `dir`.
    fn checkout(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("xtask")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\nmembers = [\"xtask\"]\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("xtask/Cargo.toml"),
            "[package]\nname = \"xtask\"\n",
        )
        .unwrap();
    }

    #[test]
    fn a_leading_root_option_is_taken_and_nothing_else() {
        let words = |text: &str| -> Vec<String> { text.split(' ').map(str::to_owned).collect() };
        let mut arguments = words("--root ../other links");
        let taken = super::take_root_option(&mut arguments).unwrap();
        assert_eq!(taken.as_deref(), Some(std::path::Path::new("../other")));
        assert_eq!(arguments, words("links"));

        let mut arguments = words("--root=there outstanding --check");
        let taken = super::take_root_option(&mut arguments).unwrap();
        assert_eq!(taken.as_deref(), Some(std::path::Path::new("there")));
        assert_eq!(arguments, words("outstanding --check"));

        // After the command's name it is the command's own argument.
        let mut arguments = words("try --root x");
        assert_eq!(super::take_root_option(&mut arguments).unwrap(), None);
        assert_eq!(arguments, words("try --root x"));

        let mut arguments = words("--root");
        let error = super::take_root_option(&mut arguments).unwrap_err();
        assert!(error.contains("needs a directory"), "{error}");
        assert_eq!(super::take_root_option(&mut Vec::new()).unwrap(), None);
    }

    #[test]
    fn the_root_is_the_nearest_checkout_at_or_above_the_current_directory() {
        let scratch = crate::testkit::Scratch::outside("xtask-root", "nearest");
        let outer = scratch.join("outer");
        checkout(&outer);
        let deep = scratch.dir("outer/crates/some/src");
        assert_eq!(super::find_root(&deep), Some(outer.clone()));
        assert_eq!(super::find_root(&outer), Some(outer.clone()));

        // A copy of the tree inside it (a second checkout under `target/`,
        // a `mutate` copy) is a checkout of its own.
        let copy = outer.join("target/copy");
        checkout(&copy);
        let in_copy = scratch.dir("outer/target/copy/docs");
        assert_eq!(super::find_root(&in_copy), Some(copy));

        // Another project's workspace, with no xtask beside it, is not one:
        // the root is the checkout around it.
        let fuzz = scratch.dir("outer/fuzz");
        std::fs::write(fuzz.join("Cargo.toml"), "[workspace]\n").unwrap();
        assert_eq!(super::find_root(&fuzz), Some(outer.clone()));
        // Nor is a crate of the workspace, with an xtask/ beside it by chance.
        let member = scratch.dir("outer/crates/member");
        std::fs::write(member.join("Cargo.toml"), "[package]\nname = \"member\"\n").unwrap();
        scratch.file("outer/crates/member/xtask/Cargo.toml", "[package]\n");
        assert_eq!(super::find_root(&member), Some(outer));

        // Outside every checkout there is none, and the root must be named.
        assert_eq!(super::find_root(scratch.path()), None);
        let error = super::pick_root(None, scratch.path()).unwrap_err();
        assert!(error.contains("--root DIR"), "{error}");
    }

    #[test]
    fn a_root_that_is_named_is_taken_from_where_xtask_runs_and_must_be_a_checkout() {
        let scratch = crate::testkit::Scratch::outside("xtask-root", "named");
        let one = scratch.join("one");
        let two = scratch.join("two");
        checkout(&one);
        checkout(&two);
        // Named, it wins over the checkout the current directory is in.
        let picked = super::pick_root(Some(std::path::Path::new("../two")), &one).unwrap();
        assert_eq!(picked, two);
        let picked = super::pick_root(Some(&two), scratch.path()).unwrap();
        assert_eq!(picked, two);
        let plain = scratch.dir("plain");
        let error = super::pick_root(Some(&plain), &one).unwrap_err();
        assert!(error.contains("not a checkout"), "{error}");
    }

    /// The xtask binary, as it runs when it was built in one checkout and is
    /// started in another: this test binary, built in this checkout, runs
    /// [`as_xtask`] (`main` with the arguments given) with its current
    /// directory in a checkout of its own. A command that works from the
    /// root (`outstanding` here, which writes the open items' index) works on
    /// that checkout, and says whose source the binary was built from; this
    /// checkout's index is left as it was.
    #[test]
    fn sys_a_build_from_one_checkout_works_on_the_checkout_it_runs_in() {
        let scratch = crate::testkit::Scratch::outside("xtask-root", "elsewhere");
        let other = scratch.join("other");
        checkout(&other);
        scratch.dir("other/crates");
        scratch.file(
            "other/docs/outstanding/an-item-of-the-other-checkout.md",
            "An item only the other checkout has.\n\n- Area: tooling\n- Owner: nobody\n\n\
             | Item | Who | When |\n|---|---|---|\n| **An item.** | Nobody | Never |\n",
        );
        let own_index = super::built_in().join("docs/outstanding/README.md");
        let own_before = std::fs::read(&own_index).unwrap();

        let run = |here: &std::path::Path, arguments: &[&str]| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "workspace::tests::as_xtask", "--ignored"])
                .args(["--nocapture", "--test-threads=1", "--quiet"])
                .env("XTASK_AS_XTASK", arguments.join("\n"))
                .current_dir(here)
                .output()
                .unwrap();
            let said = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            (output.status.success(), said)
        };

        // From a folder inside the other checkout: its index is written.
        let (passed, said) = run(&other.join("docs"), &["outstanding"]);
        assert!(passed, "{said}");
        let index = std::fs::read_to_string(other.join("docs/outstanding/README.md")).unwrap();
        assert!(
            index.contains("An item only the other checkout has."),
            "{index}"
        );
        assert!(
            said.contains("this xtask was built from the source in"),
            "{said}"
        );
        assert_eq!(
            std::fs::read(&own_index).unwrap(),
            own_before,
            "this checkout's index changed"
        );

        // From outside every checkout: refused, unless --root names one.
        let (passed, said) = run(scratch.path(), &["outstanding", "--check"]);
        assert!(!passed, "{said}");
        assert!(said.contains("--root DIR"), "{said}");
        let (passed, said) = run(
            scratch.path(),
            &["--root", "other", "outstanding", "--check"],
        );
        assert!(passed, "{said}");
        assert!(said.contains("is up to date"), "{said}");
    }

    /// `main`, with the arguments in `XTASK_AS_XTASK` (one a line), as the
    /// system test above starts it. Outside that test it does nothing.
    #[test]
    #[ignore = "run by sys_a_build_from_one_checkout_works_on_the_checkout_it_runs_in"]
    fn as_xtask() {
        let Some(arguments) = std::env::var_os("XTASK_AS_XTASK") else {
            return;
        };
        let arguments = arguments
            .to_string_lossy()
            .lines()
            .map(str::to_owned)
            .collect();
        // A failure of the command is this test's, and so the process's.
        assert!(
            crate::main_of(arguments) == std::process::ExitCode::SUCCESS,
            "xtask failed"
        );
    }
}
