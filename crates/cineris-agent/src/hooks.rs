//! Hooks in the loop (`docs/architecture/12-extensions.md`, "Tier 3:
//! hooks"; `docs/steps/28/answered.md`, "6. Hooks in the loop").
//!
//! The loop reaches hooks through one seam, [`Hooks`], beside
//! [`Policy`](crate::Policy). The core implements it with the user's and a
//! project's `[hooks.<name>]` and the hooks of the extensions' scripts; the
//! loop knows none of that. It asks, at each point, whether any hook wants
//! the event ([`Hooks::wants`], cheap and synchronous); with none, the point
//! is passed at once, with no await. With some, it runs each one the
//! implementation matched ([`Hook::run`]), reads each answer by the event's
//! contract, and acts on what they say together.
//!
//! # What the loop holds every hook to
//!
//! - **Hooks on one event run together,** and their answers are combined so
//!   that order never matters: `deny` over `ask` over `allow`.
//! - **A hook only narrows.** It can refuse a call or a prompt, or turn a
//!   call the rules allow into a question. `allow` never lifts a question
//!   the rules put. A call the rules refused, or that stopped the turn, is
//!   not shown to hooks.
//! - **A replacement of a call's arguments is new work nobody chose,** so a
//!   person always chooses it: it is checked against the tool's schema,
//!   planned, decided by the rules again, held to the protected paths
//!   ([`Hooks::check_replacement`]), and then put to the user whatever the
//!   rules would have said. Two hooks that both replace the arguments
//!   refuse the call.
//! - **A hook that fails refuses, where hooks gate.** One that fails, does
//!   not answer within its time, or answers with anything outside its
//!   event's contract counts as `deny` on `prompt.submit` and
//!   `tool.before`, with the failure named. The answer is read strictly on
//!   every event: a field the event does not take, a value of the wrong
//!   type, or a decision it does not know is a failure. On the events that
//!   only add (`tool.after`, `turn.end`, `compaction.before`) a failure is
//!   shown to the user and adds nothing.
//! - **What a hook adds is labelled with its name:** a note on a call's
//!   result, a notice for the next turn, instructions for a summary, each
//!   under a line that names the hook and says the text is the hook's.
//! - **An observer is never waited for,** and its answer is not read. It
//!   runs beside the loop, to its own time; a failure is logged.
//! - **Every wait is on the session's timer,** so tests use the fake one.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Duration;

use cineris_base::{AbsPath, CancelToken, SharedTimer};
use cineris_protocol::event::TurnOutcome;
use cineris_protocol::item::{Delivery, HookReason, ReplacedArguments, ToolOutcome};
use cineris_protocol::{CallId, Part, SessionId, TurnId};
use cineris_tools::{BoxFuture, Effect, Json};

use crate::policy::PlannedCall;

/// How long the loop waits for a hook that is waited for, when it names no
/// time of its own.
pub const WAIT_DEFAULT: Duration = Duration::from_secs(5);

/// The longest the loop waits for a hook that is waited for, whatever time
/// it names.
pub const WAIT_MAX: Duration = Duration::from_secs(60);

/// How long an observer may run, when it names no time of its own.
pub const OBSERVE_DEFAULT: Duration = Duration::from_secs(60);

/// The longest an observer may run, whatever time it names.
pub const OBSERVE_MAX: Duration = Duration::from_secs(600);

/// The most of a hook's `reason` or `context` that is kept, in bytes: the
/// rest is cut, and the cut is marked.
pub const TEXT_CAP: usize = 8 * 1024;

/// The most of a replacement's arguments that its call's result shows the
/// model, in bytes of JSON.
const SHOWN_ARGUMENTS: usize = 4 * 1024;

/// The largest replacement of a call's arguments, in bytes of JSON: one
/// larger is refused before it is put in a question or an item. (A hook's
/// whole answer is smaller than this, as the core reads it.)
pub const ARGUMENTS_CAP: usize = 256 * 1024;

/// The most runs of one observer that go on at once, by its name: a run
/// beyond them is skipped, and the skip logged.
pub const OBSERVE_CONCURRENT: usize = 4;

/// The tool that calls a dynamic tool by name: a replacement of its
/// arguments may not name another tool, and a hook's filter matches the
/// tool it names as well as it.
const TOOL_CALL: &str = "tool_call";

/// An event of the loop that hooks may be on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum HookEvent {
    /// A prompt is taken to start a turn, or a steering message is
    /// delivered. A hook may refuse it, or add context after it.
    PromptSubmit,
    /// A tool call was planned and the rules allowed it, or will ask about
    /// it; nothing has asked or run yet. A hook may refuse it, ask about it,
    /// or replace its arguments.
    ToolBefore,
    /// A tool call ran, and its result is not recorded yet. A hook may add a
    /// note to the result. Waited for.
    ToolAfter,
    /// A turn ended, whatever its outcome. A hook may add a notice for the
    /// next turn. Not waited for: the notice is delivered at the next
    /// turn's start if it is in by then, else at the next safe boundary.
    TurnEnd,
    /// A summary is about to be requested: by hand, at the threshold, after
    /// a request did not fit, or in the background. A hook may add
    /// instructions for the summary. Waited for.
    CompactionBefore,
}

impl HookEvent {
    /// The event's name, as a hook's configuration and its input say it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PromptSubmit => "prompt.submit",
            Self::ToolBefore => "tool.before",
            Self::ToolAfter => "tool.after",
            Self::TurnEnd => "turn.end",
            Self::CompactionBefore => "compaction.before",
        }
    }

    /// Whether its hooks gate: their `deny` and their failure refuse what
    /// they are asked about. On the other events they only add.
    #[must_use]
    pub const fn gates(self) -> bool {
        matches!(self, Self::PromptSubmit | Self::ToolBefore)
    }
}

/// What set off a compaction, as a `compaction.before` hook is told it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompactionTrigger {
    /// The user asked for it (`session/compact`).
    Manual,
    /// The next request was past the compaction threshold.
    Threshold,
    /// The provider said a request did not fit.
    Overflow,
    /// A summary prepared in the background, ahead of need.
    Background,
}

impl CompactionTrigger {
    /// The trigger as a hook's input says it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Threshold => "threshold",
            Self::Overflow => "overflow",
            Self::Background => "background",
        }
    }
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The call a tool event is about, as a hook's filter matches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolTarget<'a> {
    /// The tool's name.
    pub tool: &'a str,
    /// For a `tool_call`: the tool it names (`source/tool`), which a filter
    /// matches as it matches `tool_call` itself. `None` for any other call,
    /// and for a `tool_call` whose arguments name none.
    pub names: Option<&'a str>,
}

