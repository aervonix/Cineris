//! Measurements, run by hand in a release build:
//!
//! ```text
//! cargo test --release -p cortex-llm --test it -- --ignored anthropic::measure --nocapture
//! ```
//!
//! What they measure is what the adapter adds to every request: building
//! the body from a long history, and decoding a response. The numbers are
//! in `docs/performance-budgets.md`.

#![allow(clippy::print_stdout, reason = "a measurement prints what it measured")]

use std::time::Instant;

use cortex_llm::anthropic::AnthropicMessages;
use cortex_llm::{ModelRequest, Protocol};

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
    let mut content = String::new();
    while content.len() < argument_bytes {
        content.push_str("fn f() { let s = \"x\"; }\n");
    }
    let arguments = serde_json::json!({"path": "big.rs", "content": content}).to_string();
    let mut start = 0;
    while start < arguments.len() {
        let end = (start + 200).min(arguments.len());
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
/// gh workflow run try --ref <branch> -f os=windows -f command="perf -p cortex-llm"
/// cargo test --release -p cortex-llm --test it perf_decoding -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement; run it in a release build"]
fn perf_decoding_a_long_answer_and_a_large_tool_call() {
    let stream = long_stream(20_000, 1 << 20);
    let request = scenario::text();
    let decoded = decode_whole(&request, &stream);
    let blocks = decoded
        .iter()
        .filter(|event| matches!(event, cortex_llm::ModelEvent::BlockStart { .. }))
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
}
