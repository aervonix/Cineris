//! The core's items onto the transcript: 09-tui's table of what each event
//! becomes (`docs/architecture/09-tui.md`, "Structure of `cineris-tui`").
//!
//! One response of the model is one assistant item, and may become several
//! blocks: its reasoning, its answer, and the round of the tool calls it
//! asked for, in the order their parts began. A part that begins after
//! another ends that other: reasoning ends when the answer or a call
//! begins, and an answer ends when a call begins, so that a block below it
//! is never held back from the scrollback by one still open above it.
//!
//! Each block's [`BlockId`] is kept while it is open, and reached with
//! `Transcript::get_mut`, since notices and steering messages may be pushed
//! below it meanwhile. Times (reasoning's, a running call's) are given each
//! frame by [`Mapper::tick`].

use std::collections::HashMap;
use std::time::{Duration, Instant};

use cineris_protocol::event::{
    Delta, Event, ItemCompleted, ItemDelta, ItemKind, ItemStarted, TextDelta, ToolArgsDelta,
    ToolCallDelta,
};
use cineris_protocol::item::{
    AssistantMessageItem, NoticeSource, StopReason, ToolResultItem, UserMessageItem,
};
use cineris_protocol::message::Part;
use cineris_protocol::{CallId, Item, ItemId};
use serde_json::Value;

use super::tools;
use crate::highlight::Highlighter;
use crate::markdown::Markdown;
use crate::transcript::{Block, BlockId, Reasoning, Transcript};
use crate::view::{CallState, Detail, Round};

/// The most of a running command's output the interface keeps: the tail,
/// which is what a box shows (its last 4 lines while it runs, a failure's
/// last 3). The whole of it is the session's, for the viewer.
pub(crate) const OUTPUT_TAIL: usize = 64 * 1024;

/// One response's blocks while it streams.
#[derive(Debug, Default)]
struct Response {
    /// Its reasoning block while open, and when it began.
    reasoning: Option<(BlockId, Instant)>,
    /// Its answer block while open.
    answer: Option<BlockId>,
    /// The round of its tool calls.
    round: Option<u64>,
    /// The bytes of answer text that arrived as deltas.
    text: usize,
    /// Its deltas stopped arriving (`item.resync`), or its start was never
    /// seen, so that what arrived is not known to be all of it: what is
    /// missing, or all of it, is taken from the completed item.
    resync: bool,
    /// Its calls, in order.
    calls: Vec<CallId>,
    /// Their arguments, as they streamed.
    args: HashMap<CallId, String>,
}

impl Response {
    /// A response whose deltas arrive with no `item.started` before them:
    /// they may begin part-way, so none is drawn, and the whole is drawn
    /// when it completes.
    fn unseen() -> Self {
        Self {
            resync: true,
            ..Self::default()
        }
    }
}

/// Where one call's line is.
#[derive(Debug)]
struct Place {
    round: u64,
    index: usize,
    /// When its tool began to run.
    started: Option<Instant>,
    /// Whether it has ended.
    ended: bool,
}

/// A round's box: its block, while any of it is to commit.
#[derive(Debug)]
struct LiveRound {
    block: BlockId,
}

/// The mapping in progress. See the [module documentation](self).
#[derive(Debug)]
pub(crate) struct Mapper {
    highlighter: Highlighter,
    responses: HashMap<ItemId, Response>,
    calls: HashMap<CallId, Place>,
    /// A tool result item that is running, and the call it answers.
    results: HashMap<ItemId, CallId>,
    /// Tool results whose output deltas stopped (`item.resync`): their box
    /// shows the result's own text, not the part that streamed.
    resynced: std::collections::HashSet<ItemId>,
    /// The rounds still live, by the number the focus ring knows them by.
    rounds: Vec<(u64, LiveRound)>,
    next_round: u64,
    /// The last answer's text, for `/copy`.
    last_answer: String,
}

impl Mapper {
    /// The highlighter for the process, which the kept answers' late code
    /// is handed to again (step 25).
    pub(crate) fn highlighter(&self) -> &Highlighter {
        &self.highlighter
    }