impl<'a> ToolTarget<'a> {
    /// The target of a call of `tool` with `arguments`.
    #[must_use]
    pub fn of(tool: &'a str, arguments: &'a Json) -> Self {
        Self {
            tool,
            names: named_tool(tool, arguments),
        }
    }
}

/// The tool a `tool_call` names, trimmed as `tool_call` trims it.
fn named_tool<'a>(tool: &str, arguments: &'a Json) -> Option<&'a str> {
    if tool != TOOL_CALL {
        return None;
    }
    arguments.get("tool").and_then(Json::as_str).map(str::trim)
}

/// Everything a hook is given: the event, where it happened, and the
/// event's own fields. [`to_json`](Self::to_json) is the one object both
/// forms of hook receive, a command on its standard input and a script's
/// function as its argument.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HookInput {
    /// The event.
    pub event: HookEvent,
    /// The session.
    pub session: SessionId,
    /// The turn.
    pub turn: TurnId,
    /// The workspace's root.
    pub workspace: AbsPath,
    /// What the event is about.
    pub subject: HookSubject,
}

/// What an event is about.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum HookSubject {
    /// A prompt (`prompt.submit`).
    Prompt {
        /// What the user sent.
        parts: Vec<Part>,
        /// How it was sent, when a turn was running: `steer` for a steering
        /// message.
        delivery: Option<Delivery>,
    },
    /// A tool call (`tool.before`).
    Call {
        /// The tool's name.
        tool: String,
        /// The provider's id for the call.
        call_id: CallId,
        /// The arguments, as the model wrote them.
        arguments: Json,
        /// The plan in one line.
        summary: String,
        /// What the call's plan says it touches, its paths resolved: what a
        /// command's `{path}` is filled from.
        effects: Vec<Effect>,
    },
    /// A tool call that ran, and its result (`tool.after`).
    Ran {
        /// The tool's name.
        tool: String,
        /// The provider's id for the call.
        call_id: CallId,
        /// The arguments it ran with: a hook's replacement, if one was
        /// allowed, and otherwise the model's.
        arguments: Json,
        /// The plan in one line.
        summary: String,
        /// What its plan said it touches, its paths resolved.
        effects: Vec<Effect>,
        /// How it ended.
        outcome: ToolOutcome,
        /// The text of its result, as the model will be shown it.
        result: String,
    },
    /// A turn that ended (`turn.end`).
    TurnEnded {
        /// How it ended.
        outcome: TurnOutcome,
        /// The code of the error it ended with, if it ended with one.
        error: Option<String>,
    },
    /// A summary about to be requested (`compaction.before`).
    Compaction {
        /// What set it off.
        trigger: CompactionTrigger,
        /// What the user asked the summary to keep in particular.
        focus: Option<String>,
    },
}

impl HookInput {
    /// The input of `event` about `subject`: for an implementation's own
    /// tests, since the loop makes every one it gives.
    #[must_use]
    pub fn new(
        event: HookEvent,
        session: SessionId,
        turn: TurnId,
        workspace: AbsPath,
        subject: HookSubject,
    ) -> Self {
        Self {
            event,
            session,
            turn,
            workspace,
            subject,
        }
    }

    /// The input as the contract gives it to a hook: one JSON object with
    /// `event`, `session_id`, `turn_id` and `workspace`, and the event's own
    /// fields.
    ///
    /// - `prompt.submit`: `prompt` (its text) and, for one sent while a turn
    ///   ran, `delivery`.
    /// - `tool.before`: `tool`, `call_id`, `arguments` (as the model wrote
    ///   them) and `plan` (the plan in one line).
    /// - `tool.after`: the same, `arguments` being those it ran with, and
    ///   `result`: `{outcome, content}`, its outcome and its text.
    /// - `turn.end`: `outcome`, and `error` (the code) when it ended with
    ///   one.
    /// - `compaction.before`: `trigger` (`manual`, `threshold`, `overflow`
    ///   or `background`), and `focus` when the user gave one.
    #[must_use]
    pub fn to_json(&self) -> Json {
        let mut object = serde_json::Map::new();
        object.insert("event".to_owned(), self.event.as_str().into());
        object.insert("session_id".to_owned(), self.session.as_str().into());
        object.insert("turn_id".to_owned(), self.turn.as_str().into());
        object.insert(
            "workspace".to_owned(),
            self.workspace.to_native_string().into(),
        );
        match &self.subject {
            HookSubject::Prompt { parts, delivery } => {
                object.insert("prompt".to_owned(), text_of(parts).into());
                if let Some(delivery) = delivery {
                    object.insert("delivery".to_owned(), delivery.as_str().into());
                }
            }
            HookSubject::Call {
                tool,
                call_id,
                arguments,
                summary,
                effects: _,
            } => {
                object.insert("tool".to_owned(), tool.as_str().into());
                object.insert("call_id".to_owned(), call_id.as_str().into());
                object.insert("arguments".to_owned(), arguments.clone());
                object.insert("plan".to_owned(), summary.as_str().into());
            }
            HookSubject::Ran {
                tool,
                call_id,
                arguments,
                summary,
                effects: _,
                outcome,
                result,
            } => {
                object.insert("tool".to_owned(), tool.as_str().into());
                object.insert("call_id".to_owned(), call_id.as_str().into());
                object.insert("arguments".to_owned(), arguments.clone());
                object.insert("plan".to_owned(), summary.as_str().into());
                object.insert(
                    "result".to_owned(),
                    serde_json::json!({"outcome": outcome.as_str(), "content": result}),
                );
            }
            HookSubject::TurnEnded { outcome, error } => {
                object.insert("outcome".to_owned(), outcome.as_str().into());
                if let Some(error) = error {
                    object.insert("error".to_owned(), error.as_str().into());
                }
            }
            HookSubject::Compaction { trigger, focus } => {
                object.insert("trigger".to_owned(), trigger.as_str().into());
                if let Some(focus) = focus {
                    object.insert("focus".to_owned(), focus.as_str().into());
                }
            }
        }
        Json::Object(object)
    }
}

/// The text parts of a prompt, one after another.
fn text_of(parts: &[Part]) -> String {
    let mut text = String::new();
    for part in parts {
        if let Part::Text(part) = part {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&part.text);
        }
    }
    text
}

