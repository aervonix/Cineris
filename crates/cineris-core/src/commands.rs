//! Custom commands in a session (since step 28): `commands/list`, and a
//! prompt that names a command, expanded before any `prompt.submit` hook
//! sees it (`docs/architecture/12-extensions.md`, "Tier 1: custom
//! commands").
//!
//! - **From files:** the user's `<config dir>/commands/*.md` and the
//!   workspace's `.cineris/commands/*.md` ([`cineris_ext::commands`]),
//!   read each time they are listed or used, on the blocking pool: a file
//!   changed is seen at once, and a session that has none pays nothing
//!   until one is asked for. A project's are read whether or not it is
//!   trusted: they are text for the model, as its instruction files are.
//! - **From scripts:** each command a loaded extension's script registered,
//!   as the registration cache keeps it, named `<extension>:<command>`; its
//!   script runs as `Target::Command` and gives the text to send.
//! - **Which is used:** the user's command of a name over a project's,
//!   since a command's text is sent as the user's prompt.
//! - **A command's settings profile** is judged by
//!   [`profile_refusal`] and never applied in this build: one that would
//!   widen anything is refused with what it widens, and one that would not
//!   is refused too, saying so, since sending it without its profile would
//!   run it wider than its author meant.

use std::sync::Arc;

use cineris_config::{KeyPath, Loaded, TrustClass};
use cineris_ext::commands::{self as files, Owner, Root};
use cineris_protocol::error::code;
use cineris_protocol::item::{CommandSource, PromptCommand};
use cineris_protocol::lease::Level;
use cineris_protocol::methods::{
    CommandArgument, CommandCall, CommandInfo, CommandNotUsed, CommandsListParams,
    CommandsListResult,
};
use cineris_protocol::{ErrorKind, Part};
use toml::Value;

use crate::clients::ClientId;
use crate::core::{Core, Live, OpError, error};

/// A point of a `turn/start` that names a command, as its run is put on
/// the session, before the step's waits (a point a test can stop at:
/// [`cineris_base::hook`]).
pub const A_COMMAND_IS_TAKEN: &str = "core: a command's run is taken";

/// The time an extension's command has, from when it is asked, whatever its
/// manifest gives its tools: it runs while the user waits at the prompt,
/// inside `turn/start`'s step (the maintainers, after the early review of
/// the commands).
#[cfg(feature = "scripts")]
pub(crate) const COMMAND_TIME: std::time::Duration = std::time::Duration::from_secs(30);

/// A point of a `turn/start` that names an extension's command: its token
/// is on the session, and its script is about to be called (a point a test
/// can stop at: [`cineris_base::hook`]).
#[cfg(feature = "scripts")]
pub const A_SCRIPT_COMMAND_RUNS: &str = "core: an extension's command is about to run";

/// What a session's commands are, found now.
struct Found {
    files: files::Found,
    #[cfg(feature = "scripts")]
    scripts: Vec<Arc<crate::ext::tools::LoadedExtension>>,
    /// The configuration in force, whose profiles a command may name.
    loaded: Arc<Loaded>,
    trusted: bool,
    level: Level,
}

