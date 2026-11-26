//! The subcommands: sessions, configuration, permissions and credentials
//! (`docs/architecture/08-configuration.md`, `06-permissions-and-sandbox.md`).
//!
//! Each one starts a core, asks it what it needs, prints and exits. What
//! they print goes to standard output; problems go to standard error. The
//! exit codes are the headless table's: 2 for a mistake or an invalid
//! configuration, 1 for anything else that went wrong.

use std::io::{self, IsTerminal, Read};
use std::sync::Arc;

use cineris_base::Secret;
use cineris_config::{Diagnostic, Env, LayerKind, LoadOptions, Platform};
use cineris_core::{ClientInfo, Core, CoreOptions, OpError, Project, TokioTimer, TrustState};
use cineris_protocol::ErrorKind;
use cineris_protocol::ProjectKey;
use cineris_protocol::methods::{
    Op, OpResult, SessionListParams, UsageRefreshParams, UsageReportParams,
};
use cineris_protocol::usage::{UsageGroup, UsageRange};

use crate::args::{
    AuthCommand, Common, ConfigCommand, PermissionsCommand, ProjectCommand, SessionsCommand,
    StatsCommand, UsageCommand,
};
use crate::exit;

/// Writes to standard output what a subcommand found, its control
/// characters made visible (a project's rules and settings are the
/// repository's text). A reader that has gone ends the output quietly.
macro_rules! out {
    ($($argument:tt)*) => {
        crate::print_out(&crate::visible(&format!($($argument)*)))
    };
}

/// The longest key `auth login` reads.
const KEY_LIMIT: u64 = 64 * 1024;

/// A finished subcommand: its exit code.
type Done = u8;

struct Context {
    env: Env,
    name: &'static str,
}

impl Context {
    fn new(name: &'static str) -> Self {
        Self {
            env: Env::from_process(),
            name,
        }
    }

    fn core(&self, common: &Common) -> Result<Core, Done> {
        let mut options = CoreOptions::new(self.env.clone(), Arc::new(TokioTimer));
        options.profile.clone_from(&common.profile);
        options.overrides = common
            .settings
            .iter()
            .map(|(key, value, flag)| cineris_config::Override {
                key: key.clone(),
                value: value.clone(),
                flag: flag.clone(),
            })
            .collect();
        // The launcher a session is given, which starts no command here: what
        // `permissions trust` says an extension's programs can do is then
        // what a session would let them do (the fresh review of step 28).
        options.launcher = std::env::current_exe()
            .ok()
            .map(|exe| (exe, vec![std::ffi::OsString::from(crate::args::SANDBOX)]));
        Core::start(options).map_err(|error| self.fail(&error))
    }

    /// The project of the workspace root, found from the working directory
    /// by walking up, as a session's would be.
    fn project(&self, core: &Core) -> Result<Project, Done> {
        let here = crate::workspace::Here::now().map_err(|error| {
            self.say(&format!("the working directory could not be read: {error}"));
            exit::USAGE
        })?;
        for said in here.passed_over_said() {
            self.say(&said);
        }
        core.project(&here.root).map_err(|error| self.fail(&error))
    }

    fn say(&self, text: &str) {
        eprintln!("{}: {}", self.name, crate::visible(text));
    }

    /// Says what went wrong and gives the exit code of its kind.
    fn fail(&self, error: &OpError) -> Done {
        self.say(&error.message);
        match error.kind {
            ErrorKind::Usage | ErrorKind::Config => exit::USAGE,
            _ => exit::INTERNAL,
        }
    }

    fn warn(&self, diagnostics: &[Diagnostic]) {
        for diagnostic in diagnostics {
            eprintln!("{}", crate::visible(&diagnostic.render(&self.env)));
        }
    }
}

fn result(outcome: Result<Done, Done>) -> Done {
    match outcome {
        Ok(code) | Err(code) => code,
    }
}

/// `cineris sessions …`
pub(crate) fn sessions(command: SessionsCommand, name: &'static str) -> Done {
    let context = Context::new(name);
    result((|| {
        let SessionsCommand::List { all, limit, json } = command;
        let core = context.core(&Common::default())?;
        let workspace = if all {
            None
        } else {
            Some(context.project(&core)?.root)
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| {
                context.say(&format!("the runtime did not start: {error}"));
                exit::INTERNAL
            })?;
        let listed = runtime.block_on(async {
            let client = core.connect(ClientInfo::observer());
            core.handle(
                client.id,
                Op::SessionList(SessionListParams {
                    workspace: workspace.map(|path| path.to_string_lossy().into_owned()),
                    limit,
                    cursor: None,
                }),
            )
            .await
        });
        let sessions = match listed {
            Ok(OpResult::SessionList(list)) => list.sessions,
            Ok(_) => return Err(exit::INTERNAL),
            Err(error) => return Err(context.fail(&error)),
        };
        let listing = session_lines(&sessions, json);
        if sessions.is_empty() && !json {
            context.say(if all {
                "there are no stored sessions"
            } else {
                "there are no stored sessions in this workspace; --all lists every workspace's"
            });
        }
        if json {
            crate::print_out(&listing);
        } else {
            out!("{listing}");
        }
        Ok(exit::COMPLETED)
    })())
}

