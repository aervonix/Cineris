//! Finding a program the harness starts by name: its shell, and git
//! (`docs/architecture/05-tools.md`, "Which shell").
//!
//! **Only the absolute directories of `PATH` are looked in.** An empty or
//! relative entry is common (`./node_modules/.bin`) and names a directory
//! of wherever the harness was started, which is usually the workspace. A
//! file there named `bash` or `git` would otherwise run in place of the
//! real one, as soon as a session opens and before anything is asked. A
//! program started by its bare name has the same flaw twice over: on Linux
//! the child changes to its working directory and then searches the
//! parent's `PATH`, so a relative entry is read from the workspace; and on
//! Windows the system's search also looks in the current directory. So the
//! harness finds the program itself, here, and starts it by the absolute
//! path found.
//!
//! The search is the harness's own on Windows too: a name is tried with
//! `.exe` and as it is given, never with the other extensions of
//! `PATHEXT` (a `git.bat` is not git).

#![allow(
    clippy::disallowed_methods,
    reason = "looking at the file system blocks; callers run it off the core thread"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The directories of this process's `PATH`, in order, as the system splits
/// them. The relative ones are among them: [`find`] passes over them.
#[must_use]
pub fn path_directories() -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default()
}

/// The first program called `name` in the absolute directories of
/// `directories`. With `windows`, by Windows's rules: `name.exe` first when
/// `name` has no extension, then `name` itself, and any file counts as a
/// program. A candidate that is not an absolute path (a name that is itself
/// a path with a drive and no root, `C:git.exe`) is never taken.
#[must_use]
pub fn find(directories: &[PathBuf], name: &str, windows: bool) -> Option<PathBuf> {
    let mut names = vec![OsString::from(name)];
    if windows && Path::new(name).extension().is_none() {
        names.insert(0, OsString::from(format!("{name}.exe")));
    }
    directories
        .iter()
        .filter(|directory| directory.is_absolute())
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_absolute() && is_program(candidate, windows))
}

/// The first `name` in the absolute directories of `directories` that
/// passes through none of `avoid` on its way ([`passes_through`]): the
/// workspace and a session's write roots, where a command could have
/// planted a program the harness starts (found by the third review of step
/// 26). Not only where the program leads is looked at: a link inside a root
/// that leads outside is passed over too, since a command can replace the
/// link after the harness chose it (found by the early review of the
/// harness's own processes' fixes). With no `avoid` roots this is
/// [`find`].
#[must_use]
#[allow(
    clippy::disallowed_methods,
    reason = "resolving a candidate's path is part of choosing it; callers run it off the core \
              thread"
)]
pub fn find_outside(
    directories: &[PathBuf],
    name: &str,
    windows: bool,
    avoid: &[PathBuf],
) -> Option<PathBuf> {
    let mut names = vec![OsString::from(name)];
    if windows && Path::new(name).extension().is_none() {
        names.insert(0, OsString::from(format!("{name}.exe")));
    }
    let avoid = resolved(avoid);
    directories
        .iter()
        .filter(|directory| directory.is_absolute())
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| {
            candidate.is_absolute()
                && is_program(candidate, windows)
                && !passes_through(candidate, &avoid)
        })
}

/// The extensions [`find_runnable_outside`] tries on Windows, in the order
/// it tries them in each directory: a program, then the two kinds of batch
/// script, which are started through `cmd.exe`
/// (`docs/steps/28/answered.md`, "The processes", "Windows").
pub const WINDOWS_RUNNABLE: [&str; 3] = ["exe", "cmd", "bat"];

