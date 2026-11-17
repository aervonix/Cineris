//! What the fuzz targets check (`docs/testing-and-ci.md`, "Not blocking").
//!
//! Each target is a function from bytes to nothing, which panics when a
//! property does not hold. The fuzzer makes the bytes and follows coverage
//! to find the ones that reach new code.
//!
//! The input of a stream's target is read the same way by each: its first
//! two bytes decide where the rest is cut into the pieces a network would
//! deliver, and the rest is the stream. So the fuzzer explores the cuts as
//! well as the content. The extractors' target reads its input as a file.
//! The RPC server's target reads its input as what a client sends, whole,
//! so that a recorded line is an input as it is; where it is cut comes from
//! a hash of it.
//!
//! | Target | The properties |
//! |---|---|
//! | [`sse_parser`] | No input makes the parser panic. The same bytes give the same events, or the same error, however they are cut. |
//! | [`anthropic_decoder`] | No input makes the decoder panic. The same bytes give the same events however they are cut. A response ends at most once, and nothing follows its end. A block ends only after it started. Every block the decoder stores is one JSON object, so it can be sent back. |
//! | [`openai_chat_decoder`] | The same properties as the other decoder, for servers with each kind of flag: reasoning under a named member, reasoning in tags inside the text, a model that is asked for a speed tier and one that is not. |
//! | [`extractors`] | No file makes an outline extractor panic, in any language, by the fallback, or as a C or C++ header or source file. Every outline keeps its caps: its entries, and each entry's name, head and doc. |
//! | [`rpc_decoder`] | No bytes from a client make the RPC server's reading of them panic: the lines, each line's kind, an operation's parameters, an answer to each kind of request the server puts. The same bytes are read the same however they are cut. A line that is not a message, or is too long, gets an error the server can write, and a client reads that error back; a JSON object with an id the server can read gets it under that id. An operation read is the same operation once a client writes it again and the server reads it, and so is an answer, a request and a response; a float may come back a unit or two in the last place off, as serde_json reads one ([`FLOAT_ULPS`]). |

use std::fmt::Debug;
use std::sync::Arc;

use cortex_base::framing::{self, FrameDecoder};
use cortex_index::outline::{DOC_CHARS, HEAD_CHARS, MAX_ENTRIES, NAME_CHARS};
use cortex_index::{Language, extract};
use cortex_llm::anthropic::{self, AnthropicMessages};
use cortex_llm::openai_chat::{self, OpenAiChat};
use cortex_llm::{
    Catalog, EventSink, FlagValue, ModelEntry, ModelEvent, ModelRequest, Protocol, ProviderInfo,
    SseParser,
};
use cortex_protocol::jsonrpc::{ErrorObject, Message, Request, RequestId, Response, code};
use cortex_protocol::methods::{Op, ServerRequest, ServerResponse};
use cortex_protocol::model::ModelRole;
use cortex_protocol::{ModelId, ProviderId, Speed};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The RPC server's reading of a client's line (`cortex-rpc`'s `wire`
/// module), compiled here from the server's own file. This package does
/// not depend on `cortex-rpc`: that would build the whole core into the
/// fuzzer (the HTTP and TLS stack, the tools, the sandbox), where the file
/// itself needs only `cortex-base`, `cortex-protocol`, `serde` and
/// `serde_json`, which this package depends on too. `cortex-rpc` keeps the
/// file that way, and says so at its head.
#[allow(
    dead_code,
    reason = "the server's writing of events, requests and results is not what is fuzzed"
)]
#[path = "../../crates/cortex-rpc/src/wire.rs"]
mod wire;

use wire::Incoming;

/// Splits an input into the offsets at which the stream is cut, and the
/// stream. The first two bytes seed the sizes of the pieces, from one byte
/// to a few hundred.
fn cuts(data: &[u8]) -> (Vec<usize>, &[u8]) {
    let (seed, stream) = match data {
        [first, second, rest @ ..] => (u32::from(*first) << 8 | u32::from(*second), rest),
        _ => (0, data),
    };
    (seeded_cuts(seed, stream.len()), stream)
}

