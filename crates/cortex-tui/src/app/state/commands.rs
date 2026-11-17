//! The slash commands, and the pickers they open (`docs/steps/23/answered.md`,
//! "The design, before building", point 4; `docs/architecture/09-tui.md`,
//! "Slash commands").
//!
//! A command sends the core what it asks for and says what came of it in a
//! notice; one that chooses (`/model`, `/effort`, `/mode`, `/sessions`,
//! `/permissions`, `/theme`) opens a picker over the input, and acts on the
//! row accepted. A picker the application opened remembers what each of its
//! rows stands for, so that the row's index is all an answer needs.

use std::time::Instant;

use cortex_protocol::lease::Level;
use std::collections::BTreeMap;

use cortex_protocol::methods::{
    ConfigGetParams, ConfigLayer, ConfigOrigin, ConfigReloadParams, Op, PermissionsRevokeParams,
    SessionCompactParams, SessionListParams, SessionRef, SessionSetParams, SessionUndoParams,
    StoredApproval,
};
use cortex_protocol::model::{ModelRole, ReasoningControlKind};
use cortex_protocol::session::{RoleSettings, SessionSettings, SessionSummary};
use cortex_protocol::{ModelId, ProviderId, SessionId, Speed};
use serde_json::Value;

use super::{App, COMMANDS, Effect, Open, Pending};
use crate::app::ThemeChoice;
use crate::complete::{Menu, Row, Toggle};
use crate::keys::{ACTIONS, name};
use crate::transcript::Block;

/// A picker the application opened, with what each of its rows stands for.
#[derive(Debug, Clone)]
pub(crate) enum Picker {
    /// `/sessions`: the sessions of this workspace, newest first.
    Sessions(Vec<SessionSummary>),
    /// `/model`: each model, by its provider.
    Model(Vec<(ProviderId, ModelId)>),
    /// `/effort`: the main model's levels, and its fast tier.
    Effort { levels: Vec<String>, fast: Speed },
    /// `/mode`: the levels.
    Mode(Vec<Level>),
    /// `/permissions`: the approvals stored for the project.
    Permissions(Vec<StoredApproval>),
    /// `/theme`: the built-in themes.
    Theme(Vec<ThemeChoice>),
}

/// What `config/get` is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    /// `/status`: the limits in force, beside the session's state.
    Status,
    /// `/config`: the effective configuration.
    Config,
}

/// The levels, least first.
const LEVELS: [Level; 5] = [
    Level::ReadOnly,
    Level::Supervised,
    Level::Guided,
    Level::Autonomous,
    Level::Unrestricted,
];

impl App {
    fn session_id(&self) -> Option<SessionId> {
        self.session
            .as_ref()
            .map(|session| session.session_id.clone())
    }

    /// The session's id, or a line that says there is none yet.
    fn needs_session(&mut self, what: &str) -> Option<SessionId> {
        let session = self.session_id();
        if session.is_none() {
            self.say(&format!("There is no session to {what} yet."));
        }
        session
    }

    /// Changes the session's settings.
    fn set(&mut self, settings: SessionSettings) {
        let Some(session) = self.needs_session("change") else {
            return;
        };
        let ticket = self.link.send(Op::SessionSet(SessionSetParams {
            session_id: session.clone(),
            settings,
        }));
        self.pending.insert(ticket, Pending::Set(session));
    }

    fn open_picker(&mut self, name: &str, picker: Picker, menu: Menu) {
        self.picker = Some((name.to_owned(), picker));
        self.input.open_picker(menu);
    }

