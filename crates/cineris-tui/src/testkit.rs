//! A test kit for the [`App`]: the application driven as the loop drives
//! it, with a scripted [`Link`] in the core's place (`docs/testing-and-ci.md`,
//! "Testing the TUI").
//!
//! Behind the `test-kit` feature, which only dev-dependencies turn on
//! (`cargo xtask rules`): no build of the product has it. The crate's own
//! unit tests have it with no feature. Another crate's tests ask for it in
//! `[dev-dependencies]`:
//!
//! ```toml
//! cineris-tui = { workspace = true, features = ["test-kit"] }
//! ```
//!
//! | Call | What it does |
//! |---|---|
//! | [`Scripted`] | A link that records what the application sends and answers, and gives what the test queued |
//! | [`SessionBuilder`], [`session`] | A [`SessionState`] with sane defaults; a test states only what it cares about |
//! | [`caps`] | A modern terminal's capabilities, as the tests draw for |
//! | [`app`], [`app_opening`] | An application at 99 columns and 40 rows, and its link |
//! | [`opened`], [`opened_with`] | The first frame drawn and the open answered: the application has its session |
//! | [`drain`] | Everything queued on the link, taken in |
//! | [`event`] | An event of the session, as it arrives |
//! | [`approval`] | A request to run `cargo build`, with three answers |
//! | [`ask`] | A request given, and its dialog's first frame drawn |
//! | [`shown_dialog`] | An approval given and drawn until it is ready: the time a key answers it |
//! | [`FakeSetup`], [`app_first_run`], [`app_first_run_prompted`], [`app_first_run_on`] | The first run's program in a test: providers, what other tools left, servers, and what it was asked to store and write (since step 25); a first run with a prompt on the command line, or on a program of the test's own |
//!
//! Time is the test's: every call that draws or reads input takes the
//! [`Instant`] it happens at, and nothing waits.

#![allow(
    clippy::expect_used,
    reason = "test support fails by panicking, with a message that says what was expected"
)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use cineris_protocol::event::Event;
use cineris_protocol::jsonrpc::RequestId;
use cineris_protocol::lease::Level;
use cineris_protocol::methods::{
    ApprovalChoice, ApprovalDetail, ApprovalRequest, CommandDetail, Op, OpResult, ServerRequest,
    ServerResponse, SessionResult,
};
use cineris_protocol::model::ModelRole;
use cineris_protocol::session::{
    RoleState, SandboxStatus, SessionSettings, SessionState, TrustStanding,
};
use cineris_protocol::{
    CallId, ModelId, ProviderId, SessionEvent, SessionId, Speed, Timestamp, TurnId,
};
use cineris_term::{Capabilities, Overrides, Platform};

use crate::app::{App, BATCH, Inbound, Link, Open, Settings, Start, Ticket, dialog};
use crate::highlight::Highlighter;
use crate::history::MemoryHistory;
use crate::keys::KeyMap;

/// The id of the session [`session`] makes, and of the events [`event`]
/// makes.
pub const SESSION: &str = "s_test";

/// The workspace the application and the session are in.
pub const WORKSPACE: &str = "/work/app";

// ------------------------------------------------------------------ link

/// What the scripted link has seen, and what it will give.
#[derive(Debug, Default)]
pub struct Script {
    /// The operations sent, with the ticket each was given, oldest first.
    pub sent: Vec<(Ticket, Op)>,
    /// The answers to the core's requests, oldest first.
    pub answers: Vec<(RequestId, ServerResponse)>,
    /// What the test queued from the core, given oldest first.
    pub inbound: VecDeque<Inbound>,
    /// The last ticket given.
    last: u64,
}

/// A link that records what is sent and answered, and gives what the test
/// queued. A clone is the same link: the test keeps one, and the
/// application the other.
#[derive(Debug, Clone, Default)]
pub struct Scripted(Arc<Mutex<Script>>);

impl Scripted {
    /// What the link has seen and will give, to read or change.
    pub fn script(&self) -> MutexGuard<'_, Script> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queues something from the core, given after what is queued already.
    pub fn give(&self, inbound: Inbound) {
        self.script().inbound.push_back(inbound);
    }

    /// The operations sent so far, taken.
    pub fn take_sent(&self) -> Vec<(Ticket, Op)> {
        std::mem::take(&mut self.script().sent)
    }