/// The offsets at which a stream of `len` bytes is cut, from a seed of
/// sixteen bits: the pieces are from one byte to a few hundred.
fn seeded_cuts(seed: u32, len: usize) -> Vec<usize> {
    let mut state = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut cuts = Vec::new();
    let mut at = 0;
    // The top bits of the seed choose how large pieces may be.
    let largest = [1, 2, 7, 64, 400][(seed >> 13) as usize % 5];
    while at < len {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        at += 1 + (state >> 16) as usize % largest;
        cuts.push(at.min(len));
    }
    cuts
}

/// A seed of sixteen bits for the cuts of an input that is a stream as it
/// is: a hash of its bytes (FNV-1a), so the same input is always cut the
/// same way, and a change anywhere in it cuts it anew.
fn hashed_seed(stream: &[u8]) -> u32 {
    let hash = stream.iter().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    (hash ^ hash >> 16) & 0xffff
}

/// The pieces of `stream` that `cuts` make.
fn pieces<'a>(stream: &'a [u8], cuts: &'a [usize]) -> impl Iterator<Item = &'a [u8]> {
    let mut from = 0;
    cuts.iter()
        .copied()
        .chain([stream.len()])
        .filter_map(move |end| {
            let end = end.min(stream.len());
            (end > from).then(|| {
                let piece = &stream[from..end];
                from = end;
                piece
            })
        })
}

fn parse(stream: &[u8], cuts: &[usize]) -> Result<Vec<(String, String)>, String> {
    let mut parser = SseParser::new();
    let mut events = Vec::new();
    for piece in pieces(stream, cuts) {
        parser.push(piece);
        loop {
            match parser.next_event() {
                None => break,
                Some(Err(error)) => return Err(error.to_string()),
                Some(Ok(event)) => events.push((event.event.to_owned(), event.data.to_owned())),
            }
        }
    }
    Ok(events)
}

/// The SSE parser: the same events however the bytes are cut.
///
/// # Panics
///
/// When the property does not hold: that is how a fuzz target reports.
pub fn sse_parser(data: &[u8]) {
    let (cuts, stream) = cuts(data);
    let whole = parse(stream, &[]);
    let cut = parse(stream, &cuts);
    assert_eq!(whole, cut, "the cuts changed what was parsed");
}

fn model(provider: &str, id: &str) -> Arc<ModelEntry> {
    let catalog = Catalog::builtin().expect("the built-in catalog loads");
    let entry = catalog
        .get(&ProviderId::new(provider), &ModelId::new(id))
        .expect("the built-in catalog has the model");
    Arc::clone(entry)
}

fn decode(
    protocol: &dyn Protocol,
    provider: &ProviderInfo,
    request: &ModelRequest,
    stream: &[u8],
    cuts: &[usize],
) -> Result<Vec<ModelEvent>, String> {
    let mut decoder = protocol.decoder(request, provider);
    let mut parser = SseParser::new();
    let mut sink = EventSink::sent_at(request.speed.clone());
    for piece in pieces(stream, cuts) {
        parser.push(piece);
        loop {
            match parser.next_event() {
                None => break,
                Some(Err(error)) => return Err(error.to_string()),
                Some(Ok(event)) => decoder
                    .feed(event, &mut sink)
                    .map_err(|error| error.message)?,
            }
        }
    }
    decoder.finish(&mut sink).map_err(|error| error.message)?;
    Ok(sink.take())
}

/// The decoder of the `anthropic-messages` protocol.
///
/// # Panics
///
/// When a property does not hold: that is how a fuzz target reports.
pub fn anthropic_decoder(data: &[u8]) {
    let (cuts, stream) = cuts(data);
    // Half the inputs are decoded as the response to a request at fast
    // speed, where an error in the stream can belong to the tier.
    let fast = data.first().is_some_and(|byte| byte & 1 == 1);
    let mut request =
        ModelRequest::new(model(anthropic::PROVIDER, "claude-opus-5-5"), ModelRole::Main);
    if fast {
        request.speed = Speed::Fast;
    }

    let provider = anthropic::provider();
    let whole = decode(&AnthropicMessages, &provider, &request, stream, &[]);
    let cut = decode(&AnthropicMessages, &provider, &request, stream, &cuts);
    assert_eq!(whole, cut, "the cuts changed what was decoded");

    let Ok(events) = whole else { return };
    well_formed(&events);
}

