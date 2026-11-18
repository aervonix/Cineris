//! Fuzzes the outline extractors: any bytes as a file, outlined in every
//! language and by the fallback.
#![no_main]
#![allow(
    clippy::disallowed_types,
    reason = "the fuzzer's own macro reads a crash file; nothing here is the product"
)]

libfuzzer_sys::fuzz_target!(|data: &[u8]| cineris_fuzz::extractors(data));