/// The program an extension's `cineris.exec` or a hook command names by
/// `name`: the first in the absolute directories of `directories` that
/// passes through none of `avoid` ([`passes_through`]).
///
/// - With Linux's rules, `name` itself, as [`find_outside`] finds it.
/// - With Windows's, **each directory in turn** is looked in for
///   `name.exe`, then `name.cmd`, then `name.bat`, before the next
///   directory, as the system's own search goes: a `name.cmd` in an earlier
///   directory is taken before a `name.exe` in a later one. A `name` that
///   already has an extension is looked for as it is, and only when the
///   extension is one of those three ([`WINDOWS_RUNNABLE`], in any case).
///   Nothing else of `PATHEXT` (`.com`, `.ps1`, `.vbs`, `.js`) is ever
///   tried or taken, and the bare name is never taken on its own.
///
/// `None` when nothing is found, when `name` is not one plain file name,
/// or, with Windows's rules, when its extension is not one of the three.
/// [`find`] and [`find_outside`], which the harness's own shell and git
/// are found by, are not changed by this.
#[must_use]
pub fn find_runnable_outside(
    directories: &[PathBuf],
    name: &str,
    windows: bool,
    avoid: &[PathBuf],
) -> Option<PathBuf> {
    let plain = matches!(
        Path::new(name).components().collect::<Vec<_>>()[..],
        [std::path::Component::Normal(_)]
    );
    if !plain || name.contains(['/', '\\']) || (windows && name.contains(':')) {
        return None;
    }
    let names: Vec<OsString> = if !windows {
        vec![OsString::from(name)]
    } else {
        match Path::new(name).extension() {
            None => WINDOWS_RUNNABLE
                .iter()
                .map(|extension| OsString::from(format!("{name}.{extension}")))
                .collect(),
            Some(extension) if runnable_on_windows(extension) => vec![OsString::from(name)],
            Some(_) => return None,
        }
    };
    let avoid = resolved(avoid);
    directories
        .iter()
        .filter(|directory| directory.is_absolute())
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| {
            candidate.is_absolute()
                && is_program(candidate, windows)
                && !passes_through(candidate, &avoid)
        })
}

/// Whether `extension` is one [`find_runnable_outside`] takes on Windows,
/// in any case.
fn runnable_on_windows(extension: &std::ffi::OsStr) -> bool {
    extension.to_str().is_some_and(|extension| {
        WINDOWS_RUNNABLE
            .iter()
            .any(|runnable| runnable.eq_ignore_ascii_case(extension))
    })
}

/// Whether `path` is a batch script by its extension, `.cmd` or `.bat` in
/// any case: on Windows it is started through `cmd.exe`, not as a program.
#[must_use]
pub fn is_batch_script(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
}

/// Whether a program given by its absolute `path` may be started with
/// Windows's rules: its extension is `.exe`, `.cmd` or `.bat`, in any
/// case ([`WINDOWS_RUNNABLE`]). With Linux's rules, any path.
#[must_use]
pub fn runnable_by_extension(path: &Path, windows: bool) -> bool {
    !windows || path.extension().is_some_and(runnable_on_windows)
}

