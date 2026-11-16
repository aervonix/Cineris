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

Builds `cortex` and reports its size and the crates in the build.
  --check          fail over the budget too (a home path fails every run)
  --target TRIPLE  the target to build for (the host's when not given; the
                   budgets are of x86_64-unknown-linux-musl and
                   x86_64-pc-windows-msvc)
  --against-main   the difference from the size `main` last recorded (dist only)
  --profile P      `dist` (the shipped binary, and the default) or `release`
                   (thin LTO, a fraction of the build time: not the shipped
                   size, so it has no budget and no `main` to compare with)
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
    top: usize,
    at: Option<String>,
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
            top: 25,
            at: None,
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
                _ => return Err(SIZE_USAGE.to_owned()),
            }
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
                     a copy with no `.git` (the desk's `rx` makes one) cannot, and a commit not \
                     fetched is not here"
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
    let mut archive = Command::new("git")
        .args(["archive", "--format=tar", hash])
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
    shipped::build_with(&shipped::Options {
        profile: &request.profile,
        target: Some(&target),
        tree: archive.as_ref().map(|archive| archive.tree.as_path()),
        target_dir: own_target.as_deref(),
        variables: &[],
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

    // Last: it builds again, and its failure must not hide the verdict.
    let breakdown = request
        .breakdown
        .then(|| breakdown(&tree, &target_directory, &target, &request, bytes));
    verdict(paths, report.over, request.check)?;
    breakdown.unwrap_or(Ok(()))
}

/// Where cargo puts the binary of `profile` for `target`.
fn binary_in(target_directory: &Path, target: &str, profile: &str) -> PathBuf {
    let executable = if target.contains("windows") {
        "cortex.exe"
    } else {
        "cortex"
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
) -> Result<()> {
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
    let table = breakdown_table(
        &crate_sizes(&parse_nm(&String::from_utf8_lossy(&output.stdout))),
        bytes,
        request.top,
    );
    println!("\n{table}");
    steps::append_job_summary(&format!(
        "### Binary size by crate\n\n```text\n{table}```\n"
    ));
    Ok(())
}

/// The environment variable that turns a profile's `strip` off.
fn strip_variable(profile: &str) -> String {
    format!("CARGO_PROFILE_{}_STRIP", profile.to_uppercase())
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
/// (`core::ptr::drop_in_place<cortex_core::X>` is the standard library's, the
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
        let report = report(LINUX, PROFILE, 12_000_000, 180, None);
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
        let report = report(WINDOWS, PROFILE, limit + 351_744, 180, None);
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
        assert_eq!(super::report(WINDOWS, PROFILE, limit, 180, None).over, None);
    }

    #[test]
    fn the_difference_from_mains_last_size_is_signed() {
        let main = Ok(MainSize {
            bytes: 12_100_000,
            run: 43_811_142_390,
            commit: Some("c225d64".to_owned()),
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
        assert_eq!(budget("x86_64-unknown-linux-musl"), 16 * 1024 * 1024);
        assert_eq!(budget("aarch64-unknown-linux-musl"), 16 * 1024 * 1024);
        assert_eq!(budget("x86_64-pc-windows-msvc"), 18 * 1024 * 1024);
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
                top: 5,
                at: Some("origin/main~2".to_owned()),
            }
        );
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
                "| `x86_64-pc-windows-msvc` | `release` | 18874369 | 18.00 | - | no budget | 180 |"
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

    /// `nm --print-size --demangle --radix=d --defined-only`, as GNU's prints
    /// it: address, size, a letter for the kind, the name (which may hold
    /// spaces); a symbol with no size has two fields before the name.
    const NM: &str = "\
0000000000004096 0000000000000200 T serde_json::de::from_slice
0000000000004608 0000000000000040 T <serde_json::value::Value as core::fmt::Debug>::fmt
0000000000004700 0000000000000100 t core::fmt::write
0000000000004700 0000000000000100 T core::fmt::write_alias
0000000000008192 0000000000000300 r anon.0123abcd.4
0000000000008500 0000000000000030 R cortex_core::table
0000000000009000 0000000000000050 T memcpy
0000000000009100 0000000000000900 B cortex_core::bss_is_not_in_the_file
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
                "cortex_core::table",
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
            ("<&cortex_core::X as core::fmt::Debug>::fmt", "cortex_core"),
            ("core::ptr::drop_in_place<cortex_core::X>", STD),
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
                ("cortex_core".to_owned(), 30),
            ]
        );
    }

    #[test]
    fn the_breakdown_table_says_each_crates_bytes_and_share_of_the_binary() {
        let sizes = vec![
            ("serde_json".to_owned(), 600),
            (STD.to_owned(), 200),
            ("tokio".to_owned(), 100),
            ("cortex_core".to_owned(), 50),
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
    fn a_commits_tree_is_extracted_with_files_dated_now() {
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
