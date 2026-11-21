#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    reason = "tests prepare real files, and fail by panicking"
)]

use std::path::Path;

use cineris_testkit::scratch::Scratch;

use super::*;

fn workspace(test: &str) -> Scratch {
    let scratch = Scratch::outside("cineris-sandbox-protect", test);
    scratch.file("ws/.git/config", "[core]\n");
    scratch.file("ws/.git/hooks/pre-commit.sample", "#!/bin/sh\n");
    scratch.file("ws/.git/objects/ab/cdef", "blob");
    scratch.file("ws/src/main.rs", "fn main() {}\n");
    scratch
}

/// The workspace's fixed list, as the lease gives it.
fn fixed(ws: &Path) -> Vec<PathBuf> {
    [
        ".git/hooks",
        ".git/config",
        ".git/info",
        ".cineris",
        ".github/workflows",
        ".bashrc",
    ]
    .iter()
    .map(|name| ws.join(name))
    .collect()
}

/// The standard limits with a time no machine reaches. A look stops at
/// [`Limits::time`] of real time, and a test that needs the whole of a
/// small workspace looked at must not depend on how busy the machine is:
/// the standard quarter of a second was reached once by a loaded runner
/// (`a_cargo_made_as_a_link_is_found`). The time bound itself is tested
/// with a time of zero, which stops every look on any machine.
fn unhurried() -> Limits {
    Limits {
        time: Duration::from_secs(3600),
        ..Limits::default()
    }
}

/// A watch on the workspace's fixed list, with [`unhurried`] limits.
fn unhurried_watch(ws: &Path) -> Watch {
    Watch::new(ws, fixed(ws)).with_limits(unhurried())
}

fn changes(scratch: &Scratch, change: impl FnOnce(&Path)) -> Vec<(String, Change)> {
    let ws = scratch.join("ws");
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    assert!(before.is_complete());
    change(&ws);
    let (changed, after) = watch.changes_since(&before);
    assert!(after.is_complete());
    changed
        .into_iter()
        .map(|changed| {
            let relative = changed.path.strip_prefix(&ws).unwrap();
            (
                relative.display().to_string().replace('\\', "/"),
                changed.change,
            )
        })
        .collect()
}

#[test]
fn nothing_changed_is_nothing_reported() {
    let scratch = workspace("nothing_changed");
    assert_eq!(changes(&scratch, |_| {}), Vec::new());
    // Changes outside what is protected are not reported either.
    let found = changes(&scratch, |ws| {
        fs::write(ws.join("src/main.rs"), "fn main() { x() }\n").unwrap();
        fs::write(ws.join(".git/objects/ab/new"), "blob").unwrap();
        fs::create_dir_all(ws.join("target/debug")).unwrap();
    });
    assert_eq!(found, Vec::new());
}

#[test]
fn a_hook_written_a_config_changed_and_a_file_removed_are_found() {
    let scratch = workspace("hooks_and_config");
    let found = changes(&scratch, |ws| {
        fs::write(ws.join(".git/hooks/pre-commit"), "#!/bin/sh\ncurl evil\n").unwrap();
        fs::write(ws.join(".git/config"), "[core]\nfsmonitor = ./run\n").unwrap();
        fs::remove_file(ws.join(".git/hooks/pre-commit.sample")).unwrap();
    });
    assert_eq!(
        found,
        vec![
            (".git/config".to_owned(), Change::Modified),
            (".git/hooks/pre-commit".to_owned(), Change::Created),
            (".git/hooks/pre-commit.sample".to_owned(), Change::Deleted),
        ]
    );
}

#[test]
fn what_was_there_before_is_kept_to_put_back() {
    let scratch = workspace("kept_to_put_back");
    let ws = scratch.join("ws");
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    fs::write(ws.join(".git/config"), "[core]\nfsmonitor = ./run\n").unwrap();
    fs::create_dir_all(ws.join(".github/workflows")).unwrap();
    fs::write(ws.join(".github/workflows/ci.yml"), "on: push\n").unwrap();
    let (changed, _) = watch.changes_since(&before);
    let config = changed
        .iter()
        .find(|changed| changed.path.ends_with(".git/config"))
        .unwrap();
    assert_eq!(config.before, Before::File(b"[core]\n".to_vec()));
    let workflow = changed
        .iter()
        .find(|changed| changed.path.ends_with("ci.yml"))
        .unwrap();
    assert_eq!(workflow.before, Before::Absent);
    assert!(
        changed
            .iter()
            .any(|changed| changed.path.ends_with(".github/workflows")
                && changed.change == Change::Created)
    );
}

// Windows keeps a change time too, which the standard library does not
// read; the check runs only where the sandbox enforces, which is Linux.
#[cfg(unix)]
#[test]
fn a_change_that_puts_the_modification_time_back_is_still_found() {
    let scratch = workspace("time_put_back");
    let ws = scratch.join("ws");
    let config = ws.join(".git/config");
    // Content too large to keep: only what the system sets, the change
    // time and the inode, can tell.
    let watch = unhurried_watch(&ws).with_limits(Limits {
        file_bytes: 0,
        ..unhurried()
    });
    let before = watch.take();
    let time = fs::metadata(&config).unwrap().modified().unwrap();
    // Same length, other content, the old time put back.
    fs::write(&config, "[xxxx]\n").unwrap();
    fs::File::options()
        .write(true)
        .open(&config)
        .unwrap()
        .set_modified(time)
        .unwrap();
    let (changed, _) = watch.changes_since(&before);
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert_eq!(changed[0].change, Change::Modified);
}

/// A file too large to keep or to hash is compared by its size and times
/// only, and marks the look incomplete, so the result says it was not
/// fully seen.
#[test]
fn a_file_too_large_to_hash_marks_the_look_incomplete() {
    let scratch = workspace("too_large_to_hash");
    let ws = scratch.join("ws");
    let config = ws.join(".git/config");
    fs::write(&config, vec![b'a'; 4096]).unwrap();
    let watch = unhurried_watch(&ws).with_limits(Limits {
        file_bytes: 0,
        hash_bytes: 1024,
        ..unhurried()
    });
    let before = watch.take();
    // The look did not keep or hash the file, so it says it is incomplete.
    assert!(!before.is_complete());
}

#[test]
fn protected_names_are_found_at_any_depth() {
    let scratch = workspace("any_depth");
    scratch.file("ws/vendor/lib/.git/config", "[core]\n");
    scratch.file("ws/.git/modules/sub/config", "[core]\n");
    scratch.dir("ws/pkg/.cargo");
    scratch.dir("ws/bare/objects");
    scratch.dir("ws/bare/refs");
    scratch.file("ws/bare/HEAD", "ref: refs/heads/main\n");
    let found = changes(&scratch, |ws| {
        fs::write(ws.join("vendor/lib/.git/config"), "[core]\nfsmonitor = x\n").unwrap();
        fs::create_dir_all(ws.join("vendor/lib/.git/hooks")).unwrap();
        fs::write(ws.join("vendor/lib/.git/hooks/post-checkout"), "x").unwrap();
        fs::write(ws.join(".git/modules/sub/config"), "[core]\nx = 1\n").unwrap();
        fs::write(ws.join("pkg/.cargo/config.toml"), "[build]\n").unwrap();
        fs::write(ws.join("pkg/web/.npmrc"), "registry=x").unwrap_or_else(|_| {
            fs::create_dir_all(ws.join("pkg/web")).unwrap();
            fs::write(ws.join("pkg/web/.npmrc"), "registry=x").unwrap();
        });
        fs::write(ws.join("deep/a/b/.envrc"), "x").unwrap_or_else(|_| {
            fs::create_dir_all(ws.join("deep/a/b")).unwrap();
            fs::write(ws.join("deep/a/b/.envrc"), "x").unwrap();
        });
        fs::write(ws.join("bare/config"), "[core]\n").unwrap();
        fs::write(ws.join("sub.git"), "gitdir: x").unwrap();
        fs::write(ws.join("lib.git"), "not a gitlink").unwrap();
        fs::write(ws.join("other/.git"), "gitdir: elsewhere").unwrap_or_else(|_| {
            fs::create_dir_all(ws.join("other")).unwrap();
            fs::write(ws.join("other/.git"), "gitdir: elsewhere").unwrap();
        });
    });
    let paths: Vec<&str> = found.iter().map(|(path, _)| path.as_str()).collect();
    for expected in [
        "vendor/lib/.git/config",
        "vendor/lib/.git/hooks",
        "vendor/lib/.git/hooks/post-checkout",
        ".git/modules/sub/config",
        "pkg/.cargo/config.toml",
        "pkg/web/.npmrc",
        "deep/a/b/.envrc",
        "bare/config",
        "other/.git",
    ] {
        assert!(paths.contains(&expected), "{expected} not in {paths:?}");
    }
    // Names that only look alike are not protected.
    assert!(
        !paths
            .iter()
            .any(|path| path.ends_with(".git") && path.starts_with("sub"))
    );
    assert!(!paths.contains(&"lib.git"));
}

