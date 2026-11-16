//! `cargo xtask scratch-test CRATE FILE`: one throwaway test file run
//! against a crate's integration tests, with nothing to register by hand
//! and nothing left behind (`docs/testing-and-ci.md`, "A throwaway test").
//!
//! A review that confirms a finding writes a failing test for it. Without
//! this it had to add the file to `tests/it/` and a `mod` line to its
//! `main.rs`, and take both out again: a step a killed run, or a commit
//! made in a hurry, gets wrong.
//!
//! **The worktree is only read.** The run is made in a copy of the tree
//! under `target/scratch-test/tree`, which git ignores, with a build
//! directory of its own (`target/scratch-test/target`):
//!
//! 1. the copy is brought up to date with the worktree: what git tracks and
//!    what it does not ignore, as `mutate`'s copies are. Whatever an
//!    earlier run added to the copy, a run that was killed included, goes
//!    here;
//! 2. the file is put beside the crate's integration tests in the copy, as
//!    a module named `scratch_<its name>`, and `main.rs` there gets its
//!    `mod` line;
//! 3. nextest runs that module's tests alone, in the copy;
//! 4. the copy is brought up to date again, which takes the module out.
//!
//! The first run builds the crate and what it depends on in the copy's
//! build directory; later runs rebuild what changed. One run at a time: a
//! second waits for nothing and is refused while the first holds the lock.
//!
//! **A measurement is a throwaway test too** (since the seventh tooling
//! step). `--cargo-profile release` builds and runs the file in that cargo
//! profile, and `--ignored` runs the file's ignored tests, which a review's
//! `perf_` test is. `--ignored` is the cargo test flag's meaning, only the
//! ignored ones; `--include-ignored` runs the others with them. A run of
//! ignored tests is a measurement: its output is shown (`--no-capture`, which
//! also runs one test at a time) and nextest's `perf` profile gives it half an
//! hour, where the default stops a test at 10 s.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;
use crate::workspace;

const USAGE: &str = "\
usage: cargo xtask scratch-test CRATE FILE [OPTIONS] [-- NEXTEST ARGUMENTS]

Runs the tests in FILE, a Rust file of #[test] functions written as a module
of CRATE's integration tests (`use crate::…` reaches the binary's helpers), in
a copy of the tree under target/scratch-test/. Nothing is written to the
worktree. It fails when a test fails, and when FILE has no test that ran.

  --cargo-profile P  Build and run in cargo profile P (`release`, `ci`,
                     `dist`...): for a measurement of optimised code.
  --ignored          Run only the file's ignored tests, one at a time, with
                     their output shown, under nextest's `perf` profile (half
                     an hour for a test, where the default stops one at 10 s):
                     a review's `perf_` test.
  --include-ignored  The same, and the file's other tests with them.
  --no-capture       Show the tests' output (implied by the two above).";

/// Where the copy and its build are, under the worktree's build directory.
fn scratch_dir(root: &Path) -> PathBuf {
    root.join("target").join("scratch-test")
}

/// Which of the file's tests are run besides the ones that are not ignored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Ignored {
    /// The tests that are not ignored.
    #[default]
    No,
    /// The ignored ones only.
    Only,
    /// Both.
    Include,
}

/// What the run is built and run as, beyond the file.
#[derive(Debug, Default, PartialEq)]
struct Options {
    cargo_profile: Option<String>,
    ignored: Ignored,
    no_capture: bool,
}

/// What was asked.
#[derive(Debug, PartialEq)]
struct Request {
    package: String,
    file: PathBuf,
    options: Options,
    rest: Vec<String>,
}