/// What one run of a hook came to.
///
/// - A command that writes nothing is `Nothing`; one that writes one JSON
///   value is `Answer`.
/// - A script's function that returns `undefined` is `Nothing`; any other
///   value is `Answer`, `null` included.
/// - The loop reads `Answer(null)` as it reads `Nothing`: no opinion, the
///   one JSON value that says nothing. Any other value that is not an
///   object is outside the contract, and a failure.
#[derive(Debug, Clone, PartialEq)]
pub enum HookOutcome {
    /// It answered nothing: no opinion.
    Nothing,
    /// It answered with this value, which the loop reads by the event's
    /// contract.
    Answer(Json),
    /// It failed: it could not start or was refused, exited with a failure,
    /// wrote what is not one JSON value, or was stopped. Why, in words: the
    /// loop cuts it at [`TEXT_CAP`] and writes what acts unseen in it as
    /// escapes before anyone is shown it.
    Failed(String),
}

/// One hook the implementation matched to an event.
///
/// **No method has a default body**, as for [`Policy`](crate::Policy).
pub trait Hook: Send + Sync {
    /// Its name, as the user and the model are shown it. The loop writes it
    /// in its own text inside backticks, escaped (`cineris_base::in_code`).
    fn name(&self) -> &str;

    /// Whether it only observes (`observe = true`): the loop runs it and
    /// does not wait for it, and reads nothing it answers.
    fn observes(&self) -> bool;

    /// The time it names, if any. The loop waits [`WAIT_DEFAULT`] for a hook
    /// that names none and [`WAIT_MAX`] at most, and lets an observer run
    /// [`OBSERVE_DEFAULT`] and [`OBSERVE_MAX`] at most.
    fn timeout(&self) -> Option<Duration>;

    /// Runs the hook once with `input`, to its end. The loop drops the
    /// future when the hook's time is up, the turn is interrupted, the
    /// response its call came from is thrown away, or the session closes:
    /// **dropped, it stops the hook**, a program it started included. It
    /// never puts a question.
    ///
    /// **The future may be dropped before it is first polled,** and must
    /// stop the hook then too: whatever calling this started (a program, a
    /// call queued on a script's runtime) is stopped or withdrawn when the
    /// future is dropped, polled or not. The loop makes the future only once
    /// it means to poll it (an observer's, once its session is still open),
    /// but a race of its own can still drop one unpolled.
    ///
    /// **The hook's time starts when it is asked,** when this is called:
    /// time it spends waiting for its turn counts. A script's hook that
    /// queues behind a busy runtime (the extension's tool, or another
    /// hook's call, running) is using its own time, and fails when it is up
    /// as any hook does. The implementation keeps no queue of its own that
    /// the loop does not see.
    fn run<'a>(&'a self, input: &'a HookInput) -> BoxFuture<'a, HookOutcome>;
}

/// The hooks of a session, as the loop reaches them.
///
/// **No method has a default body.**
pub trait Hooks: Send + Sync {
    /// Whether any hook is on `event` whose filter matches `target` (`None`
    /// for an event about no tool). Asked first at every point, on the
    /// loop's own path, so it must be cheap: no allocation, no I/O. `false`
    /// passes the point at once.
    ///
    /// **It is `false` only when no hook, in any state, would match:** a
    /// hook that cannot run now (an extension that changed, is unreadable,
    /// failed to register, or is not approved again) is still on its event,
    /// so that a gate it holds does not lapse. It and
    /// [`matching`](Self::matching) read one snapshot of the hooks, so that
    /// a `true` is followed by the hooks it saw.
    fn wants(&self, event: HookEvent, target: Option<ToolTarget<'_>>) -> bool;

    /// The hooks on `event` whose filter matches `target`, in any order:
    /// asked only after [`wants`](Self::wants) said yes, from the same
    /// snapshot. Each is the hook as it is now; a change of the
    /// configuration after this does not reach a run begun from it.
    ///
    /// **A hook that cannot run is returned all the same,** as one whose
    /// [`Hook::run`] gives [`HookOutcome::Failed`] with why and the command
    /// that approves or disables it: where hooks gate, that refuses, and
    /// elsewhere the user is told. It is never left out.
    fn matching(&self, event: HookEvent, target: Option<ToolTarget<'_>>) -> Vec<Arc<dyn Hook>>;

    /// Whether a call whose arguments a hook replaced may be put to the
    /// user: `Err` with the reason refuses it. `call` is the replacement as
    /// the rules decided it (its arguments and its plan, which resolved its
    /// paths), its question not put yet. A replacement whose plan would
    /// write a protected path is refused here, as an extension's write is.
    /// It is called on the blocking pool, so it may read the file system.
    ///
    /// # Errors
    ///
    /// Why the replacement may not be put to the user, for the model.
    fn check_replacement(&self, call: &PlannedCall<'_>) -> Result<(), String>;
}

/// A session's hooks, and the timer their times are kept on.
#[derive(Clone)]
pub struct LoopHooks {
    /// The hooks.
    pub hooks: Arc<dyn Hooks>,
    /// What the loop waits on for each hook's time: the session's timer, so
    /// that a test's fake timer keeps it.
    pub timer: SharedTimer,
    /// What every observer runs under: the session's, once the session is
    /// opened with them, so that none outlives it.
    cancel: CancelToken,
    /// How many runs of each observer, by its name, are going on.
    observing: Arc<Mutex<HashMap<String, usize>>>,
}

impl LoopHooks {
    /// A session's hooks, kept on `timer`.
    #[must_use]
    pub fn new(hooks: Arc<dyn Hooks>, timer: SharedTimer) -> Self {
        Self {
            hooks,
            timer,
            cancel: CancelToken::new(),
            observing: Arc::default(),
        }
    }

    /// The hooks of the session whose token is `cancel`: their observers
    /// run under a child of it.
    pub(crate) fn for_session(mut self, cancel: &CancelToken) -> Self {
        self.cancel = cancel.child();
        self
    }
}

impl std::fmt::Debug for LoopHooks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoopHooks").finish_non_exhaustive()
    }
}

/// What a call's question shows of its hooks: those that asked, and a
/// replacement of its arguments. [`PlannedCall::hooks`] carries it to
/// [`Policy::ask`](crate::Policy::ask).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HookQuestion {
    /// Each hook that answered `ask`, with its reason.
    pub asked: Vec<HookReason>,
    /// The arguments one hook put in place of the model's, with the model's
    /// own as `original`. The call's `arguments` and `plan` are the
    /// replacement's.
    pub replaced: Option<ReplacedArguments>,
}

