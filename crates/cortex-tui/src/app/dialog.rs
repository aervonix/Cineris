//! A dialog in the input box's place: the request it answers, its keys, and
//! the guard that keeps typing meant for the editor from answering it
//! (`docs/steps/23/answered.md`, "The design, before building", points 2
//! and 3). It asks one of three things: may a tool call run (an approval),
//! may the project be trusted (trust), or a question of the core's.
//!
//! **Only keys meant for it answer it.** A dialog can appear while the
//! person is typing: an approval while a steering message is being
//! written, the trust question just after the first frame. So a dialog
//! takes no key until it is *ready*, and its answers are drawn only then:
//! what can be pressed is what was shown, and what is shown can be pressed
//! (the owner's rule, after the review of step 23):
//!
//! - **The grace:** [`GRACE`] after its first frame was written.
//! - **The quiet:** [`QUIET`] with no input a dialog did not take: a key or
//!   a paste that reached the editor or the viewer, while the dialog was up
//!   or just before it came. Typing that goes on through the dialog keeps it
//!   from being ready, however slow, and so does typing that stopped less
//!   than a second before it came; it is ready a second after the typing
//!   stops. Answers shown before such input are hidden again.
//! - **After an answer:** the dialog after one a key answered is ready no
//!   sooner than [`REPEAT`] after that key, so that the key held a little
//!   long, or pressed twice, never answers a dialog not seen yet.
//! - **The pause:** a key read less than [`GRACE`] after the key before it
//!   carries on the typing, whatever the dialog shows; it goes on to the
//!   editor, and so hides the answers again.
//! - **No repeat:** a key held down never answers. A repeat the terminal
//!   marks ([`Event::Repeat`](cortex_term::Event::Repeat), from the
//!   enhanced keyboard protocol's event types, for the keys with no text,
//!   and win32 input, for all) is dropped, and so is the same key as the
//!   one before it within [`REPEAT`] when that one was typing, not a key a
//!   dialog took: where the terminal does not mark repeats (the keys that
//!   type text, under the enhanced protocol too), a key held from before
//!   the dialog came looks like that ([`super::App`] tells). The quiet and
//!   the wait after an answer keep such a key from ever being taken.
//! - **Keys read together** were typed together, and none of them answers
//!   ([`super::App::burst`]).
//!
//! In the ordinary case (the dialog shows, the person reads it and presses
//! one key) the answers come [`GRACE`] after the dialog, and one key
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
//! ([`Dialog::deadline`]).
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

use std::time::{Duration, Instant};

use cortex_protocol::jsonrpc::RequestId;
use cortex_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, ApprovalResponse, QuestionRequest,
    QuestionResponse, ServerResponse, TrustRequest, TrustResponse,
};
use cortex_term::{Key, Line};

use super::tools;
use crate::keys::{Action, Context, KeyMap, name, typed};
use crate::view::dialog::{self, Answer, ApprovalView, QuestionView, TrustView};
use crate::view::{DiffLine, Look};

/// How long after a dialog is first drawn, and after the key before, a key
/// still goes to the editor.
pub(crate) const GRACE: Duration = Duration::from_millis(300);

/// How long with no input a dialog that typing reached waits before it
/// takes keys (the owner's "about 1 s", after the review of step 23). Even
/// a slow typist's gaps inside a sentence are shorter: 40 words a minute is
/// a key every 300 ms, 15 a minute one every 800 ms. And a second is short
/// beside the time it takes to read a dialog, so a person who stopped
/// typing to read it does not wait for it.
pub(crate) const QUIET: Duration = Duration::from_millis(1000);

/// How long after a key the same key again is taken for that key held
/// down, where the terminal does not say: terminals repeat a held key
/// after 250 ms to 1 s (X11's default is 660 ms, Windows' and macOS's
/// about 500 ms), then every 25 to 50 ms.
pub(crate) const REPEAT: Duration = Duration::from_millis(1000);

/// The fewest rows a dialog's detail is given.
const LEAST_DETAIL: usize = 2;