    /// A slash command, its name and the rest of its line.
    pub(super) fn command(&mut self, name: &str, rest: &str, _at: Instant) {
        match name {
            "quit" => self.exit = Some(super::Exit::Quit),
            "help" => {
                let lines = self.help();
                self.transcript.push(Block::Listing {
                    title: "Commands, then keys:".to_owned(),
                    lines,
                });
            }
            "new" => self.new_session(),
            "compact" => {
                let Some(session) = self.needs_session("compact") else {
                    return;
                };
                let ticket = self.link.send(Op::SessionCompact(SessionCompactParams {
                    session_id: session,
                    focus: (!rest.is_empty()).then(|| rest.to_owned()),
                }));
                self.pending.insert(ticket, Pending::Said);
            }
            "sessions" => {
                let ticket = self.link.send(Op::SessionList(SessionListParams {
                    workspace: Some(self.workspace.clone()),
                    limit: Some(50),
                    cursor: None,
                }));
                self.pending.insert(ticket, Pending::Sessions);
            }
            "undo" => {
                if self.running.is_some() {
                    self.say("A turn is running: stop it with esc, then undo.");
                    return;
                }
                let Some(session) = self.needs_session("undo") else {
                    return;
                };
                let ticket = self.link.send(Op::SessionUndo(SessionUndoParams {
                    session_id: session.clone(),
                    restore_files: true,
                }));
                self.pending.insert(ticket, Pending::Undo(session));
            }
            "model" => self.pick_model(),
            "effort" => self.pick_effort(),
            "mode" => self.pick_mode(),
            "permissions" => {
                let Some(session) = self.needs_session("list the permissions of") else {
                    return;
                };
                let ticket = self.link.send(Op::PermissionsList(SessionRef {
                    session_id: session,
                }));
                self.pending.insert(ticket, Pending::Permissions);
            }
            "status" => self.config_get(Shown::Status),
            "config" => self.config_get(Shown::Config),
            "reload" => {
                // The core's own configuration and every session of this
                // client: the person asked, so a standing error is said
                // again.
                let ticket = self
                    .link
                    .send(Op::ConfigReload(ConfigReloadParams::default()));
                self.pending.insert(ticket, Pending::Reload);
            }
            "profile" => self.profile(rest),
            "verify" => {
                let Some(session) = self.needs_session("verify") else {
                    return;
                };
                // A turn that only verifies, whose events say the rest; or
                // why no check could run (`no_checks`), said as the core
                // says it.
                let ticket = self.link.send(Op::VerifyRun(SessionRef {
                    session_id: session,
                }));
                self.pending.insert(ticket, Pending::Said);
            }
            "context" => self.context(),
            "copy" => {
                let answer = for_clipboard(self.mapper.last_answer());
                if answer.trim().is_empty() {
                    self.say("There is no answer to copy yet.");
                } else {
                    self.effects.push(Effect::Copy(answer));
                    // A terminal does not answer the clipboard sequence:
                    // whether it took the text cannot be told.
                    self.say(
                        "Sent the last answer to the terminal's clipboard (a terminal that does \
                         not take the clipboard sequence ignores it).",
                    );
                }
            }
            "theme" => {
                let choices = vec![ThemeChoice::Auto, ThemeChoice::Dark, ThemeChoice::Light];
                let current = choices
                    .iter()
                    .position(|choice| *choice == self.settings.theme)
                    .unwrap_or(0);
                let rows = choices
                    .iter()
                    .map(|choice| {
                        let (label, detail) = match choice {
                            ThemeChoice::Auto => ("auto", "from the terminal's background"),
                            ThemeChoice::Dark => ("dark", ""),
                            ThemeChoice::Light => ("light", ""),
                        };
                        Row::new(label, detail)
                    })
                    .collect();
                self.open_picker(
                    "theme",
                    Picker::Theme(choices),
                    Menu::picker("theme", rows, current, None),
                );
            }
            "view" => self.open_viewer(rest.trim().parse().ok()),
            "plan" => self.summon_panel("plan"),
            "project" => self.summon_panel("project"),
            "usage" => self.summon_panel("usage"),
            _ => {}
        }
    }

    /// The commands, then every key bound, from the key map.
    fn help(&self) -> Vec<String> {
        let mut lines: Vec<String> = COMMANDS
            .iter()
            .map(|(name, what)| format!("/{name:<12} {what}"))
            .collect();
        let keymap = self.input.keymap();
        lines.extend(ACTIONS.iter().filter_map(|entry| {
            let bound: Vec<String> = keymap.keys_of(entry.action).into_iter().map(name).collect();
            (!bound.is_empty())
                .then(|| format!("{:<13} {}", bound.join(" "), entry.name.replace('_', " ")))
        }));
        lines
    }

    fn config_get(&mut self, shown: Shown) {
        let Some(session) = self.needs_session("show") else {
            return;
        };
        let keys = match shown {
            Shown::Status => vec!["limits".to_owned()],
            Shown::Config => Vec::new(),
        };
        let ticket = self.link.send(Op::ConfigGet(ConfigGetParams {
            session_id: Some(session.clone()),
            keys,
        }));
        self.pending.insert(ticket, Pending::Config(shown, session));
    }