impl Request {
    fn parse(arguments: &[String]) -> Result<Self> {
        let (ours, rest) = match arguments.iter().position(|word| word == "--") {
            Some(at) => (&arguments[..at], arguments[at + 1..].to_vec()),
            None => (arguments, Vec::new()),
        };
        let mut options = Options::default();
        let mut positional = Vec::new();
        let mut words = ours.iter();
        while let Some(word) = words.next() {
            match word.as_str() {
                "--cargo-profile" => {
                    let profile = words.next().ok_or_else(|| USAGE.to_owned())?;
                    options.cargo_profile = Some(profile.clone());
                }
                "--ignored" | "--include-ignored" => {
                    let wanted = if word == "--ignored" {
                        Ignored::Only
                    } else {
                        Ignored::Include
                    };
                    if options.ignored != Ignored::No && options.ignored != wanted {
                        return Err(
                            "`--ignored` and `--include-ignored` are two answers to one \
                                    question: give one"
                                .to_owned(),
                        );
                    }
                    options.ignored = wanted;
                }
                "--no-capture" => options.no_capture = true,
                other => {
                    if let Some(profile) = other.strip_prefix("--cargo-profile=") {
                        options.cargo_profile = Some(profile.to_owned());
                    } else if other.starts_with('-') {
                        return Err(USAGE.to_owned());
                    } else {
                        positional.push(other.to_owned());
                    }
                }
            }
        }
        if let Some(profile) = &options.cargo_profile
            && (profile.starts_with(['-', '_'])
                || profile.is_empty()
                || !profile
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            return Err(format!("`{profile}` is not the name of a cargo profile"));
        }
        match positional.as_slice() {
            [package, file] => Ok(Self {
                package: package.clone(),
                file: PathBuf::from(file),
                options,
                rest,
            }),
            _ => Err(USAGE.to_owned()),
        }
    }
}

/// The module the file becomes: `scratch_` and its name, made a Rust name.
fn module_name(file: &Path) -> Result<String> {
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| format!("{} has no name to make a module of", file.display()))?;
    let name: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    Ok(format!("scratch_{name}"))
}

/// Puts `source` in the copy at `copy` as module `module` of `package`'s
/// integration tests. Returns the test filter that runs it alone.
fn place(copy: &Path, package: &str, module: &str, source: &str) -> Result<String> {
    let tests = copy.join("crates").join(package).join("tests").join("it");
    let main = tests.join("main.rs");
    let mut text = fs::read_to_string(&main).map_err(|_| {
        format!(
            "{package} has no integration tests at crates/{package}/tests/it/main.rs to run a \
             scratch test beside"
        )
    })?;
    let file = tests.join(format!("{module}.rs"));
    if file.exists() {
        return Err(format!(
            "crates/{package}/tests/it/{module}.rs is a file of the crate's own: name the \
             scratch file something else"
        ));
    }
    fs::write(&file, source).map_err(|error| format!("could not write the module: {error}"))?;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&format!(
        "\n// Added by `cargo xtask scratch-test`, in its copy of the tree only.\nmod {module};\n"
    ));
    fs::write(&main, text).map_err(|error| format!("could not write main.rs: {error}"))?;
    Ok(format!("test(/^{module}::/)"))
}

/// Takes the copy for this run. The lock is the system's, so a run that is
/// killed lets go of it.
fn lock(scratch: &Path) -> Result<File> {
    fs::create_dir_all(scratch)
        .map_err(|error| format!("could not create {}: {error}", scratch.display()))?;
    let path = scratch.join("lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| format!("could not open {}: {error}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(fs::TryLockError::WouldBlock) => Err(
            "another `cargo xtask scratch-test` is running on the copy: wait for it to end"
                .to_owned(),
        ),
        Err(fs::TryLockError::Error(error)) => {
            Err(format!("could not lock {}: {error}", path.display()))
        }
    }
}

/// Runs `request` in a copy of the tree at `root`, with `run` doing the
/// testing: the copy, the filter and the extra arguments. The copy is made
/// like the worktree before the run and again after it, whatever the run
/// did.
fn scratch_test(
    root: &Path,
    request: &Request,
    run: impl FnOnce(&Path, &str, &[String]) -> Result<()>,
) -> Result<()> {
    let source = fs::read_to_string(&request.file)
        .map_err(|error| format!("could not read {}: {error}", request.file.display()))?;
    let module = module_name(&request.file)?;
    let scratch = scratch_dir(root);
    let _lock = lock(&scratch)?;
    let copy = scratch.join("tree");
    let files = crate::mutate::tree_files(root)?;
    crate::mutate::sync(root, &copy, &files)?;
    let ran = place(&copy, &request.package, &module, &source)
        .and_then(|filter| run(&copy, &filter, &request.rest));
    // The module goes again, pass or fail.
    let tidied = crate::mutate::sync(root, &copy, &files);
    ran.and(tidied)
}