    /// The answers given so far, taken.
    pub fn take_answers(&self) -> Vec<(RequestId, ServerResponse)> {
        std::mem::take(&mut self.script().answers)
    }
}

impl Link for Scripted {
    /// Records the operation, under a ticket one more than the last.
    fn send(&mut self, op: Op) -> Ticket {
        let mut script = self.script();
        script.last += 1;
        let ticket = Ticket(script.last);
        script.sent.push((ticket, op));
        ticket
    }

    fn respond(&mut self, request: RequestId, response: ServerResponse) {
        self.script().answers.push((request, response));
    }

    fn next(&mut self) -> Option<Inbound> {
        self.script().inbound.pop_front()
    }
}

// --------------------------------------------------------------- session

/// A [`SessionState`] with sane defaults, so that a test states only what it
/// cares about: the session [`SESSION`] in [`WORKSPACE`], which is its one
/// write root, supervised, with no sandbox and no trust standing, at
/// sequence 0, no turn running, and Claude Opus at high effort as its main
/// model.
///
/// ```ignore
/// let confined = session().sandbox(SandboxStatus::Enforcing).build();
/// ```
#[derive(Debug, Clone)]
pub struct SessionBuilder(SessionState);

/// A [`SessionBuilder`] with the defaults.
#[must_use]
pub fn session() -> SessionBuilder {
    SessionBuilder::default()
}

impl Default for SessionBuilder {
    fn default() -> Self {
        let mut roles = BTreeMap::new();
        roles.insert(
            ModelRole::Main,
            RoleState {
                provider: ProviderId::from("anthropic"),
                model: ModelId::from("claude-opus-5-5"),
                effort: Some("high".to_owned()),
                effort_in_effect: Some("high".to_owned()),
                speed: Speed::Standard,
                speed_in_effect: Speed::Standard,
                speed_cooldown_until: None,
            },
        );
        Self(SessionState {
            session_id: SessionId::from(SESSION),
            workspace: WORKSPACE.to_owned(),
            title: None,
            created_at: Timestamp::from("2026-11-15T10:00:00Z"),
            parent: None,
            level: Level::Supervised,
            lease_id: None,
            sandbox: SandboxStatus::Off,
            speed: Speed::Standard,
            roles,
            seq: 0,
            running_turn: None,
            queue_length: 0,
            write_roots: vec![WORKSPACE.to_owned()],
            trust: None,
        })
    }
}

impl SessionBuilder {
    /// The session's id.
    #[must_use]
    pub fn id(mut self, id: impl Into<SessionId>) -> Self {
        self.0.session_id = id.into();
        self
    }

    /// The session's workspace, and its one write root.
    #[must_use]
    pub fn workspace(mut self, workspace: &str) -> Self {
        workspace.clone_into(&mut self.0.workspace);
        self.0.write_roots = vec![workspace.to_owned()];
        self
    }

    /// The session's title.
    #[must_use]
    pub fn title(mut self, title: &str) -> Self {
        self.0.title = Some(title.to_owned());
        self
    }

    /// The level of autonomy.
    #[must_use]
    pub fn level(mut self, level: Level) -> Self {
        self.0.level = level;
        self
    }

    /// What the sandbox is doing.
    #[must_use]
    pub fn sandbox(mut self, sandbox: SandboxStatus) -> Self {
        self.0.sandbox = sandbox;
        self
    }

    /// The project's trust standing.
    #[must_use]
    pub fn trust(mut self, trust: TrustStanding) -> Self {
        self.0.trust = Some(trust);
        self
    }

    /// The sequence number of the session's last event.
    #[must_use]
    pub fn seq(mut self, seq: u64) -> Self {
        self.0.seq = seq;
        self
    }

    /// A turn running, by its id.
    #[must_use]
    pub fn running(mut self, turn: &str) -> Self {
        self.0.running_turn = Some(TurnId::from(turn));
        self
    }

    /// The main model, by its id; the provider and effort stay.
    #[must_use]
    pub fn main_model(mut self, model: &str) -> Self {
        if let Some(main) = self.0.roles.get_mut(&ModelRole::Main) {
            main.model = ModelId::from(model);
        }
        self
    }

    /// The state.
    #[must_use]
    pub fn build(self) -> SessionState {
        self.0
    }
}

// ------------------------------------------------------------ application

