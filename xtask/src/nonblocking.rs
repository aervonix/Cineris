//! What the non-blocking workflows run (`docs/testing-and-ci.md`, "Not
//! blocking"): the release build with its size and crate count, the
//! unused-dependency check, the advisory check, and the sandbox's tests
//! with the partial Landlock.
//!
//! They live here, like the blocking checks, so that a workflow is a list of
//! `cargo xtask` commands and the same commands run at a desk.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::Result;
use crate::{runs, shipped, steps, workspace};

/// The package whose binary is shipped.
const BINARY_PACKAGE: &str = "cortex-cli";
/// The profile the binary is built and measured in.
const PROFILE: &str = shipped::PROFILE;
const MEBIBYTE: f64 = 1024.0 * 1024.0;

/// The size budget for a target, in bytes, from docs/performance-budgets.md
/// ("Binary size"): 16 MB on Linux, 18 MB on Windows.
fn budget(target: &str) -> u64 {
    let megabytes: u64 = if target.contains("windows") { 18 } else { 16 };
    megabytes * 1024 * 1024
}

/// The target cargo builds for when none is named.
fn host_target() -> Result<String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .current_dir(workspace::root())
        .output()
        .map_err(|error| format!("could not run rustc -vV: {error}"))?;
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .ok_or_else(|| "rustc -vV did not name a host".to_owned())
}

/// The packages in a build of the binary for a target: every crate cargo
/// compiles for it, ours included, with build dependencies and without
/// development ones.
fn crates_in_build(target: &str) -> Result<BTreeSet<String>> {
    let output = Command::new(workspace::cargo())
        .args([
            "tree",
            "--locked",
            "--package",
            BINARY_PACKAGE,
            "--edges",
            "normal,build",
            "--prefix",
            "none",
            "--target",
            target,
        ])
        .current_dir(workspace::root())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|error| format!("could not run cargo tree: {error}"))?;
    if !output.status.success() {
        return Err("`cargo tree` failed".to_owned());
    }
    Ok(parse_tree(&String::from_utf8_lossy(&output.stdout)))
}

/// The distinct `name version` pairs in `cargo tree --prefix none` output.
fn parse_tree(tree: &str) -> BTreeSet<String> {
    tree.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let name = words.next()?;
            let version = words.next().filter(|word| word.starts_with('v'))?;
            Some(format!("{name} {version}"))
        })
        .collect()
}

/// The size the `main` workflow last recorded for a target.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MainSize {
    bytes: u64,
    /// The run that recorded it.
    run: u64,
    /// The commit it was built from, when the record names one.
    commit: Option<String>,
}

/// The artifact the `main` workflow keeps a target's record in.
fn record_name(target: &str) -> String {
    format!("size-{target}")
}

/// The record's size and commit, from the JSON `size` writes.
fn parse_record(text: &str) -> Result<(u64, Option<String>)> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("a size record: {error}"))?;
    let bytes = value
        .get("bytes")
        .and_then(serde_json::Value::as_u64)
        .ok_or("a size record with no `bytes`")?;
    let commit = value
        .get("commit")
        .and_then(serde_json::Value::as_str)
        .map(|commit| commit.chars().take(7).collect());
    Ok((bytes, commit))
}

/// The size the `main` workflow last recorded for `target`, through `gh`:
/// the newest of its finished runs on the main branch that kept a record.
/// A run whose size job failed over the budget kept one too.
fn main_size(target: &str) -> Result<MainSize> {
    let name = record_name(target);
    let run = runs::latest_main_run_of("main", "completed", &[name.as_str()])?;
    let directory =
        runs::artifact_dir(run, &name)?.ok_or_else(|| format!("run {run} kept no `{name}`"))?;
    let file = directory.join(format!("{target}.json"));
    let text = fs::read_to_string(&file)
        .map_err(|error| format!("could not read {}: {error}", file.display()))?;
    let (bytes, commit) = parse_record(&text)?;
    Ok(MainSize { bytes, run, commit })
}

/// Bytes as mebibytes, for a person.
#[expect(
    clippy::cast_precision_loss,
    reason = "a binary's size in bytes is far below 2^52, where f64 stops being exact"
)]
fn mebibytes(bytes: u64) -> f64 {
    bytes as f64 / MEBIBYTE
}

/// A difference in bytes as kilobytes of 1,024 bytes, as the sizes are
/// given in mebibytes.
#[expect(
    clippy::cast_precision_loss,
    reason = "a difference between two binaries' sizes is far below 2^52"
)]
fn kibibytes(bytes: i128) -> f64 {
    bytes as f64 / 1024.0
}

