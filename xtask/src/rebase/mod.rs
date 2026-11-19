//! `cargo xtask rebase`: the current branch rebased onto the main branch,
//! with the conflicts that have a mechanical answer settled, and the rest
//! named for a person (`docs/testing-and-ci.md`, "Rebasing onto main").
//!
//! It fetches, then runs `git rebase`. Each time git stops, every
//! conflicted file is put in a class ([`classify`]) and settled by it: a
//! generated file and the open items' index take the side rebased onto and
//! are written again at the end; additions on both sides at one place in a
//! Rust or Markdown file are both kept, when that is all they are and the
//! result checks; anything else is left conflicted. When nothing is left the
//! rebase goes on; when something is, it stops there, says what, and
//! `rebase --continue` takes it up again once a person has settled it.
//!
//! **It never drops a side silently.** A union is written only when every
//! line of both sides is in it, in order, and it checks (balanced brackets
//! and rustfmt for Rust, the table's shape for Markdown rows). One union
//! made by hand in an earlier wave lost a brace: that is what the checks are
//! for.
//!
//! Each step is one git with plain arguments, so the command is one call
//! with no loop, heredoc or `$(git …)` for a shell to refuse.

pub(crate) mod classify;
pub(crate) mod merge;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Result;
use crate::branch::{Call, Git};
use classify::{Class, Resolution, Stages};

const USAGE: &str = "\
usage: cargo xtask rebase [--onto REF] [--no-fetch] [--no-regen]
       cargo xtask rebase --continue [--no-regen]
       cargo xtask rebase --abort

Fetches REF's remote (origin/main when not given) and rebases the current
branch onto REF. Each time git stops on a conflict, each conflicted file is
settled by its class:

  generated     what `cargo xtask regen` writes (docs/protocol/,
                docs/configuration/, the snapshots, the reference screens;
                not the recorded provider fixtures): the side rebased onto
                is taken, and the generator runs at the end.
  index         docs/outstanding/README.md: the side rebased onto, and
                `cargo xtask outstanding` at the end (which runs after every
                rebase, as git merges the index line by line).
  additions     a .rs or .md file where both sides only added lines at the
                same place: both additions, the side rebased onto first,
                when every line of both sides is in the result in its order,
                and it checks: brackets balance and rustfmt leaves it as it
                is (Rust); every line added is a table's row with the shape
                of its table (Markdown).
  anything else left conflicted, for a person.

When every file is settled the rebase goes on. When one is left, it stops
there and names it, with the reason; settle it, `git add` it, and run
`cargo xtask rebase --continue`. It ends with what it settled and how.

  --onto REF   rebase onto REF; its remote (the part before the first /)
               is fetched first, if it is one.
  --no-fetch   fetch nothing.
  --no-regen   at the end, name the generators to run rather than run them.
  --continue   carry on after a stop (the files still conflicted are tried
               again, and left if they are still not settled).
  --abort      `git rebase --abort`: the branch as it was before.

The working tree must be clean to start. The branch's tip before the rebase
is printed, with the command that takes the branch back to it.";

/// What `cargo xtask rebase` was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Mode {
    Start,
    Continue,
    Abort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    pub(crate) mode: Mode,
    pub(crate) onto: String,
    pub(crate) fetch: bool,
    pub(crate) regen: bool,
}

impl Request {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Self {
            mode: Mode::Start,
            onto: "origin/main".to_owned(),
            fetch: true,
            regen: true,
        };
        let mut onto_given = false;
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            match word.as_str() {
                "--onto" => {
                    let value = words
                        .next()
                        .filter(|value| !value.starts_with('-') && !value.is_empty())
                        .ok_or_else(|| format!("`--onto` needs a ref\n\n{USAGE}"))?;
                    request.onto = value.clone();
                    onto_given = true;
                }
                "--no-fetch" => request.fetch = false,
                "--no-regen" => request.regen = false,
                "--continue" => request.mode = Mode::Continue,
                "--abort" => request.mode = Mode::Abort,
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
            }
        }
        if request.mode != Mode::Start && (onto_given || !request.fetch) {
            return Err(format!(
                "`--onto` and `--no-fetch` are for starting a rebase\n\n{USAGE}"
            ));
        }
        if request.mode == Mode::Abort && !request.regen {
            return Err(format!("`--abort` takes nothing else\n\n{USAGE}"));
        }
        Ok(request)
    }
}

