//! CI runs on GitHub, read through `gh`: a run's JUnit reports and its
//! jobs' times. `cargo xtask ci --tiers --compare` reads reports from here,
//! and `cargo xtask ci-times` prints a run's tables in the form the steps'
//! `ci.md` files use (`docs/testing-and-ci.md`, "A CI run's test times").
//!
//! Nothing here writes to GitHub. `gh` must be installed and signed in; it
//! asks for nothing but reads.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::Result;
use crate::junit::{self, TestCase};
use crate::tiers::Os;
use crate::workspace;

/// Where a report comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// A JUnit report on disk.
    File(PathBuf),
    /// A run of the `ci` workflow, by its id.
    Run(u64),
    /// The main branch's latest successful run of the `ci` workflow.
    Main,
}

impl Source {
    /// `main`, a run's id (digits), or a path.
    pub(crate) fn parse(text: &str) -> Self {
        if text == "main" {
            Source::Main
        } else if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
            text.parse()
                .map_or_else(|_| Source::File(text.into()), Source::Run)
        } else {
            Source::File(text.into())
        }
    }
}

/// Runs `gh` with `arguments` and returns what it printed.
pub(crate) fn gh(arguments: &[&str]) -> Result<String> {
    let output = Command::new("gh")
        .args(arguments)
        .current_dir(workspace::root())
        .output()
        .map_err(|error| format!("could not run gh (is it installed?): {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`gh {}` failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|_| {
        format!(
            "`gh {}` printed bytes that are not text",
            arguments.join(" ")
        )
    })
}

/// How many of the main branch's latest successful runs are looked at for
/// one that kept the reports asked for.
const MAIN_RUNS_LOOKED_AT: usize = 20;

/// What became of a run's artifact of one name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kept {
    /// It is there to download.
    Yes,
    /// The run made none: its tests did not run (a change of documentation
    /// alone, a job that was cancelled or failed before the upload).
    Never,
    /// It was made and has expired.
    Expired,
}

/// `name`'s fate among a run's artifacts, each a name and whether it has
/// expired, as GitHub lists them.
pub(crate) fn kept(artifacts: &[(String, bool)], name: &str) -> Kept {
    match artifacts.iter().find(|(artifact, _)| artifact == name) {
        None => Kept::Never,
        Some((_, true)) => Kept::Expired,
        Some((_, false)) => Kept::Yes,
    }
}

/// A run's artifacts, asked of GitHub. A failure here is an error, never
/// "no artifacts": the network or the sign-in is not the run.
pub(crate) fn artifacts(run: u64) -> Result<Vec<(String, bool)>> {
    Ok(pairs_of(&listing(run)?))
}

/// One of a run's artifacts as GitHub lists it: its name, whether it has
/// expired, and its id, which a new upload of the same name (a job run
/// again, with `overwrite`) changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Listed {
    pub(crate) name: String,
    pub(crate) expired: bool,
    pub(crate) id: u64,
}

/// A run's artifacts with their ids, asked of GitHub.
pub(crate) fn listing(run: u64) -> Result<Vec<Listed>> {
    parse_listing(&gh(&[
        "api",
        &format!("repos/{{owner}}/{{repo}}/actions/runs/{run}/artifacts?per_page=100"),
        "--jq",
        ".artifacts[] | \"\\(.name) \\(.expired) \\(.id)\"",
    ])?)
}

/// A run's artifacts, as `name expired id` lines from GitHub's API, read.
pub(crate) fn parse_listing(listed: &str) -> Result<Vec<Listed>> {
    listed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let refused = || format!("an artifact line that is not `name expired id`: `{line}`");
            let (rest, id) = line.trim().rsplit_once(' ').ok_or_else(refused)?;
            let id = id.parse::<u64>().map_err(|_| refused())?;
            let (name, expired) = match rest.rsplit_once(' ') {
                Some((name, "true")) => (name, true),
                Some((name, "false")) => (name, false),
                _ => return Err(refused()),
            };
            Ok(Listed {
                name: name.to_owned(),
                expired,
                id,
            })
        })
        .collect()
}

/// Each artifact's name and whether it has expired.
fn pairs_of(listed: &[Listed]) -> Vec<(String, bool)> {
    listed
        .iter()
        .map(|artifact| (artifact.name.clone(), artifact.expired))
        .collect()
}

/// What a download of the artifacts `names` is kept with: each one's name
/// and id, as `listed` has them. A download whose stamp is not the
/// listing's is of an upload that has since been replaced, and is fetched
/// again. `None` when one of them is not listed.
fn stamp(listed: &[Listed], names: &[&str]) -> Option<String> {
    names
        .iter()
        .map(|name| {
            listed
                .iter()
                .find(|artifact| artifact.name == *name)
                .map(|artifact| format!("{}={}", artifact.name, artifact.id))
        })
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join(","))
}

/// Whether the download that `marker` was written beside is of the uploads
/// `stamp` names: a marker of an older form (empty) or of another upload
/// is not.
fn is_current(marker: &Path, stamp: &str) -> bool {
    fs::read_to_string(marker).is_ok_and(|text| text == stamp)
}

/// What a run must have kept for one thing to be read from it: the
/// artifacts of one of its forms, every one of them. A `ci` run keeps a
/// test job's JUnit report in one artifact (`junit-linux`; `junit-windows`
/// before the Windows job was split) or in one for each part of a split
/// job (`junit-windows-tiers` and `junit-windows-system`), read as one
/// report ([`ci_report`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Wanted {
    forms: Vec<Vec<String>>,
}

impl Wanted {
    /// One artifact, in one form.
    pub(crate) fn one(name: &str) -> Self {
        Self {
            forms: vec![vec![name.to_owned()]],
        }
    }

    /// The test report of `os` that a `ci` run keeps, in each form it has
    /// kept it in.
    pub(crate) fn ci_report(os: Os) -> Self {
        Self {
            forms: report_forms(os)
                .iter()
                .map(|form| form.iter().map(|name| (*name).to_owned()).collect())
                .collect(),
        }
    }

    /// Whether `artifacts` hold every artifact of one of the forms.
    fn kept_by(&self, artifacts: &[(String, bool)]) -> bool {
        self.forms
            .iter()
            .any(|form| form.iter().all(|name| kept(artifacts, name) == Kept::Yes))
    }

    /// `` `junit-windows`, or `junit-windows-tiers` and `junit-windows-system` ``.
    pub(crate) fn describe(&self) -> String {
        self.forms
            .iter()
            .map(|form| {
                form.iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            })
            .collect::<Vec<_>>()
            .join(", or ")
    }
}

/// The forms a `ci` run keeps the test report of `os` in, each a list of
/// artifacts that make one report together. The Windows job was one job,
/// `test-windows`, until it was split into a build job and two jobs that
/// run the tests from its archive, each keeping the report of its own
/// part, tiers 1 and 2 first ([`checks::ci_test_steps`]'s order): a run
/// from before keeps one, and the records cite such runs. The names are
/// the workflow's (`.github/workflows/ci.yml`), which a test holds to
/// these.
///
/// [`checks::ci_test_steps`]: crate::checks
pub(crate) fn report_forms(os: Os) -> &'static [&'static [&'static str]] {
    match os {
        Os::Linux => &[&["junit-linux"]],
        Os::Windows => &[
            &["junit-windows-tiers", "junit-windows-system"],
            &["junit-windows"],
        ],
    }
}

/// `needs`, each described, joined.
fn describe_all(needs: &[Wanted]) -> String {
    needs
        .iter()
        .map(Wanted::describe)
        .collect::<Vec<_>>()
        .join("; and ")
}

/// The first of `runs` (newest first, each with its artifacts) that kept
/// everything of `needs`.
pub(crate) fn first_that_kept(
    runs: &[(u64, Vec<(String, bool)>)],
    needs: &[Wanted],
) -> Option<u64> {
    runs.iter()
        .find(|(_, artifacts)| needs.iter().all(|wanted| wanted.kept_by(artifacts)))
        .map(|(run, _)| *run)
}

/// The main branch's latest successful `ci` run that kept the reports of
/// `needs`: a run of a documentation change alone is successful and keeps
/// none, and is passed over. It is printed, so that a reader knows which
/// run `main` was.
pub(crate) fn latest_main_run(needs: &[Wanted]) -> Result<u64> {
    latest_main_run_of("ci", "success", needs)
}