/// What `sessions list` prints: a line each, as JSON with `json`.
fn session_lines(sessions: &[cineris_protocol::session::SessionSummary], json: bool) -> String {
    let mut out = String::new();
    for session in sessions {
        if json {
            out.push_str(&crate::json::record(serde_json::to_string(session)));
        } else {
            out.push_str(&format!(
                "{}  {}  {:>3} turns  {}  {}\n",
                session.session_id,
                session.updated_at,
                session.turns,
                session.model.as_ref().map_or("-", |model| model.as_str()),
                session.title.as_deref().unwrap_or(""),
            ));
        }
    }
    out
}

/// `cineris models`
pub(crate) fn models(name: &'static str) -> Done {
    let context = Context::new(name);
    result((|| {
        let core = context.core(&Common::default())?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| {
                context.say(&format!("the runtime did not start: {error}"));
                exit::INTERNAL
            })?;
        let listed = runtime.block_on(async {
            let client = core.connect(ClientInfo::observer());
            core.handle(
                client.id,
                Op::ModelsList(cineris_protocol::methods::Empty {}),
            )
            .await
        });
        let models = match listed {
            Ok(OpResult::ModelsList(list)) => list.models,
            Ok(_) => return Err(exit::INTERNAL),
            Err(error) => return Err(context.fail(&error)),
        };
        let mut out = String::new();
        for model in &models {
            out.push_str(&format!(
                "{}  {}  {} tokens\n",
                model.id, model.provider, model.context_window
            ));
        }
        out!("{out}");
        Ok(exit::COMPLETED)
    })())
}

/// `cineris usage …`
pub(crate) fn usage(command: UsageCommand, name: &'static str) -> Done {
    let context = Context::new(name);
    result((|| {
        let core = context.core(&Common::default())?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .enable_io()
            .build()
            .map_err(|error| {
                context.say(&format!("the runtime did not start: {error}"));
                exit::INTERNAL
            })?;
        let range = match command.range.as_str() {
            "today" => UsageRange::Today,
            "week" => UsageRange::Week,
            "all" => UsageRange::All,
            _ => UsageRange::Month,
        };
        let group_by: Vec<UsageGroup> = command
            .by
            .iter()
            .map(|group| UsageGroup::from_wire(group))
            .collect();
        let (refreshed, report) = runtime.block_on(async {
            // A refresh is the user's own request, at the keyboard.
            let client = core.connect(ClientInfo::user());
            let refreshed = if command.refresh {
                Some(
                    core.handle(client.id, Op::UsageRefresh(UsageRefreshParams::default()))
                        .await,
                )
            } else {
                None
            };
            let report = core
                .handle(
                    client.id,
                    Op::UsageReport(UsageReportParams {
                        range,
                        group_by,
                        session_id: None,
                        project: command.project.clone().map(ProjectKey::new),
                    }),
                )
                .await;
            (refreshed, report)
        });
        let mut failed = false;
        match refreshed {
            Some(Ok(OpResult::UsageRefresh(result))) => {
                if result.providers.is_empty() {
                    context.say(
                        "nothing to refresh: `usage.refresh` in your configuration names no \
                         provider",
                    );
                }
                for outcome in &result.providers {
                    if let Some(error) = &outcome.error {
                        failed = true;
                        context.say(&format!(
                            "the refresh of {} failed: {}",
                            outcome.provider, error.message
                        ));
                    }
                }
            }
            Some(Ok(_)) => return Err(exit::INTERNAL),
            Some(Err(error)) => return Err(context.fail(&error)),
            None => {}
        }
        let report = match report {
            Ok(OpResult::UsageReport(report)) => report,
            Ok(_) => return Err(exit::INTERNAL),
            Err(error) => return Err(context.fail(&error)),
        };
        if command.json {
            crate::print_out(&usage_json(&report));
        } else {
            out!("{}", crate::usage_view::usage(&report, now()));
        }
        // A failed refresh keeps the figures from before, which are shown;
        // the exit code says it failed.
        Ok(if failed {
            exit::INTERNAL
        } else {
            exit::COMPLETED
        })
    })())
}

/// `cineris stats …`
pub(crate) fn stats(command: StatsCommand, name: &'static str) -> Done {
    let context = Context::new(name);
    result((|| {
        let core = context.core(&Common::default())?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|error| {
                context.say(&format!("the runtime did not start: {error}"));
                exit::INTERNAL
            })?;
        match command {
            StatsCommand::Reset => {
                let removed = runtime
                    .block_on(core.stats_reset())
                    .map_err(|error| context.fail(&error))?;
                out!("Removed {removed} statistics files.\n");
            }
            StatsCommand::Show { json } => {
                let summary = runtime
                    .block_on(core.stats_summary())
                    .map_err(|error| context.fail(&error))?;
                if json {
                    crate::print_out(&stats_json(&summary));
                } else {
                    out!("{}", crate::usage_view::stats(&summary));
                }
            }
        }
        Ok(exit::COMPLETED)
    })())
}

