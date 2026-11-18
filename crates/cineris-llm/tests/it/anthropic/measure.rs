//! Measurements, run by hand in a release build:
//!
//! ```text
//! cargo test --release -p cineris-llm --test it -- --ignored anthropic::measure --nocapture
//! ```
//!
//! What they measure is what the adapter adds to every request: building
//! the body from a long history, and decoding a response. The numbers are
//! in `docs/performance-budgets.md`.

#![allow(clippy::print_stdout, reason = "a measurement prints what it measured")]

use std::time::Instant;

use cineris_llm::anthropic::AnthropicMessages;
use cineris_llm::{ModelRequest, Protocol};

use super::scenario;
use super::{HAIKU, decode_whole, load, provider, user};

/// A conversation of `rounds` tool rounds: the recorded first round of the
/// conversation, with its reasoning and its two calls, over and over.
fn long_history(rounds: usize) -> ModelRequest {
    let first = scenario::desk(HAIKU, vec![user(scenario::PROMPTS[0])]);
    let events = decode_whole(&first, &load("conversation-1").response);
    let one = scenario::after(first.messages.clone(), &events, &first).expect("a round");
    let mut history = Vec::new();
    for _ in 0..rounds {
        history.extend(one.iter().cloned());
    }
    scenario::desk(HAIKU, history)
}