/// The main branch's latest run of `workflow` whose status is `status` (as
/// `gh run list --status` takes it: `success`, `completed` and so on) that
/// kept everything of `needs`.
pub(crate) fn latest_main_run_of(workflow: &str, status: &str, needs: &[Wanted]) -> Result<u64> {
    let limit = MAIN_RUNS_LOOKED_AT.to_string();
    let listed = gh(&[
        "run",
        "list",
        "--workflow",
        workflow,
        "--branch",
        "main",
        "--status",
        status,
        "--limit",
        &limit,
        "--json",
        "databaseId,headSha",
    ])?;
    let value: Value =
        serde_json::from_str(&listed).map_err(|error| format!("gh run list: {error}"))?;
    let listed: Vec<(u64, String)> = value
        .as_array()
        .map(|runs| {
            runs.iter()
                .filter_map(|run| {
                    Some((
                        run.get("databaseId")?.as_u64()?,
                        run.get("headSha")?.as_str()?.chars().take(7).collect(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    if listed.is_empty() {
        return Err(format!(
            "the main branch has no run of the `{workflow}` workflow whose status is `{status}`"
        ));
    }
    // One at a time, newest first: most of the time the first will do.
    for (run, sha) in &listed {
        let artifacts = artifacts(*run)?;
        if first_that_kept(&[(*run, artifacts)], needs).is_some() {
            println!(
                "main: run {run} (at {sha}), the main branch's latest `{workflow}` run \
                 ({status}) that kept {}",
                describe_all(needs)
            );
            return Ok(*run);
        }
    }
    Err(format!(
        "none of the main branch's last {} `{workflow}` runs ({status}) kept {}",
        listed.len(),
        describe_all(needs)
    ))
}

/// The `ci` run of the main branch at the commit a pull request's run is
/// based on (where its head's history meets `main`'s), when one kept every
/// report of `needs`: what `ci-times --pr N` compares the pull request
/// with, so that a run of `main` made after it, with other steps' tests in
/// it, does not move its figures. `None`, said, when that commit has no such
/// run (a documentation change keeps no report): the caller takes `main`'s
/// latest then.
pub(crate) fn base_main_run(head_sha: &str, needs: &[Wanted]) -> Result<Option<u64>> {
    let base = gh(&[
        "api",
        &format!("repos/{{owner}}/{{repo}}/compare/main...{head_sha}"),
        "--jq",
        ".merge_base_commit.sha",
    ])?;
    let base = base.trim();
    if base.len() != 40 || !base.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!(
            "GitHub gave no commit where {head_sha} meets main: {base:?}"
        ));
    }
    let listed = gh(&[
        "run",
        "list",
        "--workflow",
        "ci",
        "--branch",
        "main",
        "--commit",
        base,
        "--status",
        "success",
        "--json",
        "databaseId",
    ])?;
    let value: Value =
        serde_json::from_str(&listed).map_err(|error| format!("gh run list: {error}"))?;
    let mut runs = Vec::new();
    for run in value.as_array().into_iter().flatten() {
        if let Some(id) = run.get("databaseId").and_then(Value::as_u64) {
            runs.push((id, artifacts(id)?));
        }
    }
    let chosen = first_that_kept(&runs, needs);
    let short: String = base.chars().take(7).collect();
    match chosen {
        Some(run) => println!(
            "main: run {run} (at {short}), the main branch's `ci` run at the commit the pull \
             request is based on"
        ),
        None => println!(
            "main: the commit the pull request is based on ({short}) has no successful `ci` run \
             that kept {}; main's latest is taken instead, and its figures may hold other \
             changes",
            describe_all(needs)
        ),
    }
    Ok(chosen)
}

/// The run a source names, if it names one. `needs` is what a run of `main`
/// must have kept.
pub(crate) fn run_of(source: &Source, needs: &[Wanted]) -> Result<Option<u64>> {
    match source {
        Source::File(_) => Ok(None),
        Source::Run(id) => Ok(Some(*id)),
        Source::Main => latest_main_run(needs).map(Some),
    }
}

/// The JUnit report of a run's test job on `os`: downloaded once, into a
/// directory of its own under `target/xtask-runs/`, and read from there
/// afterwards, for as long as GitHub lists the same uploads: a job run
/// again replaces its report (`overwrite`), and the new one is fetched.
///
/// A run of the `ci` workflow keeps it as `junit-<os>`, or, since the
/// Windows job was split, as one artifact for each part of the job
/// ([`report_forms`]): the parts are fetched and made one report, tiers 1
/// and 2 first, kept where the one artifact would be. A run that kept one
/// part and not the other (a job cut or failed before its upload) has no
/// whole report: it is said, and taken as none. A run of the `try`
/// workflow, which keeps no such artifact, is read from its own,
/// `try-<os>`: the report of its tests in the `ci` profile
/// (`target/nextest/ci/junit.xml`, wherever the artifact keeps the runner's
/// paths). A `try` run that was an A/B holds two trees' reports and no one
/// report, and is refused: `ci-times --ab` and `try-result` read it.
///
/// `None` only when the run made no such report. An expired report, a
/// download that fails, and a question GitHub does not answer are errors.
pub(crate) fn junit_of(run: u64, os: Os) -> Result<Option<PathBuf>> {
    let name = artifact_name(os);
    let directory = download_root().join(run.to_string()).join(name);
    let report = directory.join("junit.xml");
    let tried = try_artifact_name(os);
    let listed = listing(run)?;
    let parts = match which_artifact(run, &pairs_of(&listed), os)? {
        Chosen::None => return Ok(None),
        Chosen::Partial { kept, missing } => {
            println!(
                "run {run} kept `{kept}` and not `{missing}`: one of its test jobs did not keep \
                 its report, so the run has no whole report of its tests, and none is read"
            );
            return Ok(None);
        }
        Chosen::Try => {
            return match artifact_dir_in(run, tried, &listed)? {
                Some(directory) => report_of_try(run, &directory),
                None => Ok(None),
            };
        }
        Chosen::Ci(names) => names,
    };
    let stamp = stamp(&listed, parts)
        .ok_or_else(|| format!("run {run}: an artifact was listed and then was not"))?;
    let marker = directory.join(".stamp");
    if report.is_file() && is_current(&marker, &stamp) {
        return Ok(Some(report));
    }
    let written = if let [only] = parts {
        download_one(run, only, &directory)?
    } else {
        // The parts, each downloaded whole, and then one report of them,
        // written whole or not at all.
        let mut downloaded = Vec::new();
        for part in parts {
            let part_directory = artifact_dir_in(run, part, &listed)?
                .ok_or_else(|| format!("run {run}'s `{part}` was listed and then was not there"))?;
            downloaded.push((*part, part_directory));
        }
        let Some(merged) = merge_parts(run, &downloaded)? else {
            return Ok(None);
        };
        fs::create_dir_all(&directory)
            .map_err(|error| format!("could not make {}: {error}", directory.display()))?;
        workspace::forget_report(&marker)?;
        let partial = directory.join("junit.xml.partial");
        fs::write(&partial, merged)
            .map_err(|error| format!("could not write {}: {error}", partial.display()))?;
        fs::rename(&partial, &report)
            .map_err(|error| format!("could not write {}: {error}", report.display()))?;
        Some(report)
    };
    // Written last: a report without it is fetched again.
    fs::write(&marker, &stamp)
        .map_err(|error| format!("could not write {}: {error}", marker.display()))?;
    Ok(written)
}

/// One report of a split run's `parts`, each an artifact's name and the
/// directory it was downloaded into, in the order given; `None`, said,
/// when a part holds no report. A part that kept its start probe's report
/// beside its own has its cases held above that start
/// ([`junit::with_start`]).
fn merge_parts(run: u64, parts: &[(&str, PathBuf)]) -> Result<Option<String>> {
    let mut merged: Option<String> = None;
    for (part, part_directory) in parts {
        let file = part_directory.join("junit.xml");
        if !file.is_file() {
            // A job that kept its start probe's report and not its own (its
            // run of the tests ended before nextest wrote one).
            println!(
                "run {run}'s `{part}` has no junit.xml: one of its test jobs did not keep its \
                 report, so the run has no whole report of its tests, and none is read"
            );
            return Ok(None);
        }
        let mut text = fs::read_to_string(&file)
            .map_err(|error| format!("could not read {}: {error}", file.display()))?;
        // A part that measured its own start (the system tests' job, by its
        // probe) is held above it, on its own runner, and not above the
        // other part's.
        let probe = part_directory.join(crate::tiers::START_PROBE_FILE);
        if probe.is_file() {
            // A probe the job's own check refused (a failure, too few tests,
            // none) measured no start, and the part is no measurement.
            let cases = match crate::tiers::read_probe(&probe) {
                Ok(cases) => cases,
                Err(error) => {
                    println!(
                        "run {run}'s `{part}`: {error}; the run has no whole report of its \
                         tests, and none is read"
                    );
                    return Ok(None);
                }
            };
            let (start, sample) = crate::tiers::start_of(&cases);
            println!(
                "run {run}: `{part}`'s tests are held above its own runner's start, {:.0} ms a \
                 test (its start probe's {sample} unit tests)",
                1000.0 * start
            );
            text = junit::with_start(&text, start);
        }
        merged = Some(match merged {
            None => text,
            Some(earlier) => junit::merge(&earlier, &text)
                .map_err(|error| format!("run {run}'s `{part}`: {error}"))?,
        });
    }
    merged
        .map(Some)
        .ok_or_else(|| format!("run {run}: a report of no parts"))
}

/// A run's one artifact `name`, holding its report, downloaded into
/// `directory`.
fn download_one(run: u64, name: &str, directory: &Path) -> Result<Option<PathBuf>> {
    let report = directory.join("junit.xml");
    // A partial download from an earlier attempt is not trusted.
    if directory.exists() {
        fs::remove_dir_all(directory)
            .map_err(|error| format!("could not clear {}: {error}", directory.display()))?;
    }
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not make {}: {error}", directory.display()))?;
    let run_text = run.to_string();
    let target = directory.to_string_lossy().into_owned();
    gh(&[
        "run", "download", &run_text, "--name", name, "--dir", &target,
    ])
    .map_err(|error| format!("run {run}'s `{name}` is there and could not be fetched: {error}"))?;
    if report.is_file() {
        Ok(Some(report))
    } else {
        Err(format!(
            "run {run}'s artifact `{name}` has no junit.xml (see {})",
            directory.display()
        ))
    }
}

/// Where a run keeps its report of one OS, as [`which_artifact`] finds it.
#[derive(Debug, PartialEq, Eq)]
enum Chosen {
    /// The `ci` workflow's artifacts of one form, every one kept.
    Ci(&'static [&'static str]),
    /// One part of a split job's report kept, and the other not.
    Partial {
        kept: &'static str,
        missing: &'static str,
    },
    /// A `try` run's own artifact.
    Try,
    /// Neither.
    None,
}

/// Which of a run's artifacts hold the JUnit report of `os`: the `ci`
/// workflow's, in one of its forms ([`report_forms`]), else a `try` run's
/// `try-<os>`; none when the run kept neither, and an error when one it
/// kept has expired.
fn which_artifact(run: u64, listed: &[(String, bool)], os: Os) -> Result<Chosen> {
    let expired = |name: &str| {
        format!("run {run}'s `{name}` has expired: a run's reports are kept for 14 days")
    };
    for form in report_forms(os) {
        if let Some(name) = form.iter().find(|name| kept(listed, name) == Kept::Expired) {
            return Err(expired(name));
        }
        let (kept_names, missing): (Vec<&&str>, Vec<&&str>) = form
            .iter()
            .partition(|name| kept(listed, name) == Kept::Yes);
        match (kept_names.first(), missing.first()) {
            (Some(_), None) => return Ok(Chosen::Ci(form)),
            (Some(kept), Some(missing)) => {
                return Ok(Chosen::Partial { kept, missing });
            }
            _ => {}
        }
    }
    match kept(listed, try_artifact_name(os)) {
        Kept::Yes => Ok(Chosen::Try),
        Kept::Expired => Err(expired(try_artifact_name(os))),
        Kept::Never => Ok(Chosen::None),
    }
}
/// The directory a run's artifact `name` is downloaded into, under
/// `target/xtask-runs/`, fetched through `gh` the first time. `None` only
/// when the run made no such artifact; an expired one and a download that
/// fails are errors, as for [`junit_of`].
pub(crate) fn artifact_dir(run: u64, name: &str) -> Result<Option<PathBuf>> {
    artifact_dir_in(run, name, &listing(run)?)
}

/// [`artifact_dir`], with the run's artifacts already listed.
fn artifact_dir_in(run: u64, name: &str, listed: &[Listed]) -> Result<Option<PathBuf>> {
    let directory = download_root().join(run.to_string()).join(name);
    // Written once the download is whole, with the upload's id: a partial
    // one is not trusted, nor one of an upload since replaced.
    let whole = directory.join(".downloaded");
    let current = stamp(listed, &[name]);
    if current
        .as_deref()
        .is_some_and(|current| is_current(&whole, current))
    {
        return Ok(Some(directory));
    }
    match kept(&pairs_of(listed), name) {
        Kept::Yes => {}
        Kept::Never => return Ok(None),
        Kept::Expired => {
            return Err(format!(
                "run {run}'s `{name}` has expired: a run's artifacts are kept for 14 days"
            ));
        }
    }
    if directory.exists() {
        fs::remove_dir_all(&directory)
            .map_err(|error| format!("could not clear {}: {error}", directory.display()))?;
    }
    fs::create_dir_all(&directory)
        .map_err(|error| format!("could not make {}: {error}", directory.display()))?;
    let run_text = run.to_string();
    let target = directory.to_string_lossy().into_owned();
    gh(&[
        "run", "download", &run_text, "--name", name, "--dir", &target,
    ])
    .map_err(|error| format!("run {run}'s `{name}` is there and could not be fetched: {error}"))?;
    fs::write(&whole, current.unwrap_or_default())
        .map_err(|error| format!("could not write {}: {error}", whole.display()))?;
    Ok(Some(directory))
}

fn artifact_name(os: Os) -> &'static str {
    match os {
        Os::Linux => "junit-linux",
        Os::Windows => "junit-windows",
    }
}

/// The artifact a `try` run keeps for the runner of `os`.
pub(crate) fn try_artifact_name(os: Os) -> &'static str {
    match os {
        Os::Linux => "try-linux",
        Os::Windows => "try-windows",
    }
}

/// Every file under `directory`, in path order: an artifact keeps the
/// runner's paths, so a report is somewhere in it.
pub(crate) fn files_under(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut stack = vec![directory.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)
            .map_err(|error| format!("could not list {}: {error}", dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Whether `path` ends with these folders and file name, compared by name
/// and not by the system's separator.
pub(crate) fn ends_with_names(path: &Path, names: &[&str]) -> bool {
    let parts: Vec<_> = path.components().collect();
    parts.len() >= names.len()
        && parts[parts.len() - names.len()..]
            .iter()
            .zip(names)
            .all(|(part, name)| part.as_os_str() == *name)
}

/// The side and round of an A/B round's report: `base-round-3.xml` is
/// `("base", 3)`.
pub(crate) fn ab_broke_name(file: &str) -> Option<(&str, usize)> {
    ab_report_name(&format!("{}.xml", file.strip_suffix("-broke.xml")?))
        .map(|(side, round)| (if side == "base" { "base" } else { "head" }, round))
}

/// The side and the round of a round's report kept by `ab`,
/// `<side>-round-<n>.xml`; `None` for any other name, a broken run's
/// `<side>-round-<n>-broke.xml` included ([`ab_broke_name`]).
pub(crate) fn ab_report_name(file: &str) -> Option<(&str, usize)> {
    let (side, round) = file.strip_suffix(".xml")?.split_once("-round-")?;
    let round = round.parse().ok()?;
    (side == "base" || side == "head").then_some((side, round))
}

/// The one report of a downloaded `try-<os>` artifact: its tests in the
/// `ci` profile. An artifact of an A/B run holds none that is one run's,
/// and says so.
fn report_of_try(run: u64, directory: &Path) -> Result<Option<PathBuf>> {
    let files = files_under(directory)?;
    let is_ab = |file: &PathBuf| {
        file.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| ab_report_name(name).is_some())
    };
    if files.iter().any(is_ab) {
        return Err(format!(
            "run {run} is a `try` A/B run (it kept both trees' reports, round by round): there is \
             no one report to compare. `cargo xtask ci-times --ab {run}` or `cargo xtask \
             try-result {run}` read it"
        ));
    }
    Ok(files
        .into_iter()
        .find(|file| ends_with_names(file, &["nextest", "ci", "junit.xml"])))
}

/// A source whose run has been chosen: a report on disk, or a run of the
/// `ci` or `try` workflow. `main` is chosen once, so that both OSes of a
/// comparison are read from one run of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolved {
    File(PathBuf),
    Run { id: u64, main: bool },
}

/// The artifacts a run of `main` must have kept for every OS of `oses` to be
/// read from it: one run of `main` for all of them, so that the two OSes of
/// a comparison are of one commit.
fn needs_of(oses: &[Os]) -> Vec<Wanted> {
    oses.iter().map(|os| Wanted::ci_report(*os)).collect()
}

/// `source`'s run, when it names one, chosen so that it kept the report of
/// every OS in `oses`.
pub(crate) fn resolve(source: &Source, oses: &[Os]) -> Result<Resolved> {
    let needs = needs_of(oses);
    Ok(match source {
        Source::File(path) => Resolved::File(path.clone()),
        Source::Run(id) => Resolved::Run {
            id: *id,
            main: false,
        },
        Source::Main => Resolved::Run {
            id: latest_main_run(&needs)?,
            main: true,
        },
    })
}

/// A report and its label, from wherever `source` says, for `os`.
pub(crate) fn report_of(source: &Resolved, os: Os) -> Result<(String, Vec<TestCase>)> {
    match source {
        Resolved::File(path) => Ok((path.display().to_string(), junit::read(path)?)),
        Resolved::Run { id, main } => {
            let label = if *main {
                format!("run {id}, the main branch's latest successful")
            } else {
                format!("run {id}")
            };
            let report = junit_of(*id, os)?.ok_or_else(|| {
                format!(
                    "run {id} kept no whole report of its tests ({}, or `{}`): its tests did \
                     not run, or not all of them",
                    Wanted::ci_report(os).describe(),
                    try_artifact_name(os)
                )
            })?;
            Ok((label, junit::read(&report)?))
        }
    }
}

/// What `ci-times` reads of a run.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RunInfo {
    pub(crate) id: u64,
    pub(crate) sha: String,
    pub(crate) branch: String,
    pub(crate) event: String,
    pub(crate) date: String,
    /// Each job's name, and how long it took in whole seconds when it ran
    /// to an end.
    pub(crate) jobs: Vec<(String, Option<u64>)>,
}

/// Reads `gh run view <id> --json databaseId,headSha,headBranch,event,createdAt,jobs`.
pub(crate) fn parse_run(json: &str) -> Result<RunInfo> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("gh run view: {error}"))?;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let mut jobs = Vec::new();
    for job in value
        .get("jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = job
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned();
        // A matrix job that was skipped before its matrix was made keeps
        // its name unexpanded (`pr-size-${{ matrix.name }}`): it never ran,
        // and no table has a row for it.
        if name.contains("${{") {
            continue;
        }
        let at = |key: &str| job.get(key).and_then(Value::as_str).and_then(seconds_of);
        let took = match (at("startedAt"), at("completedAt")) {
            (Some(start), Some(end)) if end >= start => Some(end - start),
            _ => None,
        };
        jobs.push((name, took));
    }
    Ok(RunInfo {
        id: value.get("databaseId").and_then(Value::as_u64).unwrap_or(0),
        sha: text("headSha"),
        branch: text("headBranch"),
        event: text("event"),
        date: text("createdAt").chars().take(10).collect(),
        jobs,
    })
}

