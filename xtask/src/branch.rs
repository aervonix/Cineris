//! `cargo xtask branch`: two changes of history a worktree session needs and
//! the shell guard makes hard (`docs/testing-and-ci.md`, "A branch's history
//! without the shell guard").
//!
//! - **`commit-records`** commits a step's records (its `ci.md`, its break
//!   lists) to a branch that is not checked out: `ci-times --write` after a
//!   pull request's last push, while the worktree is on another branch. It is
//!   made with git's plumbing, in an index of its own: the working tree, the
//!   index and the branch checked out are not touched.
//! - **`fold`** folds the staged changes, or the branch's last commit, into
//!   an earlier commit of the branch; **`to-end`** moves a commit to the
//!   branch's tip. Both rewrite the current branch from that commit on, and
//!   nothing else. The new history is made with `git merge-tree` and
//!   `git commit-tree`, which write objects and move no ref, so a conflict
//!   leaves everything as it was; the old tip is kept under
//!   `refs/xtask-backup/` before the branch is moved, and the branch is moved
//!   only if it is still where it was read.
//!
//! Each is one `cargo xtask` command with plain arguments: no loop, heredoc
//! or `$(git …)` for the guard to refuse, and no interactive rebase.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use crate::Result;

const USAGE: &str = "\
usage: cargo xtask branch commit-records --branch BRANCH --message MESSAGE [--replace] FILE...
       cargo xtask branch fold COMMIT [--last] [--base REF]
       cargo xtask branch to-end COMMIT [--base REF]

