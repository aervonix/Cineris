//! `/rewind`, fork, and the transcript read again after a rewind or an undo
//! (since step 25; `docs/steps/25/answered.md`, point 4;
//! `docs/architecture/09-tui.md`, "Rewind and fork").
//!
//! Every step is an operation through the [`Link`](super::Link), so that
//! `cineris attach` rewinds and forks as the in-process interface does:
//!
//! 1. **The points.** `/rewind` reads the active branch with `session/read`
//!    (a read beside the lane), page by page, and keeps of each prompt
//!    that began a turn only its row and the ids a rewind and a fork need,
//!    for as long as the picker is open. The first prompt is not a point:
//!    going back before it is `/new`. The picker lists them newest first.
//! 2. **The plan.** A point chosen asks the core what a rewind to just
//!    before its prompt would do (`session/rewind_plan` with `before` the
//!    prompt's item: since the review of step 25's second pull request, the
//!    state before the prompt's turn, not after the item before it),
//!    and reads the prompt whole (`session/read` of its item), for the
//!    editor afterwards.
//! 3. **The question,** in the input box's place: the plan's files, then
//!    `1` (rewind, files put back), `2` (the conversation only), `Esc`
//!    (nothing sent). It is a dialog, with the dialogs' rules of keys
//!    (`super::super::dialog`). `1` and `2` send `session/rewind` with the
//!    plan's mark: a plan that moved since is the core's to refuse
//!    (`plan_changed`), and the refusal is said, and the prompts sent
//!    meanwhile come back into the editor. Keys typed before the question
//!    is ready are the question's, and answer nothing: they would land in
//!    the hidden editor, in front of the prompt given back.
//! 4. **After it,** the prompt goes back into the editor, after what is
//!    being written there; inline a notice says the scrollback above it is
//!    no longer the conversation; and the transcript the viewer and the
//!    full screen draw is read again from the active branch (an undo's too),
//!    so that the turns undone are gone there, and `/copy` copies the last
//!    answer of the branch. A prompt sent from the answer to the end of the
//!    read waits for it, so that its turn is not read twice; `Ctrl+C` gives
//!    such prompts back.
//!
//! **Fork:** `Tab` in the `/rewind` picker forks from before the point
//! (`session/fork` with `before` the prompt's item); in `/sessions` it
//! forks the selected session at its end. The fork is opened in this
//! one's place, and the session it came from is left as it was. A fork
//! that fails gives the prompts sent meanwhile back into the editor.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use cineris_protocol::event::{Event, ItemCompleted};
use cineris_protocol::item::{Delivery as ItemDelivery, UserMessageItem};
use cineris_protocol::message::Part;
use cineris_protocol::methods::{
    Op, SessionForkParams, SessionForkResult, SessionReadParams, SessionReadResult,
    SessionRewindParams, SessionRewindPlanParams, SessionRewindPlanResult,
};
use cineris_protocol::session::{SessionState, SessionSummary};
use cineris_protocol::{ErrorData, Item, ItemId, SessionId, Timestamp};

use super::{App, HISTORY_PAGE, Pending};
use crate::app::dialog::{Dialog, RewindAsk};
use crate::app::items::Mapper;
use crate::complete::{Menu, Row};
use crate::transcript::{Block, Transcript};
use crate::view::Header;

/// The most bytes of a prompt a picker's row holds: more than a row shows.
const ROW_TEXT: usize = 512;

/// How far the clock of a transcript read again is put ahead, so that a
/// code line whose colours are not in yet is kept as late, not waited for.
const LONG_AGO: Duration = Duration::from_secs(24 * 60 * 60);

/// A point of the session to go back to: a prompt that began a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Point {
    /// The turn it began, counted from one along the active branch.
    pub(crate) turn: u64,
    /// The prompt's item: a rewind and a fork go back to just before it.
    pub(crate) item: ItemId,
    /// When it was sent, as a row says it (`14:32`), when that is known.
    pub(crate) time: Option<String>,
    /// The prompt on one line, cut to a row's worth: the picker's row.
    pub(crate) text: String,
    /// The prompt's first line, cut: what the question shows of it.
    pub(crate) first: String,
}