/// How long a run may sit with no job given a runner, in minutes, before
/// `try --dispatch` and `try-result` say so: `--queued-warn MIN` (or
/// `--queued-limit MIN`) changes it, as it does for `ci-wait`.
pub(crate) const QUEUED_WARN_MINUTES: u64 = 10;

/// The longest limit `--queued-warn` takes: a day.
const QUEUED_WARN_MOST: u64 = 24 * 60;

/// Reads the value of `--queued-warn MIN` (or `--queued-limit MIN`): a
/// number of minutes from 1 to a day.
pub(crate) fn queued_warn_minutes(flag: &str, text: &str) -> Result<u64> {
    text.trim()
        .parse::<u64>()
        .ok()
        .filter(|minutes| (1..=QUEUED_WARN_MOST).contains(minutes))
        .ok_or_else(|| format!("`{flag} {text}`: a number of minutes from 1 to {QUEUED_WARN_MOST}"))
}

/// How many minutes a run has sat with no job given a runner, when that is
/// longer than `limit_minutes`; `None` when it has not, or is not waiting.
///
/// A run waits for a runner while its status is `queued` (or `waiting`,
/// `pending`, `requested`, GitHub's other words for it), or while it is
/// `in_progress` with no job started: GitHub has marked it running and has
/// given none of its jobs a runner. `created` and `now` are seconds since
/// 1970; `jobs_started` counts the jobs that are running or have ended.
pub(crate) fn sat_queued(
    status: &str,
    created: u64,
    now: u64,
    jobs_started: usize,
    limit_minutes: u64,
) -> Option<u64> {
    let waiting = match status {
        "queued" | "waiting" | "pending" | "requested" => true,
        "in_progress" => jobs_started == 0,
        _ => false,
    };
    let sat = now.saturating_sub(created);
    (waiting && sat > limit_minutes * 60).then_some(sat / 60)
}

