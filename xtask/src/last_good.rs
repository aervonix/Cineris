//! A copy of `xtask` that still runs when `xtask` does not build.
//!
//! In the middle of a change to `xtask` itself, `cargo xtask replace` cannot
//! run: cargo builds `xtask` first, the build fails, and the one command that
//! would edit the code that does not build is the one that cannot be used.
//! So every command that ends well keeps the binary that ran it in
//! `target/xtask-last-good/`, beside a stamp that says when it was built,
//! from which commit, and whether `xtask/` held changes that were not
//! committed.
//!
//! - **`./x replace …` and `./x links`** build `xtask` first, and when that
//!   fails run the copy instead, saying so. Anywhere else the copy is run by
//!   its path: `target/xtask-last-good/xtask` (`xtask.exe` on Windows).
//! - **The copy says it is old** each time it runs: when it was built, from
//!   which commit, how long ago, and how many files of `xtask/` are newer
//!   than it, so nobody trusts an old `replace` without knowing.
//! - **It offers only `replace` and `links`.** Every other command is a check
//!   or a run whose answer depends on the code as it is now: the copy would
//!   answer for the code as it was.
//! - **The copy is never overwritten in place.** A new copy is written beside
//!   it and renamed over it; where the system will not let a running program
//!   be replaced (Windows), the old one is moved aside first, which it does
//!   allow. A copy that is running does not refresh itself.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::Result;
use crate::workspace;

/// The folder under `target/` that holds the copy and its stamp.
pub(crate) const FOLDER: &str = "xtask-last-good";

/// The commands the copy offers.
pub(crate) const OFFERED: [&str; 2] = ["replace", "links"];

/// Where the copy is kept, for the repository at `root`.
pub(crate) fn copy_dir(root: &Path) -> PathBuf {
    root.join("target").join(FOLDER)
}

/// The copy's file name: `xtask`, `xtask.exe` on Windows.
fn copy_name() -> String {
    format!("xtask{}", std::env::consts::EXE_SUFFIX)
}

/// The stamp's file name, beside the copy.
const STAMP: &str = "xtask.stamp";

/// What is known of the binary the copy was made from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stamp {
    /// The binary's size in bytes, and its modification time in
    /// nanoseconds since the epoch: what tells a new build from the one
    /// already copied, and when the copy was built.
    pub(crate) length: u64,
    pub(crate) built_nanos: u128,
    /// When the copy was made, in seconds since the epoch.
    pub(crate) copied: u64,
    /// The commit the tree was at, and whether `xtask/` had changes that
    /// were not committed, as one line of text.
    pub(crate) commit: String,
}

impl Stamp {
    /// The stamp as the file holds it.
    pub(crate) fn text(&self) -> String {
        format!(
            "binary: {} {}\ncopied: {}\ncommit: {}\n",
            self.length, self.built_nanos, self.copied, self.commit
        )
    }

    /// Reads a stamp. `None` for text that is not one.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut binary = None;
        let mut copied = None;
        let mut commit = None;
        for line in text.lines() {
            let (key, value) = line.split_once(": ")?;
            match key {
                "binary" => {
                    let (length, nanos) = value.split_once(' ')?;
                    binary = Some((length.parse().ok()?, nanos.parse().ok()?));
                }
                "copied" => copied = value.parse().ok(),
                "commit" => commit = Some(value.to_owned()),
                _ => {}
            }
        }
        let (length, built_nanos) = binary?;
        Some(Self {
            length,
            built_nanos,
            copied: copied?,
            commit: commit?,
        })
    }
}

/// A file's size and modification time in nanoseconds since the epoch.
fn fingerprint(path: &Path) -> Result<(u64, u128)> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not look at {}: {error}", path.display()))?;
    let modified = metadata
        .modified()
        .map_err(|error| format!("no modification time for {}: {error}", path.display()))?;
    Ok((metadata.len(), nanos(modified)))
}

fn nanos(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos()
}