commit-records  Commits the FILEs, as the working tree has them, to BRANCH,
                which no worktree may be using (checked out, being rebased or
                bisected): one commit on top of BRANCH, with the repository's
                identity, made in an index of its own. The working tree, the
                index and the branch checked out are not touched. A FILE is a
                path from the top of the repository, must be a record
                (docs/steps/**/*.md, docs/outstanding/*.md or
                mutations/*.breaks), a regular file reached through no link,
                and built on BRANCH's copy if BRANCH has one: BRANCH's lines
                are all in it, in their order (or the copy is HEAD's).
                --replace puts the file in place of BRANCH's copy all the
                same. BRANCH is named as it is (`@{-1}` or `-` is refused).
                It prints the commit and the command that pushes it, and
                pushes nothing.

fold            Folds the staged changes (or, with --last, the branch's last
                commit) into COMMIT, an earlier commit of the current branch,
                and makes the commits after it again on top.
to-end          Moves COMMIT to the tip of the current branch.

Both rewrite the current branch from COMMIT on, and no other branch. COMMIT
must be on the branch and not on REF (origin/main when not given): what is
there is published. They refuse a merge commit in the range, and changes not
committed (fold takes the staged ones, and refuses any others). A commit that
does not apply where it goes is named, and nothing is changed. Before the
branch is moved, its old tip is kept as refs/xtask-backup/BRANCH/UNIX-TIME,
which is printed with the command that takes the branch back to it: `git reset
--soft <that ref>` after a fold of the staged changes (they are staged again),
`git reset --keep <that ref>` after the others. A branch another worktree is
using is refused.
Needs git 2.40 or later (`git merge-tree --merge-base`).";

/// git in one repository, as these commands run it.
pub(crate) struct Git {
    dir: PathBuf,
    /// Variables set (`Some`) or removed (`None`) for every git it starts.
    env: Vec<(&'static str, Option<OsString>)>,
}

/// One `git` to run: its arguments, an index of its own, more variables,
/// and what to write to its standard input.
#[derive(Default)]
struct Call<'a> {
    index: Option<&'a Path>,
    env: Vec<(&'static str, String)>,
    input: Option<&'a [u8]>,
}

impl Git {
    /// git in `dir`, the top of a working tree: the machine's and the
    /// repository's configuration, as `git commit` reads them.
    pub(crate) fn at(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            env: Vec::new(),
        }
    }

    fn output(&self, arguments: &[&str], call: &Call<'_>) -> Result<Output> {
        let mut command = Command::new("git");
        // A hook exports these, and they would point git elsewhere.
        for variable in crate::owner_guard::GIT_LOCATION {
            command.env_remove(variable);
        }
        for (variable, value) in &self.env {
            match value {
                Some(value) => command.env(variable, value),
                None => command.env_remove(variable),
            };
        }
        if let Some(index) = call.index {
            command.env("GIT_INDEX_FILE", index);
        }
        for (variable, value) in &call.env {
            command.env(variable, value);
        }
        command
            .args(arguments)
            .current_dir(&self.dir)
            .stdin(if call.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("could not run git: {error}"))?;
        if let Some(input) = call.input
            && let Some(mut stdin) = child.stdin.take()
        {
            stdin
                .write_all(input)
                .map_err(|error| format!("could not write to git: {error}"))?;
        }
        child
            .wait_with_output()
            .map_err(|error| format!("could not run git: {error}"))
    }

    fn failed(arguments: &[&str], output: &Output) -> String {
        format!(
            "`git {}` failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }

    /// What git printed, as it printed it, when it succeeded.
    fn raw(&self, arguments: &[&str], call: &Call<'_>) -> Result<String> {
        let output = self.output(arguments, call)?;
        if !output.status.success() {
            return Err(Self::failed(arguments, &output));
        }
        String::from_utf8(output.stdout).map_err(|_| {
            format!(
                "`git {}` printed bytes that are not text",
                arguments.join(" ")
            )
        })
    }

    /// What git printed, trimmed, when it succeeded.
    fn run(&self, arguments: &[&str]) -> Result<String> {
        Ok(self.raw(arguments, &Call::default())?.trim().to_owned())
    }

    fn run_with(&self, arguments: &[&str], call: &Call<'_>) -> Result<String> {
        Ok(self.raw(arguments, call)?.trim().to_owned())
    }

    /// A question git answers by its exit code: 0 is yes and 1 is no.
    /// Anything else is an error.
    fn yes(&self, arguments: &[&str]) -> Result<bool> {
        let output = self.output(arguments, &Call::default())?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(Self::failed(arguments, &output)),
        }
    }

    /// What each of `names` names (`cat-file --batch-check`, one git for
    /// all of them): its full hash, or `None` when it names nothing. A name is
    /// any of git's (`main`, `HEAD~2`, `x^{tree}`).
    fn resolve(&self, names: &[String]) -> Result<Vec<Option<String>>> {
        if let Some(name) = names
            .iter()
            .find(|name| name.is_empty() || name.contains(char::is_whitespace))
        {
            return Err(format!("`{name}` is not a name git can look up"));
        }
        let input: String = names.iter().map(|name| format!("{name}\n")).collect();
        let printed = self.raw(
            &["cat-file", "--batch-check"],
            &Call {
                input: Some(input.as_bytes()),
                ..Call::default()
            },
        )?;
        let found = batch_check(&printed);
        if found.len() != names.len() {
            return Err(format!(
                "git looked up {} name(s) of {}",
                found.len(),
                names.len()
            ));
        }
        Ok(found)
    }

    /// The text of each blob of `hashes` (`cat-file --batch`, one git for
    /// all of them).
    fn blobs(&self, hashes: &[String]) -> Result<Vec<String>> {
        let input: String = hashes.iter().map(|hash| format!("{hash}\n")).collect();
        let output = self.output(
            &["cat-file", "--batch"],
            &Call {
                input: Some(input.as_bytes()),
                ..Call::default()
            },
        )?;
        if !output.status.success() {
            return Err(Self::failed(&["cat-file", "--batch"], &output));
        }
        let objects = batch_objects(&output.stdout)?;
        if objects.len() != hashes.len() {
            return Err(format!(
                "git read {} file(s) of {}",
                objects.len(),
                hashes.len()
            ));
        }
        Ok(objects.into_iter().map(|(_, text)| text).collect())
    }

    /// The commits `hashes` name, each read whole (`cat-file --batch`, one
    /// git for all of them).
    fn commits(&self, hashes: &[String]) -> Result<Vec<CommitInfo>> {
        let input: String = hashes.iter().map(|hash| format!("{hash}\n")).collect();
        let output = self.output(
            &["cat-file", "--batch"],
            &Call {
                input: Some(input.as_bytes()),
                ..Call::default()
            },
        )?;
        if !output.status.success() {
            return Err(Self::failed(&["cat-file", "--batch"], &output));
        }
        let objects = batch_objects(&output.stdout)?;
        if objects.len() != hashes.len() {
            return Err(format!(
                "git read {} commit(s) of {}",
                objects.len(),
                hashes.len()
            ));
        }
        objects
            .iter()
            .map(|(kind, body)| {
                if kind == "commit" {
                    CommitInfo::parse(body)
                } else {
                    Err(format!("a {kind} where a commit was looked for"))
                }
            })
            .collect()
    }
}

/// What `git cat-file --batch-check` printed, a line for each name asked:
/// `<hash> <type> <size>`, or `<name> missing` (or `ambiguous`) for a name
/// that names nothing.
pub(crate) fn batch_check(printed: &str) -> Vec<Option<String>> {
    printed
        .lines()
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            match words.as_slice() {
                [hash, _, size] if size.bytes().all(|byte| byte.is_ascii_digit()) => {
                    Some((*hash).to_owned())
                }
                _ => None,
            }
        })
        .collect()
}

/// What `git cat-file --batch` printed: for each object `<hash> <type>
/// <size>`, a line, then that many bytes of it and a line's end. Each object's
/// type and text; a message must be text.
pub(crate) fn batch_objects(printed: &[u8]) -> Result<Vec<(String, String)>> {
    let mut objects = Vec::new();
    let mut rest = printed;
    while !rest.is_empty() {
        let end = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or("git's batch ended in the middle of a header")?;
        let header = String::from_utf8_lossy(&rest[..end]).into_owned();
        let words: Vec<&str> = header.split_whitespace().collect();
        let [_, kind, size] = words.as_slice() else {
            return Err(format!(
                "git's batch has a header that is not an object's: {header}"
            ));
        };
        let size: usize = size
            .parse()
            .map_err(|_| format!("git's batch has a size that is not one: {header}"))?;
        let body = rest
            .get(end + 1..end + 1 + size)
            .ok_or("git's batch ended in the middle of an object")?;
        let body =
            String::from_utf8(body.to_vec()).map_err(|_| format!("object {header} is not text"))?;
        objects.push(((*kind).to_owned(), body));
        // The object, then the line's end that follows it.
        rest = rest.get(end + 2 + size..).unwrap_or_default();
    }
    Ok(objects)
}

/// A file that is removed when this is dropped: a temporary index.
struct Removed(PathBuf);

impl Drop for Removed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The first twelve digits of a hash, as the commands print them.
fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

// commit-records

/// What `branch commit-records` was asked for.
#[derive(Debug, PartialEq)]
pub(crate) struct RecordsRequest {
    pub(crate) branch: String,
    pub(crate) message: String,
    /// Paths from the top of the repository, with `/` between their parts.
    pub(crate) files: Vec<String>,
    /// Put each file in place of the branch's copy even when it was not
    /// built on it.
    pub(crate) replace: bool,
}

impl RecordsRequest {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let (mut branch, mut message, mut files) = (None, None, Vec::new());
        let mut replace = false;
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            let mut value = |what: &str| {
                words
                    .next()
                    .ok_or_else(|| format!("`{word}` needs {what}\n\n{USAGE}"))
            };
            match word.as_str() {
                "--branch" => branch = Some(value("a branch")?.clone()),
                "--message" | "-m" => message = Some(value("a message")?.clone()),
                "--replace" => replace = true,
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag `{other}`\n\n{USAGE}"));
                }
                file => files.push(record_path(file)?),
            }
        }
        let branch = branch
            .filter(|branch| !branch.trim().is_empty())
            .ok_or_else(|| format!("say which branch: --branch BRANCH\n\n{USAGE}"))?;
        let message = message
            .filter(|message| !message.trim().is_empty())
            .ok_or_else(|| format!("say what the commit is: --message MESSAGE\n\n{USAGE}"))?;
        if files.is_empty() {
            return Err(format!("name the records to commit\n\n{USAGE}"));
        }
        Ok(Self {
            branch,
            message,
            files,
            replace,
        })
    }
}

/// `file` as a path from the top of the repository, when it is a record: a
/// step's page (`docs/steps/**/*.md`), an item left open
/// (`docs/outstanding/*.md`) or a list of breaks (`mutations/*.breaks`).
/// Anything else is refused: this commits records alone, past the checks a
/// branch's own commit would have.
pub(crate) fn record_path(file: &str) -> Result<String> {
    let path = file.replace('\\', "/");
    let path = path.strip_prefix("./").unwrap_or(&path);
    let parts: Vec<&str> = path.split('/').collect();
    if path.is_empty()
        || path.contains(':')
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(format!(
            "`{file}` is not a path inside the repository, from its top (docs/steps/NN/ci.md)"
        ));
    }
    let page = |name: &str| name.len() > ".md".len() && name.ends_with(".md");
    let record = match parts.as_slice() {
        ["docs", "steps", _, .., name] | ["docs", "outstanding", name] => page(name),
        ["mutations", list] => list.len() > ".breaks".len() && list.ends_with(".breaks"),
        _ => false,
    };
    if record {
        Ok(path.to_owned())
    } else {
        Err(format!(
            "`{file}` is not a record: commit-records takes only docs/steps/**/*.md, \
             docs/outstanding/*.md and mutations/*.breaks. Commit anything else on its own \
             branch, where its checks run"
        ))
    }
}

/// What one worktree's git directory says of the branches it holds, as
/// `git branch -f` reads it: `HEAD` (`ref: refs/heads/…`), a rebase's
/// `rebase-merge/head-name` or `rebase-apply/head-name`, and a bisection's
/// `BISECT_START` (the branch's short name). A worktree in the middle of a
/// rebase or a bisection has a detached `HEAD`, and `git worktree list`
/// shows no branch for it.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Holds {
    pub(crate) head: Option<String>,
    pub(crate) rebase_merge: Option<String>,
    pub(crate) rebase_apply: Option<String>,
    pub(crate) bisect: Option<String>,
}

impl Holds {
    /// What the git directory `dir` holds; a file that is not there is
    /// nothing.
    pub(crate) fn read(dir: &Path) -> Self {
        let read = |path: PathBuf| {
            std::fs::read_to_string(path)
                .ok()
                .map(|text| text.trim().to_owned())
        };
        Self {
            head: read(dir.join("HEAD")),
            rebase_merge: read(dir.join("rebase-merge").join("head-name")),
            rebase_apply: read(dir.join("rebase-apply").join("head-name")),
            bisect: read(dir.join("BISECT_START")),
        }
    }

    /// How this worktree uses `reference` (`refs/heads/…`), if it does.
    pub(crate) fn uses(&self, reference: &str) -> Option<&'static str> {
        let is = |text: &Option<String>| text.as_deref() == Some(reference);
        if self
            .head
            .as_deref()
            .and_then(|head| head.strip_prefix("ref: "))
            == Some(reference)
        {
            Some("checked out")
        } else if is(&self.rebase_merge) || is(&self.rebase_apply) {
            Some("being rebased")
        } else if self
            .bisect
            .as_deref()
            .is_some_and(|name| format!("refs/heads/{name}") == reference)
        {
            Some("being bisected")
        } else {
            None
        }
    }
}

/// Every worktree of the repository whose common git directory is
/// `common`: its git directory and where it is. The main one first (the
/// common directory, beside its files), then each linked one (under
/// `worktrees/`, whose `gitdir` file names the worktree's `.git`).
pub(crate) fn worktrees(common: &Path) -> Vec<(PathBuf, PathBuf)> {
    let main = if common.file_name().is_some_and(|name| name == ".git") {
        common.parent().unwrap_or(common).to_path_buf()
    } else {
        common.to_path_buf()
    };
    let mut found = vec![(common.to_path_buf(), main)];
    let mut linked: Vec<PathBuf> = std::fs::read_dir(common.join("worktrees"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    linked.sort();
    for dir in linked {
        let at = std::fs::read_to_string(dir.join("gitdir"))
            .ok()
            .map(|text| PathBuf::from(text.trim()))
            .and_then(|dot_git| dot_git.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| dir.clone());
        found.push((dir, at));
    }
    found
}

/// The refusal for `branch` (`refs/heads/…` is `reference`) when a worktree
/// other than the one whose git directory is `except` uses it: as `git
/// branch -f` refuses a branch "used by worktree at …".
pub(crate) fn used_elsewhere(
    common: &Path,
    reference: &str,
    except: Option<&Path>,
) -> Option<String> {
    let same = |a: &Path, b: &Path| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    };
    worktrees(common).into_iter().find_map(|(dir, at)| {
        if except.is_some_and(|except| same(except, &dir)) {
            return None;
        }
        Holds::read(&dir).uses(reference).map(|how| {
            format!(
                "`{}` is used by worktree at {} ({how} there): it is not moved under it",
                reference.trim_start_matches("refs/heads/"),
                at.display()
            )
        })
    })
}

/// Whether `path` (`file` under the top of the repository `root`) is a
/// regular file inside the repository: not a link, and not reached through
/// one that leads out of it. A link's target is what `hash-object` would
/// commit, and it may be anywhere.
fn regular_inside(root: &Path, file: &str) -> Result<()> {
    // Each folder on the way, from the top down, as `git add` reads it: a
    // link anywhere on the path is refused, wherever it leads.
    let parts: Vec<&str> = file.split('/').collect();
    let mut path = root.to_path_buf();
    for (at, part) in parts.iter().enumerate() {
        path.push(part);
        let kind = std::fs::symlink_metadata(&path)
            .map_err(|_| format!("`{file}` is not in the working tree"))?
            .file_type();
        let last = at + 1 == parts.len();
        if kind.is_symlink() {
            return Err(format!(
                "`{file}` goes through a link (`{}`): commit-records commits a record's own \
                 text, never what a link points at, as `git add` will not add a path beyond one",
                parts[..=at].join("/")
            ));
        }
        if last && !kind.is_file() {
            return Err(format!("`{file}` is not a regular file"));
        }
    }
    Ok(())
}

/// Whether `working` was built by adding to `theirs`: every line of
/// `theirs` is in `working`, in its order, with lines added anywhere
/// between them. Line ends are read alike (`\r\n` as `\n`).
pub(crate) fn built_on(theirs: &str, working: &str) -> bool {
    let lines = |text: &str| -> Vec<String> {
        text.lines()
            .map(|line| line.trim_end_matches('\r').to_owned())
            .collect()
    };
    let working = lines(working);
    let mut rest = working.iter();
    lines(theirs)
        .iter()
        .all(|line| rest.any(|candidate| candidate == line))
}

/// What a record's copies say of committing it: `theirs` is the branch's
/// blob and its text, `ours` HEAD's blob, `working` the file as it is. It
/// may be committed when the branch has no copy, when the branch's copy is
/// HEAD's (the file was made from it), or when the file was built on the
/// branch's copy ([`built_on`]); with `replace`, always. Otherwise the
/// refusal, saying which and how to go on.
pub(crate) fn not_from_the_branch(
    file: &str,
    branch: &str,
    theirs: Option<(&str, &str)>,
    ours: Option<&str>,
    working: &str,
    replace: bool,
) -> Option<String> {
    let (blob, text) = theirs?;
    if replace || Some(blob) == ours || built_on(text, working) {
        return None;
    }
    Some(format!(
        "`{file}` on {branch} ({}) has lines the file here does not, in their order: the file \
         was not made by adding to {branch}'s copy, and committing it would drop them. Take \
         {branch}'s copy (`git show {branch}:{file} > {file}`), make the change on it, and run \
         this again; or give --replace to put this file in place of {branch}'s, knowing it",
        short(blob)
    ))
}

/// What `git update-ref --stdin` is given to move `branch` (`refs/heads/…`)
/// to `new` only from `old`, the commit read at the start.
pub(crate) fn records_transaction(branch: &str, new: &str, old: &str) -> String {
    format!("update {branch} {new} {old}\n")
}

/// A commit `commit-records` made.
#[derive(Debug, PartialEq)]
pub(crate) struct Committed {
    pub(crate) branch: String,
    pub(crate) old: String,
    pub(crate) new: String,
}

/// Commits `request`'s files, as the working tree has them, on top of its
/// branch, which no worktree has checked out: in an index of its own, with
/// the branch moved only from the commit read at the start.
pub(crate) fn commit_records(git: &Git, request: &RecordsRequest) -> Result<Committed> {
    let branch = git
        .run(&["check-ref-format", "--branch", &request.branch])
        .map_err(|_| format!("`{}` is not a branch's name", request.branch))?;
    // `@{-1}` and `-` are expanded to the branch checked out before: a
    // branch the user did not name is not moved.
    if branch != request.branch {
        return Err(format!(
            "`{}` stands for `{branch}`: name the branch itself",
            request.branch
        ));
    }
    let branch = &branch;
    let reference = format!("refs/heads/{branch}");
    for file in &request.files {
        regular_inside(&git.dir, file)?;
    }
    // The repository's common directory, and the index of this command's
    // own, in one git.
    let located = git.run(&[
        "rev-parse",
        "--git-common-dir",
        "--git-path",
        &format!("xtask-records-{}.index", std::process::id()),
    ])?;
    let mut located = located.lines().map(|line| git.dir.join(line));
    let (Some(common), Some(index)) = (located.next(), located.next()) else {
        return Err("git named no git directory".to_owned());
    };
    if let Some(refusal) = used_elsewhere(&common, &reference, None) {
        return Err(format!(
            "{refusal}. commit-records is for a branch no worktree is using: commit there, with \
             `git commit`"
        ));
    }
    // The branch, its tree, and each file's copy on it and on HEAD.
    let mut names = vec![
        format!("{reference}^{{commit}}"),
        format!("{reference}^{{tree}}"),
    ];
    for file in &request.files {
        names.push(format!("{reference}:{file}"));
        names.push(format!("HEAD:{file}"));
    }
    let found = git.resolve(&names)?;
    let [Some(old), Some(old_tree)] = [found[0].clone(), found[1].clone()] else {
        return Err(format!("there is no branch `{branch}` in this repository"));
    };
    // The branch's copies that are not HEAD's, read in one git, to see that
    // each file was built on its.
    let copies: Vec<(Option<String>, Option<String>)> = found[2..]
        .chunks(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect();
    let differ: Vec<String> = copies
        .iter()
        .filter_map(|(theirs, ours)| {
            theirs
                .clone()
                .filter(|theirs| Some(theirs) != ours.as_ref())
        })
        .collect();
    let texts = if differ.is_empty() || request.replace {
        Vec::new()
    } else {
        git.blobs(&differ)?
    };
    for (file, (theirs, ours)) in request.files.iter().zip(&copies) {
        let text = theirs
            .as_ref()
            .and_then(|blob| differ.iter().position(|one| one == blob))
            .and_then(|at| texts.get(at))
            .map_or("", String::as_str);
        let working = String::from_utf8_lossy(
            &std::fs::read(git.dir.join(file))
                .map_err(|error| format!("could not read `{file}`: {error}"))?,
        )
        .into_owned();
        if let Some(refusal) = not_from_the_branch(
            file,
            branch,
            theirs.as_deref().map(|blob| (blob, text)),
            ours.as_deref(),
            &working,
            request.replace,
        ) {
            return Err(refusal);
        }
    }

    let index = Removed(index);
    let _ = std::fs::remove_file(&index.0);
    let in_index = Call {
        index: Some(&index.0),
        ..Call::default()
    };
    git.run_with(&["read-tree", &old], &in_index)?;
    let mut hashing = vec!["hash-object", "-w", "--"];
    hashing.extend(request.files.iter().map(String::as_str));
    let blobs = git.run(&hashing)?;
    let blobs: Vec<&str> = blobs.lines().collect();
    if blobs.len() != request.files.len() {
        return Err(format!(
            "git hashed {} file(s) of {}",
            blobs.len(),
            request.files.len()
        ));
    }
    let entries: String = blobs
        .iter()
        .zip(&request.files)
        .map(|(blob, file)| format!("100644 blob {blob}\t{file}\n"))
        .collect();
    git.run_with(
        &["update-index", "--index-info"],
        &Call {
            index: Some(&index.0),
            input: Some(entries.as_bytes()),
            ..Call::default()
        },
    )?;
    let tree = git.run_with(&["write-tree"], &in_index)?;
    if tree == old_tree {
        return Err(format!(
            "nothing changed: {branch} already has the files as the working tree has them"
        ));
    }
    let new = git.run(&["commit-tree", &tree, "-p", &old, "-m", &request.message])?;
    git.run_with(
        &["update-ref", "-m", "xtask branch commit-records", "--stdin"],
        &Call {
            input: Some(records_transaction(&reference, &new, &old).as_bytes()),
            ..Call::default()
        },
    )
    .map_err(|error| format!("{branch} moved while the commit was made, and was left: {error}"))?;
    Ok(Committed {
        branch: branch.clone(),
        old,
        new,
    })
}

// fold and to-end

/// Which history `fold` or `to-end` makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// The staged changes, or with `last` the branch's last commit, folded
    /// into the commit.
    Fold { last: bool },
    /// The commit moved to the branch's tip.
    ToEnd,
}

/// What `branch fold` or `branch to-end` was asked for.
#[derive(Debug, PartialEq)]
pub(crate) struct RewriteRequest {
    pub(crate) kind: Kind,
    pub(crate) commit: String,
    /// What is published: a commit on it is not rewritten.
    pub(crate) base: String,
}

impl RewriteRequest {
    pub(crate) fn parse(verb: &str, arguments: &[String]) -> Result<Self> {
        let mut kind = match verb {
            "fold" => Kind::Fold { last: false },
            _ => Kind::ToEnd,
        };
        let (mut commit, mut base) = (None, "origin/main".to_owned());
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            match word.as_str() {
                "--last" if verb == "fold" => kind = Kind::Fold { last: true },
                "--base" => {
                    base = words
                        .next()
                        .ok_or_else(|| format!("`--base` needs a ref\n\n{USAGE}"))?
                        .clone();
                }
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other if other.starts_with('-') => {
                    return Err(format!("unknown flag `{other}` of `{verb}`\n\n{USAGE}"));
                }
                other if commit.is_none() => commit = Some(other.to_owned()),
                other => return Err(format!("`{other}`: name one commit\n\n{USAGE}")),
            }
        }
        let commit = commit.ok_or_else(|| format!("name the commit to {verb}\n\n{USAGE}"))?;
        Ok(Self { kind, commit, base })
    }
}

/// A commit as `git cat-file commit` writes it, read for what a new one made
/// in its place keeps: its author and date, and its message, byte for byte.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommitInfo {
    pub(crate) tree: String,
    pub(crate) parents: Vec<String>,
    pub(crate) author_name: String,
    pub(crate) author_email: String,
    /// `<seconds> <zone>`, as git keeps it.
    pub(crate) author_date: String,
    pub(crate) message: String,
}

impl CommitInfo {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        let (headers, message) = raw
            .split_once("\n\n")
            .unwrap_or((raw.trim_end_matches('\n'), ""));
        let mut parents = Vec::new();
        let mut author = None;
        let mut tree = None;
        for line in headers.lines() {
            if let Some(hash) = line.strip_prefix("tree ") {
                tree = Some(hash.to_owned());
            } else if let Some(parent) = line.strip_prefix("parent ") {
                parents.push(parent.to_owned());
            } else if let Some(rest) = line.strip_prefix("author ") {
                let open = rest.rfind(" <").ok_or("an author line with no `<`")?;
                let close = rest.rfind("> ").ok_or("an author line with no `>`")?;
                if close < open {
                    return Err(format!("an author line out of order: {rest}"));
                }
                author = Some((
                    rest[..open].to_owned(),
                    rest[open + 2..close].to_owned(),
                    rest[close + 2..].to_owned(),
                ));
            }
        }
        let (author_name, author_email, author_date) =
            author.ok_or("a commit with no author line")?;
        Ok(Self {
            tree: tree.ok_or("a commit with no tree line")?,
            parents,
            author_name,
            author_email,
            author_date,
            message: message.to_owned(),
        })
    }

    /// The first line of the message.
    pub(crate) fn subject(&self) -> &str {
        self.message.lines().next().unwrap_or_default()
    }
}

/// The tree `git merge-tree --write-tree --name-only --no-messages` made,
/// when `clean`, or the files it conflicted in when not: its first line is
/// the tree, and the conflicted files' names follow.
pub(crate) fn merge_tree_result(
    printed: &str,
    clean: bool,
) -> std::result::Result<String, Vec<String>> {
    let mut lines = printed.lines();
    let tree = lines.next().unwrap_or_default().trim().to_owned();
    if clean {
        return Ok(tree);
    }
    let mut files: Vec<String> = Vec::new();
    for line in lines.map(str::trim).filter(|line| !line.is_empty()) {
        if !files.iter().any(|file| file == line) {
            files.push(line.to_owned());
        }
    }
    Err(files)
}

/// The name of the ref the old tip is kept under, the `attempt`th tried in
/// the same second.
pub(crate) fn backup_name(branch: &str, now: u64, attempt: usize) -> String {
    if attempt == 0 {
        format!("refs/xtask-backup/{branch}/{now}")
    } else {
        format!("refs/xtask-backup/{branch}/{now}-{attempt}")
    }
}

/// What `git update-ref --stdin` is given to keep the old tip as `backup`
/// and move `branch` (`refs/heads/…`) from `old` to `new`: one transaction,
/// in which the backup must not be there yet and the branch must still be at
/// `old`, or neither happens.
pub(crate) fn move_transaction(backup: &str, old: &str, branch: &str, new: &str) -> String {
    format!("create {backup} {old}\nupdate {branch} {new} {old}\n")
}

/// The refusal when the history made again does not end at the tree it
/// must (`made`, `expected`): the branch's last tree changes, which neither
/// command may do.
pub(crate) fn tree_refusal(kind: Kind, made: &str, expected: &str) -> String {
    let must = match kind {
        Kind::Fold { last: false } => "the staged tree, what the branch would be with the fix",
        Kind::Fold { last: true } | Kind::ToEnd => "the branch's tree as it is",
    };
    format!(
        "the history made again would end at tree {}, and it must end at {} ({must}): a commit \
         after the one rewritten puts back, or takes out, part of what moved, so the branch's \
         last tree would change. Nothing was moved. Make the change a commit of its own at the \
         end, or fold it into the later commit that it meets",
        short(made),
        short(expected)
    )
}

/// What takes the branch back to `backup` after a rewrite of `kind`. After
/// a fold of the staged changes `--soft`: the index and the working tree have
/// the fix, and stay as they are, so it is staged again. After the others
/// `--keep`: the branch's tree is the same before and after.
pub(crate) fn undo(kind: Kind, backup: &str) -> String {
    match kind {
        Kind::Fold { last: false } => format!("git reset --soft {backup}"),
        Kind::Fold { last: true } | Kind::ToEnd => format!("git reset --keep {backup}"),
    }
}

/// A rewrite that was made.
#[derive(Debug, PartialEq)]
pub(crate) struct Rewritten {
    pub(crate) branch: String,
    pub(crate) old: String,
    pub(crate) new: String,
    pub(crate) backup: String,
    /// Each commit of the new history from the one rewritten on, in order:
    /// the commit it was, the commit it is, and its subject.
    pub(crate) commits: Vec<(String, String, String)>,
}

/// A commit of the branch, read: its hash and what it is.
struct Read {
    hash: String,
    info: CommitInfo,
}

/// The rewrite in progress: the new commits made, and the tree of the last.
struct Builder<'a> {
    git: &'a Git,
    commits: Vec<(String, String, String)>,
    tree: String,
}

impl Builder<'_> {
    /// A tree of `ours` with what `theirs` changed from `base` made in it, or
    /// the files they conflict in.
    fn merge(
        &self,
        base: &str,
        ours: &str,
        theirs: &str,
    ) -> Result<std::result::Result<String, Vec<String>>> {
        let merge_base = format!("--merge-base={base}");
        let arguments = [
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            &merge_base,
            ours,
            theirs,
        ];
        let output = self.git.output(&arguments, &Call::default())?;
        let clean = match output.status.code() {
            Some(0) => true,
            Some(1) => false,
            _ => return Err(Git::failed(&arguments, &output)),
        };
        Ok(merge_tree_result(
            &String::from_utf8_lossy(&output.stdout),
            clean,
        ))
    }

    /// A new commit of `tree` on `parent`, with `info`'s author, date and
    /// message; the committer is the repository's identity, now.
    fn make(&mut self, tree: String, parent: &str, was: &Read) -> Result<String> {
        let info = &was.info;
        let new = self.git.run_with(
            &["commit-tree", &tree, "-p", parent],
            &Call {
                env: vec![
                    ("GIT_AUTHOR_NAME", info.author_name.clone()),
                    ("GIT_AUTHOR_EMAIL", info.author_email.clone()),
                    ("GIT_AUTHOR_DATE", format!("@{}", info.author_date)),
                ],
                input: Some(info.message.as_bytes()),
                ..Call::default()
            },
        )?;
        self.commits
            .push((was.hash.clone(), new.clone(), info.subject().to_owned()));
        self.tree = tree;
        Ok(new)
    }

    /// `commit` made again on `onto`.
    fn pick(&mut self, commit: &Read, onto: &str) -> Result<String> {
        let parent = commit
            .info
            .parents
            .first()
            .ok_or("a commit with no parent")?;
        match self.merge(parent, onto, &commit.hash)? {
            Ok(tree) => self.make(tree, onto, commit),
            Err(files) => Err(format!(
                "commit {} ({}) does not apply where it would go: it conflicts in {}. {UNCHANGED}",
                short(&commit.hash),
                commit.info.subject(),
                files.join(", ")
            )),
        }
    }

    /// `target` with what `fixup` changed from `fixup_base` folded in, on
    /// `target`'s own parent.
    fn fold(&mut self, target: &Read, fixup: &str, fixup_base: &str) -> Result<String> {
        let parent = target
            .info
            .parents
            .first()
            .ok_or("a commit with no parent")?;
        match self.merge(fixup_base, &target.hash, fixup)? {
            Ok(tree) => self.make(tree, parent, target),
            Err(files) => Err(format!(
                "what is folded does not apply to commit {} ({}): it conflicts in {}. \
                 {UNCHANGED}",
                short(&target.hash),
                target.info.subject(),
                files.join(", ")
            )),
        }
    }
}

/// What a refusal after the checks says: nothing was written but objects no
/// ref holds.
const UNCHANGED: &str =
    "Nothing was changed: the branch, the index and the working tree are as they were";

/// The commits of `listed` (`rev-list --reverse --topo-order --parents
/// BASE..HEAD`, a line `<hash> <parents>…` each, parents before children)
/// after `target`: what a rewrite from `target` makes again, in order. `None`
/// when `target` is not among them. A merge after it, or a commit that does
/// not follow the one before it, is refused: the history from `target` on
/// must be one straight line.
pub(crate) fn chain_after(listed: &str, target: &str) -> Option<Result<Vec<String>>> {
    let lines: Vec<Vec<&str>> = listed
        .lines()
        .map(|line| line.split_whitespace().collect())
        .filter(|words: &Vec<&str>| !words.is_empty())
        .collect();
    let at = lines.iter().position(|words| words[0] == target)?;
    let after = &lines[at + 1..];
    // A merge first: what it brings in is not a line after the target.
    if let Some(words) = std::iter::once(&lines[at])
        .chain(after)
        .find(|words| words.len() > 2)
    {
        return Some(Err(format!(
            "commit {} is a merge: a history with a merge in it is not rewritten here",
            short(words[0])
        )));
    }
    let mut chain = Vec::new();
    let mut previous = target;
    for words in after {
        if words.get(1) != Some(&previous) {
            return Some(Err(format!(
                "commit {} does not follow {}: the history is not a straight line",
                short(words[0]),
                short(previous)
            )));
        }
        previous = words[0];
        chain.push(previous.to_owned());
    }
    Some(Ok(chain))
}

/// Folds into, or moves to the end, `request.commit` of the current branch,
/// as `request.kind` says: the new history is made in objects alone, checked
/// to end at the tree it must, and the branch moved to it in one transaction
/// that keeps its old tip under a backup ref and moves it only from the tip
/// read at the start. `now` is the backup's time.
pub(crate) fn rewrite(git: &Git, request: &RewriteRequest, now: u64) -> Result<Rewritten> {
    // Where HEAD is, and the index, in one git.
    let located = git
        .run(&[
            "rev-parse",
            "--symbolic-full-name",
            "HEAD",
            "--git-path",
            "index",
            "--git-common-dir",
            "--absolute-git-dir",
        ])
        .map_err(|_| "HEAD names no commit: the branch has none yet".to_owned())?;
    let located: Vec<&str> = located.lines().collect();
    let [head_ref, index, common, own] = located.as_slice() else {
        return Err(format!(
            "git said where HEAD is in a way not understood: {located:?}"
        ));
    };
    let head_ref = (*head_ref).to_owned();
    let index = git.dir.join(index);
    let branch = head_ref
        .strip_prefix("refs/heads/")
        .ok_or("HEAD is not on a branch: switch to the branch to rewrite")?
        .to_owned();
    // Checked out in another worktree too (`worktree add --force`), or
    // being rebased or bisected there: its history is not changed under it.
    if let Some(refusal) = used_elsewhere(&git.dir.join(common), &head_ref, Some(Path::new(own))) {
        return Err(refusal);
    }
    let named = &request.commit;
    let found = git.resolve(&[
        "HEAD^{commit}".to_owned(),
        format!("{named}^{{commit}}"),
        format!("{}^{{commit}}", request.base),
    ])?;
    let [old, target, base] = <[Option<String>; 3]>::try_from(found).unwrap_or_default();
    let old = old.ok_or("HEAD names no commit: the branch has none yet")?;
    let target = target.ok_or_else(|| format!("`{named}` names no commit here"))?;
    let base = base.ok_or_else(|| {
        format!(
            "`{}` names no commit here: say what is published with --base REF",
            request.base
        )
    })?;

    // The branch's commits that are not published, parents first: the
    // target must be one, and what follows it a straight line.
    let listed = git.run(&[
        "rev-list",
        "--reverse",
        "--topo-order",
        "--parents",
        &format!("{base}..{old}"),
    ])?;
    let Some(chain) = chain_after(&listed, &target) else {
        // Why it is not: two questions, asked only now.
        return Err(
            if git.yes(&["merge-base", "--is-ancestor", &target, &old])? {
                format!(
                    "`{named}` ({}) is on {} already: what is published is not rewritten",
                    short(&target),
                    request.base
                )
            } else {
                format!(
                    "`{named}` ({}) is not on {branch}: only the current branch is rewritten",
                    short(&target)
                )
            },
        );
    };
    let fold_staged = request.kind == Kind::Fold { last: false };
    if fold_staged {
        if !git.yes(&["diff", "--quiet"])? {
            return Err(
                "the working tree has changes not committed beside the staged ones (fold takes \
                 only the staged): commit them, or set them aside, first"
                    .to_owned(),
            );
        }
        if git.yes(&["diff", "--cached", "--quiet"])? {
            return Err(
                "nothing is staged to fold: stage the fix with `git add`, or fold the branch's \
                 last commit with --last"
                    .to_owned(),
            );
        }
    } else if !git.yes(&["diff", "--quiet", "HEAD"])? {
        return Err(
            "the working tree has changes not committed: commit them, or set them aside, first"
                .to_owned(),
        );
    }
    let chain = chain?;
    let last = |kind: &str| format!("commit {} is the last of {branch}: {kind}", short(&target));
    match request.kind {
        Kind::ToEnd if chain.is_empty() => {
            return Err(last("it is at the end already"));
        }
        Kind::Fold { last: true } if chain.is_empty() => {
            return Err(last("--last folds the last commit into an earlier one"));
        }
        _ => {}
    }

    // Every commit from the target on, read in one go; the last is the tip.
    let mut hashes = vec![target.clone()];
    hashes.extend(chain.iter().cloned());
    let read: Vec<Read> = hashes
        .iter()
        .cloned()
        .zip(git.commits(&hashes)?)
        .map(|(hash, info)| Read { hash, info })
        .collect();
    let (target, rest) = read.split_first().ok_or("no commit was read")?;
    let old_tree = read
        .last()
        .map_or_else(String::new, |tip| tip.info.tree.clone());
    let Some(target_parent) = target.info.parents.first().cloned() else {
        return Err(format!(
            "commit {} is the repository's first: it has no parent to build on",
            short(&target.hash)
        ));
    };
    let mut builder = Builder {
        git,
        commits: Vec::new(),
        tree: String::new(),
    };
    let (new, expected) = match request.kind {
        Kind::ToEnd => {
            let mut onto = target_parent;
            for commit in rest {
                onto = builder.pick(commit, &onto)?;
            }
            (builder.pick(target, &onto)?, old_tree)
        }
        Kind::Fold { last: true } => {
            let (tip, between) = rest.split_last().ok_or("no last commit was read")?;
            let tip_parent = between.last().map_or(&target.hash, |commit| &commit.hash);
            let mut onto = builder.fold(target, &tip.hash, tip_parent)?;
            for commit in between {
                onto = builder.pick(commit, &onto)?;
            }
            (onto, old_tree)
        }
        Kind::Fold { last: false } => {
            // The staged tree, read from a copy of the index so that the
            // index itself is not written.
            let copy =
                Removed(index.with_file_name(format!("xtask-fold-{}.index", std::process::id())));
            std::fs::copy(&index, &copy.0)
                .map_err(|error| format!("could not copy the index: {error}"))?;
            let staged_tree = git.run_with(
                &["write-tree"],
                &Call {
                    index: Some(&copy.0),
                    ..Call::default()
                },
            )?;
            let fixup = git.run(&[
                "commit-tree",
                &staged_tree,
                "-p",
                &old,
                "-m",
                "the staged changes",
            ])?;
            let mut onto = builder.fold(target, &fixup, &old)?;
            for commit in rest {
                onto = builder.pick(commit, &onto)?;
            }
            (onto, staged_tree)
        }
    };
    if builder.tree != expected {
        return Err(tree_refusal(request.kind, &builder.tree, &expected));
    }

    // The backup and the move, in one transaction: both or neither. A backup
    // of the same second is there already: the next name is tried.
    let verb = match request.kind {
        Kind::Fold { .. } => "fold",
        Kind::ToEnd => "to-end",
    };
    let mut attempt = 0;
    let backup = loop {
        let name = backup_name(&branch, now, attempt);
        let transaction = move_transaction(&name, &old, &head_ref, &new);
        let moved = git.run_with(
            &[
                "update-ref",
                "-m",
                &format!("xtask branch {verb}"),
                "--stdin",
            ],
            &Call {
                input: Some(transaction.as_bytes()),
                ..Call::default()
            },
        );
        match moved {
            Ok(_) => break name,
            Err(_) if attempt < 9 && git.resolve(&[name])?[0].is_some() => attempt += 1,
            Err(error) => {
                return Err(format!(
                    "{branch} was not moved, and no backup was made: it moved while the history \
                     was made, or the backup could not be written ({error})"
                ));
            }
        }
    };
    Ok(Rewritten {
        branch,
        old,
        new,
        backup,
        commits: builder.commits,
    })
}

/// `cargo xtask branch …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let git = Git::at(&crate::workspace::root());
    match arguments.split_first() {
        Some((verb, rest)) if verb == "commit-records" => {
            let request = RecordsRequest::parse(rest)?;
            let made = commit_records(&git, &request)?;
            println!(
                "committed {} on {} (it was {}): {}",
                short(&made.new),
                made.branch,
                short(&made.old),
                request.message.lines().next().unwrap_or_default()
            );
            for file in &request.files {
                println!("  {file}");
            }
            println!(
                "The working tree and the branch checked out are as they were. Push it with: \
                 git push origin {}",
                made.branch
            );
            Ok(())
        }
        Some((verb, rest)) if verb == "fold" || verb == "to-end" => {
            let request = RewriteRequest::parse(verb, rest)?;
            let made = rewrite(&git, &request, crate::runs::now_seconds())?;
            println!(
                "{verb}: {} was {}, and is {}",
                made.branch,
                short(&made.old),
                short(&made.new)
            );
            for (was, is, subject) in &made.commits {
                println!("  {} (was {})  {subject}", short(is), short(was));
            }
            println!(
                "The old tip is kept as {}: `{}` takes {} back to it.",
                made.backup,
                undo(request.kind, &made.backup),
                made.branch
            );
            Ok(())
        }
        Some((verb, _)) if verb == "--help" || verb == "-h" => Err(USAGE.to_owned()),
        Some((other, _)) => Err(format!("unknown `branch` command `{other}`\n\n{USAGE}")),
        None => Err(USAGE.to_owned()),
    }
}

#[cfg(test)]
mod tests;
