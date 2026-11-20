//! Integration tests (tier 2): the provider client as a whole.
//!
//! Three modules put the client together from the test protocol, the
//! scripted transport, the fake clock and the fake timer, so every wait is
//! a number the test reads back and no test takes time. [`transport`] is
//! the one that uses the real HTTP client, against the scripted SSE server
//! on the loopback interface, and [`fetch`] does the same for the fetch
//! client behind extensions' `cineris.http.fetch`, with a resolver that
//! answers what the test says.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "an integration test fails by panicking"
)]

mod anthropic;
mod client;
#[cfg(feature = "http")]
mod fetch;
#[cfg(feature = "http")]
mod live;
mod openai_chat;
mod retry;
mod speed;
mod toolbox;
#[cfg(feature = "http")]
mod transport;

use std::sync::Arc;
use std::time::Duration;

/// Run by the test runner, it passes at once. Started as a child by a test
/// of [`fetch`], in one of that module's roles, it plays that part.
#[cfg(feature = "http")]
#[test]
fn child_entry() {
    fetch::play_a_part();
}

use cineris_base::{CancelToken, ErrorKind, UtcTime};
use cineris_llm::error::ProviderError;
use cineris_llm::{
    ModelClient, ModelEntry, ModelEvent, ModelRequest, ProviderClient, ProviderInfo, RetryInfo,
    RetryPolicy, SpeedMove, Usage,
};
use cineris_protocol::Speed;
use cineris_testkit::{
    FakeClock, FakeTimer, Reply, ScriptedTransport, SeededRng, TestProtocol, block_on, fixtures,
};
use serde_json::{Value, json};

/// Where every test's clock starts.
const START: &str = "2026-11-12T10:00:00Z";

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

/// A client over a scripted transport, with a fake clock and a timer that
/// moves it.
struct Harness {
    clock: FakeClock,
    timer: FakeTimer,
    transport: ScriptedTransport,
    client: ProviderClient,
}

impl Harness {
    /// A client with the default policy whose waits finish at once.
    fn new(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self::with(replies, RetryPolicy::default(), true)
    }

    /// A client with this policy.
    fn with_policy(replies: impl IntoIterator<Item = Reply>, policy: RetryPolicy) -> Self {
        Self::with(replies, policy, true)
    }

    /// A client whose waits finish only when the test moves the clock.
    fn manual(replies: impl IntoIterator<Item = Reply>) -> Self {
        Self::with(replies, RetryPolicy::default(), false)
    }

    fn with(replies: impl IntoIterator<Item = Reply>, policy: RetryPolicy, auto: bool) -> Self {
        let clock = FakeClock::starting_at(START).unwrap();
        let timer = if auto {
            FakeTimer::auto(&clock)
        } else {
            FakeTimer::manual(&clock)
        };
        let transport = ScriptedTransport::new(replies);
        let client = ProviderClient::builder(
            Arc::new(TestProtocol),
            provider(),
            Arc::new(transport.clone()),
            Arc::new(clock.clone()),
            Arc::new(timer.clone()),
        )
        .policy(policy)
        .rng(Box::new(SeededRng::new(42)))
        .build();
        Self {
            clock,
            timer,
            transport,
            client,
        }
    }

    /// Sends a request at standard speed and reads the response to its end.
    fn run(&self) -> Vec<ModelEvent> {
        self.run_at(Speed::Standard)
    }

    /// Sends a request that asks for `speed` and reads the response to its
    /// end.
    fn run_at(&self, speed: Speed) -> Vec<ModelEvent> {
        self.run_request(request_at(fixtures::model(), speed))
    }

    /// The response's events. The client's word that the provider answered,
    /// which is timing and comes before each attempt's response, is counted
    /// by [`answered`](Self::answered) and left out here.
    fn run_request(&self, request: ModelRequest) -> Vec<ModelEvent> {
        let events = block_on(self.client.stream(request, CancelToken::new()).collect());
        without_answered(events)
    }

    /// The `speed` field of each request sent so far.
    fn speeds_sent(&self) -> Vec<String> {
        self.transport
            .bodies()
            .iter()
            .map(|body| body["speed"].as_str().unwrap().to_owned())
            .collect()
    }

    fn now(&self) -> UtcTime {
        use cineris_testkit::Clock as _;
        self.clock.now()
    }
}

fn provider() -> ProviderInfo {
    ProviderInfo::new(
        fixtures::PROVIDER,
        fixtures::PROTOCOL,
        "https://test.invalid",
    )
}

