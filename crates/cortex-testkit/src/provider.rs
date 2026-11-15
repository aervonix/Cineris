//! The scripted provider: a model that says what a test tells it to
//! (`docs/testing-and-ci.md`, "Deterministic agent tests without a
//! network").
//!
//! [`ScriptedProvider`] implements [`ModelClient`] directly, with nothing
//! below it: no protocol, no transport, no retry layer. It answers each
//! request with the next [`Response`] of its script and remembers the
//! requests, so a test of the agent loop can assert on what the loop sent.
//!
//! A [`Response`] is built from the things a model does (text, reasoning,
//! tool calls, usage, an end) and the things that go wrong (an error
//! part-way, a delay, a stall, a stream that just stops). It hands over one
//! event at a time, and only when the reader asks, so the order in which a
//! test sees things is the order of the script.
//!
//! # The output limit
//!
//! A real provider stops a response at the request's `max_output`. So does
//! this one, unless it is told not to
//! ([`without_output_limit`](ScriptedProvider::without_output_limit)): a
//! script that says more than the request allows is cut where the limit
//! falls, and ends as a provider ends it, with [`StopReason::MaxOutput`] and
//! a usage whose output is the limit. Output is counted as the loop counts a
//! request before a provider has counted it, at [`BYTES_PER_OUTPUT_TOKEN`]
//! bytes to the token, over the text, the reasoning and the arguments of the
//! tool calls. The limit is the request's `max_output`, or the model's own
//! most when the request sets none, and never more than the model's most.
//! A block that was open when the limit fell is closed with what had
//! arrived of it, as the real decoders close it; a tool call so cut has
//! arguments that are not JSON.
//!
//! ```text
//! let provider = ScriptedProvider::new([
//!     Response::new().text("Reading it.").tool_call("c1", "read", &json!({"path": "a"})).end(StopReason::ToolCalls),
//!     Response::new().text("Done.").end(StopReason::End),
//! ]);
//! ```

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cortex_base::{CancelToken, ErrorKind};
use cortex_llm::error::ProviderError;
use cortex_llm::{
    BlockKind, ModelClient, ModelEvent, ModelRequest, ModelStream, SpeedMove, Timer, Usage,
};
use cortex_protocol::speed::{SpeedFallbackBy, SpeedFallbackReason};
use cortex_protocol::usage::TokenUsage;
use cortex_protocol::{RawJson, Speed, StopReason};
use serde_json::{Value, json};

use crate::gate::Gate;

/// Bytes to an output token, as the agent loop counts before a provider has
/// counted a request (`cortex_agent::context::DEFAULT_RATIO`: a test of
/// `cortex-agent` holds the two together).
pub const BYTES_PER_OUTPUT_TOKEN: u64 = 3;

/// One scripted response.
#[derive(Debug, Clone, Default)]
pub struct Response {
    steps: Vec<Step>,
    next_index: u32,
    tokens: TokenUsage,
    served_at: Option<Speed>,
}

#[derive(Debug, Clone)]
#[expect(
    clippy::large_enum_variant,
    reason = "a script is a few dozen steps, nearly all of them events; a box per step buys nothing"
)]
enum Step {
    /// `Start`, with the model's id from the request.
    Start,
    Event(ModelEvent),
    /// A running usage event, with the response's tokens so far.
    Usage,
    End(StopReason),
    Delay(Duration),
    Stall,
    Wait(Gate),
    Abort,
}