/// `commands/list`: every command the session may send, and those found and
/// not used, with why.
pub(crate) async fn list(
    core: &Core,
    client: ClientId,
    params: &CommandsListParams,
) -> Result<CommandsListResult, OpError> {
    let live = core.open_for(client, &params.session_id)?;
    // An extension's commands are known once the session's extensions are
    // found and registered, as a turn's start waits for them; a finding
    // that failed lists the files' alone, and the turn says why.
    let _ = live.extensions.wait().await;
    let found = find(core, &live).await?;
    let mut result = CommandsListResult {
        commands: Vec::new(),
        not_used: Vec::new(),
    };
    for command in &found.files.commands {
        let source = source(command.owner);
        match command
            .profile
            .as_deref()
            .map(|profile| profile_refusal(&found, profile))
        {
            Some(why) => result.not_used.push(CommandNotUsed {
                name: command.name.clone(),
                source,
                extension: None,
                path: Some(command.path.clone()),
                reason: why,
            }),
            None => result.commands.push(CommandInfo {
                name: command.name.clone(),
                source,
                extension: None,
                description: command.description.clone(),
                arguments: command
                    .arguments
                    .iter()
                    .map(|argument| CommandArgument {
                        name: argument.name.clone(),
                        default: argument.default.clone(),
                    })
                    .collect(),
                path: Some(command.path.clone()),
            }),
        }
    }
    #[cfg(feature = "scripts")]
    for extension in &found.scripts {
        let approved = extension.approved();
        for command in &approved.registration.commands {
            result.commands.push(CommandInfo {
                name: format!("{}:{}", extension.name(), command.name),
                source: script_source(approved.source),
                extension: Some(extension.name().to_owned()),
                description: command.description.clone(),
                arguments: command
                    .arguments
                    .iter()
                    .map(|name| CommandArgument {
                        name: name.clone(),
                        default: None,
                    })
                    .collect(),
                path: None,
            });
        }
    }
    result
        .commands
        .sort_by(|one, other| one.name.cmp(&other.name));
    for left in &found.files.left_out {
        result.not_used.push(CommandNotUsed {
            name: left.name.clone(),
            source: source(left.owner),
            extension: None,
            path: Some(left.path.clone()),
            reason: sentence(&left.reason.to_string()),
        });
    }
    Ok(result)
}

/// The prompt `call` names, expanded, and where it came from: what
/// `turn/start` hands the loop in place of a prompt written out.
///
/// # Errors
///
/// `unknown_command` for a command the session does not have;
/// `command_refused` for one found and not used, or whose profile cannot
/// be applied; `command_arguments` when what was typed does not fit it;
/// `command_failed` when an extension's script failed or gave no text.
pub(crate) async fn expand(
    core: &Core,
    live: &Arc<Live>,
    call: CommandCall,
    #[cfg_attr(
        not(feature = "scripts"),
        allow(unused_variables, reason = "a file's command runs nothing")
    )]
    run: &Run<'_>,
) -> Result<(Vec<Part>, PromptCommand), OpError> {
    let found = find(core, live).await?;
    let shown = cineris_base::in_code(&format!("/{}", call.name)).into_owned();
    if let Some(command) = found.files.get(&call.name) {
        if let Some(profile) = &command.profile {
            return Err(error(
                ErrorKind::Permission,
                code::COMMAND_REFUSED,
                format!(
                    "`{shown}` is not sent: {}",
                    lower(&profile_refusal(&found, profile))
                ),
            ));
        }
        let text = command
            .expand(&call.args)
            .map_err(|why| error(ErrorKind::Usage, code::COMMAND_ARGUMENTS, sentence(&why.0)))?;
        return Ok((
            vec![Part::text(text)],
            PromptCommand {
                name: command.name.clone(),
                source: source(command.owner),
                extension: None,
                args: call.args,
            },
        ));
    }
    #[cfg(feature = "scripts")]
    if let Some((extension, command)) = call.name.split_once(':') {
        match found
            .scripts
            .iter()
            .find(|loaded| loaded.name() == extension)
        {
            Some(loaded) => {
                if let Some(definition) = loaded
                    .approved()
                    .registration
                    .commands
                    .iter()
                    .find(|definition| definition.name == command)
                {
                    let text = run_script(live, run, loaded, definition, &call.args).await?;
                    return Ok((
                        vec![Part::text(text)],
                        PromptCommand {
                            name: call.name.clone(),
                            source: script_source(loaded.approved().source),
                            extension: Some(loaded.name().to_owned()),
                            args: call.args,
                        },
                    ));
                }
            }
            // An extension the session found and does not load: why, as the
            // session was told it (held, changed, turned off, not approved),
            // in place of a command it does not know (the review of step
            // 28's commands).
            None => {
                if let Some(why) = live.command_hooks.scripts.why_not_loaded(extension) {
                    return Err(error(
                        ErrorKind::Usage,
                        code::COMMAND_REFUSED,
                        format!("`{shown}` is not sent: {}", first_lower(&why)),
                    ));
                }
            }
        }
    }
    if let Some(left) = found
        .files
        .left_out
        .iter()
        .find(|left| !left.name.is_empty() && left.name == call.name)
    {
        return Err(error(
            ErrorKind::Usage,
            code::COMMAND_REFUSED,
            format!(
                "`{shown}` is not used: `{}` {}.",
                left.path,
                lower(&left.reason.to_string())
            ),
        ));
    }
    Err(error(
        ErrorKind::Usage,
        code::UNKNOWN_COMMAND,
        format!(
            "There is no command `{shown}` in this session: `commands/list` gives the ones there \
             are."
        ),
    ))
}