fn request_at(model: Arc<ModelEntry>, speed: Speed) -> ModelRequest {
    let mut request = fixtures::request(model);
    request.speed = speed;
    request
}

fn at(time: &str) -> UtcTime {
    UtcTime::parse(time).unwrap()
}

/// A complete, successful response that says "Hello".
fn hello() -> Reply {
    Reply::ok(hello_text(&json!({"input": 10, "output": 2})))
}

fn hello_text(usage: &Value) -> String {
    cineris_testkit::protocol::text_response("Hello", usage)
}

/// A failed response with the test protocol's error body.
fn failure(status: u16, code: &str) -> Reply {
    Reply::status(status).chunk(cineris_testkit::protocol::failure_body(code, "It failed."))
}

/// A response refused for the speed tier's own limit.
fn tier_refusal(status: u16, code: &str) -> Reply {
    Reply::status(status).chunk(cineris_testkit::protocol::tier_failure_body(
        code,
        "The tier is busy.",
    ))
}

/// The stream's next event that is not the client's word that the
/// provider answered (timing, which comes before each attempt's response).
pub(crate) async fn next_event(stream: &mut cineris_llm::ModelStream) -> Option<ModelEvent> {
    loop {
        match stream.next().await {
            Some(ModelEvent::Answered) => {}
            other => return other,
        }
    }
}

/// A stream's events without the client's word that the provider answered.
fn without_answered(events: Vec<ModelEvent>) -> Vec<ModelEvent> {
    events
        .into_iter()
        .filter(|event| !matches!(event, ModelEvent::Answered))
        .collect()
}

/// One name per event, for asserting on the shape of a stream. The client's
/// word that the provider answered is left out: it is timing, and comes
/// before each attempt's response (`answered` counts them).
fn names(events: &[ModelEvent]) -> Vec<&'static str> {
    events
        .iter()
        .filter(|event| !matches!(event, ModelEvent::Answered))
        .map(|event| match event {
            ModelEvent::Answered => "answered",
            ModelEvent::Start { .. } => "start",
            ModelEvent::BlockStart { .. } => "block_start",
            ModelEvent::TextDelta { .. } => "text",
            ModelEvent::ReasoningDelta { .. } => "reasoning",
            ModelEvent::ToolCallStart { .. } => "tool_call",
            ModelEvent::ToolArgsDelta { .. } => "tool_args",
            ModelEvent::BlockEnd { .. } => "block_end",
            ModelEvent::Usage(_) => "usage",
            ModelEvent::Limits(_) => "limits",
            ModelEvent::Retry(_) => "retry",
            ModelEvent::SpeedFallback(_) => "speed_fallback",
            ModelEvent::SpeedCooldown(_) => "speed_cooldown",
            ModelEvent::End { .. } => "end",
            ModelEvent::Error(_) => "error",
        })
        .collect()
}

/// The names of a whole, successful "Hello" response.
const HELLO: [&str; 5] = ["start", "block_start", "text", "block_end", "end"];

fn retries(events: &[ModelEvent]) -> Vec<&RetryInfo> {
    events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::Retry(retry) => Some(retry),
            _ => None,
        })
        .collect()
}

fn fallbacks(events: &[ModelEvent]) -> Vec<&SpeedMove> {
    events
        .iter()
        .filter_map(|event| match event {
            ModelEvent::SpeedFallback(change) => Some(change),
            _ => None,
        })
        .collect()
}

/// The error a stream ended with.
fn final_error(events: &[ModelEvent]) -> &ProviderError {
    match events.last() {
        Some(ModelEvent::Error(error)) => error,
        other => panic!("the stream did not end with an error: {other:?}"),
    }
}

/// The usage a stream ended with.
fn final_usage(events: &[ModelEvent]) -> &Usage {
    match events.last() {
        Some(ModelEvent::End { usage, .. }) => usage,
        other => panic!("the stream did not end with a response: {other:?}"),
    }
}

/// Every stream ends with exactly one end or error, and nothing after it.
fn assert_well_formed(events: &[ModelEvent]) {
    let terminal = events.iter().filter(|event| event.is_terminal()).count();
    assert_eq!(terminal, 1, "{:?}", names(events));
    assert!(events.last().unwrap().is_terminal(), "{:?}", names(events));
}

fn assert_kind(error: &ProviderError, kind: ErrorKind, code: &str) {
    assert_eq!((error.kind, error.code.as_str()), (kind, code), "{error:?}");
}
