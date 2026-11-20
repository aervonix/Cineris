//! Integration tests (tier 2) of the extension host.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "an integration test fails by panicking"
)]

mod engine;