/// The session's commands as they are now: its files read on the blocking
/// pool, its loaded extensions' as they registered them.
async fn find(core: &Core, live: &Arc<Live>) -> Result<Found, OpError> {
    let (loaded, trusted) = {
        let config = crate::core::lock(&live.config);
        (config.in_force(), config.profile_and_trust().0)
    };
    let state = live.handle.state();
    let workspace = state.workspace.clone();
    let config_dir = loaded.dirs.config.clone();
    let options = crate::assemble::path_options(&core.inner);
    let files = Core::blocking(move || {
        let resolver = cineris_base::PathResolver::new(std::path::Path::new(&workspace), options)
            .map_err(|failure| {
            error(ErrorKind::Usage, "invalid_workspace", failure.to_string())
        })?;
        let mut roots = vec![Root::project()];
        if let Some(config) = &config_dir {
            roots.push(Root::user(config));
        }
        Ok(files::find(&resolver, &roots))
    })
    .await?;
    Ok(Found {
        files,
        #[cfg(feature = "scripts")]
        scripts: live.command_hooks.scripts.extensions(),
        loaded,
        trusted,
        level: state.level,
    })
}

/// Runs an extension's command and gives the text it returned, under a line
/// that says whose it is.
#[cfg(feature = "scripts")]
async fn run_script(
    live: &Arc<Live>,
    run: &Run<'_>,
    extension: &Arc<crate::ext::tools::LoadedExtension>,
    definition: &cineris_ext::registration::CommandDef,
    typed: &str,
) -> Result<String, OpError> {
    use cineris_tools::{CallRef, FileTable, RunCx};
    let names: Vec<&str> = definition.arguments.iter().map(String::as_str).collect();
    let assigned = files::assign(&names, typed)
        .map_err(|why| error(ErrorKind::Usage, code::COMMAND_ARGUMENTS, sentence(&why.0)))?;
    let arguments: serde_json::Map<String, serde_json::Value> = assigned
        .values
        .iter()
        .map(|(name, value)| (name.clone(), serde_json::Value::String(value.clone())))
        .collect();
    let input = serde_json::json!({"args": typed.trim(), "arguments": arguments});
    let hooks = &live.command_hooks;
    let policy = Arc::clone(hooks.commands.policy());
    let session = live.handle.id().clone();
    let shown = format!("/{}:{}", extension.name(), definition.name);
    // Outside any round: the turn its text will start is not there yet, so
    // a question it puts names this run of the command in the turn's place,
    // by ids of its own ([`run_ids`]).
    let (turn, call_id, item) = run_ids();
    // Nothing runs where its prompt would be refused: a stop left for the
    // next prompt (taken, as the prompt would have taken it), a gate held on
    // `prompt.submit`, or the running turn stopped.
    if let Some(stop) = hooks.commands.take_stop() {
        return Err(error(ErrorKind::Permission, code::COMMAND_REFUSED, stop));
    }
    if let Some(why) = hooks.prompt_gate_held() {
        return Err(error(
            ErrorKind::Permission,
            code::COMMAND_REFUSED,
            format!(
                "`{}` was not run: {why}, which would refuse what it sends.",
                cineris_base::in_code(&shown)
            ),
        ));
    }
    let running = hooks.commands.running_turn();
    let running_number = running.as_ref().map_or(0, crate::ext::turn_number);
    if running_number != 0
        && let Some(stop) = policy.stopped_for_hooks(running_number)
    {
        return Err(error(
            ErrorKind::Permission,
            code::COMMAND_REFUSED,
            format!(
                "`{}` was not run: the running turn was stopped. {stop}",
                cineris_base::in_code(&shown)
            ),
        ));
    }
    cineris_base::hook::await_point!(A_SCRIPT_COMMAND_RUNS);
    let cx = RunCx::new(
        Arc::clone(&hooks.paths),
        Arc::clone(hooks.commands.content()),
        Arc::new(FileTable::new()),
    )
    .with_cancel(run.cancel.clone())
    .with_kill(run.kill.clone())
    // The turn running as it runs, if one is: a stop of its programs stops
    // that turn, and never turn 0; and that turn's stop, made while the
    // script runs, refuses its host calls from then on, as a hook's
    // (`CallHost::decide`).
    .with_turn(running_number)
    .with_gate(cineris_agent::Policy::read_gate(policy.as_ref()))
    .with_call(CallRef {
        session,
        turn,
        call_id,
        item,
    });
    let failed = |why: String| error(ErrorKind::Tool, code::COMMAND_FAILED, sentence(&why));
    let ran = extension
        .call_command(&definition.name, input, &cx, COMMAND_TIME)
        .await;
    let value = match ran {
        Ok(value) => value,
        // A stop of its own (a program's look at the protected paths, a
        // question answered with a stop): told, and the next prompt refused
        // once, as a hook's stop is; the running turn stops with it. Before
        // the interrupt: a stop that comes with one is a stop all the same,
        // or nothing would say it, and the next prompt would go.
        Err(crate::ext::tools::CommandFailed::Stopped(note)) => {
            hooks.commands.command_stopped(&shown, &note);
            return Err(failed(format!("`{shown}` was stopped: {note}")));
        }
        Err(_) if run.cancel.is_cancelled() => {
            return Err(interrupted(&format!(
                "{}:{}",
                extension.name(),
                definition.name
            )));
        }
        Err(crate::ext::tools::CommandFailed::Other(why)) => return Err(failed(why)),
    };
    let text = match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => text,
        serde_json::Value::String(_) | serde_json::Value::Null => {
            return Err(failed(format!(
                "`{shown}` ran and gave no text to send, so no turn started"
            )));
        }
        _ => {
            return Err(failed(format!(
                "`{shown}` gave something other than text: a command's script returns the text \
                 to send, as a string"
            )));
        }
    };
    if text.len() > files::MAX_EXPANDED_BYTES {
        return Err(failed(format!(
            "`{shown}` gave more than {} KiB to send",
            files::MAX_EXPANDED_BYTES / 1024
        )));
    }
    Ok(format!(
        "The command `{}` of the extension `{}`, written by its author, gave this prompt:\n\n{text}",
        cineris_base::in_code(&shown),
        cineris_base::in_code(extension.name())
    ))
}