/// A modern terminal, as the tests draw for: `xterm-256color` with true
/// colour, in a UTF-8 locale (the Unicode glyphs), on Unix.
#[must_use]
pub fn caps() -> Capabilities {
    Capabilities::detect(
        |name| match name {
            "TERM" => Some("xterm-256color".to_owned()),
            "COLORTERM" => Some("truecolor".to_owned()),
            "LANG" => Some("C.UTF-8".to_owned()),
            _ => None,
        },
        Platform::Unix,
        Overrides::default(),
    )
}

/// An application that opens a new session in [`WORKSPACE`] after its
/// first frame, with no prompt, and its link. 99 columns and 40 rows, the
/// default settings and keys, no history and no highlighting.
#[must_use]
pub fn app(now: Instant) -> (App, Scripted) {
    app_opening(
        now,
        None,
        Open::New {
            workspace: WORKSPACE.to_owned(),
            settings: SessionSettings::default(),
            ephemeral: false,
        },
    )
}

/// As [`app`], with `settings` in place of the defaults (since step 25:
/// the full-screen presenter's tests give `presenter`).
#[must_use]
pub fn app_with_settings(now: Instant, settings: Settings) -> (App, Scripted) {
    app_built(
        now,
        None,
        Open::New {
            workspace: WORKSPACE.to_owned(),
            settings: SessionSettings::default(),
            ephemeral: false,
        },
        settings,
    )
}

/// As [`app_with_settings`], code coloured by `highlighter` (since step
/// 25: the tests of code highlighted again).
#[must_use]
pub fn app_with_highlighter(
    now: Instant,
    settings: Settings,
    highlighter: Highlighter,
) -> (App, Scripted) {
    app_built_with(
        now,
        None,
        Open::New {
            workspace: WORKSPACE.to_owned(),
            settings: SessionSettings::default(),
            ephemeral: false,
        },
        settings,
        highlighter,
    )
}

/// As [`app`], opening `open`, with `prompt` given on the command line.
#[must_use]
pub fn app_opening(now: Instant, prompt: Option<&str>, open: Open) -> (App, Scripted) {
    app_built(now, prompt, open, Settings::default())
}

fn app_built(
    now: Instant,
    prompt: Option<&str>,
    open: Open,
    settings: Settings,
) -> (App, Scripted) {
    app_built_with(now, prompt, open, settings, Highlighter::off())
}

// ---------------------------------------------------------------- first run

/// What a [`FakeSetup`] was asked to do.
#[derive(Debug, Default)]
pub struct SetupLog {
    /// Each key stored: the provider and the key's text, which the test
    /// compares with what it typed.
    pub stored: Vec<(String, String)>,
    /// Each file read, by its path.
    pub read: Vec<String>,
    /// Each configuration written.
    pub written: Vec<crate::app::setup::Chosen>,
}

/// The first run's program in a test (since step 25): answers from what
/// it holds, and keeps what it was asked.
#[derive(Debug, Clone, Default)]
pub struct FakeSetup {
    /// What [`providers`](crate::app::Setup::providers) answers.
    pub providers: crate::app::setup::Providers,
    /// What [`discover`](crate::app::Setup::discover) answers.
    pub found: crate::app::setup::Found,
    /// Each file's servers, by its path.
    pub servers: BTreeMap<String, Vec<crate::app::setup::McpServer>>,
    /// Why storing a key fails, when it does.
    pub store_fails: Option<String>,
    /// What it was asked.
    pub log: Arc<Mutex<SetupLog>>,
}

impl FakeSetup {
    /// What it was asked so far.
    ///
    /// # Panics
    ///
    /// Never: a poisoned lock is taken as it is.
    pub fn log(&self) -> MutexGuard<'_, SetupLog> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl crate::app::Setup for FakeSetup {
    fn providers(&self) -> Result<crate::app::setup::Providers, String> {
        Ok(self.providers.clone())
    }

    fn store_key(&self, provider: &str, key: cineris_base::Secret) -> Result<(), String> {
        if let Some(why) = &self.store_fails {
            return Err(why.clone());
        }
        self.log()
            .stored
            .push((provider.to_owned(), key.expose().to_owned()));
        Ok(())
    }

    fn discover(&self) -> crate::app::setup::Found {
        self.found.clone()
    }