/// The points being read for the `/rewind` picker.
#[derive(Debug)]
pub(super) struct Reading {
    session: SessionId,
    points: Vec<Point>,
    /// The turns counted so far.
    turns: u64,
}

/// A rewind on its way, from the point chosen to the rewind's answer.
#[derive(Debug)]
pub(super) struct Rewinding {
    session: SessionId,
    point: Point,
    /// The prompt whole, once read: it goes back into the editor.
    prompt: Option<String>,
    /// The mark of the plan shown, once the plan has come.
    mark: Option<String>,
    /// Whether the rewind was sent: from then on a prompt waits for it,
    /// and for the transcript read again after it (the review of step 25's
    /// second pull request).
    sent: bool,
}

/// The transcript the viewer and the full screen draw, being read again
/// from the active branch after a rewind or an undo.
#[derive(Debug)]
pub(super) struct Rereading {
    session: SessionId,
    /// Which read this is: a page of an earlier one is not this one's.
    number: u64,
    /// The kept blocks before this place are replaced: what was kept after
    /// it (the rewind's notice, and on) is kept after what is read.
    from: usize,
    transcript: Transcript,
    mapper: Mapper,
    /// The items whose events came while it is read: they are drawn from
    /// those, and not again from the read.
    seen: HashSet<String>,
    /// The turns counted, for the viewer's title.
    turns: u64,
}

/// What a fork answers to.
#[derive(Debug, Clone)]
pub(super) struct Forking {
    /// The session forked, open here until the fork is.
    from: SessionState,
    /// The item the fork was asked to end just before: the result must
    /// give it back, or the fork holds all of the session.
    before: Option<ItemId>,
    /// What is said once the fork is open.
    said: String,
}

impl App {
    // ----------------------------------------------------------- points

    /// `/rewind`: the session's earlier prompts are read, for the picker.
    pub(super) fn rewind_command(&mut self) {
        if self.running.is_some() {
            self.say("A turn is running: wait for it to end, or stop it with esc, then rewind.");
            return;
        }
        if self.held_sent.is_some() {
            self.say("Your prompts are still being sent: /rewind can go back once they are.");
            return;
        }
        if self.opening || self.reading {
            self.say("A session is being opened: /rewind can go back once it is.");
            return;
        }
        if self.rewind_under_way() || self.undoing() {
            self.say("A rewind is under way: /rewind can go back again once it is done.");
            return;
        }
        let Some(session) = self.needs_session("rewind") else {
            return;
        };
        self.points = Some(Reading {
            session: session.clone(),
            points: Vec::new(),
            turns: 0,
        });
        self.read_page(session, None, Pending::Points);
    }

    /// Whether a rewind is being chosen, asked about, done, or drawn again.
    pub(super) fn rewind_under_way(&self) -> bool {
        self.points.is_some() || self.rewinding.is_some() || self.rereading.is_some()
    }

    /// Whether a prompt sent now waits: a rewind was sent and its result
    /// is not in, or the transcript is being read again after it. A prompt
    /// whose events came before the rewind's result would be drawn from
    /// them and again from the read.
    pub(super) fn holds_prompts_for_rewind(&self) -> bool {
        self.rereading.is_some()
            || self
                .rewinding
                .as_ref()
                .is_some_and(|rewinding| rewinding.sent)
    }

    /// Whether an undo is on its way: its result is drawn again too, so a
    /// second change of the history waits for it (the review of this part).
    pub(super) fn undoing(&self) -> bool {
        self.pending
            .values()
            .any(|pending| matches!(pending, Pending::Undo(_)))
    }

    /// Asks for a page of `session`'s active branch, its result for `pending`.
    fn read_page(
        &mut self,
        session: SessionId,
        cursor: Option<String>,
        pending: fn(SessionId) -> Pending,
    ) {
        let ticket = self.link.send(Op::SessionRead(SessionReadParams {
            session_id: session.clone(),
            item_id: None,
            limit: Some(HISTORY_PAGE),
            cursor,
            redacted: false,
        }));
        self.pending.insert(ticket, pending(session));
    }