/// The ids a run of an extension's command is known by, in the place of a
/// turn's and a call's: its question names it, and what a person allows is
/// kept by them (`EnginePolicy`'s allowed questions, by turn and item).
/// Each run's are its own, from a count of the process's, so that no run
/// reuses another's answer; and none begins `t_`, so that none is ever read
/// as a turn's number: a stop is never recorded for it, nor a question of
/// it answered as a real turn's.
#[cfg(feature = "scripts")]
fn run_ids() -> (
    cineris_protocol::TurnId,
    cineris_protocol::CallId,
    cineris_protocol::ItemId,
) {
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    (
        cineris_protocol::TurnId::new(format!("cmd_{run}")),
        cineris_protocol::CallId::new(format!("cmd_{run}")),
        cineris_protocol::ItemId::new(format!("cmd_{run}")),
    )
}

/// The number of each run of a command, in the process.
static RUN_IDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A command's `turn/start` that was sent and that its step has not taken
/// yet: who sent it, what it calls, and its run's number among the
/// session's running commands, whose tokens it has from when it was sent.
pub(crate) struct Sent {
    client: ClientId,
    call: CommandCall,
    id: u64,
}

/// A command's run on the session, from when it was sent or its step began
/// to its step's end: its number, its tokens, and whether its `turn/start`
/// came through a lane ([`sent`]) or to the core without one.
pub(crate) struct Running {
    id: u64,
    cancel: cineris_base::CancelToken,
    kill: cineris_base::CancelToken,
    lane: bool,
}