    /// What `config/get` gave, shown: for `/config`, each value with where
    /// it came from.
    pub(super) fn config_shown(
        &mut self,
        shown: Shown,
        config: &serde_json::Map<String, Value>,
        origins: &BTreeMap<String, ConfigOrigin>,
    ) {
        let mut lines = Vec::new();
        flatten("", &Value::Object(config.clone()), &mut lines);
        let (title, lines) = match shown {
            Shown::Config => {
                let mut shown = Vec::new();
                for (key, value) in config {
                    let mut lines = Vec::new();
                    flatten(key, value, &mut lines);
                    let from = origins
                        .get(key)
                        .map_or_else(|| "default".to_owned(), origin);
                    shown.extend(lines.into_iter().map(|line| format!("{line}  ({from})")));
                }
                (
                    "The configuration in effect, and where each value came from:".to_owned(),
                    shown,
                )
            }
            Shown::Status => ("Status:".to_owned(), self.status(&lines)),
        };
        self.transcript.push(Block::Listing { title, lines });
    }

    /// `/profile`: with a name, the session loaded with that profile; with
    /// none, the profiles its files define.
    fn profile(&mut self, rest: &str) {
        let Some(session) = self.needs_session("load a profile for") else {
            return;
        };
        if rest.is_empty() {
            let ticket = self.link.send(Op::ConfigGet(ConfigGetParams {
                session_id: Some(session.clone()),
                keys: vec!["profile".to_owned(), "profiles".to_owned()],
            }));
            self.pending.insert(ticket, Pending::Profiles(session));
            return;
        }
        let ticket = self.link.send(Op::ConfigReload(ConfigReloadParams {
            session_id: Some(session),
            profile: Some(rest.to_owned()),
            changed_only: false,
        }));
        self.pending.insert(ticket, Pending::Reload);
    }

    /// The profiles the core says may be chosen for the session, and the
    /// one in force.
    pub(super) fn profiles_listed(&mut self, names: &[String], in_force: Option<&str>) {
        if names.is_empty() {
            self.say(
                "No profile is defined: a `[profiles.<name>]` table in a configuration file \
                 defines one, and `/profile <name>` loads this session with it.",
            );
            return;
        }
        let lines = names
            .iter()
            .map(|name| {
                if Some(name.as_str()) == in_force {
                    format!("{name}  (in force)")
                } else {
                    name.clone()
                }
            })
            .collect();
        self.transcript.push(Block::Listing {
            title: "The profiles this session may use (/profile <name> loads one):".to_owned(),
            lines,
        });
    }

