//! `cargo xtask rename branch BASE..TIP`: a branch made before the rename,
//! made again in the new names, so that it rebases onto the renamed main.
//!
//! **The mechanism.** The branch's base, BASE, is passed through the same
//! function `apply` uses ([`super::transform`]), and so is each commit of
//! BASE..TIP. That gives a commit R(BASE), whose parent is BASE and whose
//! tree is BASE renamed, and a line of commits R(c) on top of it: each with
//! its tree renamed, its message and identities renamed, its dates kept.
//! R(BASE) is what `apply` makes of BASE, so `git rebase --onto <renamed
//! main> R(BASE) NAME` replays the differences R(c) − R(parent): the
//! branch's own changes, already in the new names. They meet the renamed
//! main with no rename between them, and stop only where the branch and
//! main changed the same lines.
//!
//! **How it is written.** The trees are read with `git ls-tree` and `git
//! cat-file --batch`, and the commits written by one `git fast-import`,
//! which builds the trees and commits from a stream and moves only the ref
//! it is given (a temporary one, removed after). A blob is renamed once for
//! each path it is found at: most files do not change from one commit to
//! the next. Nothing is checked out, and no ref but NAME moves.
//!
//! The same range gives the same commits every time: the dates are kept and
//! the function is the same, so a second run finds NAME already there.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use super::{Formatter, check_moves, in_parallel, rename_path, rename_text, transform};
use crate::Result;

/// `cargo xtask rename branch …`
pub(super) fn run(root: &Path, arguments: &[&str]) -> Result<()> {
    let options = Options::parse(arguments)?;
    let formatter = Formatter::at(root)?;
    let report = run_in(&Git::at(root), &options, &formatter)?;
    println!("{report}");
    Ok(())
}

/// The rewrite of `options`' range in the repository `git` works in, and the
/// branch it names set; returns what to print.
pub(super) fn run_in(git: &Git, options: &Options, formatter: &Formatter) -> Result<String> {
    let base = git.commit(&options.base)?;
    let tip = git.commit(&options.tip)?;
    let rewritten = rewrite(git, &base, &tip, formatter)?;
    let target = format!("refs/heads/{}", options.to);
    let short = |id: &str| id.chars().take(12).collect::<String>();
    match git.optional(&["rev-parse", "--verify", "--quiet", &target])? {
        Some(existing) if existing == rewritten.tip => {
            return Ok(format!(
                "{} already holds these commits ({}): nothing to change.",
                options.to,
                short(&rewritten.tip)
            ));
        }
        Some(existing) => {
            if !options.force {
                return Err(format!(
                    "{} is there already, at {}, and holds other commits. \
                     --force replaces it.",
                    options.to,
                    short(&existing)
                ));
            }
            git.refuse_in_use(&target)?;
            git.run(&["update-ref", &target, &rewritten.tip, &existing])?;
        }
        None => {
            git.run(&["update-ref", &target, &rewritten.tip, ""])?;
        }
    }
    Ok(format!(
        "Rewrote {} commit(s) of {}..{}: {} is at {}, on {} ({} renamed).\n\
         To bring it onto the renamed main:\n\n  \
         git rebase --onto origin/main {} {}\n\n\
         then `cargo xtask regen`, `cargo xtask outstanding` and `cargo xtask rename --check`.",
        rewritten.commits,
        options.base,
        options.tip,
        options.to,
        short(&rewritten.tip),
        short(&rewritten.base),
        options.base,
        rewritten.base,
        options.to,
    ))
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Options {
    pub(super) base: String,
    pub(super) tip: String,
    pub(super) to: String,
    pub(super) force: bool,
}

impl Options {
    pub(super) fn parse(arguments: &[&str]) -> Result<Self> {
        let mut range = None;
        let mut to = None;
        let mut force = false;
        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            match *argument {
                "--to" => {
                    to = Some(
                        rest.next()
                            .ok_or("--to needs the new branch's name")?
                            .to_string(),
                    );
                }
                "--force" => force = true,
                other if other.contains("..") && range.is_none() && !other.starts_with('-') => {
                    range = Some(other);
                }
                other => return Err(format!("unexpected `{other}`\n\n{}", super::USAGE)),
            }
        }
        let range = range.ok_or_else(|| format!("name the range BASE..TIP\n\n{}", super::USAGE))?;
        let (base, tip) = range
            .split_once("..")
            .filter(|(base, tip)| !base.is_empty() && !tip.is_empty() && !tip.starts_with('.'))
            .ok_or_else(|| format!("`{range}` is not BASE..TIP"))?;
        let to = match to {
            Some(to) => to,
            None => default_name(tip).ok_or_else(|| {
                format!("{tip} is not a branch's name: name the new branch with --to")
            })?,
        };
        if to.is_empty() || to.starts_with('-') || to.contains("..") {
            return Err(format!("`{to}` is not a branch's name"));
        }
        Ok(Self {
            base: base.to_owned(),
            tip: tip.to_owned(),
            to,
            force,
        })
    }
}