    /// A page of the points' read: each prompt that began a turn, but the
    /// first, is a point. At the last page the picker opens, newest first.
    pub(super) fn points_read(&mut self, session: &SessionId, page: SessionReadResult) {
        let Some(mut reading) = self
            .points
            .take()
            .filter(|reading| reading.session == *session)
        else {
            return;
        };
        for item in page.items {
            if let Item::UserMessage(message) = &item
                && message.delivery.as_ref() != Some(&ItemDelivery::Steer)
            {
                reading.turns += 1;
                if reading.turns > 1 {
                    let time = message
                        .created_at
                        .as_ref()
                        .and_then(|at| self.local_time(at));
                    let ellipsis = self.look.glyphs.ellipsis;
                    reading.points.push(Point {
                        turn: reading.turns,
                        item: message.id.clone(),
                        time,
                        text: row_text(message, ellipsis),
                        first: first_line(message, ellipsis),
                    });
                }
            }
        }
        if let Some(cursor) = page.next_cursor {
            self.points = Some(reading);
            self.read_page(session.clone(), Some(cursor), Pending::Points);
            return;
        }
        if reading.points.is_empty() {
            self.say("There is no earlier prompt to go back to: /new starts afresh.");
            return;
        }
        let mut points = reading.points;
        points.reverse();
        let rows = points.iter().map(point_row).collect();
        self.open_picker(
            "rewind",
            super::Picker::Rewind(points),
            Menu::picker("rewind", rows, 0, None).forking("fork from here"),
        );
    }

    /// The points' read failed: it is said, and nothing is opened.
    pub(super) fn points_unread(&mut self, session: &SessionId, error: &ErrorData) {
        if self
            .points
            .as_ref()
            .is_some_and(|reading| reading.session == *session)
        {
            self.points = None;
            self.error(error);
        }
    }

    /// The local time of day of `at`, `14:32`, in the zone the viewer's
    /// title uses (read once).
    fn local_time(&mut self, at: &Timestamp) -> Option<String> {
        let time = cineris_base::UtcTime::parse(at.as_str()).ok()?;
        let zone = self.zone.get_or_insert_with(cineris_base::Zone::system);
        let local = time.unix_secs() + i64::from(zone.offset_at(time));
        let minutes = local.div_euclid(60);
        Some(format!(
            "{:02}:{:02}",
            minutes.div_euclid(60).rem_euclid(24),
            minutes.rem_euclid(60)
        ))
    }

    // ---------------------------------------------------- plan, question

    /// A point was chosen: its plan is asked for, and its prompt read whole.
    pub(super) fn rewind_to(&mut self, point: Point) {
        let Some(session) = self.needs_session("rewind") else {
            return;
        };
        let ticket = self.link.send(Op::SessionRead(SessionReadParams {
            session_id: session.clone(),
            item_id: Some(point.item.clone()),
            limit: None,
            cursor: None,
            redacted: false,
        }));
        self.pending
            .insert(ticket, Pending::RewindPrompt(session.clone()));
        let ticket = self
            .link
            .send(Op::SessionRewindPlan(SessionRewindPlanParams {
                session_id: session.clone(),
                to: None,
                before: Some(point.item.clone()),
            }));
        self.pending
            .insert(ticket, Pending::RewindPlan(session.clone()));
        self.rewinding = Some(Rewinding {
            session,
            point,
            prompt: None,
            mark: None,
            sent: false,
        });
    }

    /// The chosen prompt, read whole: the point's own item, and no other.
    pub(super) fn rewind_prompt_read(&mut self, session: &SessionId, page: &SessionReadResult) {
        let Some(rewinding) = self.rewinding_of(session) else {
            return;
        };
        let text = page.items.iter().find_map(|item| match item {
            Item::UserMessage(message) if message.id == rewinding.point.item => {
                Some(prompt_text(message))
            }
            _ => None,
        });
        rewinding.prompt = text;
    }

