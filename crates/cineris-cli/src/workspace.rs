//! Where a run is: the workspace root, found from the working directory by
//! walking up (`docs/architecture/08-configuration.md`, "The workspace
//! root"), and the working directory, where its commands start.
//!
//! Every binary (the interactive application, headless mode, the
//! subcommands) finds the root here before the core is asked anything
//! (since the follow-ups of step 23, the owner's choice of 2026-11-16):
//! started in `repo/src`, the repository's `.cineris/config.toml` is read,
//! its instructions are found from the root, the trust said at `repo/`
//! applies, and the sessions and history are the root's.

use std::io;
use std::path::{Path, PathBuf};

/// Where a run is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Here {
    /// The workspace root: the nearest directory from the working
    /// directory up that holds a `.cineris` folder or a `.git` the user
    /// owns, or the working directory itself when none does.
    pub(crate) root: PathBuf,
    /// The working directory, when it is below the root: where commands
    /// start (`session/new`'s `cwd`).
    pub(crate) cwd: Option<PathBuf>,
    /// What was passed over on the way up, said to the person.
    pub(crate) passed_over: Vec<PathBuf>,
    /// The working directory is below the root by its name but not where
    /// it leads (a link or a junction on the way to it leads out of the
    /// workspace), or its name is not Unicode: commands start at the root,
    /// and the person is told (the review of the follow-ups of step 23).
    pub(crate) cwd_not_taken: Option<PathBuf>,
}

impl Here {
    /// Where a run started in `start` is.
    pub(crate) fn of(start: &Path) -> Self {
        let found = cineris_config::find_workspace_root(start);
        let mut cwd = (found.root.as_path() != start).then(|| start.to_path_buf());
        let mut cwd_not_taken = None;
        if let Some(below) = &cwd
            && !leads_inside(below, &found.root)
        {
            cwd_not_taken = cwd.take();
        }
        Self {
            root: found.root,
            cwd,
            passed_over: found.passed_over,
            cwd_not_taken,
        }
    }

    /// Where a run started in the working directory is.
    pub(crate) fn now() -> io::Result<Self> {
        Ok(Self::of(&std::env::current_dir()?))
    }

    /// What to tell the person of what was passed over, and of a working
    /// directory not taken: a line each.
    pub(crate) fn passed_over_said(&self) -> Vec<String> {
        let mut said: Vec<String> = self
            .passed_over
            .iter()
            .map(|path| {
                format!(
                    "`{}` was not taken as the workspace's: you do not own it, or it is a link. \
                     The workspace is `{}`.",
                    path.display(),
                    self.root.display()
                )
            })
            .collect();
        if let Some(cwd) = &self.cwd_not_taken {
            said.push(format!(
                "Commands start at the workspace's root `{}`: the working directory `{}` is \
                 reached through a link that leads out of it, or its name is not Unicode.",
                self.root.display(),
                cwd.display()
            ));
        }
        said
    }
}