/// What every decoder's events must be, whatever the bytes were: a response
/// ends at most once and nothing follows its end, a block starts once and
/// holds nothing before it started, and every block that is stored is one
/// JSON object, so that it can be sent back.
fn well_formed(events: &[ModelEvent]) {
    let endings = events.iter().filter(|event| event.is_terminal()).count();
    assert!(endings <= 1, "a response ended {endings} times");
    if endings == 1 {
        assert!(
            events.last().is_some_and(ModelEvent::is_terminal),
            "an event follows the end of the response"
        );
    }
    let mut started = Vec::new();
    for event in events {
        match event {
            ModelEvent::BlockStart { index, .. } => {
                assert!(!started.contains(index), "the block {index} started twice");
                started.push(*index);
            }
            ModelEvent::BlockEnd { index, raw } => {
                assert!(started.contains(index), "the block {index} ended before it started");
                let stored: Result<serde_json::Value, _> = serde_json::from_str(raw.as_str());
                assert!(
                    stored.is_ok_and(|value| value.is_object()),
                    "a stored block is not one JSON object: {raw}"
                );
            }
            ModelEvent::TextDelta { index, .. }
            | ModelEvent::ReasoningDelta { index, .. }
            | ModelEvent::ToolCallStart { index, .. }
            | ModelEvent::ToolArgsDelta { index, .. } => {
                assert!(started.contains(index), "the block {index} has content before it started");
            }
            _ => {}
        }
    }
}

/// The decoder of the `openai-chat` protocol.
///
/// The first byte of the input chooses the server: which member reasoning
/// is read from, whether it is looked for in tags inside the text, and
/// whether the model is one that is asked for a speed tier, at fast speed
/// or at standard.
///
/// # Panics
///
/// When a property does not hold: that is how a fuzz target reports.
pub fn openai_chat_decoder(data: &[u8]) {
    let (cuts, stream) = cuts(data);
    let choice = data.first().copied().unwrap_or(0);
    let mut provider = openai_chat::providers::builtin(openai_chat::providers::OPENROUTER)
        .expect("the provider is built in")
        .info;
    // A model with tiers that are asked for by name, or a model with none.
    let id = if choice & 1 == 0 {
        "openai/gpt-6-luna"
    } else {
        "z-ai/glm-5.3-flash"
    };
    let mut request =
        ModelRequest::new(model(openai_chat::providers::OPENROUTER, id), ModelRole::Main);
    if choice & 0b11 == 0b10 {
        request.speed = Speed::Fast;
    }
    if choice & 0b100 != 0 {
        provider
            .flags
            .set(openai_chat::flag::REASONING_TAGS, FlagValue::Bool(true));
    }
    if choice & 0b1000 != 0 {
        provider.flags.set(
            openai_chat::flag::REASONING_FIELD,
            FlagValue::Text("reasoning".to_owned()),
        );
    }

    let whole = decode(&OpenAiChat, &provider, &request, stream, &[]);
    let cut = decode(&OpenAiChat, &provider, &request, stream, &cuts);
    assert_eq!(whole, cut, "the cuts changed what was decoded");

    let Ok(events) = whole else { return };
    well_formed(&events);
}