/// `cargo xtask scratch-test …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    crate::checks::require_nextest()?;
    let root = workspace::root();
    let target = scratch_dir(&root).join("target");
    scratch_test(&root, &request, |copy, filter, rest| {
        let command = nextest_command(&request.package, filter, &request.options, rest);
        println!(
            "$ cargo {}   (in target/scratch-test/tree, building in target/scratch-test/target)",
            command.join(" ")
        );
        let status = Command::new(workspace::cargo())
            .args(&command)
            .current_dir(copy)
            .env("CARGO_TARGET_DIR", &target)
            .status()
            .map_err(|error| format!("could not run cargo: {error}"))?;
        match status.code() {
            Some(0) => Ok(()),
            // nextest's "no tests to run": a file with no test in it would
            // otherwise pass.
            Some(4) => Err(format!(
                "no test of {} ran: it has no #[test] function, or the arguments after `--` \
                 filtered them all out",
                request.file.display()
            )),
            _ => Err(format!("a test of {} failed", request.file.display())),
        }
    })
}

/// Whether the arguments for nextest already say `name` (a flag, spelt
/// `--name`, or `--name=value`, or `-X`): a flag given twice is refused.
fn says(rest: &[String], long: &str, short: Option<&str>) -> bool {
    rest.iter().any(|word| {
        word == long
            || word.starts_with(&format!("{long}="))
            || short.is_some_and(|short| word == short)
    })
}

