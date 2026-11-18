//! The directory a command runs in (`docs/architecture/06-permissions-and-sandbox.md`,
//! "Command analysis as built", rule 5): a command that may write writes
//! where it runs, whether a `cd` or the call's own `cwd` took it there.
//!
//! Found in the re-decide step of step 12: at `autonomous`, `make` with the
//! call's `cwd` in `.git/hooks` ran unasked while `cd ./.git/hooks && make`
//! was asked about. A call planned again after an earlier call made a link
//! has its `cwd` resolved through the link, so the directory a command
//! runs in has to be judged wherever it came from.

use cineris_protocol::lease::Level;

use crate::Said::{Allow, Ask, ReadOnly};
use crate::{WORKSPACE, at, said, shell_in};

fn inside(relative: &str) -> String {
    format!("{WORKSPACE}/{relative}")
}

#[test]
fn a_command_that_may_write_is_asked_about_in_a_protected_working_directory() {
    let engine = at(Level::Autonomous);
    for directory in [".git", ".git/hooks", ".github/workflows", ".cineris"] {
        let cwd = inside(directory);
        assert_eq!(said(&shell_in(&engine, "make", &cwd)), Ask, "{directory}");
        // A line the analysis does not model writes there as surely, even
        // one whose words name nothing there.
        assert_eq!(
            said(&shell_in(&engine, "/usr/bin/make &", &cwd)),
            Ask,
            "{directory}"
        );
        // The same as a `cd` there.
        assert_eq!(
            said(&shell_in(
                &engine,
                &format!("cd ./{directory} && make"),
                WORKSPACE
            )),
            Ask,
            "{directory}"
        );
    }
}

#[test]
fn elsewhere_in_the_workspace_and_for_a_command_that_only_reads_nothing_changes() {
    let engine = at(Level::Autonomous);
    for cwd in [WORKSPACE.to_owned(), inside("src"), inside("crates/core")] {
        assert_eq!(said(&shell_in(&engine, "make", &cwd)), Allow, "{cwd}");
    }
    // A command that only reads had its directory judged as a read.
    assert_eq!(
        said(&shell_in(&engine, "ls", &inside(".git/hooks"))),
        ReadOnly
    );
}