/// What is said of a run that [`sat_queued`] says has sat too long.
pub(crate) fn sat_queued_text(run: u64, workflow: &str, minutes: u64) -> String {
    format!(
        "run {run} ({workflow}) has sat queued with no jobs for {minutes} min: GitHub has not \
         given it a runner. Cancel and dispatch it again (gh run cancel {run}), or keep waiting."
    )
}

/// How many of the jobs `gh run view --json jobs` lists have been given a
/// runner: they are running, or have ended.
pub(crate) fn jobs_started(jobs: &Value) -> usize {
    jobs.as_array()
        .into_iter()
        .flatten()
        .filter(|job| {
            matches!(
                job.get("status").and_then(Value::as_str),
                Some("in_progress" | "completed")
            )
        })
        .count()
}

/// What to say of run `run`, as `gh run view RUN --json
/// workflowName,status,createdAt,jobs` describes it in `json`, when it has
/// sat queued with no jobs past `limit_minutes` at `now`: `None` when it has
/// not, or when GitHub gave no time it was made.
pub(crate) fn queued_note(
    json: &str,
    run: u64,
    now: u64,
    limit_minutes: u64,
) -> Result<Option<String>> {
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("gh run view: {error}"))?;
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default();
    let Some(created) = seconds_of(text("createdAt")) else {
        return Ok(None);
    };
    let started = value.get("jobs").map_or(0, jobs_started);
    let workflow = match text("workflowName") {
        "" => "its workflow",
        name => name,
    };
    Ok(
        sat_queued(text("status"), created, now, started, limit_minutes)
            .map(|minutes| sat_queued_text(run, workflow, minutes)),
    )
}

/// Seconds since 1970, now.
pub(crate) fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Seconds since 1970 of a time as GitHub writes it, `2026-11-13T10:02:03Z`.
/// `0001-01-01T00:00:00Z`, which GitHub writes for a job that never
/// started, is none.
pub(crate) fn seconds_of(time: &str) -> Option<u64> {
    let (date, clock) = time.strip_suffix('Z')?.split_once('T')?;
    let mut date = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let mut clock = clock.split(':').map(|part| part.parse::<i64>().ok());
    let (hour, minute, second) = (clock.next()??, clock.next()??, clock.next()??);
    if year < 1970 {
        return None;
    }
    // Days from the civil date (the proleptic Gregorian calendar).
    let shifted = if month <= 2 { year - 1 } else { year };
    let era = shifted.div_euclid(400);
    let of_era = shifted - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second).ok()
}

fn run_info(run: u64) -> Result<RunInfo> {
    let id = run.to_string();
    parse_run(&gh(&[
        "run",
        "view",
        &id,
        "--json",
        "databaseId,headSha,headBranch,event,createdAt,jobs",
    ])?)
}

/// What `ci-times` was asked for.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TimesRequest {
    run: Option<String>,
    ab: Option<String>,
    against: Option<String>,
    pr: Option<u64>,
    write: Option<PathBuf>,
    placing: crate::records::Placing,
    /// The tests of each tier that an A/B's detail lists.
    top: usize,
}

/// How many of each tier's tests that moved most `ci-times --ab` lists.
const TIMES_TOP: usize = 8;

