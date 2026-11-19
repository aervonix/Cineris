//! What the non-blocking workflows run (`docs/testing-and-ci.md`, "Not
//! blocking"): the release build with its size and crate count, the
//! unused-dependency check, the advisory check, and the sandbox's tests
//! with the partial Landlock.
//!
//! They live here, like the blocking checks, so that a workflow is a list of
//! `cargo xtask` commands and the same commands run at a desk.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Result;
use crate::{runs, shipped, steps, workspace};

/// The package whose binary is shipped.
const BINARY_PACKAGE: &str = "cineris-cli";
/// The profile the binary is built and measured in.
const PROFILE: &str = shipped::PROFILE;
const MEBIBYTE: f64 = 1024.0 * 1024.0;

/// The size budget for a target, in bytes, from docs/performance-budgets.md
/// ("Binary size"): 20 MB on Linux, 25 MB on Windows, since the size step
/// (docs/decisions/0039-the-binary-s-budgets-are-20-mib-on-linux-and-25-on-windows.md).
fn budget(target: &str) -> u64 {
    let megabytes: u64 = if target.contains("windows") { 25 } else { 20 };
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
fn crates_in_build(tree: &Path, target: &str) -> Result<BTreeSet<String>> {
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
        .current_dir(tree)
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
    /// Its bytes by crate, when the run made a breakdown (Linux).
    crates: Option<Vec<(String, u64)>>,
}

/// How much a pull request's binary may grow against `main`'s before
/// `size --against-main` names the crates that grew most (Linux, where it
/// can read them): 100 KB.
const GROWTH_TO_EXPLAIN: i128 = 100 * 1024;

/// How many of the crates that grew it names.
const GREW_NAMED: usize = 3;

/// The file a breakdown's bytes by crate are kept in, beside the record.
fn crates_record_name(target: &str) -> String {
    format!("{target}.crates.json")
}

/// The bytes by crate a breakdown kept, as JSON: `{"crate": bytes, ...}`.
fn crates_record(sizes: &[(String, u64)]) -> String {
    let map: serde_json::Map<String, serde_json::Value> = sizes
        .iter()
        .map(|(name, bytes)| (name.clone(), serde_json::Value::from(*bytes)))
        .collect();
    format!("{:#}\n", serde_json::Value::Object(map))
}

fn parse_crates_record(text: &str) -> Result<Vec<(String, u64)>> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("a record by crate: {error}"))?;
    let object = value
        .as_object()
        .ok_or("a record by crate that is no object")?;
    object
        .iter()
        .map(|(name, bytes)| {
            bytes
                .as_u64()
                .map(|bytes| (name.clone(), bytes))
                .ok_or_else(|| format!("a record by crate whose `{name}` is no number"))
        })
        .collect()
}

/// The crates that grew most from `main` to `now`, largest first, with how
/// much: a crate new since `main` counts whole. Only those that grew.
fn grew_most(now: &[(String, u64)], main: &[(String, u64)], count: usize) -> Vec<(String, i128)> {
    let before: BTreeMap<&str, u64> = main
        .iter()
        .map(|(name, bytes)| (name.as_str(), *bytes))
        .collect();
    let mut grew: Vec<(String, i128)> = now
        .iter()
        .map(|(name, bytes)| {
            (
                name.clone(),
                i128::from(*bytes) - i128::from(before.get(name.as_str()).copied().unwrap_or(0)),
            )
        })
        .filter(|(_, growth)| *growth > 0)
        .collect();
    grew.sort_by(|one, other| other.1.cmp(&one.1).then_with(|| one.0.cmp(&other.0)));
    grew.truncate(count);
    grew
}