fn median(mut times: Vec<f64>) -> f64 {
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

#[test]
#[ignore = "a measurement; run it in a release build"]
fn building_a_request_from_a_long_history() {
    for rounds in [10, 100, 1000] {
        let request = long_history(rounds);
        let size = AnthropicMessages
            .build(&request, &provider())
            .unwrap()
            .body
            .len();
        let times: Vec<f64> = (0..15)
            .map(|_| {
                let started = Instant::now();
                let http = AnthropicMessages.build(&request, &provider()).unwrap();
                let took = started.elapsed().as_secs_f64() * 1e6;
                assert_eq!(http.body.len(), size);
                took
            })
            .collect();
        println!(
            "build: {rounds} tool rounds, {} messages, a body of {size} bytes: {:.0} µs (median of 15)",
            request.messages.len(),
            median(times)
        );
    }
}

#[test]
#[ignore = "a measurement; run it in a release build"]
fn decoding_a_recorded_response() {
    for name in ["text", "tool-calls", "switch-2"] {
        let stream = load(name).response;
        let request = scenario::text();
        let events = decode_whole(&request, &stream).len();
        let times: Vec<f64> = (0..200)
            .map(|_| {
                let started = Instant::now();
                let decoded = decode_whole(&request, &stream);
                let took = started.elapsed().as_secs_f64() * 1e6;
                assert_eq!(decoded.len(), events);
                took
            })
            .collect();
        let micros = median(times);
        println!(
            "decode: {name}, {} bytes, {events} events: {micros:.1} µs, {:.0} MB/s (median of 200)",
            stream.len(),
            stream.len() as f64 / micros
        );
    }
}

/// A response stream made from the recorded `text` response: its start,
/// then `pieces` text deltas of an ordinary sentence each, then a tool
/// call whose arguments are one JSON object of about `argument_bytes`
/// bytes, sent in pieces of 200 bytes, as a model writing a large file
/// sends them, then the recorded end.
fn long_stream(pieces: usize, argument_bytes: usize) -> Vec<u8> {
    let mut content = String::new();
    while content.len() < argument_bytes {
        content.push_str("fn f() { let s = \"x\"; }\n");
    }
    stream_writing(pieces, &content)
}

/// [`long_stream`] with the file the call writes given: the text of the
/// call's `content` argument.
fn stream_writing(pieces: usize, content: &str) -> Vec<u8> {
    let recorded = String::from_utf8(load("text").response).expect("the fixture is text");
    let events: Vec<&str> = recorded
        .split("\n\n")
        .filter(|event| !event.trim().is_empty())
        .collect();
    let mut stream = String::new();
    let mut push = |event: &str| {
        stream.push_str(event);
        stream.push_str("\n\n");
    };
    // The recorded start: the message and the text block.
    for event in events
        .iter()
        .take_while(|event| !event.contains("text_delta"))
    {
        push(event);
    }
    for piece in 0..pieces {
        push(&format!(
            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\
             \"delta\":{{\"type\":\"text_delta\",\"text\":\"Sentence {piece} of a long answer, \
             with \\\"quotes\\\" and a café.\\n\"}}}}"
        ));
    }
    push("event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}");
    push(
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\
         \"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_perf\",\"name\":\"write\",\
         \"input\":{}}}",
    );
    // `{"path": "big.rs", "content": "…"}`: the content escaped once as
    // JSON, and once more inside the string `partial_json` is.
    let arguments = serde_json::json!({"path": "big.rs", "content": content}).to_string();
    let mut start = 0;
    while start < arguments.len() {
        let mut end = (start + 200).min(arguments.len());
        // Real code is not all ASCII: a piece ends between characters.
        while !arguments.is_char_boundary(end) {
            end += 1;
        }
        let piece = serde_json::to_string(&arguments[start..end]).expect("text");
        push(&format!(
            "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":1,\
             \"delta\":{{\"type\":\"input_json_delta\",\"partial_json\":{piece}}}}}"
        ));
        start = end;
    }
    push("event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":1}");
    // The recorded end: the stop reason, the usage and the message's end.
    for event in events
        .iter()
        .skip_while(|event| !event.contains("message_delta"))
    {
        push(event);
    }
    stream.into_bytes()
}

/// Decoding a long streamed answer and a large tool call through the SSE
/// parser and the Anthropic decoder: what every byte of a response goes
/// through, and JSON decoding, which the study of the binary's size did not
/// measure at the optimisation levels it compared. 20,000 text deltas and a
/// call with 1 MB of arguments in 200-byte pieces, in one stream of about
/// 5.7 MB; the median of 30 decodes. Recorded in
/// `docs/measurements/binary-size.md`. A measurement:
///
/// ```text
/// gh workflow run try --ref <branch> -f os=windows -f command="perf -p cineris-llm"
/// cargo test --release -p cineris-llm --test it perf_decoding -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement; run it in a release build"]
fn perf_decoding_a_long_answer_and_a_large_tool_call() {
    let stream = long_stream(20_000, 1 << 20);
    let request = scenario::text();
    let decoded = decode_whole(&request, &stream);
    let blocks = decoded
        .iter()
        .filter(|event| matches!(event, cineris_llm::ModelEvent::BlockStart { .. }))
        .count();
    assert_eq!(blocks, 2, "the text block and the call");
    let events = decoded.len();
    let times: Vec<f64> = (0..30)
        .map(|_| {
            let started = Instant::now();
            let decoded = decode_whole(&request, &stream);
            let took = started.elapsed().as_secs_f64() * 1e3;
            assert_eq!(decoded.len(), events);
            took
        })
        .collect();
    let millis = median(times);
    println!(
        "decode: a long answer and a large call, {} bytes, {events} events: {millis:.2} ms, \
         {:.0} MB/s (median of 30)",
        stream.len(),
        stream.len() as f64 / millis / 1e3
    );
    cineris_testkit::numbers::report("llm.decode_long_answer_and_large_call", millis, "ms");
}

/// The first `bytes` of real Rust from the pinned corpus (`cargo xtask corpus
/// fetch`): whole files of 4 to 60 KB, in name order, one in a hundred, so
/// that they come from many crates, joined as a file that a model writes.
#[expect(
    clippy::disallowed_methods,
    reason = "a measurement reads the corpus it measures on"
)]
fn real_rust(bytes: usize) -> String {
    fn walk(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                walk(&path, found);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && entry
                    .metadata()
                    .is_ok_and(|meta| (4_000..=60_000).contains(&meta.len()))
            {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&cineris_testkit::corpus::real_code(), &mut found);
    let mut text = String::new();
    for path in found.iter().step_by(100) {
        if let Ok(file) = std::fs::read_to_string(path) {
            text.push_str(&file);
        }
        if text.len() >= bytes {
            break;
        }
    }
    assert!(
        text.len() >= bytes,
        "only {} bytes of real code",
        text.len()
    );
    text
}

/// Decoding a large tool call whose argument is real code, which is not the
/// repeated line of the test above: real code has quotes, backslashes,
/// non-ASCII text and long runs of white space, each of which JSON escapes
/// and the decoder reads back twice (once for the event's JSON, once for the
/// argument inside it). 1 MB of arguments in 200-byte pieces, and 2,000
/// text deltas; the median of 30 decodes. A measurement, **held to the
/// budget** of `docs/performance-budgets.md` (the owner's decision of
/// 2026-11-16): at least [`DECODE_BUDGET_MB_S`] of the stream the decoder
/// reads, a hosted runner's figure, since the test cannot tell a desk (100
/// MB/s) from a runner:
///
/// ```text
/// gh workflow run try --ref <branch> -f os=windows -f command="perf -p cineris-llm real_code"
/// ```
#[test]
#[ignore = "a measurement; run it in a release build"]
fn perf_decoding_a_large_tool_call_of_real_code() {
    let content = real_rust(1 << 20);
    let stream = stream_writing(2_000, &content);
    let request = scenario::text();
    let events = decode_whole(&request, &stream).len();
    let times: Vec<f64> = (0..30)
        .map(|_| {
            let started = Instant::now();
            let decoded = decode_whole(&request, &stream);
            let took = started.elapsed().as_secs_f64() * 1e3;
            assert_eq!(decoded.len(), events);
            took
        })
        .collect();
    let millis = median(times);
    println!(
        "decode: a large call of real code, {} bytes, {events} events: {millis:.2} ms, \
         {:.0} MB/s (median of 30)",
        stream.len(),
        stream.len() as f64 / millis / 1e3
    );
    cineris_testkit::numbers::report("llm.decode_large_call_real_code", millis, "ms");
    let rate = stream.len() as f64 / millis / 1e3;
    cineris_testkit::numbers::report("llm.decode_large_call_real_code_rate", rate, "MB/s");
    assert!(
        rate >= DECODE_BUDGET_MB_S,
        "decoding {rate:.0} MB/s of the stream, under the budget of {DECODE_BUDGET_MB_S} MB/s"
    );
}

/// Decoding a large tool call of real code, in megabytes a second of the
/// stream the decoder reads: the hosted runner's budget of
/// `docs/performance-budgets.md` (they measured 139 MB/s on Linux and 121
/// on Windows).
const DECODE_BUDGET_MB_S: f64 = 80.0;
