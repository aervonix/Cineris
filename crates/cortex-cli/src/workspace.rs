//! Where a run is: the workspace root, found from the working directory by
//! walking up (`docs/architecture/08-configuration.md`, "The workspace
//! root"), and the working directory, where its commands start.
//!
//! Every binary (the interactive application, headless mode, the
//! subcommands) finds the root here before the core is asked anything
//! (since the follow-ups of step 23, the owner's choice of 2026-11-16):
//! started in `repo/src`, the repository's `.cortex/config.toml` is read,
//! its instructions are found from the root, the trust said at `repo/`
//! applies, and the sessions and history are the root's.

use std::io;
use std::path::{Path, PathBuf};

/// Where a run is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Here {
    /// The workspace root: the nearest directory from the working
    /// directory up that holds a `.cortex` folder or a `.git` the user
    /// owns, or the working directory itself when none does.
    pub(crate) root: PathBuf,
    /// The working directory, when it is below the root: where commands
    /// start (`session/new`'s `cwd`).
    pub(crate) cwd: Option<PathBuf>,
    /// What was passed over on the way up, said to the person.
    pub(crate) passed_over: Vec<PathBuf>,
}

impl Here {
    /// Where a run started in `start` is.
    pub(crate) fn of(start: &Path) -> Self {
        let found = cortex_config::find_workspace_root(start);
        let cwd = (found.root.as_path() != start).then(|| start.to_path_buf());
        Self {
            root: found.root,
            cwd,
            passed_over: found.passed_over,
        }
    }

    /// Where a run started in the working directory is.
    pub(crate) fn now() -> io::Result<Self> {
        Ok(Self::of(&std::env::current_dir()?))
    }

    /// What to tell the person of what was passed over: a line each.
    pub(crate) fn passed_over_said(&self) -> Vec<String> {
        self.passed_over
            .iter()
            .map(|path| {
                format!(
                    "`{}` was not taken as the workspace's: you do not own it, or it is a link. \
                     The workspace is `{}`.",
                    path.display(),
                    self.root.display()
                )
            })
            .collect()
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
        let scratch =
            std::env::temp_dir().join(format!("cortex-cli-here-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        // Resolved, as the working directory the system gives is.
        std::fs::create_dir_all(&scratch).unwrap();
        let scratch = std::fs::canonicalize(&scratch).unwrap();
        let repo = scratch.join("repo");
        std::fs::create_dir_all(repo.join(".cortex")).unwrap();
        std::fs::create_dir_all(repo.join("src/deep")).unwrap();
        let below = Here::of(&repo.join("src/deep"));
        assert_eq!(below.root, repo);
        assert_eq!(below.cwd, Some(repo.join("src/deep")));
        assert!(below.passed_over.is_empty());
        let at_root = Here::of(&repo);
        assert_eq!(at_root.root, repo);
        assert_eq!(at_root.cwd, None);
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