/// What `usage --json` prints.
fn usage_json(report: &cineris_protocol::usage::UsageReport) -> String {
    crate::json::record(serde_json::to_string(report))
}

/// What `stats show --json` prints.
fn stats_json(summary: &cineris_core::StatsSummary) -> String {
    let value = serde_json::json!({
        "enabled": summary.enabled,
        "dir": summary.dir,
        "retention_months": summary.retention,
        "months": summary.months,
        "bytes": summary.bytes,
        "requests": summary.requests,
        "turns": summary.turns,
        "tool_calls": summary.tool_calls,
        "dropped": summary.dropped,
        "unreadable": summary.unreadable,
        "aggregates": match &summary.aggregates {
            cineris_core::AggregatesFound::Newer { .. } => "newer",
            cineris_core::AggregatesFound::Unreadable => "unreadable",
            cineris_core::AggregatesFound::Missing => "missing",
            _ => "read",
        },
        "set_aside": summary.set_aside,
    });
    crate::json::record(serde_json::to_string(&value))
}

/// Now, for "as of" in what is printed.
fn now() -> cineris_base::UtcTime {
    cineris_base::UtcTime::from_system(std::time::SystemTime::now())
        .unwrap_or(cineris_base::UtcTime::UNIX_EPOCH)
}

/// `cineris config …`
pub(crate) fn config(command: ConfigCommand, name: &'static str) -> Done {
    let context = Context::new(name);
    result((|| match command {
        ConfigCommand::Defaults => {
            // The defaults alone: no file of the user's or the machine's.
            let mut options = LoadOptions::new(Env::empty(Platform::CURRENT));
            options.dirs.managed_config = None;
            let loaded = cineris_config::load(&options).map_err(|failure| {
                context.say(&failure.render(&context.env));
                exit::INTERNAL
            })?;
            out!("{}", loaded.show_layer(LayerKind::Default, &context.env));
            Ok(exit::COMPLETED)
        }
        ConfigCommand::Show {
            origin,
            layer,
            common,
        } => {
            let layer = match layer
                .as_deref()
                .map(|name| (name, LayerKind::from_name(name)))
            {
                Some((name, None)) => {
                    context.say(&format!(
                        "'{name}' is not a layer: default, managed, user, project, \
                         project-local, profile, environment or command-line"
                    ));
                    return Err(exit::USAGE);
                }
                Some((_, layer)) => layer,
                None => None,
            };
            let core = context.core(&common)?;
            let project = context.project(&core)?;
            let loaded = core
                .configuration(&project, false)
                .map_err(|error| context.fail(&error))?;
            context.warn(&loaded.diagnostics);
            match layer {
                Some(layer) => out!("{}", loaded.show_layer(layer, &context.env)),
                None => out!("{}", loaded.show(&context.env, origin)),
            }
            Ok(exit::COMPLETED)
        }
        ConfigCommand::Check { common } => {
            let core = context.core(&common)?;
            let project = context.project(&core)?;
            let loaded = core
                .configuration(&project, true)
                .map_err(|error| context.fail(&error))?;
            context.warn(&loaded.diagnostics);
            out!("The configuration is valid.\n");
            Ok(exit::COMPLETED)
        }
    })())
}

