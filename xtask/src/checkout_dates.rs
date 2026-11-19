//! The tests of `.github/scripts/date-the-checkout.sh`, which the Windows
//! test job runs after it restores the main branch's build of the
//! workspace's crates (`docs/testing-and-ci.md`, "The workspace's own build,
//! cached"). It decides what cargo takes as built, so a mistake in it serves
//! a stale build with every test green: these run it, with the bash CI runs
//! it with, on a repository of their own.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::testkit::Scratch;
use crate::testkit::scratch::git;

/// 2000-01-01, the date the script gives every file that has not changed.
const OLD: u64 = 946_684_800;

/// 2015-01-01, the date a test gives every file before the script runs, to
/// see which the script left alone.
const MARK: u64 = 1_420_070_400;

/// The variables that would point git at another repository, or at the
/// machine's own settings.
const ELSEWHERE: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_COMMON_DIR",
];

/// The bash the workflow's `shell: bash` is: Git's on Windows, found beside
/// git itself (`bash` on the path may be another), and the system's on Linux.
fn bash(any_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        let exec = git(any_dir, &["--exec-path"]);
        // <Git>/mingw64/libexec/git-core
        if let Some(found) = Path::new(&exec)
            .ancestors()
            .nth(3)
            .map(|git| git.join("bin").join("bash.exe"))
            .filter(|bash| bash.is_file())
        {
            return found;
        }
    }
    PathBuf::from("bash")
}

/// Runs the script in `repository` with `key`, as the job does, and
/// returns what it printed. It must end well whatever it decides.
fn date(repository: &Path, key: &str) -> String {
    let script = crate::workspace::root().join(".github/scripts/date-the-checkout.sh");
    let mut command = Command::new(bash(repository));
    for variable in ELSEWHERE {
        command.env_remove(variable);
    }
    let output = command
        .arg(&script)
        .arg(key)
        .current_dir(repository)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", repository.join("no-such-git-config"))
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "the script failed: {said}");
    said
}

/// The seconds since 1970 of a file's date.
fn date_of(path: &Path) -> u64 {
    fs::metadata(path)
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn set_date(path: &Path, seconds: u64) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
        .unwrap();
}

/// Every tracked file of the repository, and the files a test made.
const FILES: [&str; 6] = [
    ".gitignore",
    "src/same.rs",
    "src/changed.rs",
    "src/added.rs",
    "src/moved.rs",
    "target/ci/built.rlib",
];

/// A repository with the build's commit and a later one: `changed.rs`
/// changed, `deleted.rs` deleted, `renamed.rs` renamed to `moved.rs`,
/// `added.rs` added, `same.rs` as it was. And a build folder git ignores.
fn repository(scratch: &Scratch) -> (PathBuf, String) {
    let repository = scratch.git_init("repository");
    scratch.file("repository/.gitignore", "/target/\n");
    for name in ["same", "changed", "deleted", "renamed"] {
        scratch.file(format!("repository/src/{name}.rs"), format!("// {name}\n"));
    }
    let built = scratch.git_commit_all("repository", "the build's commit");
    scratch.file("repository/src/changed.rs", "// changed since\n");
    fs::remove_file(repository.join("src/deleted.rs")).unwrap();
    git(&repository, &["mv", "src/renamed.rs", "src/moved.rs"]);
    scratch.file("repository/src/added.rs", "// added\n");
    scratch.git_commit_all("repository", "the pull request");
    scratch.file("repository/target/ci/built.rlib", "a build");
    (repository, built)
}

/// Gives every file the mark, so that what the script leaves alone shows.
fn mark(repository: &Path) {
    for file in FILES {
        set_date(&repository.join(file), MARK);
    }
}

/// Every file still has the mark: the script dated nothing, and so cargo
/// rebuilds everything.
fn left_alone(repository: &Path, said: &str) {
    for file in FILES {
        assert_eq!(
            date_of(&repository.join(file)),
            MARK,
            "{file} was dated: {said}"
        );
    }
}

#[test]
fn sys_what_differs_from_the_builds_commit_is_dated_now_and_the_rest_long_before() {
    let scratch = Scratch::outside("xtask-dates", "differs");
    let (repository, built) = repository(&scratch);
    mark(&repository);
    let before = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let said = date(
        &repository,
        &format!("workspace-build-Windows-ab12-cd34-{built}"),
    );

    // Unchanged since the build: older than any build, so taken as built.
    for file in [".gitignore", "src/same.rs"] {
        assert_eq!(date_of(&repository.join(file)), OLD, "{file}: {said}");
    }
    // Changed, added, or renamed to: now, after the build, so rebuilt.
    for file in ["src/changed.rs", "src/added.rs", "src/moved.rs"] {
        let date = date_of(&repository.join(file));
        assert!(date + 1 >= before, "{file} is dated {date}: {said}");
    }
    // A deleted file is not made again, and the build folder is not dated.
    assert!(!repository.join("src/deleted.rs").exists());
    assert!(!repository.join("src/renamed.rs").exists());
    assert_eq!(date_of(&repository.join("target/ci/built.rlib")), MARK);
    assert!(
        said.contains(&format!("made from {built}; 5 file(s) differ")),
        "{said}"
    );
}

#[test]
fn sys_whatever_the_script_cannot_be_sure_of_leaves_every_date_alone() {
    let scratch = Scratch::outside("xtask-dates", "unsure");
    let (repository, built) = repository(&scratch);
    let key = format!("workspace-build-Windows-ab12-cd34-{built}");

    // Nothing restored, a key with no commit, a commit it cannot fetch (the
    // repository has no remote).
    for key in [
        String::new(),
        "workspace-build-Windows-ab12-cd34-nothing".to_owned(),
        format!(
            "workspace-build-Windows-ab12-cd34-{}",
            "0123456789".repeat(4)
        ),
    ] {
        mark(&repository);
        let said = date(&repository, &key);
        left_alone(&repository, &said);
    }

    // A file that is not the checkout's commit: new, changed, or ignored
    // outside the build folder.
    let stray = repository.join("src/stray.rs");
    fs::write(&stray, "// written after the checkout\n").unwrap();
    mark(&repository);
    let said = date(&repository, &key);
    left_alone(&repository, &said);
    assert!(said.contains("not its commit's"), "{said}");
    fs::remove_file(&stray).unwrap();

    fs::write(
        repository.join("src/same.rs"),
        "// changed, not committed\n",
    )
    .unwrap();
    mark(&repository);
    let said = date(&repository, &key);
    left_alone(&repository, &said);
    git(&repository, &["checkout", "--quiet", "--", "src/same.rs"]);

    fs::write(repository.join(".git/info/exclude"), "/junk\n").unwrap();
    fs::write(repository.join("junk"), "ignored, outside target/\n").unwrap();
    mark(&repository);
    let said = date(&repository, &key);
    left_alone(&repository, &said);
    fs::remove_file(repository.join("junk")).unwrap();

    // An attributes file changed since the build: the bytes on disk of a
    // file whose content git holds the same may differ.
    scratch.file("repository/src/.gitattributes", "*.rs text eol=crlf\n");
    scratch.git_commit_all("repository", "attributes");
    mark(&repository);
    let said = date(&repository, &key);
    left_alone(&repository, &said);
    assert!(said.contains(".gitattributes"), "{said}");
}