/// TIP's branch name with `-renamed`, when TIP is a name (`origin/` and
/// `refs/…/` taken off).
fn default_name(tip: &str) -> Option<String> {
    if tip.contains(['~', '^', ':', '@', '{', ' ']) || tip.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let name = tip
        .strip_prefix("refs/heads/")
        .or_else(|| tip.strip_prefix("refs/remotes/origin/"))
        .or_else(|| tip.strip_prefix("origin/"))
        .unwrap_or(tip);
    Some(format!("{name}-renamed"))
}

/// The commits a rewrite made.
pub(super) struct Rewritten {
    /// R(BASE): BASE's tree renamed, on BASE.
    pub(super) base: String,
    /// R(TIP).
    pub(super) tip: String,
    /// How many commits of the range.
    pub(super) commits: usize,
}

/// Makes R(BASE) and the commits of `base..tip` again, renamed (see the
/// module's comment), and returns them. No ref is left moved.
pub(super) fn rewrite(
    git: &Git,
    base: &str,
    tip: &str,
    formatter: &Formatter,
) -> Result<Rewritten> {
    if !git.succeeds(&["merge-base", "--is-ancestor", base, tip])? {
        return Err(format!("{base} is not an ancestor of {tip}"));
    }
    let listing = git.run(&[
        "rev-list",
        "--reverse",
        "--parents",
        &format!("{base}..{tip}"),
    ])?;
    let mut commits = Vec::new();
    let mut previous = base.to_owned();
    for line in listing.lines() {
        let mut ids = line.split(' ');
        let commit = ids.next().unwrap_or_default().to_owned();
        let parents: Vec<&str> = ids.collect();
        if parents.len() != 1 {
            return Err(format!(
                "{commit} has {} parents: the range must be a line of commits, with no merge",
                parents.len()
            ));
        }
        if parents[0] != previous {
            return Err(format!(
                "{commit} is not on {previous}: the range must be one line of commits from {base}"
            ));
        }
        previous.clone_from(&commit);
        commits.push(commit);
    }
    if commits.is_empty() {
        return Err(format!("there is no commit in {base}..{tip}"));
    }

    let temporary = format!("refs/xtask-rename/{}", std::process::id());
    let marks = PathBuf::from(git.run(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-path",
        &format!("xtask-rename-marks-{}", std::process::id()),
    ])?);
    let _ = git.run(&["update-ref", "-d", &temporary]);
    let mut reader = CatFile::start(git)?;
    let mut import = git.spawn(
        &[
            "fast-import",
            "--quiet",
            "--done",
            &format!("--export-marks={}", marks.display()),
        ],
        Stdio::null(),
    )?;
    let mut stream = import.stdin.take().ok_or("fast-import has no input")?;
    let mut writer = Writer {
        cache: HashMap::new(),
        next_mark: 1,
    };

    let written = (|| -> Result<Vec<u32>> {
        let mut made = Vec::new();
        let (base_raw, _) = reader.get(base)?;
        let base_info = CommitText::parse(&base_raw)?;
        let short: String = base.chars().take(12).collect();
        let message = format!(
            "Rename the project's names in {short}\n\n\
             The tree of {base}, passed through `cargo xtask rename`: the\n\
             upstream for a rebase of the commits after it onto the renamed main.\n"
        );
        let files = writer.tree(git, &mut reader, &mut stream, base, formatter)?;
        made.push(writer.commit(
            &mut stream,
            &temporary,
            &base_info.renamed_with(message),
            base,
            &files,
        )?);
        for commit in &commits {
            let (raw, _) = reader.get(commit)?;
            let info = CommitText::parse(&raw)?.renamed();
            let files = writer.tree(git, &mut reader, &mut stream, commit, formatter)?;
            let parent = format!(":{}", made[made.len() - 1]);
            made.push(writer.commit(&mut stream, &temporary, &info, &parent, &files)?);
        }
        stream
            .write_all(b"done\n")
            .map_err(|error| format!("{WRITE_FAILED}: {error}"))?;
        Ok(made)
    })();
    // A reason of the tool's own to stop (a link, a merge) comes between two
    // commands of the stream: end it there, so that fast-import stops as
    // asked and leaves no crash report, and say that reason.
    let own_stop = written
        .as_ref()
        .err()
        .filter(|error| !error.starts_with(WRITE_FAILED))
        .cloned();
    if own_stop.is_some() {
        let _ = stream.write_all(b"done\n");
    }
    drop(stream);
    let output = import
        .wait_with_output()
        .map_err(|error| format!("fast-import: {error}"))?;
    let _ = git.run(&["update-ref", "-d", &temporary]);
    let marks_text = std::fs::read_to_string(&marks).unwrap_or_default();
    let _ = std::fs::remove_file(&marks);
    if let Some(error) = own_stop {
        return Err(error);
    }
    // fast-import's own word first: a write that failed because it stopped
    // says only that the pipe closed.
    if !output.status.success() {
        return Err(format!(
            "fast-import failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let made = written?;
    let ids: HashMap<u32, &str> = marks_text
        .lines()
        .filter_map(|line| {
            let (mark, id) = line.strip_prefix(':')?.split_once(' ')?;
            Some((mark.parse().ok()?, id))
        })
        .collect();
    let id = |mark: u32| -> Result<String> {
        ids.get(&mark)
            .map(|id| (*id).to_owned())
            .ok_or_else(|| format!("fast-import did not say which commit mark {mark} is"))
    };
    Ok(Rewritten {
        base: id(made[0])?,
        tip: id(made[made.len() - 1])?,
        commits: commits.len(),
    })
}

/// How a failed write to fast-import begins: it stopped, and its own
/// message says why.
const WRITE_FAILED: &str = "could not write to fast-import";

/// One entry of a tree as fast-import is given it: its mode, the blob (an
/// object's id or a mark) and its renamed path.
struct File {
    mode: String,
    data: String,
    path: String,
}

/// What has been written to fast-import: each blob, at each path, renamed
/// once.
struct Writer {
    /// (the blob's id, its old path) → what to name it by in the stream.
    cache: HashMap<(String, String), String>,
    next_mark: u32,
}

impl Writer {
    /// The tree of `commit`, renamed: each blob not seen at its path before
    /// read, passed through the function on every core, and written if it
    /// changed.
    fn tree(
        &mut self,
        git: &Git,
        reader: &mut CatFile,
        stream: &mut ChildStdin,
        commit: &str,
        formatter: &Formatter,
    ) -> Result<Vec<File>> {
        let listing = git.bytes(&["ls-tree", "-r", "-z", "--full-tree", commit])?;
        let entries = parse_tree(&listing)?;
        check_moves(
            entries
                .iter()
                .map(|entry| (entry.path.as_str(), entry.new_path.as_str())),
        )?;

        let mut wanted = Vec::new();
        for entry in &entries {
            let key = (entry.id.clone(), entry.path.clone());
            if entry.kind == "blob" && !self.cache.contains_key(&key) && !wanted.contains(&key) {
                wanted.push(key);
            }
        }
        if let Some(link) = entries.iter().find(|entry| entry.mode == "120000") {
            return Err(format!(
                "{} in {commit} is a symbolic link: the rename does not rename links, \
                 as `apply` does not",
                link.path
            ));
        }
        let mut read = Vec::with_capacity(wanted.len());
        for (id, path) in wanted {
            let (bytes, kind) = reader.get(&id)?;
            if kind != "blob" {
                return Err(format!("{id} at {path} is a {kind}, not a blob"));
            }
            read.push((id, path, bytes));
        }
        let renamed = in_parallel(&read, |(_, path, bytes)| {
            transform(path, bytes, formatter).contents
        });
        for ((id, path, _), contents) in read.into_iter().zip(renamed) {
            let data = match contents {
                Some(contents) => {
                    let mark = self.next_mark;
                    self.next_mark += 1;
                    let mut blob =
                        format!("blob\nmark :{mark}\ndata {}\n", contents.len()).into_bytes();
                    blob.extend_from_slice(&contents);
                    blob.push(b'\n');
                    stream
                        .write_all(&blob)
                        .map_err(|error| format!("{WRITE_FAILED}: {error}"))?;
                    format!(":{mark}")
                }
                None => id.clone(),
            };
            self.cache.insert((id, path), data);
        }
        Ok(entries
            .into_iter()
            .map(|entry| {
                let data = if entry.kind == "blob" {
                    self.cache[&(entry.id.clone(), entry.path.clone())].clone()
                } else {
                    entry.id
                };
                File {
                    mode: entry.mode,
                    data,
                    path: entry.new_path,
                }
            })
            .collect())
    }

    /// Writes one commit, on `parent` (an id or a mark), and returns its mark.
    fn commit(
        &mut self,
        stream: &mut ChildStdin,
        reference: &str,
        info: &CommitText,
        parent: &str,
        files: &[File],
    ) -> Result<u32> {
        let mark = self.next_mark;
        self.next_mark += 1;
        let mut text = format!(
            "commit {reference}\nmark :{mark}\nauthor {}\ncommitter {}\n",
            info.author, info.committer
        );
        if let Some(encoding) = &info.encoding {
            text.push_str(&format!("encoding {encoding}\n"));
        }
        text.push_str(&format!(
            "data {}\n{}\nfrom {parent}\ndeleteall\n",
            info.message.len(),
            info.message
        ));
        for file in files {
            text.push_str(&format!(
                "M {} {} {}\n",
                file.mode,
                file.data,
                quote_path(&file.path)
            ));
        }
        text.push('\n');
        stream
            .write_all(text.as_bytes())
            .map_err(|error| format!("{WRITE_FAILED}: {error}"))?;
        Ok(mark)
    }
}

/// A path as fast-import reads it: quoted, as C quotes a string, when it
/// starts with a quote or holds a line's end.
pub(super) fn quote_path(path: &str) -> Cow<'_, str> {
    if !path.starts_with('"') && !path.contains('\n') {
        return Cow::Borrowed(path);
    }
    let mut quoted = String::from("\"");
    for c in path.chars() {
        match c {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\n' => quoted.push_str("\\n"),
            c => quoted.push(c),
        }
    }
    quoted.push('"');
    Cow::Owned(quoted)
}

use std::borrow::Cow;

/// One entry of `git ls-tree -r -z`.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Entry {
    pub(super) mode: String,
    pub(super) kind: String,
    pub(super) id: String,
    pub(super) path: String,
    pub(super) new_path: String,
}