/// `cineris project …`
pub(crate) fn project(command: ProjectCommand, name: &'static str) -> Done {
    use cineris_protocol::methods::ProjectEntriesResult;
    use cineris_protocol::project::{
        CommandPurpose, EntryAction, EntryChange, EntryKind, NewCommand, NewEntry,
    };

    use crate::project_view::{self, Listing};

    let context = Context::new(name);
    result((|| {
        let core = context.core(&Common::default())?;
        let project = context.project(&core)?;
        let fail = |error: OpError| context.fail(&error);
        let usage = |message: String| {
            context.say(&message);
            exit::USAGE
        };
        let now = now().to_timestamp();
        let now = now.as_str();
        // Whether the project is trusted as it is. A store that cannot say
        // is "not trusted", which is how a session would take it: it only
        // decides whether to say that an accepted command still asks.
        let trusted = || {
            core.project_trust(&project)
                .is_ok_and(cineris_core::TrustState::rules_trusted)
        };
        // What a change made, an entry a line, and then, where a command
        // was accepted or written, whether it still asks for want of trust.
        let changed = |result: &ProjectEntriesResult, what: &str| {
            let mut out = String::new();
            for entry in &result.entries {
                out.push_str(&format!("{what} {}\n", entry.id));
            }
            for problem in &result.problems {
                out.push_str(&format!("problem: {problem}\n"));
            }
            if result
                .entries
                .iter()
                .any(|entry| project_view::runs_once_trusted(entry, now))
                && !trusted()
            {
                out.push_str(&project_view::waits_for_trust(name));
                out.push('\n');
            }
            out
        };
        // The files that could not be read, said before a mistake that may
        // follow from them: an id of a file that was not read is "not
        // there".
        let problems = |listed: &ProjectEntriesResult| {
            for problem in &listed.problems {
                context.say(problem);
            }
        };
        match command {
            ProjectCommand::List { all, kind, json } => {
                let listed = core.project_entries(&project).map_err(fail)?;
                let kind = kind.as_deref().map(EntryKind::from_wire);
                if json {
                    // `project/list`'s result, with the listing's choice of
                    // entries.
                    let chosen = serde_json::json!({
                        "entries": project_view::chosen(&listed.entries, all, kind.as_ref()),
                        "problems": listed.problems,
                    });
                    crate::print_out(&crate::json::record(serde_json::to_string(&chosen)));
                    return Ok(exit::COMPLETED);
                }
                out!(
                    "{}\n{}",
                    project.shown,
                    project_view::listing(
                        &listed.entries,
                        &listed.problems,
                        Listing {
                            all,
                            kind: kind.as_ref(),
                            now,
                            trusted: trusted(),
                            name,
                        }
                    )
                );
                Ok(exit::COMPLETED)
            }
            ProjectCommand::Change {
                action,
                ids,
                shared,
            } => {
                let listed = core.project_entries(&project).map_err(fail)?;
                let ids = if shared {
                    project_view::waiting_shared(&listed.entries)
                } else {
                    let named = ids
                        .iter()
                        .map(|given| project_view::resolve(&listed.entries, given))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|message| {
                            problems(&listed);
                            usage(message)
                        })?;
                    project_view::each_once(named)
                };
                if ids.is_empty() {
                    // Nothing waits; or the file that would say so could
                    // not be read, which is not "nothing".
                    if listed.problems.is_empty() {
                        out!("No entry of the repository's file waits for you.\n");
                        return Ok(exit::COMPLETED);
                    }
                    problems(&listed);
                    return Err(exit::USAGE);
                }
                let changes: Vec<EntryChange> = ids
                    .into_iter()
                    .map(|id| EntryChange {
                        id,
                        action: EntryAction::from_wire(&action),
                        text: None,
                    })
                    .collect();
                let done = core
                    .change_project_entries(&project, &changes, &[])
                    .map_err(fail)?;
                let mut out = match action.as_str() {
                    "accept" => changed(&done, "Accepted"),
                    "disable" => changed(&done, "Turned off"),
                    "promote" => changed(&done, "Now in the repository's file, as"),
                    _ => String::new(),
                };
                if action == "delete" {
                    for change in &changes {
                        out.push_str(&format!("Deleted {}\n", change.id));
                    }
                }
                out!("{out}");
                Ok(exit::COMPLETED)
            }
            ProjectCommand::Edit { id, text } => {
                let listed = core.project_entries(&project).map_err(fail)?;
                let id = project_view::resolve(&listed.entries, &id).map_err(|message| {
                    problems(&listed);
                    usage(message)
                })?;
                let done = core
                    .change_project_entries(
                        &project,
                        &[EntryChange {
                            id,
                            action: EntryAction::Edit,
                            text: Some(text),
                        }],
                        &[],
                    )
                    .map_err(fail)?;
                out!("{}", changed(&done, "Changed"));
                Ok(exit::COMPLETED)
            }
            ProjectCommand::Add {
                kind,
                text,
                purpose,
                command,
                cwd,
                read_only,
            } => {
                // The parser gives a purpose with every command.
                let command = command.zip(purpose).map(|(line, purpose)| NewCommand {
                    purpose: CommandPurpose::from_wire(&purpose),
                    command: line,
                    cwd,
                    read_only,
                });
                let done = core
                    .change_project_entries(
                        &project,
                        &[],
                        &[NewEntry {
                            kind: EntryKind::from_wire(&kind),
                            text,
                            command,
                            check: None,
                        }],
                    )
                    .map_err(fail)?;
                out!("{}", changed(&done, "Added"));
                Ok(exit::COMPLETED)
            }
            ProjectCommand::Path => {
                let files = core.project_files(&project).map_err(fail)?;
                let shown = |path: &std::path::Path| context.env.display_path(path);
                out!(
                    "your store:        {}\nthe repository's:  {}\n",
                    shown(&files.store),
                    shown(&files.shared)
                );
                Ok(exit::COMPLETED)
            }
        }
    })())
}