/// The most choices of a question that a digit picks.
const CHOICES: usize = 9;

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
}

/// One dialog. See the [module documentation](self).
#[derive(Debug, Clone)]
pub(crate) struct Dialog {
    pub(crate) id: RequestId,
    pub(crate) asks: Asks,
    /// When its first frame was written.
    pub(crate) shown: Option<Instant>,
    /// When input last reached the editor while the dialog was up: it is
    /// ready [`QUIET`] after.
    typed: Option<Instant>,
    /// The earliest it can be ready: [`REPEAT`] after a key that answered
    /// the dialog before it, so that the key held a little long, or pressed
    /// twice, never answers this one, which was not seen yet.
    not_before: Option<Instant>,
    /// A frame with its answers has been drawn, and nothing hid them since.
    answers_drawn: bool,
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
    /// Nothing.
    Nothing,
}

impl Dialog {
    fn new(id: RequestId, asks: Asks, leaving: bool) -> Self {
        Self {
            id,
            asks,
            shown: None,
            typed: None,
            not_before: None,
            answers_drawn: false,
            leaving,
            offered: Vec::new(),
            unreadable: false,
            note: None,
            cache: None,
        }
    }

    /// An approval, in a session whose commands are `confined` or not.
    pub(crate) fn approval(id: RequestId, request: ApprovalRequest, confined: bool) -> Self {
        let diff = match &request.detail {
            ApprovalDetail::Diff(diff) => tools::diff_lines(&diff.diff),
            _ => Vec::new(),
        };
        let leaving = confined && dialog::outside_sandbox(&request);
        Self::new(id, Asks::Approval { request, diff }, leaving)
    }

    /// The trust question of a session being opened.
    pub(crate) fn trust(id: RequestId, request: TrustRequest) -> Self {
        Self::new(id, Asks::Trust { request }, false)
    }

    /// A question of the core's.
    pub(crate) fn question(id: RequestId, request: QuestionRequest) -> Self {
        Self::new(id, Asks::Question { request }, false)
    }

    /// The call the dialog is about, if it is about one.
    pub(crate) fn call(&self) -> Option<&cortex_protocol::CallId> {
        match &self.asks {
            Asks::Approval { request, .. } => Some(&request.call_id),
            Asks::Trust { .. } | Asks::Question { .. } => None,
        }
    }

    /// Its first frame was written at `at`.
    pub(crate) fn drawn(&mut self, at: Instant) {
        if self.shown.is_none() {
            self.shown = Some(at);
        }
    }

    /// The look changed: the rows are drawn again.
    pub(crate) fn restyled(&mut self) {
        self.cache = None;
    }

    /// Input reached the editor at `at` while the dialog was up: it is
    /// ready only [`QUIET`] after, and answers it showed are hidden.
    pub(crate) fn typed(&mut self, at: Instant) {
        self.typed = Some(self.typed.map_or(at, |typed| typed.max(at)));
        self.hide_answers();
    }

    /// It is ready no earlier than `at`: a key answered the dialog before it
    /// at `at - REPEAT`.
    pub(crate) fn not_before(&mut self, at: Instant) {
        self.not_before = Some(self.not_before.map_or(at, |before| before.max(at)));
    }

    /// The dialog starts again as if just shown: its grace begins with its
    /// next frame, and its answers are hidden until then. After the program
    /// was continued (`fg`), or the external editor gave the terminal back:
    /// a key typed ahead meanwhile was not typed at this dialog.
    pub(crate) fn hide(&mut self) {
        self.shown = None;
        self.typed = None;
        self.hide_answers();
    }

    fn hide_answers(&mut self) {
        if self.answers_drawn {
            self.answers_drawn = false;
            self.cache = None;
        }
    }

    /// When the dialog is ready to take keys and show its answers: the end
    /// of its grace, and [`QUIET`] after typing reached the editor while it
    /// was up. `None` until its first frame.
    fn ready_at(&self) -> Option<Instant> {
        let ready = (self.shown? + GRACE).max(self.not_before.unwrap_or(self.shown?));
        Some(self.typed.map_or(ready, |typed| ready.max(typed + QUIET)))
    }

