use std::path::Path;

use cortex_testkit::Scratch;
use cortex_testkit::scratch::git;

use super::{
    CommitInfo, Git, Kind, RecordsRequest, RewriteRequest, backup_name, batch_check, batch_objects,
    chain_after, checked_out, commit_records, merge_tree_result, record_path, rewrite,
};

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn only_a_record_is_committed_and_its_path_is_read_from_the_top() {
    for (file, path) in [
        ("docs/steps/24/ci.md", "docs/steps/24/ci.md"),
        ("./docs/README.md", "docs/README.md"),
        (
            "docs\\steps\\tooling-10\\answered.md",
            "docs/steps/tooling-10/answered.md",
        ),
        (
            "mutations/tooling-10-runs.breaks",
            "mutations/tooling-10-runs.breaks",
        ),
    ] {
        assert_eq!(record_path(file).as_deref(), Ok(path), "{file}");
    }
    for file in [
        "xtask/src/main.rs",
        "docs/diagram.svg",
        "docs/.md",
        "mutations/sub/x.breaks",
        "mutations/.breaks",
        "AGENTS.md",
        "README.md",
    ] {
        let error = record_path(file).unwrap_err();
        assert!(error.contains("is not a record"), "{file}: {error}");
    }
    for file in [
        "",
        "/docs/a.md",
        "docs/../xtask/x.md",
        "docs//a.md",
        "C:/docs/a.md",
    ] {
        let error = record_path(file).unwrap_err();
        assert!(error.contains("is not a path inside"), "{file}: {error}");
    }
}

#[test]
fn a_request_is_read_and_what_it_lacks_is_said() {
    assert_eq!(
        RecordsRequest::parse(&strings(&[
            "--branch",
            "step-24",
            "-m",
            "Record the CI times",
            "docs/steps/24/ci.md",
        ])),
        Ok(RecordsRequest {
            branch: "step-24".to_owned(),
            message: "Record the CI times".to_owned(),
            files: strings(&["docs/steps/24/ci.md"]),
        })
    );
    for (words, said) in [
        (&["-m", "x", "docs/a.md"][..], "--branch BRANCH"),
        (&["--branch", "b", "docs/a.md"], "--message MESSAGE"),
        (
            &["--branch", "b", "-m", " ", "docs/a.md"],
            "--message MESSAGE",
        ),
        (&["--branch", "b", "-m", "x"], "name the records"),
        (&["--branch", "b", "-m", "x", "src/a.rs"], "is not a record"),
        (&["--branch", "b", "--push"], "unknown flag `--push`"),
    ] {
        let error = RecordsRequest::parse(&strings(words)).unwrap_err();
        assert!(error.contains(said), "{words:?}: {error}");
    }
    assert_eq!(
        RewriteRequest::parse("fold", &strings(&["abc", "--last", "--base", "main"])),
        Ok(RewriteRequest {
            kind: Kind::Fold { last: true },
            commit: "abc".to_owned(),
            base: "main".to_owned(),
        })
    );
    assert_eq!(
        RewriteRequest::parse("to-end", &strings(&["abc"])).map(|request| request.base),
        Ok("origin/main".to_owned())
    );
    for (verb, words, said) in [
        ("to-end", &["abc", "--last"][..], "unknown flag `--last`"),
        ("fold", &[], "name the commit to fold"),
        ("fold", &["a", "b"], "name one commit"),
    ] {
        let error = RewriteRequest::parse(verb, &strings(words)).unwrap_err();
        assert!(error.contains(said), "{verb} {words:?}: {error}");
    }
}

#[test]
fn the_commits_after_the_target_are_a_straight_line_or_refused() {
    // Parents first, as `rev-list --reverse --topo-order --parents` lists
    // them: a merge before the target is not in what is rewritten.
    let listed = "s1 b\ns2 s1\nm b s2\nt m\nc1 t\nc2 c1\n";
    assert_eq!(chain_after(listed, "t"), Some(Ok(strings(&["c1", "c2"]))));
    assert_eq!(chain_after(listed, "c2"), Some(Ok(Vec::new())));
    assert_eq!(chain_after(listed, "nowhere"), None);
    let error = chain_after(listed, "s2").unwrap().unwrap_err();
    assert!(error.contains("commit m is a merge"), "{error}");
    let error = chain_after("t b\nm t x\n", "t").unwrap().unwrap_err();
    assert!(error.contains("commit m is a merge"), "{error}");
    let error = chain_after("t b\nc1 t\nc2 t\n", "t").unwrap().unwrap_err();
    assert!(error.contains("commit c2 does not follow c1"), "{error}");
}