/// The outline extractors, on the input as a file: decoded as a pass
/// decodes one (`cortex_index::text::decode`), and outlined in every
/// language, by the fallback, and as a C and a C++ header and source file
/// (a prototype is outlined in a header only).
///
/// # Panics
///
/// When an extractor panics, or an outline passes a cap: more entries than
/// [`MAX_ENTRIES`], an empty name, or a name, a head or a doc longer than
/// [`NAME_CHARS`], [`HEAD_CHARS`] or [`DOC_CHARS`] characters.
pub fn extractors(data: &[u8]) {
    let text = cortex_index::text::decode(data);
    let mut runs: Vec<(Option<Language>, &str)> = Language::ALL
        .into_iter()
        .map(|language| (Some(language), "a"))
        .collect();
    runs.extend([
        (None, "a"),
        (Some(Language::C), "a.c"),
        (Some(Language::C), "a.h"),
        (Some(Language::Cpp), "a.cpp"),
        (Some(Language::Cpp), "a.hpp"),
    ]);
    for (language, file) in runs {
        let outline = extract(language, file, &text);
        assert!(
            outline.entries.len() <= MAX_ENTRIES,
            "{language:?} {file}: {} entries",
            outline.entries.len()
        );
        for entry in &outline.entries {
            assert!(!entry.name.is_empty(), "{language:?} {file}: {entry:?}");
            assert!(
                entry.name.chars().count() <= NAME_CHARS,
                "{language:?} {file}: a name too long: {entry:?}"
            );
            assert!(
                entry.head.chars().count() <= HEAD_CHARS,
                "{language:?} {file}: a head too long: {entry:?}"
            );
            assert!(
                entry
                    .doc
                    .as_ref()
                    .is_none_or(|doc| doc.chars().count() <= DOC_CHARS),
                "{language:?} {file}: a doc too long: {entry:?}"
            );
        }
    }
}

/// The longest line the RPC target's server reads. The server's own limit
/// is 32 MiB (`ServeOptions::max_line`); this one must stay below the
/// longest input `cargo xtask fuzz` lets the fuzzer make (`-max_len`, 8 KiB),
/// so that a line too long is reached as well.
const RPC_LINE_LIMIT: usize = 4096;

/// What the server makes of one thing in a client's stream.
#[derive(Debug, PartialEq)]
enum Read {
    /// A line too long, or a stream that ended within a line: the error the
    /// server answers it with, under no id.
    Frame(ErrorObject),
    /// A whole line, without its end, and what the server reads it as.
    Line(Vec<u8>, Incoming),
}

/// What the server reads from `stream` delivered in the pieces `cuts` make,
/// as a connection reads it: every whole line taken after each piece, and
/// the end of the stream told last.
fn serve(stream: &[u8], cuts: &[usize]) -> Vec<Read> {
    let mut decoder = FrameDecoder::with_max_line_len(RPC_LINE_LIMIT);
    let mut read = Vec::new();
    for piece in pieces(stream, cuts) {
        decoder.push(piece);
        while let Some(line) = decoder.next_line() {
            read.push(match line {
                Ok(line) => Read::Line(line.to_vec(), wire::read(line)),
                Err(error) => Read::Frame(error.to_error_object()),
            });
        }
    }
    if let Err(error) = decoder.finish() {
        read.push(Read::Frame(error.to_error_object()));
    }
    read
}

/// The RPC server's decoder: what `cortex-rpc` does with the bytes a client
/// sends, before the core is asked anything.
///
/// The input is the stream as it is, read whole and read cut where a hash
/// of it says, by the framing's decoder with a short line limit. Each line
/// is read as the server reads it (`wire::read`). A request's operation is
/// read from its method and parameters (`Op::from_wire`), as the server
/// reads every request, `initialize` included. An answer's result is read
/// as the answer to each kind of request the server puts
/// (`ServerResponse::from_wire`), since the server reads it against the
/// method of the request its id names.
///
/// # Panics
///
/// When a property does not hold: that is how a fuzz target reports.
pub fn rpc_decoder(data: &[u8]) {
    let cuts = seeded_cuts(hashed_seed(data), data.len());
    let whole = serve(data, &[]);
    let cut = serve(data, &cuts);
    assert_eq!(whole, cut, "the cuts changed what the server read");

    for read in whole {
        match read {
            Read::Frame(error) => answers_back(None, error),
            Read::Line(line, Incoming::Bad { id, error }) => {
                assert!(
                    error.code == code::PARSE_ERROR || error.code == code::INVALID_REQUEST,
                    "a line that is not a message is answered with the code {}",
                    error.code
                );
                // A JSON object with an id the server can read is answered
                // under that id, so that its client can match the error.
                if let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(&line) {
                    let readable = object
                        .get("id")
                        .and_then(|id| RequestId::deserialize(id).ok());
                    assert_eq!(id, readable, "a line is not answered under its id");
                }
                answers_back(id, error);
            }
            // None is defined from a client; the server ignores each.
            Read::Line(_, Incoming::Notification) => {}
            Read::Line(_, Incoming::Request(request)) => request_reads_back(request),
            Read::Line(_, Incoming::Response(response)) => answer_reads_back(response),
        }
    }
}

