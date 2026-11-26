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
    if let Some((extension, command)) = call.name.split_once(':')
        && let Some(loaded) = found
            .scripts
            .iter()
            .find(|loaded| loaded.name() == extension)
        && let Some(definition) = loaded
            .approved()
            .registration
            .commands
            .iter()
            .find(|definition| definition.name == command)
    {
        let text = run_script(live, loaded, definition, &call.args).await?;
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
    // a question it puts names the command in the turn's place.
    let turn = cineris_protocol::TurnId::new(format!("command_{}", extension.name()));
    let cx = RunCx::new(
        Arc::clone(&hooks.paths),
        Arc::clone(hooks.commands.content()),
        Arc::new(FileTable::new()),
    )
    .with_cancel(hooks.commands.runs_token())
    .with_gate(cineris_agent::Policy::read_gate(policy.as_ref()))
    .with_call(CallRef {
        session,
        turn,
        call_id: cineris_protocol::CallId::new(format!("command_{}", definition.name)),
        item: cineris_protocol::ItemId::new(format!(
            "command_{}_{}",
            extension.name(),
            definition.name
        )),
    });
    let failed = |why: String| error(ErrorKind::Tool, code::COMMAND_FAILED, sentence(&why));
    let value = extension
        .call_command(&definition.name, input, &cx)
        .await
        .map_err(failed)?;
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