#[test]
fn the_worktrees_branches_and_a_commits_parts_are_read() {
    let listed = "worktree /work/main\nHEAD 1111\nbranch refs/heads/main\n\n\
                  worktree /work/detached\nHEAD 2222\ndetached\n\n\
                  worktree /work/other\nHEAD 3333\nbranch refs/heads/step-24\n";
    assert_eq!(
        checked_out(listed),
        [
            ("/work/main".to_owned(), "refs/heads/main".to_owned()),
            ("/work/other".to_owned(), "refs/heads/step-24".to_owned()),
        ]
    );
    let raw = "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
               parent 1111111111111111111111111111111111111111\n\
               author A. Writer <a@example.invalid> 1700000000 +0100\n\
               committer Someone Else <c@example.invalid> 1700000500 +0000\n\
               gpgsig -----BEGIN SIGNATURE-----\n \n -----END SIGNATURE-----\n\
               \n\
               Subject line\n\nBody, kept as it is.  \n";
    let info = CommitInfo::parse(raw).unwrap();
    assert_eq!(
        info,
        CommitInfo {
            tree: "4b825dc642cb6eb9a060e54bf8d69288fbee4904".to_owned(),
            parents: vec!["1111111111111111111111111111111111111111".to_owned()],
            author_name: "A. Writer".to_owned(),
            author_email: "a@example.invalid".to_owned(),
            author_date: "1700000000 +0100".to_owned(),
            message: "Subject line\n\nBody, kept as it is.  \n".to_owned(),
        }
    );
    assert_eq!(info.subject(), "Subject line");
    assert!(CommitInfo::parse("tree x\n\nno author\n").is_err());
    assert_eq!(merge_tree_result("abc\n", true), Ok("abc".to_owned()));
    assert_eq!(
        batch_check("1111 commit 230\nmain~9^{commit} missing\n2222 tree 40\nx ambiguous\n"),
        [Some("1111".to_owned()), None, Some("2222".to_owned()), None]
    );
    // Two objects: the first's text holds a line's end and ends without
    // one, the second's is a message of more than one line.
    let printed = b"aaaa commit 5\nab\ncd\nbbbb commit 3\nx\n\n\n";
    assert_eq!(
        batch_objects(printed),
        Ok(vec![
            ("commit".to_owned(), "ab\ncd".to_owned()),
            ("commit".to_owned(), "x\n\n".to_owned()),
        ])
    );
    assert!(batch_objects(b"aaaa commit 50\nshort\n").is_err());
    assert!(batch_objects(b"aaaa missing\n").is_err());
    assert_eq!(
        merge_tree_result("abc\nd.txt\nd.txt\nsrc/e.rs\n\n", false),
        Err(vec!["d.txt".to_owned(), "src/e.rs".to_owned()])
    );
    assert_eq!(
        backup_name("step/x", 1_700_000_000, 0),
        "refs/xtask-backup/step/x/1700000000"
    );
    assert_eq!(
        backup_name("b", 1_700_000_000, 2),
        "refs/xtask-backup/b/1700000000-2"
    );
}

/// git as the commands run it, on a test's repository: none of the
/// machine's configuration, and none of its identity from the environment,
/// so that the identity a commit gets is the repository's own.
fn isolated(dir: &Path) -> Git {
    let mut env: Vec<(&'static str, Option<std::ffi::OsString>)> = vec![
        ("GIT_CONFIG_NOSYSTEM", Some("1".into())),
        (
            "GIT_CONFIG_GLOBAL",
            Some(dir.join("no-such-git-config").into_os_string()),
        ),
    ];
    for variable in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_AUTHOR_DATE",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "GIT_COMMITTER_DATE",
    ] {
        env.push((variable, None));
    }
    Git {
        dir: dir.to_path_buf(),
        env,
    }
}