    /// `/context`: what the last request held, by part.
    fn context(&mut self) {
        let Some(parts) = self.usage.as_ref().and_then(|usage| usage.context.clone()) else {
            self.say(
                "No request has been made in this session yet: /context measures the last one.",
            );
            return;
        };
        let whole = parts.estimate.max(1);
        let share = |tokens: u64| {
            // A share to one decimal place, which f64 holds exactly enough.
            #[expect(
                clippy::cast_precision_loss,
                reason = "a share shown to one decimal place"
            )]
            let share = tokens as f64 * 100.0 / whole as f64;
            share
        };
        let rows = [
            ("system prompt", parts.system),
            ("tools", parts.tools),
            ("summary", parts.summary),
            ("your messages", parts.user),
            ("answers", parts.assistant),
            ("tool results", parts.tool_results),
            ("notices", parts.notices),
        ];
        let mut lines: Vec<String> = rows
            .iter()
            .map(|(name, tokens)| {
                format!(
                    "{name:<14} {:>7}  {:>5.1}%",
                    crate::view::blocks::count(*tokens),
                    share(*tokens)
                )
            })
            .collect();
        let of = |part: u64, whole: u64| {
            if whole == 0 {
                String::new()
            } else {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a share shown to one decimal place"
                )]
                let share = part as f64 * 100.0 / whole as f64;
                format!(" ({share:.1}%)")
            }
        };
        lines.push(format!(
            "{:<14} {:>7}  of the {} a request may hold{}, in a window of {}{}",
            "the request",
            crate::view::blocks::count(parts.estimate),
            crate::view::blocks::count(parts.limit),
            of(parts.estimate, parts.limit),
            crate::view::blocks::count(parts.window),
            of(parts.estimate, parts.window),
        ));
        self.transcript.push(Block::Listing {
            title: "The last request's context, in tokens (estimates):".to_owned(),
            lines,
        });
    }

    /// `/status`: the session's state, and the limits in force.
    fn status(&self, limits: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        let mut line = |label: &str, value: String| {
            out.push(format!("{label:<10} {value}"));
        };
        if let Some(session) = &self.session {
            if let Some(main) = session.roles.get(&ModelRole::Main) {
                line("provider", main.provider.as_str().to_owned());
                line("model", main.model.as_str().to_owned());
                line(
                    "effort",
                    main.effort_in_effect
                        .clone()
                        .unwrap_or_else(|| "none".to_owned()),
                );
                line("speed", main.speed_in_effect.as_str().to_owned());
            }
            line("mode", session.level.as_str().to_owned());
            line("sandbox", session.sandbox.as_str().to_owned());
            if let Some(trust) = &session.trust {
                use cortex_protocol::session::TrustStanding;
                let said = match trust {
                    TrustStanding::Never => {
                        "never: you said never to trust it; `cortex permissions trust` trusts \
                         it, `cortex permissions forget` asks again"
                            .to_owned()
                    }
                    TrustStanding::RulesChanged => {
                        "trusted, its rules changed since: `cortex permissions trust` approves \
                         them"
                            .to_owned()
                    }
                    // `--trust-project`, or a yes the trust store could not
                    // keep (the review of step 23's fixes): nothing is kept.
                    TrustStanding::ForTheRun => {
                        "trusted for now only, nothing kept: by --trust-project, or a yes the \
                         trust store could not keep"
                            .to_owned()
                    }
                    other => other.as_str().replace('_', " "),
                };
                line("trust", said);
            }
            line("session", session.session_id.as_str().to_owned());
            line("workspace", session.workspace.clone());
        }
        let limits: Vec<&String> = limits
            .iter()
            .filter(|line| !line.ends_with("= null"))
            .collect();
        if limits.is_empty() {
            line("limits", "none".to_owned());
        } else {
            for (index, limit) in limits.iter().enumerate() {
                line(if index == 0 { "limits" } else { "" }, (*limit).clone());
            }
        }
        line("mcp", "none".to_owned());
        line("extensions", "none".to_owned());
        out
    }

    fn pick_model(&mut self) {
        if self.models.is_empty() {
            self.say("The models are not known yet: try again in a moment.");
            return;
        }
        let current = self
            .session
            .as_ref()
            .and_then(|session| session.roles.get(&ModelRole::Main))
            .map(|main| main.model.clone());
        let mut current_at = 0;
        let mut rows = Vec::new();
        let mut chosen = Vec::new();
        for (index, model) in self.models.iter().enumerate() {
            if Some(&model.id) == current.as_ref() {
                current_at = index;
            }
            rows.push(Row::new(
                model.id.as_str(),
                format!(
                    "{} · {}k context",
                    model.provider.as_str(),
                    model.context_window / 1000
                ),
            ));
            chosen.push((model.provider.clone(), model.id.clone()));
        }
        self.open_picker(
            "model",
            Picker::Model(chosen),
            Menu::picker("model", rows, current_at, None),
        );
    }

    fn pick_effort(&mut self) {
        let Some(main) = self
            .session
            .as_ref()
            .and_then(|session| session.roles.get(&ModelRole::Main))
            .cloned()
        else {
            self.say("There is no session yet.");
            return;
        };
        let Some(model) = self.models.iter().find(|model| model.id == main.model) else {
            self.say("The model's levels are not known yet: try again in a moment.");
            return;
        };
        if model.reasoning.kind != ReasoningControlKind::Levels || model.reasoning.levels.is_empty()
        {
            self.say(&format!(
                "{} has no levels of effort to choose from.",
                model.id.as_str()
            ));
            return;
        }
        let levels = model.reasoning.levels.clone();
        let current = main
            .effort_in_effect
            .as_ref()
            .and_then(|effort| levels.iter().position(|level| level == effort))
            .unwrap_or(0);
        // The toggle is there only when the model offers a fast speed.
        let fast = model.speeds.first().map(|offer| offer.speed.clone());
        let toggle = fast.as_ref().map(|_| Toggle {
            label: "fast mode".to_owned(),
            on: !main.speed_in_effect.is_standard(),
            note: "costs more".to_owned(),
        });
        let rows = levels.iter().map(|level| Row::new(level, "")).collect();
        self.open_picker(
            "effort",
            Picker::Effort {
                levels,
                fast: fast.unwrap_or(Speed::Standard),
            },
            Menu::picker("effort", rows, current, toggle),
        );
    }

    fn pick_mode(&mut self) {
        let current = self
            .session
            .as_ref()
            .and_then(|session| LEVELS.iter().position(|level| *level == session.level))
            .unwrap_or(1);
        let rows = LEVELS
            .iter()
            .map(|level| {
                let detail = match level {
                    Level::ReadOnly => "reads, and asks before anything else",
                    Level::Supervised => "asks before what changes things",
                    Level::Guided => "asks less, inside the sandbox",
                    Level::Autonomous => "asks only past the sandbox",
                    _ => "asks nothing",
                };
                Row::new(level.as_str(), detail)
            })
            .collect();
        self.open_picker(
            "mode",
            Picker::Mode(LEVELS.to_vec()),
            Menu::picker("mode", rows, current, None),
        );
    }

    /// The sessions of this workspace, in a picker.
    pub(super) fn sessions_listed(&mut self, sessions: Vec<SessionSummary>) {
        if sessions.is_empty() {
            self.say("There is no other session here.");
            return;
        }
        let current = self.session_id();
        let at = sessions
            .iter()
            .position(|summary| Some(&summary.session_id) == current.as_ref())
            .unwrap_or(0);
        let rows = sessions
            .iter()
            .map(|summary| {
                Row::new(
                    summary
                        .title
                        .clone()
                        .unwrap_or_else(|| summary.session_id.as_str().to_owned()),
                    summary.updated_at.as_str(),
                )
            })
            .collect();
        self.open_picker(
            "sessions",
            Picker::Sessions(sessions),
            Menu::picker("sessions", rows, at, None),
        );
    }

    /// The approvals stored for the project, in a picker that removes.
    pub(super) fn permissions_listed(&mut self, approvals: Vec<StoredApproval>) {
        if approvals.is_empty() {
            self.say("No approvals are stored for this project.");
            return;
        }
        let rows = approvals
            .iter()
            .map(|approval| {
                Row::new(
                    format!(
                        "{} {} {}",
                        approval.rule.action.as_str(),
                        approval.rule.effect.as_str(),
                        approval.rule.pattern
                    ),
                    format!("since {} · delete removes it", approval.created_at.as_str()),
                )
            })
            .collect();
        self.open_picker(
            "permissions",
            Picker::Permissions(approvals),
            Menu::picker("permissions", rows, 0, None),
        );
    }

    /// A picker's row was accepted.
    pub(super) fn picked(&mut self, name: &str, index: usize) {
        let Some((opened, picker)) = self.picker.take() else {
            return;
        };
        if opened != name {
            return;
        }
        match picker {
            Picker::Model(models) => {
                let Some((provider, model)) = models.get(index).cloned() else {
                    return;
                };
                let mut settings = SessionSettings::default();
                settings.roles.insert(
                    ModelRole::Main,
                    RoleSettings {
                        provider: Some(provider),
                        model: Some(model),
                        ..RoleSettings::default()
                    },
                );
                self.set(settings);
            }
            Picker::Effort { levels, .. } => {
                let Some(level) = levels.get(index).cloned() else {
                    return;
                };
                let mut settings = SessionSettings::default();
                settings.roles.insert(
                    ModelRole::Main,
                    RoleSettings {
                        effort: Some(level),
                        ..RoleSettings::default()
                    },
                );
                self.set(settings);
            }
            Picker::Mode(levels) => {
                let Some(level) = levels.get(index).cloned() else {
                    return;
                };
                self.set(SessionSettings {
                    level: Some(level),
                    ..SessionSettings::default()
                });
            }
            Picker::Sessions(sessions) => {
                let Some(summary) = sessions.get(index) else {
                    return;
                };
                if Some(&summary.session_id) == self.session_id().as_ref() {
                    return;
                }
                self.switch_to(summary.session_id.clone());
            }
            Picker::Permissions(_) => {}
            Picker::Theme(choices) => {
                let Some(choice) = choices.get(index).copied() else {
                    return;
                };
                self.settings.theme = choice;
                self.theme_picked = Some(choice);
                self.look = crate::app::look(&self.settings, &self.caps);
                for dialog in &mut self.dialogs {
                    dialog.restyled();
                }
            }
        }
    }

    /// A picker's toggle changed: `/effort`'s fast mode.
    pub(super) fn toggled(&mut self, name: &str, on: bool) {
        if let Some((opened, Picker::Effort { fast, .. })) = &self.picker
            && opened == name
        {
            let speed = if on { fast.clone() } else { Speed::Standard };
            self.set(SessionSettings {
                speed: Some(speed),
                ..SessionSettings::default()
            });
        }
    }

    /// A picker's row is to be removed: a stored approval.
    pub(super) fn removed(&mut self, name: &str, index: usize) {
        let Some((opened, Picker::Permissions(approvals))) = &self.picker else {
            return;
        };
        if opened != name {
            return;
        }
        let Some(approval) = approvals.get(index).cloned() else {
            return;
        };
        let Some(session) = self.session_id() else {
            return;
        };
        let ticket = self
            .link
            .send(Op::PermissionsRevoke(PermissionsRevokeParams {
                session_id: session.clone(),
                ids: vec![approval.id],
            }));
        self.pending.insert(ticket, Pending::Said);
        // The picker is opened again with what is left.
        let ticket = self.link.send(Op::PermissionsList(SessionRef {
            session_id: session,
        }));
        self.pending.insert(ticket, Pending::Permissions);
    }

    /// Leaves this session for another of the workspace's: the one open is
    /// closed, and the other opened with its history drawn.
    fn switch_to(&mut self, session: SessionId) {
        // A held prompt on its way holds the session too: what follows it
        // is this session's.
        if self.running.is_some() || self.held_sent.is_some() {
            self.say("A turn is running: wait for it to end, or stop it with esc.");
            return;
        }
        // Until the one under way is open and drawn, what comes back is
        // its: another is switched to after.
        if self.opening || self.reading {
            self.say("A session is being opened: /sessions can switch once it is.");
            return;
        }
        if let Some(open) = self.session.take() {
            let ticket = self.link.send(Op::SessionClose(SessionRef {
                session_id: open.session_id,
            }));
            self.pending.insert(ticket, Pending::Said);
        }
        self.left_session();
        self.had_turn = false;
        self.last_cost = None;
        self.usage = None;
        self.send_open(Open::Resume(session));
    }
}