#[cfg(unix)]
#[test]
fn a_link_is_compared_and_never_followed() {
    let scratch = workspace("links");
    scratch.file("elsewhere/hook", "#!/bin/sh\n");
    let target = scratch.join("elsewhere");
    let found = changes(&scratch, |ws| {
        std::os::unix::fs::symlink(&target, ws.join(".git/hooks/pre-push")).unwrap();
        std::os::unix::fs::symlink(&target, ws.join("linked")).unwrap();
    });
    assert_eq!(
        found,
        vec![(".git/hooks/pre-push".to_owned(), Change::Created)]
    );
}

#[test]
fn a_look_that_stops_at_its_bound_reports_nothing_beyond_it() {
    let scratch = workspace("bound_both_ways");
    // A protected name deeper than a look of ten directories can reach,
    // whatever order the listings come in.
    let deep: String = (0..14).map(|level| format!("d{level}/")).collect();
    scratch.file(format!("ws/{deep}.npmrc"), "registry=x");
    let ws = scratch.join("ws");
    let full = unhurried_watch(&ws);
    let capped = unhurried_watch(&ws).with_limits(Limits {
        directories: 10,
        ..unhurried()
    });
    let mut whole = full.take();
    let mut part = capped.take();
    assert!(whole.is_complete() && !part.is_complete());
    // Taken as if the files were written long before (a test cannot age a
    // file's change time): what changed since a look began, in a place it
    // did not reach, is named by that time, and nothing here did.
    whole.taken.0 += 3600;
    part.taken.0 += 3600;
    // Nothing changed: a later look that reached less sees nothing gone,
    // and one that reached more sees nothing new.
    assert_eq!(whole.compare(&part), Vec::new());
    assert_eq!(part.compare(&whole), Vec::new());
    // What both reached is still compared: the fixed list is always looked
    // at.
    fs::write(ws.join(".git/config"), "[core]\nx = 1\n").unwrap();
    let changed = whole.compare(&capped.take());
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert!(changed[0].path.ends_with(".git/config"));
}

#[cfg(unix)]
#[test]
fn a_linked_git_directory_is_not_looked_through() {
    let scratch = workspace("linked_git");
    scratch.file("elsewhere/config", "[core]\nsecret = 1\n");
    scratch.file("elsewhere/hooks/post-merge", "#!/bin/sh\n");
    let ws = scratch.join("ws");
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    fs::rename(ws.join(".git"), ws.join(".git.old")).unwrap();
    std::os::unix::fs::symlink(scratch.join("elsewhere"), ws.join(".git")).unwrap();
    let (changed, after) = watch.changes_since(&before);
    assert!(after.is_complete());
    // The link is what changed, and what was under the directory is gone;
    // nothing beyond the link is looked at or reported.
    let shown: Vec<(String, Change)> = changed
        .iter()
        .map(|one| {
            let path = one.path.strip_prefix(&ws).unwrap();
            (path.display().to_string(), one.change)
        })
        .collect();
    assert!(
        shown.contains(&(".git".to_owned(), Change::Created)),
        "{shown:?}"
    );
    assert!(
        shown.contains(&(".git/config".to_owned(), Change::Deleted)),
        "{shown:?}"
    );
    assert!(
        !shown.iter().any(|(path, _)| path.contains("post-merge")),
        "{shown:?}"
    );
    let config = changed
        .iter()
        .find(|one| one.path.ends_with(".git/config"))
        .unwrap();
    assert_eq!(config.before, Before::File(b"[core]\n".to_vec()));
}

#[cfg(unix)]
#[test]
fn a_change_of_mode_alone_is_found_and_cannot_be_put_back_by_content() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = workspace("mode");
    let ws = scratch.join("ws");
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    let hook = ws.join(".git/hooks/pre-commit.sample");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let (changed, _) = watch.changes_since(&before);
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert_eq!(changed[0].change, Change::Modified);
    assert_eq!(changed[0].before, Before::Unknown);
    // The same bytes written again change nothing that matters.
    let before = watch.take();
    let content = fs::read(ws.join(".git/config")).unwrap();
    fs::write(ws.join(".git/config"), content).unwrap();
    assert_eq!(watch.changes_since(&before).0, Vec::new());
}

#[test]
fn past_its_limits_a_look_says_it_is_incomplete() {
    let scratch = workspace("limits");
    for index in 0..20 {
        scratch.dir(format!("ws/many/d{index}"));
    }
    let ws = scratch.join("ws");
    let limits = Limits {
        directories: 5,
        ..unhurried()
    };
    let watch = unhurried_watch(&ws).with_limits(limits);
    let before = watch.take();
    assert!(!before.is_complete());
    // A change to what was looked at is still found.
    fs::write(ws.join(".git/config"), "[core]\nx = 1\n").unwrap();
    let (changed, _) = watch.changes_since(&before);
    assert!(
        changed
            .iter()
            .any(|changed| changed.path.ends_with(".git/config"))
    );

    // Content past the limits is compared by size and times, not kept.
    let small = Limits {
        file_bytes: 3,
        ..unhurried()
    };
    let watch = unhurried_watch(&ws).with_limits(small);
    let before = watch.take();
    fs::write(ws.join(".git/config"), "[core]\ny = 22\n").unwrap();
    let (changed, _) = watch.changes_since(&before);
    let config = changed
        .iter()
        .find(|changed| changed.path.ends_with(".git/config"))
        .unwrap();
    assert_eq!(config.before, Before::Unknown);
}