    /// The rewind on its way for `session`, if there is one.
    fn rewinding_of(&mut self, session: &SessionId) -> Option<&mut Rewinding> {
        self.rewinding
            .as_mut()
            .filter(|rewinding| rewinding.session == *session)
    }

    /// The plan came: the question takes the input box's place.
    pub(super) fn rewind_planned(&mut self, session: &SessionId, plan: SessionRewindPlanResult) {
        let Some(rewinding) = self.rewinding_of(session) else {
            return;
        };
        rewinding.mark = Some(plan.mark.clone());
        // The turns undone are the plan's, which the mark holds: one run
        // since the points were read is among them, and said.
        let ask = RewindAsk {
            turn: rewinding.point.turn,
            prompt: rewinding.point.first.clone(),
            plan,
        };
        self.own_dialogs += 1;
        let dialog = Dialog::rewind(self.own_dialogs, ask);
        self.show_dialog(dialog);
    }

    /// The plan could not be made, or the rewind was refused: why is said,
    /// and nothing more is done. The prompts sent after the answer were
    /// meant for the conversation rewound: they come back into the editor.
    pub(super) fn rewind_failed(&mut self, session: &SessionId, error: &ErrorData) {
        if self.rewinding_of(session).is_some() {
            self.rewinding = None;
            self.error(error);
            if !self.held_prompts.is_empty() && self.held_sent.is_none() {
                self.give_back_held();
                self.stop_running();
            }
        }
    }

    /// The question answered: `1` (files put back) or `2` sends the rewind
    /// with the plan's mark; `Esc` sends nothing.
    pub(super) fn rewind_answered(&mut self, choice: Option<bool>) {
        self.dialogs.pop_front();
        let Some(restore_files) = choice else {
            self.rewinding = None;
            return;
        };
        let Some(rewinding) = self.rewinding.as_mut() else {
            return;
        };
        rewinding.sent = true;
        let ticket = self.link.send(Op::SessionRewind(SessionRewindParams {
            session_id: rewinding.session.clone(),
            to: None,
            before: Some(rewinding.point.item.clone()),
            restore_files,
            plan: rewinding.mark.clone(),
        }));
        self.pending
            .insert(ticket, Pending::Rewind(rewinding.session.clone()));
    }

    /// The question was closed from outside (a turn ended meanwhile): the
    /// rewind is not done.
    pub(super) fn rewind_question_closed(&mut self) {
        if self.rewinding.take().is_some() {
            self.say(
                "A turn ended while the rewind was asked about, so nothing was rewound: /rewind \
                 shows the points as they are now.",
            );
        }
    }

    /// The rewind is done: its prompt goes back into the editor, a notice
    /// says where the conversation goes on from, and the transcript is read
    /// again.
    pub(super) fn rewound(&mut self, session: &SessionId, state: SessionState) {
        let Some(rewinding) = self.rewinding.take() else {
            return;
        };
        if rewinding.session != *session {
            return;
        }
        self.session = Some(state);
        let turn = rewinding.point.turn;
        let kept = format!("Rewound to before turn {turn}. The conversation goes on from there.");
        if self.full_screen() {
            self.say(&kept);
        } else {
            // The scrollback cannot be taken back: the notice says what
            // above it no longer counts. The viewer, which draws the
            // transcript read again, keeps the shorter one.
            let inline = format!(
                "Rewound to before turn {turn}. The conversation goes on from there; what is \
                 above this line is no longer part of it."
            );
            self.say(&inline);
            self.rewound_notice = Some((inline, kept));
        }
        match &rewinding.prompt {
            Some(prompt) => self.give_back(prompt),
            None => self
                .say("The prompt of that turn could not be read, so it is not in the input box."),
        }
        self.read_again(session.clone());
    }

    // ------------------------------------------------------- read again