/// What was done with one conflicted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum How {
    /// The side rebased onto, to be written again by the generator named.
    Taken(&'static str),
    /// Both sides' additions, at this many places.
    United(usize),
    /// Left for a person, and why.
    Left(String),
}

/// One stop of the rebase: the commit git was putting on, and its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stop {
    /// `<short hash> <subject>`.
    pub(crate) commit: String,
    pub(crate) files: Vec<(String, How)>,
}

/// What a run did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Report {
    /// The branch, or `HEAD` when it is detached.
    pub(crate) branch: String,
    /// Its tip before the rebase.
    pub(crate) old: Option<String>,
    pub(crate) onto: String,
    pub(crate) stops: Vec<Stop>,
    /// Commits dropped as empty once their conflicts were settled.
    pub(crate) dropped: Vec<String>,
    /// Whether the rebase is done; if not, it stopped for a person.
    pub(crate) finished: bool,
    /// What was done at the end: the index and the generators.
    pub(crate) after: Vec<String>,
}

/// What a run keeps between a stop and `--continue`, in the git directory:
/// the tip before the rebase, and the generators of the files taken.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct State {
    pub(crate) old: Option<String>,
    pub(crate) onto: Option<String>,
    pub(crate) generators: BTreeSet<String>,
}

impl State {
    pub(crate) fn parse(text: &str) -> Self {
        let mut state = Self::default();
        for line in text.lines() {
            match line.split_once(' ') {
                Some(("old", hash)) => state.old = Some(hash.to_owned()),
                Some(("onto", reference)) => state.onto = Some(reference.to_owned()),
                Some(("generator", name)) => {
                    state.generators.insert(name.to_owned());
                }
                _ => {}
            }
        }
        state
    }

    pub(crate) fn text(&self) -> String {
        let mut text = String::new();
        if let Some(old) = &self.old {
            text.push_str(&format!("old {old}\n"));
        }
        if let Some(onto) = &self.onto {
            text.push_str(&format!("onto {onto}\n"));
        }
        for name in &self.generators {
            text.push_str(&format!("generator {name}\n"));
        }
        text
    }
}

/// The files `git ls-files -u -z` lists, each with the blob of each of its
/// stages (base, ours, theirs) that it has.
pub(crate) fn unmerged(listed: &str) -> Result<BTreeMap<String, [Option<String>; 3]>> {
    let mut files: BTreeMap<String, [Option<String>; 3]> = BTreeMap::new();
    for entry in listed.split('\0').filter(|entry| !entry.is_empty()) {
        let (header, path) = entry
            .split_once('\t')
            .ok_or_else(|| format!("git listed `{entry}`, which is not an unmerged entry"))?;
        let words: Vec<&str> = header.split_whitespace().collect();
        let [_, hash, stage] = words.as_slice() else {
            return Err(format!(
                "git listed `{entry}`, which is not an unmerged entry"
            ));
        };
        let stage: usize = stage
            .parse()
            .ok()
            .filter(|stage| (1..=3).contains(stage))
            .ok_or_else(|| format!("git listed `{entry}`, with no stage 1 to 3"))?;
        files.entry(path.to_owned()).or_default()[stage - 1] = Some((*hash).to_owned());
    }
    Ok(files)
}

/// The command a run uses for each thing it needs from outside git: whether
/// a Rust text is formatted, and what is done at the end with the
/// generators named (the open items' index always among them).
pub(crate) struct Hooks<'a> {
    pub(crate) formatted: &'a dyn Fn(&str) -> std::result::Result<(), String>,
    pub(crate) finish: &'a mut dyn FnMut(&BTreeSet<String>) -> Result<Vec<String>>,
}

/// git with its editor turned off: `rebase --continue` takes each commit's
/// message as it is.
fn quiet<'a>() -> Call<'a> {
    Call {
        env: vec![("GIT_EDITOR", ":".to_owned())],
        ..Call::default()
    }
}

/// Whether a rebase is in progress in the git directory `dir` (this
/// working tree's own, where git keeps a rebase's state).
fn in_progress(dir: &Path) -> bool {
    dir.join("rebase-merge").is_dir() || dir.join("rebase-apply").is_dir()
}

