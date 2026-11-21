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
//! - **A hook that fails refuses.** One that fails, does not answer within
//!   its time, or answers with anything outside its event's contract counts
//!   as `deny`, with the failure named. The answer is enforced, so it is
//!   read strictly: a field the event does not take, a value of the wrong
//!   type, or a decision it does not know is a failure.
//! - **An observer is never waited for,** and its answer is not read. It
//!   runs beside the loop, to its own time; a failure is logged.
//! - **Every wait is on the session's timer,** so tests use the fake one.

use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use cineris_base::{AbsPath, SharedTimer};
use cineris_protocol::item::{Delivery, HookReason, ReplacedArguments};
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
}

impl HookEvent {
    /// The event's name, as a hook's configuration and its input say it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PromptSubmit => "prompt.submit",
            Self::ToolBefore => "tool.before",
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
}

impl HookInput {
    /// The input as the contract gives it to a hook: one JSON object with
    /// `event`, `session_id`, `turn_id` and `workspace`, and the event's own
    /// fields. A prompt's are `prompt` (its text) and, for one sent while a
    /// turn ran, `delivery`; a call's are `tool`, `call_id`, `arguments` and
    /// `plan` (the plan in one line).
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
#[derive(Debug, Clone, PartialEq)]
pub enum HookOutcome {
    /// It answered nothing: no opinion.
    Nothing,
    /// It answered with this value, which the loop reads by the event's
    /// contract.
    Answer(Json),
    /// It failed: it could not start or was refused, exited with a failure,
    /// wrote what is not one JSON value, or was stopped. Why, in words.
    Failed(String),
}

/// One hook the implementation matched to an event.
///
/// **No method has a default body**, as for [`Policy`](crate::Policy).
pub trait Hook: Send + Sync {
    /// Its name, as the user and the model are shown it.
    fn name(&self) -> &str;

    /// Whether it only observes (`observe = true`): the loop runs it and
    /// does not wait for it, and reads nothing it answers.
    fn observes(&self) -> bool;

    /// The time it names, if any. The loop waits [`WAIT_DEFAULT`] for a hook
    /// that names none and [`WAIT_MAX`] at most, and lets an observer run
    /// [`OBSERVE_DEFAULT`] and [`OBSERVE_MAX`] at most.
    fn timeout(&self) -> Option<Duration>;

    /// Runs the hook once with `input`, to its end. The loop drops the
    /// future when the hook's time is up, the turn is interrupted, or the
    /// response its call came from is thrown away: **dropped, it stops the
    /// hook**, a program it started included. It never puts a question.
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
    fn wants(&self, event: HookEvent, target: Option<ToolTarget<'_>>) -> bool;

    /// The hooks on `event` whose filter matches `target`, in any order:
    /// asked only after [`wants`](Self::wants) said yes. Each is the hook as
    /// it is now; a change of the configuration after this does not reach
    /// a run begun from it.
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

/// Cuts `text` to [`TEXT_CAP`] bytes at a character's edge, and marks the
/// cut.
fn capped(mut text: String) -> String {
    if text.len() > TEXT_CAP {
        let mut end = TEXT_CAP;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(" […]");
    }
    text
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
/// whose fields are each one the event takes, of its type. `decision` is
/// `allow`, `deny` or (for `tool.before`) `ask`; `reason` is text;
/// `arguments` (`tool.before` only) is any JSON; `context`
/// (`prompt.submit` only) is text.
fn read(event: HookEvent, value: Json) -> Result<Answer, String> {
    let Json::Object(fields) = value else {
        return Err("its answer is not a JSON object".to_owned());
    };
    let mut answer = Answer::default();
    for (key, value) in fields {
        let text = |value: Json| match value {
            Json::String(text) => Ok(capped(text)),
            _ => Err(format!("its `{}` is not text", shown_key(&key))),
        };
        match (key.as_str(), event) {
            ("decision", _) => {
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
            ("reason", _) => answer.reason = Some(text(value)?),
            ("arguments", HookEvent::ToolBefore) => answer.arguments = Some(value),
            ("context", HookEvent::PromptSubmit) => answer.context = Some(text(value)?),
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
        hook.timeout().map_or(OBSERVE_DEFAULT, |time| time.min(OBSERVE_MAX))
    } else {
        hook.timeout().map_or(WAIT_DEFAULT, |time| time.min(WAIT_MAX))
    }
}

/// Runs `hooks` with `input`: each observer on a task of its own, not
/// waited for; the rest together, each within its time, and what each said.
pub(crate) async fn ask_all(
    set: &LoopHooks,
    hooks: Vec<Arc<dyn Hook>>,
    input: HookInput,
) -> Vec<Heard> {
    let (observers, waited): (Vec<_>, Vec<_>) = hooks.into_iter().partition(|hook| hook.observes());
    let input = Arc::new(input);
    for hook in observers {
        observe(set.timer.clone(), hook, Arc::clone(&input));
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
                    Some(HookOutcome::Failed(why)) => Err(why),
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

/// Runs an observer on a task of its own, to its time, and logs a failure.
fn observe(timer: SharedTimer, hook: Arc<dyn Hook>, input: Arc<HookInput>) {
    tokio::spawn(async move {
        let limit = time_of(hook.as_ref());
        let failure = match within(&timer, limit, hook.run(&input)).await {
            None => Some(format!(
                "it did not end within {} s, and was stopped",
                limit.as_secs_f64()
            )),
            Some(HookOutcome::Failed(why)) => Some(why),
            Some(_) => None,
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
        }) => format!("the hook `{}` refused it: {reason}", heard.hook),
        Ok(_) => format!("the hook `{}` refused it", heard.hook),
        Err(failure) => format!(
            "the hook `{}` failed, and a hook that fails refuses: {failure}",
            heard.hook
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
            .map(|one| format!("`{}`", one.hook))
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
            Some(reason) => format!("`{}`: {reason}", hook.hook),
            None => format!("`{}`", hook.hook),
        })
        .collect();
    format!(
        "A hook refused {what}, which was not sent to the model ({}).",
        each.join("; ")
    )
}

/// The notice a hook's context is added to the conversation as, under a
/// line that names the hook.
pub(crate) fn context_notice(hook: &str, context: &str) -> String {
    format!(
        "The hook `{hook}` adds this to the prompt before it. It is the hook's text, not the \
         user's.\n\n{context}"
    )
}

/// The line a call's result begins with when a hook replaced its
/// arguments: which hook, and what it put in their place.
pub(crate) fn replaced_note(replaced: &ReplacedArguments) -> String {
    let mut shown = replaced.arguments.to_string();
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
        "The hook `{}` put these arguments in place of the ones you wrote, and the user was \
         asked about the call with them: {shown}.",
        replaced.hook
    );
    if let Some(reason) = &replaced.reason {
        note.push_str(" Its reason: ");
        note.push_str(reason);
    }
    note
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
        assert!(read(before, json!({"decision": "ask", "reason": "why"})).is_ok());
        assert!(read(before, json!({"arguments": {"path": "b"}})).is_ok());
        assert!(read(submit, json!({"decision": "deny", "context": "x"})).is_ok());
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
}
