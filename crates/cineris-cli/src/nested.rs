//! The CLI's own grammar, handed to the permission engine: how a run of
//! Cineris that a session's command starts stands (since step 28's third
//! pull request: the early review's M3, the maintainers' Option A). The
//! engine finds each run of Cineris in a command line and asks this, with
//! the words after the program's name, so that what it refuses is what the
//! binary would do, read by [`crate::args::parse`] itself.
//!
//! - **Refused:** each subcommand that approves, trusts, keeps credentials
//!   or answers for the person ([`standing`] says which, with a choice for
//!   each variant of [`Command`] and of its subcommands, and no `_` arm);
//!   the application itself; a headless run carrying a flag whose
//!   [`Nested`](crate::args::Nested) choice refuses it; and the internal
//!   subcommands.
//! - **Safe:** the help, the version, the subcommands that only read, and a
//!   headless run with only safe flags.
//! - **Where more words follow that only the shell knows,** only a run whose
//!   subcommand and action are written out already, and only read, is
//!   safe.

use std::ffi::OsString;
use std::sync::Arc;

use cineris_core::approving::{Invocation, Kind, OwnCommands, Refusal, Standing};

use crate::args::{
    AuthCommand, Choice, Command, ConfigCommand, ExtCommand, FLAGS, Format, GUARD, Nested,
    PermissionsCommand, ProjectCommand, Run, SANDBOX, SessionsCommand, SetupCommand, StatsCommand,
    parse,
};

fn refused(what: impl Into<String>, kind: Kind) -> Standing {
    Standing::Refused(Refusal::new(what, kind))
}

fn approves(what: &str) -> Standing {
    refused(format!("cineris {what}"), Kind::Approves)
}

/// How a parsed command line stands, run by a session's command.
pub(crate) fn standing(command: &Command) -> Standing {
    match command {
        Command::Help(_) | Command::Version | Command::Models | Command::Usage(_) => Standing::Safe,
        Command::Run(run) => run_standing(run),
        Command::Sessions(sessions) => match sessions {
            SessionsCommand::List { .. } => Standing::Safe,
        },
        Command::Config(config) => match config {
            ConfigCommand::Show { .. } | ConfigCommand::Check { .. } | ConfigCommand::Defaults => {
                Standing::Safe
            }
        },
        Command::Permissions(permissions) => match permissions {
            PermissionsCommand::List => Standing::Safe,
            PermissionsCommand::Revoke { .. } => approves("permissions revoke"),
            PermissionsCommand::Trust { .. } => approves("permissions trust"),
            PermissionsCommand::Untrust => approves("permissions untrust"),
            PermissionsCommand::Forget => approves("permissions forget"),
        },
        Command::Auth(auth) => match auth {
            AuthCommand::Status => Standing::Safe,
            AuthCommand::Login { .. } => approves("auth login"),
            AuthCommand::Logout { .. } => approves("auth logout"),
        },
        Command::Stats(stats) => match stats {
            StatsCommand::Show { .. } => Standing::Safe,
            StatsCommand::Reset => approves("stats reset"),
        },
        Command::Project(project) => match project {
            ProjectCommand::List { .. } | ProjectCommand::Path => Standing::Safe,
            ProjectCommand::Change { action, .. } => approves(&format!("project {action}")),
            ProjectCommand::Edit { .. } => approves("project edit"),
            ProjectCommand::Add { .. } => approves("project add"),
        },
        Command::Ext(ext) => match ext {
            ExtCommand::List | ExtCommand::Inspect { .. } => Standing::Safe,
            ExtCommand::Disable { .. } => approves("ext disable"),
            ExtCommand::Remove { .. } => approves("ext remove"),
            ExtCommand::Add { .. } => approves("ext add"),
        },
        Command::Setup(setup) => match setup {
            SetupCommand::SkillFolders { .. } => approves("setup skill-folders"),
        },
        Command::Serve(_) => approves("serve"),
        Command::Attach(_) => approves("attach"),
    }
}