/// `cargo nextest run` for the scratch module's tests: `filter` picks the
/// module, and a filterset among `rest` (`-E 'test(/stuck/)'`) narrows it,
/// joined with `and`. nextest runs a test that any of its filtersets
/// matches, so given beside the module's own it would run every test of
/// the file (the review of step 16 was misled by that).
///
/// `options` add what a measurement needs: the cargo profile, the ignored
/// tests (and then nextest's `perf` profile, whose limit is half an hour, and
/// the output shown, unless `rest` says another profile or already says it).
fn nextest_command(package: &str, filter: &str, options: &Options, rest: &[String]) -> Vec<String> {
    let mut command = vec![
        "nextest".to_owned(),
        "run".to_owned(),
        "--locked".to_owned(),
        "-p".to_owned(),
        package.to_owned(),
    ];
    command.extend(workspace::implied_features(&[package.to_owned()]));
    if let Some(profile) = &options.cargo_profile {
        command.extend(["--cargo-profile".to_owned(), profile.clone()]);
    }
    let measuring = options.ignored != Ignored::No;
    match options.ignored {
        Ignored::No => {}
        Ignored::Only => command.extend(["--run-ignored".to_owned(), "only".to_owned()]),
        Ignored::Include => command.extend(["--run-ignored".to_owned(), "all".to_owned()]),
    }
    if measuring && !says(rest, "--profile", Some("-P")) {
        command.extend(["--profile".to_owned(), "perf".to_owned()]);
    }
    if (measuring || options.no_capture) && !says(rest, "--no-capture", None) {
        command.push("--no-capture".to_owned());
    }
    let (given, rest) = crate::checks::filtersets(rest);
    let filter = match given {
        Some(given) => format!("({filter}) and ({given})"),
        None => filter.to_owned(),
    };
    command.extend(["-E".to_owned(), filter]);
    command.extend(rest);
    command
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[test]
    fn a_filter_after_the_separator_narrows_the_files_tests() {
        let words = |text: &[&str]| {
            text.iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        let command = super::nextest_command(
            "cortex-stats",
            "test(/^scratch_f::/)",
            &Options::default(),
            &words(&["-E", "test(/stuck/)", "--no-capture"]),
        );
        assert_eq!(
            command,
            words(&[
                "nextest",
                "run",
                "--locked",
                "-p",
                "cortex-stats",
                "-E",
                "(test(/^scratch_f::/)) and (test(/stuck/))",
                "--no-capture",
            ])
        );
        // Without one, the module's filter alone.
        let command = super::nextest_command(
            "cortex-stats",
            "test(/^scratch_f::/)",
            &Options::default(),
            &[],
        );
        assert_eq!(command[5..], words(&["-E", "test(/^scratch_f::/)"]));
    }

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|word| (*word).to_owned()).collect()
    }

    /// The arguments of nextest for a request of `package`'s file, as the
    /// words after `-p cortex-stats`.
    fn after_the_package(arguments: &[&str]) -> Vec<String> {
        let request = Request::parse(&words(arguments)).unwrap();
        let command = super::nextest_command(
            &request.package,
            "test(/^scratch_f::/)",
            &request.options,
            &request.rest,
        );
        command[5..].to_vec()
    }

    #[test]
    fn a_cargo_profile_is_passed_to_nextest() {
        assert_eq!(
            after_the_package(&["cortex-stats", "f.rs", "--cargo-profile", "release"]),
            words(&["--cargo-profile", "release", "-E", "test(/^scratch_f::/)"])
        );
        assert_eq!(
            after_the_package(&["cortex-stats", "f.rs", "--cargo-profile=dist"]),
            words(&["--cargo-profile", "dist", "-E", "test(/^scratch_f::/)"])
        );
    }

    #[test]
    fn the_ignored_tests_are_run_alone_one_at_a_time_with_their_output_under_the_perf_profile() {
        assert_eq!(
            after_the_package(&["cortex-stats", "f.rs", "--ignored"]),
            words(&[
                "--run-ignored",
                "only",
                "--profile",
                "perf",
                "--no-capture",
                "-E",
                "test(/^scratch_f::/)",
            ])
        );
        // With the file's other tests when asked.
        assert_eq!(
            after_the_package(&["cortex-stats", "f.rs", "--include-ignored"])[..2],
            words(&["--run-ignored", "all"])
        );
        // A measurement in release, narrowed to one test.
        assert_eq!(
            after_the_package(&[
                "cortex-stats",
                "f.rs",
                "--cargo-profile",
                "release",
                "--ignored",
                "--",
                "-E",
                "test(/perf_/)",
            ]),
            words(&[
                "--cargo-profile",
                "release",
                "--run-ignored",
                "only",
                "--profile",
                "perf",
                "--no-capture",
                "-E",
                "(test(/^scratch_f::/)) and (test(/perf_/))",
            ])
        );
    }

    #[test]
    fn a_profile_or_no_capture_of_the_callers_is_not_given_twice() {
        // nextest refuses a flag twice, and the caller's profile is theirs.
        assert_eq!(
            after_the_package(&[
                "cortex-stats",
                "f.rs",
                "--ignored",
                "--",
                "--profile",
                "ci",
                "--no-capture",
            ]),
            words(&[
                "--run-ignored",
                "only",
                "-E",
                "test(/^scratch_f::/)",
                "--profile",
                "ci",
                "--no-capture",
            ])
        );
        // Output shown without the ignored tests.
        assert_eq!(
            after_the_package(&["cortex-stats", "f.rs", "--no-capture"]),
            words(&["--no-capture", "-E", "test(/^scratch_f::/)"])
        );
    }

    #[test]
    fn the_options_are_read_and_a_bad_one_is_refused() {
        let request = Request::parse(&words(&[
            "--ignored",
            "cortex-core",
            "--cargo-profile",
            "release",
            "f.rs",
        ]))
        .unwrap();
        assert_eq!(request.package, "cortex-core");
        assert_eq!(
            request.options,
            Options {
                cargo_profile: Some("release".to_owned()),
                ignored: Ignored::Only,
                no_capture: false,
            }
        );
        for bad in [
            &["c", "f.rs", "--ignored", "--include-ignored"][..],
            &["c", "f.rs", "--cargo-profile"],
            &["c", "f.rs", "--cargo-profile", "re lease"],
            &["c", "f.rs", "--cargo-profile=--x"],
            &["c", "f.rs", "--frobnicate"],
        ] {
            assert!(Request::parse(&words(bad)).is_err(), "{bad:?}");
        }
    }
    use std::path::Path;

    use super::{Ignored, Options, Request, module_name, scratch_test};

    /// A tree with git's files, as a worktree has them: a crate with an
    /// integration-test binary.
    fn tree(name: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("xtask-scratch-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let it = root.join("crates/demo/tests/it");
        fs::create_dir_all(&it).unwrap();
        fs::write(it.join("main.rs"), "mod one;\n").unwrap();
        fs::write(it.join("one.rs"), "#[test]\nfn one() {}\n").unwrap();
        fs::write(root.join("Cargo.toml"), "[workspace]\n").unwrap();
        root
    }

    /// Every file under `dir` with its content, but the build directory.
    fn contents(dir: &Path) -> Vec<(String, Vec<u8>)> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let relative = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                if relative == "target" {
                    continue;
                }
                if path.is_dir() {
                    walk(&path, base, out);
                } else {
                    out.push((relative, fs::read(&path).unwrap()));
                }
            }
        }
        let mut out = Vec::new();
        walk(dir, dir, &mut out);
        out.sort();
        out
    }

    #[test]
    fn the_file_runs_in_a_copy_and_nothing_is_left_behind() {
        let root = tree("left");
        let outside = root.with_extension("files");
        fs::create_dir_all(&outside).unwrap();
        let file = outside.join("finding.rs");
        fs::write(&file, "#[test]\nfn the_finding() { assert!(false); }\n").unwrap();
        let before = contents(&root);
        let request = Request {
            package: "demo".to_owned(),
            file: file.clone(),
            options: Options::default(),
            rest: vec!["--no-capture".to_owned()],
        };
        let mut seen = None;
        let outcome = scratch_test(&root, &request, |copy, filter, rest| {
            // While the test runs, the worktree is as it was.
            assert_eq!(contents(&root), before, "the worktree was written to");
            let it = copy.join("crates/demo/tests/it");
            seen = Some((
                fs::read_to_string(it.join("main.rs")).unwrap(),
                fs::read_to_string(it.join("scratch_finding.rs")).unwrap(),
                filter.to_owned(),
                rest.to_vec(),
            ));
            Err("a test failed".to_owned())
        });
        // The run's own failure is the command's.
        assert_eq!(outcome, Err("a test failed".to_owned()));
        let (main, module, filter, rest) = seen.unwrap();
        assert!(main.starts_with("mod one;\n") && main.ends_with("mod scratch_finding;\n"));
        assert!(module.contains("fn the_finding"));
        assert_eq!(filter, "test(/^scratch_finding::/)");
        assert_eq!(rest, ["--no-capture"]);
        // The worktree is as it was; the copy is the worktree again.
        assert_eq!(contents(&root), before);
        let copy = root.join("target/scratch-test/tree");
        assert_eq!(contents(&copy), before);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_crate_without_integration_tests_or_a_clashing_name_is_refused() {
        let root = tree("refused");
        let outside = root.with_extension("files");
        fs::create_dir_all(&outside).unwrap();
        let file = outside.join("one.rs");
        fs::write(&file, "#[test]\nfn x() {}\n").unwrap();
        let request = |package: &str| Request {
            package: package.to_owned(),
            file: file.clone(),
            options: Options::default(),
            rest: Vec::new(),
        };
        let never = |_: &Path, _: &str, _: &[String]| -> Result<(), String> {
            panic!("nothing should run")
        };
        let error = scratch_test(&root, &request("other"), never).unwrap_err();
        assert!(error.contains("has no integration tests"), "{error}");
        // A file of the crate's own is never overwritten, even in the copy.
        let clash = root.join("crates/demo/tests/it/scratch_one.rs");
        fs::write(&clash, "// the crate's own\n").unwrap();
        let error = scratch_test(&root, &request("demo"), never).unwrap_err();
        assert!(error.contains("a file of the crate's own"), "{error}");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn a_request_and_a_module_name_are_read() {
        let words = |list: &[&str]| {
            list.iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        let request =
            Request::parse(&words(&["cortex-core", "f.rs", "--", "--no-capture"])).unwrap();
        assert_eq!(request.package, "cortex-core");
        assert_eq!(request.rest, ["--no-capture"]);
        assert!(Request::parse(&words(&["cortex-core"])).is_err());
        assert!(Request::parse(&words(&["-p", "x", "f.rs"])).is_err());
        assert_eq!(
            module_name(Path::new("/tmp/Review R4-check.rs")).unwrap(),
            "scratch_review_r4_check"
        );
    }
}