/// A look past its time stops, says it is incomplete, and reports nothing
/// beyond where it stopped; what it asked for by name is still compared. A
/// time of zero is past at the first check, so the look stops there on any
/// machine, however fast: the walk lists nothing, and of the fixed list
/// each path is recorded and none is looked into.
#[test]
fn past_its_time_a_look_says_it_is_incomplete() {
    let scratch = workspace("out_of_time");
    scratch.file("ws/pkg/.npmrc", "registry=x");
    let ws = scratch.join("ws");
    let hurried = unhurried_watch(&ws).with_limits(Limits {
        time: Duration::ZERO,
        ..unhurried()
    });
    let before = hurried.take();
    assert!(!before.is_complete());
    let recorded: Vec<String> = before
        .entries
        .keys()
        .map(|path| {
            let relative = path.strip_prefix(&ws).unwrap();
            relative.display().to_string().replace('\\', "/")
        })
        .collect();
    assert_eq!(recorded, [".git/config", ".git/hooks"], "{recorded:?}");
    assert!(before.listed.is_empty(), "{:?}", before.listed);

    fs::write(ws.join(".git/config"), "[core]\nx = 1\n").unwrap();
    fs::write(ws.join(".git/hooks/pre-commit"), "#!/bin/sh\n").unwrap();
    fs::write(ws.join("pkg/.npmrc"), "registry=y").unwrap();
    let (changed, after) = hurried.changes_since(&before);
    assert!(!after.is_complete());
    let found: Vec<(String, Change)> = changed
        .iter()
        .map(|one| {
            let relative = one.path.strip_prefix(&ws).unwrap();
            (
                relative.display().to_string().replace('\\', "/"),
                one.change,
            )
        })
        .collect();
    assert_eq!(found, [(".git/config".to_owned(), Change::Modified)]);

    // The same workspace with time to spare is looked at whole, and the
    // changes past where the hurried look stopped are there to be found.
    let whole = unhurried_watch(&ws).take();
    assert!(whole.is_complete());
    assert!(
        whole
            .entries
            .keys()
            .any(|path| path.ends_with("pkg/.npmrc"))
            && whole
                .entries
                .keys()
                .any(|path| path.ends_with(".git/hooks/pre-commit"))
    );
}

#[cfg(unix)]
#[test]
fn a_change_of_content_and_mode_keeps_the_content() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = workspace("content_and_mode");
    let ws = scratch.join("ws");
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    let config = ws.join(".git/config");
    fs::write(&config, "[core]\nhooksPath = /srv/x\n").unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    let (changed, _) = watch.changes_since(&before);
    assert_eq!(changed.len(), 1, "{changed:?}");
    // The content can still be put back; the mode is put back by hand.
    assert_eq!(changed[0].before, Before::File(b"[core]\n".to_vec()));
    assert!(changed[0].mode_changed);
}

#[cfg(unix)]
#[test]
fn a_linked_module_or_worktree_is_not_looked_through() {
    let scratch = workspace("linked_module");
    scratch.file("elsewhere/config", "[core]\n");
    scratch.file("elsewhere/gitdir", "/srv/one\n");
    let ws = scratch.join("ws");
    fs::create_dir_all(ws.join(".git/modules")).unwrap();
    fs::create_dir_all(ws.join(".git/worktrees")).unwrap();
    std::os::unix::fs::symlink(scratch.join("elsewhere"), ws.join(".git/modules/sub")).unwrap();
    std::os::unix::fs::symlink(scratch.join("elsewhere"), ws.join(".git/worktrees/w")).unwrap();
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    // What the links lead to is not the workspace's, and is not looked at.
    assert!(
        !before
            .entries
            .keys()
            .any(|path| path.starts_with(ws.join(".git/modules/sub/"))
                && path != &ws.join(".git/modules/sub")
                || path.starts_with(ws.join(".git/worktrees/w/"))
                    && path != &ws.join(".git/worktrees/w")),
        "{:?}",
        before.entries.keys().collect::<Vec<_>>()
    );
    fs::write(scratch.join("elsewhere/config"), "[core]\nx = 1\n").unwrap();
    fs::write(scratch.join("elsewhere/gitdir"), "/srv/two\n").unwrap();
    assert_eq!(watch.changes_since(&before).0, Vec::new());
}

/// A `commondir` in the main repository points git's configuration and
/// hooks at another directory. The pointer, and what it names, are watched
/// now, where before only a worktree's `commondir` was (finding 3 of the
/// third review of step 26).
#[test]
fn a_commondir_in_the_main_repository_is_found() {
    let scratch = workspace("commondir");
    let found = changes(&scratch, |ws| {
        fs::create_dir_all(ws.join("build/common/objects")).unwrap();
        fs::create_dir_all(ws.join("build/common/hooks")).unwrap();
        fs::write(
            ws.join("build/common/config"),
            "[core]\n\tfsmonitor = ./build/run.sh\n",
        )
        .unwrap();
        fs::write(ws.join("build/common/hooks/pre-commit"), "#!/bin/sh\n").unwrap();
        fs::write(ws.join(".git/commondir"), "../build/common\n").unwrap();
    });
    assert!(
        found
            .iter()
            .any(|(path, change)| path == ".git/commondir" && *change == Change::Created),
        "{found:?}"
    );
    assert!(
        found.iter().any(|(path, _)| path == "build/common/config"),
        "{found:?}"
    );
    assert!(
        found
            .iter()
            .any(|(path, _)| path == "build/common/hooks/pre-commit"),
        "{found:?}"
    );
}

/// A `.cargo` made as a link to a directory that holds a `config.toml` was
/// passed over: the walk recorded `.cargo` only as a real directory. It is
/// recorded now, at any depth, with its configuration by name through the
/// link (finding 4 of the third review of step 26).
#[cfg(unix)]
#[test]
fn a_cargo_made_as_a_link_is_found() {
    let scratch = workspace("cargo_link");
    let ws = scratch.join("ws");
    fs::create_dir_all(ws.join("vendor/x")).unwrap();
    fs::write(
        ws.join("vendor/x/config.toml"),
        "[build]\nrustc-wrapper = \"./vendor/run.sh\"\n",
    )
    .unwrap();
    fs::create_dir_all(ws.join("pkg")).unwrap();
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    assert!(before.is_complete());
    std::os::unix::fs::symlink(ws.join("vendor/x"), ws.join(".cargo")).unwrap();
    std::os::unix::fs::symlink(ws.join("vendor/x"), ws.join("pkg/.cargo")).unwrap();
    let (changed, after) = watch.changes_since(&before);
    assert!(after.is_complete());
    let shown: Vec<String> = changed
        .iter()
        .map(|one| {
            one.path
                .strip_prefix(&ws)
                .unwrap()
                .display()
                .to_string()
                .replace('\\', "/")
        })
        .collect();
    assert!(
        shown.iter().any(|path| path == ".cargo") && shown.iter().any(|path| path == "pkg/.cargo"),
        "a .cargo link was not found: {shown:?}"
    );
    // The link is recorded as a link, never looked through as a tree: its
    // real target's name is not among the entries.
    assert!(
        !shown.iter().any(|path| path.starts_with("vendor/")),
        "the link was looked through: {shown:?}"
    );
}

/// A `commondir` a confined command wrote to point outside the workspace is
/// recorded as a pointer, but not followed: the look must never read or name
/// a file outside the workspace (finding 1 of the independent review of the
/// step 26 fixes).
#[test]
fn a_commondir_pointing_outside_the_workspace_is_not_followed() {
    let scratch = workspace("commondir_outside");
    // A sibling of the workspace, reached by `..` from inside `.git`.
    fs::create_dir_all(scratch.join("common-outside/hooks")).unwrap();
    fs::write(scratch.join("common-outside/config"), "[core]\n").unwrap();
    fs::write(
        scratch.join("common-outside/hooks/pre-commit"),
        "#!/bin/sh\n",
    )
    .unwrap();
    let ws = scratch.join("ws");
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    assert!(before.is_complete());
    fs::write(ws.join(".git/commondir"), "../../common-outside\n").unwrap();
    let (changed, _after) = watch.changes_since(&before);
    let paths: Vec<String> = changed
        .iter()
        .map(|one| one.path.display().to_string().replace('\\', "/"))
        .collect();
    assert!(
        paths.iter().any(|path| path.ends_with(".git/commondir")),
        "the pointer was not recorded: {paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path.contains("common-outside")),
        "the look followed the pointer outside the workspace: {paths:?}"
    );
}