    /// The transcript the viewer and the full screen draw is read again
    /// from the active branch: after a rewind, or an undo.
    pub(super) fn read_again(&mut self, session: SessionId) {
        let Some(state) = self.session.clone() else {
            return;
        };
        let mut transcript = Transcript::new();
        transcript.keep_finished();
        transcript.push(Block::Header(self.header_of(&state)));
        self.rereads += 1;
        let number = self.rereads;
        self.rereading = Some(Rereading {
            session: session.clone(),
            number,
            from: self.kept.len(),
            transcript,
            mapper: Mapper::new(self.mapper.highlighter().clone()),
            seen: HashSet::new(),
            turns: 0,
        });
        self.read_reread_page(session, number, None);
    }

    /// Asks for a page of the transcript read again, read `number`.
    fn read_reread_page(&mut self, session: SessionId, number: u64, cursor: Option<String>) {
        let ticket = self.link.send(Op::SessionRead(SessionReadParams {
            session_id: session.clone(),
            item_id: None,
            limit: Some(HISTORY_PAGE),
            cursor,
            redacted: false,
        }));
        self.pending
            .insert(ticket, Pending::Reread(session, number));
    }

    /// An item whose events came while the transcript is read again.
    pub(super) fn seen_while_read_again(&mut self, session: &SessionId, item: &str) {
        if let Some(rereading) = self
            .rereading
            .as_mut()
            .filter(|rereading| rereading.session == *session)
        {
            rereading.seen.insert(item.to_owned());
        }
    }

    /// A page of the transcript read again. At the last, the kept
    /// transcript is replaced by it, and the prompts sent meanwhile go.
    pub(super) fn reread_page(
        &mut self,
        session: &SessionId,
        number: u64,
        page: SessionReadResult,
        now: Instant,
    ) {
        let Some(rereading) = self
            .rereading
            .as_mut()
            .filter(|rereading| rereading.session == *session && rereading.number == number)
        else {
            return;
        };
        for item in page.items {
            let Some(id) = item.id().map(str::to_owned) else {
                continue;
            };
            // Drawn from its events, or to be: not twice.
            if rereading.seen.contains(&id)
                || page.incomplete.iter().any(|open| open.as_str() == id)
            {
                continue;
            }
            if let Item::UserMessage(message) = &item
                && message.delivery.as_ref() != Some(&ItemDelivery::Steer)
            {
                rereading.turns += 1;
            }
            rereading.mapper.event(
                &mut rereading.transcript,
                &Event::ItemCompleted(ItemCompleted { item }),
                now,
            );
        }
        if let Some(cursor) = page.next_cursor {
            self.read_reread_page(session.clone(), number, Some(cursor));
            return;
        }
        let Some(mut rereading) = self.rereading.take() else {
            return;
        };
        rereading.mapper.turn_done(&mut rereading.transcript, now);
        rereading.transcript.set_now(LONG_AGO);
        let _ = rereading
            .transcript
            .commit_parts(self.width.max(1), &self.look);
        let head = rereading.transcript.take_finished();
        self.kept.replace_before(rereading.from, head);
        // `/copy` copies the branch's last answer, not one undone, unless
        // an answer came from its events while the branch was read.
        if rereading.seen.is_empty() {
            self.mapper
                .set_last_answer(rereading.mapper.last_answer().to_owned());
        }
        self.turns = (rereading.turns > 0).then_some((1, rereading.turns));
        self.kept_replaced();
        self.send_held_after_reread(now);
    }

    /// The transcript could not be read again: why is said, the views keep
    /// what they had, and the prompts sent meanwhile go.
    pub(super) fn reread_failed(
        &mut self,
        session: &SessionId,
        number: u64,
        error: &ErrorData,
        now: Instant,
    ) {
        if self
            .rereading
            .as_ref()
            .is_some_and(|rereading| rereading.session == *session && rereading.number == number)
        {
            self.rereading = None;
            self.error(error);
            self.send_held_after_reread(now);
        }
    }

    /// The prompts held while the transcript was read again go, in order.
    fn send_held_after_reread(&mut self, now: Instant) {
        if self.held_prompts.is_empty() {
            return;
        }
        self.send_held(true, now);
        if self.held_sent.is_none() {
            self.stop_running();
        }
    }