pub(super) fn parse_tree(listing: &[u8]) -> Result<Vec<Entry>> {
    let text = std::str::from_utf8(listing).map_err(|_| "a path in the tree is not UTF-8")?;
    text.split('\0')
        .filter(|record| !record.is_empty())
        .map(|record| {
            let (head, path) = record
                .split_once('\t')
                .ok_or_else(|| format!("unexpected tree entry `{record}`"))?;
            let mut fields = head.split(' ');
            let (Some(mode), Some(kind), Some(id), None) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(format!("unexpected tree entry `{record}`"));
            };
            Ok(Entry {
                mode: mode.to_owned(),
                kind: kind.to_owned(),
                id: id.to_owned(),
                path: path.to_owned(),
                new_path: rename_path(path).into_owned(),
            })
        })
        .collect()
}

/// The parts of a commit's text that are made again.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct CommitText {
    pub(super) author: String,
    pub(super) committer: String,
    pub(super) encoding: Option<String>,
    pub(super) message: String,
}

impl CommitText {
    pub(super) fn parse(raw: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(raw).map_err(|_| "a commit's text is not UTF-8")?;
        let (head, message) = text.split_once("\n\n").unwrap_or((text, ""));
        let field = |name: &str| {
            head.lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix(' '))
                .map(str::to_owned)
        };
        Ok(Self {
            author: field("author").ok_or("a commit with no author")?,
            committer: field("committer").ok_or("a commit with no committer")?,
            encoding: field("encoding"),
            message: message.to_owned(),
        })
    }

    /// The identities and the message, renamed.
    pub(super) fn renamed(&self) -> Self {
        Self {
            author: rename_text("", &self.author).into_owned(),
            committer: rename_text("", &self.committer).into_owned(),
            encoding: self.encoding.clone(),
            message: rename_text("", &self.message).into_owned(),
        }
    }

    fn renamed_with(&self, message: String) -> Self {
        Self {
            message,
            ..self.renamed()
        }
    }
}