const TIMES_USAGE: &str =
    "usage: cargo xtask ci-times <RUN ID> [--against <RUN ID>|main] [--pr N] [--write FILE]
                            [--section NAME] [--heading TEXT]
       cargo xtask ci-times --ab <TRY RUN ID> [--against REF] [--write FILE]
                            [--section NAME] [--heading TEXT] [--top N]

The first writes the section \"On the pull request\" of a step's ci.md (with
--write), and prints it: which pull request and which main, each job's time,
and each tier's time above the start against its budget. The second does the
same for each OS's section \"<OS>, both trees on one runner\" from a try run
made with -f against=REF, prints the pull request's CI section from it, and
prints, for each OS, each tier's crates and tests that moved (--top N tests a
tier, 8 when not given; 0 prints no detail). What it writes goes between two
marks, and writing again replaces only that; a section of that name it did not
write is refused, and nothing is written.

--section NAME makes what it writes one section called NAME, with marks that
carry the name: a step with several sections (the pull request's, then the
review's fixes') writes each under its own, and writing one again replaces only
that. An A/B's two OSes are then `###` parts of it. --heading TEXT is the
heading a new section is added with, when it should not be its name (a dated
one); with no --section it is the name too, which is how an older page that has
no marks gets a section of its own. A section is added at the end of the page.
A heading of that text that the tool did not write is refused. A page that is
not there, or is empty, is made, with the heading its place in docs/steps/
gives it (`# After step 24: its CI times`), its folder with it.";

impl TimesRequest {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Self {
            top: TIMES_TOP,
            ..Self::default()
        };
        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            let mut value = || {
                rest.next()
                    .ok_or_else(|| format!("`{argument}` needs a value\n{TIMES_USAGE}"))
            };
            match argument.as_str() {
                "--ab" => request.ab = Some(value()?.clone()),
                "--against" => request.against = Some(value()?.clone()),
                "--pr" => {
                    let text = value()?;
                    request.pr = Some(
                        text.parse::<u64>()
                            .map_err(|_| format!("`--pr {text}`: a number\n{TIMES_USAGE}"))?,
                    );
                }
                "--top" => {
                    let text = value()?;
                    request.top = text
                        .parse::<usize>()
                        .map_err(|_| format!("`--top {text}`: a number\n{TIMES_USAGE}"))?;
                }
                "--write" => request.write = Some(PathBuf::from(value()?)),
                "--section" => request.placing.section = Some(value()?.clone()),
                "--heading" => request.placing.heading = Some(value()?.clone()),
                "--help" | "-h" => return Err(TIMES_USAGE.to_owned()),
                other if request.run.is_none() && !other.starts_with('-') => {
                    request.run = Some(other.to_owned());
                }
                other => return Err(format!("`{other}`?\n{TIMES_USAGE}")),
            }
        }
        request.placing.check()?;
        if request.placing.is_given() && request.write.is_none() {
            return Err(format!(
                "`--section` and `--heading` say where `--write` puts the tables\n{TIMES_USAGE}"
            ));
        }
        if request.ab.is_some() && (request.run.is_some() || request.pr.is_some()) {
            return Err(format!("--ab takes a try run alone\n{TIMES_USAGE}"));
        }
        Ok(request)
    }
}

/// `cargo xtask ci-times <run> [--against <run>|main]`.
pub(crate) fn ci_times(arguments: &[String]) -> Result<()> {
    let TimesRequest {
        run,
        ab,
        against,
        pr,
        write,
        placing,
        top,
    } = TimesRequest::parse(arguments)?;
    const USAGE: &str = TIMES_USAGE;
    if let Some(ab) = ab {
        let Source::Run(id) = Source::parse(&ab) else {
            return Err(format!("`{ab}` is not a run's id\n{USAGE}"));
        };
        return crate::records::ab_times(
            id,
            against.as_deref().unwrap_or("main"),
            write.as_deref(),
            &placing,
            top,
        );
    }

    let Some(run) = run else {
        return Err(USAGE.to_owned());
    };
    let run = &run;
    let against = against.as_ref();
    let run = match Source::parse(run) {
        Source::Run(id) => id,
        _ => return Err(format!("`{run}` is not a run's id\n{USAGE}")),
    };
    let needs = needs_of(&[Os::Linux, Os::Windows]);
    let against = match against.map(|text| Source::parse(text)) {
        // A pull request's own run is compared with `main` where the pull
        // request is based, not with `main`'s latest.
        None | Some(Source::Main) if pr.is_some() => {
            let head = run_info(run)?.sha;
            match base_main_run(&head, &needs)? {
                Some(base) => Some(base),
                None => Some(latest_main_run(&needs)?),
            }
        }
        None => None,
        Some(Source::File(path)) => {
            return Err(format!(
                "`{}` is not a run's id or `main`\n{USAGE}",
                path.display()
            ));
        }
        Some(source) => run_of(&source, &needs)?,
    };
    let mut runs = Vec::new();
    for id in against.into_iter().chain([run]) {
        let info = run_info(id)?;
        let mut reports = Vec::new();
        for os in [Os::Linux, Os::Windows] {
            // A run whose tests did not run (a documentation change, a
            // cancelled job) has no report for that OS, and its column says
            // so. Anything else that keeps a report from being read is an
            // error: a table without the column would not say why.
            let cases = match junit_of(id, os)? {
                Some(path) => Some(junit::read(&path)?),
                None => {
                    println!(
                        "run {id} kept no whole report of its tests ({}): its tests did not \
                         run, or not all of them",
                        Wanted::ci_report(os).describe()
                    );
                    None
                }
            };
            reports.push(cases);
        }
        runs.push(Measured {
            info,
            linux: reports[0].take(),
            windows: reports[1].take(),
        });
    }
    // A report with failed tests is no measurement of them (a timeout reads
    // as a test of its limit's length): said, and never written into a
    // step's record.
    let failed = failed_reports(&runs);
    for line in &failed {
        println!("warning: {line}");
    }
    if write.is_some() && !failed.is_empty() {
        return Err(format!(
            "not written: {}. A step's record holds the times of runs whose tests passed.",
            failed.join("; ")
        ));
    }
    crate::records::pr_times(&runs, pr, write.as_deref(), &placing)
}

/// The reports of `runs` that hold a failed test, one line each.
pub(crate) fn failed_reports(runs: &[Measured]) -> Vec<String> {
    let mut lines = Vec::new();
    for run in runs {
        for (os, cases) in [("Linux", &run.linux), ("Windows", &run.windows)] {
            let failed: Vec<String> = cases
                .iter()
                .flatten()
                .filter(|case| case.failed)
                .map(TestCase::id)
                .collect();
            if !failed.is_empty() {
                lines.push(format!(
                    "run {}'s {os} report has {} failed test(s) ({}{})",
                    run.info.id,
                    failed.len(),
                    failed
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", "),
                    if failed.len() > 3 { ", …" } else { "" }
                ));
            }
        }
    }
    lines
}

/// A run, and its test jobs' reports where they exist.
pub(crate) struct Measured {
    pub(crate) info: RunInfo,
    pub(crate) linux: Option<Vec<TestCase>>,
    pub(crate) windows: Option<Vec<TestCase>>,
}

/// Where a run's artifacts are downloaded: a folder for each run under the
/// build directory, read from there afterwards.
fn download_root() -> PathBuf {
    workspace::root().join("target").join("xtask-runs")
}

#[cfg(test)]
mod artifact_tests {
    use super::{Kept, Listed, Wanted, first_that_kept, kept, pairs_of, parse_listing};
    use crate::tiers::Os;

    #[test]
    fn a_report_is_absent_only_when_the_run_made_none() {
        let listed = parse_listing("junit-linux false 11\njunit-windows true 12\n").unwrap();
        let artifacts = pairs_of(&listed);
        assert_eq!(kept(&artifacts, "junit-linux"), Kept::Yes);
        assert_eq!(kept(&artifacts, "junit-windows"), Kept::Expired);
        assert_eq!(kept(&artifacts, "junit-other"), Kept::Never);
        assert_eq!(listed[1].id, 12);
        assert_eq!(parse_listing("").unwrap(), []);
        assert!(parse_listing("junit-linux maybe 3\n").is_err());
        assert!(parse_listing("junit-linux false\n").is_err());
        assert!(parse_listing("junit-linux false x\n").is_err());
        // A name with a space in it keeps it.
        assert_eq!(
            parse_listing("a b false 7").unwrap(),
            [Listed {
                name: "a b".to_owned(),
                expired: false,
                id: 7
            }]
        );
    }