/// `roots` each at its real path. A place is compared with them by its
/// real path, and on Windows `canonicalize` returns a verbatim (`\\?\`)
/// path, which a plain root is never a prefix of: a root left unresolved
/// would never catch a candidate under it (found on Windows by the
/// independent review's test of step 26). A root that cannot be resolved
/// stays as it is.
#[must_use]
pub fn resolved(roots: &[PathBuf]) -> Vec<PathBuf> {
    roots
        .iter()
        .map(|root| std::fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect()
}

/// The most links followed on the way to a program before it is taken to
/// be inside a root: the system's own limit is about as many.
const MOST_LINKS: usize = 40;

/// Whether `path`, on its way to the file it names, passes through any of
/// `roots` ([`resolved`] ones): any of its directories, the place it is
/// found at, a link on the way (of a directory or of the file itself), or
/// what a link leads to. A program reached through a place inside a root
/// is one a command that can write the root can change after the harness
/// chose it, so it is not taken, wherever its last link leads. Each place
/// is compared by its directory's real path and its own name, so a short
/// name or another case on Windows names the same place. `true` too when
/// the way has more than [`MOST_LINKS`] links.
#[must_use]
pub fn passes_through(path: &Path, roots: &[PathBuf]) -> bool {
    let Some(places) = places_on_the_way(path) else {
        return true;
    };
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    places.iter().chain(std::iter::once(&real)).any(|place| {
        roots
            .iter()
            .any(|root| crate::path::location_within(place, root))
    })
}

/// A part of a path, owned, for following links one at a time.
enum Part {
    /// A prefix and a root: where an absolute path starts again.
    Root(PathBuf),
    /// `..`.
    Up,
    /// A name.
    Name(OsString),
}

fn parts(path: &Path) -> Vec<Part> {
    use std::path::Component;
    let mut parts = Vec::new();
    let mut root = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => root.push(prefix.as_os_str()),
            Component::RootDir => {
                root.push(component.as_os_str());
                parts.push(Part::Root(std::mem::take(&mut root)));
            }
            Component::CurDir => {}
            Component::ParentDir => parts.push(Part::Up),
            Component::Normal(name) => parts.push(Part::Name(name.to_owned())),
        }
    }
    if !root.as_os_str().is_empty() {
        // A drive with no root (`C:x`): taken as given.
        parts.insert(0, Part::Root(root));
    }
    parts
}

/// Every place `path` is reached through, in order, each link followed as
/// the system follows it: every directory, every link, and every place a
/// link leads to. Each is its directory's real path joined to its name.
/// `None` past [`MOST_LINKS`] links, or when a link cannot be read.
fn places_on_the_way(path: &Path) -> Option<Vec<PathBuf>> {
    let mut pending: std::collections::VecDeque<Part> = parts(path).into();
    let mut here = PathBuf::new();
    let mut places = Vec::new();
    let mut links = 0usize;
    while let Some(part) = pending.pop_front() {
        match part {
            Part::Root(root) => here = root,
            Part::Up => {
                here.pop();
            }
            Part::Name(name) => {
                let next = here.join(&name);
                let directory = std::fs::canonicalize(&here).unwrap_or_else(|_| here.clone());
                places.push(directory.join(&name));
                let is_link = crate::path::is_link(&next).unwrap_or(false);
                if is_link {
                    links += 1;
                    if links > MOST_LINKS {
                        return None;
                    }
                    let target = std::fs::read_link(&next).ok()?;
                    // A relative target goes on from the link's directory,
                    // which is where `here` stands.
                    for part in parts(&target).into_iter().rev() {
                        pending.push_front(part);
                    }
                } else {
                    here = next;
                }
            }
        }
    }
    Some(places)
}

/// Whether `path` is a file that can be run: a regular file, and on Linux
/// one with a permission to run it. On Windows any file counts.
#[must_use]
pub fn is_program(path: &Path, windows: bool) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    if !windows {
        use std::os::unix::fs::PermissionsExt;
        return metadata.permissions().mode() & 0o111 != 0;
    }
    let _ = windows;
    true
}