/// git in one repository, with nothing in the environment pointing it at
/// another (a hook exports `GIT_DIR` and its kin).
pub(super) struct Git {
    dir: PathBuf,
}

impl Git {
    pub(super) fn at(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
        }
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new("git");
        for variable in crate::owner_guard::GIT_LOCATION {
            command.env_remove(variable);
        }
        command
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .args(arguments)
            .current_dir(&self.dir);
        command
    }

    fn bytes(&self, arguments: &[&str]) -> Result<Vec<u8>> {
        let output = self
            .command(arguments)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("could not run git: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "git {} failed: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(output.stdout)
    }

    pub(super) fn run(&self, arguments: &[&str]) -> Result<String> {
        let bytes = self.bytes(arguments)?;
        Ok(String::from_utf8_lossy(&bytes).trim().to_owned())
    }

    fn succeeds(&self, arguments: &[&str]) -> Result<bool> {
        let status = self
            .command(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("could not run git: {error}"))?;
        Ok(status.success())
    }

    /// The output of a command that fails when what it looks for is not
    /// there (`rev-parse --verify --quiet`).
    fn optional(&self, arguments: &[&str]) -> Result<Option<String>> {
        let output = self
            .command(arguments)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("could not run git: {error}"))?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned()))
    }

    pub(super) fn commit(&self, name: &str) -> Result<String> {
        self.optional(&[
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{name}^{{commit}}"),
        ])?
        .ok_or_else(|| format!("{name} is not a commit"))
    }

    /// Fails when a worktree uses `reference`: has it checked out, or is in
    /// the middle of rebasing or bisecting it (its `HEAD` is detached then,
    /// and `git worktree list` names no branch). Moving it would leave that
    /// worktree out of step with its branch.
    fn refuse_in_use(&self, reference: &str) -> Result<()> {
        let common = PathBuf::from(self.run(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
        ])?);
        match crate::branch::used_elsewhere(&common, reference, None) {
            Some(refusal) => Err(format!("{refusal}; --force does not replace it")),
            None => Ok(()),
        }
    }

    fn spawn(&self, arguments: &[&str], stdout: Stdio) -> Result<Child> {
        self.command(arguments)
            .stdin(Stdio::piped())
            .stdout(stdout)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run git: {error}"))
    }
}