    /// When a frame is due for the dialog's answers: the moment it is
    /// ready, until a frame with them has been drawn. The time may have
    /// passed: a frame is then due at once.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        if self.answers_drawn || self.unreadable {
            return None;
        }
        self.ready_at()
    }

    /// Whether the dialog takes keys at `at`: it is ready, and a frame has
    /// drawn its answers.
    pub(crate) fn takes_keys(&self, at: Instant) -> bool {
        self.answers_drawn && self.ready_at().is_some_and(|ready| at >= ready)
    }

    /// Whether a key read at `at` may answer the dialog, the key before it
    /// having been read at `previous`: once it takes keys, and after a
    /// pause.
    fn takes_keys_at(&self, at: Instant, previous: Option<Instant>) -> bool {
        self.takes_keys(at)
            && previous.is_none_or(|previous| at.saturating_duration_since(previous) >= GRACE)
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
            Asks::Question { .. } => ServerResponse::Question(QuestionResponse { answer: None }),
        }
    }

    /// The answer the note gives, sent with `Enter`.
    fn with_note(&self, note: String) -> ServerResponse {
        let note = (!note.trim().is_empty()).then_some(note);
        match &self.asks {
            Asks::Question { .. } => ServerResponse::Question(QuestionResponse { answer: note }),
            Asks::Approval { .. } | Asks::Trust { .. } => {
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
        previous: Option<Instant>,
        repeat: bool,
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
        // A key held down answers nothing, and once the dialog takes keys
        // it goes nowhere: its first press went where it went.
        if repeat && self.takes_keys(at) {
            return Keyed::Nothing;
        }
        if !self.takes_keys_at(at, previous) {
            // Showing the request whole answers nothing: it is the
            // dialog's once its first frame is up.
            if action == Some(Action::ShowWhole)
                && typed(key).is_none()
                && self.shown.is_some()
                && matches!(self.asks, Asks::Approval { .. } | Asks::Trust { .. })
            {
                return Keyed::ShowWhole;
            }
            // Typing goes on where it was going; the dialog's own keys that
            // type nothing (`Esc`) do nothing yet.
            return if typed(key).is_none() && action.is_some_and(dialogs) {
                Keyed::Nothing
            } else {
                Keyed::Pass
            };
        }
        // Only what was drawn whole can be pressed.
        let drawn = |pick: Pick| self.offered.contains(&pick);
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
            (Asks::Trust { .. }, action) if drawn(Pick::Act(action)) => match action {
                Action::AllowOnce => Keyed::Answer(ServerResponse::Trust(TrustResponse {
                    trusted: true,
                    never: false,
                })),
                Action::Never => Keyed::Answer(ServerResponse::Trust(TrustResponse {
                    trusted: false,
                    never: true,
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

    /// The keys the hint row shows while the dialog is open.
    pub(crate) fn hint_keys(&self, keymap: &KeyMap) -> Vec<String> {
        if self.note.is_some() {
            let enter = match &self.asks {
                Asks::Question { .. } => "enter sends the answer",
                Asks::Approval { .. } | Asks::Trust { .. } => "enter denies with the note",
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
        };
        let mut keys = vec![format!("{deny} {what}")];
        if matches!(self.asks, Asks::Approval { .. } | Asks::Trust { .. })
            && let Some(whole) = keymap.keys_of(Action::ShowWhole).into_iter().next()
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
        let ready = self.ready_at().is_some_and(|ready| now >= ready);
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
        self.answers_drawn = ready && readable;
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
            Asks::Trust { .. } => {
                let mut answers = Vec::new();
                if !clipped {
                    answers.push(act(Action::AllowOnce, "Trust it"));
                }
                answers.push(act(Action::Deny, "Not now"));
                answers.push(act(Action::Never, "Never for this project"));
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
                            label: cortex_term::sanitize(choice).replace(['\n', '\t'], " "),
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
