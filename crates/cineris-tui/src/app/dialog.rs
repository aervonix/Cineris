//! A dialog in the input box's place: the request it answers, its keys, and
//! the guard that keeps typing meant for the editor from answering it
//! (`docs/steps/23/answered.md`, "The design, before building", points 2
//! and 3). It asks one of four things: may a tool call run (an approval),
//! may the project be trusted (trust), a question of the core's, or, the
//! application's own, whether to put the files back in a rewind (since step
//! 25: `docs/steps/25/answered.md`, point 4).
//!
//! **Only keys meant for it answer it.** A dialog can appear while the
//! person is typing: an approval while a steering message is being
//! written, the trust question just after the first frame. So a dialog
//! takes no key until it is *ready*, and its answers are drawn only then:
//! what can be pressed is what was shown, and what is shown can be pressed
//! (the owner's rule, after the review of step 23):
//!
//! - **The grace:** [`GRACE`](super::ready::GRACE) after its first frame was written.
//! - **The quiet:** [`QUIET`] with no input a dialog did not take: a key or
//!   a paste that reached the editor or the viewer, while the dialog was up
//!   or just before it came. Typing that goes on through the dialog keeps it
//!   from being ready, however slow, and so does typing that stopped less
//!   than a second before it came; it is ready a second after the typing
//!   stops. Answers shown before such input are hidden again.
//! - **After an answer:** the dialog after one a key answered is ready no
//!   sooner than [`REPEAT`] after that key, so that the key held a little
//!   long, or pressed twice, never answers a dialog not seen yet.
//! - **No repeat:** a key held down never answers. A repeat the terminal
//!   marks ([`Event::Repeat`](cineris_term::Event::Repeat), from the
//!   enhanced keyboard protocol's event types, for the keys with no text,
//!   and win32 input, for all) goes where its press went: the editor for
//!   typing, nowhere for a key a dialog took. Where the terminal does not
//!   mark them (the keys that type text, under the enhanced protocol too),
//!   a repeat is the same key again within [`REPEAT`], and the quiet keeps
//!   it from being taken: each repeat that reaches the editor holds the
//!   dialog a second more ([`QUIET`] is at least [`REPEAT`]), and after an
//!   answer the next dialog waits [`REPEAT`].
//! - **Keys read together** were typed together: they are input no dialog
//!   took, and none of them answers ([`super::App::burst`]).
//!
//! In the ordinary case (the dialog shows, the person reads it and presses
//! one key) the answers come [`GRACE`](super::ready::GRACE) after the dialog, and one key
//! answers. While it does not take keys, what types a character goes on to
//! the editor, where it was going, and the dialog's own other keys (`Esc`)
//! do nothing, so that a key meant to deny never interrupts the turn
//! instead. Once it takes keys, a key it has no use for that types a
//! character goes on to the editor, as input (typing that resumed after a
//! pause is kept, and hides the answers again); any other does nothing. A
//! paste never answers a dialog. After the program is continued (`fg`) or
//! the external editor gives the terminal back, the dialog starts its grace
//! again ([`Dialog::hide`]). All of these are comparisons of times; the one
//! wake they need is the moment the dialog is ready, to draw the answers
//! ([`Dialog::deadline`]). The times are kept by a
//! [`Readiness`](super::ready::Readiness), which the panels share (step 24).
//!
//! **Only what was seen can be pressed, and allowed for longer.** Only an
//! answer drawn whole answers (and `Esc`, which the hint row shows: deny,
//! not now, no answer). An answer that lasts (an approval for the session
//! or the project, trusting a project) is not offered for a request to
//! leave the sandbox, nor for one any of whose rows did not fit: nothing
//! that lasts is given for something the person could not read in full.
//! Keys that come in one read were typed together, and none of them
//! answers ([`super::App::burst`]).
//!
//! **It fits the window.** The question and the answers wrap, and are never
//! cut. The line of what a lasting answer would allow gives way first, then
//! the detail, cut with a line that says how much more there is. In a window
//! too small even for that, the rows between the question and the answers
//! are dropped; in one too small for those two, no answer is taken until it
//! grows.
//!
//! **A note is the note's.** Once "deny with a note" is chosen (or "answer"
//! to a question with no choices), keys and a paste go to the note, and its
//! row shows the note's end.
//!
//! **The rewind's question** is answered by its digits: `1` rewinds and puts
//! the plan's files back, `2` rewinds the conversation only, and `Esc`
//! cancels, sending nothing. No answer is preselected and `Enter` chooses
//! nothing. `1` puts files back, so it is offered by the rule of a lasting
//! answer: only once every file of the plan was drawn whole, in the dialog
//! or in the viewer (`Ctrl+O`, [`Dialog::seen_whole`]).

