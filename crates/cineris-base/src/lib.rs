//! Small shared primitives that every other crate leans on
//! (`docs/architecture/01-workspace.md`).
//!
//! | Module | What it gives |
//! |---|---|
//! | [`error`] | [`ErrorKind`], the closed set that drives retries, exit codes and the protocol's error kind, and [`Classified`], by which an error says which kind it is. |
//! | [`secret`] | [`Secret`], a credential that cannot be printed or serialised. |
//! | [`secret_buf`] | [`SecretBuf`], a key being typed: fixed capacity, written over with zeros when dropped, as a best effort (since step 25). |
//! | [`time`], [`clock`] | [`UtcTime`] and its RFC 3339 form; the [`Clock`] trait, the system clock and the [`FakeClock`]. |
//! | [`rng`] | The [`Rng`] trait, the system generator and the [`SeededRng`]. |
//! | [`id`] | [`IdGenerator`] for unique, time-ordered ids and [`IdCounter`] for counted ones. |
//! | [`cancel`] | [`CancelToken`], the tree of cancellation tokens. |
//! | [`path`] | [`AbsPath`], [`RelPath`] and [`PathResolver`]: paths read, checked and resolved by one set of rules, with the Windows rules. |
//! | [`text`] | Width in terminal columns, wrapping and truncation, by grapheme cluster. |
//! | [`visible`](mod@visible) | [`visible()`]: text from the workspace as the model and a client are shown it, every character that would act unseen (a line break, a bidirectional control, a tag character) written as an escape. |
//! | [`atomic`] | [`write_atomic`]: replace a file's content in one step. |
//! | [`nofollow`] | [`open_regular`]: open a regular file without following a link and without waiting on a FIFO. |
//! | [`hash`] | [`ContentHash`]: the hash a piece of content is known by. |
//! | [`hook`] | [`hook::point`]: a place in library code where a test can make the thread stop, and with the `test-hooks` feature the `Hold` that does it. Nothing at all without the feature. |
//! | [`private`] | Files and directories that only the current user can open, on Linux and on Windows. |
//! | [`framing`] | The newline-delimited JSON reader and writer of the RPC protocol. |
//! | [`watch`] | [`WatchList`]: the modification-time check behind hot-reload. |
//! | [`zone`] | [`Zone`] and [`LocalDate`]: the local date at a moment, for the usage view's days. |
//!
//! # Blocking and async
//!
//! This crate has no async runtime. The modules that touch the file system
//! ([`path`]'s resolver, [`atomic`], [`nofollow`], [`private`], [`watch`]) block, and async callers
//! run them on the blocking pool. [`CancelToken`] is built on the standard
//! library's wakers and works under any runtime. [`framing`] has a decoder
//! that does no I/O at all, so one implementation serves pipes, sockets and
//! tests.
//!
//! # Fakes
//!
//! [`FakeClock`] and [`SeededRng`] live here, beside the traits they
//! implement, as ordinary public types: this crate's own tests need them,
//! and it cannot depend on the test kit, which depends on it. Other crates'
//! tests reach them through `cineris-testkit`.
//!
//! # Unsafe code
//!
//! Two modules of this crate may use `unsafe`, both on Windows only:
//! `private::windows`, which sets access lists through the Windows API, and
//! reads the reparse tag and the identity of an open file for [`nofollow`]
//! (`docs/decisions/0029-owner-only-files-on-windows.md`); and
//! `zone::windows`, which reads the system's time zone, which only that API
//! can give. Everywhere else in the crate the workspace's
//! `unsafe_code = "deny"` holds.

pub mod atomic;
pub mod cancel;
pub mod child_env;
pub mod clock;
pub mod error;
pub mod framing;
pub mod git;
pub mod hash;
pub mod hook;
pub mod id;
pub mod log;
pub mod nofollow;
pub mod path;
pub mod private;
pub mod program;
pub mod redact;
pub mod rng;
pub mod secret;
pub mod secret_buf;
pub mod text;
pub mod time;
pub mod timer;
pub mod visible;
pub mod watch;
pub mod zone;

#[cfg(test)]
mod cases;
mod sync;

pub use atomic::{AtomicWriteError, AtomicWriteOptions, write_atomic};
pub use cancel::CancelToken;
pub use clock::{Clock, FakeClock, SharedClock, SystemClock};
pub use error::{Classified, ErrorKind, Retry};
pub use framing::{FrameDecoder, FrameError, FrameReader, FrameWriter, StreamError};
pub use git::{GIT_ENV, GIT_HARDENING};
pub use hash::{ContentHash, ContentHasher};
pub use id::{IdCounter, IdGenerator};
pub use nofollow::{FileIdentity, OpenError, open_regular};
pub use path::{
    AbsPath, PathError, PathOptions, PathResolver, PathStyle, RelPath, ResolvedPath,
    location_within, same_location, strip_location_prefix,
};
pub use redact::SecretFilter;
pub use rng::{Rng, SeededRng, SystemRng};
pub use secret::Secret;
pub use secret_buf::SecretBuf;
pub use time::{TimeError, UtcTime};
pub use timer::{SharedTimer, Sleep, Timer};
pub use visible::{acts_unseen, unescape, visible, visible_keeping, visible_keeping_after};
pub use watch::{FileStamp, TimedStamp, WatchList};
pub use zone::{LocalDate, Zone};