    pub(crate) fn new(highlighter: Highlighter) -> Self {
        Self {
            highlighter,
            responses: HashMap::new(),
            calls: HashMap::new(),
            results: HashMap::new(),
            resynced: std::collections::HashSet::new(),
            rounds: Vec::new(),
            next_round: 1,
            last_answer: String::new(),
        }
    }

    /// The last answer's text.
    #[must_use]
    pub(crate) fn last_answer(&self) -> &str {
        &self.last_answer
    }

    /// The round that holds `call`, by the number the viewer counts boxes
    /// with (since step 24): rounds are numbered from 1 in the order their
    /// boxes were pushed, which is the order they commit in. Known until
    /// the turn ends.
    #[must_use]
    pub(crate) fn round_of(&self, call: &CallId) -> Option<u64> {
        self.calls.get(call).map(|place| place.round)
    }

    /// The rounds whose boxes are live, in the order drawn, by the numbers
    /// the focus ring knows them by.
    pub(crate) fn live_rounds(&mut self, transcript: &mut Transcript) -> Vec<u64> {
        self.rounds.retain(|(_, round)| {
            matches!(transcript.get_mut(round.block), Some(Block::Round(round)) if round.is_running())
        });
        self.rounds.iter().map(|(id, _)| *id).collect()
    }

    /// Opens or closes a live round's box: the given one, or the last.
    pub(crate) fn toggle_round(
        &mut self,
        transcript: &mut Transcript,
        which: Option<u64>,
        look: &crate::view::Look,
    ) {
        let found = match which {
            Some(id) => self.rounds.iter().find(|(number, _)| *number == id),
            None => self.rounds.last(),
        };
        let Some((_, round)) = found else {
            return;
        };
        if let Some(Block::Round(round)) = transcript.get_mut(round.block) {
            round.open = Some(!crate::view::shows_detail(round, look));
        }
    }

    /// Gives the open blocks the time: reasoning's length and each running
    /// call's.
    pub(crate) fn tick(&mut self, transcript: &mut Transcript, now: Instant) {
        for response in self.responses.values() {
            if let Some((block, began)) = response.reasoning
                && let Some(Block::Reasoning(reasoning)) = transcript.get_mut(block)
            {
                reasoning.elapsed = now.saturating_duration_since(began);
            }
        }
        for place in self.calls.values() {
            if place.ended {
                continue;
            }
            let Some(started) = place.started else {
                continue;
            };
            if let Some(call) = self.call_mut(transcript, place)
                && matches!(call.state, CallState::Running(_))
            {
                call.state = CallState::Running(now.saturating_duration_since(started));
            }
        }
    }

    /// A call is waiting for the person's answer.
    pub(crate) fn waiting(&mut self, transcript: &mut Transcript, call: &CallId) {
        if let Some(place) = self.calls.get(call)
            && !place.ended
            && let Some(call) = self.call_mut(transcript, place)
        {
            call.state = CallState::Waiting;
        }
    }

    /// The call's answer was given: it is running again until its result.
    pub(crate) fn answered(&mut self, transcript: &mut Transcript, call: &CallId) {
        if let Some(place) = self.calls.get(call)
            && !place.ended
            && let Some(call) = self.call_mut(transcript, place)
            && call.state == CallState::Waiting
        {
            call.state = CallState::Running(Duration::ZERO);
        }
    }

    /// One event of the session.
    pub(crate) fn event(&mut self, transcript: &mut Transcript, event: &Event, now: Instant) {
        match event {
            Event::ItemStarted(started) => match &started.kind {
                ItemKind::AssistantMessage => {
                    self.responses.entry(started.item_id.clone()).or_default();
                }
                ItemKind::ToolResult => {
                    if let Some(call) = &started.call_id {
                        self.results.insert(started.item_id.clone(), call.clone());
                        if let Some(place) = self.calls.get_mut(call) {
                            place.started = Some(now);
                        }
                        if let Some(place) = self.calls.get(call)
                            && let Some(line) = self.call_mut(transcript, place)
                        {
                            line.state = CallState::Running(Duration::ZERO);
                        }
                    }
                }
                _ => {}
            },
            Event::ItemDelta(delta) => self.delta(transcript, &delta.item_id, &delta.delta, now),
            Event::ItemReset(reset) => self.reset(transcript, &reset.item_id, now),
            Event::ItemResync(resync) => {
                if let Some(response) = self.responses.get_mut(&resync.item_id) {
                    response.resync = true;
                }
                if self.results.contains_key(&resync.item_id) {
                    self.resynced.insert(resync.item_id.clone());
                }
            }
            Event::ItemCompleted(completed) => self.completed(transcript, &completed.item, now),
            _ => {}
        }
    }