use std::time::Instant;

use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, ApprovalResponse, QuestionRequest,
    QuestionResponse, ServerResponse, TrustRequest, TrustResponse,
};

pub(crate) use crate::view::dialog::RewindAsk;
use cineris_term::{Key, Line};

#[cfg(any(test, feature = "test-kit"))]
pub(crate) use super::ready::GRACE;
use super::ready::Readiness;
/// The dialog's times, which the panels share (step 24): kept here by name
/// for what reads them as the dialog's.
pub(crate) use super::ready::{QUIET, REPEAT};
use super::tools;
use crate::keys::{Action, Context, KeyMap, name, typed};
use crate::view::dialog::{self, Answer, ApprovalView, QuestionView, RewindView, TrustView};
use crate::view::{DiffLine, Look};

/// The fewest rows a dialog's detail is given.
const LEAST_DETAIL: usize = 2;

/// The most choices of a question that a digit picks.
const CHOICES: usize = 9;

/// Which dialog it is: a request of the core's, answered through the link,
/// or a question of the application's own (since step 25).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DialogId {
    /// A request of the core's, by its id.
    Request(RequestId),
    /// The application's own question, by its number.
    Own(u64),
}

/// What a dialog asks.
#[derive(Debug, Clone)]
pub(crate) enum Asks {
    /// May this tool call run?
    Approval {
        request: ApprovalRequest,
        /// The diff's lines, parsed once.
        diff: Vec<DiffLine>,
    },
    /// May the project be trusted?
    Trust { request: TrustRequest },
    /// A question of the core's.
    Question { request: QuestionRequest },
    /// Whether to put the files back in a rewind (since step 25): the
    /// application's own question, which the core's plan fills.
    Rewind { ask: Box<RewindAsk> },
}

/// One answer a dialog offers: an action of its keys, or a question's
/// choice, by its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pick {
    Act(Action),
    Choice(usize),
}

/// What a dialog's rows were drawn for: the same again draws the same.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Drawn {
    rows: usize,
    width: usize,
    ready: bool,
    note: Option<String>,
}

/// How a dialog is drawn this time.
struct Shape {
    answers: Vec<Answer>,
    hidden: bool,
    detail_rows: usize,
    withheld: bool,
    /// The trust question's line of what each answer does is left out: it
    /// gives way before the detail, and takes no answer away.
    unsaid: bool,
}

/// One dialog. See the [module documentation](self).
#[derive(Debug, Clone)]
pub(crate) struct Dialog {
    pub(crate) id: DialogId,
    pub(crate) asks: Asks,
    /// When it is ready: its first frame, the input no dialog took while it
    /// was up or just before it came ([`QUIET`] after), the earliest it can
    /// be ready ([`REPEAT`] after a key that answered the dialog before it,
    /// so that the key held a little long, or pressed twice, never answers
    /// this one, which was not seen yet), and whether a frame with its
    /// answers has been drawn since nothing hid them.
    ready: Readiness,
    /// It asks to leave the sandbox of a session whose commands are
    /// confined.
    leaving: bool,
    /// The answers drawn whole the last time they were drawn: only these
    /// answer (and `Esc`, which the hint row shows).
    offered: Vec<Pick>,
    /// The window was too small for its question and its answers the last
    /// time they were to be drawn: no answer is taken until it is not.
    unreadable: bool,
    /// The note being written: after "deny with a note", or an answer to a
    /// question with no choices.
    note: Option<String>,
    /// Its rows as last drawn, and what for.
    cache: Option<(Drawn, Vec<Line>)>,
    /// Every row of it was drawn in the viewer (`Ctrl+O`): what the
    /// dialog had to cut has been seen (since step 25, the rewind's).
    seen_whole: bool,
    /// It took keys once: what is typed after that, at a pace, is typing
    /// that goes on where it was going.
    was_ready: bool,
}