/// `cineris permissions …`
pub(crate) fn permissions(command: PermissionsCommand, name: &'static str) -> Done {
    let context = Context::new(name);
    result((|| {
        match &command {
            PermissionsCommand::List => {}
            PermissionsCommand::Revoke { .. } => {
                crate::not_under_a_session(name, "permissions revoke")?;
            }
            PermissionsCommand::Trust { .. } => {
                crate::not_under_a_session(name, "permissions trust")?;
            }
            PermissionsCommand::Untrust => {
                crate::not_under_a_session(name, "permissions untrust")?;
            }
            PermissionsCommand::Forget => {
                crate::not_under_a_session(name, "permissions forget")?;
            }
        }
        let core = context.core(&Common::default())?;
        let project = context.project(&core)?;
        let fail = |error: OpError| context.fail(&error);
        match command {
            PermissionsCommand::List => {
                let trust = core.project_trust(&project).map_err(fail)?;
                let approvals = core.stored_approvals(&project).map_err(fail)?;
                let mut out = format!(
                    "{}: {}\n",
                    project.shown,
                    match trust {
                        TrustState::Untrusted => "not trusted",
                        TrustState::Trusted => "trusted",
                        TrustState::RulesChanged =>
                            "trusted, but its permission rules have changed since; approve them \
                             with `permissions trust`",
                        TrustState::Never =>
                            "never trusted: you said never to trust it, so it is not asked about; \
                             `permissions trust` trusts it, `permissions forget` asks again",
                        TrustState::GitNotTrusted =>
                            "trusted, but not its git: it is a repository whose git you were not \
                             shown when you trusted it, or kept out, so git does not read its \
                             configuration; `permissions trust` trusts it",
                    }
                );
                if approvals.is_empty() {
                    out.push_str("No stored approvals.\n");
                } else {
                    out.push_str("Stored approvals:\n");
                    for approval in &approvals {
                        out.push_str(&format!(
                            "  {}  {} {} {}  (since {})\n",
                            approval.id,
                            approval.rule.action.as_str(),
                            approval.rule.effect.as_str(),
                            approval.rule.pattern,
                            approval.created_at,
                        ));
                    }
                }
                out!("{out}");
                Ok(exit::COMPLETED)
            }
            PermissionsCommand::Revoke { ids } => {
                let ids: Vec<_> = ids.iter().map(|id| id.as_str().into()).collect();
                let removed = core.revoke_approvals(&project, &ids).map_err(fail)?;
                if removed == 0 {
                    context.say("no stored approval of this project has those ids");
                    return Err(exit::USAGE);
                }
                out!("Removed {removed} of {}.\n", ids.len());
                Ok(exit::COMPLETED)
            }
            PermissionsCommand::Trust {
                extensions: named_extensions,
                hooks: named_hooks,
            } => {
                // Its extensions are shown as the trust question shows
                // them, each with its hash, and approved only on a yes at a
                // terminal or, with none to ask at, each one named with that
                // hash (since step 28; by hash since the owner's answer to
                // the early review of its third pull request).
                // What trusting it would turn the sandbox off with, said
                // first, in the trust question's words.
                for warning in core.project_sandbox_warnings(&project).map_err(fail)? {
                    out!("{warning}\n");
                }
                let extensions = core.project_extensions(&project).map_err(fail)?;
                // Its hook commands, shown and approved with its
                // extensions, as the trust question lists them (step 28).
                let hooks = core.project_hook_commands(&project).map_err(fail)?;
                let mut approved = (Vec::new(), Vec::new());
                if !extensions.is_empty() || !hooks.is_empty() {
                    let mut listing = String::new();
                    if !extensions.is_empty() {
                        listing.push_str(
                            "Its extensions, which trusting them would let run, as they are \
                             now:\n",
                        );
                    }
                    for extension in &extensions {
                        // The author's words on a line of their own.
                        listing
                            .push_str(&format!("  {}\n", extension.detail.replace('\n', "\n    ")));
                        listing.push_str(&format!("    Its hash is {}.\n", extension.hash()));
                    }
                    if !hooks.is_empty() {
                        listing.push_str(
                            "Its hooks, which trusting them would let run, as they are now:\n",
                        );
                    }
                    for hook in &hooks {
                        listing.push_str(&format!("  {}\n", hook.detail.replace('\n', "\n    ")));
                        listing.push_str(&format!("    Its hash is {}.\n", hook.hash()));
                    }
                    out!("{listing}");
                    let what = match (extensions.is_empty(), hooks.is_empty()) {
                        (false, true) => "extensions",
                        (true, false) => "hooks",
                        _ => "extensions and hooks",
                    };
                    // Each one named must be one shown, at the hash shown,
                    // wherever it is named: otherwise nothing is trusted.
                    let shown_extensions: Vec<(String, String)> = extensions
                        .iter()
                        .map(|one| (one.name.clone(), one.hash().to_string()))
                        .collect();
                    let shown_hooks: Vec<(String, String)> = hooks
                        .iter()
                        .map(|one| (one.name.clone(), one.hash().to_string()))
                        .collect();
                    for (flag, named, shown) in [
                        ("--extension", &named_extensions, &shown_extensions),
                        ("--hook", &named_hooks, &shown_hooks),
                    ] {
                        if let Some(why) = named_not_shown(flag, named, shown) {
                            context.say(&format!("the project was not trusted: {why}"));
                            return Err(exit::USAGE);
                        }
                    }
                    let at_terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
                    if at_terminal {
                        out!("Approve these {what} too? [y/N] ");
                        let mut line = String::new();
                        let yes = io::stdin().read_line(&mut line).is_ok()
                            && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
                        if yes {
                            approved = (extensions, hooks);
                        }
                    } else {
                        let is_named = |named: &[(String, String)], name: &str| {
                            named.iter().any(|(one, _)| one == name)
                        };
                        approved = (
                            extensions
                                .into_iter()
                                .filter(|one| is_named(&named_extensions, &one.name))
                                .collect(),
                            hooks
                                .into_iter()
                                .filter(|one| is_named(&named_hooks, &one.name))
                                .collect(),
                        );
                    }
                    let left = shown_extensions.len() + shown_hooks.len()
                        - approved.0.len()
                        - approved.1.len();
                    if left > 0 {
                        context.say(&format!(
                            "{left} of the project's {what} were left unapproved: `{} \
                             permissions trust` asks again{}",
                            context.name,
                            if at_terminal {
                                ""
                            } else {
                                ", or approves each with `--extension NAME=HASH` or `--hook \
                                 NAME=HASH`, naming it with the hash it is shown with"
                            }
                        ));
                    }
                }
                let trusted = core
                    .trust_project_with(&project, &approved.0, &approved.1)
                    .map_err(fail)?;
                let mut out = format!(
                    "{} {}{}.\n",
                    if trusted.was_trusted && trusted.git {
                        "Approved the permission rules and git of"
                    } else if trusted.was_trusted {
                        "Approved the permission rules of"
                    } else {
                        "Trusted"
                    },
                    project.shown,
                    if trusted.was_refused {
                        ", which you had said never to trust"
                    } else {
                        ""
                    }
                );
                if trusted.git {
                    out.push_str(
                        "Git may read this repository's own configuration, which can name \
                         programs for git to run.\n",
                    );
                }
                if !trusted.rules.is_empty() {
                    out.push_str("Its permission rules, as they are now:\n");
                    for (profile, rule) in &trusted.rules {
                        let place = profile
                            .as_deref()
                            .map(|profile| format!("  (profile {profile})"))
                            .unwrap_or_default();
                        out.push_str(&format!(
                            "  {} {} {}{place}\n",
                            rule.action.as_str(),
                            rule.effect.as_str(),
                            rule.pattern
                        ));
                    }
                }
                if !trusted.extensions.is_empty() {
                    out.push_str(&format!(
                        "Approved its extensions: {}.\n",
                        trusted
                            .extensions
                            .iter()
                            .map(|name| format!("`{name}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !trusted.hooks.is_empty() {
                    out.push_str(&format!(
                        "Approved its hooks: {}.\n",
                        trusted
                            .hooks
                            .iter()
                            .map(|name| format!("`{name}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !trusted.settings.is_empty() {
                    out.push_str("Its settings that now apply:\n");
                    for (profile, setting) in &trusted.settings {
                        let place = profile
                            .as_deref()
                            .map(|profile| format!("  (profile {profile})"))
                            .unwrap_or_default();
                        out.push_str(&format!("  {} = {}{place}\n", setting.key, setting.value));
                    }
                }
                out!("{out}");
                Ok(exit::COMPLETED)
            }
            PermissionsCommand::Forget => {
                if core.forget_project(&project).map_err(fail)? {
                    out!(
                        "Forgot what you said of {}: the next session there asks again.\n",
                        project.shown
                    );
                } else {
                    out!("Nothing was kept about {}.\n", project.shown);
                }
                Ok(exit::COMPLETED)
            }
            PermissionsCommand::Untrust => {
                if core.untrust_project(&project).map_err(fail)? {
                    out!("{} is not trusted any more.\n", project.shown);
                } else {
                    out!("{} was not trusted.\n", project.shown);
                }
                Ok(exit::COMPLETED)
            }
        }
    })())
}

/// `cineris auth …`
pub(crate) fn auth(command: AuthCommand, name: &'static str) -> Done {
    let context = Context::new(name);
    result((|| {
        let core = context.core(&Common::default())?;
        match command {
            AuthCommand::Status => {
                let mut out = String::new();
                for provider in core.credentials() {
                    let line = match &provider.status {
                        Ok(status) => {
                            let mut line = match &status.active {
                                Some(source) => format!("key from {source}"),
                                None => "no key".to_owned(),
                            };
                            if status.stored_but_unused {
                                line.push_str("; a stored key is not used");
                            }
                            if let Some(warning) = &status.store_warning {
                                line.push_str(&format!("; {warning}"));
                            }
                            line
                        }
                        Err(error) => format!("unknown: {error}"),
                    };
                    out.push_str(&format!("{}: {line}\n", provider.provider));
                }
                out!("{out}");
                Ok(exit::COMPLETED)
            }
            AuthCommand::Login { provider } => {
                known_provider(&context, &core, &provider)?;
                let stdin = io::stdin();
                // At a terminal the key is read without echo, and never drawn
                // (since step 25).
                if stdin.is_terminal() && io::stdout().is_terminal() {
                    let key = read_at_terminal(&context, &core, &provider)?;
                    core.store_credential(&provider, &key).map_err(|error| {
                        context.say(&error.message);
                        exit::AUTH
                    })?;
                    out!("Stored the key of {provider}.\n");
                    return Ok(exit::COMPLETED);
                }
                if stdin.is_terminal() {
                    context.say(&format!(
                        "give the key on standard input, so that it is not echoed or kept in \
                         the shell's history: `{} auth login {provider} < key-file`",
                        context.name
                    ));
                    return Err(exit::USAGE);
                }
                let mut key = String::new();
                stdin
                    .lock()
                    .take(KEY_LIMIT + 1)
                    .read_to_string(&mut key)
                    .map_err(|error| {
                        context.say(&format!("standard input could not be read: {error}"));
                        exit::USAGE
                    })?;
                if key.len() as u64 > KEY_LIMIT {
                    context.say("standard input holds more than a key: 64 KiB at most");
                    return Err(exit::USAGE);
                }
                let key = key.trim();
                if key.is_empty() {
                    context.say("standard input held no key");
                    return Err(exit::USAGE);
                }
                core.store_credential(&provider, &Secret::new(key))
                    .map_err(|error| {
                        context.say(&error.message);
                        exit::AUTH
                    })?;
                out!("Stored the key of {provider}.\n");
                Ok(exit::COMPLETED)
            }
            AuthCommand::Logout { provider } => {
                known_provider(&context, &core, &provider)?;
                let removed = core.remove_credential(&provider).map_err(|error| {
                    context.say(&error.message);
                    exit::AUTH
                })?;
                if removed {
                    out!("Removed the stored key of {provider}.\n");
                } else {
                    out!("No key of {provider} was stored.\n");
                }
                Ok(exit::COMPLETED)
            }
        }
    })())
}

/// Why a `NAME=HASH` named for `flag` is refused: a name not shown, or
/// one shown at another hash. `None` when each is one shown.
fn named_not_shown(
    flag: &str,
    named: &[(String, String)],
    shown: &[(String, String)],
) -> Option<String> {
    named.iter().find_map(|(name, hash)| {
        let code = cineris_base::in_code(name);
        match shown.iter().find(|(one, _)| one == name) {
            None => Some(format!("{flag} names `{code}`, which is not one shown")),
            Some((_, shown)) if !hash.trim().eq_ignore_ascii_case(shown) => Some(format!(
                "{flag} names `{code}` with {}, which is not the hash it is shown with ({shown})",
                crate::visible(hash)
            )),
            Some(_) => None,
        }
    })
}

/// Reads a key for `provider` at the terminal, without echo
/// (`cineris_tui::secret::read_key`): `Esc` or `Ctrl+C` stores nothing, and
/// a signal ends the command with its code.
fn read_at_terminal(context: &Context, core: &Core, provider: &str) -> Result<Secret, Done> {
    let (kind, described) = core.credential_store();
    let store = crate::setup::key_store(kind, &described);
    if let cineris_tui::app::setup::KeyStore::Unavailable(why) = &store {
        context.say(&format!("no key can be stored here: {why}"));
        return Err(exit::AUTH);
    }
    let asking = cineris_tui::secret::Asking {
        provider: provider.to_owned(),
        kept: store.kept(),
        ascii_only: store.ascii_only(),
    };
    let read = cineris_tui::secret::read_key(cineris_tui::app::Overrides::default(), &asking);
    if let Some(code) = cineris_tui::app::signalled() {
        cineris_tui::app::signal_ended();
        return Err(code);
    }
    match read {
        Ok(Some(key)) => Ok(key),
        Ok(None) => {
            context.say("no key was stored");
            Err(exit::USAGE)
        }
        Err(error) => {
            context.say(&format!(
                "the terminal could not be used ({error}): give the key on standard input \
                 instead, `{} auth login {provider} < key-file`",
                context.name
            ));
            Err(exit::USAGE)
        }
    }
}

/// Refuses a provider the core does not know: a key stored under a typo
/// would be kept and never used.
fn known_provider(context: &Context, core: &Core, provider: &str) -> Result<(), Done> {
    let known: Vec<String> = core
        .credentials()
        .into_iter()
        .map(|credential| credential.provider)
        .collect();
    if known.iter().any(|name| name == provider) {
        return Ok(());
    }
    context.say(&format!(
        "there is no provider `{provider}`. The providers are: {}",
        known.join(", ")
    ));
    Err(exit::USAGE)
}

#[cfg(test)]
mod tests {
    use cineris_protocol::ids::Timestamp;
    use cineris_protocol::session::SessionSummary;
    use cineris_protocol::usage::{UsageReport, UsageRow, UsageTotals};
    use cineris_protocol::{ModelId, SessionId};

    use super::*;
    use crate::json::tests::parsed;

    /// With no terminal, `permissions trust` approves each extension and
    /// hook named with the hash it is shown with, and nothing named that
    /// is not shown, or is named with another hash: those refuse the trust
    /// whole (the owner's answer to the early review of step 28's third
    /// pull request).
    #[test]
    fn a_projects_extensions_are_named_by_the_hashes_shown() {
        let shown = vec![
            ("probe".to_owned(), "ab12".to_owned()),
            ("lint".to_owned(), "cd34".to_owned()),
        ];
        let named = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
            pairs
                .iter()
                .map(|(name, hash)| ((*name).to_owned(), (*hash).to_owned()))
                .collect()
        };
        assert_eq!(named_not_shown("--extension", &[], &shown), None);
        assert_eq!(
            named_not_shown("--extension", &named(&[("probe", " AB12 ")]), &shown),
            None
        );
        let why = named_not_shown("--extension", &named(&[("probe", "ab13")]), &shown).unwrap();
        assert!(
            why.contains("not the hash it is shown with (ab12)"),
            "{why}"
        );
        let why = named_not_shown("--hook", &named(&[("other", "ab12")]), &shown).unwrap();
        assert!(
            why.contains("--hook names `other`, which is not one shown"),
            "{why}"
        );
    }

    /// Every hostile name, inside a text that is otherwise plain.
    fn hostile_texts() -> Vec<(String, String)> {
        cineris_testkit::hostile::all()
            .into_iter()
            .map(|name| (name.label, format!("x{}y", name.text)))
            .collect()
    }

    /// `sessions list --json` is JSON, safe at a terminal, whatever a
    /// session's title, workspace or model holds (the follow-ups of step
    /// 23: it printed `\u{202e}`, which is not JSON).
    #[test]
    fn sessions_list_json_is_json_for_every_hostile_name() {
        for (label, text) in hostile_texts() {
            let session = SessionSummary {
                session_id: SessionId::new("s_1"),
                workspace: text.clone(),
                title: Some(text.clone()),
                created_at: Timestamp::new("2026-11-16T10:00:00Z"),
                updated_at: Timestamp::new("2026-11-16T10:00:00Z"),
                parent: None,
                model: Some(ModelId::new(text.clone())),
                turns: 2,
            };
            let back = parsed(&session_lines(&[session.clone(), session], true));
            assert_eq!(back.len(), 2, "{label}");
            for line in back {
                assert_eq!(line["title"], text.as_str(), "{label}");
                assert_eq!(line["workspace"], text.as_str(), "{label}");
                assert_eq!(line["model"], text.as_str(), "{label}");
            }
        }
    }

    /// `usage --json` alike, with a project's label, a model and a zone.
    #[test]
    fn usage_json_is_json_for_every_hostile_name() {
        for (label, text) in hostile_texts() {
            let report = UsageReport {
                range: UsageRange::Month,
                from: None,
                to: None,
                group_by: vec![UsageGroup::Model],
                rows: vec![UsageRow {
                    model: Some(ModelId::new(text.clone())),
                    project_label: Some(text.clone()),
                    ..UsageRow::default()
                }],
                totals: UsageTotals::default(),
                limits: Vec::new(),
                sources: vec![text.clone()],
                recording: true,
                time: None,
                tools: Vec::new(),
                zone: Some(text.clone()),
                notes: vec![text.clone()],
            };
            let back = parsed(&usage_json(&report));
            assert_eq!(back[0]["rows"][0]["model"], text.as_str(), "{label}");
            assert_eq!(
                back[0]["rows"][0]["project_label"],
                text.as_str(),
                "{label}"
            );
            assert_eq!(back[0]["zone"], text.as_str(), "{label}");
            assert_eq!(back[0]["notes"][0], text.as_str(), "{label}");
        }
    }

    /// `stats show --json` alike, with the directory and a file set aside.
    #[test]
    fn stats_json_is_json_for_every_hostile_name() {
        for (label, text) in hostile_texts() {
            let summary = cineris_core::StatsSummary {
                enabled: true,
                dir: Some(text.clone()),
                retention: 12,
                months: vec![text.clone()],
                bytes: 1,
                requests: 2,
                turns: 3,
                tool_calls: 4,
                dropped: 0,
                unreadable: 0,
                aggregates: cineris_core::AggregatesFound::Missing,
                set_aside: vec![text.clone()],
            };
            let back = parsed(&stats_json(&summary));
            assert_eq!(back[0]["dir"], text.as_str(), "{label}");
            assert_eq!(back[0]["set_aside"][0], text.as_str(), "{label}");
            assert_eq!(back[0]["aggregates"], "missing", "{label}");
        }
    }
}