/// Where a run keeps its [`State`].
fn state_file(dir: &Path) -> PathBuf {
    dir.join("xtask-rebase")
}

fn read_state(dir: &Path) -> State {
    State::parse(&std::fs::read_to_string(state_file(dir)).unwrap_or_default())
}

fn write_state(dir: &Path, state: &State) -> Result<()> {
    let path = state_file(dir);
    std::fs::write(&path, state.text())
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

fn forget_state(dir: &Path) -> Result<()> {
    let path = state_file(dir);
    match std::fs::remove_file(&path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("could not remove {}: {error}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Rebases as `request` asks, in the repository whose top is `top`.
pub(crate) fn rebase(
    git: &Git,
    top: &Path,
    request: &Request,
    hooks: &mut Hooks<'_>,
) -> Result<Report> {
    // The branch and this working tree's git directory, in one git.
    let asked = git.run(&["rev-parse", "--abbrev-ref", "HEAD", "--git-dir"])?;
    let mut asked = asked.lines();
    let (Some(branch), Some(dir)) = (asked.next(), asked.next()) else {
        return Err("git named no branch or no git directory".to_owned());
    };
    let dir = &top.join(dir);
    let mut report = Report {
        branch: branch.to_owned(),
        onto: request.onto.clone(),
        ..Report::default()
    };
    let mut state = match request.mode {
        Mode::Abort => {
            if !in_progress(dir) {
                return Err("no rebase is in progress".to_owned());
            }
            git.run(&["rebase", "--abort"])?;
            forget_state(dir)?;
            report
                .after
                .push("The rebase was aborted: the branch is as it was.".to_owned());
            report.finished = true;
            return Ok(report);
        }
        Mode::Continue => {
            if !in_progress(dir) {
                return Err(
                    "no rebase is in progress: start one with `cargo xtask rebase`".to_owned(),
                );
            }
            let state = read_state(dir);
            report.onto = state
                .onto
                .clone()
                .unwrap_or_else(|| "(not recorded)".to_owned());
            state
        }
        Mode::Start => {
            if in_progress(dir) {
                return Err(
                    "a rebase is in progress: `cargo xtask rebase --continue` or `--abort`"
                        .to_owned(),
                );
            }
            let dirty = git.run(&["status", "--porcelain", "--untracked-files=no"])?;
            if !dirty.is_empty() {
                return Err(format!(
                    "the working tree has changes not committed: commit them first\n{dirty}"
                ));
            }
            if request.fetch {
                fetch(git, &request.onto, &mut report)?;
            }
            let state = State {
                old: Some(git.run(&["rev-parse", "HEAD"])?),
                onto: Some(request.onto.clone()),
                generators: BTreeSet::new(),
            };
            write_state(dir, &state)?;
            let started = git.output(&["rebase", &request.onto], &quiet())?;
            if !started.status.success() && !in_progress(dir) {
                forget_state(dir)?;
                return Err(format!(
                    "`git rebase {}` failed: {}",
                    request.onto,
                    String::from_utf8_lossy(&started.stderr).trim()
                ));
            }
            state
        }
    };
    report.old.clone_from(&state.old);

    while in_progress(dir) {
        let files = unmerged(&git.raw(&["ls-files", "-u", "-z"], &Call::default())?)?;
        if files.is_empty() {
            go_on(git, dir, &mut report)?;
            continue;
        }
        let stop = settle(git, top, &files, &mut state, hooks)?;
        write_state(dir, &state)?;
        let left = stop
            .files
            .iter()
            .any(|(_, how)| matches!(how, How::Left(_)));
        report.stops.push(stop);
        if left {
            return Ok(report);
        }
        go_on(git, dir, &mut report)?;
    }

    report.finished = true;
    let after = (hooks.finish)(&state.generators)?;
    report.after.extend(after);
    forget_state(dir)?;
    Ok(report)
}

/// Fetches the remote `onto` names (`origin` of `origin/main`), when it is
/// one of the repository's.
fn fetch(git: &Git, onto: &str, report: &mut Report) -> Result<()> {
    let remotes = git.run(&["remote"])?;
    match onto.split_once('/') {
        Some((remote, _)) if remotes.lines().any(|line| line == remote) => {
            git.run(&["fetch", "--quiet", remote])?;
        }
        _ => report
            .after
            .push(format!("`{onto}` names no remote: nothing was fetched.")),
    }
    Ok(())
}

/// `git rebase --continue` once no file is conflicted: it goes on to the
/// next commit, which may stop again. A commit left with no change once its
/// conflicts were settled is dropped (`--skip`), and named.
fn go_on(git: &Git, dir: &Path, report: &mut Report) -> Result<()> {
    let went = git.output(&["rebase", "--continue"], &quiet())?;
    if went.status.success() || !in_progress(dir) {
        return Ok(());
    }
    let still = unmerged(&git.raw(&["ls-files", "-u", "-z"], &Call::default())?)?;
    if !still.is_empty() {
        // It went on, and the next commit stopped.
        return Ok(());
    }
    if git.yes(&["diff", "--cached", "--quiet"])? {
        let commit = stopped_commit(git);
        git.output(&["rebase", "--skip"], &quiet())?;
        report.dropped.push(commit.unwrap_or_default());
        return Ok(());
    }
    Err(format!(
        "`git rebase --continue` failed: {}",
        String::from_utf8_lossy(&went.stderr).trim()
    ))
}

/// The commit git stopped on: `<short hash> <subject>`.
fn stopped_commit(git: &Git) -> Option<String> {
    git.run(&["show", "-s", "--format=%h %s", "REBASE_HEAD"])
        .ok()
}

/// Settles each conflicted file of a stop as its class says: writes and
/// adds what is settled, and leaves the rest as git left it.
fn settle(
    git: &Git,
    top: &Path,
    files: &BTreeMap<String, [Option<String>; 3]>,
    state: &mut State,
    hooks: &Hooks<'_>,
) -> Result<Stop> {
    let mut stop = Stop {
        commit: stopped_commit(git).unwrap_or_else(|| "(a commit)".to_owned()),
        files: Vec::new(),
    };
    let (mut taken, mut settled) = (Vec::new(), Vec::new());
    for (path, blobs) in files {
        let class = classify::classify(path);
        let stages = match class {
            Class::Additions(_) => match texts(git, blobs) {
                Ok(stages) => stages,
                Err(why) => {
                    stop.files.push((path.clone(), How::Left(why)));
                    continue;
                }
            },
            _ => Stages {
                base: blobs[0].clone(),
                ours: blobs[1].clone(),
                theirs: blobs[2].clone(),
            },
        };
        let how = match classify::resolve(path, class, &stages, hooks.formatted) {
            Resolution::Taken { then } => {
                taken.push(path.as_str());
                state.generators.insert(then.to_owned());
                How::Taken(then)
            }
            Resolution::United { text, places } => {
                let file = top.join(path);
                std::fs::write(&file, text)
                    .map_err(|error| format!("could not write {}: {error}", file.display()))?;
                settled.push(path.as_str());
                How::United(places)
            }
            Resolution::Left(why) => How::Left(why),
        };
        stop.files.push((path.clone(), how));
    }
    if !taken.is_empty() {
        let mut checkout = vec!["checkout", "--ours", "--"];
        checkout.extend(&taken);
        git.run(&checkout)?;
    }
    settled.extend(taken);
    if !settled.is_empty() {
        let mut add = vec!["add", "--"];
        add.extend(&settled);
        git.run(&add)?;
    }
    Ok(stop)
}

/// The three texts of a conflicted file, read in one git; refused when one
/// is not text.
fn texts(git: &Git, blobs: &[Option<String>; 3]) -> std::result::Result<Stages, String> {
    let present: Vec<String> = blobs.iter().flatten().cloned().collect();
    let mut read = git
        .blobs(&present)
        .map_err(|_| "a side of it is not text: settle it by hand".to_owned())?
        .into_iter();
    let mut next = |blob: &Option<String>| blob.as_ref().and_then(|_| read.next());
    Ok(Stages {
        base: next(&blobs[0]),
        ours: next(&blobs[1]),
        theirs: next(&blobs[2]),
    })
}

/// The run's report, as it is printed.
pub(crate) fn describe(report: &Report) -> String {
    let mut text = String::new();
    let old = report
        .old
        .as_deref()
        .map(|hash| hash.get(..12).unwrap_or(hash));
    if report.stops.is_empty() && report.finished {
        text.push_str(&format!(
            "Rebased {} onto {} with no conflict.\n",
            report.branch, report.onto
        ));
    }
    for stop in &report.stops {
        text.push_str(&format!("Stopped at {}:\n", stop.commit));
        for (path, how) in &stop.files {
            let line = match how {
                How::Taken(generator) if *generator == "outstanding" => format!(
                    "  taken     {path}: the side rebased onto; `cargo xtask outstanding` \
                     writes it at the end"
                ),
                How::Taken(generator) => format!(
                    "  taken     {path}: the side rebased onto; `cargo xtask regen {generator}` \
                     writes it at the end"
                ),
                How::United(places) => format!(
                    "  united    {path}: both sides' additions at {places} place(s); every \
                     line of both kept, and checked"
                ),
                How::Left(why) => format!("  LEFT      {path}: {why}"),
            };
            text.push_str(&line);
            text.push('\n');
        }
    }
    for commit in &report.dropped {
        text.push_str(&format!(
            "Dropped {commit}: nothing of it was left once its conflicts were settled.\n"
        ));
    }
    if report.finished {
        if !report.stops.is_empty() {
            text.push_str(&format!(
                "Rebased {} onto {}: every conflict settled.\n",
                report.branch, report.onto
            ));
        }
        for line in &report.after {
            text.push_str(line);
            text.push('\n');
        }
        if let Some(old) = old {
            text.push_str(&format!(
                "The tip before was {old}: `git reset --keep {old}` takes the branch back to \
                 it.\n"
            ));
        }
    } else {
        text.push_str(
            "The rebase is stopped. Settle each file LEFT above, `git add` it, then run \
             `cargo xtask rebase --continue` (or `--abort`).\n",
        );
        if let Some(old) = old {
            text.push_str(&format!("The tip before the rebase was {old}.\n"));
        }
    }
    text
}

/// Whether `text` is as rustfmt formats it, with the workspace's edition and
/// its rustfmt.toml (`top`'s). A text rustfmt cannot parse is not.
pub(crate) fn rustfmt_leaves(top: &Path, text: &str) -> std::result::Result<(), String> {
    use std::io::Write as _;
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .current_dir(top)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not run rustfmt: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|error| format!("could not write to rustfmt: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("could not run rustfmt: {error}"))?;
    if !output.status.success() {
        let said = String::from_utf8_lossy(&output.stderr);
        return Err(said.lines().next().unwrap_or("it failed").to_owned());
    }
    if String::from_utf8_lossy(&output.stdout) == text.replace("\r\n", "\n") {
        Ok(())
    } else {
        Err("it would lay the result out otherwise".to_owned())
    }
}

/// What is done at the end of a rebase in this repository: the open items'
/// index written from its items, and the generators of the files taken run
/// (or, with `regen` false, named).
fn finish(top: &Path, generators: &BTreeSet<String>, regen: bool) -> Result<Vec<String>> {
    let mut said = Vec::new();
    if top.join(crate::outstanding::INDEX).is_file() {
        said.push(crate::outstanding::write(top, false)?);
    }
    let names: Vec<String> = generators
        .iter()
        .filter(|name| *name != "outstanding")
        .cloned()
        .collect();
    if !names.is_empty() {
        let command = format!("cargo xtask regen {}", names.join(" "));
        if regen {
            println!("$ {command}");
            crate::regen::run(&names)?;
            said.push(format!("Ran `{command}`."));
        } else {
            said.push(format!("Run `{command}` to write the files taken."));
        }
    }
    if !said.is_empty() {
        said.push(
            "What the end wrote is not committed: look at it, and commit it (or fold it in \
             with `cargo xtask branch fold`)."
                .to_owned(),
        );
    }
    Ok(said)
}

/// `cargo xtask rebase …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    let top = crate::workspace::root();
    if !top.join(".git").exists() {
        return Err("this tree has no .git: run it in a git checkout".to_owned());
    }
    let git = Git::at(&top);
    let formatted = |text: &str| rustfmt_leaves(&top, text);
    let mut end = |generators: &BTreeSet<String>| finish(&top, generators, request.regen);
    let report = rebase(
        &git,
        &top,
        &request,
        &mut Hooks {
            formatted: &formatted,
            finish: &mut end,
        },
    )?;
    print!("{}", describe(&report));
    if report.finished {
        Ok(())
    } else {
        Err("the rebase stopped on files left for a person".to_owned())
    }
}