    /// The kept blocks were replaced: the views drawn from them start
    /// again, and late code is asked for its colours again.
    fn kept_replaced(&mut self) {
        let highlighter = self.mapper.highlighter().clone();
        for recolouring in std::mem::take(&mut self.recolouring) {
            highlighter.close(recolouring.id());
        }
        self.redraw_blocks.clear();
        let stops = self.kept.stops();
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.kept_replaced(stops);
        }
        if let Some(screen) = self.screen.as_mut() {
            *screen = super::screen::ScreenView::default();
        }
        self.ask_colours(0);
    }

    /// What is kept of a finished block: the inline rewind's notice is kept
    /// in its shorter words, which hold in the viewer too.
    pub(super) fn kept_words(
        &mut self,
        mut finished: Vec<crate::transcript::kept::Finished>,
    ) -> Vec<crate::transcript::kept::Finished> {
        let Some((inline, kept)) = self.rewound_notice.as_ref() else {
            return finished;
        };
        let mut found = false;
        for block in &mut finished {
            if let Block::Notice(text) = &mut block.block
                && text == inline
            {
                text.clone_from(kept);
                found = true;
            }
        }
        if found {
            self.rewound_notice = None;
        }
        finished
    }

    /// The session's header card, as an open draws it.
    pub(super) fn header_of(&self, state: &SessionState) -> Header {
        let main = state.roles.get(&cineris_protocol::model::ModelRole::Main);
        Header {
            version: self.version.clone(),
            model: main
                .map(|role| role.model.as_str().to_owned())
                .unwrap_or_default(),
            effort: main.and_then(|role| role.effort_in_effect.clone().or(role.effort.clone())),
            folder: self.folder.clone(),
            branch: None,
            profile: None,
        }
    }

    /// The zone times are said in, for a test that says which.
    #[cfg(test)]
    pub(crate) fn set_zone(&mut self, zone: cineris_base::Zone) {
        self.zone = Some(zone);
    }

    /// Forgets every rewind under way: the session was left.
    pub(super) fn forget_rewind(&mut self) {
        self.points = None;
        self.rewinding = None;
        self.rereading = None;
        self.rewound_notice = None;
    }

    // -------------------------------------------------------------- fork

    /// `Tab` in a picker that forks: `/rewind`'s forks from just before the
    /// point's prompt, `/sessions`' forks the session at its end.
    pub(super) fn forked(&mut self, name: &str, index: usize) {
        let Some((opened, picker)) = self.picker.take() else {
            return;
        };
        if opened != name {
            return;
        }
        match picker {
            super::Picker::Rewind(points) => {
                let Some(point) = points.get(index) else {
                    return;
                };
                self.fork_here(
                    Some(point.item.clone()),
                    format!(
                        "Forked from before turn {}: this is a new session, and the one it came \
                         from is kept as it was.",
                        point.turn
                    ),
                );
            }
            super::Picker::Sessions(sessions) => {
                let Some(summary) = sessions.get(index) else {
                    return;
                };
                let said = format!(
                    "Forked {}: this is a new session with all of its history, and the one it \
                     came from is kept as it was.",
                    session_name(summary, self.look.glyphs.ellipsis)
                );
                if self.session_id().as_ref() == Some(&summary.session_id) {
                    self.fork_here(None, said);
                } else {
                    // Said once the fork is open, in its place.
                    let id = summary.session_id.clone();
                    if self.leave_for(super::Open::Fork(id)) {
                        self.open_said = Some(said);
                    }
                }
            }
            _ => {}
        }
    }

    /// Forks the session open here from just before the item `before` (at
    /// its end when `None`), and opens the fork in its place. Until the
    /// fork's answer, what comes is held, and a prompt sent waits for the
    /// fork.
    fn fork_here(&mut self, before: Option<ItemId>, said: String) {
        if self.running.is_some() {
            self.say("A turn is running: wait for it to end, or stop it with esc, then fork.");
            return;
        }
        if self.held_sent.is_some() {
            self.say("Your prompts are still being sent: a fork can be made once they are.");
            return;
        }
        if self.opening || self.reading || self.rewind_under_way() || self.undoing() {
            self.say("The session is being read: a fork can be made once it is.");
            return;
        }
        let Some(state) = self.session.take() else {
            self.say("There is no session to fork yet.");
            return;
        };
        self.opening = true;
        let ticket = self.link.send(Op::SessionFork(SessionForkParams {
            session_id: state.session_id.clone(),
            at: None,
            before: before.clone(),
        }));
        self.pending.insert(
            ticket,
            Pending::Fork(Box::new(Forking {
                from: state,
                before,
                said,
            })),
        );
    }

    /// The fork is open: the session it came from is closed here, and the
    /// fork drawn in its place. A fork asked to end before a prompt whose
    /// result does not say it does holds all of the session: a core that
    /// does not know `before` forks it all, and what is said says so (the
    /// review of the fixes of step 25's second pull request).
    pub(super) fn fork_opened(
        &mut self,
        forking: Forking,
        forked: SessionForkResult,
        now: Instant,
    ) {
        self.forked_from = Some(forking.from.session_id);
        if forking.before.is_some() && forked.before != forking.before {
            self.say(
                "Forked with all of the session's history: the core did not end the fork before \
                 that turn, and may be older than this client. This is a new session, and the \
                 one it came from is kept as it was.",
            );
        } else {
            self.say(&forking.said);
        }
        self.had_turn = false;
        self.last_cost = None;
        self.usage = None;
        self.opened(forked.session, true, now);
    }

    /// The fork failed: the session forked is the one open again, what was
    /// held while it was asked for follows, and why is said.
    pub(super) fn fork_failed(&mut self, forking: Forking, error: &ErrorData, now: Instant) {
        self.opening = false;
        self.session = Some(forking.from);
        let held = std::mem::take(&mut self.held);
        for event in held {
            self.event(event, now);
        }
        self.error(error);
        // Prompts sent while the fork was asked for were meant for the
        // fork: they come back into the editor, as an open that failed
        // gives them back, and do not run at the end of the session that
        // stays (the review of step 25's second pull request).
        self.give_back_held();
        self.stop_running();
    }
}