/// A `.cargo` made as a link to somewhere outside the workspace is recorded
/// as the link, but not looked through: a confined command cannot change a
/// file outside the write roots, and the look must not read or name one
/// (finding 1 of the independent review of the step 26 fixes).
#[cfg(unix)]
#[test]
fn a_cargo_link_leading_outside_is_not_followed() {
    let scratch = workspace("cargo_outside");
    fs::create_dir_all(scratch.join("cargo-outside")).unwrap();
    fs::write(
        scratch.join("cargo-outside/config.toml"),
        "[build]\nrustc-wrapper = \"/tmp/run.sh\"\n",
    )
    .unwrap();
    let ws = scratch.join("ws");
    let watch = unhurried_watch(&ws);
    let before = watch.take();
    assert!(before.is_complete());
    std::os::unix::fs::symlink(scratch.join("cargo-outside"), ws.join(".cargo")).unwrap();
    let (changed, _after) = watch.changes_since(&before);
    let paths: Vec<String> = changed
        .iter()
        .map(|one| one.path.display().to_string().replace('\\', "/"))
        .collect();
    assert!(
        paths.iter().any(|path| path.ends_with("/.cargo")),
        "the link was not recorded: {paths:?}"
    );
    assert!(
        !paths.iter().any(|path| path.contains("cargo-outside"))
            && !paths
                .iter()
                .any(|path| path.ends_with(".cargo/config.toml")),
        "the look followed the link outside the workspace: {paths:?}"
    );
}