impl Response {
    /// A response that has begun and said nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            steps: vec![Step::Start],
            ..Self::default()
        }
    }

    /// A response that is exactly these events, with nothing added: for a
    /// test that needs a sequence the builder would not produce.
    #[must_use]
    pub fn events(events: impl IntoIterator<Item = ModelEvent>) -> Self {
        Self {
            steps: events.into_iter().map(Step::Event).collect(),
            ..Self::default()
        }
    }

    /// A text block, delivered in one piece.
    #[must_use]
    pub fn text(self, text: &str) -> Self {
        self.text_in(&[text])
    }

    /// A text block, delivered in the pieces given.
    #[must_use]
    pub fn text_in(mut self, pieces: &[&str]) -> Self {
        let index = self.block(BlockKind::Text);
        for piece in pieces {
            self.steps.push(Step::Event(ModelEvent::TextDelta {
                index,
                text: (*piece).to_owned(),
            }));
        }
        let raw = json!({"type": "text", "text": pieces.concat()});
        self.end_block(index, &raw)
    }

    /// A reasoning block with this visible text.
    #[must_use]
    pub fn reasoning(mut self, text: &str) -> Self {
        let index = self.block(BlockKind::Reasoning);
        self.steps.push(Step::Event(ModelEvent::ReasoningDelta {
            index,
            text: text.to_owned(),
        }));
        let raw = json!({"type": "reasoning", "text": text, "signature": "scripted"});
        self.end_block(index, &raw)
    }

    /// A tool call. Its arguments arrive as two fragments, neither of them
    /// valid JSON alone, as a provider streams them.
    #[must_use]
    pub fn tool_call(self, id: &str, name: &str, arguments: &Value) -> Self {
        let text = arguments.to_string();
        let middle = (1..text.len())
            .rev()
            .find(|at| *at <= text.len() / 2 && text.is_char_boundary(*at))
            .unwrap_or(0);
        let (first, second) = text.split_at(middle);
        self.tool_call_in(id, name, &[first, second])
    }

    /// A tool call whose arguments arrive as exactly these fragments. They
    /// need not add up to valid JSON: this is how a test scripts a
    /// malformed call.
    #[must_use]
    pub fn tool_call_in(mut self, id: &str, name: &str, fragments: &[&str]) -> Self {
        let index = self.block(BlockKind::ToolCall);
        self.steps.push(Step::Event(ModelEvent::ToolCallStart {
            index,
            id: id.to_owned(),
            name: name.to_owned(),
        }));
        for fragment in fragments {
            self.steps.push(Step::Event(ModelEvent::ToolArgsDelta {
                index,
                json: (*fragment).to_owned(),
            }));
        }
        let raw =
            json!({"type": "tool_call", "id": id, "name": name, "arguments": fragments.concat()});
        self.end_block(index, &raw)
    }

    /// A block the neutral model does not name, with the provider's JSON.
    #[must_use]
    pub fn provider_block(mut self, raw: &Value) -> Self {
        let index = self.block(BlockKind::ProviderBlock);
        self.end_block(index, raw)
    }

    /// The tokens the response has used. They are reported by every
    /// [`usage`](Self::usage) event after this and by the end.
    #[must_use]
    pub fn tokens(mut self, tokens: TokenUsage) -> Self {
        self.tokens = tokens;
        self
    }

    /// A running usage event at this point, with the tokens set so far.
    #[must_use]
    pub fn usage(mut self) -> Self {
        self.steps.push(Step::Usage);
        self
    }

    /// The provider served the request at `speed`, whatever was asked for.
    /// When that is another speed than the request's, the response says so
    /// with a fallback event marked as the provider's doing, and its usage
    /// records the speed that ran and the speed that was asked for.
    #[must_use]
    pub fn served_at(mut self, speed: Speed) -> Self {
        self.served_at = Some(speed);
        self
    }

    /// Any event, at this point.
    #[must_use]
    pub fn event(mut self, event: ModelEvent) -> Self {
        self.steps.push(Step::Event(event));
        self
    }

    /// The response ends, for this reason, with its usage.
    #[must_use]
    pub fn end(mut self, stop: StopReason) -> Self {
        self.steps.push(Step::End(stop));
        self
    }

    /// The response fails at this point: its last event is this error.
    #[must_use]
    pub fn error(self, error: ProviderError) -> Self {
        self.event(ModelEvent::Error(error))
    }

    /// The response waits here for `duration`, on the provider's timer.
    /// Without a timer it does not wait.
    #[must_use]
    pub fn delay(mut self, duration: Duration) -> Self {
        self.steps.push(Step::Delay(duration));
        self
    }

    /// The response stops here and never goes on: it ends only when the
    /// request is cancelled, with an `Interrupted` error.
    #[must_use]
    pub fn stall(mut self) -> Self {
        self.steps.push(Step::Stall);
        self
    }

    /// The response waits here until the test opens `gate`.
    #[must_use]
    pub fn wait(mut self, gate: &Gate) -> Self {
        self.steps.push(Step::Wait(gate.clone()));
        self
    }

    /// The stream stops here with no end and no error. The real client
    /// never does this: it is here so that a loop can be shown to survive a
    /// client that breaks its word.
    #[must_use]
    pub fn abort(mut self) -> Self {
        self.steps.push(Step::Abort);
        self
    }

    fn block(&mut self, kind: BlockKind) -> u32 {
        let index = self.next_index;
        self.next_index += 1;
        self.steps
            .push(Step::Event(ModelEvent::BlockStart { index, kind }));
        index
    }

    fn end_block(mut self, index: u32, raw: &Value) -> Self {
        self.steps.push(Step::Event(ModelEvent::BlockEnd {
            index,
            raw: RawJson::new(raw.to_string()),
        }));
        self
    }
}