/// A prompt's text: its text parts, one after another.
fn prompt_text(message: &UserMessageItem) -> String {
    let parts: Vec<&str> = message
        .parts
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();
    parts.join("\n")
}

/// A prompt as a picker's row holds it: on one line, cut to a row's worth
/// with `ellipsis`.
fn row_text(message: &UserMessageItem, ellipsis: &str) -> String {
    let text = prompt_text(message);
    let mut end = text.len().min(ROW_TEXT);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut row: String = text[..end].split_whitespace().collect::<Vec<_>>().join(" ");
    if end < text.len() {
        row.push_str(ellipsis);
    }
    row
}

/// A prompt's first line, cut to a row's worth, with `ellipsis` when there
/// is more.
fn first_line(message: &UserMessageItem, ellipsis: &str) -> String {
    let text = prompt_text(message);
    let first = text.trim_start().lines().next().unwrap_or_default();
    let mut end = first.len().min(ROW_TEXT);
    while !first.is_char_boundary(end) {
        end -= 1;
    }
    let mut line = first[..end].trim_end().to_owned();
    if end < text.trim().len() {
        line.push_str(ellipsis);
    }
    line
}

/// A point's row: `turn 12  14:32  fix the lexer…`. The menu sanitises it
/// and cuts it at the row.
fn point_row(point: &Point) -> Row {
    let label = match &point.time {
        Some(time) => format!("turn {}  {time}  {}", point.turn, point.text),
        None => format!("turn {}  {}", point.turn, point.text),
    };
    Row::new(label, "")
}

/// A session as a notice names it: its title in quotes, or its id.
fn session_name(summary: &SessionSummary, ellipsis: &str) -> String {
    match &summary.title {
        Some(title) => format!("\"{}\"", super::one_line_cut(title, 40, ellipsis)),
        None => summary.session_id.as_str().to_owned(),
    }
}