/// A command's `turn/start` from `client`, as it is sent, before it waits
/// for its turn in the client's lane (`Core::early`): its run's tokens are
/// put on the session now, so that an interrupt sent behind it, while it
/// waits there, stops it before its step begins (the review of step 28's
/// commands). Its step takes them ([`Run::taken`]). `cancel` is the run's
/// own token, below the session's ([`Run`]).
pub(crate) fn sent(
    live: &Live,
    client: ClientId,
    call: &CommandCall,
    cancel: cineris_base::CancelToken,
) {
    let id = RUN_IDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let kill = cineris_base::CancelToken::new();
    crate::core::lock(&live.running_commands).push(Running {
        id,
        cancel,
        kill,
        lane: true,
    });
    crate::core::lock(&live.sent_commands).push(Sent {
        client,
        call: call.clone(),
        id,
    });
}

/// One command sent in a `turn/start`, from the top of its step to its
/// end: its tokens are on the session, so that an interrupt from any client
/// that may drive the session cancels it at once. Its script, a question
/// and a program end then; a step still waiting before its script (on
/// another start, the extensions, `session.start`'s hooks) ends when that
/// wait does, and sends nothing. Each run has tokens of its own: `cancel`, a
/// child of the session's token made for it and held here, so that the
/// session's close and the core's shutdown stop it (a token keeps its
/// children only while something holds them: one made below a token
/// nothing holds is reached by nothing above it, the review of step 28's
/// commands, M2); and one that kills its programs at once, for a second
/// interrupt.
pub(crate) struct Run<'a> {
    live: &'a Live,
    id: u64,
    pub(crate) cancel: cineris_base::CancelToken,
    pub(crate) kill: cineris_base::CancelToken,
}

impl<'a> Run<'a> {
    /// A run of a command in `live` whose token is `cancel`, its tokens put
    /// on the session now.
    pub(crate) fn start(live: &'a Live, cancel: cineris_base::CancelToken) -> Self {
        let id = RUN_IDS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let kill = cineris_base::CancelToken::new();
        crate::core::lock(&live.running_commands).push(Running {
            id,
            cancel: cancel.clone(),
            kill: kill.clone(),
            lane: false,
        });
        Self {
            live,
            id,
            cancel,
            kill,
        }
    }

    /// The run of `call`, sent by `client` in a `turn/start`, as its step
    /// takes it: the one put on the session when it was sent ([`sent`]),
    /// interrupted already if an interrupt came behind it; the first of
    /// that client's that calls the same, since a lane runs a client's
    /// operations in the order sent. A new one, whose token is `cancel`,
    /// when none was put there: the operation came to the core without a
    /// lane (headless mode, a test). Taken at the top of the step, before
    /// anything can refuse it, so that what was put there is let go on
    /// every way out of the step: one left would be taken by a later
    /// `turn/start` of the same client and call, interrupted already if an
    /// interrupt came meanwhile (the review of step 28's commands, L3).
    pub(crate) fn taken(
        live: &'a Live,
        client: ClientId,
        call: &CommandCall,
        cancel: cineris_base::CancelToken,
    ) -> Self {
        let sent = {
            let mut sent = crate::core::lock(&live.sent_commands);
            sent.iter()
                .position(|one| one.client == client && one.call == *call)
                .map(|at| sent.remove(at).id)
        };
        let tokens = sent.and_then(|id| {
            crate::core::lock(&live.running_commands)
                .iter()
                .find(|run| run.id == id)
                .map(|run| (id, run.cancel.clone(), run.kill.clone()))
        });
        match tokens {
            Some((id, cancel, kill)) => Self {
                live,
                id,
                cancel,
                kill,
            },
            None => Self::start(live, cancel),
        }
    }