/// Where `git` is, for every git the harness starts itself: found once in
/// the life of the process, in the absolute directories of its `PATH`.
/// `None` when there is none, which a caller treats as git not being
/// installed.
#[must_use]
pub fn git() -> Option<&'static Path> {
    static GIT: OnceLock<Option<PathBuf>> = OnceLock::new();
    GIT.get_or_init(|| find(&path_directories(), "git", cfg!(windows)))
        .as_deref()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::{Rng, SystemRng};

    fn scratch(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cineris-base-program-{test}-{:016x}",
            SystemRng::new().next_u64()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn program(directory: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(directory).unwrap();
        let path = directory.join(name);
        std::fs::write(&path, "").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        path
    }

    /// `target` written relative to this process's current directory, as a
    /// relative entry of a user's `PATH` is. `None` on another drive.
    fn relative_to_here(target: &Path) -> Option<PathBuf> {
        use std::path::Component;
        let here = std::env::current_dir().unwrap();
        let anchor = |path: &Path| {
            path.components()
                .take_while(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
                .collect::<PathBuf>()
                .to_string_lossy()
                .to_ascii_lowercase()
        };
        if anchor(&here) != anchor(target) {
            return None;
        }
        let names = |path: &Path| {
            path.components()
                .filter(|part| matches!(part, Component::Normal(_)))
                .count()
        };
        let mut relative: PathBuf = (0..names(&here)).map(|_| "..").collect();
        relative.extend(
            target
                .components()
                .filter(|part| matches!(part, Component::Normal(_))),
        );
        Some(relative)
    }

    /// A relative entry that leads to a planted `git` is passed over, on
    /// either system's rules, and an absolute one after it is taken. Found
    /// by the independent review of the fixes to step 13a: the harness
    /// started `git` by its bare name, so the system searched the relative
    /// entry.
    #[test]
    fn a_relative_path_entry_does_not_make_a_file_there_the_program() {
        let dir = scratch("relative");
        let planted = dir.join("node_modules").join(".bin");
        let real = dir.join("real");
        for (windows, name) in [(false, "git"), (true, "git.exe")] {
            program(&planted, name);
            let wanted = program(&real, name);
            let Some(relative) = relative_to_here(&planted) else {
                return;
            };
            assert!(relative.join(name).is_file(), "{}", relative.display());
            let directories = [
                PathBuf::new(),
                PathBuf::from("."),
                relative.clone(),
                real.clone(),
            ];
            assert_eq!(find(&directories, "git", windows), Some(wanted), "{name}");
            assert_eq!(find(&[relative], "git", windows), None, "{name}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On Windows a name is tried with `.exe` and as given: never as a
    /// batch file or a script, wherever one is on the path.
    #[test]
    fn on_windows_only_an_exe_or_the_name_itself_is_taken() {
        let dir = scratch("pathext");
        let first = dir.join("first");
        for name in ["git.bat", "git.cmd", "git.com", "git.ps1"] {
            program(&first, name);
        }
        let exe = program(&dir.join("second"), "git.exe");
        assert_eq!(find(&[first, dir.join("second")], "git", true), Some(exe));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// By Windows's rules a program for `cineris.exec` is looked for in each
    /// directory in turn as `.exe`, `.cmd`, `.bat`: a `.cmd` where there is
    /// no `.exe` beside it, and before an `.exe` in a later directory; never
    /// a `.ps1`, `.com`, `.vbs` or `.js`, nor the bare name, wherever they
    /// are. Checked on every system, as the rules are a value here.
    #[test]
    fn by_windows_rules_a_runnable_is_looked_for_a_directory_at_a_time() {
        let dir = scratch("runnable");
        let first = dir.join("first");
        let second = dir.join("second");
        let shim = program(&first, "x.cmd");
        let later_exe = program(&second, "x.exe");
        let both = [first.clone(), second.clone()];
        assert_eq!(
            find_runnable_outside(&both, "x", true, &[]),
            Some(shim.clone())
        );
        // In one directory, `.exe` before `.cmd` before `.bat`.
        let bat = program(&first, "x.bat");
        let exe = program(&first, "x.exe");
        assert_eq!(
            find_runnable_outside(&both, "x", true, &[]),
            Some(exe.clone())
        );
        std::fs::remove_file(&exe).unwrap();
        assert_eq!(
            find_runnable_outside(&both, "x", true, &[]),
            Some(shim.clone())
        );
        std::fs::remove_file(&shim).unwrap();
        assert_eq!(
            find_runnable_outside(&both, "x", true, &[]),
            Some(bat.clone())
        );
        std::fs::remove_file(&bat).unwrap();
        assert_eq!(
            find_runnable_outside(&both, "x", true, &[]),
            Some(later_exe)
        );

        // Nothing else of `PATHEXT`, and not the bare name.
        let only = dir.join("only");
        for name in ["y", "y.ps1", "y.com", "y.vbs", "y.js", "y.cmd.txt"] {
            program(&only, name);
        }
        let only = [only];
        assert_eq!(find_runnable_outside(&only, "y", true, &[]), None);
        for named in ["y.ps1", "y.com", "y.vbs", "y.js", "y.cmd.txt"] {
            assert_eq!(
                find_runnable_outside(&only, named, true, &[]),
                None,
                "{named}"
            );
        }
        // A name with one of the three, in any case, is looked for as it is.
        let upper = program(&only[0], "z.CMD");
        assert_eq!(
            find_runnable_outside(&only, "z.CMD", true, &[]),
            Some(upper.clone())
        );
        assert_eq!(
            find_runnable_outside(&only, "z.cmd", true, &[]).is_some(),
            cfg!(windows)
        );
        // A path, or a drive with no root, is not a name.
        for not_a_name in ["a/y", "a\\y", "C:y", "..", "."] {
            assert_eq!(
                find_runnable_outside(&only, not_a_name, true, &[]),
                None,
                "{not_a_name}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The avoided roots and the relative directories of `PATH` are passed
    /// over by these rules as by [`find_outside`]'s; by Linux's rules the
    /// name is looked for as it is, and no `.cmd` is tried.
    #[test]
    fn a_runnable_inside_an_avoided_root_or_a_relative_directory_is_passed_over() {
        let dir = scratch("runnable-avoid");
        let ws = dir.join("ws");
        let planted = program(&ws.join("bin"), "x.cmd");
        let real = program(&dir.join("usr"), "x.cmd");
        let path = [ws.join("bin"), dir.join("usr")];
        assert_eq!(
            find_runnable_outside(&path, "x", true, std::slice::from_ref(&ws)),
            Some(real)
        );
        assert_eq!(find_runnable_outside(&path, "x", true, &[]), Some(planted));
        if let Some(relative) = relative_to_here(&ws.join("bin")) {
            assert_eq!(find_runnable_outside(&[relative], "x", true, &[]), None);
        }

        let linux = program(&dir.join("lin"), "tool");
        program(&dir.join("lin"), "tool.cmd");
        let lin = [dir.join("lin")];
        assert_eq!(find_runnable_outside(&lin, "tool", false, &[]), Some(linux));
        assert_eq!(
            find_runnable_outside(&lin, "tool.cmd", false, &[]),
            Some(dir.join("lin").join("tool.cmd"))
        );
        assert_eq!(
            find_runnable_outside(&lin, "tool", false, std::slice::from_ref(&dir)),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_batch_script_is_known_by_its_extension_in_any_case() {
        for (path, script, runnable) in [
            ("C:\\a\\x.cmd", true, true),
            ("C:\\a\\x.BAT", true, true),
            ("C:\\a\\x.Exe", false, true),
            ("C:\\a\\x.ps1", false, false),
            ("C:\\a\\x", false, false),
            ("C:\\a\\x.cmd.txt", false, false),
        ] {
            assert_eq!(is_batch_script(Path::new(path)), script, "{path}");
            assert_eq!(
                runnable_by_extension(Path::new(path), true),
                runnable,
                "{path}"
            );
            assert!(runnable_by_extension(Path::new(path), false), "{path}");
        }
    }

    #[test]
    fn git_is_found_where_the_system_has_it() {
        // Every system these tests run on has git, on an absolute entry.
        let git = git().expect("git on PATH");
        assert!(git.is_absolute(), "{}", git.display());
    }

    /// A program inside a write root (where a confined command could have
    /// planted it) is passed over, and the next one on the path is taken;
    /// when nothing is left outside the roots, there is none.
    #[test]
    fn find_outside_passes_over_a_program_inside_a_root() {
        let dir = scratch("outside");
        let root = dir.join("ws").join("bin");
        let clean = dir.join("usr").join("bin");
        for windows in [false, true] {
            let name = if windows { "git.exe" } else { "git" };
            let planted = program(&root, name);
            let real = program(&clean, name);
            let path = [root.clone(), clean.clone()];
            // With the root avoided, the planted one is passed over.
            assert_eq!(
                find_outside(&path, "git", windows, &[dir.join("ws")]),
                Some(real.clone()),
                "{name}"
            );
            // With nothing avoided, the first is taken: that is `find`.
            assert_eq!(
                find_outside(&path, "git", windows, &[]),
                Some(planted.clone())
            );
            assert_eq!(
                find(&path, "git", windows),
                find_outside(&path, "git", windows, &[])
            );
            // With only the planted one, and the root avoided, there is
            // none.
            assert_eq!(
                find_outside(
                    std::slice::from_ref(&root),
                    "git",
                    windows,
                    &[dir.join("ws")]
                ),
                None
            );
            let _ = std::fs::remove_file(&real);
            let _ = std::fs::remove_file(&planted);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Makes `link` lead to `target`, a file or a directory. `false` where
    /// the system would not (Windows without the right to make links,
    /// which CI's runners have).
    fn link(target: &Path, link: &Path) -> bool {
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        match made {
            Ok(()) => true,
            Err(error) => {
                assert!(
                    std::env::var_os("CI").is_none(),
                    "CI's runners can make links: {error}"
                );
                false
            }
        }
    }

    /// A program reached through a place inside an avoided root is passed
    /// over, whichever way the link points: a link inside the root that
    /// leads out (the link can be replaced after the harness chose it), a
    /// link outside that leads in, a link outside that leads through a link
    /// inside the root and out again, and a directory of `PATH` that is a
    /// link into the root. Found by the early review of the harness's own
    /// processes' fixes: only where the last link led was looked at.
    #[test]
    fn a_link_into_or_out_of_an_avoided_root_is_passed_over() {
        let windows = cfg!(windows);
        let name = if windows { "git.exe" } else { "git" };
        let dir = scratch("links");
        let ws = dir.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let real = program(&dir.join("usr").join("bin"), name);
        let real_dir = real.parent().unwrap().to_path_buf();
        let outside = program(&dir.join("opt").join("bin"), name);
        let avoid = [ws.clone()];

        // In the root, leading out.
        let out_of = ws.join("venv").join("bin");
        if !link(&outside, &out_of.join(name)) {
            return;
        }
        // Outside, leading in.
        let planted = program(&ws.join("planted"), name);
        let into = dir.join("into");
        assert!(link(&planted, &into.join(name)));
        // Outside, through a link in the root, and out again.
        let middle = ws.join("middle");
        assert!(link(&outside, &middle));
        let through = dir.join("through");
        assert!(link(&middle, &through.join(name)));
        // A directory of `PATH` that is a link into the root.
        let linked_dir = dir.join("linked-dir");
        assert!(link(&ws.join("planted"), &linked_dir));

        for entry in [&out_of, &into, &through, &linked_dir] {
            let path = [entry.clone(), real_dir.clone()];
            assert_eq!(
                find_outside(&path, "git", windows, &avoid),
                Some(real_dir.join(name)),
                "through {}",
                entry.display()
            );
            // Nothing avoided, the link is taken: it is a program.
            assert_eq!(
                find_outside(&path, "git", windows, &[]),
                Some(entry.join(name)),
                "{}",
                entry.display()
            );
        }
        // A link outside that leads outside is taken.
        let elsewhere = dir.join("elsewhere");
        assert!(link(&outside, &elsewhere.join(name)));
        assert_eq!(
            find_outside(std::slice::from_ref(&elsewhere), "git", windows, &avoid),
            Some(elsewhere.join(name))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A chain of links that never ends is taken to be inside a root.
    #[cfg(unix)]
    #[test]
    fn a_link_that_leads_to_itself_is_inside() {
        let dir = scratch("loop");
        let looped = dir.join("git");
        std::os::unix::fs::symlink(&looped, &looped).unwrap();
        assert!(passes_through(&looped, &[dir.join("elsewhere")]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