impl HookQuestion {
    /// The lines an approval's `reasons` holds for these hooks, after the
    /// rules' own: one for each hook that asked, and one that says a hook
    /// changed the call. So a client that reads neither `hooks` nor
    /// `replaced` still shows why it is asked, and that the call is not the
    /// one the model wrote.
    #[must_use]
    pub fn reasons(&self) -> Vec<String> {
        self.asked
            .iter()
            .map(HookReason::asking_line)
            .chain(self.replaced.as_ref().map(ReplacedArguments::reason_line))
            .collect()
    }
}

// --------------------------------------------------------------- answers

/// A decision a hook's answer may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Said {
    Allow,
    Deny,
    Ask,
}

/// One hook's answer, read by its event's contract.
#[derive(Debug, Clone, Default, PartialEq)]
struct Answer {
    decision: Option<Said>,
    reason: Option<String>,
    arguments: Option<Json>,
    context: Option<String>,
}

/// What a hook wrote, as the harness passes it on: cut to [`TEXT_CAP`]
/// bytes at a character's edge, the cut marked, and every character that
/// acts unseen written as an escape, but for its line breaks.
fn capped(mut text: String) -> String {
    if text.len() > TEXT_CAP {
        let mut end = TEXT_CAP;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(" […]");
    }
    match cineris_base::visible_keeping(&text, &['\n']) {
        Cow::Borrowed(_) => text,
        Cow::Owned(escaped) => escaped,
    }
}

/// What [`capped`] passed on, on one line: its line breaks, the only
/// characters that begin a line [`capped`] keeps, each a space.
fn on_one_line(text: String) -> String {
    if text.contains('\n') {
        text.replace('\n', " ")
    } else {
        text
    }
}

/// A hook's name as the harness writes it inside backticks.
fn named(hook: &str) -> Cow<'_, str> {
    cineris_base::in_code(hook)
}

/// A hook's text under the harness's line that names it: every line after
/// `> `, and an empty line as `>`, as section 7 quotes an extension's text,
/// so that no line of it reads as the harness's own.
fn quoted(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A field's name as a failure shows it: at most 64 bytes of it.
fn shown_key(key: &str) -> String {
    let mut end = key.len().min(64);
    while !key.is_char_boundary(end) {
        end -= 1;
    }
    key[..end].escape_debug().to_string()
}

/// Reads one hook's answer by `event`'s contract, strictly: an object
/// whose fields are each one the event takes, of its type.
///
/// **This is the one reader of the answers to the loop's five events**
/// (`prompt.submit`, `tool.before`, `tool.after`, `turn.end`,
/// `compaction.before`): an implementation of [`Hooks`] hands the loop
/// what a hook wrote ([`HookOutcome::Answer`]) and does not read it by the
/// contract itself. `cineris_ext::hook::Answer` reads the answers to the
/// core's own two events (`session.start`, `session.end`), which the loop
/// never sees, and keeps to this table for the fields they share: a field
/// or a decision added here is added there in the same change, and the
/// other way round.
///
/// - `prompt.submit`: `decision` (`allow` or `deny`), `reason` and
///   `context`, both text.
/// - `tool.before`: `decision` (`allow`, `deny` or `ask`), `reason` (text)
///   and `arguments` (any JSON).
/// - `tool.after`, `turn.end` and `compaction.before`: `context` (text)
///   alone: what the hook adds.
///
/// `null` says nothing, as an empty answer does. Text is passed on as
/// [`capped`] makes it.
fn read(event: HookEvent, value: Json) -> Result<Answer, String> {
    let fields = match value {
        Json::Object(fields) => fields,
        Json::Null => return Ok(Answer::default()),
        _ => return Err("its answer is not a JSON object".to_owned()),
    };
    let mut answer = Answer::default();
    for (key, value) in fields {
        let text = |value: Json| match value {
            Json::String(text) => Ok(capped(text)),
            _ => Err(format!("its `{}` is not text", shown_key(&key))),
        };
        match (key.as_str(), event) {
            ("decision", HookEvent::PromptSubmit | HookEvent::ToolBefore) => {
                let Json::String(decision) = value else {
                    return Err("its `decision` is not text".to_owned());
                };
                answer.decision = Some(match (decision.as_str(), event) {
                    ("allow", _) => Said::Allow,
                    ("deny", _) => Said::Deny,
                    ("ask", HookEvent::ToolBefore) => Said::Ask,
                    (other, _) => {
                        return Err(format!(
                            "its `decision` is `{}`, which a `{event}` hook may not answer",
                            shown_key(other)
                        ));
                    }
                });
            }
            ("reason", HookEvent::PromptSubmit | HookEvent::ToolBefore) => {
                // A reason goes into the harness's own sentences and into
                // a question's reason lines: on one line, so that none of
                // it begins a line that reads as the harness's.
                answer.reason = Some(on_one_line(text(value)?));
            }
            ("arguments", HookEvent::ToolBefore) => answer.arguments = Some(value),
            ("context", HookEvent::PromptSubmit) => answer.context = Some(text(value)?),
            (
                "context",
                HookEvent::ToolAfter | HookEvent::TurnEnd | HookEvent::CompactionBefore,
            ) => answer.context = Some(text(value)?),
            (other, _) => {
                return Err(format!(
                    "its answer has `{}`, which a `{event}` hook's answer does not have",
                    shown_key(other)
                ));
            }
        }
    }
    Ok(answer)
}

/// What one hook that was waited for said: its answer, or its failure.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Heard {
    hook: String,
    answer: Result<Answer, String>,
}

impl Heard {
    /// The hook named with its reason, or with its failure.
    fn reason(&self) -> HookReason {
        HookReason {
            hook: self.hook.clone(),
            reason: match &self.answer {
                Ok(answer) => answer.reason.clone(),
                Err(failure) => Some(format!("It failed: {failure}.")),
            },
        }
    }

    /// Whether it refuses: it said `deny`, or failed.
    fn refuses(&self) -> bool {
        match &self.answer {
            Ok(answer) => answer.decision == Some(Said::Deny),
            Err(_) => true,
        }
    }
}

// ------------------------------------------------------------ running them