/// `git cat-file --batch`: objects read one at a time, by their ids.
struct CatFile {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl CatFile {
    fn start(git: &Git) -> Result<Self> {
        let mut child = git.spawn(&["cat-file", "--batch"], Stdio::piped())?;
        let input = child.stdin.take().ok_or("cat-file has no input")?;
        let output = BufReader::new(child.stdout.take().ok_or("cat-file has no output")?);
        Ok(Self {
            child,
            input,
            output,
        })
    }

    /// The object's bytes and its kind.
    fn get(&mut self, id: &str) -> Result<(Vec<u8>, String)> {
        let failed = |error: std::io::Error| format!("could not read {id} from git: {error}");
        writeln!(self.input, "{id}").map_err(failed)?;
        self.input.flush().map_err(failed)?;
        let mut header = String::new();
        self.output.read_line(&mut header).map_err(failed)?;
        let mut fields = header.trim_end().split(' ');
        let (Some(_), Some(kind), Some(size)) = (fields.next(), fields.next(), fields.next())
        else {
            return Err(format!("git has no object {id}: `{}`", header.trim_end()));
        };
        let size: usize = size
            .parse()
            .map_err(|_| format!("unexpected answer from cat-file: `{}`", header.trim_end()))?;
        let mut bytes = vec![0; size + 1];
        self.output.read_exact(&mut bytes).map_err(failed)?;
        bytes.pop();
        Ok((bytes, kind.to_owned()))
    }
}

impl Drop for CatFile {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