/// Whether `cwd` is Unicode, and leads to a directory inside `root` once
/// links and junctions are followed, both as the system resolves them
/// (compared by `cineris_base::location_within`, which takes the canonical
/// form's prefix off and folds case as the file system does). Where either
/// cannot be resolved, the core judges it (`session/new` refuses a `cwd`
/// outside the workspace).
fn leads_inside(cwd: &Path, root: &Path) -> bool {
    if cwd.to_str().is_none() {
        return false;
    }
    match (std::fs::canonicalize(cwd), std::fs::canonicalize(root)) {
        (Ok(cwd), Ok(root)) => cineris_base::location_within(&cwd, &root),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Started in a folder of a repository, the root is the repository's,
    /// and the folder is where commands start; started at the root, there
    /// is no folder (the follow-ups of step 23: every binary took the
    /// working directory for the root).
    #[test]
    fn the_root_is_found_by_walking_up_and_the_folder_is_kept() {
        let scratch = std::env::temp_dir().join(format!("cineris-cli-here-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        // Resolved, as the working directory the system gives is.
        std::fs::create_dir_all(&scratch).unwrap();
        let scratch = std::fs::canonicalize(&scratch).unwrap();
        let repo = scratch.join("repo");
        std::fs::create_dir_all(repo.join(".cineris")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        let below = Here::of(&repo.join("src/deep"));
        assert_eq!(below.root, repo);
        assert_eq!(below.cwd, Some(repo.join("src/deep")));
        assert!(below.passed_over.is_empty());
        assert!(below.passed_over_said().is_empty());
        let at_root = Here::of(&repo);
        assert_eq!(at_root.root, repo);
        assert_eq!(at_root.cwd, None);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A working directory below the root by its name but reached through
    /// a link out of the workspace starts at the root, and says so; a
    /// `.cineris` that is a link is passed over, and said (the review of the
    /// follow-ups of step 23: the first failed the session's open).
    #[cfg(unix)]
    #[test]
    fn a_start_through_a_link_out_starts_at_the_root_and_says_why() {
        let scratch =
            std::env::temp_dir().join(format!("cineris-cli-here-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let scratch = std::fs::canonicalize(&scratch).unwrap();
        let repo = scratch.join("repo");
        std::fs::create_dir_all(repo.join(".cineris")).unwrap();
        std::fs::create_dir_all(scratch.join("outside/sub")).unwrap();
        std::os::unix::fs::symlink(scratch.join("outside"), repo.join("lib")).unwrap();
        let here = Here::of(&repo.join("lib/sub"));
        assert_eq!(here.root, repo);
        assert_eq!(here.cwd, None, "a start outside the workspace");
        assert_eq!(here.cwd_not_taken, Some(repo.join("lib/sub")));
        let said = here.passed_over_said();
        assert!(
            said.iter()
                .any(|line| line.contains("Commands start at the workspace's root")),
            "{said:?}"
        );
        // A `.cineris` that is a link: passed over, and said.
        let linked = scratch.join("linked");
        std::fs::create_dir_all(linked.join("src")).unwrap();
        std::os::unix::fs::symlink(scratch.join("outside"), linked.join(".cineris")).unwrap();
        let here = Here::of(&linked.join("src"));
        assert!(
            here.passed_over.contains(&linked.join(".cineris")),
            "{here:?}"
        );
        assert!(
            here.passed_over_said()
                .iter()
                .any(|line| line.contains("was not taken as the workspace's")),
            "{:?}",
            here.passed_over_said()
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// The same on Windows, with a real junction (`mklink /J`, which needs
    /// no administrator's rights): a working directory below the root by
    /// its name, reached through a junction out of the workspace, starts
    /// at the root and says so, where it failed the session's open (the
    /// full review of the follow-ups of step 23, finding 3). `sys_`: it
    /// starts `cmd.exe` to make the junction.
    #[cfg(windows)]
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test starts cmd.exe once, to make a junction"
    )]
    fn sys_a_start_through_a_junction_out_starts_at_the_root_and_says_why() {
        let scratch =
            std::env::temp_dir().join(format!("cineris-cli-here-junction-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let repo = scratch.join("repo");
        std::fs::create_dir_all(repo.join(".cineris")).unwrap();
        let outside = scratch.join("outside");
        std::fs::create_dir_all(outside.join("sub")).unwrap();
        let link = repo.join("lib");
        let made = std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(&link)
            .arg(&outside)
            .output()
            .expect("cmd.exe starts");
        assert!(made.status.success(), "{made:?}");
        let start = link.join("sub");
        let here = Here::of(&start);
        assert_eq!(here.root, repo);
        assert_eq!(here.cwd, None, "a start outside the workspace");
        assert_eq!(here.cwd_not_taken, Some(start.clone()));
        let said = here.passed_over_said();
        assert!(
            said.iter()
                .any(|line| line.contains("Commands start at the workspace's root")),
            "{said:?}"
        );
        // A folder of the workspace itself, beside the junction, is taken.
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let inside = Here::of(&repo.join("src"));
        assert_eq!(inside.cwd, Some(repo.join("src")));
        // The junction is removed as a directory, without what it leads to.
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