/// What `size` reports: the job summary's table, and the line it fails
/// with when the binary is over its budget.
#[derive(Debug, PartialEq, Eq)]
struct Report {
    summary: String,
    over: Option<String>,
}

/// The report of one target's binary. `main` is the size the `main`
/// workflow last recorded, when it was asked for: `Some(Err)` says why it
/// could not be read, which is shown and fails nothing.
fn report(target: &str, bytes: u64, crates: usize, main: Option<&Result<MainSize>>) -> Report {
    let limit = budget(target);
    let (megabytes, limit_megabytes) = (mebibytes(bytes), mebibytes(limit));
    let room = if bytes <= limit {
        format!("{:.2} MB left", mebibytes(limit - bytes))
    } else {
        format!("**{:.2} MB over**", mebibytes(bytes - limit))
    };
    let mut summary = format!(
        "### Binary size and crate count\n\n\
         | Target | Profile | Bytes | MB | Budget | Room | Crates |\n\
         |---|---|---:|---:|---:|---|---:|\n\
         | `{target}` | `{PROFILE}` | {bytes} | {megabytes:.2} | {limit_megabytes:.0} MB | {room} | {crates} |\n\n"
    );
    match main {
        None => {}
        Some(Ok(main)) => {
            let difference = i128::from(bytes) - i128::from(main.bytes);
            let at = main
                .commit
                .as_ref()
                .map_or_else(String::new, |commit| format!(" at {commit}"));
            summary.push_str(&format!(
                "Against `main`'s last recorded size, {} bytes (run {}{at}): **{difference:+} \
                 bytes** ({:+.1} KB).\n\n",
                main.bytes,
                main.run,
                kibibytes(difference)
            ));
        }
        Some(Err(error)) => {
            summary.push_str(&format!(
                "`main`'s last recorded size could not be read: {error}\n\n"
            ));
        }
    }
    let over = (bytes > limit).then(|| {
        format!(
            "the {target} binary is {bytes} bytes ({megabytes:.2} MB), over its budget of \
             {limit_megabytes:.0} MB by {} bytes (docs/performance-budgets.md, \"Binary size\"). \
             Find what grew, or raise the question: the budget is not loosened quietly.",
            bytes - limit
        )
    });
    if let Some(over) = &over {
        summary.push_str(&format!("**Over budget:** {over}\n\n"));
    }
    Report { summary, over }
}

/// `cargo xtask size [--check] [--target TRIPLE] [--against-main]`
pub(crate) fn size(arguments: &[String]) -> Result<()> {
    let mut check = false;
    let mut against_main = false;
    let mut target = None;
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--check" => check = true,
            "--against-main" => against_main = true,
            "--target" => {
                target = Some(
                    arguments
                        .next()
                        .ok_or("--target needs a target triple")?
                        .clone(),
                );
            }
            _ => {
                return Err(
                    "usage: cargo xtask size [--check] [--target TRIPLE] [--against-main]"
                        .to_owned(),
                );
            }
        }
    }
    let target = match target {
        Some(target) => target,
        None => host_target()?,
    };

    shipped::build(Some(&target))?;

    let metadata = workspace::metadata(&workspace::root())?;
    let executable = if target.contains("windows") {
        "cortex.exe"
    } else {
        "cortex"
    };
    let binary: PathBuf = metadata
        .target_directory
        .join(&target)
        .join(PROFILE)
        .join(executable);
    let bytes = fs::metadata(&binary)
        .map_err(|error| format!("could not read {}: {error}", binary.display()))?
        .len();
    let crates = crates_in_build(&target)?.len();
    let limit = budget(&target);
    println!(
        "\n{target}, `{PROFILE}` profile\n  binary   {bytes} bytes ({:.2} MB; budget {:.0} MB)\n  crates   {crates}",
        mebibytes(bytes),
        mebibytes(limit)
    );
    let main = against_main.then(|| main_size(&target));
    if let Some(main) = &main {
        match main {
            Ok(main) => println!(
                "  main     {} bytes (run {}): {:+} bytes",
                main.bytes,
                main.run,
                i128::from(bytes) - i128::from(main.bytes)
            ),
            Err(error) => println!("  main     not read: {error}"),
        }
    }
    let paths = shipped::check_file(&binary);
    let report = report(&target, bytes, crates, main.as_ref());
    steps::append_job_summary(&report.summary);
    if let Err(message) = &paths {
        steps::append_job_summary(&format!("**Home paths:** {message}\n\n"));
    }

    // The record, for the workflow to keep as an artifact.
    let record = serde_json::json!({
        "target": target,
        "profile": PROFILE,
        "bytes": bytes,
        "budget_bytes": limit,
        "crates": crates,
        "commit": std::env::var("GITHUB_SHA").ok(),
    });
    let directory = metadata.target_directory.join("size");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    let file = directory.join(format!("{target}.json"));
    fs::write(&file, format!("{record:#}\n"))
        .map_err(|error| format!("could not write {}: {error}", file.display()))?;

    // A home path fails every run, measured or checked: it is never right.
    paths?;
    match report.over {
        Some(over) if check => Err(over),
        _ => Ok(()),
    }
}