/// A [`ModelClient`] that plays a script. Clones share the script and the
/// record of requests.
#[derive(Debug, Clone, Default)]
pub struct ScriptedProvider {
    state: Arc<Mutex<State>>,
    timer: Option<Arc<dyn Timer>>,
}

#[derive(Debug, Default)]
struct State {
    responses: VecDeque<Response>,
    requests: Vec<ModelRequest>,
    /// Set by [`ScriptedProvider::without_output_limit`]: the default is to
    /// cut a response at the request's `max_output`.
    unlimited: bool,
}

impl ScriptedProvider {
    /// A provider that answers with these responses, in order.
    #[must_use]
    pub fn new(responses: impl IntoIterator<Item = Response>) -> Self {
        let provider = Self::default();
        for response in responses {
            provider.push(response);
        }
        provider
    }

    /// The same provider, with a timer for [`Response::delay`].
    #[must_use]
    pub fn with_timer(mut self, timer: Arc<dyn Timer>) -> Self {
        self.timer = Some(timer);
        self
    }

    /// The same provider, which no longer cuts a response at the request's
    /// `max_output`: for a test whose script is longer than the request
    /// allows and that means it to be played whole. Like the script, the
    /// setting is shared by every clone, so a test can change the provider
    /// of a rig that is already made.
    #[must_use]
    pub fn without_output_limit(self) -> Self {
        self.set_output_limit(false);
        self
    }

    /// Turns the cut at the request's `max_output` on or off, for every
    /// clone ([`without_output_limit`](Self::without_output_limit) is this
    /// at construction).
    pub fn set_output_limit(&self, on: bool) {
        self.lock().unlimited = !on;
    }

    /// Adds a response to the end of the script.
    pub fn push(&self, response: Response) {
        self.lock().responses.push_back(response);
    }

    /// Every request received so far, in order. A request is received when
    /// its stream is first read.
    #[must_use]
    pub fn requests(&self) -> Vec<ModelRequest> {
        self.lock().requests.clone()
    }