    /// A download is read again only while GitHub lists the uploads it was
    /// made of: a job run again replaces its report (`overwrite`), with a
    /// new id, and a marker of the older form (empty) is no stamp.
    #[test]
    fn a_download_is_reused_only_while_its_uploads_are_the_ones_listed() {
        let first =
            parse_listing("junit-windows-tiers false 1\njunit-windows-system false 2\n").unwrap();
        let rerun =
            parse_listing("junit-windows-tiers false 1\njunit-windows-system false 9\n").unwrap();
        let parts = ["junit-windows-tiers", "junit-windows-system"];
        let stamp = super::stamp(&first, &parts).unwrap();
        assert_eq!(stamp, "junit-windows-tiers=1,junit-windows-system=2");
        assert_ne!(super::stamp(&rerun, &parts).unwrap(), stamp);
        assert_eq!(super::stamp(&first, &["junit-linux"]), None);

        let dir = std::env::temp_dir().join(format!("xtask-runs-stamp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join(".stamp");
        let missing = super::is_current(&marker, &stamp);
        std::fs::write(&marker, "").unwrap();
        let old_form = super::is_current(&marker, &stamp);
        std::fs::write(&marker, &stamp).unwrap();
        let same = super::is_current(&marker, &stamp);
        let replaced = super::is_current(&marker, &super::stamp(&rerun, &parts).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            (missing, old_form, same, replaced),
            (false, false, true, false)
        );
    }

    #[test]
    fn main_is_the_newest_run_that_kept_every_report_asked_for() {
        let named = |names: &[(&str, bool)]| {
            names
                .iter()
                .map(|(name, expired)| ((*name).to_owned(), *expired))
                .collect::<Vec<_>>()
        };
        let runs = vec![
            // A change of documentation alone: no report.
            (30, named(&[])),
            (29, named(&[("junit-linux", false)])),
            (
                28,
                named(&[("junit-linux", false), ("junit-windows", true)]),
            ),
            (
                27,
                named(&[("junit-linux", false), ("junit-windows", false)]),
            ),
        ];
        let linux = Wanted::ci_report(Os::Linux);
        let windows = Wanted::ci_report(Os::Windows);
        assert_eq!(
            first_that_kept(&runs, std::slice::from_ref(&linux)),
            Some(29)
        );
        assert_eq!(
            first_that_kept(&runs, &[linux.clone(), windows.clone()]),
            Some(27)
        );
        assert_eq!(first_that_kept(&runs[..3], &[windows]), None);
        assert_eq!(
            first_that_kept(&runs, &[Wanted::one("junit-windows")]),
            Some(27)
        );
    }

    /// Since the Windows job was split, a run keeps Windows' report in two
    /// parts: a run of `main` kept it when it kept both, and a run that
    /// kept one part only did not.
    #[test]
    fn a_split_run_kept_its_windows_report_when_it_kept_both_parts() {
        let named = |names: &[&str]| {
            names
                .iter()
                .map(|name| ((*name).to_owned(), false))
                .collect::<Vec<_>>()
        };
        let runs = vec![
            (40, named(&["junit-linux", "junit-windows-tiers"])),
            (39, named(&["junit-linux", "junit-windows-system"])),
            (
                38,
                named(&["junit-linux", "junit-windows-tiers", "junit-windows-system"]),
            ),
        ];
        let needs = [Wanted::ci_report(Os::Linux), Wanted::ci_report(Os::Windows)];
        assert_eq!(first_that_kept(&runs, &needs), Some(38));
        // Neither part alone stands for the report.
        assert_eq!(first_that_kept(&runs[..2], &needs), None);
        assert_eq!(
            Wanted::ci_report(Os::Windows).describe(),
            "`junit-windows-tiers` and `junit-windows-system`, or `junit-windows`"
        );
        assert_eq!(Wanted::ci_report(Os::Linux).describe(), "`junit-linux`");
    }

    /// The names the readers look for are the ones the `ci` workflow
    /// uploads: each part's artifact is named in its job's matrix, and the
    /// Linux one in its job.
    #[test]
    fn the_report_artifacts_are_named_as_the_ci_workflow_names_them() {
        let workflow =
            std::fs::read_to_string(crate::workspace::root().join(".github/workflows/ci.yml"))
                .unwrap();
        for os in [Os::Linux, Os::Windows] {
            let current = super::report_forms(os)[0];
            for name in current {
                assert!(
                    workflow.contains(&format!("junit: {name}\n"))
                        || workflow.contains(&format!("name: {name}\n")),
                    "ci.yml does not upload `{name}`"
                );
            }
        }
        // Tiers 1 and 2 first, as the one job's report held them.
        assert_eq!(
            super::report_forms(Os::Windows)[0],
            ["junit-windows-tiers", "junit-windows-system"]
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{Measured, RunInfo, Source, download_root, failed_reports, parse_run, seconds_of};
    use crate::junit;

    #[test]
    fn a_report_with_a_failed_test_is_named_as_no_measurement() {
        let info = |id: u64| RunInfo {
            id,
            sha: "abc".to_owned(),
            branch: "b".to_owned(),
            event: "pull_request".to_owned(),
            date: "2026-11-16".to_owned(),
            jobs: Vec::new(),
        };
        let mut failed = junit::tests::case("cineris-base", "t::a", 0.1);
        failed.failed = true;
        let runs = [
            Measured {
                info: info(1),
                linux: Some(vec![junit::tests::case("cineris-base", "t::a", 0.1)]),
                windows: None,
            },
            Measured {
                info: info(2),
                linux: None,
                windows: Some(vec![failed]),
            },
        ];
        assert_eq!(
            failed_reports(&runs),
            ["run 2's Windows report has 1 failed test(s) (cineris-base t::a)"]
        );
        assert_eq!(failed_reports(&runs[..1]), Vec::<String>::new());
    }

    #[test]
    fn a_source_is_main_a_run_or_a_file() {
        assert_eq!(Source::parse("main"), Source::Main);
        assert_eq!(Source::parse("19283746501"), Source::Run(19_283_746_501));
        assert_eq!(
            Source::parse("junit/junit.xml"),
            Source::File("junit/junit.xml".into())
        );
        // Too many digits for a run's id: a file of that name.
        assert_eq!(
            Source::parse("99999999999999999999999"),
            Source::File("99999999999999999999999".into())
        );
        assert!(download_root().ends_with("target/xtask-runs"));
    }

    #[test]
    fn github_times_are_read() {
        assert_eq!(seconds_of("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(seconds_of("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(seconds_of("2026-11-13T10:02:03Z"), Some(1_794_564_123));
        // A job that never started.
        assert_eq!(seconds_of("0001-01-01T00:00:00Z"), None);
        assert_eq!(seconds_of("yesterday"), None);
    }

    /// The rule for a run that waits for a runner, at fixed times: no clock
    /// is read and nothing waits.
    #[test]
    fn a_run_that_sat_queued_with_no_jobs_past_the_limit_is_named_and_no_other() {
        use super::{jobs_started, queued_warn_minutes, sat_queued, sat_queued_text};
        let created = seconds_of("2026-11-16T10:00:00Z").unwrap();
        let at = |time: &str| seconds_of(time).unwrap();
        // Ten minutes is the limit, not past it; a second more is.
        assert_eq!(
            sat_queued("queued", created, at("2026-11-16T10:10:00Z"), 0, 10),
            None
        );
        assert_eq!(
            sat_queued("queued", created, at("2026-11-16T10:10:01Z"), 0, 10),
            Some(10)
        );
        assert_eq!(
            sat_queued("queued", created, at("2026-11-16T10:42:59Z"), 0, 10),
            Some(42)
        );
        // GitHub's other words for a run that waits.
        for status in ["waiting", "pending", "requested"] {
            assert_eq!(
                sat_queued(status, created, at("2026-11-16T10:30:00Z"), 0, 10),
                Some(30),
                "{status}"
            );
        }
        // In progress: only while no job has started.
        assert_eq!(
            sat_queued("in_progress", created, at("2026-11-16T10:30:00Z"), 0, 10),
            Some(30)
        );
        assert_eq!(
            sat_queued("in_progress", created, at("2026-11-16T10:30:00Z"), 1, 10),
            None
        );
        // A finished run, and a limit of the caller's own.
        assert_eq!(
            sat_queued("completed", created, at("2026-11-16T11:00:00Z"), 0, 10),
            None
        );
        assert_eq!(
            sat_queued("queued", created, at("2026-11-16T10:30:00Z"), 0, 45),
            None
        );
        // A clock behind the run's creation is no time at all.
        assert_eq!(sat_queued("queued", created, created - 600, 0, 10), None);
        assert_eq!(
            sat_queued_text(77, "try", 12),
            "run 77 (try) has sat queued with no jobs for 12 min: GitHub has not given it a \
             runner. Cancel and dispatch it again (gh run cancel 77), or keep waiting."
        );
        let jobs: serde_json::Value = serde_json::from_str(
            r#"[{"status":"queued"},{"status":"in_progress"},{"status":"completed"},{"status":"waiting"},{}]"#,
        )
        .unwrap();
        assert_eq!(jobs_started(&jobs), 2);
        // A run as gh describes it: queued, its jobs listed and none given a
        // runner; then one of them started.
        let view = r#"{"workflowName":"try","status":"in_progress","createdAt":"2026-11-16T10:00:00Z","jobs":[{"status":"queued"},{"status":"queued"}]}"#;
        let note = super::queued_note(view, 9, at("2026-11-16T10:15:00Z"), 10).unwrap();
        assert_eq!(note, Some(sat_queued_text(9, "try", 15)));
        let one_started = view.replacen("queued", "in_progress", 1);
        assert_eq!(
            super::queued_note(&one_started, 9, at("2026-11-16T10:15:00Z"), 10),
            Ok(None)
        );
        assert_eq!(
            super::queued_note(view, 9, at("2026-11-16T10:05:00Z"), 10),
            Ok(None)
        );
        assert!(super::queued_note("not json", 9, 0, 10).is_err());
        assert_eq!(queued_warn_minutes("--queued-warn", "25"), Ok(25));
        for text in ["0", "x", "-3", "1441"] {
            assert!(
                queued_warn_minutes("--queued-limit", text)
                    .unwrap_err()
                    .contains("`--queued-limit"),
                "{text}"
            );
        }
    }

    /// What `gh run view --json …` printed for a run, cut down.
    const RUN: &str = r#"{"createdAt":"2026-11-13T09:58:00Z","databaseId":4242,"event":"pull_request","headBranch":"step-x","headSha":"0123456789abcdef","jobs":[
        {"name":"lint-linux","startedAt":"2026-11-13T10:00:00Z","completedAt":"2026-11-13T10:00:48Z","conclusion":"success"},
        {"name":"test-linux","startedAt":"2026-11-13T10:00:00Z","completedAt":"2026-11-13T10:00:56Z","conclusion":"success"},
        {"name":"test-windows","startedAt":"2026-11-13T10:00:05Z","completedAt":"2026-11-13T10:02:23Z","conclusion":"success"},
        {"name":"lint-windows","startedAt":"0001-01-01T00:00:00Z","completedAt":"0001-01-01T00:00:00Z","conclusion":"cancelled"},
        {"name":"pr-size-${{ matrix.name }}","startedAt":"2026-11-13T10:02:24Z","completedAt":"2026-11-13T10:02:23Z","conclusion":"skipped"}]}"#;

    #[test]
    fn a_run_is_read_from_what_gh_prints() {
        let info = parse_run(RUN).unwrap();
        assert_eq!(
            info,
            RunInfo {
                id: 4242,
                sha: "0123456789abcdef".to_owned(),
                branch: "step-x".to_owned(),
                event: "pull_request".to_owned(),
                date: "2026-11-13".to_owned(),
                jobs: vec![
                    ("lint-linux".to_owned(), Some(48)),
                    ("test-linux".to_owned(), Some(56)),
                    ("test-windows".to_owned(), Some(138)),
                    ("lint-windows".to_owned(), None),
                ],
            }
        );
        assert!(parse_run("not json").is_err());
    }

    #[test]
    fn the_tables_are_those_of_a_steps_ci_page_and_name_main_and_the_pr() {
        let main = RunInfo {
            id: 4000,
            sha: "fedcba9876543210".to_owned(),
            branch: "main".to_owned(),
            event: "push".to_owned(),
            date: "2026-11-12".to_owned(),
            jobs: vec![
                ("lint-linux".to_owned(), Some(52)),
                ("test-linux".to_owned(), Some(68)),
            ],
        };
        let runs = [
            Measured {
                info: main,
                linux: Some(junit::parse(include_str!(
                    "../fixtures/tiers/compare-main.xml"
                ))),
                windows: None,
            },
            Measured {
                info: parse_run(RUN).unwrap(),
                linux: Some(junit::parse(include_str!(
                    "../fixtures/tiers/compare-branch.xml"
                ))),
                windows: Some(junit::parse(include_str!("../fixtures/tiers/junit.xml"))),
            },
        ];
        assert_eq!(
            crate::records::pr_section(&runs, Some(46)),
            "Pull request 46, on `main` at `fedcba9`. \"Main\" is main's own run at `fedcba9` (run 4000).
The PR's run is 4242, at `0123456`.

| Job | Main, `fedcba9` | The PR at `0123456` |
|---|---|---|
| `lint-linux` | 52 s | 48 s |
| `test-linux` | 68 s | 56 s |
| `lint-windows` | — | — |
| `test-windows` | — | 138 s |

| Time above process start, against the budget | Linux, main (10 ms) | Linux, `0123456` (20 ms) | Windows, `0123456` (200 ms) |
|---|---|---|---|
| Tier 1, unit (20 s on Linux, 25 s on Windows) | 1.0 s | 1.2 s | 5.0 s |
| Tier 2, integration (40 s) | 1.5 s | 3.5 s | 9.2 s |
| Tier 3, system (60 s on Linux, 90 s on Windows) | 2.0 s | 2.0 s | 14.2 s |
"
        );
    }
}

#[cfg(test)]
mod try_artifact_tests {
    use std::path::{Path, PathBuf};

    use super::{
        Chosen, Resolved, TimesRequest, ab_report_name, ends_with_names, files_under, needs_of,
        report_of_try, which_artifact,
    };

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xtask-runs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn keep(root: &Path, relative: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "x").unwrap();
    }

    #[test]
    fn an_ab_reports_name_says_its_side_and_round_and_nothing_else_is_one() {
        assert_eq!(ab_report_name("base-round-3.xml"), Some(("base", 3)));
        assert_eq!(ab_report_name("head-round-12.xml"), Some(("head", 12)));
        for not in [
            "other-round-1.xml",
            "base-round-x.xml",
            "base-round-1.json",
            "junit.xml",
            "base-round-.xml",
        ] {
            assert_eq!(ab_report_name(not), None, "{not}");
        }
    }

    #[test]
    fn a_path_ends_with_names_whatever_the_separator_or_the_depth() {
        let path = Path::new("work")
            .join("repo")
            .join("target")
            .join("nextest")
            .join("ci")
            .join("junit.xml");
        assert!(ends_with_names(&path, &["nextest", "ci", "junit.xml"]));
        assert!(ends_with_names(&path, &["junit.xml"]));
        assert!(!ends_with_names(&path, &["nextest", "stress", "junit.xml"]));
        // A folder of that name is not enough: it is the whole tail.
        assert!(!ends_with_names(&path, &["ci", "nextest", "junit.xml"]));
        assert!(!ends_with_names(
            Path::new("junit.xml"),
            &["ci", "junit.xml"]
        ));
    }

    #[test]
    fn a_try_runs_report_is_found_wherever_the_runners_paths_put_it() {
        let dir = scratch("try");
        keep(&dir, "tmp/try-output.log");
        keep(&dir, "work/repo/target/nextest/stress/junit.xml");
        keep(&dir, "work/repo/target/nextest/ci/junit.xml");
        let found = report_of_try(5, &dir).unwrap().unwrap();
        assert!(
            ends_with_names(&found, &["nextest", "ci", "junit.xml"]),
            "{found:?}"
        );
        // Files come in path order, whatever the disk's.
        let files = files_under(&dir).unwrap();
        let mut sorted = files.clone();
        sorted.sort();
        assert_eq!(files, sorted);
        assert_eq!(files.len(), 3);

        // A run of another command has no such report: none, not an error.
        let startup = scratch("startup");
        keep(&startup, "tmp/try-output.log");
        assert_eq!(report_of_try(6, &startup).unwrap(), None);

        // An A/B run has two trees' rounds and no one report: refused, and
        // the message says which commands read it.
        keep(&dir, "work/repo/target/ab/head-round-1.xml");
        let error = report_of_try(7, &dir).unwrap_err();
        assert!(error.contains("run 7 is a `try` A/B run"), "{error}");
        assert!(
            error.contains("ci-times --ab 7") && error.contains("try-result 7"),
            "{error}"
        );
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(startup);
    }

    #[test]
    fn a_report_is_the_ci_workflows_or_failing_that_a_try_runs_and_an_expired_one_is_an_error() {
        use crate::tiers::Os;
        let listed = |names: &[(&str, bool)]| {
            names
                .iter()
                .map(|(name, expired)| ((*name).to_owned(), *expired))
                .collect::<Vec<_>>()
        };
        // A `ci` run from before the split: its own report.
        let ci = listed(&[("junit-linux", false), ("junit-windows", false)]);
        assert_eq!(
            which_artifact(1, &ci, Os::Windows).unwrap(),
            Chosen::Ci(&["junit-windows"])
        );
        assert_eq!(
            which_artifact(1, &ci, Os::Linux).unwrap(),
            Chosen::Ci(&["junit-linux"])
        );
        // A split run: both parts, tiers 1 and 2 first.
        let split = listed(&[
            ("junit-linux", false),
            ("junit-windows-system", false),
            ("junit-windows-tiers", false),
        ]);
        assert_eq!(
            which_artifact(1, &split, Os::Windows).unwrap(),
            Chosen::Ci(&["junit-windows-tiers", "junit-windows-system"])
        );
        // One part only: no whole report, said, and never the one part read
        // as the whole.
        let cut = listed(&[("junit-linux", false), ("junit-windows-tiers", false)]);
        assert_eq!(
            which_artifact(1, &cut, Os::Windows).unwrap(),
            Chosen::Partial {
                kept: "junit-windows-tiers",
                missing: "junit-windows-system"
            }
        );
        // A part that has expired is an error, as a whole report's is.
        let old_part = listed(&[
            ("junit-windows-tiers", false),
            ("junit-windows-system", true),
        ]);
        let error = which_artifact(1, &old_part, Os::Windows).unwrap_err();
        assert!(
            error.contains("`junit-windows-system` has expired"),
            "{error}"
        );
        // A `try` run of Windows alone: Windows' is there and Linux's is not.
        let tried = listed(&[("try-windows", false)]);
        assert_eq!(which_artifact(1, &tried, Os::Windows).unwrap(), Chosen::Try);
        assert_eq!(which_artifact(1, &tried, Os::Linux).unwrap(), Chosen::None);
        // A run that kept nothing (a change of documentation): none.
        assert_eq!(which_artifact(1, &[], Os::Linux).unwrap(), Chosen::None);
        // One that has expired is an error naming it, never "none".
        let old = listed(&[("junit-linux", true), ("try-windows", true)]);
        let error = which_artifact(1, &old, Os::Linux).unwrap_err();
        assert!(error.contains("`junit-linux` has expired"), "{error}");
        let error = which_artifact(1, &old, Os::Windows).unwrap_err();
        assert!(error.contains("`try-windows` has expired"), "{error}");
    }

    #[test]
    fn a_run_of_main_must_have_kept_every_oses_report_so_that_both_are_of_one_commit() {
        use super::Wanted;
        use crate::tiers::Os;
        assert_eq!(needs_of(&[Os::Windows]), [Wanted::ci_report(Os::Windows)]);
        assert_eq!(
            needs_of(&[Os::Linux, Os::Windows]),
            [Wanted::ci_report(Os::Linux), Wanted::ci_report(Os::Windows)]
        );
    }

    #[test]
    fn a_report_on_disk_is_resolved_to_itself_and_a_run_to_its_id() {
        use super::{Source, resolve};
        assert_eq!(
            resolve(&Source::File("a.xml".into()), &[crate::tiers::Os::Linux]).unwrap(),
            Resolved::File("a.xml".into())
        );
        assert_eq!(
            resolve(
                &Source::Run(9),
                &[crate::tiers::Os::Linux, crate::tiers::Os::Windows]
            )
            .unwrap(),
            Resolved::Run { id: 9, main: false }
        );
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn ci_times_reads_its_sections_and_refuses_what_it_could_not_place() {
        let request = TimesRequest::parse(&strings(&[
            "42",
            "--against",
            "main",
            "--pr",
            "56",
            "--write",
            "ci.md",
            "--section",
            "fixes",
            "--heading",
            "The fixes, 2026-11-16",
        ]))
        .unwrap();
        assert_eq!(request.run.as_deref(), Some("42"));
        assert_eq!(request.pr, Some(56));
        assert_eq!(request.placing.section.as_deref(), Some("fixes"));
        assert_eq!(
            request.placing.heading.as_deref(),
            Some("The fixes, 2026-11-16")
        );
        let ab = TimesRequest::parse(&strings(&["--ab", "9", "--top", "3"])).unwrap();
        assert_eq!((ab.ab.as_deref(), ab.top), (Some("9"), 3));
        assert_eq!(
            TimesRequest::parse(&strings(&["--ab", "9"])).unwrap().top,
            8
        );
        for (arguments, expected) in [
            (
                &["1", "--section", "x"][..],
                "say where `--write` puts the tables",
            ),
            (
                &["1", "--heading", "x"],
                "say where `--write` puts the tables",
            ),
            (&["1", "--write", "f", "--section", "a\"b"], "double quote"),
            (
                &["1", "--write", "f", "--heading", "## x"],
                "the `## ` before it",
            ),
            (&["--ab", "9", "1"], "--ab takes a try run alone"),
            (&["--ab", "9", "--pr", "1"], "--ab takes a try run alone"),
            (&["--top", "x"], "`--top x`: a number"),
            (&["--section"], "needs a value"),
        ] {
            let error = TimesRequest::parse(&strings(arguments)).unwrap_err();
            assert!(error.contains(expected), "{arguments:?}: {error}");
        }
    }

    /// A split run's two parts are made one report, tiers 1 and 2 first,
    /// the system tests held above the start their probe measured; a part
    /// with no report of its own (its probe's alone) makes no report.
    #[test]
    fn a_split_runs_parts_are_one_report_with_the_system_tests_above_their_own_start() {
        let suite = |binary: &str, name: &str, time: &str| {
            format!(
                "<testsuites name=\"nextest-run\">\n<testsuite name=\"{binary}\"><testcase \
                 name=\"{name}\" classname=\"{binary}\" time=\"{time}\"/></testsuite>\n\
                 </testsuites>\n"
            )
        };
        let dir = scratch("merge-parts");
        let (tiers, system) = (dir.join("tiers"), dir.join("system"));
        std::fs::create_dir_all(&tiers).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(
            tiers.join("junit.xml"),
            suite("cineris-base", "t::a", "0.010"),
        )
        .unwrap();
        std::fs::write(
            system.join("junit.xml"),
            suite("cineris-cli::system", "t::sys_a", "1.000"),
        )
        .unwrap();
        let probe: String = (0..600)
            .map(|index| {
                format!(
                    "<testsuite name=\"cineris-base\"><testcase name=\"t::q{index}\" \
                     classname=\"cineris-base\" time=\"0.020\"/></testsuite>\n"
                )
            })
            .collect();
        std::fs::write(
            system.join(crate::tiers::START_PROBE_FILE),
            format!("<testsuites name=\"nextest-run\">\n{probe}</testsuites>\n"),
        )
        .unwrap();
        let parts = [
            ("junit-windows-tiers", tiers.clone()),
            ("junit-windows-system", system.clone()),
        ];
        let merged = super::merge_parts(1, &parts).unwrap().unwrap();
        let cases = crate::junit::parse(&merged);
        let names: Vec<(&str, Option<f64>)> = cases
            .iter()
            .map(|case| (case.name.as_str(), case.start))
            .collect();
        assert_eq!(names, [("t::a", None), ("t::sys_a", Some(0.02))]);
        // A probe the job's own check refused measured no start: no report.
        std::fs::write(
            system.join(crate::tiers::START_PROBE_FILE),
            "<testsuites name=\"nextest-run\">\n<testsuite name=\"cineris-base\"><testcase \
             name=\"t::q\" classname=\"cineris-base\" time=\"0.020\"/></testsuite>\n\
             </testsuites>\n",
        )
        .unwrap();
        assert_eq!(super::merge_parts(1, &parts).unwrap(), None);
        // A part that kept its probe's report and not its own.
        std::fs::remove_file(system.join("junit.xml")).unwrap();
        let none = super::merge_parts(1, &parts).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(none, None);
    }
}