/// A repository with an identity of its own in its configuration, written
/// into its file: each git costs tens of milliseconds on Windows.
fn repository(scratch: &Scratch) -> std::path::PathBuf {
    let repo = scratch.git_init("repo");
    let config = repo.join(".git").join("config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str(
        "[user]\n\tname = Branch Tester\n\temail = branch@cortex-harness.invalid\n\
         [commit]\n\tgpgsign = false\n",
    );
    std::fs::write(&config, text).unwrap();
    repo
}

/// Everything a command might have touched: every ref, HEAD, the index's
/// entries and every file of the working tree, in two gits.
#[cfg(unix)]
fn state(repo: &Path) -> String {
    fn files(dir: &Path, found: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if path.is_dir() {
                files(&path, found);
            } else {
                found.push(format!(
                    "{} {:?}",
                    path.display(),
                    std::fs::read_to_string(&path).unwrap()
                ));
            }
        }
    }
    let mut found = vec![
        git(repo, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        std::fs::read_to_string(repo.join(".git").join("HEAD")).unwrap(),
        git(repo, &["ls-files", "--stage"]),
    ];
    files(repo, &mut found);
    found.join("\n--\n")
}

/// Makes the commits of `stream`, a `git fast-import` stream, in one git.
fn import(repo: &Path, stream: &str) {
    use std::io::Write as _;
    let mut child = std::process::Command::new("git")
        .args(["fast-import", "--quiet"])
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", repo.join("no-such-git-config"))
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

/// A commit of a `fast-import` stream on `branch`: mark `mark`, on the
/// commit marked `from`, with `file` holding `text`.
fn imported(branch: &str, mark: u32, from: Option<u32>, file: &str, text: &str) -> String {
    let at = 1_700_000_000 + u64::from(mark) * 60;
    let subject = file.trim_end_matches(".txt").to_uppercase();
    let from = from
        .map(|from| format!("from :{from}\n"))
        .unwrap_or_default();
    format!(
        "commit {branch}\nmark :{mark}\n\
         author Cortex Test <test@cortex-harness.invalid> {at} +0100\n\
         committer Cortex Test <test@cortex-harness.invalid> {at} +0100\n\
         data {}\n{subject}\n\n{from}M 100644 inline {file}\ndata {}\n{text}\n",
        subject.len() + 1,
        text.len()
    )
}

/// Both commands on Windows as on Linux, in as few gits as they can be
/// shown in (each starts in about 0.05 s on a hosted Windows runner): records
/// committed to a branch that is not checked out, with the system's paths,
/// an index of its own and the worktree list; and staged changes folded
/// into an earlier commit, through a copy of the index. The two tests after it hold the rest, on Unix: what they add is
/// git's own logic, the same on every system, and on Windows it would cost
/// seconds of the system tier.
#[test]
fn sys_records_reach_a_branch_not_checked_out_and_a_fold_rewrites_only_the_current_one() {
    let scratch = Scratch::outside("xtask-branch", "both");
    let repo = repository(&scratch);
    // `records` and origin/main at the base; A and B after it on main.
    let mut stream = imported("refs/heads/main", 1, None, "base.txt", "base\n");
    stream.push_str(&imported("refs/heads/main", 2, Some(1), "a.txt", "a\n"));
    stream.push_str(&imported("refs/heads/main", 3, Some(2), "b.txt", "b\n"));
    stream.push_str(
        "reset refs/remotes/origin/main\nfrom :1\n\nreset refs/heads/records\nfrom :1\n\n",
    );
    import(&repo, &stream);
    git(&repo, &["checkout", "--quiet", "main"]);
    let git_here = isolated(&repo);

    std::fs::create_dir_all(repo.join("docs").join("steps").join("24")).unwrap();
    std::fs::write(repo.join("docs/steps/24/ci.md"), "the times\n").unwrap();
    // HEAD, the index and the files, read with no git: nothing else runs
    // git between the two readings.
    let untouched = || {
        (
            std::fs::read(repo.join(".git").join("HEAD")).unwrap(),
            std::fs::read(repo.join(".git").join("index")).unwrap(),
            ["base.txt", "a.txt", "b.txt", "docs/steps/24/ci.md"]
                .map(|file| std::fs::read(repo.join(file)).unwrap()),
        )
    };
    let before = untouched();
    let request = |branch: &str| RecordsRequest {
        branch: branch.to_owned(),
        message: "Record the CI times".to_owned(),
        files: strings(&["docs/steps/24/ci.md"]),
    };
    let made = commit_records(&git_here, &request("records")).unwrap();
    assert!(
        untouched() == before,
        "the worktree, its index and HEAD are untouched"
    );
    let blob = git(&repo, &["hash-object", "docs/steps/24/ci.md"]);

    // A staged change to A's file, folded into A: B is made again on it.
    std::fs::write(repo.join("a.txt"), "a, mended\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    let moved = rewrite(
        &git_here,
        &RewriteRequest {
            kind: Kind::Fold { last: false },
            commit: "main~1".to_owned(),
            base: "origin/main".to_owned(),
        },
        1_700_000_000,
    )
    .unwrap();
    let read = git(
        &repo,
        &[
            "rev-parse",
            "records",
            "records^",
            "records:docs/steps/24/ci.md",
            "main",
            "main~1:a.txt",
            ":a.txt",
            "main~2",
            &moved.backup,
        ],
    );
    let read: Vec<&str> = read.lines().collect();
    assert_eq!(read[..2], [made.new.as_str(), made.old.as_str()]);
    assert_eq!(read[2], blob, "the record as the working tree has it");
    assert_eq!(read[3], moved.new);
    assert_eq!(read[4], read[5], "A has the file as it was staged");
    assert_eq!(read[6], made.old, "the base is not made again");
    assert_eq!(read[7], moved.old, "the backup holds the old tip");
    assert_eq!(moved.commits.len(), 2);
}

#[cfg(unix)]
#[test]
fn sys_records_are_committed_to_a_branch_not_checked_out_and_the_worktree_is_left_alone() {
    let scratch = Scratch::outside("xtask-branch", "records");
    let repo = repository(&scratch);
    std::fs::create_dir_all(repo.join("docs/steps/24")).unwrap();
    std::fs::write(repo.join("docs/steps/24/ci.md"), "old\n").unwrap();
    std::fs::write(repo.join("main.rs"), "fn main() {}\n").unwrap();
    git(&repo, &["add", "--all"]);
    git(&repo, &["commit", "--quiet", "-m", "base"]);
    git(&repo, &["branch", "step-24"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "held",
            "../held-worktree",
        ],
    );
    // The records as `ci-times --write` leaves them, while the worktree is on
    // `main`: one changed, one new, and a change that is not a record.
    std::fs::write(repo.join("docs/steps/24/ci.md"), "the times\n").unwrap();
    std::fs::write(repo.join("docs/steps/24/answered.md"), "answers\n").unwrap();
    std::fs::write(repo.join("main.rs"), "fn main() { todo!() }\n").unwrap();
    let before = state(&repo);

    let request = |branch: &str, files: &[&str]| RecordsRequest {
        branch: branch.to_owned(),
        message: "Record the CI times".to_owned(),
        files: strings(files),
    };
    let records = ["docs/steps/24/ci.md", "docs/steps/24/answered.md"];
    let git_here = isolated(&repo);
    let made = commit_records(&git_here, &request("step-24", &records)).unwrap();

    // One commit on top of the branch, with the records as the working tree
    // has them and nothing else, in the repository's identity.
    let blobs = git(&repo, &["hash-object", records[0], records[1]]);
    let read = git(
        &repo,
        &[
            "rev-parse",
            "step-24",
            "step-24^",
            "main",
            "step-24:docs/steps/24/ci.md",
            "step-24:docs/steps/24/answered.md",
            "step-24:main.rs",
            "main:main.rs",
        ],
    );
    let read: Vec<&str> = read.lines().collect();
    assert_eq!(read[..2], [made.new.as_str(), made.old.as_str()]);
    assert_eq!(made.old, read[2], "the branch was where main is");
    assert_eq!(read[3..5], blobs.lines().collect::<Vec<_>>()[..]);
    assert_eq!(read[5], read[6], "a file that is not a record is as it was");
    assert_eq!(
        git(
            &repo,
            &["log", "-1", "--format=%an <%ae>|%cn|%s", "step-24"]
        ),
        "Branch Tester <branch@cortex-harness.invalid>|Branch Tester|Record the CI times"
    );
    // The worktree, its index, its branch and every other ref: as they were,
    // but for the branch the commit is on.
    let without = |text: &str| {
        text.lines()
            .filter(|line| !line.starts_with("refs/heads/step-24 "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(without(&state(&repo)), without(&before));

    // What is refused, each before anything is written.
    let refused = |branch: &str, files: &[&str], said: &str| {
        let error = commit_records(&git_here, &request(branch, files)).unwrap_err();
        assert!(error.contains(said), "{branch} {files:?}: {error}");
    };
    refused("step-24", &records, "nothing changed");
    refused("main", &records, "is checked out in");
    refused("held", &records, "held-worktree");
    refused("no-such", &records, "no branch `no-such`");
    refused(
        "step-24",
        &["docs/steps/24/gone.md"],
        "not in the working tree",
    );
    assert_eq!(git(&repo, &["rev-parse", "step-24"]), made.new);
    assert_eq!(without(&state(&repo)), without(&before));
}

#[cfg(unix)]
#[test]
fn sys_fold_and_to_end_rewrite_only_the_current_branch_keep_a_backup_and_refuse_a_conflict() {
    let scratch = Scratch::outside("xtask-branch", "rewrite");
    let repo = repository(&scratch);
    // main and origin/main at the base, then A to D on `work`, and `keep` at
    // D: made in one git, and checked out in another.
    let mut stream = imported("refs/heads/main", 1, None, "base.txt", "base\n");
    for (mark, name) in [(2, "a"), (3, "b"), (4, "c"), (5, "d")] {
        stream.push_str(&imported(
            "refs/heads/work",
            mark,
            Some(mark - 1),
            &format!("{name}.txt"),
            &format!("{name}\n"),
        ));
    }
    stream
        .push_str("reset refs/remotes/origin/main\nfrom :1\n\nreset refs/heads/keep\nfrom :5\n\n");
    import(&repo, &stream);
    git(&repo, &["checkout", "--quiet", "work"]);
    let hashes = git(
        &repo,
        &["rev-parse", "work~3", "work~2", "work", "work^{tree}"],
    );
    let [a, b, d, d_tree]: [&str; 4] = hashes.lines().collect::<Vec<_>>().try_into().unwrap();
    // A commit's author, date and message as they are kept, the message's
    // last line's end included, and who committed it.
    let kept = |commit: &str| {
        git(
            &repo,
            &[
                "log",
                "-1",
                "--format=%an %ad [%B] %cn",
                "--date=raw",
                commit,
            ],
        )
    };
    // As a commit made again keeps them: the author's, and the repository's
    // identity as committer.
    let rewritten = |commit: &str| {
        let was = kept(commit);
        format!(
            "{} Branch Tester",
            was.strip_suffix(" Cortex Test").unwrap()
        )
    };
    let b_kept = rewritten(b);
    let subjects = |repo: &Path| git(repo, &["log", "--format=%s", "origin/main..work"]);
    let git_here = isolated(&repo);
    let request = |kind: Kind, commit: &str| RewriteRequest {
        kind,
        commit: commit.to_owned(),
        base: "origin/main".to_owned(),
    };

    // B to the end: the same tree at the tip, B last with its author, date
    // and message, the repository's identity as committer; A, another branch
    // and the backup of the old tip as they must be.
    let moved = rewrite(&git_here, &request(Kind::ToEnd, b), 1_700_000_000).unwrap();
    assert_eq!(subjects(&repo), "B\nD\nC\nA");
    assert_eq!(moved.backup, "refs/xtask-backup/work/1700000000");
    assert_eq!(moved.old, d);
    let now = git(
        &repo,
        &[
            "rev-parse",
            "work^{tree}",
            &moved.backup,
            "work",
            "work~3",
            "keep",
        ],
    );
    assert_eq!(
        now.lines().collect::<Vec<_>>(),
        [d_tree, d, moved.new.as_str(), a, d],
        "the tip's tree, the backup, the tip, the commit before B and another branch"
    );
    assert_eq!(kept("work"), b_kept);
    assert_eq!(moved.commits.len(), 3);

    // The last commit (B) folded into A: three commits, the same tree, and
    // B's file in A. The backup of the same second is a ref of its own.
    let moved = rewrite(
        &git_here,
        &request(Kind::Fold { last: true }, a),
        1_700_000_000,
    )
    .unwrap();
    assert_eq!(subjects(&repo), "D\nC\nA");
    assert_eq!(moved.backup, "refs/xtask-backup/work/1700000000-1");
    assert_eq!(
        git(&repo, &["rev-parse", "work^{tree}", "work~2:b.txt"]),
        format!("{d_tree}\n{}", git(&repo, &["hash-object", "b.txt"]))
    );
    assert_eq!(
        kept("work~2"),
        rewritten(a),
        "A keeps its author and message"
    );

    // The staged changes folded into C: C has them, and the tip is the
    // staged tree.
    std::fs::write(repo.join("c.txt"), "c, mended\n").unwrap();
    git(&repo, &["add", "c.txt"]);
    let staged = git(&repo, &["write-tree"]);
    rewrite(
        &git_here,
        &request(Kind::Fold { last: false }, "work~1"),
        1_700_000_001,
    )
    .unwrap();
    assert_eq!(subjects(&repo), "D\nC\nA");
    let read = git(
        &repo,
        &["rev-parse", "work^{tree}", "work~1:c.txt", ":c.txt"],
    );
    let read: Vec<&str> = read.lines().collect();
    assert_eq!(read[0], staged, "the tip is the staged tree");
    assert_eq!(read[1], read[2], "C has the file as it was staged");

    // What is refused, with nothing moved.
    let before = state(&repo);
    let refused = |kind: Kind, commit: &str, said: &str| {
        let error = rewrite(&git_here, &request(kind, commit), 1_700_000_002).unwrap_err();
        assert!(error.contains(said), "{kind:?} {commit}: {error}");
    };
    refused(Kind::Fold { last: false }, "work~1", "nothing is staged");
    refused(Kind::ToEnd, "origin/main", "is on origin/main already");
    refused(Kind::ToEnd, "keep", "is not on work");
    refused(Kind::ToEnd, "work", "at the end already");
    refused(Kind::Fold { last: true }, "work", "is the last of work");
    std::fs::write(repo.join("d.txt"), "not committed\n").unwrap();
    refused(Kind::ToEnd, "work~1", "changes not committed");
    std::fs::write(repo.join("d.txt"), "d\n").unwrap();
    assert_eq!(state(&repo), before, "a refusal moves nothing");

    // A conflict: E changes the file D made, so D cannot go after it. Every
    // ref, the index and the working tree are as they were, and no backup is
    // made.
    std::fs::write(repo.join("d.txt"), "d, changed\n").unwrap();
    git(&repo, &["commit", "--quiet", "-am", "E"]);
    let before = state(&repo);
    let error = rewrite(&git_here, &request(Kind::ToEnd, "work~1"), 1_700_000_003).unwrap_err();
    let e = git(&repo, &["rev-parse", "--short=12", "work"]);
    assert!(
        error.contains(&format!("commit {e} (E) does not apply")) && error.contains("d.txt"),
        "{error}"
    );
    assert_eq!(state(&repo), before);

    // A merge in the range.
    let merge = git(
        &repo,
        &[
            "commit-tree",
            "HEAD^{tree}",
            "-p",
            "HEAD",
            "-p",
            "keep",
            "-m",
            "merge",
        ],
    );
    git(&repo, &["update-ref", "refs/heads/work", &merge]);
    let error = rewrite(&git_here, &request(Kind::ToEnd, "work~2"), 1_700_000_004).unwrap_err();
    assert!(error.contains("is a merge"), "{error}");
    assert_eq!(git(&repo, &["rev-parse", "work"]), merge);
}