/// `cargo xtask unused`
///
/// The tool is run by its own name, not as `cargo machete`: started through
/// cargo from inside `cargo run`, version 0.9.2 took the word `machete` for
/// the directory to analyse and failed (seen on the first nightly run).
pub(crate) fn unused() -> Result<()> {
    const TOOL: &str = "cargo-machete";
    println!("$ {TOOL}");
    let status = Command::new(TOOL)
        .current_dir(workspace::root())
        .status()
        .map_err(|error| {
            format!(
                "could not run {TOOL}: {error}. Is it installed? The version CI uses is in \
                 docs/dependencies.md."
            )
        })?;
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "`{TOOL}` failed.\n    It reads the sources and reports a dependency that no code names. \
         Remove it from the manifest and from docs/dependencies.md, or, if it is used in a way \
         the tool cannot see, say so in the manifest under [package.metadata.cargo-machete]."
    ))
}

/// `cargo xtask advisories`
pub(crate) fn advisories() -> Result<()> {
    steps::cargo(&["deny", "check", "advisories"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_crate_in_the_tree_is_counted_once() {
        let tree = "\
cortex-cli v0.0.0 (/repo/crates/cortex-cli)
cortex-core v0.0.0 (/repo/crates/cortex-core)
cortex-protocol v0.0.0 (/repo/crates/cortex-protocol)
serde v1.0.229
serde_core v1.0.229
serde_derive v1.0.229 (proc-macro)
serde_json v1.0.154
serde_core v1.0.229
cortex-protocol v0.0.0 (/repo/crates/cortex-protocol) (*)

[build-dependencies]
";
        let crates = parse_tree(tree);
        assert_eq!(crates.len(), 7, "{crates:?}");
        assert!(crates.contains("serde_derive v1.0.229"));
    }

    const LINUX: &str = "x86_64-unknown-linux-musl";
    const WINDOWS: &str = "x86_64-pc-windows-msvc";

    #[test]
    fn a_binary_within_its_budget_shows_its_room_and_fails_nothing() {
        let report = report(LINUX, 12_000_000, 180, None);
        assert_eq!(report.over, None);
        assert!(
            report
                .summary
                .contains("| `x86_64-unknown-linux-musl` | `dist` | 12000000 | 11.44 | 16 MB | 4.56 MB left | 180 |"),
            "{}",
            report.summary
        );
        assert!(!report.summary.contains("main"), "{}", report.summary);
    }

    #[test]
    fn a_binary_over_its_budget_fails_with_a_line_that_says_by_how_much() {
        let limit = budget(WINDOWS);
        let report = report(WINDOWS, limit + 351_744, 180, None);
        let over = report.over.expect("over the budget");
        assert!(
            over.starts_with("the x86_64-pc-windows-msvc binary is 19226112 bytes (18.34 MB), over its budget of 18 MB by 351744 bytes"),
            "{over}"
        );
        assert!(
            report.summary.contains("**0.34 MB over**"),
            "{}",
            report.summary
        );
        assert!(report.summary.contains(&format!("**Over budget:** {over}")));
        // One byte at the budget is within it.
        assert_eq!(super::report(WINDOWS, limit, 180, None).over, None);
    }

    #[test]
    fn the_difference_from_mains_last_size_is_signed() {
        let main = Ok(MainSize {
            bytes: 12_100_000,
            run: 43_811_142_390,
            commit: Some("c225d64".to_owned()),
        });
        let smaller = report(LINUX, 12_000_000, 180, Some(&main)).summary;
        assert!(
            smaller.contains(
                "12100000 bytes (run 43811142390 at c225d64): **-100000 bytes** (-97.7 KB)"
            ),
            "{smaller}"
        );
        let larger = report(LINUX, 12_102_048, 180, Some(&main)).summary;
        assert!(larger.contains("**+2048 bytes** (+2.0 KB)"), "{larger}");
        // A record that could not be read is said, and fails nothing.
        let unread = Err("gh: not signed in".to_owned());
        let report = report(LINUX, 12_000_000, 180, Some(&unread));
        assert!(
            report
                .summary
                .contains("could not be read: gh: not signed in")
        );
        assert_eq!(report.over, None);
    }

    #[test]
    fn a_size_record_is_read_back() {
        let text = "{\n  \"target\": \"x\",\n  \"bytes\": 16258408,\n  \"commit\": \"c225d64aaaabbbb\"\n}\n";
        assert_eq!(
            parse_record(text).unwrap(),
            (16_258_408, Some("c225d64".to_owned()))
        );
        assert_eq!(
            parse_record("{\"bytes\": 1, \"commit\": null}").unwrap(),
            (1, None)
        );
        assert!(parse_record("{\"crates\": 1}").is_err());
        assert_eq!(record_name(LINUX), "size-x86_64-unknown-linux-musl");
    }

    #[test]
    fn the_budgets_are_the_ones_in_the_performance_page() {
        assert_eq!(budget("x86_64-unknown-linux-musl"), 16 * 1024 * 1024);
        assert_eq!(budget("aarch64-unknown-linux-musl"), 16 * 1024 * 1024);
        assert_eq!(budget("x86_64-pc-windows-msvc"), 18 * 1024 * 1024);
    }
}

/// What the sandbox's signal test prints when the kernel's Landlock does not
/// scope signals: the gap of the partial case, named.
const PARTIAL_MARK: &str = "does not scope signals: the gap the status names";

/// `cargo xtask partial-landlock`: `cortex-sandbox`'s tests, as the blocking
/// job runs them, on a kernel whose Landlock is older than 6 (the `nightly`
/// workflow's Ubuntu 22.04 job, ABI 4). The confinement tests fail rather
/// than skip on CI where the sandbox does not enforce; this fails too when
/// the run did not show the partial case, so that a kernel that moves on
/// does not leave the case untested in silence.
pub(crate) fn partial_landlock() -> Result<()> {
    crate::checks::require_nextest()?;
    let mut run = crate::checks::ci_run(&["cortex-sandbox".to_owned()]);
    run.extend(partial_arguments());
    println!("$ cargo {}", run.join(" "));
    let output = Command::new(workspace::cargo())
        .args(&run)
        .current_dir(workspace::root())
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("could not run cargo: {error}"))?;
    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{printed}");
    partial_verdict(output.status.success(), &printed)
}

/// What the run adds to the blocking job's: the backends, which only a
/// build with `cortex-core` in it turns on, and without which the
/// confinement tests are not compiled at all; and the tests' own output,
/// where the partial case is named.
fn partial_arguments() -> [String; 4] {
    [
        "--features".to_owned(),
        "cortex-sandbox/backends".to_owned(),
        "--success-output".to_owned(),
        "immediate".to_owned(),
    ]
}

/// Whether a run of the sandbox's tests passed and showed the partial case.
fn partial_verdict(passed: bool, printed: &str) -> Result<()> {
    if !passed {
        return Err("the sandbox's tests failed with the partial Landlock".to_owned());
    }
    if let Some(line) = printed.lines().find(|line| line.contains(PARTIAL_MARK)) {
        println!("\nThe partial case ran: {}", line.trim());
        Ok(())
    } else {
        Err(format!(
            "the tests passed, but none said `{PARTIAL_MARK}`, so the partial case ran \
             nowhere: either the confinement tests were not built (they need \
             cortex-sandbox's `backends` feature), or this kernel's Landlock scopes signals. In \
             the second case move the job to a runner with Landlock 4 or 5, or say in \
             docs/testing-and-ci.md that it is no longer tested."
        ))
    }
}

#[cfg(test)]
mod partial_tests {
    use super::partial_verdict;

    #[test]
    fn the_partial_case_must_be_seen_and_the_tests_must_pass() {
        let shown = "    PASS [ 0.1s] cortex-sandbox::it confined::sys_a_signal_to_a_process_outside_the_sandbox_is_refused\n\
                     Landlock 4 does not scope signals: the gap the status names\n";
        assert_eq!(partial_verdict(true, shown), Ok(()));
        let error =
            partial_verdict(true, "    PASS [ 0.1s] cortex-sandbox::it confined::x\n").unwrap_err();
        assert!(error.contains("the partial case ran nowhere"), "{error}");
        // The confinement tests are built only with the backends.
        assert!(
            super::partial_arguments()
                .windows(2)
                .any(|pair| pair == ["--features", "cortex-sandbox/backends"])
        );
        let error = partial_verdict(false, shown).unwrap_err();
        assert!(error.contains("tests failed"), "{error}");
    }
}
