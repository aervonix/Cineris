//! Integration tests (tier 2): a project's knowledge on a real file
//! system. Each test has a directory of its own, with a state directory
//! and a workspace in it. No test waits: time comes from the fake clock.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "an integration test prepares real files, fails by panicking, and a measurement prints"
)]

mod learn;
mod measure;
mod project;
mod render;
mod store;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cineris_base::{FakeClock, SharedClock, UtcTime};
use cineris_knowledge::{Entry, Project, Snapshot};
use cineris_protocol::ids::EntryId;
use cineris_protocol::project::{EntryAction, EntryChange};
use cineris_testkit::Scratch;

/// When every test begins.
pub(crate) const START: &str = "2026-11-16T10:00:00Z";

pub(crate) const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// One project: a state directory and a workspace, and the clock.
pub(crate) struct World {
    pub(crate) scratch: Scratch,
    pub(crate) clock: Arc<FakeClock>,
}

impl World {
    pub(crate) fn new(scratch: Scratch) -> Self {
        scratch.dir("work");
        Self {
            scratch,
            clock: Arc::new(FakeClock::starting_at(START).unwrap()),
        }
    }

    /// The private store's file. Its directory is not made.
    pub(crate) fn store_file(&self) -> PathBuf {
        self.scratch
            .join("state")
            .join("projects")
            .join("0123456789abcdef0123456789abcdef")
            .join("knowledge.toml")
    }

    pub(crate) fn good_copy(&self) -> PathBuf {
        self.project().store().good_copy()
    }

    /// A handle on the project, as another session or process would have.
    pub(crate) fn project(&self) -> Project {
        let clock: SharedClock = self.clock.clone();
        Project::new(self.store_file(), self.scratch.join("work"), clock)
    }

    pub(crate) fn now(&self) -> UtcTime {
        self.project().now()
    }

    /// Writes the private store by hand, as a person would.
    pub(crate) fn write_store(&self, text: &str) {
        std::fs::create_dir_all(self.store_file().parent().unwrap()).unwrap();
        std::fs::write(self.store_file(), text).unwrap();
    }

    pub(crate) fn store_text(&self) -> String {
        std::fs::read_to_string(self.store_file()).unwrap()
    }

    /// Writes the repository's shared file.
    pub(crate) fn write_shared(&self, text: &str) {
        self.scratch.file("work/.cineris/profile.toml", text);
    }

    pub(crate) fn shared_text(&self) -> String {
        self.scratch.read_text("work/.cineris/profile.toml")
    }

    pub(crate) fn load(&self) -> Snapshot {
        self.project().load(None)
    }
}

pub(crate) fn id(text: &str) -> EntryId {
    EntryId::new(text)
}

pub(crate) fn change(entry: &str, action: EntryAction) -> EntryChange {
    EntryChange {
        id: id(entry),
        action,
        text: None,
    }
}

/// The ids of the entries in use now, in the snapshot's order.
pub(crate) fn in_use(world: &World, snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .in_use(world.now())
        .map(|entry| entry.id.as_str().to_owned())
        .collect()
}

/// The command lines that may run without asking now.
pub(crate) fn unasked(world: &World, snapshot: &Snapshot) -> Vec<String> {
    snapshot
        .runs_unasked(world.now())
        .filter_map(|entry: &Entry| entry.command.as_ref())
        .map(|command| command.run.clone())
        .collect()
}

/// A private store with one entry of each standing a test needs: an
/// accepted observed command, an observed command that became active by
/// itself, a model's proposal, a user's convention, and a disabled fact.
pub(crate) const MIXED: &str = r#"version = 1

[[entry]]
id = "k_1"
kind = "command"
text = "The project's test command."
status = "active"
basis = "observed"
confirmed = "2026-11-16T09:00:00Z"
expires = "2027-02-14T09:00:00Z"
accepted = "2026-11-16T09:30:00Z"
seen = 2
learned = { session = "s_1", at = "2026-11-16T09:00:00Z" }
command = { purpose = "test", run = "cargo test", usual_ms = 4200 }

[[entry]]
id = "k_2"
kind = "command"
text = "The project's build command."
status = "active"
basis = "observed"
confirmed = "2026-11-16T09:00:00Z"
expires = "2027-02-14T09:00:00Z"
seen = 2
learned = { session = "s_1", at = "2026-11-16T09:00:00Z" }
command = { purpose = "build", run = "cargo build", cwd = "crates/app" }

[[entry]]
id = "k_3"
kind = "fact"
text = "The release script is scripts/release.sh."
status = "proposed"
basis = "proposed"
evidence = "README.md, line 40"
learned = { session = "s_1", at = "2026-11-16T09:10:00Z" }

[[entry]]
id = "k_4"
kind = "convention"
text = "Errors are returned, never panicked on."
status = "active"

[[entry]]
id = "k_5"
kind = "fact"
text = "The old build used make."
status = "disabled"
"#;

/// A shared file with a command and a convention.
pub(crate) const TEAM: &str = r#"# Our profile.
version = 1

[[entry]]
kind = "command"
text = "Lints the workspace."
command = { purpose = "check", run = "cargo clippy" }

[[entry]]
kind = "convention"
text = "Every public item is documented."
"#;

/// Makes a symbolic link at `link` to `target`, a directory's or a file's.
/// `false` where this machine cannot: on Windows making one needs a
/// privilege an ordinary account lacks unless Developer Mode is on. The CI
/// runner has it, and there the link must be made.
pub(crate) fn link(target: &std::path::Path, link: &std::path::Path, directory: bool) -> bool {
    #[cfg(unix)]
    {
        let _ = directory;
        std::os::unix::fs::symlink(target, link).unwrap();
        true
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::{symlink_dir, symlink_file};
        let made = if directory {
            symlink_dir(target, link)
        } else {
            symlink_file(target, link)
        };
        match made {
            Ok(()) => true,
            Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
                println!("skipped: cannot make a symbolic link here ({error})");
                false
            }
            Err(error) => panic!("the CI runner cannot make a symbolic link: {error}"),
        }
    }
}

/// A private store, as a person would write it, of exactly `bytes` bytes:
/// a short fact `k_1`, then facts of long text. Written back by the
/// harness it is larger, by the lines the harness adds.
pub(crate) fn store_of(bytes: usize) -> String {
    let entry = |number: usize, text: usize| {
        format!(
            "[[entry]]\nid = \"k_{number}\"\nkind = \"fact\"\ntext = \"{}\"\nstatus = \"active\"\n",
            "t".repeat(text)
        )
    };
    let mut store = String::from("version = 1\n");
    store.push_str(&entry(1, 6));
    let mut number = 1;
    while bytes - store.len() > 1_300 {
        number += 1;
        store.push_str(&entry(number, 600));
    }
    // What is left is shared between two entries, so that neither text is
    // empty or too long.
    let left = bytes - store.len();
    let overhead = entry(number + 1, 0).len() + entry(number + 2, 0).len();
    let first = (left - overhead) / 2;
    store.push_str(&entry(number + 1, first));
    store.push_str(&entry(number + 2, left - overhead - first));
    assert_eq!(store.len(), bytes);
    store
}