    /// How many responses have not been used.
    #[must_use]
    pub fn remaining(&self) -> usize {
        self.lock().responses.len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A block that has begun and not ended, with what it has said so far.
#[derive(Debug, Default)]
struct Open {
    kind: Option<BlockKind>,
    said: String,
    id: String,
    name: String,
}

/// The room a response has, in bytes, and what it has used.
#[derive(Debug)]
struct OutputLimit {
    /// The limit in tokens: the usage of a response cut at it.
    tokens: u64,
    left: u64,
    open: BTreeMap<u32, Open>,
}

impl OutputLimit {
    fn new(tokens: u64) -> Self {
        Self {
            tokens,
            left: tokens.saturating_mul(BYTES_PER_OUTPUT_TOKEN),
            open: BTreeMap::new(),
        }
    }

    /// Whether the response goes on past `event`. `None` passes the event
    /// as it was scripted. `Some` is the events to send in its place, because
    /// the limit fell inside it: what fits of it, then the close of every
    /// block still open, after which the response ends at the limit.
    fn admit(&mut self, event: &ModelEvent) -> Option<Vec<ModelEvent>> {
        match event {
            ModelEvent::BlockStart { index, kind } => {
                self.open.insert(
                    *index,
                    Open {
                        kind: Some(*kind),
                        ..Open::default()
                    },
                );
            }
            ModelEvent::ToolCallStart { index, id, name } => {
                let open = self.open.entry(*index).or_default();
                open.id.clone_from(id);
                open.name.clone_from(name);
            }
            ModelEvent::BlockEnd { index, .. } => {
                self.open.remove(index);
            }
            ModelEvent::TextDelta { index, text }
            | ModelEvent::ReasoningDelta { index, text }
            | ModelEvent::ToolArgsDelta { index, json: text } => {
                if text.len() as u64 <= self.left {
                    self.left -= text.len() as u64;
                    self.open.entry(*index).or_default().said.push_str(text);
                    return None;
                }
                return Some(self.cut(event));
            }
            _ => {}
        }
        None
    }

    /// The events that stand for `delta` once the limit has fallen inside
    /// it: what fits of it, then the close of each open block with what it
    /// had said.
    fn cut(&mut self, delta: &ModelEvent) -> Vec<ModelEvent> {
        let room = usize::try_from(self.left).unwrap_or(usize::MAX);
        let mut events = Vec::new();
        let (ModelEvent::TextDelta { index, text }
        | ModelEvent::ReasoningDelta { index, text }
        | ModelEvent::ToolArgsDelta { index, json: text }) = delta
        else {
            unreachable!("only a delta is cut");
        };
        let fits = (0..=room.min(text.len()))
            .rev()
            .find(|at| text.is_char_boundary(*at))
            .unwrap_or(0);
        if fits > 0 {
            let piece = text[..fits].to_owned();
            self.open.entry(*index).or_default().said.push_str(&piece);
            let index = *index;
            events.push(match delta {
                ModelEvent::TextDelta { .. } => ModelEvent::TextDelta { index, text: piece },
                ModelEvent::ReasoningDelta { .. } => {
                    ModelEvent::ReasoningDelta { index, text: piece }
                }
                _ => ModelEvent::ToolArgsDelta { index, json: piece },
            });
        }
        self.left = 0;
        for (index, open) in std::mem::take(&mut self.open) {
            let raw = match open.kind {
                Some(BlockKind::Reasoning) => {
                    json!({"type": "reasoning", "text": open.said, "signature": "scripted"})
                }
                Some(BlockKind::ToolCall) => json!({
                    "type": "tool_call", "id": open.id, "name": open.name, "arguments": open.said,
                }),
                _ => json!({"type": "text", "text": open.said}),
            };
            events.push(ModelEvent::BlockEnd {
                index,
                raw: RawJson::new(raw.to_string()),
            });
        }
        events
    }
}

impl ModelClient for ScriptedProvider {
    fn stable_prefix(&self, request: &ModelRequest) -> Option<Vec<u8>> {
        Some(crate::protocol::neutral_prefix(request))
    }

    fn stream(&self, request: ModelRequest, cancel: CancelToken) -> ModelStream {
        let provider = self.clone();
        ModelStream::generate(move |out| async move {
            let model = request.model.info.id.to_string();
            let requested = request.speed.clone();
            let max_output = request.model.info.max_output;
            let unlimited = provider.lock().unlimited;
            let mut limit = (!unlimited).then(|| {
                OutputLimit::new(
                    request
                        .max_output
                        .map_or(max_output, |asked| asked.min(max_output)),
                )
            });
            let response = {
                let mut state = provider.lock();
                state.requests.push(request);
                state.responses.pop_front()
            };
            let Some(response) = response else {
                out.emit(ModelEvent::Error(ProviderError::new(
                    ErrorKind::Internal,
                    "script_exhausted",
                    "The scripted provider has no response left for this request.",
                )))
                .await;
                return;
            };

            // The speed the response "ran at": what the script says served
            // it, and otherwise what was asked for.
            let ran = response
                .served_at
                .clone()
                .unwrap_or_else(|| requested.clone());
            let usage = |tokens: &TokenUsage| Usage {
                tokens: tokens.clone(),
                speed: ran.clone(),
                speed_requested: (ran != requested).then(|| requested.clone()),
                provider_cost: None,
                raw: None,
            };

            for step in response.steps {
                if cancel.is_cancelled() {
                    out.emit(ModelEvent::Error(ProviderError::interrupted()))
                        .await;
                    return;
                }
                match step {
                    Step::Start => {
                        out.emit(ModelEvent::Start {
                            response_id: Some("scripted".to_owned()),
                            model: model.clone(),
                        })
                        .await;
                        if ran != requested {
                            out.emit(ModelEvent::SpeedFallback(SpeedMove {
                                requested: requested.clone(),
                                served: ran.clone(),
                                by: SpeedFallbackBy::Provider,
                                reason: SpeedFallbackReason::Unspecified,
                                cooldown_until: None,
                            }))
                            .await;
                        }
                    }
                    Step::Event(event) => {
                        let last = event.is_terminal();
                        let Some(limit) = limit.as_mut() else {
                            out.emit(event).await;
                            if last {
                                return;
                            }
                            continue;
                        };
                        match limit.admit(&event) {
                            None => {
                                out.emit(event).await;
                                if last {
                                    return;
                                }
                            }
                            Some(events) => {
                                for event in events {
                                    out.emit(event).await;
                                }
                                let mut tokens = response.tokens.clone();
                                tokens.output = limit.tokens;
                                out.emit(ModelEvent::End {
                                    raw_stop: StopReason::MaxOutput.as_str().to_owned(),
                                    stop: StopReason::MaxOutput,
                                    stop_detail: None,
                                    usage: usage(&tokens),
                                })
                                .await;
                                return;
                            }
                        }
                    }
                    Step::Usage => out.emit(ModelEvent::Usage(usage(&response.tokens))).await,
                    Step::End(stop) => {
                        out.emit(ModelEvent::End {
                            raw_stop: stop.as_str().to_owned(),
                            stop,
                            stop_detail: None,
                            usage: usage(&response.tokens),
                        })
                        .await;
                        return;
                    }
                    Step::Delay(duration) => {
                        if let Some(timer) = &provider.timer
                            && cancel.race(timer.sleep(duration)).await.is_none()
                        {
                            out.emit(ModelEvent::Error(ProviderError::interrupted()))
                                .await;
                            return;
                        }
                    }
                    Step::Stall => {
                        cancel.cancelled().await;
                        out.emit(ModelEvent::Error(ProviderError::interrupted()))
                            .await;
                        return;
                    }
                    Step::Wait(gate) => {
                        if cancel.race(gate.opened()).await.is_none() {
                            out.emit(ModelEvent::Error(ProviderError::interrupted()))
                                .await;
                            return;
                        }
                    }
                    Step::Abort => return,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    use crate::{FakeTimer, block_on};
    use cortex_base::FakeClock;

    fn names(events: &[ModelEvent]) -> Vec<&'static str> {
        events
            .iter()
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

    fn play(provider: &ScriptedProvider) -> Vec<ModelEvent> {
        let request = fixtures::request(fixtures::model());
        block_on(provider.stream(request, CancelToken::new()).collect())
    }

    #[test]
    fn it_expresses_text_reasoning_tool_calls_and_usage() {
        let tokens = TokenUsage {
            input: 120,
            output: 40,
            reasoning: Some(12),
            ..TokenUsage::default()
        };
        let provider = ScriptedProvider::new([Response::new()
            .reasoning("The file first.")
            .text_in(&["Reading ", "it."])
            .tool_call("call_1", "read", &json!({"path": "src/main.rs"}))
            .tokens(tokens.clone())
            .usage()
            .end(StopReason::ToolCalls)]);
        let events = play(&provider);
        assert_eq!(
            names(&events),
            [
                "start",
                "block_start",
                "reasoning",
                "block_end",
                "block_start",
                "text",
                "text",
                "block_end",
                "block_start",
                "tool_call",
                "tool_args",
                "tool_args",
                "block_end",
                "usage",
                "end"
            ]
        );
        // The tool's arguments arrive in pieces that are JSON only together.
        let fragments: Vec<&str> = events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::ToolArgsDelta { json, .. } => Some(json.as_str()),
                _ => None,
            })
            .collect();
        assert!(serde_json::from_str::<Value>(fragments[0]).is_err());
        assert_eq!(
            serde_json::from_str::<Value>(&fragments.concat()).unwrap(),
            json!({"path": "src/main.rs"})
        );
        // Blocks are numbered in order, and the end carries the usage.
        let indexes: Vec<u32> = events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::BlockStart { index, .. } => Some(*index),
                _ => None,
            })
            .collect();
        assert_eq!(indexes, [0, 1, 2]);
        let Some(ModelEvent::End { stop, usage, .. }) = events.last() else {
            panic!("{events:?}");
        };
        assert_eq!(*stop, StopReason::ToolCalls);
        assert_eq!(usage.tokens, tokens);
        assert_eq!(
            (usage.speed.clone(), usage.speed_requested.clone()),
            (Speed::Standard, None)
        );

        assert_eq!(provider.requests().len(), 1);
        assert_eq!(provider.requests()[0].system, "You are a test.");
        assert_eq!(provider.remaining(), 0);
    }

    #[test]
    fn responses_are_played_in_order_and_running_out_is_an_error() {
        let provider = ScriptedProvider::new([
            Response::new().text("one").end(StopReason::End),
            Response::new().text("two").end(StopReason::End),
        ]);
        assert_eq!(provider.remaining(), 2);
        let text = |events: &[ModelEvent]| {
            events.iter().find_map(|event| match event {
                ModelEvent::TextDelta { text, .. } => Some(text.clone()),
                _ => None,
            })
        };
        assert_eq!(text(&play(&provider)).as_deref(), Some("one"));
        assert_eq!(text(&play(&provider)).as_deref(), Some("two"));
        let events = play(&provider);
        let [ModelEvent::Error(error)] = events.as_slice() else {
            panic!("{events:?}");
        };
        assert_eq!(error.code, "script_exhausted");
        assert_eq!(provider.requests().len(), 3);
    }

    #[test]
    fn an_error_part_way_is_the_last_event() {
        let failure = ProviderError::new(ErrorKind::Transient, "overloaded", "Busy.");
        let provider = ScriptedProvider::new([Response::new()
            .text("half an ans")
            .error(failure.clone())
            .text("never said")]);
        let events = play(&provider);
        assert_eq!(
            names(&events),
            ["start", "block_start", "text", "block_end", "error"]
        );
        assert_eq!(events.last(), Some(&ModelEvent::Error(failure)));
    }

    #[test]
    fn a_stall_lasts_until_the_request_is_cancelled() {
        let provider = ScriptedProvider::new([Response::new().text("so far").stall()]);
        let cancel = CancelToken::new();
        let mut stream = provider.stream(fixtures::request(fixtures::model()), cancel.clone());
        block_on(async {
            for _ in 0..4 {
                assert!(stream.next().await.is_some());
            }
            // The stream is now waiting. Cancelling is what ends it.
            cancel.cancel();
            let last = stream.next().await;
            let Some(ModelEvent::Error(error)) = last else {
                panic!("{last:?}");
            };
            assert_eq!(error.kind, ErrorKind::Interrupted);
            assert_eq!(stream.next().await, None);
        });
    }

    #[test]
    fn an_abort_ends_the_stream_with_no_last_word() {
        let provider = ScriptedProvider::new([Response::new().text("cut").abort()]);
        let events = play(&provider);
        assert_eq!(
            names(&events),
            ["start", "block_start", "text", "block_end"]
        );
        assert!(!events.last().unwrap().is_terminal());
    }

    #[test]
    fn a_delay_is_a_wait_on_the_timer_and_a_gate_waits_for_the_test() {
        let clock = FakeClock::default();
        let timer = FakeTimer::auto(&clock);
        let gate = Gate::new();
        let provider = ScriptedProvider::new([Response::new()
            .delay(Duration::from_secs(3))
            .text("late")
            .wait(&gate)
            .end(StopReason::End)])
        .with_timer(Arc::new(timer.clone()));
        let mut stream = provider.stream(fixtures::request(fixtures::model()), CancelToken::new());
        block_on(async {
            assert!(matches!(
                stream.next().await,
                Some(ModelEvent::Start { .. })
            ));
            assert!(
                timer.sleeps().is_empty(),
                "nothing happens ahead of the reader"
            );
            assert!(matches!(
                stream.next().await,
                Some(ModelEvent::BlockStart { .. })
            ));
            assert_eq!(timer.sleeps(), [Duration::from_secs(3)]);
            assert!(stream.next().await.is_some());
            assert!(stream.next().await.is_some());
            gate.open();
            assert!(matches!(stream.next().await, Some(ModelEvent::End { .. })));
        });
    }

    #[test]
    fn a_response_served_at_another_speed_says_so_and_records_both() {
        let provider = ScriptedProvider::new([
            Response::new()
                .served_at(Speed::Standard)
                .text("ok")
                .usage()
                .end(StopReason::End),
            Response::new().text("ok").end(StopReason::End),
        ]);
        let ask_fast = || {
            let mut request = fixtures::request(fixtures::model());
            request.speed = Speed::Fast;
            block_on(provider.stream(request, CancelToken::new()).collect())
        };

        let downgraded = ask_fast();
        assert_eq!(names(&downgraded)[..2], ["start", "speed_fallback"]);
        let ModelEvent::SpeedFallback(change) = &downgraded[1] else {
            panic!("{downgraded:?}");
        };
        assert_eq!(
            (
                change.requested.clone(),
                change.served.clone(),
                change.by.clone()
            ),
            (Speed::Fast, Speed::Standard, SpeedFallbackBy::Provider)
        );
        for event in &downgraded {
            if let ModelEvent::Usage(usage) | ModelEvent::End { usage, .. } = event {
                assert_eq!(usage.speed, Speed::Standard);
                assert_eq!(usage.speed_requested, Some(Speed::Fast));
            }
        }

        let served = ask_fast();
        assert!(!names(&served).contains(&"speed_fallback"));
        let Some(ModelEvent::End { usage, .. }) = served.last() else {
            panic!("{served:?}");
        };
        assert_eq!(
            (usage.speed.clone(), usage.speed_requested.clone()),
            (Speed::Fast, None)
        );
    }

    #[test]
    fn raw_events_are_played_exactly() {
        let events = vec![
            ModelEvent::TextDelta {
                index: 9,
                text: "odd".to_owned(),
            },
            ModelEvent::Error(ProviderError::interrupted()),
        ];
        let provider = ScriptedProvider::new([Response::events(events.clone())]);
        assert_eq!(play(&provider), events);
    }

    /// The events of `provider`'s next response to a request that allows
    /// `max_output` tokens (the model's own most when `None`).
    fn play_limited(provider: &ScriptedProvider, max_output: Option<u64>) -> Vec<ModelEvent> {
        let mut request = fixtures::request(fixtures::model());
        request.max_output = max_output;
        block_on(provider.stream(request, CancelToken::new()).collect())
    }

    fn ended(events: &[ModelEvent]) -> (StopReason, u64) {
        let Some(ModelEvent::End { stop, usage, .. }) = events.last() else {
            panic!("{events:?}");
        };
        (stop.clone(), usage.tokens.output)
    }

    fn block_ends(events: &[ModelEvent]) -> Vec<Value> {
        events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::BlockEnd { raw, .. } => {
                    Some(serde_json::from_str(raw.as_str()).unwrap())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn an_answer_past_the_requests_limit_is_cut_and_ends_at_the_limit() {
        // Two tokens are six bytes.
        let provider = ScriptedProvider::new([Response::new()
            .text("abcdefghij")
            .text("never said")
            .tokens(TokenUsage {
                input: 50,
                output: 99,
                ..TokenUsage::default()
            })
            .end(StopReason::End)]);
        let events = play_limited(&provider, Some(2));
        assert_eq!(
            names(&events),
            ["start", "block_start", "text", "block_end", "end"]
        );
        assert_eq!(
            events[2],
            ModelEvent::TextDelta {
                index: 0,
                text: "abcdef".to_owned()
            }
        );
        assert_eq!(
            block_ends(&events),
            [json!({"type": "text", "text": "abcdef"})]
        );
        let ModelEvent::End {
            raw_stop, usage, ..
        } = events.last().unwrap()
        else {
            panic!("{events:?}");
        };
        assert_eq!(raw_stop, "max_output");
        assert_eq!(
            (usage.tokens.output, usage.tokens.input),
            (2, 50),
            "the usage says the limit was reached, and keeps what else it said"
        );
    }

    #[test]
    fn an_answer_that_fits_exactly_is_played_whole() {
        let provider =
            ScriptedProvider::new([Response::new().text("abc").text("def").end(StopReason::End)]);
        let events = play_limited(&provider, Some(2));
        assert_eq!(ended(&events), (StopReason::End, 0));
        assert_eq!(block_ends(&events).len(), 2);
    }

    #[test]
    fn text_reasoning_and_arguments_all_count_and_the_cut_closes_the_open_block() {
        let provider = ScriptedProvider::new([Response::new()
            .reasoning("abc")
            .text("de")
            .tool_call_in(
                "c1",
                "write",
                &["{\"path\": ", "\"a.txt\", \"content\": \"long\"}"],
            )
            .text("never said")
            .end(StopReason::ToolCalls)]);
        // Four tokens are twelve bytes: 3 + 2, and 7 of the arguments' first piece.
        let events = play_limited(&provider, Some(4));
        assert_eq!(ended(&events), (StopReason::MaxOutput, 4));
        let ends = block_ends(&events);
        assert_eq!(ends.len(), 3, "{events:?}");
        assert_eq!(ends[0]["text"], "abc");
        assert_eq!(ends[1]["text"], "de");
        assert_eq!(
            ends[2],
            json!({
                "type": "tool_call", "id": "c1", "name": "write",
                "arguments": "{\"path\""
            }),
            "a call cut in its arguments, which are not JSON"
        );
        // Nothing after the cut was played.
        assert_eq!(
            names(&events)
                .iter()
                .filter(|name| **name == "text")
                .count(),
            1
        );
    }

    #[test]
    fn a_cut_never_splits_a_character() {
        let provider = ScriptedProvider::new([Response::new().text("aé€xyz").end(StopReason::End)]);
        // Six bytes of "aé€": the euro sign takes bytes 3 to 5, so a limit of
        // one token (three bytes) falls inside it.
        let events = play_limited(&provider, Some(1));
        assert_eq!(block_ends(&events), [json!({"type": "text", "text": "aé"})]);
    }

    #[test]
    fn a_request_with_no_limit_is_held_to_the_models_most_and_never_past_it() {
        // `test-model` writes at most 8,000 tokens: 24,000 bytes.
        let at_most = "x".repeat(24_000);
        for (asked, text, stop) in [
            (None, at_most.clone(), StopReason::End),
            (None, format!("{at_most}x"), StopReason::MaxOutput),
            (
                Some(1_000_000),
                format!("{at_most}x"),
                StopReason::MaxOutput,
            ),
        ] {
            let provider =
                ScriptedProvider::new([Response::new().text(&text).end(StopReason::End)]);
            assert_eq!(
                ended(&play_limited(&provider, asked)).0,
                stop,
                "asked {asked:?}, {} bytes",
                text.len()
            );
        }
    }

    #[test]
    fn a_provider_without_the_output_limit_plays_every_answer_whole() {
        let provider =
            ScriptedProvider::new([Response::new().text("abcdefghij").end(StopReason::End)])
                .without_output_limit();
        let events = play_limited(&provider, Some(1));
        assert_eq!(ended(&events), (StopReason::End, 0));
        assert_eq!(
            block_ends(&events),
            [json!({"type": "text", "text": "abcdefghij"})]
        );
    }
}