    /// An item a read gave in progress, with what it had then: drawn as
    /// its events would have drawn it, to go on with the events after the
    /// read.
    pub(crate) fn in_progress(&mut self, transcript: &mut Transcript, item: &Item, now: Instant) {
        let mut events = Vec::new();
        let delta = |item_id: &ItemId, delta: Delta| {
            Event::ItemDelta(ItemDelta {
                item_id: item_id.clone(),
                delta,
            })
        };
        match item {
            Item::AssistantMessage(message) => {
                events.push(Event::ItemStarted(ItemStarted {
                    item_id: message.id.clone(),
                    kind: ItemKind::AssistantMessage,
                    tool: None,
                    call_id: None,
                }));
                for part in &message.parts {
                    match part {
                        Part::Reasoning(reasoning) => events.push(delta(
                            &message.id,
                            Delta::Reasoning(TextDelta {
                                text: reasoning.text.clone(),
                            }),
                        )),
                        Part::Text(text) => events.push(delta(
                            &message.id,
                            Delta::Text(TextDelta {
                                text: text.text.clone(),
                            }),
                        )),
                        Part::ToolCall(call) => {
                            events.push(delta(
                                &message.id,
                                Delta::ToolCall(ToolCallDelta {
                                    call_id: call.id.clone(),
                                    name: call.name.clone(),
                                }),
                            ));
                            // Not valid JSON until the call is complete:
                            // the text so far, as text.
                            let json = match &call.arguments {
                                Value::Null => String::new(),
                                Value::String(json) => json.clone(),
                                arguments => arguments.to_string(),
                            };
                            if !json.is_empty() {
                                events.push(delta(
                                    &message.id,
                                    Delta::ToolArgs(ToolArgsDelta {
                                        call_id: call.id.clone(),
                                        json,
                                    }),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Item::ToolResult(result) => {
                events.push(Event::ItemStarted(ItemStarted {
                    item_id: result.id.clone(),
                    kind: ItemKind::ToolResult,
                    tool: Some(result.tool.clone()),
                    call_id: Some(result.call_id.clone()),
                }));
                let output: String = result
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text(text) => Some(text.text.as_str()),
                        _ => None,
                    })
                    .collect();
                if !output.is_empty() {
                    events.push(delta(
                        &result.id,
                        Delta::ToolOutput(TextDelta { text: output }),
                    ));
                }
            }
            // Nothing else streams: it is drawn as it is.
            item => events.push(Event::ItemCompleted(ItemCompleted { item: item.clone() })),
        }
        for event in &events {
            self.event(transcript, event, now);
        }
    }

    fn delta(&mut self, transcript: &mut Transcript, item: &ItemId, delta: &Delta, now: Instant) {
        match delta {
            Delta::ToolOutput(output) => {
                let Some(call) = self.results.get(item).cloned() else {
                    return;
                };
                let Some(place) = self.calls.get(&call) else {
                    return;
                };
                if let Some(line) = self.call_mut(transcript, place) {
                    match &mut line.detail {
                        Detail::Output(text) => {
                            text.push_str(&output.text);
                            keep_tail(text);
                        }
                        detail => {
                            let mut text = output.text.clone();
                            keep_tail(&mut text);
                            *detail = Detail::Output(text);
                        }
                    }
                }
            }
            Delta::Reasoning(text) => {
                let response = self
                    .responses
                    .entry(item.clone())
                    .or_insert_with(Response::unseen);
                if response.resync {
                    return;
                }
                if let Some(answer) = response.answer.take() {
                    finish_answer(transcript, answer, &mut self.last_answer);
                }
                if let Some((block, _)) = response.reasoning
                    && let Some(Block::Reasoning(reasoning)) = transcript.get_mut(block)
                {
                    reasoning.text.push_str(&text.text);
                    return;
                }
                let block = transcript.push(Block::Reasoning(Reasoning {
                    text: text.text.clone(),
                    ..Reasoning::default()
                }));
                response.reasoning = Some((block, now));
            }
            Delta::Text(text) => {
                let response = self
                    .responses
                    .entry(item.clone())
                    .or_insert_with(Response::unseen);
                if response.resync {
                    return;
                }
                if let Some(reasoning) = response.reasoning.take() {
                    finish_reasoning(transcript, reasoning, now);
                }
                let answer = match response.answer {
                    Some(answer) => answer,
                    None => {
                        let answer = transcript.push(Block::Answer(
                            Markdown::answer()
                                .with_highlighter(self.highlighter.clone())
                                .keeping(),
                        ));
                        response.answer = Some(answer);
                        self.last_answer.clear();
                        answer
                    }
                };
                if let Some(Block::Answer(markdown)) = transcript.get_mut(answer) {
                    markdown.push(&text.text);
                }
                self.last_answer.push_str(&text.text);
                response.text += text.text.len();
            }
            Delta::ToolCall(call) => {
                let round = self.response_round(transcript, item, now);
                let response = self
                    .responses
                    .entry(item.clone())
                    .or_insert_with(Response::unseen);
                response.calls.push(call.call_id.clone());
                let index = match self.round_block(transcript, round) {
                    Some(Block::Round(boxed)) => {
                        boxed.calls.push(tools::started(&call.name));
                        boxed.calls.len() - 1
                    }
                    _ => return,
                };
                self.calls.insert(
                    call.call_id.clone(),
                    Place {
                        round,
                        index,
                        started: None,
                        ended: false,
                    },
                );
            }
            Delta::ToolArgs(args) => {
                let response = self
                    .responses
                    .entry(item.clone())
                    .or_insert_with(Response::unseen);
                response
                    .args
                    .entry(args.call_id.clone())
                    .or_default()
                    .push_str(&args.json);
            }
            _ => {}
        }
    }

    /// The response's round, made when its first call begins: its reasoning
    /// and its answer end there.
    fn response_round(&mut self, transcript: &mut Transcript, item: &ItemId, now: Instant) -> u64 {
        let response = self
            .responses
            .entry(item.clone())
            .or_insert_with(Response::unseen);
        if let Some(reasoning) = response.reasoning.take() {
            finish_reasoning(transcript, reasoning, now);
        }
        if let Some(answer) = response.answer.take() {
            finish_answer(transcript, answer, &mut self.last_answer);
        }
        if let Some(round) = response.round {
            return round;
        }
        let block = transcript.push(Block::Round(Round::default()));
        let number = self.next_round;
        self.next_round += 1;
        self.rounds.push((number, LiveRound { block }));
        response.round = Some(number);
        number
    }

    fn round_block<'t>(&self, transcript: &'t mut Transcript, round: u64) -> Option<&'t mut Block> {
        let (_, live) = self.rounds.iter().find(|(number, _)| *number == round)?;
        transcript.get_mut(live.block)
    }

    fn call_mut<'t>(
        &self,
        transcript: &'t mut Transcript,
        place: &Place,
    ) -> Option<&'t mut crate::view::Call> {
        match self.round_block(transcript, place.round)? {
            Block::Round(round) => round.calls.get_mut(place.index),
            _ => None,
        }
    }

    /// The response's partial content was thrown away, to be tried again:
    /// what it showed ends where it is, and its calls, which never ran,
    /// are marked so.
    fn reset(&mut self, transcript: &mut Transcript, item: &ItemId, now: Instant) {
        let Some(response) = self.responses.remove(item) else {
            return;
        };
        self.end_response(transcript, response, now);
        // The retried response streams under the same id, from its start.
        self.responses.insert(item.clone(), Response::default());
    }

    fn end_response(&mut self, transcript: &mut Transcript, response: Response, now: Instant) {
        if let Some(reasoning) = response.reasoning {
            finish_reasoning(transcript, reasoning, now);
        }
        if let Some(answer) = response.answer {
            finish_answer(transcript, answer, &mut self.last_answer);
        }
        for call in &response.calls {
            self.end_unstarted(transcript, call);
        }
    }

    /// A call that never began to run ends as interrupted.
    fn end_unstarted(&mut self, transcript: &mut Transcript, call: &CallId) {
        let Some(place) = self.calls.get(call) else {
            return;
        };
        if place.ended || place.started.is_some() {
            return;
        }
        if let Some(line) = self.call_mut(transcript, place)
            && matches!(line.state, CallState::Running(_) | CallState::Waiting)
        {
            line.state = CallState::Interrupted;
        }
        if let Some(place) = self.calls.get_mut(call) {
            place.ended = true;
        }
    }

    fn completed(&mut self, transcript: &mut Transcript, item: &Item, now: Instant) {
        match item {
            Item::UserMessage(message) => {
                transcript.push(Block::User(user_text(message)));
            }
            Item::AssistantMessage(message) => self.response_completed(transcript, message, now),
            Item::ToolResult(result) => self.result(transcript, result),
            Item::Notice(notice) if notice.source == NoticeSource::Map => {
                // The harness's own, given to the model: shown folded.
                let lines = notice.text.lines().count();
                transcript.push(Block::Notice(format!(
                    "The repository map was given to the model ({lines} lines)."
                )));
            }
            Item::Summary(_) => {
                transcript.push(Block::Notice(
                    "The conversation was summarised, and a new context begins with the summary."
                        .to_owned(),
                ));
            }
            // The other notices are the model's; the events tell the person.
            _ => {}
        }
    }

    fn response_completed(
        &mut self,
        transcript: &mut Transcript,
        message: &AssistantMessageItem,
        now: Instant,
    ) {
        let mut response = self.responses.remove(&message.id).unwrap_or_default();
        let text: String = message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect();
        // What did not arrive as deltas: a client behind (`item.resync`),
        // or a response that sent none.
        if response.text < text.len() && text.is_char_boundary(response.text) {
            let missing = &text[response.text..];
            if let Some(reasoning) = response.reasoning.take() {
                finish_reasoning(transcript, reasoning, now);
            }
            let answer = match response.answer {
                Some(answer) => answer,
                None => {
                    if response.text == 0 {
                        self.last_answer.clear();
                    }
                    let answer = transcript.push(Block::Answer(
                        Markdown::answer()
                            .with_highlighter(self.highlighter.clone())
                            .keeping(),
                    ));
                    response.answer = Some(answer);
                    answer
                }
            };
            if let Some(Block::Answer(markdown)) = transcript.get_mut(answer) {
                markdown.push(missing);
            }
            self.last_answer.push_str(missing);
        }
        // The calls, with their whole arguments; one that sent no delta is
        // added now.
        for part in &message.parts {
            let Part::ToolCall(call) = part else {
                continue;
            };
            if !self.calls.contains_key(&call.id) {
                let round = match response.round {
                    Some(round) => round,
                    None => {
                        if let Some(answer) = response.answer.take() {
                            finish_answer(transcript, answer, &mut self.last_answer);
                        }
                        let block = transcript.push(Block::Round(Round::default()));
                        let number = self.next_round;
                        self.next_round += 1;
                        self.rounds.push((number, LiveRound { block }));
                        response.round = Some(number);
                        number
                    }
                };
                let index = match self.round_block(transcript, round) {
                    Some(Block::Round(boxed)) => {
                        boxed.calls.push(tools::started(&call.name));
                        boxed.calls.len() - 1
                    }
                    _ => continue,
                };
                self.calls.insert(
                    call.id.clone(),
                    Place {
                        round,
                        index,
                        started: None,
                        ended: false,
                    },
                );
                response.calls.push(call.id.clone());
            }
            let arguments = match &call.arguments {
                Value::Null => response
                    .args
                    .get(&call.id)
                    .and_then(|text| serde_json::from_str(text).ok())
                    .unwrap_or(Value::Null),
                arguments => arguments.clone(),
            };
            if let Some(place) = self.calls.get(&call.id)
                && let Some(line) = self.call_mut(transcript, place)
            {
                line.target = tools::target(&call.name, &arguments);
            }
        }
        // A response that stopped part-way runs none of its calls.
        let stopped = matches!(
            message.stop_reason,
            Some(StopReason::Interrupted | StopReason::Error | StopReason::MalformedToolCall)
        );
        if stopped {
            let calls = std::mem::take(&mut response.calls);
            for call in &calls {
                self.end_unstarted(transcript, call);
            }
        }
        if let Some(reasoning) = response.reasoning.take() {
            finish_reasoning(transcript, reasoning, now);
        }
        if let Some(answer) = response.answer.take() {
            finish_answer(transcript, answer, &mut self.last_answer);
        }
        // A response's reasoning that sent no delta is not drawn: its text,
        // when it has any, is in the viewer.
    }

    fn result(&mut self, transcript: &mut Transcript, result: &ToolResultItem) {
        self.results.remove(&result.id);
        let resynced = self.resynced.remove(&result.id);
        if !self.calls.contains_key(&result.call_id) {
            // A result for a call this interface never saw begin: a box of
            // its own.
            let block = transcript.push(Block::Round(Round::default()));
            let number = self.next_round;
            self.next_round += 1;
            self.rounds.push((number, LiveRound { block }));
            if let Some(Block::Round(round)) = transcript.get_mut(block) {
                round.calls.push(tools::started(&result.tool));
            }
            self.calls.insert(
                result.call_id.clone(),
                Place {
                    round: number,
                    index: 0,
                    started: None,
                    ended: false,
                },
            );
        }
        let Some(place) = self.calls.get(&result.call_id) else {
            return;
        };
        if let Some(line) = self.call_mut(transcript, place) {
            // What streamed, unless some of it was dropped: then the
            // result's own text, which is whole.
            let output = match &line.detail {
                Detail::Output(text) if !resynced => text.clone(),
                _ => String::new(),
            };
            tools::finish(line, result, &output);
        }
        if let Some(place) = self.calls.get_mut(&result.call_id) {
            place.ended = true;
        }
    }

    /// The turn is over: what is still open ends as it is, and a call that
    /// never ended is interrupted.
    pub(crate) fn turn_done(&mut self, transcript: &mut Transcript, now: Instant) {
        let responses: Vec<Response> = self.responses.drain().map(|(_, r)| r).collect();
        for response in responses {
            self.end_response(transcript, response, now);
        }
        let open: Vec<CallId> = self
            .calls
            .iter()
            .filter(|(_, place)| !place.ended)
            .map(|(call, _)| call.clone())
            .collect();
        for call in open {
            if let Some(place) = self.calls.get(&call)
                && let Some(line) = self.call_mut(transcript, place)
                && matches!(line.state, CallState::Running(_) | CallState::Waiting)
            {
                line.state = CallState::Interrupted;
            }
        }
        self.calls.clear();
        self.results.clear();
    }
}

/// Reasoning ends: how long it took, and its text as the summary.
fn finish_reasoning(transcript: &mut Transcript, (block, began): (BlockId, Instant), now: Instant) {
    if let Some(Block::Reasoning(reasoning)) = transcript.get_mut(block) {
        reasoning.elapsed = now.saturating_duration_since(began);
        let text = reasoning.text.trim();
        reasoning.summary = (!text.is_empty()).then(|| text.to_owned());
        reasoning.finished = true;
    }
}

fn finish_answer(transcript: &mut Transcript, block: BlockId, _last: &mut String) {
    if let Some(Block::Answer(markdown)) = transcript.get_mut(block) {
        markdown.finish();
    }
}

/// What the person sent, as their message shows it.
fn user_text(message: &UserMessageItem) -> String {
    let mut text = String::new();
    for part in &message.parts {
        match part {
            Part::Text(part) => {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&part.text);
            }
            Part::Image(_) => {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str("[image]");
            }
            _ => {}
        }
    }
    text
}

/// Keeps the last [`OUTPUT_TAIL`] bytes, from a line's start where one is
/// near.
fn keep_tail(text: &mut String) {
    if text.len() <= OUTPUT_TAIL {
        return;
    }
    let mut cut = text.len() - OUTPUT_TAIL;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    if let Some(line) = text[cut..].find('\n') {
        cut += line + 1;
    }
    text.drain(..cut);
}

#[cfg(test)]
mod tests;
