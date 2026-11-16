//! A file that stalls whoever opens it: a FIFO with nobody at its other end
//! (`docs/testing-and-ci.md`, "A reviewer's kit"). Unix only.
//!
//! Opening a FIFO for writing waits until something opens it for reading,
//! and the other way round. Code that opens a path on a disk that does not
//! answer (a network share, a failing drive) waits too, and no test can
//! make a real disk do that. A FIFO in the file's place does it, with no
//! timing: the open blocks in the kernel until the test lets it go.
//!
//! **Nothing stands in on Windows.** An open of a file there fails or
//! succeeds at once, a named pipe is not a path that `File::open` can wait
//! on, and a locked file is refused, not waited for. A test of what a stall
//! does is `#[cfg(unix)]`, as the tests that use a FIFO are.
//!
//! The test must run the code that opens the file on a thread of its own and
//! wait for it within a bound ([`Gate::wait`](crate::Gate::wait)), so that
//! a mistake is a failed test and not a hung one.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "test support: it makes a FIFO and opens it, on a thread of its own, to stall and release a test's file"
)]

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A FIFO made at a path, with nobody at its other end. See the
/// [module](self).
#[derive(Debug)]
pub struct Stalled {
    path: PathBuf,
    released: bool,
}

/// Makes a FIFO at `path`, whose directory must exist and where nothing
/// is. Whoever opens it waits; a writer that waits goes on when the test
/// calls [`Stalled::release`] or drops the value. It stands in for a disk
/// that holds a *writer*: the release opens the other end for reading,
/// which a reader that waits is not let go by (two readers of a FIFO both
/// wait for a writer).
///
/// # Panics
///
/// When `mkfifo` cannot be run or refuses.
#[must_use]
pub fn stalled_file(path: &Path) -> Stalled {
    let made = Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("run mkfifo");
    assert!(made.success(), "mkfifo failed for {}", path.display());
    Stalled {
        path: path.to_path_buf(),
        released: false,
    }
}

impl Stalled {
    /// The path of the FIFO.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Lets the openers that are waiting now go on, and throws away what
    /// they write. An opener that comes later waits again. It is the other
    /// end of the FIFO, held by a thread of its own that ends when the
    /// writers are gone (or with the process, if none ever comes), so it
    /// never makes the test wait. Dropping the value does it too; the FIFO
    /// must still be where it was then, which is so while the test's
    /// scratch directory is.
    pub fn release(&mut self) {
        if std::mem::replace(&mut self.released, true) {
            return;
        }
        let path = self.path.clone();
        // If the thread cannot be started, nothing is released: the
        // opener's own bound fails the test.
        let _ = std::thread::Builder::new()
            .name("cortex-testkit-stalled-release".to_owned())
            .spawn(move || {
                if let Ok(mut other_end) = File::open(path) {
                    let _ = std::io::copy(&mut other_end, &mut std::io::sink());
                }
            });
    }
}

impl Drop for Stalled {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::FileTypeExt;
    use std::time::Duration;

    use super::*;
    use crate::{Gate, Scratch};

    /// `mkfifo` is a process, hence the `sys_` name.
    #[test]
    fn sys_a_stalled_file_is_a_fifo_and_holds_an_opener_until_it_is_released() {
        let scratch = Scratch::outside("cortex-testkit-stalled", "stalled_file");
        let path = scratch.path().join("disk.log");
        let mut stalled = stalled_file(&path);
        // What makes the open wait is the kernel's, and is the file's kind.
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_fifo()
        );
        assert_eq!(stalled.path(), path);

        let (about_to_open, opened) = (Gate::new(), Gate::new());
        let opener = {
            let (path, about_to_open, opened) =
                (path.clone(), about_to_open.clone(), opened.clone());
            std::thread::spawn(move || {
                about_to_open.open();
                let file = std::fs::OpenOptions::new().write(true).open(path);
                opened.open();
                file.map(|_| ())
            })
        };
        assert!(about_to_open.wait(Duration::from_secs(5)));
        // Nothing reads the FIFO, so the open cannot have ended: this holds
        // whether or not the thread has reached it yet.
        assert!(!opened.is_open(), "the open of a stalled file ended");

        stalled.release();
        assert!(
            opened.wait(Duration::from_secs(5)),
            "the opener was not let go"
        );
        opener.join().unwrap().expect("the open succeeded");
    }
}