/// Putting back what a confined start changed (the third security fix):
/// each path is made as the look before found it, never through a link, and
/// a look taken after says what is left. Links are planted at the path, at
/// a directory on its way, above it, and swapped in while it is put back.
#[cfg(target_os = "linux")]
mod putting_back {
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
    }

    fn set_mode(path: &Path, mode: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Looks, makes `change`, and puts back what changed: what it was
    /// given, and what it did.
    fn put_back_after(ws: &Path, change: impl FnOnce(&Path)) -> (Vec<Changed>, PutBack) {
        let watch = unhurried_watch(ws);
        let before = watch.take();
        assert!(before.is_complete());
        change(ws);
        let (changed, _) = watch.changes_since(&before);
        assert!(!changed.is_empty(), "the change was not found");
        let kept = ws.parent().unwrap().join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        (changed, put)
    }

    /// The names in a directory, which a put-back must leave nothing of its
    /// own in.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn what_a_start_changed_is_made_as_it_was() {
        let scratch = workspace("put_back");
        let ws = scratch.join("ws");
        set_mode(&ws.join(".git/hooks/pre-commit.sample"), 0o644);
        let (changed, put) = put_back_after(&ws, |ws| {
            fs::write(ws.join(".git/config"), "[filter \"x\"]\n\tclean = evil\n").unwrap();
            fs::write(ws.join(".git/hooks/pre-commit"), "#!/bin/sh\nevil\n").unwrap();
            set_mode(&ws.join(".git/hooks/pre-commit"), 0o755);
            // A change of mode alone.
            set_mode(&ws.join(".git/hooks/pre-commit.sample"), 0o755);
            fs::create_dir_all(ws.join(".github/workflows")).unwrap();
            fs::write(ws.join(".github/workflows/ci.yml"), "on: push\n").unwrap();
            fs::create_dir_all(ws.join(".git/info")).unwrap();
            fs::write(ws.join(".git/info/attributes"), "* filter=x\n").unwrap();
        });
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        assert_eq!(put.restored.len(), changed.len(), "{:?}", put.restored);
        assert_eq!(
            fs::read_to_string(ws.join(".git/config")).unwrap(),
            "[core]\n"
        );
        assert!(!ws.join(".git/hooks/pre-commit").exists());
        assert_eq!(mode(&ws.join(".git/hooks/pre-commit.sample")), 0o644);
        assert!(!ws.join(".github/workflows").exists());
        assert!(!ws.join(".git/info").exists());
        // Nothing of the put-back's own is left beside what it wrote.
        assert_eq!(names(&ws.join(".git/hooks")), ["pre-commit.sample"]);
        assert!(
            !names(&ws.join(".git"))
                .iter()
                .any(|name| name.contains("cineris")),
            "{:?}",
            names(&ws.join(".git"))
        );
    }

    #[test]
    fn a_file_and_a_directory_that_went_are_made_again() {
        let scratch = workspace("put_back_gone");
        let ws = scratch.join("ws");
        set_mode(&ws.join(".git/hooks"), 0o750);
        set_mode(&ws.join(".git/config"), 0o640);
        let (_, put) = put_back_after(&ws, |ws| {
            fs::remove_dir_all(ws.join(".git/hooks")).unwrap();
            fs::remove_file(ws.join(".git/config")).unwrap();
        });
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        assert_eq!(
            fs::read_to_string(ws.join(".git/config")).unwrap(),
            "[core]\n"
        );
        assert_eq!(mode(&ws.join(".git/config")), 0o640);
        assert_eq!(mode(&ws.join(".git/hooks")), 0o750);
        assert_eq!(
            fs::read_to_string(ws.join(".git/hooks/pre-commit.sample")).unwrap(),
            "#!/bin/sh\n"
        );
    }

    /// A link put at a protected file, at a directory on the way to one, or
    /// where nothing was, is removed itself: what it leads to, outside the
    /// workspace, is never written, and the path is made again.
    #[test]
    fn a_link_put_at_a_protected_path_is_removed_and_never_followed() {
        let scratch = workspace("put_back_links");
        let ws = scratch.join("ws");
        let outside = scratch.join("outside");
        scratch.file("outside/config", "outside\n");
        scratch.file("outside/hooks/pre-commit", "outside hook\n");
        let (_, put) = put_back_after(&ws, |ws| {
            fs::remove_file(ws.join(".git/config")).unwrap();
            symlink(outside.join("config"), ws.join(".git/config")).unwrap();
            fs::remove_dir_all(ws.join(".git/hooks")).unwrap();
            symlink(outside.join("hooks"), ws.join(".git/hooks")).unwrap();
            symlink(outside.join("nowhere"), ws.join(".cineris")).unwrap();
        });
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        let config = fs::symlink_metadata(ws.join(".git/config")).unwrap();
        assert!(config.is_file(), "{config:?}");
        assert_eq!(
            fs::read_to_string(ws.join(".git/config")).unwrap(),
            "[core]\n"
        );
        assert!(
            fs::symlink_metadata(ws.join(".git/hooks"))
                .unwrap()
                .is_dir()
        );
        assert_eq!(names(&ws.join(".git/hooks")), ["pre-commit.sample"]);
        assert!(fs::symlink_metadata(ws.join(".cineris")).is_err());
        // What the links led to is as it was.
        assert_eq!(
            fs::read_to_string(outside.join("config")).unwrap(),
            "outside\n"
        );
        assert_eq!(names(&outside.join("hooks")), ["pre-commit"]);
        assert_eq!(
            fs::read_to_string(outside.join("hooks/pre-commit")).unwrap(),
            "outside hook\n"
        );
        assert!(!outside.join("nowhere").exists());
    }

    /// A path whose directory, above the protected one, is a link now
    /// (`.git` itself, to a folder outside with a configuration of its own)
    /// is not put back: nothing is written where the link leads, and the
    /// path is left, to be put back by hand.
    #[test]
    fn a_path_whose_directory_is_a_link_now_is_left_alone() {
        let scratch = workspace("put_back_through");
        let ws = scratch.join("ws");
        let planted = "[filter \"x\"]\n\tclean = evil\n";
        scratch.file("outside/git/config", planted);
        let (_, put) = put_back_after(&ws, |ws| {
            fs::rename(ws.join(".git"), scratch.join("moved-git")).unwrap();
            symlink(scratch.join("outside/git"), ws.join(".git")).unwrap();
        });
        assert_eq!(
            fs::read_to_string(scratch.join("outside/git/config")).unwrap(),
            planted
        );
        assert_eq!(names(&scratch.join("outside/git")), ["config"]);
        assert!(
            put.left
                .iter()
                .any(|one| one.path == ws.join(".git/config")),
            "{:?}",
            put.left
        );
    }

    /// A folder above a protected one, not watched itself (`.github`, above
    /// `.github/workflows`), swapped for a link to a folder outside that
    /// holds a `workflows` of its own: nothing on the way to a protected
    /// path is followed when it is put back, so nothing is written or given
    /// a mode where the link leads, and the paths are left, to be put back
    /// by hand.
    #[test]
    fn a_path_under_a_folder_that_is_a_link_now_is_left_alone() {
        let scratch = workspace("put_back_under_a_link");
        let ws = scratch.join("ws");
        scratch.file("ws/.github/workflows/ci.yml", "on: push\n");
        let theirs = scratch.file("outside/github/workflows/ci.yml", "theirs\n");
        let folder = scratch.join("outside/github/workflows");
        set_mode(&folder, 0o751);
        let (_, put) = put_back_after(&ws, |ws| {
            fs::rename(ws.join(".github"), scratch.join("moved-github")).unwrap();
            symlink(scratch.join("outside/github"), ws.join(".github")).unwrap();
        });
        assert_eq!(fs::read_to_string(&theirs).unwrap(), "theirs\n");
        assert_eq!(mode(&folder), 0o751);
        assert!(
            put.left
                .iter()
                .any(|one| one.path == ws.join(".github/workflows/ci.yml")),
            "{:?}",
            put.left
        );
    }

    /// The directory a path lies in is swapped for a link to a folder
    /// outside after it was opened and before the path is written: the
    /// write goes to the directory that was opened, never where the link
    /// leads, and the path, reached through a link now, is left.
    #[test]
    fn a_directory_swapped_for_a_link_while_it_is_put_back_takes_nothing_elsewhere() {
        let scratch = workspace("put_back_race_dir");
        let ws = scratch.join("ws");
        let outside = scratch.dir("outside");
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        fs::write(ws.join(".git/hooks/pre-commit.sample"), "evil\n").unwrap();
        let (changed, _) = watch.changes_since(&before);
        assert_eq!(changed.len(), 1, "{changed:?}");
        let hold = cineris_testkit::Hold::new(PUTTING_BACK);
        let kept = ws.parent().unwrap().join("kept");
        let putting = std::thread::spawn({
            let watch = watch.clone();
            move || watch.put_back(&before, &changed, &|_| true, Some(&kept))
        });
        hold.reach();
        fs::rename(ws.join(".git/hooks"), ws.join(".git/hooks-moved")).unwrap();
        symlink(&outside, ws.join(".git/hooks")).unwrap();
        hold.release();
        let put = putting.join().unwrap();
        assert!(
            names(&outside).is_empty(),
            "the write went through the link: {:?}",
            names(&outside)
        );
        assert_eq!(
            fs::read_to_string(ws.join(".git/hooks-moved/pre-commit.sample")).unwrap(),
            "#!/bin/sh\n"
        );
        // A later round removes the link itself and makes the folder again,
        // with what was in it.
        assert!(
            put.left.is_empty()
                == fs::symlink_metadata(ws.join(".git/hooks"))
                    .unwrap()
                    .is_dir(),
            "{:?}",
            put.left
        );
    }

    /// A hook planted in `.git/hooks`, its folder then made searchable and
    /// not readable (git still finds a hook by its name): the look after
    /// cannot list the folder and says it is incomplete rather than take
    /// what is in it for gone; the put-back gives the folder its mode back
    /// without needing to read it, looks again, and in a second round puts
    /// back the hook it can see now (found by the early review of the third
    /// security fix).
    #[test]
    fn a_hook_hidden_in_a_folder_made_unreadable_is_found_and_put_back() {
        let scratch = workspace("put_back_unreadable");
        let ws = scratch.join("ws");
        let hooks = ws.join(".git/hooks");
        let was = mode(&hooks);
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        assert!(before.is_complete());
        fs::write(hooks.join("pre-commit"), "#!/bin/sh\nevil\n").unwrap();
        set_mode(&hooks.join("pre-commit"), 0o755);
        set_mode(&hooks, 0o333);
        if fs::read_dir(&hooks).is_ok() {
            // A user that reads past modes (root): nothing is hidden.
            set_mode(&hooks, 0o755);
            return;
        }
        let (changed, after) = watch.changes_since(&before);
        assert!(
            !after.is_complete(),
            "an unreadable folder was taken as looked at"
        );
        assert!(
            !changed
                .iter()
                .any(|one| one.path.ends_with("pre-commit.sample")),
            "what is in an unreadable folder was taken for gone: {changed:?}"
        );
        let kept = ws.parent().unwrap().join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        let mode_now = mode(&hooks);
        set_mode(&hooks, 0o755);
        assert_eq!(mode_now, was, "{mode_now:o}");
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        assert!(
            put.changed
                .iter()
                .any(|one| one.path == hooks.join("pre-commit")),
            "the hook it hid is not named: {:?}",
            put.changed
        );
        assert!(!hooks.join("pre-commit").exists());
        assert!(put.after.is_complete());
    }

    /// A directory whose mode lacks its owner's write right gets back what
    /// was in it: it is made again open to its owner, its children are put
    /// back, and its own mode is given last.
    #[test]
    fn a_directory_without_its_owners_write_right_gets_its_children_back() {
        let scratch = workspace("put_back_read_only_dir");
        let ws = scratch.join("ws");
        let hooks = ws.join(".git/hooks");
        set_mode(&hooks, 0o555);
        let (_, put) = put_back_after(&ws, |_| {
            set_mode(&hooks, 0o755);
            fs::remove_dir_all(&hooks).unwrap();
        });
        let mode_now = mode(&hooks);
        set_mode(&hooks, 0o755);
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        assert_eq!(mode_now, 0o555);
        assert_eq!(
            fs::read_to_string(hooks.join("pre-commit.sample")).unwrap(),
            "#!/bin/sh\n"
        );
    }

    /// A protected path that was a link (a `.cargo/config.toml` kept in a
    /// shared folder) and that a command replaced with a file of its own is
    /// made again as the link, to where it led; what it leads to is not
    /// written.
    #[test]
    fn a_link_a_command_replaced_is_made_again() {
        let scratch = workspace("put_back_link_again");
        let ws = scratch.join("ws");
        let shared = scratch.file("shared/config.toml", "[build]\njobs = 4\n");
        fs::create_dir_all(ws.join(".cargo")).unwrap();
        symlink(&shared, ws.join(".cargo/config.toml")).unwrap();
        let (_, put) = put_back_after(&ws, |ws| {
            fs::remove_file(ws.join(".cargo/config.toml")).unwrap();
            fs::write(
                ws.join(".cargo/config.toml"),
                "[build]\nrustc-wrapper = \"/tmp/evil\"\n",
            )
            .unwrap();
        });
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        assert_eq!(
            fs::read_link(ws.join(".cargo/config.toml")).unwrap(),
            shared
        );
        assert_eq!(fs::read_to_string(&shared).unwrap(), "[build]\njobs = 4\n");
    }

    /// A directory swapped for a link to a folder outside just before its
    /// mode is put back: the mode is not set where the link leads.
    #[test]
    fn a_directory_swapped_for_a_link_while_its_mode_is_put_back_sets_nothing_where_it_leads() {
        let scratch = workspace("put_back_race_mode");
        let ws = scratch.join("ws");
        let outside = scratch.dir("outside");
        set_mode(&outside, 0o751);
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        set_mode(&ws.join(".git/hooks"), 0o700);
        let (changed, _) = watch.changes_since(&before);
        assert_eq!(changed.len(), 1, "{changed:?}");
        let hold = cineris_testkit::Hold::new(PUTTING_BACK_A_MODE);
        let kept = ws.parent().unwrap().join("kept");
        let putting = std::thread::spawn({
            let watch = watch.clone();
            move || watch.put_back(&before, &changed, &|_| true, Some(&kept))
        });
        hold.reach();
        fs::rename(ws.join(".git/hooks"), ws.join(".git/hooks-moved")).unwrap();
        symlink(&outside, ws.join(".git/hooks")).unwrap();
        hold.release();
        let put = putting.join().unwrap();
        assert_eq!(mode(&outside), 0o751, "a mode was set where the link leads");
        // A later round removes the link itself and makes the folder again.
        let hooks = fs::symlink_metadata(ws.join(".git/hooks")).unwrap();
        assert!(hooks.is_dir() || !put.left.is_empty(), "{hooks:?}");
    }

    /// The file itself is swapped for a link to a file outside after its
    /// directory was opened: the link is replaced, and what it led to is
    /// not written.
    #[test]
    fn a_file_swapped_for_a_link_while_it_is_put_back_is_replaced() {
        let scratch = workspace("put_back_race_file");
        let ws = scratch.join("ws");
        let target = scratch.file("outside/target", "outside\n");
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        fs::write(ws.join(".git/config"), "[filter \"x\"]\n\tclean = evil\n").unwrap();
        let (changed, _) = watch.changes_since(&before);
        assert_eq!(changed.len(), 1, "{changed:?}");
        let hold = cineris_testkit::Hold::new(PUTTING_BACK);
        let kept = ws.parent().unwrap().join("kept");
        let putting = std::thread::spawn({
            let watch = watch.clone();
            move || watch.put_back(&before, &changed, &|_| true, Some(&kept))
        });
        hold.reach();
        fs::remove_file(ws.join(".git/config")).unwrap();
        symlink(&target, ws.join(".git/config")).unwrap();
        hold.release();
        let put = putting.join().unwrap();
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        assert!(
            fs::symlink_metadata(ws.join(".git/config"))
                .unwrap()
                .is_file()
        );
        assert_eq!(
            fs::read_to_string(ws.join(".git/config")).unwrap(),
            "[core]\n"
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "outside\n");
    }

    // ------------------------------------- the full review of the fix

    /// A workspace for the full review's cases: the folder `core.hooksPath`
    /// names (`scripts/hooks`), a submodule's configuration, and the
    /// watch of the session, the hook names of that folder among it.
    fn reviewed(test: &str) -> (Scratch, PathBuf, Watch) {
        let scratch = workspace(test);
        let ws = scratch.join("ws");
        scratch.file("ws/scripts/hooks/README", "hooks\n");
        scratch.file("ws/.git/modules/sub/config", "[core]\n");
        scratch.file("ws/.git/modules/sub/HEAD", "ref: refs/heads/main\n");
        scratch.file("ws/sub/.git", "gitdir: ../.git/modules/sub\n");
        let watch = unhurried_watch(&ws).with_more([ws.join("scripts/hooks/pre-commit")]);
        (scratch, ws, watch)
    }

    /// A folder of the command's own that no look can list (`chmod 000`)
    /// hides nothing else: the look is incomplete for that folder alone,
    /// and goes on, so a hook planted in the hooks folder and a filter in a
    /// submodule's configuration are found and put back; the folder, the
    /// command's, is opened and taken away with what is in it.
    #[test]
    fn a_folder_no_look_can_list_hides_nothing_else() {
        let (_scratch, ws, watch) = reviewed("hidden_after");
        let before = watch.take();
        assert!(before.is_complete());
        fs::create_dir(ws.join(".git/hooks/zz")).unwrap();
        fs::write(ws.join(".git/hooks/zz/x"), "x").unwrap();
        set_mode(&ws.join(".git/hooks/zz"), 0o000);
        fs::write(ws.join("scripts/hooks/pre-commit"), "#!/bin/sh\nevil\n").unwrap();
        fs::write(
            ws.join(".git/modules/sub/config"),
            "[core]\n\tfsmonitor = /tmp/evil\n",
        )
        .unwrap();
        let (changed, after) = watch.changes_since(&before);
        let unlistable = after.unlistable().to_vec();
        let kept = ws.parent().unwrap().join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        let _ = fs::set_permissions(ws.join(".git/hooks/zz"), fs::Permissions::from_mode(0o755));
        assert_eq!(unlistable, [ws.join(".git/hooks/zz")]);
        assert!(
            !ws.join("scripts/hooks/pre-commit").exists(),
            "{:?}",
            put.left
        );
        assert_eq!(
            fs::read_to_string(ws.join(".git/modules/sub/config")).unwrap(),
            "[core]\n"
        );
        assert!(!ws.join(".git/hooks/zz").exists(), "{:?}", put.left);
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        assert!(put.after.is_complete());
    }

    /// An empty folder no look can list, there before the command, does
    /// not blind the look: a workflow changed and one added are found.
    #[test]
    fn a_folder_no_look_can_list_does_not_blind_the_rest() {
        let (_scratch, ws, watch) = reviewed("hidden_blind");
        fs::create_dir_all(ws.join(".github/workflows")).unwrap();
        fs::write(ws.join(".github/workflows/ci.yml"), "on: push\n").unwrap();
        fs::create_dir(ws.join(".git/hooks/zz")).unwrap();
        set_mode(&ws.join(".git/hooks/zz"), 0o000);
        let before = watch.take();
        fs::write(ws.join(".github/workflows/ci.yml"), "on: push\njobs: {}\n").unwrap();
        fs::write(ws.join(".github/workflows/new.yml"), "on: push\n").unwrap();
        let (changed, _) = watch.changes_since(&before);
        set_mode(&ws.join(".git/hooks/zz"), 0o755);
        let paths: Vec<&PathBuf> = changed.iter().map(|one| &one.path).collect();
        assert!(
            paths.contains(&&ws.join(".github/workflows/ci.yml"))
                && paths.contains(&&ws.join(".github/workflows/new.yml")),
            "{paths:?}"
        );
    }

    /// A protected link that leads to a script in the workspace (a hook
    /// linked to `tools/pre-commit`): a change to the script, which is what
    /// git runs, is found and put back.
    #[test]
    fn a_hook_linked_to_a_workspace_script_is_watched_through_its_link() {
        let (scratch, ws, watch) = reviewed("linked_hook");
        scratch.file("ws/tools/pre-commit", "#!/bin/sh\ncargo fmt --check\n");
        symlink("../../tools/pre-commit", ws.join(".git/hooks/pre-commit")).unwrap();
        let before = watch.take();
        fs::write(ws.join("tools/pre-commit"), "#!/bin/sh\nevil\n").unwrap();
        let (changed, _) = watch.changes_since(&before);
        assert!(
            changed
                .iter()
                .any(|one| one.path == ws.join("tools/pre-commit")),
            "{changed:?}"
        );
        let kept = ws.parent().unwrap().join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        assert!(put.left.is_empty(), "left: {:?}", put.left);
        assert_eq!(
            fs::read_to_string(ws.join("tools/pre-commit")).unwrap(),
            "#!/bin/sh\ncargo fmt --check\n"
        );
        // The link taken away is the link's change: the file it led to,
        // which is there still, is not named gone.
        let before = watch.take();
        fs::remove_file(ws.join(".git/hooks/pre-commit")).unwrap();
        let (changed, _) = watch.changes_since(&before);
        let paths: Vec<&PathBuf> = changed.iter().map(|one| &one.path).collect();
        assert_eq!(paths, [&ws.join(".git/hooks/pre-commit")]);
    }

    /// A link a command makes at a protected path, to a file the workspace
    /// keeps, has the link taken away and never the file: a place only a
    /// later look follows a link to is not taken for made.
    #[test]
    fn a_link_a_command_makes_to_a_kept_file_takes_nothing_of_it() {
        let (scratch, ws, watch) = reviewed("link_made");
        scratch.file("ws/tools/build.sh", "#!/bin/sh\nmake\n");
        let before = watch.take();
        symlink("../../tools/build.sh", ws.join(".git/hooks/pre-push")).unwrap();
        let (changed, _) = watch.changes_since(&before);
        let kept = ws.parent().unwrap().join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        assert!(fs::symlink_metadata(ws.join(".git/hooks/pre-push")).is_err());
        assert_eq!(
            fs::read_to_string(ws.join("tools/build.sh")).unwrap(),
            "#!/bin/sh\nmake\n",
            "{:?}",
            put.changed
        );
    }

    /// A path a later look first watches (one the configuration named
    /// during a command) is not taken for made: what was there before the
    /// command is not named, and what changed since the look before began
    /// is named, with what it was unknown, and never touched.
    #[test]
    fn a_path_first_watched_after_the_command_is_never_taken_for_made() {
        let scratch = workspace("first_watched");
        let ws = scratch.join("ws");
        scratch.file("ws/.githooks/pre-commit", "#!/bin/sh\ncargo fmt --check\n");
        let before_watch = unhurried_watch(&ws);
        let mut before = before_watch.take();
        scratch.file("ws/.githooks/pre-push", "#!/bin/sh\nevil\n");
        let after_watch = unhurried_watch(&ws).with_more([
            ws.join(".githooks/pre-commit"),
            ws.join(".githooks/pre-push"),
        ]);
        let newly = |changed: &[Changed]| -> Vec<PathBuf> {
            changed
                .iter()
                .filter(|one| one.path.starts_with(ws.join(".githooks")))
                .map(|one| one.path.clone())
                .collect()
        };
        // A look before taken after both were written (its start set an
        // hour on, since a test cannot age a file's change time): neither is
        // named, as a file there before the command is not.
        before.taken.0 += 3600;
        let (changed, _) = after_watch.changes_since(&before);
        assert!(newly(&changed).is_empty(), "{changed:?}");
        // One taken before both: both are named, with what they were
        // unknown, and the put-back touches neither.
        before.taken = (0, 0);
        let (changed, _) = after_watch.changes_since(&before);
        assert_eq!(newly(&changed).len(), 2, "{changed:?}");
        assert!(
            changed
                .iter()
                .filter(|one| one.path.starts_with(ws.join(".githooks")))
                .all(|one| one.before == Before::Unknown)
        );
        let kept = ws.parent().unwrap().join("kept");
        let put = after_watch.put_back(&before, &changed, &|_| true, Some(&kept));
        assert!(ws.join(".githooks/pre-commit").exists());
        assert!(
            ws.join(".githooks/pre-push").exists(),
            "a file first watched was touched"
        );
        assert!(
            put.left
                .iter()
                .any(|one| one.path == ws.join(".githooks/pre-push"))
        );
    }

    /// What a file held when the put-back writes over it is kept, as it
    /// was then, in the folder it is given; a file too large to keep is
    /// not written over, and is left to put back by hand.
    #[test]
    fn what_a_file_held_is_kept_and_one_too_large_is_left() {
        let scratch = workspace("kept_copies");
        let ws = scratch.join("ws");
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        fs::write(ws.join(".git/config"), "[core]\n\tthe user's = edit\n").unwrap();
        fs::write(ws.join(".git/hooks/pre-push"), "#!/bin/sh\nmade\n").unwrap();
        let large = vec![b'x'; usize::try_from(KEEP_BYTES).unwrap() + 1];
        fs::write(ws.join(".git/hooks/pre-commit.sample"), &large).unwrap();
        let (changed, _) = watch.changes_since(&before);
        let kept = scratch.join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        assert_eq!(
            fs::read_to_string(ws.join(".git/config")).unwrap(),
            "[core]\n"
        );
        let (path, copy) = put
            .kept
            .iter()
            .find(|(path, _)| *path == ws.join(".git/config"))
            .expect("a kept copy of the configuration");
        assert_eq!(path, &ws.join(".git/config"));
        assert_eq!(
            fs::read_to_string(copy).unwrap(),
            "[core]\n\tthe user's = edit\n"
        );
        assert_eq!(mode(copy), 0o600);
        // A file made is kept before it is removed.
        assert!(!ws.join(".git/hooks/pre-push").exists());
        let (_, copy) = put
            .kept
            .iter()
            .find(|(path, _)| *path == ws.join(".git/hooks/pre-push"))
            .expect("a kept copy of the hook made");
        assert_eq!(fs::read_to_string(copy).unwrap(), "#!/bin/sh\nmade\n");
        // Too large to keep: left as it is, and named.
        assert_eq!(
            fs::read(ws.join(".git/hooks/pre-commit.sample"))
                .unwrap()
                .len(),
            large.len()
        );
        assert!(
            put.left
                .iter()
                .any(|one| one.path == ws.join(".git/hooks/pre-commit.sample")),
            "{:?}",
            put.left
        );
        // With nowhere to keep it, nothing that would be lost is touched.
        let before = watch.take();
        fs::write(ws.join(".git/config"), "[core]\n\tagain = 1\n").unwrap();
        let (changed, _) = watch.changes_since(&before);
        let put = watch.put_back(&before, &changed, &|_| true, None);
        assert_eq!(
            fs::read_to_string(ws.join(".git/config")).unwrap(),
            "[core]\n\tagain = 1\n"
        );
        assert!(!put.left.is_empty());
    }

    /// A folder that could not be listed before the command, which the
    /// command opened and planted in, is put back with a mode its owner can
    /// list: never one that hides from every later look what is in it.
    #[test]
    fn a_folder_is_never_given_back_a_mode_no_look_can_list() {
        let scratch = workspace("never_unlistable");
        let ws = scratch.join("ws");
        let zz = ws.join(".git/hooks/zz");
        fs::create_dir(&zz).unwrap();
        set_mode(&zz, 0o000);
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        set_mode(&zz, 0o755);
        fs::write(zz.join("pre-commit"), "#!/bin/sh\nevil\n").unwrap();
        let (changed, _) = watch.changes_since(&before);
        let kept = scratch.join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        let now = mode(&zz);
        set_mode(&zz, 0o755);
        assert_eq!(now, 0o500, "{:?}", put.left);
        assert!(put.after.unlistable().is_empty());
        // What the look before could not see is not taken for made: what
        // changed since it began is named, to put back by hand.
        assert!(
            put.left.iter().any(|one| one.path == zz.join("pre-commit")),
            "{:?}",
            put.left
        );
    }

    /// A protected file swapped for a FIFO between the look's finding it
    /// and its reading it does not hold the look: it is opened with no link
    /// followed and without waiting. `mkfifo` is a process: hence the
    /// `sys_` name.
    #[test]
    fn sys_a_file_swapped_for_a_fifo_while_it_is_read_does_not_hold_the_look() {
        let scratch = workspace("fifo_swap");
        let ws = scratch.join("ws");
        let watch = unhurried_watch(&ws);
        let hold = cineris_testkit::Hold::new(READING_A_PROTECTED_FILE);
        let looking = std::thread::spawn({
            let watch = watch.clone();
            move || watch.take()
        });
        hold.reach();
        // Whichever file the look is at, each is a FIFO now.
        for name in [".git/config", ".git/hooks/pre-commit.sample"] {
            let path = ws.join(name);
            fs::remove_file(&path).unwrap();
            let made = std::process::Command::new("mkfifo")
                .arg(&path)
                .status()
                .expect("run mkfifo");
            assert!(made.success(), "mkfifo failed");
        }
        hold.release();
        let snapshot = looking.join().unwrap();
        assert!(!snapshot.is_empty());
    }

    // --------------------------------- the narrow check of the full review

    /// A hook planted with no read right for its owner (a program runs
    /// without one) is given the right through a descriptor that needs
    /// none, kept, and removed.
    #[test]
    fn a_hook_its_owner_cannot_read_is_kept_and_removed() {
        let scratch = workspace("unreadable_hook");
        let ws = scratch.join("ws");
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        let hook = ws.join(".git/hooks/pre-commit");
        fs::write(&hook, b"\x7fELF...").unwrap();
        set_mode(&hook, 0o111);
        let (changed, _) = watch.changes_since(&before);
        let kept = scratch.join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        let there = fs::symlink_metadata(&hook).is_ok();
        if there {
            set_mode(&hook, 0o644);
        }
        assert!(!there, "left: {:?}", put.left);
        let (_, copy) = put
            .kept
            .iter()
            .find(|(path, _)| *path == hook)
            .expect("a kept copy of the hook");
        assert_eq!(fs::read(copy).unwrap(), b"\x7fELF...");
    }

    /// Two put-backs in one session keep their copies in one folder: the
    /// second copy of a file of the same name takes the next free name, and
    /// the second change is put back too.
    #[test]
    fn a_second_put_back_of_the_same_file_takes_the_next_name() {
        let scratch = workspace("second_copy");
        let ws = scratch.join("ws");
        let watch = unhurried_watch(&ws);
        let kept = scratch.join("kept");
        for (round, text) in [
            "[core]\n\tbare = false\n",
            "[filter \"x\"]\n\tclean = evil\n",
        ]
        .into_iter()
        .enumerate()
        {
            let before = watch.take();
            fs::write(ws.join(".git/config"), text).unwrap();
            let (changed, _) = watch.changes_since(&before);
            let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
            assert!(put.left.is_empty(), "round {round}: {:?}", put.left);
            assert_eq!(
                fs::read_to_string(ws.join(".git/config")).unwrap(),
                "[core]\n"
            );
            let (_, copy) = &put.kept[0];
            assert_eq!(fs::read_to_string(copy).unwrap(), text);
        }
        assert_eq!(
            names(&kept),
            ["001-config", "002-config"].map(String::from),
            "{:?}",
            names(&kept)
        );
    }

    /// A file whose name is near the system's bound is kept under a name
    /// that fits: the count before it does not push it past.
    #[test]
    fn a_file_with_a_long_name_is_kept_under_a_name_that_fits() {
        let scratch = workspace("long_name");
        let ws = scratch.join("ws");
        let long = format!("{}.yml", "w".repeat(250));
        scratch.file(format!("ws/.github/workflows/{long}"), "on: push\n");
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        fs::write(ws.join(".github/workflows").join(&long), "on: [push]\n").unwrap();
        let (changed, _) = watch.changes_since(&before);
        let kept = scratch.join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        assert!(put.left.is_empty(), "{:?}", put.left);
        let (_, copy) = &put.kept[0];
        let name = copy.file_name().unwrap().len();
        assert!(name <= 255, "{name}");
        assert_eq!(fs::read_to_string(copy).unwrap(), "on: [push]\n");
    }

    /// A folder the look before listed, which is no protected path itself
    /// (`.git/modules`, where submodules are found), closed to every look
    /// by the command after it planted a filter in a submodule's
    /// configuration: the put-back gives it its owner's right to list it
    /// back, looks again, and puts the configuration back.
    #[test]
    fn a_folder_the_look_before_listed_and_a_command_closed_is_opened_and_looked_in() {
        let scratch = workspace("closed_modules");
        let ws = scratch.join("ws");
        scratch.file("ws/.git/modules/sub/config", "[core]\n");
        scratch.file("ws/.git/modules/sub/HEAD", "ref: refs/heads/main\n");
        scratch.dir("ws/.git/modules/sub/hooks");
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        assert!(before.is_complete());
        fs::write(
            ws.join(".git/modules/sub/config"),
            "[core]\n\tfsmonitor = /tmp/evil\n",
        )
        .unwrap();
        set_mode(&ws.join(".git/modules"), 0o311);
        let (changed, after) = watch.changes_since(&before);
        let closed = after.unlistable().to_vec();
        let kept = scratch.join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        let now = mode(&ws.join(".git/modules"));
        set_mode(&ws.join(".git/modules"), 0o755);
        assert_eq!(closed, [ws.join(".git/modules")]);
        assert_eq!(put.reopened, [ws.join(".git/modules")]);
        assert_eq!(now, 0o711);
        assert_eq!(
            fs::read_to_string(ws.join(".git/modules/sub/config")).unwrap(),
            "[core]\n",
            "{:?}",
            put.left
        );
        assert!(put.after.unlistable().is_empty());
    }

    /// A watched name reached through a link the command made (`ln -s
    /// tools .githooks`, where `core.hooksPath` names `.githooks`): the
    /// link itself is recorded, taken for made and removed, and what it led
    /// to is left alone.
    #[test]
    fn a_link_made_on_the_way_to_a_watched_name_is_taken_away() {
        let scratch = workspace("link_on_the_way");
        let ws = scratch.join("ws");
        scratch.file("ws/tools/pre-commit", "#!/bin/sh\nevil\n");
        let watch = unhurried_watch(&ws).with_more([ws.join(".githooks/pre-commit")]);
        let before = watch.take();
        symlink("tools", ws.join(".githooks")).unwrap();
        let (changed, _) = watch.changes_since(&before);
        assert!(
            changed
                .iter()
                .any(|one| one.path == ws.join(".githooks") && one.change == Change::Created),
            "{changed:?}"
        );
        let kept = scratch.join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        assert!(
            fs::symlink_metadata(ws.join(".githooks")).is_err(),
            "{:?}",
            put.left
        );
        assert_eq!(
            fs::read_to_string(ws.join("tools/pre-commit")).unwrap(),
            "#!/bin/sh\nevil\n"
        );
    }

    /// A hook behind more closed folders than the put-back's rounds: a
    /// command plants a hook in a repository five folders down, then closes
    /// each folder to every look, deepest first (`chmod 311`: searchable, so
    /// git still finds the hook). Every folder under one opened again that
    /// the look before listed is opened too, all the way down, before the
    /// next look, so the hook is found and taken away (found by the last
    /// review of the third security fix, where three were opened and the
    /// hook never named).
    #[test]
    fn a_hook_behind_five_closed_folders_is_found_and_taken_away() {
        let scratch = workspace("five_closed");
        let ws = scratch.join("ws");
        let chain = ["a", "a/b", "a/b/c", "a/b/c/d", "a/b/c/d/e"];
        scratch.file("ws/a/b/c/d/e/sub/.git/config", "[core]\n");
        scratch.dir("ws/a/b/c/d/e/sub/.git/hooks");
        let watch = unhurried_watch(&ws);
        let before = watch.take();
        assert!(before.is_complete());
        let hook = ws.join("a/b/c/d/e/sub/.git/hooks/pre-commit");
        fs::write(&hook, "#!/bin/sh\nevil\n").unwrap();
        set_mode(&hook, 0o755);
        for folder in chain.iter().rev() {
            set_mode(&ws.join(folder), 0o311);
        }
        let (changed, _) = watch.changes_since(&before);
        let kept = scratch.join("kept");
        let put = watch.put_back(&before, &changed, &|_| true, Some(&kept));
        let modes: Vec<u32> = chain.iter().map(|folder| mode(&ws.join(folder))).collect();
        for folder in chain {
            set_mode(&ws.join(folder), 0o755);
        }
        assert!(
            !hook.exists(),
            "reopened {:?}, left {:?}",
            put.reopened,
            put.left
        );
        assert_eq!(put.reopened.len(), chain.len(), "{:?}", put.reopened);
        assert_eq!(modes, [0o711; 5]);
        assert!(put.after.unlistable().is_empty());
    }
}