/// `message` as the one line its writer makes, without the line's end.
fn written(message: &Message) -> Vec<u8> {
    let mut line = Vec::new();
    framing::encode(message, &mut line).expect("a message that was read is written again");
    assert_eq!(
        line.pop(),
        Some(b'\n'),
        "a message is not written as a line"
    );
    line
}

/// `value` as JSON.
fn json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("what was read is written")
}

/// The most two floats may be apart, in units in the last place, and be
/// taken for the same float read back. serde_json, as the harness builds it
/// (without its `float_roundtrip` feature), reads a float only to within a
/// unit or two of the nearest, so a float written and read again may come
/// back that far off: `1.0715660391465826e-75` is read as `…825e-75`, which
/// is written so and read again as `…823e-75`. Over 20 million random
/// floats, normal and subnormal, none came back more than two units off.
const FLOAT_ULPS: u64 = 4;

/// That `again`, which `first` was written as and read back from, is the
/// same as `first`, but for floats [`FLOAT_ULPS`] apart at most. When the
/// two differ, their JSON (`as_json`) must differ, and in such floats
/// alone: two values that differ where their JSON is the same differ in
/// something the writing left out.
fn read_back_the_same<T: PartialEq + Debug>(
    first: &T,
    again: &T,
    as_json: impl Fn(&T) -> Value,
    what: &str,
) {
    if first == again {
        return;
    }
    let (first_json, again_json) = (as_json(first), as_json(again));
    assert!(
        first_json != again_json && same_but_floats(&first_json, &again_json),
        "{what} differs once written and read again:\n  first: {first:?}\n  again: {again:?}"
    );
}

/// Whether `a` and `b` are the same JSON, but for floats [`FLOAT_ULPS`]
/// apart at most.
fn same_but_floats(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) if x.is_f64() && y.is_f64() => {
            let (Some(x), Some(y)) = (x.as_f64(), y.as_f64()) else {
                return false;
            };
            x == y
                || (x.is_sign_negative() == y.is_sign_negative()
                    && x.to_bits().abs_diff(y.to_bits()) <= FLOAT_ULPS)
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_but_floats(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same_but_floats(a, b)))
        }
        _ => a == b,
    }
}

/// The error the server answers what it could not read with, written as
/// the server writes it (`wire::error`): one line, which a client reads
/// back as a response under the same id, with the same error.
fn answers_back(id: Option<RequestId>, error: ErrorObject) {
    let mut line = Vec::new();
    wire::error(id.clone(), error.clone(), &mut line).expect("the server writes its error");
    assert_eq!(line.pop(), Some(b'\n'), "an error is not written as a line");
    let back: Message = framing::decode(&line).expect("a client reads the server's error");
    assert_eq!(
        back,
        Message::Response(Response {
            id,
            result: Err(error)
        }),
        "the server's error differs once read back"
    );
}