/// What `size --against-main` says of a binary that grew by more than
/// [`GROWTH_TO_EXPLAIN`]: the crates that grew most, or why it cannot say.
fn growth_text(
    growth: i128,
    now: Option<&[(String, u64)]>,
    main: Option<&[(String, u64)]>,
) -> Option<String> {
    if growth <= GROWTH_TO_EXPLAIN {
        return None;
    }
    let said = match (now, main) {
        (Some(now), Some(main)) => {
            let grew = grew_most(now, main, GREW_NAMED);
            if grew.is_empty() {
                "no crate's symbols grew: what grew is in no symbol (headers, unwind tables, \
                 padding)"
                    .to_owned()
            } else {
                let named: Vec<String> = grew
                    .iter()
                    .map(|(name, bytes)| format!("`{name}` {:+.1} KB", kibibytes(*bytes)))
                    .collect();
                format!("the crates that grew most: {}", named.join(", "))
            }
        }
        (None, _) => "the crates that grew are named by the Linux job, whose symbols can be \
                      read (`--breakdown`)"
            .to_owned(),
        (Some(_), None) => "`main`'s record has no bytes by crate (its size job made no \
                            breakdown), so the crates that grew cannot be named"
            .to_owned(),
    };
    Some(format!(
        "It grew by {:.1} KB, more than {} KB: {said}.",
        kibibytes(growth),
        GROWTH_TO_EXPLAIN / 1024
    ))
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
    // A record by crate is there when the run made a breakdown; a broken
    // one is said and taken for none.
    let crates = match fs::read_to_string(directory.join(crates_record_name(target))) {
        Ok(text) => match parse_crates_record(&text) {
            Ok(crates) => Some(crates),
            Err(error) => {
                println!("  main's record by crate is not read: {error}");
                None
            }
        },
        Err(_) => None,
    };
    Ok(MainSize {
        bytes,
        run,
        commit,
        crates,
    })
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
fn report(
    target: &str,
    profile: &str,
    bytes: u64,
    crates: usize,
    main: Option<&Result<MainSize>>,
) -> Report {
    let limit = budget(target);
    let (megabytes, limit_megabytes) = (mebibytes(bytes), mebibytes(limit));
    // The budgets are of the shipped build.
    let shipped = profile == PROFILE;
    let room = if !shipped {
        "no budget".to_owned()
    } else if bytes <= limit {
        format!("{:.2} MB left", mebibytes(limit - bytes))
    } else {
        format!("**{:.2} MB over**", mebibytes(bytes - limit))
    };
    let budget_cell = if shipped {
        format!("{limit_megabytes:.0} MB")
    } else {
        "-".to_owned()
    };
    let mut summary = format!(
        "### Binary size and crate count\n\n\
         | Target | Profile | Bytes | MB | Budget | Room | Crates |\n\
         |---|---|---:|---:|---:|---|---:|\n\
         | `{target}` | `{profile}` | {bytes} | {megabytes:.2} | {budget_cell} | {room} | {crates} |\n\n"
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
    let over = (shipped && bytes > limit).then(|| {
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

const SIZE_USAGE: &str = "\
usage: cargo xtask size [--check] [--target TRIPLE] [--against-main]
                        [--profile release|dist] [--breakdown [--top N]] [--at COMMIT]

Builds `cineris` and reports its size and the crates in the build.
  --check          fail over the budget too (a home path fails every run)
  --target TRIPLE  the target to build for (the host's when not given; the
                   budgets are of x86_64-unknown-linux-musl and
                   x86_64-pc-windows-msvc)
  --against-main   the difference from the size `main` last recorded (dist only)
  --base HASH      with --against-main: the pull request's base, and a line
                   saying so when main's record is of another commit
  --profile P      `dist` (the shipped binary, and the default) or `release`
                   (thin LTO, stripped as dist is, a fraction of the build time:
                   not the shipped size, so it has no budget and no `main` to
                   compare with)
  --breakdown      the bytes of the binary by crate, from the symbols of an
                   unstripped build of the same code (Linux targets; builds twice)
  --top N          how many crates the breakdown lists (25 when not given)
  --at COMMIT      build the tree of that commit (read from this repository
                   with git archive, extracted into the build directory) in
                   place of this one";

/// What `cargo xtask size` was asked for.
#[derive(Debug, PartialEq)]
pub(crate) struct SizeRequest {
    check: bool,
    against_main: bool,
    target: Option<String>,
    profile: String,
    breakdown: bool,
    /// `--record-crates`: make the breakdown for the record by crate (and
    /// the explanation of growth) without printing its table, and say a
    /// failure of it rather than fail: the `main` workflow's Linux size job.
    record_crates: bool,
    top: usize,
    at: Option<String>,
    /// The commit the difference from `main` should be from: a pull
    /// request's base. `main`'s record can be of another.
    base: Option<String>,
}

/// What to say when the record `main`'s size was read from is of a commit
/// other than `base`, the pull request's base: the difference then holds
/// what merged between the two, or misses it. Nothing when they are one
/// commit, or when the record names none (it is said that it names none).
fn base_note(record: Option<&str>, base: &str) -> Option<String> {
    let short: String = base.chars().take(7).collect();
    match record {
        Some(record) if base.starts_with(record) || record.starts_with(&short) => None,
        Some(record) => Some(format!(
            "The record is of `{record}`, not of this pull request's base `{short}`: the \
             difference holds what changed between the two."
        )),
        None => Some(format!(
            "The record names no commit, so whether it is of this pull request's base `{short}` \
             is not known."
        )),
    }
}

/// Whether `text` can be given to git as a commit's name: no option, no
/// space, none of the characters a revision's syntax does something with
/// beyond a branch, a tag, a hash and `~` or `^` of them.
fn commit_name(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('-')
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-./^~".contains(&byte))
}

impl SizeRequest {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = SizeRequest {
            check: false,
            against_main: false,
            target: None,
            profile: PROFILE.to_owned(),
            breakdown: false,
            record_crates: false,
            top: 25,
            at: None,
            base: None,
        };
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = |what: &str| {
                words
                    .next()
                    .ok_or_else(|| format!("`{word}` needs {what}\n\n{SIZE_USAGE}"))
            };
            match word.as_str() {
                "--check" => request.check = true,
                "--against-main" => request.against_main = true,
                "--breakdown" => request.breakdown = true,
                "--record-crates" => request.record_crates = true,
                "--target" => request.target = Some(value("a target triple")?.clone()),
                "--profile" => {
                    let profile = value("release or dist")?;
                    if !["release", "dist"].contains(&profile.as_str()) {
                        return Err(format!("`--profile {profile}`: release or dist"));
                    }
                    request.profile.clone_from(profile);
                }
                "--top" => {
                    let text = value("a number")?;
                    request.top = text
                        .parse()
                        .ok()
                        .filter(|top| *top > 0)
                        .ok_or_else(|| format!("`--top {text}` is not a number above zero"))?;
                }
                "--at" => {
                    let commit = value("a commit")?;
                    if !commit_name(commit) {
                        return Err(format!(
                            "`--at {commit}` is not a commit's name: a hash, a branch or a tag \
                             (letters, digits and `_-./^~` only)"
                        ));
                    }
                    request.at = Some(commit.clone());
                }
                "--base" => {
                    let commit = value("a commit's hash")?;
                    if commit.len() < 7 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        return Err(format!(
                            "`--base {commit}` is not a commit's hash (seven or more hexadecimal \
                             digits)"
                        ));
                    }
                    request.base = Some(commit.to_ascii_lowercase());
                }
                _ => return Err(SIZE_USAGE.to_owned()),
            }
        }
        if request.base.is_some() && !request.against_main {
            return Err(
                "`--base` says what `--against-main`'s record should be of: give both".to_owned(),
            );
        }
        if request.profile != PROFILE {
            for (given, flag, why) in [
                (
                    request.check,
                    "--check",
                    "the budgets are of the shipped binary, the `dist` build",
                ),
                (
                    request.against_main,
                    "--against-main",
                    "`main` records the `dist` build's size",
                ),
            ] {
                if given {
                    return Err(format!(
                        "`{flag}` does not go with `--profile {}`: {why}",
                        request.profile
                    ));
                }
            }
        }
        Ok(request)
    }

    /// Refuses a breakdown of a target it cannot read: it reads the symbols of
    /// an ELF binary with `nm`.
    fn check_breakdown(&self, target: &str) -> Result<()> {
        if self.breakdown && !target.contains("linux") {
            return Err(format!(
                "`--breakdown` reads the symbols of an unstripped ELF binary with `nm`, and is \
                 for the Linux targets: the `{target}` binary keeps its symbols in a separate \
                 file (a PDB), which this does not read. The same code is about 1.15 to 1.2 times \
                 the Linux binary's size on Windows: break that one down"
            ));
        }
        Ok(())
    }
}

/// Whether `cargo xtask size` would take `arguments` on a runner whose
/// system is Linux (`linux`) or not, which it reads and runs nothing: how
/// `try` refuses what it cannot pass on before it builds. A runner builds for
/// its own system.
pub(crate) fn check_arguments(arguments: &[String], linux: bool) -> Result<()> {
    let request = SizeRequest::parse(arguments)?;
    if request.at.is_some() {
        return Err(
            "`--at` reads another commit from the repository's history, which a runner's \
             checkout (one commit deep) does not have: use the `commit` input to run the \
             size of any commit"
                .to_owned(),
        );
    }
    let target = request.target.clone().unwrap_or_else(|| {
        if linux {
            LINUX_TARGET.to_owned()
        } else {
            "x86_64-pc-windows-msvc".to_owned()
        }
    });
    if target.contains("windows") == linux {
        return Err(format!(
            "`--target {target}` is not a target this runner builds for: a runner builds for its \
             own system, and `os` chooses it"
        ));
    }
    request.check_breakdown(&target)
}

/// The Linux target of the budgets: the static one.
pub(crate) const LINUX_TARGET: &str = "x86_64-unknown-linux-musl";

/// A commit's tree, extracted from this repository's history into a folder of
/// the build directory, and removed when this is dropped.
struct Archive {
    tree: PathBuf,
    hash: String,
}

impl Archive {
    /// Extracts the tree of `commit`.
    fn make(commit: &str) -> Result<Self> {
        let root = workspace::root();
        let hash = git_in(
            &root,
            &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
        )
        .map_err(|error| {
            format!(
                "`--at {commit}`: {error}. It reads the commit with git, in this repository: \
                     a copy with no `.git` cannot, and a commit not fetched is not here"
            )
        })?;
        let directory = workspace::metadata(&root)?
            .target_directory
            .join("size-at")
            .join(hash.chars().take(12).collect::<String>());
        if directory.exists() {
            fs::remove_dir_all(&directory)
                .map_err(|error| format!("could not clear {}: {error}", directory.display()))?;
        }
        fs::create_dir_all(&directory)
            .map_err(|error| format!("could not make {}: {error}", directory.display()))?;
        let archive = Self {
            tree: directory,
            hash,
        };
        extract(&root, &archive.hash, &archive.tree)?;
        Ok(archive)
    }
}

impl Drop for Archive {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.tree);
    }
}