/// What a key did to a dialog.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Keyed {
    /// It was not the dialog's: the editor takes it.
    Pass,
    /// A global action: the application does it.
    Global(Action),
    /// The dialog changed: draw it again.
    Changed,
    /// The dialog is answered.
    Answer(ServerResponse),
    /// The request is to be shown whole, in the viewer (`Ctrl+O`).
    ShowWhole,
    /// The rewind's question is answered (since step 25): with files put
    /// back or not, or cancelled (`None`), which sends nothing.
    Rewound(Option<bool>),
    /// Nothing.
    Nothing,
}

impl Dialog {
    fn new(id: DialogId, asks: Asks, leaving: bool) -> Self {
        Self {
            id,
            asks,
            ready: Readiness::default(),
            leaving,
            offered: Vec::new(),
            unreadable: false,
            note: None,
            cache: None,
            seen_whole: false,
            was_ready: false,
        }
    }

    /// An approval, in a session whose commands are `confined` or not.
    pub(crate) fn approval(id: RequestId, request: ApprovalRequest, confined: bool) -> Self {
        let diff = match &request.detail {
            ApprovalDetail::Diff(diff) => tools::diff_lines(&diff.diff),
            _ => Vec::new(),
        };
        let leaving = confined && dialog::outside_sandbox(&request);
        Self::new(
            DialogId::Request(id),
            Asks::Approval { request, diff },
            leaving,
        )
    }

    /// The trust question of a session being opened.
    pub(crate) fn trust(id: RequestId, request: TrustRequest) -> Self {
        Self::new(DialogId::Request(id), Asks::Trust { request }, false)
    }

    /// A question of the core's.
    pub(crate) fn question(id: RequestId, request: QuestionRequest) -> Self {
        Self::new(DialogId::Request(id), Asks::Question { request }, false)
    }

    /// The rewind's question, the application's own `number`th (since step
    /// 25).
    pub(crate) fn rewind(number: u64, ask: RewindAsk) -> Self {
        Self::new(
            DialogId::Own(number),
            Asks::Rewind { ask: Box::new(ask) },
            false,
        )
    }

    /// Whether it answers the core's request `id`.
    pub(crate) fn is_for(&self, id: &RequestId) -> bool {
        matches!(&self.id, DialogId::Request(own) if own == id)
    }

    /// The call the dialog is about, if it is about one.
    pub(crate) fn call(&self) -> Option<&cineris_protocol::CallId> {
        match &self.asks {
            Asks::Approval { request, .. } => Some(&request.call_id),
            Asks::Trust { .. } | Asks::Question { .. } | Asks::Rewind { .. } => None,
        }
    }

    /// Every row of it was drawn in the viewer: what the dialog cut has
    /// been seen, and what is offered only for what was seen whole (the
    /// rewind's `1`) is offered from now on.
    pub(crate) fn seen_whole(&mut self) {
        if !self.seen_whole {
            self.seen_whole = true;
            self.cache = None;
        }
    }

    /// When its first frame was written, if it has been.
    pub(crate) fn shown(&self) -> Option<Instant> {
        self.ready.shown()
    }

    /// Its first frame was written at `at`.
    pub(crate) fn drawn(&mut self, at: Instant) {
        self.ready.drawn(at);
    }

    /// The look changed: the rows are drawn again.
    pub(crate) fn restyled(&mut self) {
        self.cache = None;
    }

    /// Input no dialog took was read at `at`, while the dialog was up or
    /// just before it came: it is ready only [`QUIET`] after, and answers it
    /// showed are hidden.
    pub(crate) fn typed(&mut self, at: Instant) {
        self.was_ready |= self.takes_keys(at);
        if self.ready.typed(at) {
            self.cache = None;
        }
    }

    /// It is ready no earlier than `at`: a key answered the dialog before it
    /// at `at - REPEAT`.
    pub(crate) fn not_before(&mut self, at: Instant) {
        self.ready.not_before(at);
    }

    /// The dialog starts again as if just shown: its grace begins with its
    /// next frame, and its answers are hidden until then. After the program
    /// was continued (`fg`), or the external editor gave the terminal back:
    /// a key typed ahead meanwhile was not typed at this dialog.
    pub(crate) fn hide(&mut self) {
        if self.ready.hide() {
            self.cache = None;
        }
    }

