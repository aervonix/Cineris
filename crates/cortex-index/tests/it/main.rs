//! Integration tests (tiers 1 and 2): outlines of fixture files, and the
//! index on a real file system.
//!
//! Every test that writes works in a directory of its own. Time comes from
//! the fake clock; nothing waits on a real one.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::missing_panics_doc,
    reason = "an integration test prepares real files, fails by panicking, and prints measurements"
)]

mod index;
mod map;
mod memory;
mod outlines;
mod perf;

use std::path::{Path, PathBuf};

/// The resident memory of this process, in kilobytes, where the system
/// says (Linux): `VmRSS` now, or `VmHWM`, the most it has been.
pub(crate) fn resident_kib(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find(|line| line.starts_with(field))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
}

/// Where the fixture files are.
pub(crate) fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

/// Compares text with a checked-in snapshot under `tests/it/snapshots`.
/// With `CORTEX_UPDATE_SNAPSHOTS` set, the snapshot is written instead, to
/// be reviewed like code.
pub(crate) fn assert_snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("it")
        .join("snapshots")
        .join(name);
    if std::env::var_os("CORTEX_UPDATE_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a snapshot has a directory"))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        std::fs::write(&path, actual).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("no snapshot {}: {error}", path.display()))
        // A checkout on Windows may have turned the line endings.
        .replace("\r\n", "\n");
    assert!(
        expected == actual,
        "{name} differs from its snapshot.\n--- snapshot\n{expected}\n--- now\n{actual}\n\
         If the change is intended, run the test with CORTEX_UPDATE_SNAPSHOTS=1 and review the \
         difference."
    );
}