    /// `Err` when the run was interrupted: nothing is sent and no turn
    /// starts, said with the kind `interrupted`, which a front end does not
    /// show as a failure.
    pub(crate) fn go_on(&self, name: &str) -> Result<(), OpError> {
        if self.cancel.is_cancelled() {
            Err(interrupted(name))
        } else {
            Ok(())
        }
    }
}

impl Drop for Run<'_> {
    fn drop(&mut self) {
        crate::core::lock(&self.live.running_commands).retain(|run| run.id != self.id);
    }
}

/// A `turn/interrupt` from `client`, as it is sent through a lane
/// (`Core::early`): interrupts every command sent in a `turn/start` of
/// `live` whose step has not ended, running in its step or sent and
/// waiting in a lane for its step to begin ([`sent`]), and notes where it
/// was sent among the runs, for its own step ([`cut_of`]).
pub(crate) fn interrupt_sent(live: &Live, client: ClientId, now: bool) {
    let before = RUN_IDS.load(std::sync::atomic::Ordering::SeqCst);
    crate::core::lock(&live.interrupts_sent).push((client, before));
    interrupt(live, before, now);
}

/// Where the `turn/interrupt` of `client` whose step this is was sent among
/// the runs, taken: the number of the first run sent after it. `None` for
/// one that came to the core without a lane (headless mode, a test), whose
/// step reaches every command running then. Taken at the top of the step,
/// before anything can refuse it, so that none is left for a later one.
pub(crate) fn cut_of(live: &Live, client: ClientId) -> Option<u64> {
    let mut sent = crate::core::lock(&live.interrupts_sent);
    sent.iter()
        .position(|(one, _)| *one == client)
        .map(|at| sent.remove(at).1)
}

/// Interrupts every command of `live` sent in a `turn/start` whose step has
/// not ended, but one sent through a lane at or after the run numbered
/// `before`: an interrupt's own step, run in the lane's order, leaves a
/// command sent behind it to run (the review of step 28's commands, L2).
/// One that came to the core without a lane is reached whatever its number:
/// no early look reached it. With `now` it kills their programs at once.
pub(crate) fn interrupt(live: &Live, before: u64, now: bool) {
    for run in crate::core::lock(&live.running_commands)
        .iter()
        .filter(|run| !run.lane || run.id < before)
    {
        run.cancel.cancel();
        if now {
            run.kill.cancel();
        }
    }
}

/// The refusal of a command that was interrupted.
fn interrupted(name: &str) -> Box<cineris_protocol::ErrorData> {
    error(
        ErrorKind::Interrupted,
        code::COMMAND_FAILED,
        format!(
            "`{}` was interrupted, so nothing was sent and no turn started.",
            cineris_base::in_code(&format!("/{name}"))
        ),
    )
}

/// Why a command that names `profile` is not sent: what the profile would
/// widen, or, when it would widen nothing, that a command's profile is not
/// applied in this build.
///
/// A profile is judged as a project's file is: each key it sets against
/// the session's value in force, by the rule the configuration's schema
/// gives the key ([`cineris_config::Trust::class`]), and only a preference
/// passes. So it names a profile of the user's own layers, or of a trusted
/// project's; it never raises the level (the session's own), the sandbox,
/// its network or its write roots; it adds no `allow` rule; it changes no
/// provider or model to one the session does not use; and it sets nothing
/// that runs code, raises what is spent or is the user's alone.
fn profile_refusal(found: &Found, profile: &str) -> String {
    let named = cineris_base::in_code(profile);
    match judge_profile(&found.loaded, found.trusted, &found.level, profile) {
        Err(why) => format!("It names the settings profile `{named}`, which {why}."),
        Ok(()) => format!(
            "It names the settings profile `{named}`, and a command's profile is not applied in \
             this build: sent without it, the command would run with the session's own \
             settings, which may be wider than its author meant."
        ),
    }
}