    fn read_servers(&self, path: &str) -> Result<Vec<crate::app::setup::McpServer>, String> {
        self.log().read.push(path.to_owned());
        self.servers
            .get(path)
            .cloned()
            .ok_or_else(|| "it is not there".to_owned())
    }

    fn write_config(&self, chosen: &crate::app::setup::Chosen) -> Result<String, String> {
        self.log().written.push(chosen.clone());
        Ok("~/.config/cineris/config.toml".to_owned())
    }
}

/// An application at the start of a first run on `setup` in a window of
/// `columns` by `rows`, opening a new session in [`WORKSPACE`] once the
/// steps before it are done, and its link.
#[must_use]
pub fn app_first_run(
    now: Instant,
    setup: &FakeSetup,
    columns: usize,
    rows: usize,
) -> (App, Scripted) {
    first_run_app(now, Arc::new(setup.clone()), columns, rows, None)
}

/// As [`app_first_run`], the command line having given `prompt`, which
/// is sent once the session opens (since the full review of step 25's
/// third pull request).
#[must_use]
pub fn app_first_run_prompted(
    now: Instant,
    setup: &FakeSetup,
    columns: usize,
    rows: usize,
    prompt: &str,
) -> (App, Scripted) {
    first_run_app(now, Arc::new(setup.clone()), columns, rows, Some(prompt))
}

/// As [`app_first_run`], on a program of the test's own: one that holds
/// an answer until the test lets it go, say (since the full review of
/// step 25's third pull request).
#[must_use]
pub fn app_first_run_on(
    now: Instant,
    setup: Arc<dyn crate::app::Setup>,
    columns: usize,
    rows: usize,
) -> (App, Scripted) {
    first_run_app(now, setup, columns, rows, None)
}

fn first_run_app(
    now: Instant,
    setup: Arc<dyn crate::app::Setup>,
    columns: usize,
    rows: usize,
    prompt: Option<&str>,
) -> (App, Scripted) {
    let link = Scripted::default();
    let start = Start {
        version: "0.1.0".to_owned(),
        folder: "~/src/parser".to_owned(),
        cwd: None,
        workspace: WORKSPACE.to_owned(),
        open: Open::New {
            workspace: WORKSPACE.to_owned(),
            settings: SessionSettings::default(),
            ephemeral: false,
        },
        prompt: prompt.map(str::to_owned),
        settings: Settings::default(),
        keymap: KeyMap::defaults(),
        notices: Vec::new(),
        history: Box::new(MemoryHistory::new()),
        highlighter: Highlighter::off(),
        restyle: None,
        export: None,
        setup: Some(setup),
    };
    let app = App::new(
        Box::new(link.clone()),
        start,
        &caps(),
        columns - 1,
        rows,
        now,
    );
    (app, link)
}

fn app_built_with(
    now: Instant,
    prompt: Option<&str>,
    open: Open,
    settings: Settings,
    highlighter: Highlighter,
) -> (App, Scripted) {
    let link = Scripted::default();
    let start = Start {
        version: "0.1.0".to_owned(),
        folder: "~/src/app".to_owned(),
        cwd: None,
        workspace: WORKSPACE.to_owned(),
        open,
        prompt: prompt.map(str::to_owned),
        settings,
        keymap: KeyMap::defaults(),
        notices: Vec::new(),
        history: Box::new(MemoryHistory::new()),
        highlighter,
        restyle: None,
        export: None,
        setup: None,
    };
    let app = App::new(Box::new(link.clone()), start, &caps(), 99, 40, now);
    (app, link)
}

/// An event of the session [`SESSION`], in turn `t_1`, as it arrives.
#[must_use]
pub fn event(event: Event) -> Inbound {
    Inbound::Event(SessionEvent {
        session_id: SessionId::from(SESSION),
        seq: 0,
        turn_id: Some(TurnId::from("t_1")),
        event,
    })
}

/// Takes in everything queued on the link, at `now`.
pub fn drain(app: &mut App, now: Instant) {
    while app.drain(now, BATCH).more {}
}

/// Draws the first frame at `now` and answers the open with the default
/// [`session`]: the application has its session.
///
/// # Panics
///
/// When the first frame sent no `session/new`: the application was opened
/// with another [`Open`] (use [`opened_with`] and answer it yourself), or
/// was opened already.
pub fn opened(app: &mut App, link: &Scripted, now: Instant) {
    opened_with(app, link, now, session().build());
}