/// Waits for whichever is ready first: `work`, or the end of `limit` on the
/// timer. `None` when the time ran out first. Work that is ready as the
/// time runs out counts.
async fn within<T>(
    timer: &SharedTimer,
    limit: Duration,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    let mut work = std::pin::pin!(work);
    let mut sleep = timer.sleep(limit);
    std::future::poll_fn(|context| {
        if let Poll::Ready(done) = work.as_mut().poll(context) {
            return Poll::Ready(Some(done));
        }
        if sleep.as_mut().poll(context).is_ready() {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// Waits for every future of `all`, polling each in turn on one task, and
/// returns their outputs in the order given. Dropped, it drops every one
/// not done.
async fn together<T>(all: Vec<BoxFuture<'_, T>>) -> Vec<T> {
    let mut slots: Vec<(Option<BoxFuture<'_, T>>, Option<T>)> =
        all.into_iter().map(|future| (Some(future), None)).collect();
    std::future::poll_fn(|context| {
        let mut pending = false;
        for (future, done) in &mut slots {
            if let Some(running) = future {
                match running.as_mut().poll(context) {
                    Poll::Ready(output) => {
                        *done = Some(output);
                        *future = None;
                    }
                    Poll::Pending => pending = true,
                }
            }
        }
        if pending {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    slots.into_iter().filter_map(|(_, done)| done).collect()
}

/// The time the loop gives `hook`.
fn time_of(hook: &dyn Hook) -> Duration {
    if hook.observes() {
        hook.timeout()
            .map_or(OBSERVE_DEFAULT, |time| time.min(OBSERVE_MAX))
    } else {
        hook.timeout()
            .map_or(WAIT_DEFAULT, |time| time.min(WAIT_MAX))
    }
}

/// Whether a point runs its observers: at the first time it is reached,
/// and not when a call is decided again, since an observer decides
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Observers {
    Run,
    Skip,
}

/// Runs `hooks` with `input`: each observer, unless `observers` says to
/// skip them, on a task of its own and not waited for; the rest together,
/// each within its time, and what each said.
pub(crate) async fn ask_all(
    set: &LoopHooks,
    hooks: Vec<Arc<dyn Hook>>,
    input: HookInput,
    observers: Observers,
) -> Vec<Heard> {
    let (observing, waited): (Vec<_>, Vec<_>) = hooks.into_iter().partition(|hook| hook.observes());
    let input = Arc::new(input);
    if observers == Observers::Run {
        for hook in observing {
            observe(set, hook, Arc::clone(&input));
        }
    }
    let event = input.event;
    let runs: Vec<BoxFuture<'_, Heard>> = waited
        .iter()
        .map(|hook| -> BoxFuture<'_, Heard> {
            let input = &*input;
            let timer = &set.timer;
            Box::pin(async move {
                let limit = time_of(hook.as_ref());
                let answer = match within(timer, limit, hook.run(input)).await {
                    None => Err(format!(
                        "it did not answer within {} s",
                        limit.as_secs_f64()
                    )),
                    Some(HookOutcome::Failed(why)) => Err(capped(why)),
                    Some(HookOutcome::Nothing) => Ok(Answer::default()),
                    Some(HookOutcome::Answer(value)) => read(event, value),
                };
                Heard {
                    hook: hook.name().to_owned(),
                    answer,
                }
            })
        })
        .collect();
    let mut heard = together(runs).await;
    // What they said is combined so that order never matters, and what is
    // shown of it is in the order of their names.
    heard.sort_by(|one, other| one.hook.cmp(&other.hook));
    heard
}

/// One run of an observer counted while it goes on.
struct Observing {
    runs: Arc<Mutex<HashMap<String, usize>>>,
    name: String,
}

impl Drop for Observing {
    fn drop(&mut self) {
        let mut runs = self.runs.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = runs.get_mut(&self.name) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                runs.remove(&self.name);
            }
        }
    }
}

/// Runs an observer on a task of its own, to its time, under the session's
/// token, and logs a failure. A run beyond [`OBSERVE_CONCURRENT`] of the
/// same observer is skipped, and the skip logged.
fn observe(set: &LoopHooks, hook: Arc<dyn Hook>, input: Arc<HookInput>) {
    // A closed session runs nothing more.
    if set.cancel.is_cancelled() {
        return;
    }
    let name = hook.name().to_owned();
    {
        let mut runs = set.observing.lock().unwrap_or_else(PoisonError::into_inner);
        let count = runs.entry(name.clone()).or_insert(0);
        if *count >= OBSERVE_CONCURRENT {
            tracing::warn!(
                hook = name,
                event = input.event.as_str(),
                "an observing hook was not run: {OBSERVE_CONCURRENT} runs of it are going on"
            );
            return;
        }
        *count += 1;
    }
    let counted = Observing {
        runs: Arc::clone(&set.observing),
        name,
    };
    let (timer, cancel) = (set.timer.clone(), set.cancel.clone());
    tokio::spawn(async move {
        let _counted = counted;
        let limit = time_of(hook.as_ref());
        // The run is made inside the block, which the race polls only when
        // the session is still open: a run is never made and dropped
        // unpolled (the review of the hook seam, M3).
        let run = async { within(&timer, limit, hook.run(&input)).await };
        let failure = match cancel.race(run).await {
            // The session closed: stopped with it.
            None => None,
            Some(None) => Some(format!(
                "it did not end within {} s, and was stopped",
                limit.as_secs_f64()
            )),
            Some(Some(HookOutcome::Failed(why))) => Some(capped(why)),
            Some(Some(_)) => None,
        };
        if let Some(failure) = failure {
            tracing::warn!(
                hook = hook.name(),
                event = input.event.as_str(),
                "an observing hook failed: {failure}"
            );
        }
    });
}

// ----------------------------------------------------------- combining them

/// What a call's `tool.before` hooks said together.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Before {
    /// No hook narrowed anything: the rules' decision stands.
    Pass,
    /// One or more asked: the call is put to the user, with their reasons.
    Ask(Vec<HookReason>),
    /// The call may not run: why, for the model.
    Deny(String),
    /// One hook replaced the arguments, and none refused.
    Replace {
        hook: String,
        arguments: Json,
        reason: Option<String>,
        /// The other hooks that asked.
        asked: Vec<HookReason>,
    },
}

/// Why a hook refused, in a clause of a sentence for the model.
fn refusal(heard: &Heard) -> String {
    match &heard.answer {
        Ok(Answer {
            reason: Some(reason),
            ..
        }) => format!("the hook `{}` refused it: {reason}", named(&heard.hook)),
        Ok(_) => format!("the hook `{}` refused it", named(&heard.hook)),
        Err(failure) => format!(
            "the hook `{}` failed, and a hook that fails refuses: {failure}",
            named(&heard.hook)
        ),
    }
}

/// Combines what a call's hooks said: `deny` (a failure included) over a
/// replacement over `ask` over `allow`. Two replacements refuse.
pub(crate) fn before(heard: Vec<Heard>) -> Before {
    let refused: Vec<String> = heard
        .iter()
        .filter(|one| one.refuses())
        .map(refusal)
        .collect();
    if !refused.is_empty() {
        return Before::Deny(refused.join("; "));
    }
    let replacing: Vec<&Heard> = heard
        .iter()
        .filter(|one| matches!(&one.answer, Ok(answer) if answer.arguments.is_some()))
        .collect();
    if replacing.len() > 1 {
        let names: Vec<String> = replacing
            .iter()
            .map(|one| format!("`{}`", named(&one.hook)))
            .collect();
        return Before::Deny(format!(
            "the hooks {} each replaced its arguments, and a call runs with one hook's \
             replacement or with none",
            names.join(" and ")
        ));
    }
    let asked: Vec<HookReason> = heard
        .iter()
        .filter(|one| matches!(&one.answer, Ok(answer) if answer.decision == Some(Said::Ask)))
        .filter(|one| !matches!(&one.answer, Ok(answer) if answer.arguments.is_some()))
        .map(Heard::reason)
        .collect();
    if let Some(one) = replacing.first()
        && let Ok(answer) = &one.answer
        && let Some(arguments) = &answer.arguments
    {
        return Before::Replace {
            hook: one.hook.clone(),
            arguments: arguments.clone(),
            reason: answer.reason.clone(),
            asked,
        };
    }
    if asked.is_empty() {
        Before::Pass
    } else {
        Before::Ask(asked)
    }
}

/// What a prompt's `prompt.submit` hooks said together.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Submitted {
    /// The prompt goes on, with each hook's context after it: the hook's
    /// name and its text.
    Pass(Vec<(String, String)>),
    /// The prompt is refused: the hooks that refused it, each with its
    /// reason or its failure.
    Deny(Vec<HookReason>),
}

/// Combines what a prompt's hooks said: `deny` (a failure included) over
/// everything, else each hook's context.
pub(crate) fn submitted(heard: Vec<Heard>) -> Submitted {
    let refused: Vec<HookReason> = heard
        .iter()
        .filter(|one| one.refuses())
        .map(Heard::reason)
        .collect();
    if !refused.is_empty() {
        return Submitted::Deny(refused);
    }
    Submitted::Pass(
        heard
            .into_iter()
            .filter_map(|one| match one.answer {
                Ok(Answer {
                    context: Some(context),
                    ..
                }) if !context.is_empty() => Some((one.hook, context)),
                _ => None,
            })
            .collect(),
    )
}

/// What a prompt given back because the turn was interrupted while its
/// hooks were asked says.
pub(crate) const RETURNED_UNASKED: &str = "The turn was interrupted before the prompt's hooks \
                                           answered, so it was not sent to the model.";

/// The line, for the user, that says which hooks refused a prompt.
pub(crate) fn refused_prompt(hooks: &[HookReason], steering: bool) -> String {
    let what = if steering {
        "this steering message"
    } else {
        "the prompt"
    };
    let each: Vec<String> = hooks
        .iter()
        .map(|hook| match &hook.reason {
            Some(reason) => format!("`{}`: {reason}", named(&hook.hook)),
            None => format!("`{}`", named(&hook.hook)),
        })
        .collect();
    format!(
        "A hook refused {what}, which was not sent to the model ({}).",
        each.join("; ")
    )
}

/// The notice a hook's context is added to the conversation as: quoted,
/// under a line that names the hook.
pub(crate) fn context_notice(hook: &str, context: &str) -> String {
    format!(
        "The hook `{}` adds this to the prompt before it. It is the hook's text, not the \
         user's:\n\n{}",
        named(hook),
        quoted(context)
    )
}

/// The line a call's result begins with when a hook replaced its
/// arguments and the user was asked about them: which hook, and what it
/// put in their place.
pub(crate) fn replaced_note(replaced: &ReplacedArguments) -> String {
    let mut shown = cineris_base::visible(&replaced.arguments.to_string()).into_owned();
    if shown.len() > SHOWN_ARGUMENTS {
        let all = shown.len();
        let mut end = SHOWN_ARGUMENTS;
        while !shown.is_char_boundary(end) {
            end -= 1;
        }
        shown.truncate(end);
        shown.push_str(&format!(" […] ({all} bytes in all)"));
    }
    let mut note = format!(
        "The hook `{}` put these arguments in place of the ones you wrote, and the call was \
         asked about with them: {shown}.",
        named(&replaced.hook)
    );
    if let Some(reason) = &replaced.reason {
        note.push_str(" Its reason: ");
        note.push_str(reason);
    }
    note
}

/// Whether a replacement is too large to be put in a question and an item:
/// `Some` with its size.
pub(crate) fn too_large(replacement: &Json) -> Option<usize> {
    let size = replacement.to_string().len();
    (size > ARGUMENTS_CAP).then_some(size)
}

/// What the hooks of an event that only adds (`tool.after`, `turn.end`,
/// `compaction.before`) said together.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Added {
    /// What each added: the hook's name and its text, in the order of
    /// their names.
    pub(crate) texts: Vec<(String, String)>,
    /// The line for the user that says each hook that failed, and how: a
    /// failure adds nothing.
    pub(crate) failed: Vec<String>,
}

/// Combines what an event's hooks that only add said: each one's text,
/// and each failure, which adds nothing.
pub(crate) fn added(event: HookEvent, heard: Vec<Heard>) -> Added {
    let mut added = Added::default();
    for one in heard {
        match one.answer {
            Ok(Answer {
                context: Some(text),
                ..
            }) if !text.is_empty() => added.texts.push((one.hook, text)),
            Ok(_) => {}
            Err(failure) => added.failed.push(format!(
                "The hook `{}` failed on `{event}`, so it added nothing: {failure}.",
                named(&one.hook)
            )),
        }
    }
    added
}

/// The note a `tool.after` hook's text is added to a call's result as:
/// quoted, under a line that names the hook.
pub(crate) fn result_note(hook: &str, text: &str) -> String {
    format!(
        "The hook `{}` adds this note to the result above. It is the hook's text, not the \
         tool's:\n\n{}",
        named(hook),
        quoted(text)
    )
}

/// The notice a `turn.end` hook's text is given to the next turn as:
/// quoted, under a line that names the hook.
pub(crate) fn turn_end_notice(hook: &str, text: &str) -> String {
    format!(
        "The hook `{}` adds this after the turn before ended. It is the hook's text, not the \
         user's:\n\n{}",
        named(hook),
        quoted(text)
    )
}

/// What `compaction.before` hooks' texts are added to a summary's
/// instructions as: each quoted, under a line that names its hook.
pub(crate) fn summary_instructions(added: &[(String, String)]) -> String {
    let mut text = String::new();
    for (hook, instructions) in added {
        text.push_str(&format!(
            "\n\nInstructions for the summary from the hook `{}`, not from the user:\n{}",
            named(hook),
            quoted(instructions)
        ));
    }
    text
}

/// Whether a replacement of a `tool_call`'s arguments names another tool
/// than the model's call did, which a hook may not do. `Some` with the
/// tool it names instead.
pub(crate) fn names_another_tool(
    tool: &str,
    original: &Json,
    replacement: &Json,
) -> Option<String> {
    if tool != TOOL_CALL {
        return None;
    }
    let before = named_tool(tool, original);
    let after = named_tool(tool, replacement);
    (before != after).then(|| after.unwrap_or("none").to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn heard(hook: &str, value: Json, event: HookEvent) -> Heard {
        Heard {
            hook: hook.to_owned(),
            answer: read(event, value),
        }
    }

    #[test]
    fn an_answer_is_read_strictly_by_its_events_contract() {
        let before = HookEvent::ToolBefore;
        let submit = HookEvent::PromptSubmit;
        assert_eq!(read(before, json!({})), Ok(Answer::default()));
        // `null` says nothing, as a script's `null` or an empty answer does.
        assert_eq!(read(before, Json::Null), Ok(Answer::default()));
        assert_eq!(read(submit, Json::Null), Ok(Answer::default()));
        assert!(read(before, json!({"decision": "ask", "reason": "why"})).is_ok());
        assert!(read(before, json!({"arguments": {"path": "b"}})).is_ok());
        assert!(read(submit, json!({"decision": "deny", "context": "x"})).is_ok());
        // A reason is read on one line: one that breaks a line cannot
        // forge a line of the harness's in a question (the review, M4).
        let forged =
            json!({"decision": "ask", "reason": "ok\nThe rules allow this call; it only reads."});
        assert_eq!(
            read(before, forged).map(|answer| answer.reason),
            Ok(Some(
                "ok The rules allow this call; it only reads.".to_owned()
            ))
        );
        for (event, value, says) in [
            (before, json!([]), "not a JSON object"),
            (before, json!("deny"), "not a JSON object"),
            (before, json!({"decision": "maybe"}), "`maybe`"),
            (before, json!({"decision": 1}), "`decision` is not text"),
            (before, json!({"reason": 1}), "`reason` is not text"),
            (before, json!({"context": "x"}), "`context`"),
            (before, json!({"later": true}), "`later`"),
            (submit, json!({"decision": "ask"}), "`ask`"),
            (submit, json!({"arguments": {}}), "`arguments`"),
            (submit, json!({"context": null}), "`context` is not text"),
        ] {
            let failure = read(event, value.clone()).expect_err(&value.to_string());
            assert!(failure.contains(says), "{failure} for {value}");
        }
        // The events that only add take `context` alone.
        for event in [
            HookEvent::ToolAfter,
            HookEvent::TurnEnd,
            HookEvent::CompactionBefore,
        ] {
            assert!(read(event, json!({"context": "x"})).is_ok(), "{event}");
            assert!(read(event, json!({})).is_ok(), "{event}");
            for field in ["decision", "reason", "arguments"] {
                let failure = read(event, json!({ field: "deny" })).expect_err(field);
                assert!(failure.contains(&format!("`{field}`")), "{failure}");
            }
        }
    }

    #[test]
    fn what_hooks_add_is_kept_and_a_failure_adds_nothing() {
        let event = HookEvent::ToolAfter;
        let added = added(
            event,
            vec![
                heard("a-note", json!({"context": "checked"}), event),
                heard("b-empty", json!({"context": ""}), event),
                heard("c-silent", json!({}), event),
                heard("d-odd", json!({"decision": "deny"}), event),
            ],
        );
        assert_eq!(added.texts, [("a-note".to_owned(), "checked".to_owned())]);
        assert_eq!(added.failed.len(), 1);
        assert!(added.failed[0].starts_with("The hook `d-odd` failed on `tool.after`"));
    }

    #[test]
    fn a_questions_reasons_name_each_hook_and_the_replacement() {
        let question = HookQuestion {
            asked: vec![HookReason {
                hook: "slow".to_owned(),
                reason: Some("tests take a while".to_owned()),
            }],
            replaced: Some(ReplacedArguments {
                hook: "offline".to_owned(),
                arguments: json!({}),
                original: Some(json!({})),
                reason: None,
            }),
        };
        assert_eq!(
            question.reasons(),
            [
                "The hook `slow` asks: tests take a while",
                "The hook `offline` changed the arguments the model wrote: the call would run \
                 with the hook's.",
            ]
        );
    }

    #[test]
    fn a_long_reason_is_cut_at_a_character() {
        let long = "é".repeat(TEXT_CAP);
        let answer = read(HookEvent::ToolBefore, json!({"reason": long})).unwrap();
        let reason = answer.reason.unwrap();
        assert!(reason.len() <= TEXT_CAP + " […]".len());
        assert!(reason.ends_with(" […]"));
    }

    #[test]
    fn deny_beats_a_replacement_beats_ask_beats_allow_in_any_order() {
        let event = HookEvent::ToolBefore;
        let allow = heard("a-allow", json!({"decision": "allow"}), event);
        let ask = heard("b-ask", json!({"decision": "ask", "reason": "look"}), event);
        let replace = heard("c-replace", json!({"arguments": {"path": "b"}}), event);
        let deny = heard("d-deny", json!({"decision": "deny", "reason": "no"}), event);
        let failed = Heard {
            hook: "e-failed".to_owned(),
            answer: Err("it exited with 2".to_owned()),
        };
        assert_eq!(before(vec![allow.clone()]), Before::Pass);
        assert_eq!(
            before(vec![allow.clone(), ask.clone()]),
            Before::Ask(vec![ask.reason()])
        );
        assert!(matches!(
            before(vec![ask.clone(), replace.clone(), allow.clone()]),
            Before::Replace { hook, asked, .. } if hook == "c-replace" && asked == vec![ask.reason()]
        ));
        for denying in [&deny, &failed] {
            let all = vec![
                allow.clone(),
                ask.clone(),
                replace.clone(),
                (*denying).clone(),
            ];
            assert!(matches!(before(all), Before::Deny(why) if why.contains(&denying.hook)));
        }
        let twice = heard("f-replace", json!({"arguments": {"path": "c"}}), event);
        assert!(matches!(
            before(vec![replace, twice]),
            Before::Deny(why) if why.contains("`c-replace` and `f-replace`")
        ));
    }

    #[test]
    fn a_tool_call_replacement_may_not_name_another_tool() {
        let original = json!({"tool": "tracker/find", "arguments": {}});
        assert_eq!(
            names_another_tool(
                "tool_call",
                &original,
                &json!({"tool": " tracker/find ", "arguments": {"q": 1}})
            ),
            None
        );
        assert_eq!(
            names_another_tool("tool_call", &original, &json!({"tool": "tracker/delete"})),
            Some("tracker/delete".to_owned())
        );
        assert_eq!(
            names_another_tool("tool_call", &original, &json!({"arguments": {}})),
            Some("none".to_owned())
        );
        assert_eq!(names_another_tool("write", &original, &json!({})), None);
    }

    /// The golden transcript's lines of a hook (`cineris-protocol`'s
    /// `tests/golden/v1-session.jsonl`) are what the loop and the server
    /// write, from the functions that write them: the message of a prompt
    /// given back, a hook's context as a notice, the `reasons` lines of a
    /// question hooks put, and the note that begins the result of a call
    /// that ran with a hook's arguments (the review of the hook seam).
    #[test]
    fn the_golden_transcripts_hook_lines_are_what_the_loop_writes() {
        const GOLDEN: &str = include_str!("../../cineris-protocol/tests/golden/v1-session.jsonl");
        let mut seen = 0;
        for line in GOLDEN.lines() {
            let line: Json = serde_json::from_str(line).expect("a line of the transcript is JSON");
            let params = &line["params"];
            if params["kind"] == "prompt_returned" {
                let hooks: Vec<HookReason> =
                    serde_json::from_value(params["hooks"].clone()).expect("its hooks");
                let steering = params["delivery"] == "steer";
                assert_eq!(
                    params["message"],
                    refused_prompt(&hooks, steering),
                    "{line}"
                );
                seen += 1;
            }
            if line["method"] == "approval/request" && params.get("hooks").is_some() {
                let question = HookQuestion {
                    asked: serde_json::from_value(params["hooks"].clone()).expect("its hooks"),
                    replaced: serde_json::from_value(params["replaced"].clone())
                        .expect("its replacement"),
                };
                assert_eq!(params["reasons"], json!(question.reasons()), "{line}");
                seen += 1;
            }
            let item = &params["item"];
            if item["kind"] == "notice" && item["source"] == "hook" {
                let context = "Ticket 42 is about the login page.";
                assert_eq!(
                    item["text"],
                    context_notice("ticket-context", context),
                    "{line}"
                );
                seen += 1;
            }
            if item["kind"] == "tool_result" && item.get("replaced").is_some() {
                let replaced: ReplacedArguments =
                    serde_json::from_value(item["replaced"].clone()).expect("its replacement");
                assert_eq!(
                    item["content"][0]["text"],
                    replaced_note(&replaced),
                    "{line}"
                );
                seen += 1;
            }
        }
        assert_eq!(seen, 4, "the transcript's four lines of a hook");
    }

    /// An observing hook that counts its runs, none of which ends.
    #[derive(Default)]
    struct Counting(std::sync::atomic::AtomicUsize);

    impl Counting {
        fn runs(&self) -> usize {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Hook for Counting {
        fn name(&self) -> &str {
            "watching"
        }

        fn observes(&self) -> bool {
            true
        }

        fn timeout(&self) -> Option<Duration> {
            None
        }

        fn run<'a>(&'a self, _input: &'a HookInput) -> BoxFuture<'a, HookOutcome> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(std::future::pending())
        }
    }

    /// Hooks that match nothing: `observe` is handed its hook.
    struct NoHooks;

    impl Hooks for NoHooks {
        fn wants(&self, _event: HookEvent, _target: Option<ToolTarget<'_>>) -> bool {
            false
        }

        fn matching(
            &self,
            _event: HookEvent,
            _target: Option<ToolTarget<'_>>,
        ) -> Vec<Arc<dyn Hook>> {
            Vec::new()
        }

        fn check_replacement(&self, _call: &PlannedCall<'_>) -> Result<(), String> {
            Ok(())
        }
    }

    /// An observer's run is made only once its session is known to be
    /// open (the review, M3): a session closed before the observer's task
    /// first runs never has `run` called, and one closed before it is
    /// asked for starts no task and counts no run.
    #[test]
    fn an_observer_is_not_run_for_a_session_that_has_closed() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        let clock = cineris_base::FakeClock::starting_at("2026-11-13T10:00:00Z").expect("a time");
        let timer: SharedTimer = Arc::new(cineris_testkit::timer::FakeTimer::manual(&clock));
        let session = CancelToken::new();
        let set = LoopHooks::new(Arc::new(NoHooks), timer).for_session(&session);
        let hook = Arc::new(Counting::default());
        let input = Arc::new(HookInput::new(
            HookEvent::TurnEnd,
            SessionId::new("s_1"),
            TurnId::new("t_1"),
            AbsPath::from_os(&std::env::temp_dir()).expect("an absolute path"),
            HookSubject::TurnEnded {
                outcome: TurnOutcome::Completed,
                error: None,
            },
        ));
        runtime.block_on(async {
            // Handed to its task, which has not run yet when the session
            // closes.
            observe(&set, Arc::clone(&hook) as Arc<dyn Hook>, Arc::clone(&input));
            session.cancel();
            for _ in 0..16 {
                tokio::task::yield_now().await;
            }
            assert_eq!(hook.runs(), 0, "a run was made for a closed session");
            // Asked for after the close: not even counted.
            observe(&set, Arc::clone(&hook) as Arc<dyn Hook>, input);
            let counted = set.observing.lock().map(|runs| runs.len()).unwrap_or(1);
            assert_eq!(counted, 0, "a run was counted for a closed session");
        });
        assert_eq!(hook.runs(), 0);
    }
}