/// Whether the profile `name` widens nothing of the session's: `Err` with
/// what it does, in a clause.
fn judge_profile(loaded: &Loaded, trusted: bool, level: &Level, name: &str) -> Result<(), String> {
    let usable = loaded.profiles.iter().any(|known| known == name)
        || (trusted && loaded.project_profiles.iter().any(|known| known == name));
    if !usable {
        return Err(
            if loaded.project_profiles.iter().any(|known| known == name) {
                "is this project's, and the project is not trusted".to_owned()
            } else {
                "no layer of the configuration defines".to_owned()
            },
        );
    }
    // Every layer's table of it, a project's too: what it would set.
    let tables = loaded.profile_tables(name);
    if tables.is_empty() {
        return Err("could not be read".to_owned());
    }
    for (_, table) in tables {
        for (key, value) in table {
            judge(loaded, level, &KeyPath::new([key.as_str()]), value)?;
        }
    }
    Ok(())
}

/// Judges one value a profile sets at `key`.
fn judge(loaded: &Loaded, level: &Level, key: &KeyPath, value: &Value) -> Result<(), String> {
    use cineris_config::schema::{Lookup, Meta, lookup};
    let shown = || cineris_base::in_code(&key.to_string()).into_owned();
    let trust = match lookup(key) {
        Lookup::Setting { child, .. } => match child.meta {
            Meta::Leaf { trust, joined, .. } => {
                if joined && let Value::Array(entries) = value {
                    for entry in entries {
                        let class = trust.class(entry, None);
                        if class != TrustClass::Preference {
                            return Err(widens(&shown(), class));
                        }
                    }
                    return Ok(());
                }
                trust
            }
            _ => return Err(format!("sets `{}`, which cannot be judged", shown())),
        },
        Lookup::Reserved { trust } => trust,
        Lookup::Table { .. } => {
            let Value::Table(table) = value else {
                return Err(format!("sets `{}`, which cannot be judged", shown()));
            };
            for (inner, value) in table {
                judge(loaded, level, &key.child(inner), value)?;
            }
            return Ok(());
        }
        _ => return Err(format!("sets `{}`, which cannot be judged", shown())),
    };
    // The level is judged against the session's own, which `session/set`
    // may have moved: at or below it narrows, above it widens, a level this
    // build does not know included.
    if key.to_string() == "permissions.mode" {
        let rank = |name: &str| Level::KNOWN.iter().position(|known| *known == name);
        return match (value.as_str().and_then(rank), rank(level.as_str())) {
            (Some(wanted), Some(now)) if wanted <= now => Ok(()),
            _ => Err(widens(&shown(), TrustClass::WidensAccess)),
        };
    }
    let current = loaded.value(key);
    match trust.class(value, current.as_ref()) {
        TrustClass::Preference => Ok(()),
        // The model, a reasoning level, a provider: only the session's own.
        TrustClass::RaisesCost if current.as_ref() == Some(value) => Ok(()),
        class => Err(widens(&shown(), class)),
    }
}

/// What a key of a class other than a preference does, in a clause.
fn widens(key: &str, class: TrustClass) -> String {
    let what = match class {
        TrustClass::RaisesCost => {
            "it could change the provider or the model, or raise what you spend"
        }
        TrustClass::WidensAccess => "it widens what the session may do",
        TrustClass::RunsCode => "it can run code",
        TrustClass::FirstLimit => "it sets a first limit, where the session has none",
        TrustClass::StopsCommands => "it can keep every command from working",
        TrustClass::UserOnly | TrustClass::KeptApart | TrustClass::Preference => {
            "it is above the session's, or yours alone to set"
        }
    };
    format!("sets `{key}`: {what}, and a command may only narrow the session")
}

/// The protocol's word for whose a file is.
fn source(owner: Owner) -> CommandSource {
    match owner {
        Owner::User => CommandSource::User,
        Owner::Project => CommandSource::Project,
    }
}

/// The protocol's word for whose an extension is.
#[cfg(feature = "scripts")]
fn script_source(source: cineris_ext::source::Source) -> CommandSource {
    match source {
        cineris_ext::source::Source::User => CommandSource::User,
        cineris_ext::source::Source::Project => CommandSource::Project,
    }
}