/// Whether `exe` is the copy: it lives in the copy's folder.
pub(crate) fn is_the_copy(exe: &Path) -> bool {
    exe.parent()
        .and_then(Path::file_name)
        .is_some_and(|folder| folder == FOLDER)
}

/// Keeps `exe`, which has just run a command to its end, as the last good
/// copy, unless it is the one already kept. Returns whether it was copied.
///
/// `now` is the time in seconds since the epoch, and `commit` what to say of
/// the tree the binary was built from.
pub(crate) fn refresh(root: &Path, exe: &Path, now: u64, commit: &str) -> Result<bool> {
    let dir = copy_dir(root);
    let stamp_path = dir.join(STAMP);
    let (length, built_nanos) = fingerprint(exe)?;
    let kept = fs::read_to_string(&stamp_path)
        .ok()
        .and_then(|text| Stamp::parse(&text));
    let present = dir.join(copy_name()).is_file();
    if present
        && kept
            .as_ref()
            .is_some_and(|kept| kept.length == length && kept.built_nanos == built_nanos)
    {
        return Ok(false);
    }
    fs::create_dir_all(&dir)
        .map_err(|error| format!("could not make {}: {error}", dir.display()))?;
    let pid = std::process::id();
    let new = dir.join(format!("{}.new-{pid}", copy_name()));
    fs::copy(exe, &new).map_err(|error| {
        format!(
            "could not copy {} to {}: {error}",
            exe.display(),
            new.display()
        )
    })?;
    install(&new, &dir.join(copy_name()), pid)?;
    let stamp = Stamp {
        length,
        built_nanos,
        copied: now,
        commit: commit.to_owned(),
    };
    let new_stamp = dir.join(format!("{STAMP}.new-{pid}"));
    fs::write(&new_stamp, stamp.text())
        .and_then(|()| fs::rename(&new_stamp, &stamp_path))
        .map_err(|error| format!("could not write {}: {error}", stamp_path.display()))?;
    forget_moved_aside(&dir);
    Ok(true)
}

/// Puts `new` in place of `target`. Where `target` cannot be replaced
/// because a process is running it, it is moved aside and `new` put in its
/// place: Windows refuses to overwrite or delete a running program and
/// allows it to be renamed.
fn install(new: &Path, target: &Path, pid: u32) -> Result<()> {
    if fs::rename(new, target).is_ok() {
        return Ok(());
    }
    let aside = target.with_file_name(format!(
        "{}.old-{pid}",
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("xtask")
    ));
    if target.exists() {
        fs::rename(target, &aside).map_err(|error| {
            format!(
                "could not move {} aside to {}: {error}",
                target.display(),
                aside.display()
            )
        })?;
    }
    fs::rename(new, target).map_err(|error| {
        format!(
            "could not put {} in place of {}: {error}",
            new.display(),
            target.display()
        )
    })
}

/// Removes the copies that were moved aside, and any a run that ended half
/// way left. One that is still running cannot be removed, and stays for the
/// next time.
fn forget_moved_aside(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.contains(".old-") || name.contains(".new-") {
            // Another process may be writing a `.new-` file at this moment:
            // an error here is its to deal with, and ours to ignore.
            drop(fs::remove_file(entry.path()));
        }
    }
}

/// What to say of the tree the binary was built from: the short commit, and
/// a note when `xtask/` had changes that were not committed. Best effort:
/// where git does not answer, `unknown`.
pub(crate) fn describe_tree(root: &Path) -> String {
    let git = |arguments: &[&str]| -> Option<String> {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(root)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".to_owned());
    let changed = git(&[
        "status",
        "--porcelain",
        "--untracked-files=no",
        "--",
        "xtask",
    ])
    .is_some_and(|text| !text.is_empty());
    if changed {
        format!("{commit}, with changes in xtask/ not committed")
    } else {
        commit
    }
}

/// The files of `xtask/` (and its manifest) changed since `built_nanos`, by
/// their paths from `root`: what the copy does not know.
pub(crate) fn newer_than(root: &Path, built_nanos: u128) -> Vec<String> {
    let mut found = Vec::new();
    let manifest = root.join("xtask").join("Cargo.toml");
    if modified_after(&manifest, built_nanos) {
        found.push("xtask/Cargo.toml".to_owned());
    }
    walk(
        root,
        &root.join("xtask").join("src"),
        built_nanos,
        &mut found,
    );
    found.sort();
    found
}

fn modified_after(path: &Path, built_nanos: u128) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| nanos(modified) > built_nanos)
}