/// As [`opened`], the session being `state`.
///
/// # Panics
///
/// As [`opened`].
pub fn opened_with(app: &mut App, link: &Scripted, now: Instant, state: SessionState) {
    let _ = app.frame(now);
    app.drawn(now);
    let sent = link.take_sent();
    let (ticket, _) = sent
        .iter()
        .find(|(_, op)| matches!(op, Op::SessionNew(_)))
        .expect("the session is opened after the first frame");
    link.give(Inbound::Done(
        *ticket,
        Ok(OpResult::SessionNew(SessionResult { session: state })),
    ));
    drain(app, now);
}

// ---------------------------------------------------------------- dialogs

/// An approval to run `cargo build` in [`WORKSPACE`], in the sandbox, for
/// the call `call`: allow once, allow for the session, or deny.
#[must_use]
pub fn approval(call: &str) -> ServerRequest {
    ServerRequest::Approval(ApprovalRequest {
        session_id: SessionId::from(SESSION),
        turn_id: TurnId::from("t_1"),
        call_id: CallId::from(call),
        tool: "shell".to_owned(),
        summary: "Run cargo build".to_owned(),
        detail: ApprovalDetail::Command(CommandDetail {
            command: "cargo build".to_owned(),
            cwd: WORKSPACE.to_owned(),
            outside_sandbox: false,
        }),
        choices: vec![
            ApprovalChoice::AllowOnce,
            ApprovalChoice::AllowSession,
            ApprovalChoice::Deny,
        ],
        reasons: vec!["`cargo build` is not on the safe list.".to_owned()],
        adds: Vec::new(),
    })
}

/// Gives the application `request` under the id `id`, and draws its
/// dialog's first frame at `now`: its grace starts then, and a key in it
/// goes to the editor.
pub fn ask(app: &mut App, link: &Scripted, id: i64, request: ServerRequest, now: Instant) {
    link.give(Inbound::Request(RequestId::Number(id), request));
    drain(app, now);
    let _ = app.frame(now);
    app.drawn(now);
}