/// A request: the operation the server reads from it, or the error it
/// answers it with. An operation is the same once a client writes it again
/// and the server reads it, and so is the request itself.
fn request_reads_back(request: Request) {
    let Incoming::Request(again) = wire::read(&written(&Message::Request(request.clone()))) else {
        panic!("a request written again is not read as one");
    };
    read_back_the_same(&request, &again, json, "a request");
    let Request { id, method, params } = request;
    match Op::from_wire(&method, params) {
        Ok(op) => {
            assert_eq!(op.method(), method);
            let sent = op
                .to_request(id.clone())
                .expect("an operation's parameters are written");
            let Incoming::Request(read) = wire::read(&written(&Message::Request(sent))) else {
                panic!("an operation written is not read as a request");
            };
            assert_eq!(read.id, id);
            let again = Op::from_wire(&read.method, read.params)
                .expect("an operation written is read again");
            assert_eq!(again.method(), op.method());
            read_back_the_same(
                &op,
                &again,
                |op| op.params().expect("an operation's parameters are written"),
                "an operation",
            );
        }
        Err(error) => {
            let expected = if Op::METHODS.contains(&method.as_str()) {
                code::INVALID_PARAMS
            } else {
                code::METHOD_NOT_FOUND
            };
            let error = error.to_error_object();
            assert_eq!(error.code, expected, "`{method}`: {}", error.message);
            answers_back(Some(id), error);
        }
    }
}

