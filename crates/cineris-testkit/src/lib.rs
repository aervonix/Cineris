//! Test support for the whole workspace (`docs/testing-and-ci.md`). A
//! dev-dependency only: nothing here is part of a shipped build.
//!
//! | Module | What it gives |
//! |---|---|
//! | re-exports | [`FakeClock`] and [`SeededRng`], which live in `cineris-base` beside the traits they implement. |
//! | [`cases`] | Property tests without a property-testing crate. |
//! | [`corpus`] | [`corpus::real_code`]: the pinned corpus of about 20,000 `.rs` files of real Rust, verified, for a `perf_` test that measures real code. Fetched by `cargo xtask corpus fetch`; no blocking test asks for it. |
//! | [`canary`] | [`assert_never_printed`]: no `Debug` or `Display` form of a value holds a made-up key. |
//! | [`timer`] | [`FakeTimer`]: waits that move the fake clock instead of waiting. |
//! | [`gate`] | [`Gate`]: how two sides of a test take turns without sleeping. |
//! | [`hostile`] | The hostile names every test takes its names from: control and format characters, bidirectional controls, line breaks, escape sequences, very long names, Windows device names, and what a file system takes as a file name. The same set on every system. |
//! | [`hold`] | [`Hold`]: a point of library code (`cineris_base::hook`) that a test makes the library stop at, and lets go. For race tests that need the library to stop between two of its steps. |
//! | [`stalled`] | [`stalled_file`] (Unix only): a FIFO with nobody at its other end, which stalls whoever opens it, for code that must go on, or give up, when a disk does not answer. |
//! | [`untouched`] | [`Untouched`]: something the start-up path must not touch (the repository index, the statistics), held while a session opens. |
//! | [`scratch`] | [`Scratch`]: a directory of a test's own, with files, a git repository when the test needs one, and nothing left behind. |
//! | [`seed`] | The seed of a test whose order of events is seeded: 0 in an ordinary run, the next one for each repeat of `cargo xtask stress`, and printed so that a failure can be run again. |
//! | [`children`] | Child processes for process tests: the test binary run again as a child, a grandchild or a child that will not stop, and a [`Roll`](children::Roll) that shows each one start and die. |
//! | [`role`] | The test binary as a child in roles ([`Role`](role::Role)), a [`Report`](role::Report) file of what each child found, and a [`ModeShell`](role::ModeShell) that says whether the terminal's modes came back. For tests of a terminal. |
//! | [`rerun`] | [`Rerun`](rerun::Rerun): the running test again in a child process, with a `PATH`, a working directory and variables of its own, for tests of how a program is found. |
//! | [`exec`] | [`block_on`], which runs async code with no runtime, and [`join`], which runs two futures together. |
//! | [`brief`] | [`brief`]: a long text in an assertion's message as its size and its two ends. |
//! | [`built`] | [`ScriptedProvider::built_as`]: the script behind a real protocol's request builder, failing a request the protocol cannot build. |
//! | [`provider`] | [`ScriptedProvider`]: a model that says what the test tells it to. For tests of the agent loop. |
//! | [`transport`] | [`ScriptedTransport`]: recorded responses in place of a network. For tests of protocols and of the client. |
//! | [`protocol`] | [`TestProtocol`]: a wire dialect made for tests. |
//! | [`server`] | [`SseServer`]: a provider's HTTP endpoint on the loopback interface. For tests of the real HTTP client, and of the real binary. |
//! | [`fixtures`] | Made-up models with every shape of speed tier, and requests to them. |
//! | [`numbers`] | [`numbers::report`]: a measurement's name, value and unit as one line, `perf: NAME VALUE UNIT`, which `cargo xtask perf-ab` reads to compare two trees' `perf_` tests. |
//! | [`phases`] | [`Phases`]: where a `perf_` test's time went, by named phase, printed as a breakdown with each phase's time and share. Always on. |
//! | [`profile`] | Where a test's time went, by phase and wait, with `CINERIS_TESTKIT_PROFILE=1`. Nothing when it is off. |
//! | [`watch`] | What a test was doing when it ran out of time: its waits and its process tree, written to its standard error before nextest stops it. And `CINERIS_TESTKIT_PAUSE_MS`, which widens the scripted server's races on purpose. |
//! | [`fixture`] | Recorded provider responses: [`Fixture`](fixture::Fixture), the files one is made of, and [`Scrubber`](fixture::Scrubber), which takes credentials and identifiers out of a recording and refuses one that holds the key. |
//! | [`vterm`] | [`VTerm`]: a virtual terminal, and a [`Report`](vterm::Report) of what each frame did to it. For tests of the presenter, and to parse a program's output. |
//! | [`pty`] | [`pty::Session`]: a program run in a pseudo-terminal (ConPTY on Windows), its output recorded and parsed, keys typed into it, and its console closed under it ([`pty::Session::close_console`]). |
//! | [`exit`] | [`Exit`](exit::Exit) and [`Verdict`](exit::Verdict): how a process ended, read from either system's wording (`exit status: 1`, `exit code: 1`, `signal: 6`, `exit code: 0xc000013a`), so that a test spells none. |
//! | [`cadence`] | [`Cadence`](cadence::Cadence): keys typed at a pace, read together, pasted, or held down with a terminal's auto-repeat ([`AutoRepeat`](cadence::AutoRepeat)), as events with the times they were read, for a test of an interface that tells them apart by time. No real waiting: the times are the test's. |
//! | [`glyphs`] | [`GlyphSets`](glyphs::GlyphSets): match a screen against text spelled once, as a function of the glyph set, when the interface may have drawn the Unicode set or its ASCII stand-ins. |
//!
//! # No real waiting
//!
//! Nothing here sleeps. Time is the [`FakeClock`]; a wait is a
//! [`FakeTimer`]'s; an order between two threads is a [`Gate`]. The limits
//! that appear on blocking waits are bounds that turn a mistake into a
//! failed test.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test support fails by panicking, with a message that says what was expected"
)]

pub mod brief;
pub mod built;
pub mod cadence;
pub mod canary;
pub mod cases;
pub mod children;
pub mod conformance;
pub mod corpus;
pub mod exec;
pub mod exit;
pub mod fixture;
pub mod fixtures;
pub mod gate;
pub mod glyphs;
pub mod hold;
pub mod hostile;
pub mod numbers;
pub mod phases;
pub mod profile;
pub mod protocol;
pub mod provider;
pub mod pty;
pub mod rerun;
pub mod role;
pub mod schema;
pub mod scratch;
pub mod seed;
pub mod server;
#[cfg(unix)]
pub mod stalled;
pub mod timer;
pub mod transport;
pub mod untouched;
pub mod vterm;
pub mod watch;

pub use brief::brief;
pub use built::Built;
pub use canary::{Printed, assert_never_printed};
pub use cineris_base::hook::{MARK_FAILS, READY_DIVISOR};
pub use cineris_base::{Clock, FakeClock, Rng, SeededRng};
pub use exec::{block_on, join};
pub use gate::Gate;
pub use hold::Hold;
pub use phases::Phases;
pub use protocol::TestProtocol;
pub use provider::{Response, ScriptedProvider};
pub use scratch::Scratch;
pub use server::{ReceivedRequest, ServerReply, SseServer};
#[cfg(unix)]
pub use stalled::{Stalled, stalled_file};
pub use timer::FakeTimer;
pub use transport::{Reply, ScriptedTransport};
pub use untouched::Untouched;
pub use vterm::VTerm;