/// Gives the application an [`approval`] for the call `c_1` under the id
/// `id`, and draws it as the loop does: its first frame at `now`, and the
/// frame with its answers when it is ready (at the end of its grace, or a
/// second after a key that answered the dialog before it), and no sooner
/// than twice its grace. Returns the time of that frame: one key then
/// answers the dialog.
pub fn shown_dialog(app: &mut App, link: &Scripted, id: i64, now: Instant) -> Instant {
    ask(app, link, id, approval("c_1"), now);
    let at = app.deadline().map_or(now + dialog::GRACE * 2, |due| {
        due.max(now + dialog::GRACE * 2)
    });
    let _ = app.frame(at);
    at
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use cineris_protocol::methods::{ApprovalResponse, TurnInterruptParams, TurnInterruptResult};
    use cineris_term::ColorDepth;

    use super::*;
    use crate::keys::parse;

    fn press(app: &mut App, binding: &str, at: Instant) {
        app.terminal(
            &cineris_term::Event::Key(parse(binding).expect("a key")),
            at,
        );
    }

    fn allowed_once(link: &Scripted) -> bool {
        link.take_answers().iter().any(|(_, response)| {
            matches!(
                response,
                ServerResponse::Approval(ApprovalResponse {
                    choice: ApprovalChoice::AllowOnce,
                    ..
                })
            )
        })
    }

    #[test]
    fn the_scripted_link_records_each_operation_under_a_new_ticket_and_gives_in_order() {
        let link = Scripted::default();
        let mut sender = link.clone();
        let op = || Op::TurnInterrupt(TurnInterruptParams::new(SessionId::from(SESSION)));
        let first = sender.send(op());
        let second = sender.send(op());
        assert_ne!(first, second, "two operations under one ticket");
        let sent = link.take_sent();
        assert_eq!(
            sent.iter().map(|(ticket, _)| *ticket).collect::<Vec<_>>(),
            [first, second]
        );
        assert!(link.take_sent().is_empty(), "taken twice");

        sender.respond(
            RequestId::Number(4),
            ServerResponse::Approval(ApprovalResponse {
                choice: ApprovalChoice::Deny,
                note: None,
            }),
        );
        assert_eq!(link.take_answers().len(), 1);
        assert!(link.take_answers().is_empty());

        let done = |ticket| {
            Inbound::Done(
                ticket,
                Ok(OpResult::TurnInterrupt(TurnInterruptResult::default())),
            )
        };
        link.give(done(first));
        link.give(done(second));
        assert_eq!(sender.next(), Some(done(first)));
        assert_eq!(sender.next(), Some(done(second)));
        assert_eq!(sender.next(), None);
    }

    #[test]
    fn a_session_has_sane_defaults_and_states_only_what_a_test_says() {
        let plain = session().build();
        assert_eq!(plain.session_id, SessionId::from(SESSION));
        assert_eq!(plain.workspace, WORKSPACE);
        assert_eq!(plain.write_roots, [WORKSPACE]);
        assert_eq!(plain.level, Level::Supervised);
        assert_eq!(plain.sandbox, SandboxStatus::Off);
        assert_eq!(plain.trust, None);
        assert_eq!(plain.running_turn, None);
        assert!(plain.roles.contains_key(&ModelRole::Main));

        let told = session()
            .id("s_other")
            .workspace("/elsewhere")
            .title("a title")
            .level(Level::Autonomous)
            .sandbox(SandboxStatus::Enforcing)
            .trust(TrustStanding::Never)
            .seq(5)
            .running("t_9")
            .main_model("some-open-model")
            .build();
        assert_eq!(told.session_id, SessionId::from("s_other"));
        assert_eq!(told.workspace, "/elsewhere");
        assert_eq!(told.write_roots, ["/elsewhere"], "the write root follows");
        assert_eq!(told.title.as_deref(), Some("a title"));
        assert_eq!(told.level, Level::Autonomous);
        assert_eq!(told.sandbox, SandboxStatus::Enforcing);
        assert_eq!(told.trust, Some(TrustStanding::Never));
        assert_eq!(told.seq, 5);
        assert_eq!(told.running_turn, Some(TurnId::from("t_9")));
        let main = &told.roles[&ModelRole::Main];
        assert_eq!(main.model, ModelId::from("some-open-model"));
        assert_eq!(main.provider, ProviderId::from("anthropic"), "kept");
        // What was not said is the default.
        assert_eq!(session().seq(5).build().sandbox, SandboxStatus::Off);
    }

    #[test]
    fn caps_are_a_modern_terminals() {
        let caps = caps();
        assert_eq!(caps.color, ColorDepth::TrueColor);
        assert!(caps.unicode, "the Unicode glyphs");
        assert!(caps.sync, "a synchronized update");
    }

    #[test]
    fn opened_gives_the_application_its_session() {
        let now = Instant::now();
        let (mut app, link) = app(now);
        assert!(app.session().is_none());
        opened(&mut app, &link, now);
        assert_eq!(
            app.session().map(|state| state.session_id.clone()),
            Some(SessionId::from(SESSION))
        );
        assert!(
            link.script().inbound.is_empty(),
            "the open's result is taken"
        );

        let (mut mine, link) = super::app(now);
        opened_with(&mut mine, &link, now, session().id("s_mine").build());
        assert_eq!(
            mine.session().map(|state| state.session_id.clone()),
            Some(SessionId::from("s_mine"))
        );
    }

    #[test]
    fn an_asked_dialog_starts_its_grace_at_its_first_frame() {
        let now = Instant::now();
        let (mut app, link) = app(now);
        opened(&mut app, &link, now);
        let shown = now + Duration::from_secs(5);
        ask(&mut app, &link, 1, approval("c_1"), shown);
        assert_eq!(app.deadline(), Some(shown + dialog::GRACE));
        // In its grace a key goes to the editor.
        press(&mut app, "y", shown + dialog::GRACE / 2);
        assert!(!allowed_once(&link));
    }

    #[test]
    fn a_shown_dialog_is_answered_by_one_key_at_the_time_it_returns() {
        let now = Instant::now();
        let (mut app, link) = app(now);
        opened(&mut app, &link, now);
        let shown = now + Duration::from_secs(5);
        let at = shown_dialog(&mut app, &link, 1, shown);
        assert!(at >= shown + dialog::GRACE, "ready inside its grace");
        // The key comes straight after: the kit drew the answers' frame,
        // and the test draws none.
        press(&mut app, "y", at);
        assert!(
            allowed_once(&link),
            "a key at the time returned answered nothing"
        );
    }
}