/// An answer as `/copy` puts it on the clipboard (since the review of step
/// 23): pasted into a terminal, a control character in it would be typed
/// (an escape ending bracketed paste early, and the rest run as keys), so
/// every one but the line feed and the tab is dropped, a carriage return
/// before a line feed with them; and what acts unseen is written as its
/// escape, as the screen shows it (`cortex_term::sanitize`).
pub(crate) fn for_clipboard(answer: &str) -> String {
    let typed: String = answer
        .chars()
        .filter(|&c| matches!(c, '\n' | '\t') || cortex_term::sanitize::visible(c).is_none())
        .collect();
    cortex_term::sanitize(&typed).into_owned()
}

/// Where a value came from, as a person reads it: the layer, then the file
/// and line when there is one.
fn origin(origin: &ConfigOrigin) -> String {
    let layer = match origin.layer {
        ConfigLayer::Default => "default",
        ConfigLayer::Managed => "managed",
        ConfigLayer::User => "user",
        ConfigLayer::Project => "project",
        ConfigLayer::ProjectLocal => "project, local",
        ConfigLayer::Profile => "profile",
        ConfigLayer::Environment => "environment",
        ConfigLayer::CommandLine => "command line or this session",
        _ => "another layer",
    };
    match (&origin.file, origin.line) {
        (Some(file), Some(line)) => format!("{layer}, {file}:{line}"),
        (Some(file), None) => format!("{layer}, {file}"),
        _ => layer.to_owned(),
    }
}

/// `value` as `key = value` lines, its tables flattened into dotted keys.
fn flatten(prefix: &str, value: &Value, lines: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                let key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&key, value, lines);
            }
        }
        other => lines.push(format!("{prefix} = {other}")),
    }
}