/// What git prints for `arguments` in `directory`, trimmed.
fn git_in(directory: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The tree of `hash`, in the repository at `repository`, into `into`, with
/// every file dated now. `git archive` dates each file as its commit, and a
/// build directory that holds artifacts of a later build takes a file that
/// old for unchanged: cargo's fingerprints compare modification times
/// (`tar -m` leaves them as the extraction makes them).
fn extract(repository: &Path, hash: &str, into: &Path) -> Result<()> {
    // The commit's bytes, whatever the machine's git says of line endings
    // (Git for Windows turns `core.autocrlf` on for every repository).
    let mut archive = Command::new("git")
        .args(["-c", "core.autocrlf=false", "archive", "--format=tar", hash])
        .current_dir(repository)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run git archive: {error}"))?;
    let tar = Command::new("tar")
        .args(["-x", "-m", "-f", "-", "-C"])
        .arg(into)
        .stdin(archive.stdout.take().ok_or("git archive gave no output")?)
        .status()
        .map_err(|error| format!("could not run tar: {error}"))?;
    let archived = archive
        .wait()
        .map_err(|error| format!("could not wait for git archive: {error}"))?;
    if archived.success() && tar.success() {
        Ok(())
    } else {
        Err(format!(
            "could not extract {hash}: git archive {archived}, tar {tar}"
        ))
    }
}

/// `cargo xtask size [--check] [--target TRIPLE] [--against-main]
/// [--profile release|dist] [--breakdown [--top N]] [--at COMMIT]`
pub(crate) fn size(arguments: &[String]) -> Result<()> {
    let request = SizeRequest::parse(arguments)?;
    let target = match &request.target {
        Some(target) => target.clone(),
        None => host_target()?,
    };
    request.check_breakdown(&target)?;

    // This tree, or the tree of a commit, extracted and built in a folder of
    // its own.
    let archive = request.at.as_deref().map(Archive::make).transpose()?;
    let tree = archive
        .as_ref()
        .map_or_else(workspace::root, |archive| archive.tree.clone());
    let own_target = archive.as_ref().map(|archive| archive.tree.join("target"));
    let target_directory = match &own_target {
        Some(directory) => directory.clone(),
        None => workspace::metadata(&tree)?.target_directory,
    };
    let stripping = stripped_as_shipped(&request.profile);
    let variables: Vec<(&str, &str)> = stripping
        .iter()
        .map(|(name, value)| (name.as_str(), *value))
        .collect();
    shipped::build_with(&shipped::Options {
        profile: &request.profile,
        target: Some(&target),
        tree: archive.as_ref().map(|archive| archive.tree.as_path()),
        target_dir: own_target.as_deref(),
        variables: &variables,
    })?;

    let binary = binary_in(&target_directory, &target, &request.profile);
    let bytes = fs::metadata(&binary)
        .map_err(|error| format!("could not read {}: {error}", binary.display()))?
        .len();
    let crates = crates_in_build(&tree, &target)?.len();
    let limit = budget(&target);
    if request.profile == PROFILE {
        println!(
            "\n{target}, `{PROFILE}` profile\n  binary   {bytes} bytes ({:.2} MB; budget {:.0} MB)\n  crates   {crates}",
            mebibytes(bytes),
            mebibytes(limit)
        );
    } else {
        println!(
            "\n{target}, `{}` profile (not the shipped build: no budget)\n  binary   {bytes} bytes ({:.2} MB)\n  crates   {crates}",
            request.profile,
            mebibytes(bytes)
        );
    }
    if let Some(archive) = &archive {
        println!("  commit   {}", archive.hash);
    }
    let main = request.against_main.then(|| main_size(&target));
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
    let mut report = report(&target, &request.profile, bytes, crates, main.as_ref());
    if let (Some(Ok(main)), Some(base)) = (&main, &request.base)
        && let Some(note) = base_note(main.commit.as_deref(), base)
    {
        println!("  {note}");
        report.summary.push_str(&format!("{note}\n\n"));
    }
    if let Some(archive) = &archive {
        report
            .summary
            .push_str(&format!("Built from commit {}.\n\n", archive.hash));
    }
    steps::append_job_summary(&report.summary);
    if let Err(message) = &paths {
        steps::append_job_summary(&format!("**Home paths:** {message}\n\n"));
    }

    // The record, for the workflow to keep as an artifact: of this tree's own
    // build in the shipped profile (the commit it names is the workflow's).
    if archive.is_none() {
        let record = serde_json::json!({
            "target": target,
            "profile": request.profile,
            "bytes": bytes,
            "budget_bytes": limit,
            "crates": crates,
            "commit": std::env::var("GITHUB_SHA").ok(),
        });
        let directory = target_directory.join("size");
        fs::create_dir_all(&directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
        let file = directory.join(format!("{target}.json"));
        fs::write(&file, format!("{record:#}\n"))
            .map_err(|error| format!("could not write {}: {error}", file.display()))?;
    }

    // Last: it builds again, and its failure must not hide the verdict. It
    // runs when it is asked for, and on Linux when the binary grew past
    // what is explained by name against `main`'s record.
    let growth = match &main {
        Some(Ok(main)) => Some(i128::from(bytes) - i128::from(main.bytes)),
        _ => None,
    };
    let readable = target.contains("linux");
    let explain = growth.is_some_and(|growth| growth > GROWTH_TO_EXPLAIN);
    let sizes = (request.breakdown || ((request.record_crates || explain) && readable))
        .then(|| breakdown(&tree, &target_directory, &target, &request, bytes));
    let sizes = match (sizes, archive.is_none()) {
        (Some(Ok(sizes)), true) => {
            let file = target_directory
                .join("size")
                .join(crates_record_name(&target));
            Some(
                fs::write(&file, crates_record(&sizes))
                    .map(|()| sizes)
                    .map_err(|error| format!("could not write {}: {error}", file.display())),
            )
        }
        (sizes, _) => sizes,
    };
    if let (Some(growth), Some(Ok(main))) = (growth, &main) {
        let now = match &sizes {
            Some(Ok(sizes)) => Some(sizes.as_slice()),
            _ => None,
        };
        if let Some(text) = growth_text(growth, now, main.crates.as_deref()) {
            println!("  {text}");
            steps::append_job_summary(&format!("{text}\n\n"));
        }
    }
    verdict(paths, report.over, request.check)?;
    breakdown_outcome(
        sizes.as_ref().map(|sizes| sizes.as_ref().map(|_| ())),
        request.breakdown,
    )
}

/// How the bytes by crate end `size`: asked for by name (`--breakdown`),
/// their failure is the command's; made for the record (`--record-crates`)
/// or to explain growth, it is said and fails nothing.
fn breakdown_outcome(
    made: Option<std::result::Result<(), &String>>,
    asked_by_name: bool,
) -> Result<()> {
    match made {
        Some(Err(error)) if asked_by_name => Err(error.clone()),
        Some(Err(error)) => {
            println!("  the bytes by crate were not read: {error}");
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Where cargo puts the binary of `profile` for `target`.
fn binary_in(target_directory: &Path, target: &str, profile: &str) -> PathBuf {
    let executable = if target.contains("windows") {
        "cineris.exe"
    } else {
        "cineris"
    };
    target_directory.join(target).join(profile).join(executable)
}

/// How `size` ends: a home path fails every run, measured or checked, as it
/// is never right; a binary over its budget fails a run with `--check`.
fn verdict(paths: Result<()>, over: Option<String>, check: bool) -> Result<()> {
    paths?;
    match over {
        Some(over) if check => Err(over),
        _ => Ok(()),
    }
}

/// What `--breakdown` does: builds the same code again with its symbols kept
/// (in a build directory of its own, so that neither build rebuilds the
/// other), reads them with `nm`, and prints the bytes by crate.
///
/// The stripped binary is the measured one; the unstripped one differs only
/// by its symbol table, which is not loaded. A symbol's size is the bytes of
/// its code or data, and what is in no symbol (the headers, the unwind
/// tables, what the linker pads) is the last row: `bytes` less the symbols'.
fn breakdown(
    tree: &Path,
    target_directory: &Path,
    target: &str,
    request: &SizeRequest,
    bytes: u64,
) -> Result<Vec<(String, u64)>> {
    let own = target_directory.join("size-breakdown");
    let variable = strip_variable(&request.profile);
    shipped::build_with(&shipped::Options {
        profile: &request.profile,
        target: Some(target),
        tree: Some(tree),
        target_dir: Some(&own),
        variables: &[(variable.as_str(), "none")],
    })?;
    let binary = binary_in(&own, target, &request.profile);
    let output = Command::new("nm")
        .args(["--print-size", "--demangle", "--radix=d", "--defined-only"])
        .arg(&binary)
        .output()
        .map_err(|error| {
            format!(
                "could not run nm: {error}. `--breakdown` reads symbols with `nm`, which is in \
                 binutils"
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "nm failed on {}: {}",
            binary.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let sizes = crate_sizes(&parse_nm(&String::from_utf8_lossy(&output.stdout)));
    if request.breakdown {
        let table = breakdown_table(&sizes, bytes, request.top);
        println!("\n{table}");
        steps::append_job_summary(&format!(
            "### Binary size by crate\n\n```text\n{table}```\n"
        ));
    }
    Ok(sizes)
}

/// The environment variable that sets a profile's `strip`.
fn strip_variable(profile: &str) -> String {
    format!("CARGO_PROFILE_{}_STRIP", profile.to_uppercase())
}

/// What a build of `profile` is given so that it is stripped, as `dist` is
/// (`strip = true` in the manifest): `release` is not, and its size would
/// be mostly the symbol table's. Nothing for the shipped profile.
fn stripped_as_shipped(profile: &str) -> Vec<(String, &'static str)> {
    if profile == PROFILE {
        Vec::new()
    } else {
        vec![(strip_variable(profile), "symbols")]
    }
}

/// A symbol of the binary: its size in bytes and its demangled name.
#[derive(Debug, PartialEq, Eq)]
struct Symbol {
    size: u64,
    name: String,
}

/// The symbols of `nm --print-size --demangle --radix=d` output that take
/// room in the file: code and data with a size, not the uninitialised data
/// (`bss`), absolute or other symbols that hold no bytes. Names at the same
/// address count once (the linker may give one body several names).
fn parse_nm(text: &str) -> Vec<Symbol> {
    let mut seen = BTreeSet::new();
    let mut symbols = Vec::new();
    for line in text.lines() {
        let mut parts = line.splitn(4, ' ');
        let (Some(address), Some(size), Some(kind), Some(name)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(address), Ok(size)) = (address.parse::<u64>(), size.parse::<u64>()) else {
            continue;
        };
        if size == 0 || !"tTdDrRwWvVgGsSiI".contains(kind) || kind.len() != 1 {
            continue;
        }
        if seen.insert(address) {
            symbols.push(Symbol {
                size,
                name: name.to_owned(),
            });
        }
    }
    symbols
}

/// What a symbol with no crate in its name is counted as.
const NO_CRATE: &str = "[C, assembly and libc]";
/// What data with no name of its own (a string, a table) is counted as.
const ANONYMOUS: &str = "[strings and tables with no name]";
/// The standard library's crates, counted as one.
const STD: &str = "std (core, alloc, std)";

/// The crate a demangled symbol belongs to: the first path of its name, which
/// for a method of a trait is the type's (`<serde_json::Value as
/// Debug>::fmt` is `serde_json`'s) and for generic code is the generic's own
/// (`core::ptr::drop_in_place<cineris_core::X>` is the standard library's, the
/// way it is for `cargo bloat`). A symbol of no Rust crate is C or assembly.
fn crate_of(name: &str) -> String {
    const PRIMITIVES: [&str; 19] = [
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        "f32", "f64", "bool", "char", "str", "dyn", "fn",
    ];
    if name.starts_with("anon.") || name.starts_with(".L") || name.starts_with("str.") {
        return ANONYMOUS.to_owned();
    }
    let path = name.trim_start_matches(['<', '&', '*', '(', ' ']);
    let path = ["const ", "mut ", "dyn "]
        .iter()
        .fold(path, |path, word| path.strip_prefix(word).unwrap_or(path));
    if path.starts_with('[') {
        return STD.to_owned();
    }
    let word: String = path
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let after = &path[word.len()..];
    if word.is_empty() || !after.starts_with("::") {
        // `u32 as core::fmt::Debug`, `str::len`: no crate of its own.
        if PRIMITIVES.contains(&word.as_str())
            && (after.starts_with(" as ") || after.starts_with('>'))
        {
            return STD.to_owned();
        }
        return NO_CRATE.to_owned();
    }
    if PRIMITIVES.contains(&word.as_str()) {
        return STD.to_owned();
    }
    match word.as_str() {
        "core" | "alloc" | "std" | "std_detect" | "panic_unwind" | "unwind" => STD.to_owned(),
        _ => word,
    }
}

/// The bytes of each crate, largest first.
fn crate_sizes(symbols: &[Symbol]) -> Vec<(String, u64)> {
    let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
    for symbol in symbols {
        *sizes.entry(crate_of(&symbol.name)).or_default() += symbol.size;
    }
    let mut sizes: Vec<(String, u64)> = sizes.into_iter().collect();
    sizes.sort_by(|one, other| other.1.cmp(&one.1).then_with(|| one.0.cmp(&other.0)));
    sizes
}

/// The breakdown as text: the `top` largest crates with their bytes and share
/// of the binary's `total`, the rest of the crates in one row, and what no
/// symbol holds.
fn breakdown_table(sizes: &[(String, u64)], total: u64, top: usize) -> String {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a binary's size in bytes is far below 2^52"
    )]
    let share = |bytes: u64| bytes as f64 / total.max(1) as f64 * 100.0;
    let symbols: u64 = sizes.iter().map(|(_, bytes)| bytes).sum();
    let mut rows: Vec<(String, u64)> = sizes.iter().take(top).cloned().collect();
    let rest = &sizes[rows.len()..];
    if !rest.is_empty() {
        rows.push((
            format!("{} more crates", rest.len()),
            rest.iter().map(|(_, bytes)| bytes).sum(),
        ));
    }
    rows.push((
        "[in no symbol: headers, unwind tables, padding]".to_owned(),
        total.saturating_sub(symbols),
    ));
    let width = rows
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or(0);
    let mut text = format!(
        "The binary's {total} bytes by crate, from the symbols of an unstripped build of the \
         same code ({} crates; generic code counts for the crate that owns the generic):\n\n",
        sizes.len()
    );
    for (name, bytes) in rows {
        text.push_str(&format!(
            "{name:<width$}  {bytes:>10}  {:>5.1}%\n",
            share(bytes)
        ));
    }
    if symbols > total {
        text.push_str(
            "\nThe symbols add up to more than the binary: code the linker folded or dropped \
             is still named, so the shares are a guide to where the code is, not to the byte.\n",
        );
    }
    text
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
mod base_tests {
    use super::base_note;

    #[test]
    fn a_record_of_another_commit_than_the_base_is_said_and_of_the_base_is_not() {
        let base = "e73ef24abcdef0123456789abcdef0123456789a";
        assert_eq!(base_note(Some("e73ef24"), base), None);
        let other = base_note(Some("00e5482"), base).unwrap();
        assert!(
            other.contains("`00e5482`, not of this pull request's base `e73ef24`"),
            "{other}"
        );
        assert!(base_note(None, base).unwrap().contains("names no commit"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(list: &[(&str, u64)]) -> Vec<(String, u64)> {
        list.iter()
            .map(|(name, bytes)| ((*name).to_owned(), *bytes))
            .collect()
    }

    #[test]
    fn the_crates_that_grew_most_are_named_largest_first_and_a_new_one_whole() {
        let main = sizes(&[
            ("cineris_core", 500_000),
            ("serde", 100_000),
            ("regex", 80_000),
        ]);
        let now = sizes(&[
            ("cineris_core", 650_000),
            ("serde", 90_000),
            ("regex", 81_000),
            ("syntect", 120_000),
        ]);
        assert_eq!(
            grew_most(&now, &main, 3),
            [
                ("cineris_core".to_owned(), 150_000),
                ("syntect".to_owned(), 120_000),
                ("regex".to_owned(), 1_000),
            ]
        );
        assert_eq!(grew_most(&now, &main, 1).len(), 1);
        // A crate that shrank is not one that grew.
        assert!(
            grew_most(&main, &now, 3)
                .iter()
                .all(|(name, _)| name == "serde")
        );
    }

    #[test]
    fn growth_past_100_kb_is_explained_by_crate_and_less_is_not() {
        let main = sizes(&[("cineris_core", 500_000)]);
        let now = sizes(&[("cineris_core", 650_000)]);
        assert_eq!(growth_text(100 * 1024, Some(&now), Some(&main)), None);
        let said = growth_text(150_000, Some(&now), Some(&main)).unwrap();
        assert!(said.contains("more than 100 KB"), "{said}");
        assert!(said.contains("`cineris_core` +146.5 KB"), "{said}");
        // Windows reads no symbols, and an older record of main's has none.
        let windows = growth_text(150_000, None, Some(&main)).unwrap();
        assert!(windows.contains("named by the Linux job"), "{windows}");
        let older = growth_text(150_000, Some(&now), None).unwrap();
        assert!(older.contains("has no bytes by crate"), "{older}");
        // Growth no symbol holds.
        let same = growth_text(150_000, Some(&main), Some(&main)).unwrap();
        assert!(same.contains("in no symbol"), "{same}");
    }

    #[test]
    fn a_breakdown_fails_size_only_when_it_was_asked_for_by_name() {
        let failed = "nm failed".to_owned();
        assert_eq!(
            breakdown_outcome(Some(Err(&failed)), true),
            Err(failed.clone())
        );
        assert_eq!(breakdown_outcome(Some(Err(&failed)), false), Ok(()));
        assert_eq!(breakdown_outcome(Some(Ok(())), true), Ok(()));
        assert_eq!(breakdown_outcome(None, false), Ok(()));
    }

    #[test]
    fn the_record_by_crate_is_asked_for_apart_from_the_breakdown() {
        let record = SizeRequest::parse(&strings(&["--record-crates"])).unwrap();
        assert!(record.record_crates && !record.breakdown);
        let both = SizeRequest::parse(&strings(&["--breakdown", "--record-crates"])).unwrap();
        assert!(both.record_crates && both.breakdown);
    }

    #[test]
    fn a_record_by_crate_is_read_back_as_it_was_written() {
        let written = sizes(&[("a", 1), ("b", 2)]);
        let mut read = parse_crates_record(&crates_record(&written)).unwrap();
        read.sort();
        assert_eq!(read, written);
        assert!(parse_crates_record("[1]").is_err());
        assert!(parse_crates_record("{\"a\": \"x\"}").is_err());
    }

    #[test]
    fn each_crate_in_the_tree_is_counted_once() {
        let tree = "\
cineris-cli v0.0.0 (/repo/crates/cineris-cli)
cineris-core v0.0.0 (/repo/crates/cineris-core)
cineris-protocol v0.0.0 (/repo/crates/cineris-protocol)
serde v1.0.229
serde_core v1.0.229
serde_derive v1.0.229 (proc-macro)
serde_json v1.0.154
serde_core v1.0.229
cineris-protocol v0.0.0 (/repo/crates/cineris-protocol) (*)

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
        let report = report(LINUX, PROFILE, 12_000_000, 180, None);
        assert_eq!(report.over, None);
        assert!(
            report
                .summary
                .contains("| `x86_64-unknown-linux-musl` | `dist` | 12000000 | 11.44 | 20 MB | 8.56 MB left | 180 |"),
            "{}",
            report.summary
        );
        assert!(!report.summary.contains("main"), "{}", report.summary);
    }

    #[test]
    fn a_binary_over_its_budget_fails_with_a_line_that_says_by_how_much() {
        let limit = budget(WINDOWS);
        let report = report(WINDOWS, PROFILE, limit + 351_744, 180, None);
        let over = report.over.expect("over the budget");
        assert!(
            over.starts_with("the x86_64-pc-windows-msvc binary is 26566144 bytes (25.34 MB), over its budget of 25 MB by 351744 bytes"),
            "{over}"
        );
        assert!(
            report.summary.contains("**0.34 MB over**"),
            "{}",
            report.summary
        );
        assert!(report.summary.contains(&format!("**Over budget:** {over}")));
        // One byte at the budget is within it.
        assert_eq!(super::report(WINDOWS, PROFILE, limit, 180, None).over, None);
    }

    #[test]
    fn the_difference_from_mains_last_size_is_signed() {
        let main = Ok(MainSize {
            bytes: 12_100_000,
            run: 43_811_142_390,
            commit: Some("c225d64".to_owned()),
            crates: None,
        });
        let smaller = report(LINUX, PROFILE, 12_000_000, 180, Some(&main)).summary;
        assert!(
            smaller.contains(
                "12100000 bytes (run 43811142390 at c225d64): **-100000 bytes** (-97.7 KB)"
            ),
            "{smaller}"
        );
        let larger = report(LINUX, PROFILE, 12_102_048, 180, Some(&main)).summary;
        assert!(larger.contains("**+2048 bytes** (+2.0 KB)"), "{larger}");
        // A record that could not be read is said, and fails nothing.
        let unread = Err("gh: not signed in".to_owned());
        let report = report(LINUX, PROFILE, 12_000_000, 180, Some(&unread));
        assert!(
            report
                .summary
                .contains("could not be read: gh: not signed in")
        );
        assert_eq!(report.over, None);
    }

    #[test]
    fn a_home_path_fails_every_run_and_the_budget_only_a_check() {
        let paths = || Err::<(), _>("holds paths of the machine".to_owned());
        let over = || Some("over its budget".to_owned());
        assert!(verdict(paths(), None, false).is_err());
        assert!(verdict(paths(), None, true).is_err());
        assert_eq!(
            verdict(Ok(()), over(), true),
            Err("over its budget".to_owned())
        );
        assert_eq!(verdict(Ok(()), over(), false), Ok(()));
        assert_eq!(verdict(Ok(()), None, true), Ok(()));
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
        assert_eq!(budget("x86_64-unknown-linux-musl"), 20 * 1024 * 1024);
        assert_eq!(budget("aarch64-unknown-linux-musl"), 20 * 1024 * 1024);
        assert_eq!(budget("x86_64-pc-windows-msvc"), 25 * 1024 * 1024);
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn a_size_request_is_read_and_what_makes_no_sense_is_refused() {
        let all = SizeRequest::parse(&strings(&[
            "--check",
            "--against-main",
            "--target",
            LINUX,
            "--breakdown",
            "--top",
            "5",
            "--at",
            "origin/main~2",
        ]))
        .unwrap();
        assert_eq!(
            all,
            SizeRequest {
                check: true,
                against_main: true,
                target: Some(LINUX.to_owned()),
                profile: "dist".to_owned(),
                breakdown: true,
                record_crates: false,
                top: 5,
                at: Some("origin/main~2".to_owned()),
                base: None,
            }
        );
        assert_eq!(
            SizeRequest::parse(&strings(&["--against-main", "--base", "E73EF24AB"]))
                .unwrap()
                .base
                .as_deref(),
            Some("e73ef24ab")
        );
        for bad in [
            &["--base", "e73ef24"][..],
            &["--against-main", "--base", "e73ef2"][..],
            &["--against-main", "--base", "main"][..],
            &["--against-main", "--base"][..],
        ] {
            assert!(SizeRequest::parse(&strings(bad)).is_err(), "{bad:?}");
        }
        assert_eq!(
            SizeRequest::parse(&strings(&["--profile", "release"]))
                .unwrap()
                .profile,
            "release"
        );
        for (words, expected) in [
            (&["--profile", "ci"][..], "release or dist"),
            (&["--profile"], "needs release or dist"),
            (
                &["--check", "--profile", "release"],
                "the budgets are of the shipped binary",
            ),
            (
                &["--against-main", "--profile", "release"],
                "records the `dist` build",
            ),
            (&["--top", "0"], "above zero"),
            (&["--at", "-x"], "not a commit's name"),
            (&["--at", "a b"], "not a commit's name"),
            (&["--at", "main; rm"], "not a commit's name"),
            (&["--at", "main@{1}"], "not a commit's name"),
            (&["--nope"], "usage: cargo xtask size"),
        ] {
            let error = SizeRequest::parse(&strings(words)).unwrap_err();
            assert!(error.contains(expected), "{words:?}: {error}");
        }
    }

    #[test]
    fn a_runner_builds_for_its_own_system_and_cannot_break_a_windows_binary_down() {
        assert_eq!(check_arguments(&strings(&["--check"]), true), Ok(()));
        assert_eq!(check_arguments(&strings(&["--breakdown"]), true), Ok(()));
        assert_eq!(
            check_arguments(&strings(&["--target", LINUX, "--breakdown"]), true),
            Ok(())
        );
        let windows = check_arguments(&strings(&["--breakdown"]), false).unwrap_err();
        assert!(windows.contains("for the Linux targets"), "{windows}");
        assert_eq!(check_arguments(&strings(&["--check"]), false), Ok(()));
        let foreign = check_arguments(&strings(&["--target", WINDOWS]), true).unwrap_err();
        assert!(
            foreign.contains("not a target this runner builds for"),
            "{foreign}"
        );
        let foreign = check_arguments(&strings(&["--target", LINUX]), false).unwrap_err();
        assert!(
            foreign.contains("not a target this runner builds for"),
            "{foreign}"
        );
        let at = check_arguments(&strings(&["--at", "abc"]), true).unwrap_err();
        assert!(at.contains("use the `commit` input"), "{at}");
    }

    #[test]
    fn a_build_that_is_not_the_shipped_one_has_no_budget_and_is_not_over_it() {
        let report = report(WINDOWS, "release", budget(WINDOWS) + 1, 180, None);
        assert_eq!(report.over, None);
        assert!(
            report.summary.contains(
                "| `x86_64-pc-windows-msvc` | `release` | 26214401 | 25.00 | - | no budget | 180 |"
            ),
            "{}",
            report.summary
        );
    }

    #[test]
    fn the_strip_setting_of_a_profile_is_turned_off_by_its_variable() {
        assert_eq!(strip_variable("dist"), "CARGO_PROFILE_DIST_STRIP");
        assert_eq!(strip_variable("release"), "CARGO_PROFILE_RELEASE_STRIP");
    }

    #[test]
    fn a_release_build_is_stripped_as_the_shipped_one_is_and_dist_is_left_alone() {
        assert_eq!(stripped_as_shipped("dist"), Vec::<(String, &str)>::new());
        assert_eq!(
            stripped_as_shipped("release"),
            [("CARGO_PROFILE_RELEASE_STRIP".to_owned(), "symbols")]
        );
    }

    /// `nm --print-size --demangle --radix=d --defined-only`, as GNU's prints
    /// it: address, size, a letter for the kind, the name (which may hold
    /// spaces); a symbol with no size has two fields before the name.
    const NM: &str = "\
0000000000004096 0000000000000200 T serde_json::de::from_slice
0000000000004608 0000000000000040 T <serde_json::value::Value as core::fmt::Debug>::fmt
0000000000004700 0000000000000100 t core::fmt::write
0000000000004700 0000000000000100 T core::fmt::write_alias
0000000000008192 0000000000000300 r anon.0123abcd.4
0000000000008500 0000000000000030 R cineris_core::table
0000000000009000 0000000000000050 T memcpy
0000000000009100 0000000000000900 B cineris_core::bss_is_not_in_the_file
0000000000009200 0000000000000000 T zero_sized
0000000000009300 W _start
0000000000009400 0000000000000020 W <u32 as core::fmt::Display>::fmt
";

    #[test]
    fn the_symbols_that_take_room_in_the_file_are_read_and_an_address_counts_once() {
        let symbols = parse_nm(NM);
        let names: Vec<&str> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "serde_json::de::from_slice",
                "<serde_json::value::Value as core::fmt::Debug>::fmt",
                "core::fmt::write",
                "anon.0123abcd.4",
                "cineris_core::table",
                "memcpy",
                "<u32 as core::fmt::Display>::fmt",
            ],
            "bss, an empty symbol, one with no size and the second name of an address are left out"
        );
        assert_eq!(symbols[1].size, 40);
    }

    #[test]
    fn a_symbol_belongs_to_the_crate_whose_path_it_starts_with() {
        for (name, expected) in [
            ("serde_json::de::from_slice", "serde_json"),
            (
                "<serde_json::value::Value as core::fmt::Debug>::fmt",
                "serde_json",
            ),
            (
                "<&cineris_core::X as core::fmt::Debug>::fmt",
                "cineris_core",
            ),
            ("core::ptr::drop_in_place<cineris_core::X>", STD),
            ("alloc::vec::Vec<T>::push", STD),
            ("std::io::stdio::print", STD),
            (
                "<alloc::string::String as core::fmt::Write>::write_str",
                STD,
            ),
            ("<[u8]>::to_vec", STD),
            ("<u32 as core::fmt::Display>::fmt", STD),
            ("<str>::len", STD),
            ("str::from_utf8", STD),
            ("<dyn core::any::Any>::type_id", STD),
            ("<*const u8 as core::fmt::Debug>::fmt", STD),
            ("tokio::runtime::task::raw::poll", "tokio"),
            ("memcpy", NO_CRATE),
            ("main", NO_CRATE),
            ("anon.0123abcd.4", ANONYMOUS),
            (".L__unnamed_1", ANONYMOUS),
            ("str.0", ANONYMOUS),
        ] {
            assert_eq!(crate_of(name), expected, "{name}");
        }
    }

    #[test]
    fn the_crates_are_summed_and_the_largest_comes_first() {
        assert_eq!(
            crate_sizes(&parse_nm(NM)),
            [
                (ANONYMOUS.to_owned(), 300),
                ("serde_json".to_owned(), 240),
                (STD.to_owned(), 120),
                (NO_CRATE.to_owned(), 50),
                ("cineris_core".to_owned(), 30),
            ]
        );
    }

    #[test]
    fn the_breakdown_table_says_each_crates_bytes_and_share_of_the_binary() {
        let sizes = vec![
            ("serde_json".to_owned(), 600),
            (STD.to_owned(), 200),
            ("tokio".to_owned(), 100),
            ("cineris_core".to_owned(), 50),
        ];
        let table = breakdown_table(&sizes, 1000, 2);
        assert!(
            table.contains("The binary's 1000 bytes by crate"),
            "{table}"
        );
        assert!(table.contains("4 crates"), "{table}");
        let row = |start: &str| {
            table
                .lines()
                .find(|line| line.starts_with(start))
                .unwrap_or_else(|| panic!("{start}: {table}"))
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(row("serde_json"), "serde_json 600 60.0%");
        assert_eq!(row("std (core"), "std (core, alloc, std) 200 20.0%");
        assert_eq!(row("2 more crates"), "2 more crates 150 15.0%");
        assert_eq!(
            row("[in no symbol"),
            "[in no symbol: headers, unwind tables, padding] 50 5.0%"
        );
        assert!(!table.contains("tokio"), "{table}");
        assert!(!table.contains("add up to more"), "{table}");
        // Symbols that sum to more than the file are said, and the rest is none.
        let over = breakdown_table(&sizes, 900, 10);
        assert!(over.contains("add up to more than the binary"), "{over}");
        assert!(over.contains(" 0  "), "{over}");
    }

    fn git(repository: &std::path::Path, date: &str, arguments: &[&str]) {
        let status = Command::new("git")
            .args(arguments)
            .current_dir(repository)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .status()
            .expect("git runs");
        assert!(status.success(), "git {arguments:?}");
    }

    /// A commit's tree is extracted with every file dated now, and not as the
    /// commit: a build directory with artifacts newer than the commit would
    /// take a file dated as the commit for unchanged.
    #[test]
    fn sys_a_commits_tree_is_extracted_with_files_dated_now() {
        let directory = std::env::temp_dir().join(format!("xtask-size-at-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        let (repository, into) = (directory.join("repo"), directory.join("tree"));
        fs::create_dir_all(&repository).unwrap();
        fs::create_dir_all(&into).unwrap();
        git(&repository, "2001-02-03T04:05:06+00:00", &["init", "-q"]);
        fs::create_dir_all(repository.join("src")).unwrap();
        fs::write(repository.join("src").join("lib.rs"), "pub fn a() {}\n").unwrap();
        fs::write(repository.join("Cargo.toml"), "[package]\n").unwrap();
        git(&repository, "2001-02-03T04:05:06+00:00", &["add", "-A"]);
        git(
            &repository,
            "2001-02-03T04:05:06+00:00",
            &["commit", "-q", "-m", "A commit long ago"],
        );
        let hash = git_in(&repository, &["rev-parse", "--verify", "HEAD^{commit}"]).unwrap();
        assert_eq!(hash.len(), 40, "{hash}");
        extract(&repository, &hash, &into).unwrap();
        let file = into.join("src").join("lib.rs");
        assert_eq!(fs::read_to_string(&file).unwrap(), "pub fn a() {}\n");
        let age = std::time::SystemTime::now()
            .duration_since(fs::metadata(&file).unwrap().modified().unwrap())
            .unwrap_or_default();
        assert!(
            age < std::time::Duration::from_secs(3600),
            "the file is dated as the commit: it is {age:?} old"
        );
        // A commit that is not there is said, not extracted.
        let error = git_in(&repository, &["rev-parse", "--verify", "nope^{commit}"]).unwrap_err();
        assert!(error.contains("failed"), "{error}");
        let _ = fs::remove_dir_all(&directory);
    }
}

/// What the sandbox's signal test prints when the kernel's Landlock does not
/// scope signals: the gap of the partial case, named.
const PARTIAL_MARK: &str = "does not scope signals: the gap the status names";

/// `cargo xtask partial-landlock`: `cineris-sandbox`'s tests, as the blocking
/// job runs them, on a kernel whose Landlock is older than 6 (the `nightly`
/// workflow's Ubuntu 22.04 job, ABI 4). The confinement tests fail rather
/// than skip on CI where the sandbox does not enforce; this fails too when
/// the run did not show the partial case, so that a kernel that moves on
/// does not leave the case untested in silence.
pub(crate) fn partial_landlock() -> Result<()> {
    crate::checks::require_nextest()?;
    let mut run = crate::checks::ci_run(&["cineris-sandbox".to_owned()]);
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
/// build with `cineris-core` in it turns on, and without which the
/// confinement tests are not compiled at all; and the tests' own output,
/// where the partial case is named.
fn partial_arguments() -> [String; 4] {
    [
        "--features".to_owned(),
        "cineris-sandbox/backends".to_owned(),
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
             cineris-sandbox's `backends` feature), or this kernel's Landlock scopes signals. In \
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
        let shown = "    PASS [ 0.1s] cineris-sandbox::it confined::sys_a_signal_to_a_process_outside_the_sandbox_is_refused\n\
                     Landlock 4 does not scope signals: the gap the status names\n";
        assert_eq!(partial_verdict(true, shown), Ok(()));
        let error = partial_verdict(true, "    PASS [ 0.1s] cineris-sandbox::it confined::x\n")
            .unwrap_err();
        assert!(error.contains("the partial case ran nowhere"), "{error}");
        // The confinement tests are built only with the backends.
        assert!(
            super::partial_arguments()
                .windows(2)
                .any(|pair| pair == ["--features", "cineris-sandbox/backends"])
        );
        let error = partial_verdict(false, shown).unwrap_err();
        assert!(error.contains("tests failed"), "{error}");
    }
}

/// The git-heavy system tests of tools run locally or on Linux CI, which the
/// blocking job runs on Linux alone and the Windows nightly runs by name
/// (`docs/testing-and-ci.md`, "A system test that starts more than about
/// 20 processes"). Each is ignored on Windows with a reason whose last words
/// say that the nightly runs it there; a test holds this list to them.
pub(crate) const GIT_TESTS_ON_WINDOWS: &[&str] = &[
    "owner_guard::local::tests::sys_a_push_is_searched_in_every_place_it_can_carry_a_term",
    "owner_guard::local::tests::sys_a_clean_push_and_what_the_remote_has_find_nothing",
    "owner_guard::local::tests::sys_what_the_machine_or_another_remote_could_hide_is_still_read",
    "owner_guard::local::tests::sys_a_repository_that_requires_the_file_refuses_a_push_without_it",
    "owner_guard::local::tests::sys_a_failing_git_is_an_error_not_a_pass",
    "owner_guard::tests::sys_a_real_range_is_read_for_its_lines_its_paths_and_its_people",
    "rebase::tests::sys_a_rebase_settles_rows_tests_and_a_generated_file_and_regenerates_at_the_end",
    "branch::tests::sys_records_reach_a_branch_not_checked_out_and_a_fold_rewrites_only_the_current_one",
    "mutate::changed::tests::sys_a_real_repository_gives_the_files_a_diff_touches_with_both_names_of_a_move",
    "git_read::tests::sys_the_three_questions_read_a_branch_its_changes_and_its_new_files",
    "rename::tests::sys_a_rust_file_is_formatted_again_only_when_it_was_formatted",
    "rename::tests::sys_a_branch_is_made_again_in_the_new_names_on_every_system",
];

/// `cargo xtask git-tests-on-windows`: runs [`GIT_TESTS_ON_WINDOWS`], ignored
/// on Windows, by their exact names, and fails unless each one ran and
/// passed. A name that matches no test would otherwise pass having run
/// nothing.
pub(crate) fn git_tests_on_windows() -> Result<()> {
    let mut run = vec![
        "test",
        "--locked",
        "-p",
        "xtask",
        "--",
        "--ignored",
        "--exact",
    ];
    run.extend(GIT_TESTS_ON_WINDOWS);
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
    every_test_passed(output.status.success(), &printed, GIT_TESTS_ON_WINDOWS)
}

/// Whether the run passed and its output has a passing line for each of
/// `names` (`test NAME ... ok`, as the test harness prints it).
fn every_test_passed(succeeded: bool, printed: &str, names: &[&str]) -> Result<()> {
    if !succeeded {
        return Err("the tests failed (above)".to_owned());
    }
    let passed: BTreeSet<&str> = printed
        .lines()
        .filter_map(|line| line.trim().strip_prefix("test ")?.strip_suffix(" ... ok"))
        .collect();
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| !passed.contains(name))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} of the {} tests named did not run: {}.\n    A test renamed or moved must be renamed in \
         GIT_TESTS_ON_WINDOWS (xtask/src/nonblocking.rs) too.",
        missing.len(),
        names.len(),
        missing.join(", ")
    ))
}

#[cfg(test)]
mod git_tests_on_windows_tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use super::{GIT_TESTS_ON_WINDOWS, every_test_passed};
    use crate::workspace;

    /// The end of the reason such a test gives for being ignored on Windows.
    const MARK: &str = "the nightly runs it on Windows\"";

    /// Each test under `dir` whose ignore gives [`MARK`], by its path in the
    /// test binary: its file's modules, `tests` for a module inline in it.
    fn marked(src: &Path, dir: &Path, into: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                marked(src, &path, into);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap();
            if !text.contains(MARK) {
                continue;
            }
            let relative = path.strip_prefix(src).unwrap().with_extension("");
            let mut module: Vec<String> = relative
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            if module.last().is_some_and(|last| last != "tests") {
                assert!(text.contains("mod tests {"), "{}", path.display());
                module.push("tests".to_owned());
            }
            for (at, _) in text.match_indices(MARK) {
                let name = text[at..]
                    .split("fn ")
                    .nth(1)
                    .and_then(|after| after.split('(').next())
                    .unwrap_or_else(|| panic!("no test after the mark in {}", path.display()));
                into.insert(format!("{}::{name}", module.join("::")));
            }
        }
    }

    /// A test marked but not named is run on Windows by nobody, and a name
    /// with no test fails the nightly: the list and the marks agree, path
    /// for path, and the nightly runs the command.
    #[test]
    fn every_test_kept_off_windows_is_named_for_the_nightly() {
        let root = workspace::root();
        let src = root.join("xtask").join("src");
        let mut kept_off = BTreeSet::new();
        marked(&src, &src, &mut kept_off);
        let named: BTreeSet<String> = GIT_TESTS_ON_WINDOWS
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        assert_eq!(named.len(), GIT_TESTS_ON_WINDOWS.len(), "a name twice");
        assert_eq!(named, kept_off);
        let nightly = fs::read_to_string(root.join(".github/workflows/nightly.yml")).unwrap();
        assert!(
            nightly.contains("-- git-tests-on-windows"),
            "the nightly runs it"
        );
    }

    #[test]
    fn a_name_that_ran_nothing_fails_the_run_and_is_named() {
        let printed = "running 2 tests\ntest a::tests::sys_one ... ok\ntest b::sys_two ... ok\n\n\
                       test result: ok. 2 passed; 0 failed; 0 ignored\n";
        assert_eq!(
            every_test_passed(true, printed, &["a::tests::sys_one", "b::sys_two"]),
            Ok(())
        );
        let error =
            every_test_passed(true, printed, &["a::tests::sys_one", "c::sys_three"]).unwrap_err();
        assert!(
            error.starts_with("1 of the 2 tests named did not run: c::sys_three."),
            "{error}"
        );
        // A test that ran and failed is not a pass, nor one that was ignored.
        let failed = "test a::tests::sys_one ... FAILED\ntest b::sys_two ... ignored\n";
        assert!(every_test_passed(true, failed, &["a::tests::sys_one"]).is_err());
        assert!(every_test_passed(true, failed, &["b::sys_two"]).is_err());
        assert!(every_test_passed(false, printed, &["a::tests::sys_one"]).is_err());
    }
}