/// A clause as a sentence: its first letter in capitals, a full stop after.
fn sentence(clause: &str) -> String {
    let mut chars = clause.chars();
    let mut out: String = chars
        .next()
        .map(|first| first.to_uppercase().collect())
        .unwrap_or_default();
    out.push_str(chars.as_str());
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// Sentences that follow a colon: the first letter in lower case, the rest
/// as they are.
#[cfg(feature = "scripts")]
fn first_lower(sentences: &str) -> String {
    let mut chars = sentences.chars();
    let mut out: String = chars
        .next()
        .map(|first| first.to_lowercase().collect())
        .unwrap_or_default();
    out.push_str(chars.as_str());
    out
}

/// A sentence as a clause: its first letter in lower case, its full stop
/// taken off.
fn lower(sentence: &str) -> String {
    let mut chars = sentence.chars();
    let mut out: String = chars
        .next()
        .map(|first| first.to_lowercase().collect())
        .unwrap_or_default();
    out.push_str(chars.as_str());
    out.trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user's configuration of `text`, loaded with no project.
    fn loaded(text: &str) -> Loaded {
        #![allow(
            clippy::disallowed_methods,
            reason = "a test writes its own configuration file"
        )]
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cineris-core-commands-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.toml");
        std::fs::write(&file, text).unwrap();
        let env = cineris_config::Env::empty(cineris_config::Platform::CURRENT)
            .with("CINERIS_CONFIG", file.to_str().unwrap());
        let mut options = cineris_config::LoadOptions::new(env);
        options.dirs.managed_config = Some(dir.join("managed.toml"));
        let loaded = cineris_config::load(&options).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        loaded
    }

    /// A command's profile may only narrow the session: each key is judged
    /// against the value in force as a project's file would be, and only a
    /// preference passes. What widens the level, the sandbox, its network
    /// or write roots, adds an allow rule, runs code, changes the model or
    /// is the user's alone is refused, naming the key; what narrows passes.
    #[test]
    fn a_commands_profile_may_only_narrow_the_session() {
        let config = loaded(
            "[model]\nmain = \"claude-opus-5-5\"\n\n\
             [profiles.narrow]\npermissions.mode = \"read-only\"\nverify.mode = \"off\"\n\
             model.main = \"claude-opus-5-5\"\n\
             [[profiles.narrow.permissions.rules]]\neffect = \"exec\"\nmatch = \"git push\"\n\
             action = \"deny\"\n\n\
             [profiles.level]\npermissions.mode = \"autonomous\"\n\
             [profiles.network]\nsandbox.network = \"full\"\n\
             [profiles.roots]\nsandbox.write = [\"/tmp/elsewhere\"]\n\
             [profiles.allow]\n[[profiles.allow.permissions.rules]]\neffect = \"exec\"\n\
             match = \"rm\"\naction = \"allow\"\n\n\
             [profiles.model]\nmodel.main = \"claude-haiku-5-5\"\n\
             [profiles.sandbox]\nsandbox.enabled = false\n\
             [profiles.checks]\nverify.commands = [\"make\"]\n",
        );
        let judged = |name: &str, level: Level| judge_profile(&config, false, &level, name);
        assert_eq!(judged("narrow", Level::Supervised), Ok(()));
        for (name, key) in [
            ("level", "permissions.mode"),
            ("network", "sandbox.network"),
            ("roots", "sandbox.write"),
            ("allow", "permissions.rules"),
            ("model", "model.main"),
            ("sandbox", "sandbox.enabled"),
            ("checks", "verify.commands"),
        ] {
            let why = judged(name, Level::Supervised).unwrap_err();
            assert!(why.contains(&format!("sets `{key}`")), "{name}: {why}");
        }
        // At or below the session's own level it narrows; above it, it does
        // not, whatever the files say.
        assert_eq!(judged("level", Level::Autonomous), Ok(()));
        assert!(
            judged("absent", Level::Supervised)
                .unwrap_err()
                .contains("no layer")
        );
    }
}