fn walk(root: &Path, dir: &Path, built_nanos: u128, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, built_nanos, found);
        } else if modified_after(&path, built_nanos) {
            let shown = path.strip_prefix(root).unwrap_or(&path);
            found.push(
                shown
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
}

/// The warning the copy prints each time it runs.
pub(crate) fn warning(stamp: &Stamp, now: u64, newer: &[String]) -> String {
    let built = stamp.built_nanos / 1_000_000_000;
    let built = u64::try_from(built).unwrap_or(0);
    let mut text = format!(
        "xtask: this is the last good copy of xtask, built on {} ({}), from commit {}.\n\
         xtask: it is not what the source in xtask/ says now",
        utc(built),
        age(now.saturating_sub(built)),
        stamp.commit
    );
    if newer.is_empty() {
        text.push_str(
            ": nothing under xtask/ is newer than it, but the build may be broken for \
             another reason.",
        );
    } else {
        text.push_str(&format!(
            ": {} file{} under xtask/ {} changed since: {}.",
            newer.len(),
            if newer.len() == 1 { "" } else { "s" },
            if newer.len() == 1 { "has" } else { "have" },
            newer.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
                + if newer.len() > 5 { ", and more" } else { "" }
        ));
    }
    text.push_str(
        "\nxtask: it offers only `replace` and `links`. Make xtask build again \
         (`cargo build -p xtask`) for the rest.",
    );
    text
}

/// What a command said to the copy, if it is not one the copy offers.
pub(crate) fn refusal(command: Option<&str>) -> Option<String> {
    match command {
        None | Some("help" | "--help" | "-h") => None,
        Some(command) if OFFERED.contains(&command) => None,
        Some(command) => Some(format!(
            "`{command}` is not offered by the last good copy of xtask, which offers {}: its \
             answer would be about the code as it was, not as it is. Make xtask build again \
             with `cargo build -p xtask`.",
            OFFERED.map(|name| format!("`{name}`")).join(" and ")
        )),
    }
}

/// `1_700_000_000` as `2023-11-14 22:13:20 UTC`.
pub(crate) fn utc(seconds: u64) -> String {
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rest = seconds % 86_400;
    // Days since 1970-01-01 to a calendar date (the civil-from-days method).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3_600,
        rest % 3_600 / 60,
        rest % 60
    )
}

/// A span of seconds as one coarse unit: `40 s ago`, `12 min ago`.
pub(crate) fn age(seconds: u64) -> String {
    match seconds {
        0..=89 => format!("{seconds} s ago"),
        90..=5_399 => format!("{} min ago", (seconds + 30) / 60),
        5_400..=129_599 => format!("{} h ago", (seconds + 1_800) / 3_600),
        _ => format!("{} days ago", (seconds + 43_200) / 86_400),
    }
}