/// The flags a run carries, by the names [`FLAGS`] gives them, each with
/// its value where its choice depends on one.
pub(crate) fn flags_of(run: &Run) -> Vec<(&'static str, Option<String>)> {
    let fields = [
        ("-p", run.print),
        ("--output", run.format != Format::Text),
        ("--include-deltas", run.include_deltas),
        ("--continue", run.choice == Choice::Continue),
        ("--resume", matches!(run.choice, Choice::Resume(_))),
        ("--fork", matches!(run.choice, Choice::Fork(_))),
        ("--no-session", run.no_session),
        ("--allow", !run.allow.is_empty()),
        ("--deny", !run.deny.is_empty()),
        ("--trust-project", run.trust_project),
        ("--fail-on-denial", run.fail_on_denial),
        ("--system-append", run.system_append.is_some()),
        ("--profile", run.common.profile.is_some()),
        ("--log-level", run.log_level.is_some()),
        ("-q", run.quiet),
        ("-v", run.verbose),
    ];
    let mut given: Vec<(&'static str, Option<String>)> = fields
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| (name, None))
        .collect();
    for (_, value, flag) in &run.common.settings {
        let name = match flag.as_deref() {
            None => Some("-c"),
            Some(flag) => FLAGS
                .iter()
                .flat_map(|known| known.names.split(", "))
                .find(|name| *name == flag),
        };
        // A flag the table does not name is refused below.
        given.push((name.unwrap_or("an unknown flag"), Some(value.clone())));
    }
    given
}

/// How a run stands: the application is refused; a headless run, by the
/// choice of each flag it carries.
fn run_standing(run: &Run) -> Standing {
    if !run.print {
        return refused("cineris", Kind::Application);
    }
    for (name, value) in flags_of(run) {
        let choice = FLAGS
            .iter()
            .find(|flag| flag.names.split(", ").any(|known| known == name))
            .map_or(Nested::Refused, |flag| flag.nested);
        let allowed = match choice {
            Nested::Safe => true,
            Nested::Refused => false,
            Nested::Narrowing(values) => {
                value.is_some_and(|value| values.contains(&value.as_str()))
            }
        };
        if !allowed {
            return refused(format!("cineris -p {name}"), Kind::Widens);
        }
    }
    Standing::Safe
}

/// Whether the words written out already fix what runs, whatever more
/// words follow: a subcommand and its action, or a subcommand that takes
/// none.
fn closed(command: &Command, written: &[String]) -> bool {
    let given = |flags: [&str; 2]| written.iter().any(|word| flags.contains(&word.as_str()));
    match command {
        // The help that a missing action prints is not what was asked for:
        // the word that follows may be the action.
        Command::Help(_) => given(["-h", "--help"]),
        Command::Version => given(["-V", "--version"]),
        Command::Models | Command::Usage(_) => true,
        Command::Run(_) => false,
        _ => written.iter().filter(|word| !word.starts_with('-')).count() >= 2,
    }
}

/// The grammar the engine is given: how a run of Cineris with the words
/// of `invocation` stands. Where more words follow that only the shell
/// knows, only what they cannot change is safe; where the words may not be
/// the program's own (`exact` false), neither the help, the version nor
/// words it cannot read are.
pub(crate) fn grammar(invocation: &Invocation<'_>) -> Standing {
    let (rest, open, exact) = (invocation.words, invocation.open, invocation.exact);
    if let Some(first) = rest.first()
        && (first == SANDBOX || first == GUARD)
    {
        return refused(format!("cineris {first}"), Kind::Internal);
    }
    let parsed = |words: &[String]| parse(words.iter().map(OsString::from).collect());
    let hidden = || refused(format!("cineris {}", rest.join(" ")), Kind::Hidden);
    match parsed(rest) {
        // The binary refuses what it cannot read, and does nothing: where
        // the words are its own.
        Err(_) if !open && exact => Standing::Safe,
        Err(_) if !open => hidden(),
        Err(_) => {
            // An action that takes a word, written out: whatever the word
            // is, it is that action.
            let mut more = rest.to_vec();
            more.push("x".to_owned());
            match parsed(&more) {
                Ok(command) if closed(&command, rest) && standing(&command) == Standing::Safe => {
                    Standing::Safe
                }
                Ok(command) if standing(&command) != Standing::Safe => standing(&command),
                _ => hidden(),
            }
        }
        Ok(Command::Help(_) | Command::Version) if !exact => hidden(),
        Ok(command) => {
            let standing = standing(&command);
            if !open || standing != Standing::Safe || closed(&command, rest) {
                standing
            } else {
                hidden()
            }
        }
    }
}

/// What the engine is told of this program's own commands: its names, as
/// it was invoked and as it is on disk (a link under another name among
/// them), and its grammar.
pub(crate) fn own_commands() -> Arc<OwnCommands> {
    let name = |path: &std::path::Path| {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
    };
    let mut names = Vec::new();
    names.extend(
        std::env::args_os()
            .next()
            .and_then(|invoked| name(std::path::Path::new(&invoked))),
    );
    names.extend(std::env::current_exe().ok().and_then(|exe| name(&exe)));
    Arc::new(OwnCommands::new(names, Some(Arc::new(grammar))))
}

#[cfg(test)]
mod tests;