    /// Whether the window was too small for its answers when they were
    /// due: it takes no key until a frame in a larger one draws them.
    pub(crate) fn unreadable(&self) -> bool {
        self.unreadable
    }

    /// When a frame is due for the dialog's answers: the moment it is
    /// ready, until a frame with them has been drawn. The time may have
    /// passed: a frame is then due at once.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        if self.ready.answers_drawn() || self.unreadable {
            return None;
        }
        self.ready.ready_at()
    }

    /// Whether the dialog takes keys at `at`: it is ready, and a frame has
    /// drawn its answers.
    pub(crate) fn takes_keys(&self, at: Instant) -> bool {
        self.ready.takes_keys(at)
    }

    /// Whether what is typed at `at` is the dialog's, to answer nothing:
    /// the application's own question (the rewind's), until it first takes
    /// keys. It was asked for a moment ago, from the picker, so nothing
    /// typed is on its way to the editor it hides, and what reached the
    /// editor would come back in front of the prompt the rewind gives back,
    /// for an `Enter` to send (the review of step 25's second pull
    /// request). Once it was ready, a draft typed at it goes to the editor
    /// as at any dialog.
    pub(crate) fn swallows(&self, at: Instant) -> bool {
        matches!(self.asks, Asks::Rewind { .. }) && !self.was_ready && !self.takes_keys(at)
    }

    /// What `Esc` (and the other keys of `deny`) gives: deny, not now, or no
    /// answer.
    fn refusal(&self) -> ServerResponse {
        match &self.asks {
            Asks::Approval { .. } => ServerResponse::Approval(ApprovalResponse {
                choice: ApprovalChoice::Deny,
                note: None,
            }),
            Asks::Trust { .. } => ServerResponse::Trust(TrustResponse::default()),
            // The rewind's `Esc` is `Keyed::Rewound(None)`, before this.
            Asks::Question { .. } | Asks::Rewind { .. } => {
                ServerResponse::Question(QuestionResponse { answer: None })
            }
        }
    }

    /// The answer the note gives, sent with `Enter`.
    fn with_note(&self, note: String) -> ServerResponse {
        let note = (!note.trim().is_empty()).then_some(note);
        match &self.asks {
            Asks::Question { .. } => ServerResponse::Question(QuestionResponse { answer: note }),
            Asks::Approval { .. } | Asks::Trust { .. } | Asks::Rewind { .. } => {
                ServerResponse::Approval(ApprovalResponse {
                    choice: ApprovalChoice::Deny,
                    note,
                })
            }
        }
    }

    /// A key, read at `at`, the key before it at `previous`; `repeat` when
    /// it is a key held down (see the [module documentation](self)).
    pub(crate) fn key(
        &mut self,
        key: Key,
        at: Instant,
        repeat: bool,
        after_taken: bool,
        keymap: &KeyMap,
    ) -> Keyed {
        // A global action is the application's whenever it comes: `Ctrl+C`
        // interrupts the turn, which withdraws the question.
        if let Some(
            action @ (Action::Interrupt | Action::Redraw | Action::Suspend | Action::ToggleRound),
        ) = keymap.lookup(&[], key)
        {
            return Keyed::Global(action);
        }
        // The note was asked for with a key the dialog took: what is typed
        // after it is the note's, at any pace.
        if let Some(note) = &mut self.note {
            return match keymap.lookup(&[Context::Editor], key) {
                Some(Action::Submit) => {
                    let note = std::mem::take(note);
                    self.note = None;
                    self.cache = None;
                    Keyed::Answer(self.with_note(note))
                }
                Some(Action::Stop) => {
                    self.note = None;
                    self.cache = None;
                    Keyed::Changed
                }
                Some(Action::Backspace) => {
                    note.pop();
                    self.cache = None;
                    Keyed::Changed
                }
                _ => match typed(key) {
                    Some(c) => {
                        note.push(c);
                        self.cache = None;
                        Keyed::Changed
                    }
                    None => Keyed::Nothing,
                },
            };
        }
        let action = keymap.lookup(&[Context::Dialog], key);
        // A key held down never answers: it goes where its first press
        // went, the editor for typing, nowhere for a key a dialog took.
        if repeat && self.takes_keys(at) {
            return if after_taken {
                Keyed::Nothing
            } else {
                Keyed::Pass
            };
        }
        if !self.takes_keys(at) {
            // Showing the request whole answers nothing: it is the
            // dialog's once its first frame is up.
            if action == Some(Action::ShowWhole)
                && typed(key).is_none()
                && self.ready.shown().is_some()
                && matches!(
                    self.asks,
                    Asks::Approval { .. } | Asks::Trust { .. } | Asks::Rewind { .. }
                )
            {
                return Keyed::ShowWhole;
            }
            // The rewind's question: every key waits for it, and holds its
            // quiet as typing does.
            if self.swallows(at) {
                return Keyed::Nothing;
            }
            // Typing goes on where it was going; the dialog's own keys that
            // type nothing (`Esc`) do nothing yet, and nor do its answer keys
            // right after a key a dialog took (approving one dialog after
            // another quickly is not typing into the hidden editor).
            return if action.is_some_and(dialogs) && (typed(key).is_none() || after_taken) {
                Keyed::Nothing
            } else {
                Keyed::Pass
            };
        }
        // Only what was drawn whole can be pressed.
        let drawn = |pick: Pick| self.offered.contains(&pick);
        // The rewind's question (since step 25): its two answers by their
        // digits, and `Esc` cancels, sending nothing. `Enter` is nothing.
        if let Asks::Rewind { .. } = &self.asks {
            if let Some(place) = typed(key)
                .and_then(|c| c.to_digit(10))
                .and_then(|digit| digit.checked_sub(1))
                && drawn(Pick::Choice(place as usize))
            {
                return Keyed::Rewound(Some(place == 0));
            }
            return match action {
                Some(Action::Deny) => Keyed::Rewound(None),
                Some(Action::ShowWhole) => Keyed::ShowWhole,
                _ => unused(key),
            };
        }
        // A question's choice, by its digit.
        if let Asks::Question { request } = &self.asks
            && let Some(digit) = typed(key).and_then(|c| c.to_digit(10))
            && let Some(place) = (digit as usize).checked_sub(1)
            && drawn(Pick::Choice(place))
        {
            return Keyed::Answer(ServerResponse::Question(QuestionResponse {
                answer: request.choices.get(place).cloned(),
            }));
        }
        let Some(action) = action else {
            return unused(key);
        };
        match (&self.asks, action) {
            // `Esc`, and the other keys of `deny`, are always on offer: no
            // answer is what each of them is, and the hint row shows it.
            (_, Action::Deny) => Keyed::Answer(self.refusal()),
            (Asks::Approval { .. } | Asks::Trust { .. }, Action::ShowWhole) => Keyed::ShowWhole,
            (Asks::Approval { .. }, action) if drawn(Pick::Act(action)) => match action {
                Action::AllowOnce => Keyed::Answer(approve(ApprovalChoice::AllowOnce)),
                Action::AllowSession => Keyed::Answer(approve(ApprovalChoice::AllowSession)),
                Action::AllowProject => Keyed::Answer(approve(ApprovalChoice::AllowProject)),
                Action::DenyWithNote => {
                    self.note = Some(String::new());
                    self.cache = None;
                    Keyed::Changed
                }
                _ => unused(key),
            },
            (Asks::Trust { request }, action) if drawn(Pick::Act(action)) => match action {
                Action::AllowOnce => Keyed::Answer(ServerResponse::Trust(TrustResponse {
                    trusted: true,
                    ..TrustResponse::default()
                })),
                // The third answer: never, of a project not trusted; keep
                // trusting it without what is new, of one trusted already
                // (the follow-ups of step 23).
                Action::Never => Keyed::Answer(ServerResponse::Trust(TrustResponse {
                    never: !request.trusted,
                    keep: request.trusted,
                    ..TrustResponse::default()
                })),
                _ => unused(key),
            },
            (Asks::Question { .. }, Action::AllowOnce) if drawn(Pick::Act(Action::AllowOnce)) => {
                self.note = Some(String::new());
                self.cache = None;
                Keyed::Changed
            }
            _ => unused(key),
        }
    }

    /// A paste: the note's while one is being written, and then it says
    /// so; otherwise never the dialog's.
    /// The note being written, taken: the dialog is closed from outside.
    pub(crate) fn take_note(&mut self) -> Option<String> {
        self.cache = None;
        self.note.take()
    }

    pub(crate) fn paste(&mut self, text: &str) -> bool {
        let Some(note) = &mut self.note else {
            return false;
        };
        note.push_str(text);
        self.cache = None;
        true
    }

    /// Whether a note is being written in it: its keys are then the
    /// note's.
    pub(crate) fn writing_note(&self) -> bool {
        self.note.is_some()
    }

    /// The keys the hint row shows while the dialog is open.
    pub(crate) fn hint_keys(&self, keymap: &KeyMap) -> Vec<String> {
        if self.note.is_some() {
            let enter = match &self.asks {
                Asks::Question { .. } => "enter sends the answer",
                Asks::Approval { .. } | Asks::Trust { .. } | Asks::Rewind { .. } => {
                    "enter denies with the note"
                }
            };
            return vec![enter.to_owned(), "esc goes back".to_owned()];
        }
        let deny = keymap
            .keys_of(Action::Deny)
            .into_iter()
            .map(name)
            .find(|text| text == "esc")
            .or_else(|| keymap.keys_of(Action::Deny).into_iter().next().map(name))
            .unwrap_or_else(|| "esc".to_owned());
        let what = match &self.asks {
            Asks::Approval { .. } => "denies",
            Asks::Trust { .. } => "is not now",
            Asks::Question { .. } => "leaves it unanswered",
            Asks::Rewind { .. } => "cancels",
        };
        let mut keys = vec![format!("{deny} {what}")];
        if matches!(
            self.asks,
            Asks::Approval { .. } | Asks::Trust { .. } | Asks::Rewind { .. }
        ) && let Some(whole) = keymap.keys_of(Action::ShowWhole).into_iter().next()
        {
            keys.push(format!("{} shows it whole", name(whole)));
        }
        keys
    }

    /// The request drawn whole, `width` columns wide, for the viewer: its
    /// question and all its detail, without its answers.
    pub(crate) fn whole(&self, width: usize, look: &Look) -> Vec<Line> {
        let mut lines = self
            .render(
                &Shape {
                    answers: Vec::new(),
                    hidden: true,
                    detail_rows: usize::MAX,
                    withheld: false,
                    unsaid: false,
                },
                width,
                look,
            )
            .lines;
        while lines.last().is_some_and(|line| line.width() == 0) {
            lines.pop();
        }
        lines
    }

    /// The dialog's rows at `now`, at most `rows` of them and `width`
    /// columns wide. Its answers are drawn once they can be pressed.
    pub(crate) fn view(
        &mut self,
        keymap: &KeyMap,
        rows: usize,
        width: usize,
        look: &Look,
        now: Instant,
    ) -> Vec<Line> {
        let ready = self.ready.ready(now);
        let drawn_for = Drawn {
            rows,
            width,
            ready,
            note: self.note.clone(),
        };
        if let Some((key, lines)) = &self.cache
            && *key == drawn_for
        {
            return lines.clone();
        }
        let (lines, readable, offered) = self.draw(keymap, rows, width, look, ready);
        self.ready.set_answers_drawn(ready && readable);
        // Too small is known only once the answers are due: before, the
        // loop is still woken for them.
        self.unreadable = ready && !readable;
        if ready {
            self.offered = offered;
        }
        self.cache = Some((drawn_for, lines.clone()));
        lines
    }

    /// The dialog drawn in `shape`.
    fn render(&self, shape: &Shape, width: usize, look: &Look) -> dialog::Drawn {
        match &self.asks {
            Asks::Approval { request, diff } => dialog::approval(
                request,
                &ApprovalView {
                    answers: shape.answers.clone(),
                    answers_hidden: shape.hidden,
                    diff,
                    detail_rows: shape.detail_rows,
                    note: self.note.as_deref(),
                    leaving: self.leaving,
                    withheld: shape.withheld,
                },
                width,
                look,
            ),
            Asks::Trust { request } => dialog::trust(
                request,
                &TrustView {
                    answers: shape.answers.clone(),
                    answers_hidden: shape.hidden,
                    detail_rows: shape.detail_rows,
                    unsaid: shape.unsaid,
                },
                width,
                look,
            ),
            Asks::Question { request } => dialog::question(
                request,
                &QuestionView {
                    answers: shape.answers.clone(),
                    answers_hidden: shape.hidden,
                    detail_rows: shape.detail_rows,
                    note: self.note.as_deref(),
                },
                width,
                look,
            ),
            Asks::Rewind { ask } => dialog::rewind(
                ask,
                &RewindView {
                    answers: shape.answers.clone(),
                    answers_hidden: shape.hidden,
                    detail_rows: shape.detail_rows,
                },
                width,
                look,
            ),
        }
    }

    /// Whether the dialog has a line of what a lasting answer would allow,
    /// which can give way before its detail does.
    fn says_what_lasts(&self) -> bool {
        matches!(&self.asks, Asks::Approval { request, .. } if !request.adds.is_empty())
            && !self.leaving
    }

    /// Draws the dialog to fit `rows`. Says whether its question and its
    /// answers could both be shown, and which answers were drawn whole.
    fn draw(
        &self,
        keymap: &KeyMap,
        rows: usize,
        width: usize,
        look: &Look,
        ready: bool,
    ) -> (Vec<Line>, bool, Vec<Pick>) {
        // Rows are kept for every answer there could be; fewer take no more.
        let every = self.answers(keymap, false);
        let mut shape = Shape {
            answers: every.iter().map(|(_, answer)| answer.clone()).collect(),
            hidden: !ready,
            detail_rows: rows.max(LEAST_DETAIL),
            withheld: false,
            unsaid: false,
        };
        // The note of what a lasting answer would allow gives way first, so
        // that the detail stays whole (no lasting answer is offered then),
        // and then the detail.
        let mut fitted = self.render(&shape, width, look);
        loop {
            let over = fitted.lines.len().saturating_sub(rows);
            if over == 0 {
                break;
            }
            let room = fitted.detail.min(shape.detail_rows);
            if !shape.withheld && self.says_what_lasts() {
                shape.withheld = true;
            } else if !shape.unsaid && matches!(self.asks, Asks::Trust { .. }) {
                shape.unsaid = true;
            } else if room > LEAST_DETAIL {
                shape.detail_rows = room.saturating_sub(over).max(LEAST_DETAIL);
            } else {
                break;
            }
            fitted = self.render(&shape, width, look);
        }
        let fits = fitted.lines.len() <= rows;
        // A lasting answer is offered only for what was seen whole, itself
        // included.
        let lasting_cut = every
            .iter()
            .zip(&fitted.whole)
            .any(|((pick, _), whole)| !whole && self.lasts(*pick));
        let clipped = fitted.clipped || shape.withheld || !fits || lasting_cut;
        let mut drawn = fitted;
        let mut offered = Vec::new();
        if ready {
            let answers = self.answers(keymap, clipped);
            shape.answers = answers.iter().map(|(_, answer)| answer.clone()).collect();
            shape.hidden = false;
            drawn = self.render(&shape, width, look);
            offered = answers
                .iter()
                .zip(&drawn.whole)
                .filter(|(_, whole)| **whole)
                .map(|((pick, _), _)| *pick)
                .collect();
        }
        let (question, answer_rows) = (drawn.question, drawn.answer_rows);
        let mut lines = drawn.lines;
        let mut readable = true;
        if lines.len() > rows {
            // Still too tall: its question, and its last rows, which hold
            // the answers. In a window too small for both, nothing it shows
            // can be pressed.
            readable = rows >= question + answer_rows;
            let head = question.min(rows);
            let tail = lines.split_off(lines.len() - (rows - head));
            lines.truncate(head);
            lines.extend(tail);
        }
        (lines, readable, offered)
    }

    /// Whether `pick` gives something that lasts: an approval for the
    /// session or the project, or trust in the project.
    fn lasts(&self, pick: Pick) -> bool {
        match &self.asks {
            Asks::Approval { .. } => {
                matches!(pick, Pick::Act(Action::AllowSession | Action::AllowProject))
            }
            Asks::Trust { .. } => pick == Pick::Act(Action::AllowOnce),
            Asks::Question { .. } => false,
            // Putting files back: offered only for a plan seen whole.
            Asks::Rewind { .. } => pick == Pick::Choice(0),
        }
    }

    /// The answers on offer, each with what picks it and its key as the key
    /// map names it.
    fn answers(&self, keymap: &KeyMap, clipped: bool) -> Vec<(Pick, Answer)> {
        let key = |action: Action| {
            keymap
                .keys_of(action)
                .into_iter()
                .map(name)
                .find(|text| text.chars().count() == 1)
                .or_else(|| keymap.keys_of(action).into_iter().next().map(name))
                .unwrap_or_default()
        };
        let act = |action: Action, label: &str| {
            (
                Pick::Act(action),
                Answer {
                    key: key(action),
                    label: label.to_owned(),
                },
            )
        };
        match &self.asks {
            Asks::Approval { request, .. } => {
                let lasting = !(self.leaving || clipped);
                let mut answers = Vec::new();
                for choice in &request.choices {
                    let (action, label) = match choice {
                        ApprovalChoice::AllowOnce if self.leaving => {
                            (Action::AllowOnce, "Run once outside the sandbox")
                        }
                        ApprovalChoice::AllowOnce => (Action::AllowOnce, "Allow once"),
                        ApprovalChoice::AllowSession if lasting => {
                            (Action::AllowSession, "Allow this session")
                        }
                        ApprovalChoice::AllowProject if lasting => {
                            (Action::AllowProject, "Always allow here")
                        }
                        ApprovalChoice::Deny => (Action::Deny, "Deny"),
                        _ => continue,
                    };
                    answers.push(act(action, label));
                }
                if !self.leaving && request.choices.contains(&ApprovalChoice::Deny) {
                    answers.push(act(Action::DenyWithNote, "Deny with a note"));
                }
                answers
            }
            Asks::Trust { request } => {
                let mut answers = Vec::new();
                if !clipped {
                    answers.push(act(Action::AllowOnce, "Trust it"));
                }
                answers.push(act(Action::Deny, "Not now"));
                answers.push(act(Action::Never, dialog::third_trust_answer(request)));
                answers
            }
            // `1` only once every file of the plan was drawn whole, here
            // or in the viewer.
            Asks::Rewind { .. } => {
                let choice = |place: usize, label: &str| {
                    (
                        Pick::Choice(place),
                        Answer {
                            key: (place + 1).to_string(),
                            label: label.to_owned(),
                        },
                    )
                };
                let mut answers = Vec::new();
                if !clipped || self.seen_whole {
                    answers.push(choice(0, "Rewind it and put back the files listed"));
                }
                answers.push(choice(
                    1,
                    "Rewind the conversation only; the files stay as they are",
                ));
                answers
            }
            Asks::Question { request } if request.choices.is_empty() => vec![
                act(Action::AllowOnce, "Answer"),
                act(Action::Deny, "Leave it unanswered"),
            ],
            Asks::Question { request } => request
                .choices
                .iter()
                .take(CHOICES)
                .enumerate()
                .map(|(place, choice)| {
                    (
                        Pick::Choice(place),
                        Answer {
                            key: (place + 1).to_string(),
                            // From outside, as every text a view draws.
                            label: cineris_term::sanitize(choice).replace(['\n', '\t'], " "),
                        },
                    )
                })
                .collect(),
        }
    }
}

/// A key a dialog that takes keys has no use for: one that types a
/// character goes on to the editor, as typing that resumed after a pause
/// (and so is input, which hides the answers again for a second); any
/// other does nothing (since the review of step 23).
fn unused(key: Key) -> Keyed {
    if typed(key).is_some() {
        Keyed::Pass
    } else {
        Keyed::Nothing
    }
}

/// An approval's answer with no note.
fn approve(choice: ApprovalChoice) -> ServerResponse {
    ServerResponse::Approval(ApprovalResponse { choice, note: None })
}

/// Whether `action` is one of a dialog's own, not a global one.
fn dialogs(action: Action) -> bool {
    matches!(
        action,
        Action::AllowOnce
            | Action::AllowSession
            | Action::AllowProject
            | Action::Deny
            | Action::DenyWithNote
            | Action::Never
            | Action::ShowWhole
    )
}

#[cfg(test)]
mod tests;