/// The time now, in seconds since the epoch.
pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Called at the start of a run of the copy: says that it is one, and
/// refuses a command it does not offer.
pub(crate) fn announce(root: &Path, exe: &Path, command: Option<&str>) -> Result<()> {
    let stamp = fs::read_to_string(exe.with_file_name(STAMP))
        .ok()
        .and_then(|text| Stamp::parse(&text));
    let mut stderr = std::io::stderr().lock();
    match stamp {
        Some(stamp) => {
            let newer = newer_than(root, stamp.built_nanos);
            drop(writeln!(stderr, "{}", warning(&stamp, now(), &newer)));
        }
        None => {
            drop(writeln!(
                stderr,
                "xtask: this is a copy of xtask kept from an earlier build, and its stamp is \
                 missing: nothing says how old it is."
            ));
        }
    }
    drop(stderr);
    match refusal(command) {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

/// Called when a command has ended well: keeps this binary as the last good
/// copy. A failure to do so is a note, not a failure of the command.
pub(crate) fn keep_after_success() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // A runner's build is never used again, and its copy would only be
    // cached.
    if is_the_copy(&exe) || std::env::var_os("GITHUB_ACTIONS").is_some() {
        return;
    }
    let root = workspace::root();
    if let Err(message) = refresh(&root, &exe, now(), &describe_tree(&root)) {
        eprintln!("xtask: note: the last good copy of xtask was not refreshed: {message}");
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use super::{
        FOLDER, Stamp, age, copy_dir, copy_name, is_the_copy, newer_than, refresh, refusal, utc,
        warning,
    };

    /// A fresh folder under the system's temporary directory.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "xtask-last-good-{name}-{}-{}",
            std::process::id(),
            super::now()
        ));
        drop(fs::remove_dir_all(&dir));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A binary of `content`, modified `seconds` after the epoch.
    fn binary(dir: &Path, content: &str, seconds: u64) -> PathBuf {
        let path = dir.join("built-xtask");
        fs::write(&path, content).unwrap();
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
            .unwrap();
        path
    }

    #[test]
    fn a_stamp_reads_back_as_it_was_written() {
        let stamp = Stamp {
            length: 123_456,
            built_nanos: 1_700_000_000_123_456_789,
            copied: 1_700_000_100,
            commit: "abc1234, with changes in xtask/ not committed".to_owned(),
        };
        assert_eq!(Stamp::parse(&stamp.text()), Some(stamp));
        assert_eq!(Stamp::parse("not a stamp"), None);
        assert_eq!(Stamp::parse("copied: 1\ncommit: x\n"), None);
    }

    #[test]
    fn times_are_written_as_utc_and_ages_in_one_unit() {
        assert_eq!(utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc(1_700_000_000), "2023-11-14 22:13:20 UTC");
        // A leap day, and the end of a year.
        assert_eq!(utc(1_709_164_800), "2024-02-29 00:00:00 UTC");
        assert_eq!(utc(1_735_689_599), "2024-12-31 23:59:59 UTC");
        assert_eq!(age(40), "40 s ago");
        assert_eq!(age(720), "12 min ago");
        assert_eq!(age(3 * 3_600), "3 h ago");
        assert_eq!(age(3 * 86_400), "3 days ago");
    }

    #[test]
    fn a_good_run_keeps_its_binary_and_the_same_build_is_not_copied_twice() {
        let root = scratch("refresh");
        let exe = binary(&root, "build one", 1_000);
        assert!(refresh(&root, &exe, 5_000, "abc1234").unwrap());
        let dir = copy_dir(&root);
        let kept = dir.join(copy_name());
        assert_eq!(fs::read_to_string(&kept).unwrap(), "build one");
        let stamp = Stamp::parse(&fs::read_to_string(dir.join("xtask.stamp")).unwrap()).unwrap();
        assert_eq!(stamp.commit, "abc1234");
        assert_eq!(stamp.copied, 5_000);
        assert_eq!(stamp.built_nanos, 1_000 * 1_000_000_000);

        // The same build again: nothing is copied, the stamp stays.
        assert!(!refresh(&root, &exe, 9_000, "def5678").unwrap());
        let again = Stamp::parse(&fs::read_to_string(dir.join("xtask.stamp")).unwrap()).unwrap();
        assert_eq!(again, stamp);

        // What an earlier run left beside the copy (a program moved aside,
        // a file half written) is cleared by the next one.
        fs::write(dir.join(format!("{}.old-77", copy_name())), "aside").unwrap();
        fs::write(dir.join("xtask.stamp.new-77"), "half").unwrap();

        // A build of the same size and another time is another build.
        let exe = binary(&root, "build ONE", 1_500);
        assert!(refresh(&root, &exe, 7_000, "abc1234").unwrap());
        assert_eq!(fs::read_to_string(&kept).unwrap(), "build ONE");

        // A new build replaces the copy and the stamp, and leaves no
        // half-written file beside them.
        let exe = binary(&root, "build two, longer", 2_000);
        assert!(refresh(&root, &exe, 9_000, "def5678").unwrap());
        assert_eq!(fs::read_to_string(&kept).unwrap(), "build two, longer");
        let stamp = Stamp::parse(&fs::read_to_string(dir.join("xtask.stamp")).unwrap()).unwrap();
        assert_eq!((stamp.commit.as_str(), stamp.copied), ("def5678", 9_000));
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut expected = vec!["xtask.stamp".to_owned(), copy_name()];
        expected.sort();
        assert_eq!(names, expected);
        drop(fs::remove_dir_all(&root));
    }

    #[test]
    fn a_missing_copy_is_made_again_though_the_stamp_is_there() {
        let root = scratch("missing");
        let exe = binary(&root, "build", 1_000);
        assert!(refresh(&root, &exe, 5, "abc").unwrap());
        fs::remove_file(copy_dir(&root).join(copy_name())).unwrap();
        assert!(refresh(&root, &exe, 6, "abc").unwrap());
        assert!(copy_dir(&root).join(copy_name()).is_file());
        drop(fs::remove_dir_all(&root));
    }

    #[test]
    fn the_copy_is_known_by_its_folder() {
        let root = Path::new("repo");
        assert!(is_the_copy(&copy_dir(root).join(copy_name())));
        assert!(!is_the_copy(
            &root.join("target").join("debug").join("xtask")
        ));
        assert_eq!(FOLDER, "xtask-last-good");
    }

    #[test]
    fn the_copy_offers_replace_and_links_and_nothing_else() {
        assert_eq!(refusal(Some("replace")), None);
        assert_eq!(refusal(Some("links")), None);
        assert_eq!(refusal(None), None);
        assert_eq!(refusal(Some("--help")), None);
        for command in ["check", "ci", "test", "rules", "regen", "mutate", "gen"] {
            let message = refusal(Some(command)).unwrap();
            assert!(message.contains(&format!("`{command}`")), "{message}");
            assert!(message.contains("cargo build -p xtask"), "{message}");
        }
    }

    #[test]
    fn files_newer_than_the_build_are_found_and_older_ones_are_not() {
        let root = scratch("newer");
        let src = root.join("xtask").join("src").join("rules");
        fs::create_dir_all(&src).unwrap();
        let write = |path: PathBuf, seconds: u64| {
            fs::write(&path, "x").unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
                .unwrap();
        };
        write(root.join("xtask").join("Cargo.toml"), 100);
        write(root.join("xtask").join("src").join("old.rs"), 100);
        write(root.join("xtask").join("src").join("edited.rs"), 300);
        write(src.join("layout.rs"), 400);
        let found = newer_than(&root, 200 * 1_000_000_000);
        assert_eq!(found, ["xtask/src/edited.rs", "xtask/src/rules/layout.rs"]);
        assert!(newer_than(&root, 500 * 1_000_000_000).is_empty());
        drop(fs::remove_dir_all(&root));
    }

    #[test]
    fn the_warning_says_when_from_which_commit_and_what_changed_since() {
        let stamp = Stamp {
            length: 1,
            built_nanos: 1_700_000_000 * 1_000_000_000,
            copied: 1_700_000_001,
            commit: "abc1234".to_owned(),
        };
        let newer = vec![
            "xtask/src/schema.rs".to_owned(),
            "xtask/src/main.rs".to_owned(),
        ];
        let text = warning(&stamp, 1_700_000_000 + 720, &newer);
        for needle in [
            "2023-11-14 22:13:20 UTC",
            "12 min ago",
            "abc1234",
            "2 files under xtask/ have changed since: xtask/src/schema.rs, xtask/src/main.rs",
            "only `replace` and `links`",
        ] {
            assert!(text.contains(needle), "`{needle}` is not in:\n{text}");
        }
        let text = warning(&stamp, 1_700_000_000 + 5, &[]);
        assert!(text.contains("nothing under xtask/ is newer"), "{text}");
    }
}
