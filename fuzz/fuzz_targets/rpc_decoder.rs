//! Fuzzes the RPC server's decoder: any bytes as what a client sends, cut
//! anywhere, read into lines, messages, operations and answers.
#![no_main]
#![allow(
    clippy::disallowed_types,
    reason = "the fuzzer's own macro reads a crash file; nothing here is the product"
)]

libfuzzer_sys::fuzz_target!(|data: &[u8]| cortex_fuzz::rpc_decoder(data));