/// An answer to a request the server put. The server reads it against the
/// method of the request its id names; here it is read against each. An
/// answer is the same once written again and read, and so is the response.
fn answer_reads_back(response: Response) {
    let Incoming::Response(again) = wire::read(&written(&Message::Response(response.clone())))
    else {
        panic!("a response written again is not read as one");
    };
    read_back_the_same(&response, &again, json, "a response");
    // An error in its place declines the request.
    let Response {
        id,
        result: Ok(value),
    } = response
    else {
        return;
    };
    for method in ServerRequest::METHODS {
        let Ok(answer) = ServerResponse::from_wire(method, value.clone()) else {
            continue;
        };
        assert_eq!(answer.method(), *method);
        let result = answer.result().expect("an answer is written");
        let sent = Message::Response(Response {
            id: id.clone(),
            result: Ok(result),
        });
        let Incoming::Response(Response {
            result: Ok(read), ..
        }) = wire::read(&written(&sent))
        else {
            panic!("an answer written is not read as one");
        };
        let again =
            ServerResponse::from_wire(method, read).expect("an answer written is read again");
        read_back_the_same(
            &answer,
            &again,
            |answer| answer.result().expect("an answer is written"),
            &format!("an answer to `{method}`"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extractors_target_takes_code_and_nonsense() {
        extractors(b"fn a() {}\nclass B:\n    def c(self): pass\n# D\n");
        extractors(b"\xEF\xBB\xBFextern \"C\" template<> int f() {\r\n}\r\n");
        for nonsense in [&b""[..], b"\xff\xfe{", b"<<<A\n/*", b"x = /[/\n\"\"\"", b"`"] {
            extractors(nonsense);
        }
    }

    #[test]
    fn the_cuts_cover_the_stream_and_come_from_the_seed() {
        let data = [0x12, 0x34, b'a', b'b', b'c', b'd', b'e', b'f', b'g'];
        let (cuts, stream) = cuts(&data);
        assert_eq!(stream, b"abcdefg");
        assert_eq!(cuts.last(), Some(&stream.len()));
        assert!(cuts.windows(2).all(|pair| pair[0] < pair[1]));
        let joined: Vec<u8> = pieces(stream, &cuts).flatten().copied().collect();
        assert_eq!(joined, stream);
        // The same input, the same cuts.
        assert_eq!(super::cuts(&data).0, cuts);
        assert_eq!(super::cuts(&[]).1, b"");
        assert_eq!(super::cuts(&[7]).1, [7]);
    }

    #[test]
    fn both_targets_take_a_real_stream_and_nonsense() {
        let stream = b"\x00\x05event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"model\":\"x\",\"usage\":{\"input_tokens\":1}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
        sse_parser(stream);
        anthropic_decoder(stream);
        for nonsense in [&b""[..], b"\xff", b"\x01\x02data: {\n\n", b"zz\r\r\n\ndata\n\n: x\n"] {
            sse_parser(nonsense);
            anthropic_decoder(nonsense);
            openai_chat_decoder(nonsense);
        }
        // A stream of the other dialect, for each choice of server: the
        // first byte of the input chooses the server.
        let chat = b"data: {\"id\":\"c\",\"model\":\"m\",\"service_tier\":\"default\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"<think>hm</think>hi\",\"reasoning\":\"so\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"r\",\"arguments\":\"{}\"}}]},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n";
        for choice in 0..16u8 {
            let mut input = vec![choice, 0x05];
            input.extend_from_slice(chat);
            openai_chat_decoder(&input);
        }
    }

    /// The seeds `cargo xtask fuzz` starts the RPC target from
    /// (`crates/cortex-rpc/fixtures/lines`).
    const RPC_SEEDS: [&[u8]; 8] = [
        include_bytes!("../../crates/cortex-rpc/fixtures/lines/requests.jsonl"),
        include_bytes!("../../crates/cortex-rpc/fixtures/lines/initialize.jsonl"),
        include_bytes!("../../crates/cortex-rpc/fixtures/lines/turn-start.jsonl"),
        include_bytes!("../../crates/cortex-rpc/fixtures/lines/answers.jsonl"),
        include_bytes!("../../crates/cortex-rpc/fixtures/lines/notification.jsonl"),
        include_bytes!("../../crates/cortex-rpc/fixtures/lines/batch.jsonl"),
        include_bytes!("../../crates/cortex-rpc/fixtures/lines/invalid.jsonl"),
        include_bytes!("../../crates/cortex-rpc/fixtures/lines/too-long.jsonl"),
    ];

    #[test]
    fn the_rpc_target_takes_its_seeds_and_nonsense() {
        for seed in RPC_SEEDS {
            rpc_decoder(seed);
        }
        let mut long = vec![b'x'; RPC_LINE_LIMIT + 2];
        rpc_decoder(&long);
        long.extend_from_slice(b"\r\n{\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n");
        rpc_decoder(&long);
        let deep = format!("{}{}\n", "[".repeat(200), "]".repeat(200));
        for nonsense in [
            &b""[..],
            b"\n\r\n",
            b"\xff\xfe\n",
            b"{\"jsonrpc\":\"2.0\",\"id\":1e999,\"method\":\"x\"}\n",
            b"{\"jsonrpc\":\"2.0\",\"id\":18446744073709551615,\"result\":1}\n",
            b"{\"jsonrpc\":\"2.0\",\"id\":[1],\"method\":\"shutdown\"}\n",
            b"{\"jsonrpc\":\"1.0\",\"id\":\"a\",\"method\":\"shutdown\"}\n",
            b"{\"jsonrpc\":\"2.0\",\"id\":\"a\",\"result\":{\"choice\":\"allow_once\",\"x\":2.5e-300}}\n",
            deep.as_bytes(),
        ] {
            rpc_decoder(nonsense);
        }
    }

    /// A float that serde_json reads back a unit or two off, in each place
    /// the target writes and reads again: a request's parameters, an
    /// operation's, a response's result and an error's data. Each of these
    /// failed the target before it took that (found by its review).
    #[test]
    fn a_float_read_back_a_unit_off_is_taken_for_the_same() {
        let float = "1.0715660391465826e-75";
        for line in [
            format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"x\",\"params\":{float}}}\n"),
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":{{\"workspace\":\"/w\",\"config\":{{\"x\":{float}}}}}}}\n"
            ),
            format!("{{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{float}}}\n"),
            format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":4,\"error\":{{\"code\":1,\"message\":\"m\",\"data\":[{float}]}}}}\n"
            ),
            "{\"jsonrpc\":\"2.0\",\"id\":5,\"result\":1111111111111111111111111111111111}\n"
                .to_owned(),
        ] {
            rpc_decoder(line.as_bytes());
        }
    }

    #[test]
    fn same_but_floats_takes_a_few_units_and_nothing_else() {
        let value = |text: &str| -> Value { serde_json::from_str(text).unwrap() };
        let at = |bits: u64| Value::from(f64::from_bits(bits));
        let bits = 1.0715660391465826e-75_f64.to_bits();
        assert!(same_but_floats(&at(bits), &at(bits + FLOAT_ULPS)));
        assert!(same_but_floats(
            &Value::Array(vec![at(bits)]),
            &Value::Array(vec![at(bits - 1)])
        ));
        assert!(!same_but_floats(&at(bits), &at(bits + FLOAT_ULPS + 1)));
        assert!(same_but_floats(&value("[0.0]"), &value("[-0.0]")));
        assert!(!same_but_floats(&value("1.0"), &value("-1.0")));
        assert!(!same_but_floats(&value("1.0"), &value("1")));
        assert!(!same_but_floats(
            &value("{\"a\":1.5}"),
            &value("{\"b\":1.5}")
        ));
        assert!(!same_but_floats(&value("[1.5]"), &value("[1.5,1.5]")));
        assert!(!same_but_floats(&value("\"a\""), &value("\"b\"")));
    }

    /// Two values that differ where their JSON is the same differ in
    /// something the writing left out: not a float a unit off.
    #[test]
    #[should_panic(expected = "a value differs once written and read again")]
    fn a_difference_the_json_does_not_show_fails() {
        read_back_the_same(&1_u8, &2_u8, |_| Value::Null, "a value");
    }

    #[test]
    #[should_panic(expected = "a value differs once written and read again")]
    fn a_difference_other_than_a_float_fails() {
        read_back_the_same(&1_u8, &2_u8, |n| Value::from(*n), "a value");
    }

    /// The requests seed is the golden transcript's client, so it holds
    /// every operation of the protocol, each read as one.
    #[test]
    fn the_rpc_requests_seed_holds_every_operation() {
        let mut methods = Vec::new();
        for read in serve(RPC_SEEDS[0], &[]) {
            let Read::Line(_, Incoming::Request(request)) = read else {
                panic!("not a request: {read:?}");
            };
            let op = Op::from_wire(&request.method, request.params).expect("an operation");
            methods.push(op.method());
        }
        methods.sort_unstable();
        methods.dedup();
        let mut all = Op::METHODS.to_vec();
        all.sort_unstable();
        assert_eq!(methods, all);
    }

    /// The invalid seed holds one line of each kind the server refuses, and
    /// the too-long seed a line past the target's limit.
    #[test]
    fn the_rpc_refused_seeds_are_refused_line_by_line() {
        let codes = |seed: &[u8]| -> Vec<(Option<RequestId>, i64)> {
            serve(seed, &[])
                .into_iter()
                .map(|read| match read {
                    Read::Frame(error) => (None, error.code),
                    Read::Line(_, Incoming::Bad { id, error }) => (id, error.code),
                    Read::Line(_, Incoming::Request(request)) => (
                        Some(request.id),
                        Op::from_wire(&request.method, request.params)
                            .map_or_else(|error| error.to_error_object().code, |_| 0),
                    ),
                    other => panic!("not refused: {other:?}"),
                })
                .collect()
        };
        assert_eq!(
            codes(RPC_SEEDS[6]),
            [
                (None, code::PARSE_ERROR),
                (Some(RequestId::Number(7)), code::INVALID_REQUEST),
                (None, code::INVALID_REQUEST),
                (Some(RequestId::Number(8)), code::METHOD_NOT_FOUND),
                (Some(RequestId::Number(9)), code::INVALID_PARAMS),
                (Some(RequestId::from("srv-6")), code::INVALID_REQUEST),
                // The last line, cut off by the end of the stream.
                (None, code::PARSE_ERROR),
            ]
        );
        // The line too long, and a `shutdown` read after it (0: taken).
        assert_eq!(
            codes(RPC_SEEDS[7]),
            [
                (None, code::INVALID_REQUEST),
                (Some(RequestId::Number(6)), 0)
            ]
        );
    }

    #[test]
    fn an_rpc_input_is_cut_by_its_own_hash() {
        let data = b"{\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n";
        let seed = hashed_seed(data);
        assert!(seed <= 0xffff);
        assert_eq!(hashed_seed(data), seed);
        assert_ne!(hashed_seed(b"{}\n"), seed);
        let cuts = seeded_cuts(seed, data.len());
        assert_eq!(cuts.last(), Some(&data.len()));
        let joined: Vec<u8> = pieces(data, &cuts).flatten().copied().collect();
        assert_eq!(joined, data);
    }
}
