//! Putting one session together: configuration, trust, the permission
//! engine, the models, the store and the tools, each from the crate that
//! owns it (`docs/architecture/00-overview.md`).
//!
//! Everything here blocks (it reads configuration, the trust store, the
//! session's files, and looks for a shell), so the core runs it on the
//! blocking pool and opens the session's loop with what comes back.
//!
//! # Trust, worked out once
//!
//! Whether the project is trusted is worked out in one place ([`trust`]),
//! and it is two answers, because trust lets a project do two different
//! kinds of thing:
//!
//! - **Loosen what is asked, or raise what is spent** ([`Trust::rules`]):
//!   given to configuration (which holds back what a project may only set
//!   once trusted) and to the permission engine (which sets a project's
//!   `allow` rules aside until then). This answer is yes only while the
//!   project's rules are the ones the user saw: any change to them sets it
//!   aside until the change is approved.
//! - **Have git read the repository's own configuration**
//!   ([`Trust::repository`]): given to the engine (which takes git's
//!   commands for ones that only read in a trusted repository alone,
//!   because a repository's configuration can make git run a program), to
//!   the shell tool (which asks git what a command changed, under the same
//!   condition) and to the capture of the base state (the same again). A change to the
//!   project's permission rules does not touch what git would run, so
//!   this answer is the user's trust alone: a project that adds a `deny`
//!   does not lose its base state for it. Since the follow-ups of step 23
//!   it is the user's trust *with git shown*: a folder trusted before it
//!   was a repository has git asked about when it becomes one
//!   (`TrustStore`'s version 2).
//!
//! # The trust question's answers
//!
//! A project not trusted is asked "Trust it", "Not now" or "Never for this
//! project". A project trusted already, with what is new in it (its rules
//! changed, or git, now that it is a repository), is asked "Trust it",
//! "Not now" or "Keep trusting it, without what is new" ([`TrustAnswer`]):
//! never is not offered there, since it would take away the trust given
//! before (the owner's decision of 2026-11-16).

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "assembling a session reads files, on the blocking pool"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use cineris_agent::{
    Account, ContextParts, ContextSettings, Environment, Ledger, LoopConfig, Models, NoStats,
    SessionInfo, SessionParts, SpeedSettings, Workspace,
};
use cineris_base::{AbsPath, CancelToken, PathOptions, PathResolver};
use cineris_config::{LayerKind, LoadOptions, Loaded, Override, Settings, StateDir, TrustClass};
use cineris_llm::{BoxFuture, ImageError, ImageSource};
use cineris_policy::{
    ApprovalStore, Definition, Engine, NoSandbox, Parts, ProjectRules, Protected, RuleSource,
    SandboxState, ShellSyntax, SourcedRule, StartError, TrustStore, default_sandbox,
};
use cineris_protocol::event::{Event, Notice, SandboxNotice};
use cineris_protocol::lease::{Budget, HardLimits, Rule};
use cineris_protocol::methods::{TrustResource, TrustResourceKind};
use cineris_protocol::model::ModelRole;
use cineris_protocol::session::{SessionSettings, TrustStanding};
use cineris_protocol::{ArtifactId, ErrorData, ErrorKind, LeaseId, Plan, ProjectKey, SessionId};
use cineris_session::entry::{LeaseEvent, LeaseRecord, Meta};
use cineris_session::{
    BaseCapture, BaseLimits, Body, Durability, Git, KEEP_LIMIT, SessionError, SessionOptions,
    SessionsRoot, SystemGit,
};
use cineris_tools::process::{Guard, Runner};
use cineris_tools::{
    ContentStore, EditTool, Editing, FileTable, MemoryCheckpoints, MemoryProposals, MemoryStore,
    OutputLimits, PlanBoard, PlanTool, RememberTool, Shell, ShellConfig, ShellSearch, ShellTool,
    ToolRegistry, Toolbox, WriteTool,
};

use crate::checkpoints::{RecordingShell, Rounds, StopsWithTurn};
use crate::content::{DiskContent, StoredImages};
use crate::core::Inner;
use crate::events::SessionSink;
use crate::instructions::DiskInstructions;
use crate::policy::{EnginePolicy, PolicyParts};
use crate::provider;
use crate::redact::LogRedactor;
use crate::sandbox::{self, SessionGit, SessionSandbox, Temp, status_of};
use crate::shell_cache;
use crate::store::{Disk, Hooks, Kept, LogStore, SharedLog};
use cineris_tools::shell::ShellKind;

/// Where a session's commands start, given `session/new`'s `cwd`: the
/// directory relative to the root, with `/` between names, or `None` for
/// the root itself (since the follow-ups of step 23).
///
/// # Errors
///
/// Of kind `usage` (`invalid_cwd`) when `cwd` is not inside the workspace.
fn start_below(root: &AbsPath, cwd: &Path) -> Result<Option<String>, Box<ErrorData>> {
    let outside = || {
        error(
            ErrorKind::Usage,
            "invalid_cwd",
            format!(
                "`{}` is not inside the workspace `{}`: a session's commands start inside it.",
                cineris_base::in_code(&cwd.display().to_string()),
                cineris_base::in_code(root.as_str())
            ),
        )
    };
    let resolved = resolve_dir(cwd).ok_or_else(outside)?;
    let rest = cineris_base::strip_location_prefix(&resolved.to_fs_path(), &root.to_fs_path())
        .ok_or_else(outside)?;
    // A name that is not Unicode cannot be said to the model as it is: the
    // session starts at the root (the review of the follow-ups of step 23).
    let Some(names) = rest
        .components()
        .map(|part| part.as_os_str().to_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()
    else {
        return Ok(None);
    };
    Ok((!names.is_empty()).then(|| names.join("/")))
}

/// Whether a start kept in a session's log names a directory below the
/// root: names with `/` between them, none empty, `.` or `..`, and nothing
/// that makes it absolute.
fn is_below(start: &str) -> bool {
    !start.is_empty()
        && start.split('/').all(|name| {
            let mut parts = Path::new(name).components();
            matches!(
                (parts.next(), parts.next()),
                (Some(std::path::Component::Normal(only)), None) if only == name
            )
        })
}

/// Whether the workspace is in a git repository: a `.git` in it or in a
/// directory above it. Looked at, not asked of git, which starts nothing.
fn in_git(root: &AbsPath) -> bool {
    in_git_at(&root.to_fs_path())
}

/// [`in_git`] of a root given as a path.
pub(crate) fn in_git_at(root: &Path) -> bool {
    root.ancestors()
        .any(|dir| std::fs::symlink_metadata(dir.join(".git")).is_ok())
}

/// The context's settings, from `[context]` and `compat.instructions`.
///
/// `compat.instructions` governs the names other tools' files have,
/// `AGENTS.md` and `CLAUDE.md`, in the workspace: off, only the names in
/// `context.instruction_names` are read there. The user's own files in the
/// configuration directory are the harness's, and are read either way, as
/// `[compat]` is for readers of other tools' files only
/// (`docs/architecture/08-configuration.md`; the review of step 15, its
/// fourth note, found both turned off with it).
pub(crate) fn context_settings(settings: &Settings) -> ContextSettings {
    let context = &settings.context;
    let mut names: Vec<String> = if settings.compat.instructions {
        cineris_agent::context::DEFAULT_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    } else {
        Vec::new()
    };
    // Only plain documents: a name with a separator, or one that is not a
    // `.md` file, could make the harness read and send something that is
    // not instructions (`cineris_agent::context::is_instruction_name`).
    for name in &context.instruction_names {
        if cineris_agent::context::is_instruction_name(name) && !names.contains(name) {
            names.push(name.clone());
        }
    }
    ContextSettings {
        auto_compact: context.auto_compact,
        background: context.background,
        compact_at: context.compact_at,
        prepare_at: context.prepare_at,
        keep_recent_tokens: context.keep_recent_tokens,
        instruction_names: names,
        instructions_cap: usize::try_from(context.instructions_cap).unwrap_or(usize::MAX),
        ..ContextSettings::default()
    }
}

fn error(kind: ErrorKind, code: &str, message: impl Into<String>) -> Box<ErrorData> {
    Box::new(ErrorData::new(kind, code, message))
}

fn session_error(error: &SessionError) -> Box<ErrorData> {
    Box::new(cineris_base::Classified::to_error_data(error))
}

/// Which session to put together.
#[derive(Debug, Clone)]
pub(crate) enum Which {
    /// A new one, with this id.
    New {
        id: SessionId,
        title: Option<String>,
        ephemeral: bool,
        /// Where its commands start when it is not the root: `session/new`'s
        /// `cwd`, inside the workspace.
        cwd: Option<PathBuf>,
    },
    /// A stored one.
    Resume { id: SessionId },
}

/// A session to write as a fork of another, as it is put together: once
/// its configuration has passed the gate of its level, so that a fork that
/// is refused leaves no stored session, and the configuration is read once
/// (the reviews of step 27's second pull request).
#[derive(Clone)]
pub(crate) struct ForkFrom {
    /// The log of the session forked.
    pub(crate) log: SharedLog,
    /// The entry the fork goes back to: the end of the branch with none.
    pub(crate) at: Option<cineris_session::entry::EntryId>,
}

impl std::fmt::Debug for ForkFrom {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForkFrom")
            .field("at", &self.at)
            .finish_non_exhaustive()
    }
}

/// What a session is asked to be.
#[derive(Debug, Clone)]
pub(crate) struct Wanted {
    pub(crate) which: Which,
    /// For `session/fork`: the session to write it from, before it is
    /// opened as a stored one (`which` is then [`Which::Resume`]).
    pub(crate) fork: Option<ForkFrom>,
    pub(crate) workspace: PathBuf,
    pub(crate) settings: SessionSettings,
    /// Overrides for this session, over the core's own.
    pub(crate) overrides: Vec<Override>,
    /// Whether the session may not start at a higher level than it would
    /// without `settings` and `overrides`: they come from a client that is
    /// not a `user`, which `session/set` would not let raise the level.
    pub(crate) no_raise: bool,
}

/// The images of a session's store, which exists only after the clients
/// that will ask for them.
#[derive(Default)]
struct LateImages(OnceLock<StoredImages>);

impl std::fmt::Debug for LateImages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LateImages").finish_non_exhaustive()
    }
}

impl ImageSource for LateImages {
    fn load<'a>(&'a self, artifact: &'a ArtifactId) -> BoxFuture<'a, Result<Vec<u8>, ImageError>> {
        match self.0.get() {
            Some(images) => images.load(artifact),
            None => Box::pin(std::future::ready(Err(ImageError::new(
                "the session's store is not open",
            )))),
        }
    }
}

/// What the engine was started from, kept so that it can be started again
/// at another level.
#[derive(Debug, Clone)]
pub(crate) struct EngineTemplate {
    pub(crate) parts: Parts,
}

/// A session, put together and not yet running.
pub(crate) struct Assembled {
    pub(crate) parts: SessionParts,
    pub(crate) sink: Arc<SessionSink>,
    pub(crate) log: SharedLog,
    pub(crate) hooks: Arc<Hooks>,
    pub(crate) policy: Arc<EnginePolicy>,
    pub(crate) board: Arc<PlanBoard>,
    pub(crate) sandbox: Arc<SessionSandbox>,
    pub(crate) template: Arc<std::sync::Mutex<EngineTemplate>>,
    pub(crate) loaded: Arc<Loaded>,
    pub(crate) images: Arc<dyn ImageSource>,
    /// The session's store of versions and artifacts.
    pub(crate) content: Arc<dyn ContentStore>,
    /// What to tell front ends once the session is open.
    pub(crate) notices: Vec<Event>,
    pub(crate) lease_count: u64,
    /// The repository index, whose first pass starts once the loop runs.
    pub(crate) repo: Option<crate::repo_map::RepoIndex>,
    /// The project's knowledge as this session has it. `None` with no
    /// state directory to keep it in.
    pub(crate) knowledge: Option<Arc<crate::knowledge::Knowledge>>,
    /// What a reload of the configuration works from and changes (step
    /// 24): the configuration in force and how it was loaded.
    pub(crate) config: crate::reload::SessionConfig,
    /// The files the session's configuration was read from, stamped.
    pub(crate) watched: crate::reload::Watched,
    /// The session's verifier, which `verify/run` asks first whether any
    /// check could run, and whose settings a reload changes.
    pub(crate) verifier: Arc<crate::verify::CoreVerifier>,
    /// The accounts the ledger draws on, the lease's first and the day's
    /// after it: a reload changes their budgets and keeps what was spent.
    pub(crate) accounts: Vec<Account>,
    /// The shell's settings a reload changes, when there is a shell.
    pub(crate) shell_live: Arc<cineris_tools::ShellLive>,
    /// Whether git may read the repository's own configuration: the shell's
    /// look and the base capture read it, and a reload lowers it.
    pub(crate) repository: Arc<std::sync::atomic::AtomicBool>,
    /// The session's extensions, to find once its loop runs (step 28).
    pub(crate) extensions: crate::ext::found::Pending,
    /// The session's hook commands (step 28).
    #[cfg(feature = "scripts")]
    pub(crate) command_hooks: Arc<crate::ext::SessionHooks>,
}

/// The root of the workspace at `path`, as a session opened there would
/// have it.
pub(crate) fn workspace_root(path: &Path) -> Option<AbsPath> {
    PathResolver::new(path, PathOptions::new())
        .ok()
        .map(|resolver| resolver.root().clone())
}

/// Where a directory really is: resolved through any link, whether or not
/// all of it exists yet.
///
/// The deepest ancestor that is a directory is found with a `metadata` call
/// each, and only there is a resolver made, which walks the path from its
/// root: making one for each missing level in turn cost a session's set-up
/// a third of its time on Windows (27 resolvers, 9 ms of 20, the seventh
/// tooling step). A directory the resolver refuses is passed over as a
/// missing one is, as before.
pub(crate) fn resolve_dir(path: &Path) -> Option<AbsPath> {
    let mut missing: Vec<&std::ffi::OsStr> = Vec::new();
    let mut existing = path;
    loop {
        let is_dir = std::fs::metadata(existing).is_ok_and(|metadata| metadata.is_dir());
        if is_dir && let Ok(resolver) = PathResolver::new(existing, PathOptions::new()) {
            let mut resolved = resolver.root().clone();
            for name in missing.iter().rev() {
                resolved = resolved.child(name).ok()?;
            }
            return Some(resolved);
        }
        missing.push(existing.file_name()?);
        existing = existing.parent()?;
    }
}

/// The source of a rule the configuration holds, by the layer it is in.
pub(crate) fn rule_source(layer: LayerKind, defined_in: Option<LayerKind>) -> RuleSource {
    match layer {
        LayerKind::Managed => RuleSource::Managed,
        LayerKind::Project | LayerKind::ProjectLocal => RuleSource::Project,
        LayerKind::CommandLine => RuleSource::CommandLine,
        // A profile is the user's unless a project's file defines it.
        LayerKind::Profile => match defined_in {
            Some(LayerKind::Project | LayerKind::ProjectLocal) => RuleSource::Project,
            Some(LayerKind::Managed) => RuleSource::Managed,
            _ => RuleSource::User,
        },
        _ => RuleSource::User,
    }
}

/// Every permission rule of a loaded configuration, with who wrote it.
fn sourced_rules(loaded: &Loaded) -> Vec<SourcedRule> {
    loaded
        .rules()
        .into_iter()
        .map(|placed| {
            let source = rule_source(placed.origin.layer, placed.origin.defined_in);
            SourcedRule::new(placed.rule, source)
        })
        .collect()
}

/// A yes the store could not keep, which trust for this session holds to:
/// the rules the person saw, and whether git was trusted with them (since
/// the follow-ups of step 23: a yes to changed rules alone is no yes to
/// git, on a reload as at the open).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ForTheRun {
    pub(crate) pin: Definition,
    pub(crate) git: bool,
    /// The project's extensions the question listed, by the hashes shown:
    /// approved for this session alone (the maintainers' call on step 28's
    /// early review). A folder changed since stands changed.
    pub(crate) extensions: Vec<Definition>,
    /// The project's hooks the question listed, by the hashes shown,
    /// approved for this session alone as its extensions are. One changed
    /// since stands changed.
    pub(crate) hooks: Vec<Definition>,
}

/// What was learned about the project's trust.
pub(crate) struct Trust {
    /// Whether the project may loosen what is asked and raise what is
    /// spent: it is trusted, and its rules are the ones the user saw.
    rules: bool,
    /// Whether git may read the repository's own configuration: the user
    /// trusts the project, with git shown.
    repository: bool,
    loaded: Loaded,
    /// The files `loaded` was read from, stamped as they were read: what a
    /// reload of the session looks at (step 24). `None` where the trust was
    /// worked out otherwise; the session's first reload then makes it.
    watched: Option<cineris_config::LiveConfig>,
    /// The rules the person said yes to when the store could not keep the
    /// yes: trust for this session only holds to them, as the store's pins
    /// would (the second review of step 24's first part).
    pub(crate) for_the_run: Option<ForTheRun>,
    /// What went wrong reading the trust store, for the user.
    problem: Option<String>,
    /// Why git does not read the repository's configuration in a trusted
    /// project, for the user.
    git_note: Option<String>,
    /// What the user has said of the project, for a front end's status.
    standing: TrustStanding,
    /// The project is trusted, its rules have changed, and the person kept
    /// trusting it without these rules: they are not asked about.
    rules_declined: bool,
    /// The project is trusted and is a repository, and git was never shown
    /// (nor declined): it is asked about.
    git_new: bool,
    /// The project's trust as the store gave it when this was worked out:
    /// `None` when the store was not read, or could not be. The trust
    /// question is shaped by this one reading (the early review of step
    /// 28's second part: a second reading could differ from the first).
    stored: Option<cineris_policy::ProjectTrust>,
    /// What the trust question found in the project's folder of
    /// extensions, when it looked: the session's finding takes it in place
    /// of reading the folder again.
    found: Option<crate::ext::found::Looked>,
}

/// The project's rules as trust pins them (the owner's answer of
/// 2026-11-15): those its files hold outside any profile, and each profile
/// its files define with the rules it adds, whichever profile is selected.
/// `loaded` is a load that trusts the project; `with_profile` loads it so
/// with another profile selected.
pub(crate) fn pinned_rules(
    loaded: &Loaded,
    with_profile: &dyn Fn(&str) -> Result<Loaded, Box<ErrorData>>,
) -> Result<ProjectRules, Box<ErrorData>> {
    let project = |kind: LayerKind| matches!(kind, LayerKind::Project | LayerKind::ProjectLocal);
    let own = |loaded: &Loaded, in_profile: bool| -> Vec<Rule> {
        loaded
            .rules()
            .into_iter()
            .filter(|placed| {
                if in_profile {
                    placed.origin.layer == LayerKind::Profile
                        && placed.origin.defined_in.is_some_and(project)
                } else {
                    project(placed.origin.layer)
                }
            })
            .map(|placed| placed.rule)
            .collect()
    };
    let mut rules = ProjectRules {
        base: own(loaded, false),
        profiles: Vec::new(),
    };
    for name in &loaded.project_profiles {
        let rules_of = if loaded.profile.as_deref() == Some(name.as_str()) {
            own(loaded, true)
        } else {
            let other = with_profile(name)?;
            // A profile the managed file's lock keeps from being selected
            // never applies: another profile's rules are not pinned under
            // its name (the review of step 23).
            if other.profile.as_deref() != Some(name.as_str()) {
                continue;
            }
            own(&other, true)
        };
        rules.profiles.push((name.clone(), rules_of));
    }
    Ok(rules)
}

/// The project's settings that wait for trust, as the question lists them
/// and as trusting turns them on: those its files set outside any profile,
/// and those each profile its files define sets, whichever is selected,
/// each with the profile (`None`: outside any). `loaded` is a load that
/// does not trust the project; `with_profile` loads it so with another
/// profile selected (the review of step 23: a project's profile can be
/// selected, and its settings must have been shown).
pub(crate) fn held_back_settings(
    loaded: &Loaded,
    with_profile: &dyn Fn(&str) -> Result<Loaded, Box<ErrorData>>,
) -> Result<Vec<(Option<String>, cineris_config::Withheld)>, Box<ErrorData>> {
    let project = |kind: LayerKind| matches!(kind, LayerKind::Project | LayerKind::ProjectLocal);
    let in_profile = |withheld: &cineris_config::Withheld| {
        withheld.origin.layer == LayerKind::Profile
            && withheld.origin.defined_in.is_some_and(project)
    };
    let mut settings: Vec<_> = loaded
        .withheld
        .iter()
        .filter(|withheld| withheld.origin.layer != LayerKind::Profile)
        .map(|withheld| (None, withheld.clone()))
        .collect();
    for name in &loaded.project_profiles {
        let other;
        let of = if loaded.profile.as_deref() == Some(name.as_str()) {
            loaded
        } else {
            other = with_profile(name)?;
            // One the managed file's lock keeps from being selected never
            // applies.
            if other.profile.as_deref() != Some(name.as_str()) {
                continue;
            }
            &other
        };
        settings.extend(
            of.withheld
                .iter()
                .filter(|withheld| in_profile(withheld))
                .map(|withheld| (Some(name.clone()), withheld.clone())),
        );
    }
    Ok(settings)
}

/// What the trust question asks of a project, and what a yes pins.
#[derive(Debug, Clone)]
pub(crate) struct TrustQuestion {
    pub(crate) key: ProjectKey,
    /// The workspace's root, as a person reads it.
    pub(crate) root: String,
    pub(crate) resources: Vec<TrustResource>,
    /// The definition of the rules that were shown: a yes pins this, not
    /// the files as they are when the answer comes.
    pub(crate) pin: Definition,
    /// The project is trusted already: the question is about what is new
    /// in it, and its third answer is "keep trusting it, without what is
    /// new", not never.
    pub(crate) trusted: bool,
    /// The rules were listed: a yes pins them, and a keep (of a trusted
    /// project) declines them.
    pub(crate) rules_shown: bool,
    /// Git was listed: a yes trusts it, and a keep declines it.
    pub(crate) git_shown: bool,
    /// The project's extensions that were listed, as they were read: a yes
    /// pins each (step 28).
    pub(crate) extensions: Vec<Definition>,
    /// The project's hooks that were listed, as they were read: a yes pins
    /// each, kind `hook`, by the hash of what was shown (step 28).
    pub(crate) hooks: Vec<Definition>,
}

/// A person's answer to the trust question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustAnswer {
    /// Trust it: pin what was shown.
    Yes,
    /// Not now: not asked again by this core, and nothing stored.
    NotNow,
    /// Never for this project: stored, and not asked at later starts. Only
    /// of a project not trusted.
    Never,
    /// Keep trusting it as it was trusted, without what is new: stored, and
    /// what is new not asked about while it is what it is. Only of a
    /// project trusted already (the follow-ups of step 23).
    Keep,
}

/// What one setting held back for trust can do, said to the person asked.
fn held_back(withheld: &cineris_config::Withheld) -> String {
    let does = match withheld.class {
        TrustClass::FirstLimit => "a first limit, where you have none: it could stop every turn",
        TrustClass::RunsCode => "it can run code",
        TrustClass::WidensAccess => "it widens what the session may do",
        TrustClass::RaisesCost => "it can raise what you spend",
        TrustClass::StopsCommands => {
            "it takes variables from every command: it could stop them all"
        }
        _ => "it waits for trust",
    };
    format!("{} = {} ({does})", withheld.key, withheld.value)
}

/// Where a session's workspace is and how its configuration is loaded:
/// the same for the look at its trust and for its assembly.
struct Place {
    resolver: Arc<PathResolver>,
    root: AbsPath,
    load: LoadOptions,
    state: Option<StateDir>,
}

/// The resolver of a session in `workspace`: the home directory the
/// core's environment gives is what `~` stands for.
pub(crate) fn path_options(inner: &Inner) -> PathOptions {
    let home = inner.options.env.home().as_deref().and_then(resolve_dir);
    let mut path_options = PathOptions::new();
    if let Some(home) = home {
        path_options = path_options.home(home);
    }
    path_options
}

fn place(inner: &Inner, wanted: &Wanted) -> Result<Place, Box<ErrorData>> {
    let options = &inner.options;
    let env = &options.env;
    let resolver = PathResolver::new(&wanted.workspace, path_options(inner)).map_err(|failure| {
        error(
            ErrorKind::Usage,
            "invalid_workspace",
            format!(
                "`{}` cannot be a workspace: {failure}",
                cineris_base::in_code(&wanted.workspace.display().to_string())
            ),
        )
    })?;
    let resolver = Arc::new(resolver);
    let root = resolver.root().clone();
    let mut load = LoadOptions::new(env.clone());
    if let Some(dirs) = &options.dirs {
        load.dirs = dirs.clone();
    }
    load.workspace = Some(root.to_fs_path());
    load.profile.clone_from(&options.profile);
    load.overrides.clone_from(&options.overrides);
    load.overrides.extend(wanted.overrides.iter().cloned());
    load.overrides.extend(settings_overrides(&wanted.settings));
    let state = StateDir::from_dirs(&load.dirs);
    Ok(Place {
        resolver,
        root,
        load,
        state,
    })
}

/// The project's trust as a session would be put together with it, and
/// the question to ask a person who can answer it, if there is one: a
/// project not trusted, or trusted with its rules changed since, that the
/// person has not said never to, nor not now to in this core's life, and
/// that has something to trust (git, in a repository; a rule; a setting
/// held back). Blocks.
pub(crate) fn look(
    inner: &Inner,
    wanted: &Wanted,
) -> Result<(Trust, Option<TrustQuestion>), Box<ErrorData>> {
    let place = place(inner, wanted)?;
    let mut trust = trust(inner, place.state.as_ref(), &place.root, &place.load)?;
    if inner.options.trust_project {
        return Ok((trust, None));
    }
    // The question is shaped by the one reading of the store that the
    // trust was worked out from, never by a second, which a change in
    // between could make another (the early review of step 28's second
    // part: settings held back by the first and not by the second would be
    // turned on by a yes without having been shown).
    cineris_base::hook::point(crate::ext::found::THE_QUESTIONS_TRUST_IS_READ);
    let Some(stored) = trust.stored.clone() else {
        // A store that cannot be read is said, and nothing is asked: an
        // answer could not be kept.
        return Ok((trust, None));
    };
    let key = place.root.project_key();
    if stored.is_refused()
        || inner
            .not_now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&key)
    {
        return Ok((trust, None));
    }
    // A trusted project is asked about only what is new in it: its rules
    // changed and not kept out, git, now that it is a repository, or an
    // extension not approved as it is now (step 28). The project's folder
    // of extensions is looked in only now, below the never and the not
    // now: a look when it has none, two when it has a `.cineris`. The
    // user's folder is listed for the names it takes, and the places in
    // its extensions looked at within the session's bound, none of their
    // files read, only when the project has an extension. What was read
    // is handed to the session, which does not read it again.
    let rules_new = !trust.rules && !trust.rules_declined;
    let quiet = trust.standing != TrustStanding::Untrusted && !rules_new && !trust.git_new;
    let (found, watched) = crate::ext::found::in_project(&place.resolver, &inner.options.clock);
    let trusted_load = |profile: Option<&str>| {
        let mut options = place.load.clone();
        options.trust = cineris_config::ProjectTrust::Trusted;
        if let Some(profile) = profile {
            options.profile = Some(profile.to_owned());
        }
        cineris_config::load(&options).map_err(|failure| {
            Box::new(ErrorData::new(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&inner.options.env),
            ))
        })
    };
    let mut peeked = None;
    // The project's hooks, read apart from the user's whatever its trust:
    // each not approved as it is now is listed (step 28).
    let configured_hooks = crate::ext::hooks::Configured::of(&trust.loaded);
    // What its programs can do is said of the load a yes puts in force,
    // the project's own settings in it: the question's own may hold back
    // a `sandbox.write` that turns the sandbox off (the fresh review of
    // step 28). A project whose rules are trusted has it loaded already.
    let confined = if found.found.is_empty() && configured_hooks.project.is_empty() {
        false
    } else if trust.rules {
        programs_confined(inner, &trust.loaded, &place.root, place.state.as_ref())
    } else {
        let load = trusted_load(None)?;
        let confined = programs_confined(inner, &load, &place.root, place.state.as_ref());
        peeked = Some(load);
        confined
    };
    let extensions = if found.found.is_empty() {
        crate::ext::found::Asked::default()
    } else {
        let users = crate::ext::found::UserNames::listed(
            &place.resolver,
            place.load.dirs.config.as_deref(),
        );
        crate::ext::found::asked(&found, &users, &stored, confined)
    };
    let hooks = crate::ext::hooks::asked(&configured_hooks, &stored, confined);
    // Handed to the session without the extensions' bytes, its places
    // stamped as they were before they were read.
    trust.found = Some(crate::ext::found::looked(&found, watched));
    if quiet && extensions.pins.is_empty() && hooks.pins.is_empty() {
        return Ok((trust, None));
    }
    let trusted = stored.is_trusted();
    // What the question lists: everything for a project not trusted; for
    // one trusted, what is new.
    let rules_shown = !trusted || rules_new;
    let git_shown = if trusted {
        trust.git_new
    } else {
        in_git(&place.root)
    };
    let peek = match peeked {
        Some(peek) => peek,
        None => trusted_load(None)?,
    };
    let rules = pinned_rules(&peek, &|profile| trusted_load(Some(profile)))?;
    let mut resources = Vec::new();
    if git_shown {
        resources.push(TrustResource {
            kind: TrustResourceKind::Repository,
            name: "git".to_owned(),
            detail: "Git may read this repository's own configuration, which can name programs \
                     for git to run: it is how the harness sees what a command changed, and \
                     reads the repository's state."
                .to_owned(),
            changed: false,
        });
    }
    for (profile, rule) in rules.each().filter(|_| rules_shown) {
        resources.push(TrustResource {
            kind: TrustResourceKind::Rules,
            name: profile.map_or_else(
                || "permissions.rules".to_owned(),
                |profile| format!("profiles.{profile}.permissions.rules"),
            ),
            detail: format!(
                "{} {} {}",
                rule.action.as_str(),
                rule.effect.as_str(),
                rule.pattern
            ),
            changed: trusted,
        });
    }
    let untrusted_load = |profile: &str| {
        let mut options = place.load.clone();
        options.trust = cineris_config::ProjectTrust::Untrusted;
        options.profile = Some(profile.to_owned());
        cineris_config::load(&options).map_err(|failure| {
            Box::new(ErrorData::new(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&inner.options.env),
            ))
        })
    };
    // A trusted project's settings wait only for its changed rules: with
    // those kept out, or unchanged, they are not what is asked about.
    let settings = if rules_shown {
        held_back_settings(&trust.loaded, &untrusted_load)?
    } else {
        Vec::new()
    };
    for (profile, withheld) in settings {
        resources.push(TrustResource {
            kind: TrustResourceKind::Settings,
            name: match profile {
                Some(profile) => format!("profiles.{profile}.{}", withheld.key),
                None => withheld.key.to_string(),
            },
            detail: held_back(&withheld),
            changed: false,
        });
    }
    resources.extend(extensions.resources);
    resources.extend(hooks.resources);
    // A held-back write root that would turn the sandbox off is said before
    // anything else it lists (the maintainers, after the fresh review of
    // step 28): what trust turns on there is the sandbox off.
    if rules_shown {
        let warnings: Vec<TrustResource> = harness_write_warnings::<Box<ErrorData>>(
            inner,
            &place.root,
            place.state.as_ref(),
            (&trust.loaded, &peek),
            &|profile| Ok((untrusted_load(profile)?, trusted_load(Some(profile))?)),
        )?
        .into_iter()
        .map(|detail| TrustResource {
            kind: TrustResourceKind::Settings,
            name: "sandbox.write".to_owned(),
            detail,
            changed: false,
        })
        .collect();
        resources.splice(0..0, warnings);
    }
    if resources.is_empty() {
        return Ok((trust, None));
    }
    let question = TrustQuestion {
        root: shown_root(&place.root, inner.options.env.home().as_deref()),
        key,
        resources,
        pin: Definition::of_project_rules(&rules),
        trusted,
        rules_shown,
        git_shown,
        extensions: extensions.pins,
        hooks: hooks.pins,
    };
    Ok((trust, Some(question)))
}

/// Keeps a person's answer to the trust question, and gives the trust to
/// put the session together with: after a yes, worked out again from the
/// store, so that a change to the files while the question was open reads
/// as rules changed. Not now is kept in this core only. Blocks.
///
/// An answer the trust store cannot keep still opens the session, with a
/// notice (the review of step 23): a yes trusts the project for this
/// session alone, as `--trust-project` would, while its rules are still the
/// ones shown; a never or a keep is taken as not now.
pub(crate) fn answer_trust(
    inner: &Inner,
    wanted: &Wanted,
    question: &TrustQuestion,
    answer: TrustAnswer,
    mut trust: Trust,
) -> Result<Trust, Box<ErrorData>> {
    // What the question found in the project's folder goes on to the
    // session, whatever the answer made of the trust.
    let found = trust.found.take();
    let mut answered = kept_answer(inner, wanted, question, answer, trust)?;
    answered.found = found;
    Ok(answered)
}

/// [`answer_trust`], but for what the question found.
fn kept_answer(
    inner: &Inner,
    wanted: &Wanted,
    question: &TrustQuestion,
    answer: TrustAnswer,
    trust: Trust,
) -> Result<Trust, Box<ErrorData>> {
    let place = place(inner, wanted)?;
    let Some(state) = place.state.as_ref() else {
        return Ok(trust);
    };
    let store = TrustStore::new(state.trust_file(), Arc::clone(&inner.options.clock));
    let not_now = || {
        inner
            .not_now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(question.key.clone());
    };
    // The project's trust was taken away while a question about what is
    // new in it was open (revoked, forgotten or refused, by another client
    // or process): an answer about what is new is no answer about the whole
    // project, nor past a never.
    // And a first question's yes is no yes past a never said while it was
    // open (the review of the follow-ups of step 23).
    let gone = |failure: &cineris_policy::TrustError| {
        matches!(failure, cineris_policy::TrustError::NotUnderstood { .. })
            && store.project(&question.key).is_ok_and(|now| {
                if question.trusted {
                    !now.is_trusted()
                } else {
                    now.is_refused()
                }
            })
    };
    let gone_trust = || -> Result<Trust, Box<ErrorData>> {
        let mut now = self::trust(inner, place.state.as_ref(), &place.root, &place.load)?;
        now.problem = Some(
            "This project's trust changed while you were asked, so your answer was not used and \
             nothing of the project that needs trust is used in this session. A session opened \
             again asks afresh, unless the project was marked never."
                .to_owned(),
        );
        Ok(now)
    };
    match answer {
        TrustAnswer::Yes => {
            let mut pin = if question.rules_shown {
                vec![question.pin.clone()]
            } else {
                Vec::new()
            };
            // Each extension and hook listed, as it was read when it was
            // listed.
            pin.extend(question.extensions.iter().cloned());
            pin.extend(question.hooks.iter().cloned());
            let kept = if question.trusted {
                store.approve(&question.key, &pin, question.git_shown)
            } else {
                store.trust_unless_refused(&question.key, &question.root, &pin, question.git_shown)
            };
            match kept {
                Ok(_) => self::trust(inner, place.state.as_ref(), &place.root, &place.load),
                // Nothing is written, and the session opens with the trust
                // as it stands now (the review of step 23's fixes).
                Err(failure) if gone(&failure) => gone_trust(),
                Err(failure) => {
                    trusted_unkept(inner, &place.load, question, &failure, trust.repository)
                }
            }
        }
        // Only of a project trusted already; of one that is not, it is not
        // now (`Core::trust_for` reads it so).
        TrustAnswer::Keep => {
            let rules = question.rules_shown.then_some(&question.pin);
            match store.keep_without(&question.key, rules, question.git_shown) {
                Ok(_) => {
                    // The extensions and hooks listed stay unapproved, and
                    // the store keeps no record of a keep of them: they are
                    // not asked about again in this core, as after not now.
                    if !question.extensions.is_empty() || !question.hooks.is_empty() {
                        not_now();
                    }
                    self::trust(inner, place.state.as_ref(), &place.root, &place.load)
                }
                Err(failure) if gone(&failure) => gone_trust(),
                Err(failure) => {
                    not_now();
                    Ok(Trust {
                        problem: Some(format!(
                            "Your answer could not be kept, so this project will be asked about \
                             again when the harness starts next; until then what is new in it \
                             is not used: {failure}"
                        )),
                        ..trust
                    })
                }
            }
        }
        TrustAnswer::Never => match store.refuse(&question.key, &question.root) {
            // In force at once: the session asked in is put together with
            // the project refused, as a later start is, not with the trust
            // worked out before the answer (the review of the second pull
            // request).
            Ok(()) => self::trust(inner, place.state.as_ref(), &place.root, &place.load),
            Err(failure) => {
                not_now();
                Ok(Trust {
                    problem: Some(format!(
                        "Your answer could not be kept, so this project will be asked about \
                         again when the harness starts next; until then it is not trusted: \
                         {failure}"
                    )),
                    ..trust
                })
            }
        },
        TrustAnswer::NotNow => {
            not_now();
            Ok(trust)
        }
    }
}

/// The trust of a session whose yes the trust store could not keep: the
/// project trusted for this session, as `--trust-project` trusts it for a
/// run, when its rules are still the ones that were shown; when they
/// changed while the question was open, what they could loosen is set
/// aside, as it is after a yes the store kept. Git reads the repository's
/// configuration only when it was shown, or was trusted before
/// (`git_before`): a yes to changed rules alone is no yes to git declined
/// or never shown (the review of the follow-ups of step 23).
fn trusted_unkept(
    inner: &Inner,
    options: &LoadOptions,
    question: &TrustQuestion,
    failure: &cineris_policy::TrustError,
    git_before: bool,
) -> Result<Trust, Box<ErrorData>> {
    let repository = git_after_an_unkept_yes(question.git_shown, git_before);
    // The store's reason, with its file under `~`.
    let failure = match StateDir::from_dirs(&options.dirs) {
        Some(state) => crate::ext::found::with_paths_shown(
            &failure.to_string(),
            &[&state.trust_file()],
            inner.options.env.home().as_deref(),
        ),
        None => failure.to_string(),
    };
    // The extensions and hooks the question listed are trusted with the
    // rest, by the hashes shown, for this session alone.
    let what = match (question.extensions.is_empty(), question.hooks.is_empty()) {
        (true, true) => "its rules",
        (false, true) => "its rules and the extensions you were shown",
        (true, false) => "its rules and the hooks you were shown",
        (false, false) => "its rules and the extensions and hooks you were shown",
    };
    let load = |trusted: bool, profile: Option<&str>| {
        let mut options = options.clone();
        options.trust = if trusted {
            cineris_config::ProjectTrust::Trusted
        } else {
            cineris_config::ProjectTrust::Untrusted
        };
        if let Some(profile) = profile {
            options.profile = Some(profile.to_owned());
        }
        cineris_config::load(&options).map_err(|failure| {
            Box::new(ErrorData::new(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&inner.options.env),
            ))
        })
    };
    let peek = load(true, None)?;
    let rules = pinned_rules(&peek, &|profile| load(true, Some(profile)))?;
    if Definition::of_project_rules(&rules) == question.pin {
        Ok(Trust {
            rules: true,
            repository,
            loaded: peek,
            watched: None,
            for_the_run: Some(ForTheRun {
                pin: question.pin.clone(),
                git: repository,
                extensions: question.extensions.clone(),
                hooks: question.hooks.clone(),
            }),
            problem: Some(format!(
                "Your answer could not be kept, so this project, {what}, is trusted for this \
                 session only, and will be asked about again when a session opens: {failure}"
            )),
            git_note: None,
            standing: TrustStanding::ForTheRun,
            rules_declined: false,
            git_new: false,
            stored: None,
            found: None,
        })
    } else {
        Ok(Trust {
            rules: false,
            repository,
            loaded: load(false, None)?,
            watched: None,
            for_the_run: Some(ForTheRun {
                pin: question.pin.clone(),
                git: repository,
                extensions: question.extensions.clone(),
                hooks: question.hooks.clone(),
            }),
            problem: Some(format!(
                "Your answer could not be kept, so this project, {what}, is trusted for this \
                 session only, and will be asked about again when a session opens; its \
                 permission rules changed while you were asked, so its settings and its allow \
                 rules are set aside: {failure}"
            )),
            git_note: None,
            standing: TrustStanding::RulesChanged,
            rules_declined: false,
            git_new: false,
            stored: None,
            found: None,
        })
    }
}

/// Whether git reads the repository's configuration in a session whose
/// yes the store could not keep: only when git was in what the yes was
/// given to, or was trusted before. A yes to changed rules alone is no yes
/// to git declined or never shown (the early review of the follow-ups of
/// step 23: it was always yes).
fn git_after_an_unkept_yes(git_shown: bool, git_before: bool) -> bool {
    git_shown || git_before
}

/// A root as a person reads it: under the home directory with `~`, and
/// otherwise as the path's own text, never the system's canonical form
/// (`\\?\C:\…` on Windows).
fn shown_root(root: &AbsPath, home: Option<&Path>) -> String {
    let path = root.to_fs_path();
    let home = home.and_then(resolve_dir).map(|home| home.to_fs_path());
    match home
        .as_deref()
        .and_then(|home| cineris_base::strip_location_prefix(&path, home))
    {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
        None => root.to_string(),
    }
}

/// Works out whether the project is trusted, and loads its configuration
/// with that answer.
fn trust(
    inner: &Inner,
    state: Option<&StateDir>,
    root: &AbsPath,
    options: &LoadOptions,
) -> Result<Trust, Box<ErrorData>> {
    // Loaded with its files stamped as they are read, for the session's
    // reloads to look at later: no load of its own.
    let load = |trusted: bool| {
        let mut options = options.clone();
        options.trust = if trusted {
            cineris_config::ProjectTrust::Trusted
        } else {
            cineris_config::ProjectTrust::Untrusted
        };
        cineris_config::LiveConfig::start(options, Arc::clone(&inner.options.clock)).map_err(
            |failure| {
                Box::new(ErrorData::new(
                    ErrorKind::Config,
                    "invalid_configuration",
                    failure.render(&inner.options.env),
                ))
            },
        )
    };
    let made = |live: cineris_config::LiveConfig| ((*live.current()).clone(), Some(live));
    if inner.options.trust_project {
        // The user's own command line says so, for this run.
        let (loaded, watched) = made(load(true)?);
        return Ok(Trust {
            rules: true,
            repository: true,
            loaded,
            watched,
            for_the_run: None,
            problem: None,
            git_note: None,
            standing: TrustStanding::ForTheRun,
            rules_declined: false,
            git_new: false,
            stored: None,
            found: None,
        });
    }
    let (stored, problem) = match state {
        Some(state) => {
            let store = TrustStore::new(state.trust_file(), Arc::clone(&inner.options.clock));
            match store.project(&root.project_key()) {
                Ok(trust) => (trust, None),
                // A trust store that cannot be read trusts nothing.
                Err(error) => (
                    cineris_policy::ProjectTrust::untrusted(),
                    Some(format!(
                        "The trust store could not be read, so this project is treated as not \
                         trusted: {}",
                        crate::ext::found::with_paths_shown(
                            &error.to_string(),
                            &[&state.trust_file()],
                            inner.options.env.home().as_deref(),
                        )
                    )),
                ),
            }
        }
        None => (cineris_policy::ProjectTrust::untrusted(), None),
    };
    // What was read, for the trust question to be shaped by: none when the
    // store was not read whole.
    let read = (problem.is_none() && state.is_some()).then(|| stored.clone());
    if !stored.is_trusted() {
        // A project the user said never to trust is not asked about: the
        // session says so, with the command that changes it.
        let problem = problem.or_else(|| {
            stored.is_refused().then(|| {
                "You said never to trust this project, so nothing of it that needs trust is \
                 used, and it is not asked about. `cineris permissions trust` trusts it."
                    .to_owned()
            })
        });
        let standing = if stored.is_refused() {
            TrustStanding::Never
        } else {
            TrustStanding::Untrusted
        };
        let (loaded, watched) = made(load(false)?);
        return Ok(Trust {
            rules: false,
            repository: false,
            loaded,
            watched,
            for_the_run: None,
            problem,
            git_note: None,
            standing,
            rules_declined: false,
            git_new: false,
            stored: read,
            found: None,
        });
    }
    // Git reads the repository's configuration only once it was shown and
    // trusted (version 2 of the store). A trusted folder that has become a
    // repository since, or one trusted under an older store, which kept no
    // record of git, is asked about it; until then git does not read it.
    let git_new = !stored.git_trusted() && !stored.git_declined() && in_git(root);
    let git_note = git_new.then(|| {
        if stored.from_older_store() {
            "The trust store was written by an older version of Cineris, which did not \
             record whether you were shown git when you trusted this project, so git does not \
             read the repository's own configuration until you are asked again, or until \
             `cineris permissions trust` trusts it."
                .to_owned()
        } else {
            "You were not shown git when you trusted this project (it became a git repository \
             since, or was trusted by an older version, which did not record it), so git does \
             not read the repository's own configuration until you are asked, or until `cineris \
             permissions trust` trusts it."
                .to_owned()
        }
    });
    // Trusted when the user last looked. The rules the project's files hold
    // now are read as a trusted project's would be, so that none is held
    // back before it is compared with what the user saw, in every profile
    // the project defines.
    let peek = load(true)?;
    let with_profile = |profile: &str| {
        let mut options = options.clone();
        options.trust = cineris_config::ProjectTrust::Trusted;
        options.profile = Some(profile.to_owned());
        cineris_config::load(&options).map_err(|failure| {
            Box::new(ErrorData::new(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&inner.options.env),
            ))
        })
    };
    let rules = pinned_rules(&peek.current(), &with_profile)?;
    if stored.project_rules_approved(&rules) {
        let (loaded, watched) = made(peek);
        Ok(Trust {
            rules: true,
            repository: stored.git_trusted(),
            loaded,
            watched,
            for_the_run: None,
            problem,
            git_note,
            standing: TrustStanding::Trusted,
            rules_declined: false,
            git_new,
            stored: read,
            found: None,
        })
    } else {
        // The user still trusts the project; what they have not seen is
        // its rules as they are now. What the rules could loosen is set
        // aside, and what has nothing to do with them is not.
        let rules_declined = stored.rules_declined(&rules);
        let (loaded, watched) = made(load(false)?);
        Ok(Trust {
            rules: false,
            repository: stored.git_trusted(),
            loaded,
            watched,
            for_the_run: None,
            problem: Some(if rules_declined {
                RULES_DECLINED.to_owned()
            } else {
                "This project's permission rules have changed since it was trusted, so its \
                 settings and its allow rules are set aside until the change is approved."
                    .to_owned()
            }),
            git_note,
            standing: TrustStanding::RulesChanged,
            rules_declined,
            git_new,
            stored: read,
            found: None,
        })
    }
}

/// The session's settings as configuration overrides, which rank as the
/// command line does: they are the user's own.
pub(crate) fn settings_overrides(settings: &SessionSettings) -> Vec<Override> {
    let text = |value: &str| toml::Value::String(value.to_owned()).to_string();
    let mut overrides = Vec::new();
    let mut set = |key: String, value: String| {
        overrides.push(Override {
            key,
            value,
            flag: None,
        });
    };
    if let Some(level) = &settings.level {
        set("permissions.mode".to_owned(), text(level.as_str()));
    }
    if let Some(speed) = &settings.speed {
        set("model.speed".to_owned(), text(speed.as_str()));
    }
    for (role, role_settings) in &settings.roles {
        let role = role.as_str();
        if let Some(model) = &role_settings.model {
            set(format!("model.{role}"), text(model.as_str()));
        }
        if let Some(provider) = &role_settings.provider {
            set(
                format!("model.roles.{role}.provider"),
                text(provider.as_str()),
            );
        }
        if let Some(effort) = &role_settings.effort {
            set(format!("model.roles.{role}.effort"), text(effort));
        }
        if let Some(speed) = &role_settings.speed {
            set(format!("model.roles.{role}.speed"), text(speed.as_str()));
        }
    }
    overrides
}

/// The budget a set of limits is, in the lease's terms.
pub(crate) fn budget(limits: &cineris_config::schema::LimitSet) -> Budget {
    Budget {
        usd_nanos: limits.cost_usd_nanos(),
        tokens: limits.tokens,
        requests: limits.requests,
        wall_clock_ms: limits
            .time
            .map(|time| u64::try_from(time.as_millis()).unwrap_or(u64::MAX)),
    }
}

/// The parts of an engine for a session in `root`.
#[expect(
    clippy::too_many_arguments,
    reason = "each is one thing the engine is made from"
)]
fn engine_parts(
    inner: &Inner,
    loaded: &Loaded,
    state: Option<&StateDir>,
    resolver: &Arc<PathResolver>,
    session: &SessionId,
    lease_id: LeaseId,
    trusted: bool,
    repository_trusted: bool,
    shell: ShellSyntax,
    notices: &mut Vec<String>,
) -> Parts {
    let settings = &loaded.settings;
    let root = resolver.root().clone();
    let env = &inner.options.env;
    let home_dir = env.home();
    let home = home_dir.as_deref().and_then(resolve_dir);

    let mut rules = sourced_rules(loaded);
    rules.extend(
        inner
            .options
            .rules
            .iter()
            .cloned()
            .map(|rule| SourcedRule::new(rule, RuleSource::CommandLine)),
    );

    let store = state
        .and_then(|state| state.approvals_file(&root.project_key()))
        .map(|file| ApprovalStore::new(file, Arc::clone(&inner.options.clock)));
    let approved = match store.as_ref().map(ApprovalStore::load) {
        Some(Ok(approved)) => approved,
        Some(Err(error)) => {
            // An approvals file that cannot be read approves nothing.
            notices.push(format!(
                "The stored approvals for this project could not be read, so none applies: \
                 {error}"
            ));
            Vec::new()
        }
        None => Vec::new(),
    };

    let mut protected = Protected::none();
    let dirs = &loaded.dirs;
    if let Some(config) = dirs.config.as_deref().and_then(resolve_dir) {
        protected = protected.closed(config);
    }
    // The stored keys, wherever they are kept: in the configuration
    // directory on Linux, and on Windows in a folder of their own beside the
    // data directory's sessions. The folder is closed whole: the file, its
    // lock, and a temporary file a write that was killed left behind.
    if let Some(folder) = dirs.credentials.as_deref().and_then(resolve_dir) {
        protected = protected.closed(folder);
    }
    // The user's configuration file, which `CINERIS_CONFIG` can put outside
    // the configuration directory: it holds the level and the rules the
    // next session starts with, and can hold a provider's header. Closed
    // as named and, when it is a link (a dotfiles repository), where it
    // leads, which is what a read or a write of it reaches.
    if let Some(file) = dirs.user_config.as_deref() {
        if let Some(named) = resolve_dir(file) {
            protected = protected.closed(named);
        }
        if let Some(target) = std::fs::canonicalize(file)
            .ok()
            .and_then(|target| AbsPath::from_os(&target).ok())
        {
            protected = protected.closed(target);
        }
    }
    if let Some(state) = state.and_then(|state| resolve_dir(state.root())) {
        protected = protected.closed(state);
    }
    if let Some(data) = loaded.data_dir().as_deref().and_then(resolve_dir) {
        let sessions = SessionsRoot::in_data_dir(&data.to_fs_path());
        protected = protected.unwritable(data);
        if let Some(sessions) = resolve_dir(sessions.path()) {
            protected = protected.other_sessions(sessions, root.project_key());
        }
    }

    // A `sandbox.write` entry that holds the harness's own files is left
    // out of the grant, and the user is told which (the third security
    // fix).
    for entry in left_out_writes(inner, loaded, &root, state) {
        notices.push(left_out_notice(inner, &entry, &root));
    }
    Parts {
        lease_id,
        level: settings.permissions.mode.clone(),
        rules,
        approved,
        sandbox_policy: sandbox_grant(inner, loaded, &root, session, state),
        sandbox: inner.sandbox.state(settings.sandbox.enabled).0,
        models: Vec::new(),
        budget: budget(&settings.limits.session),
        limits: HardLimits::default(),
        workspace: root,
        home,
        trusted,
        repository_trusted,
        protected,
        shell,
        locate: Arc::clone(resolver) as Arc<dyn cineris_policy::Locate>,
        store,
        clock: Arc::clone(&inner.options.clock),
    }
}

/// A path the user wrote in `sandbox.write` or `sandbox.protect`, as an
/// absolute path: `~` is the user's home, and a relative path is under the
/// workspace `root`. `None` when it names a home there is none of. Every
/// reader of those settings places an entry here, so that the sandbox's
/// grant, the roots a front end is told of and the roots the harness's own
/// programs are kept out of are the same places (found by the early review
/// of the harness's own processes' fixes: the last read `~/.cargo` as a
/// relative name, and so kept nothing out).
pub(crate) fn placed_entry(entry: &str, home: Option<&Path>, root: &AbsPath) -> Option<PathBuf> {
    let expanded = cineris_config::load::expand_home(entry, home)?;
    Some(if expanded.is_absolute() {
        expanded
    } else {
        root.to_fs_path().join(expanded)
    })
}

/// An entry of `sandbox.write` or `sandbox.protect` as the root the sandbox
/// is given: where it really is ([`placed_entry`], then resolved), or as it
/// is written when it cannot be placed, for the engine to refuse by name.
fn placed_root(inner: &Inner, entry: &str, root: &AbsPath) -> String {
    let home = inner.options.env.home();
    placed_entry(entry, home.as_deref(), root)
        .as_deref()
        .and_then(resolve_dir)
        .map_or_else(|| entry.to_owned(), |resolved| resolved.as_str().to_owned())
}

/// An entry of `sandbox.write` as the user is shown it: where it really is,
/// under `~`, or as it is written when it cannot be placed.
fn shown_entry(inner: &Inner, entry: &str, root: &AbsPath) -> String {
    let home = inner.options.env.home();
    placed_entry(entry, home.as_deref(), root)
        .as_deref()
        .and_then(resolve_dir)
        .map_or_else(
            || entry.to_owned(),
            |placed| crate::ext::found::shown_path(&placed.to_fs_path(), home.as_deref()),
        )
}

/// The entries of `sandbox.write` in `loaded` that are left out of the
/// sandbox's grant: each that holds the harness's own files (its
/// configuration, state or data directory, the user's configuration file,
/// the program started as the launcher), or lies inside one, judged on its
/// own as a session's start judges its roots ([`holds_harness_files`]). A
/// confined command allowed to write there could change what it is
/// allowed, or put a program that confines nothing where the next launcher
/// is started from. Leaving the entry out keeps the rest of the grant, and
/// the sandbox enforcing; before the third security fix such an entry
/// turned the sandbox off for the whole session (decision 0042). A
/// project's entry and the user's own are left out alike. Only where the
/// sandbox would enforce: elsewhere nothing is confined, and the grant's
/// roots bind nothing. Blocks.
pub(crate) fn left_out_writes(
    inner: &Inner,
    loaded: &Loaded,
    root: &AbsPath,
    state: Option<&StateDir>,
) -> Vec<String> {
    let (sandbox, _) = inner.sandbox.state(loaded.settings.sandbox.enabled);
    if sandbox != SandboxState::Enforcing {
        return Vec::new();
    }
    let own = HarnessFilesAt::of(loaded, state, inner.sandbox.program());
    loaded
        .settings
        .sandbox
        .write
        .iter()
        .filter(|entry| own.held_by(&placed_root(inner, entry, root)))
        .cloned()
        .collect()
}

/// What the user is told of a `sandbox.write` entry that is left out
/// ([`left_out_writes`]): the entry where it really is, under `~`.
pub(crate) fn left_out_notice(inner: &Inner, entry: &str, root: &AbsPath) -> String {
    format!(
        "`sandbox.write` names `{}`, which holds Cineris's own files (its configuration, its \
         state or data, or its program): a command allowed to write there could change what it \
         is allowed. It is left out, so commands do not write there, and the sandbox stays on. \
         To let commands write in a folder there, name one that holds none of them instead.",
        cineris_base::in_code(&shown_entry(inner, entry, root))
    )
}

/// What the session's sandbox grants, from `loaded`: the workspace, the
/// session's temporary directory and `sandbox.write` to write, the
/// protected paths, and `sandbox.network`.
pub(crate) fn sandbox_grant(
    inner: &Inner,
    loaded: &Loaded,
    root: &AbsPath,
    session: &SessionId,
    state: Option<&StateDir>,
) -> cineris_protocol::lease::SandboxPolicy {
    let settings = &loaded.settings;
    let place = |entry: &String| -> String { placed_root(inner, entry, root) };
    let temporary = sandbox::temp_path(session);
    let mut writable: Vec<String> = resolve_dir(&temporary)
        .map(|path| path.as_str().to_owned())
        .into_iter()
        .collect();
    // An entry that holds the harness's own files is left out, and the rest
    // is granted (the third security fix).
    let left_out = left_out_writes(inner, loaded, root, state);
    writable.extend(
        settings
            .sandbox
            .write
            .iter()
            .filter(|entry| !left_out.contains(entry))
            .map(place),
    );
    let protect: Vec<String> = settings.sandbox.protect.iter().map(place).collect();
    let mut policy = default_sandbox(root, &writable, &protect);
    policy.network = settings.sandbox.network;
    policy
}

/// The engine's parts after a reload (step 24): the template's, with the
/// rules, the level, the sandbox's grant, the session's budget and the
/// project's trust taken from `loaded`, the configuration now in force.
/// What the template holds besides (the approvals' store, the protected
/// paths, the shell's syntax) is the session's own.
pub(crate) fn reloaded_parts(
    inner: &Inner,
    template: &Parts,
    loaded: &Loaded,
    trusted: bool,
    repository_trusted: bool,
    session: &SessionId,
    state: Option<&StateDir>,
) -> Parts {
    let mut parts = template.clone();
    let mut rules = sourced_rules(loaded);
    rules.extend(
        inner
            .options
            .rules
            .iter()
            .cloned()
            .map(|rule| SourcedRule::new(rule, RuleSource::CommandLine)),
    );
    parts.rules = rules;
    parts.level = loaded.settings.permissions.mode.clone();
    parts.sandbox_policy = sandbox_grant(inner, loaded, &template.workspace, session, state);
    parts.budget = budget(&loaded.settings.limits.session);
    parts.trusted = trusted;
    parts.repository_trusted = repository_trusted;
    parts
}

/// The project's trust now, judged as a start judges it (`trust`), and
/// nothing loaded for it put in force: what step 24's reload decides the
/// one load it makes with. `rules` is `None` when the project's files could
/// not be read to compare its rules with what was pinned: the reload then
/// keeps the trust in force, and its load reports the error.
pub(crate) struct Judged {
    pub(crate) rules: Option<bool>,
    pub(crate) repository: bool,
    pub(crate) problem: Option<String>,
    pub(crate) standing: TrustStanding,
}

/// The sentence of a trusted project whose changed rules the person kept
/// out ("keep trusting it, without the new rules").
const RULES_DECLINED: &str = "This project's permission rules have changed since it was trusted, \
                              and you chose to keep trusting it without the new rules: its \
                              settings and its allow rules are set aside. `cineris permissions \
                              trust` approves them.";

/// See [`Judged`]. Blocks: it reads the trust store and the project's
/// files.
pub(crate) fn judge(
    inner: &Inner,
    state: Option<&StateDir>,
    root: &AbsPath,
    options: &LoadOptions,
    for_the_run: Option<&ForTheRun>,
) -> Judged {
    let peek = |profile: Option<&str>| load_trusted(inner, options, profile);
    judge_pins(inner, state, root, for_the_run, || {
        peek(None).and_then(|peeked| pinned_rules(&peeked, &|profile| peek(Some(profile))))
    })
}

/// The judgement again, of the rules in `loaded`, a configuration a reload
/// loaded with the project trusted: checked on what was actually loaded,
/// after the judgement that let it load trusted, so that a file written in
/// between is never in force unchecked (the review of step 24's first
/// part). Its rules are `None` when they cannot be told. Blocks.
pub(crate) fn judge_loaded(
    inner: &Inner,
    state: Option<&StateDir>,
    root: &AbsPath,
    loaded: &Loaded,
    options: &LoadOptions,
    for_the_run: Option<&ForTheRun>,
) -> Judged {
    judge_pins(inner, state, root, for_the_run, || {
        pinned_rules(loaded, &|profile| {
            load_trusted(inner, options, Some(profile))
        })
    })
}

/// The configuration loaded with the project trusted, in `profile`.
fn load_trusted(
    inner: &Inner,
    options: &LoadOptions,
    profile: Option<&str>,
) -> Result<Loaded, Box<ErrorData>> {
    let mut options = options.clone();
    options.trust = cineris_config::ProjectTrust::Trusted;
    if let Some(profile) = profile {
        options.profile = Some(profile.to_owned());
    }
    cineris_config::load(&options).map_err(|failure| {
        Box::new(ErrorData::new(
            ErrorKind::Config,
            "invalid_configuration",
            failure.render(&inner.options.env),
        ))
    })
}

/// Judges the project's rules, `pinned` (read only when they are needed),
/// against what approves them: a yes for this session only, or the trust
/// store.
fn judge_pins(
    inner: &Inner,
    state: Option<&StateDir>,
    root: &AbsPath,
    for_the_run: Option<&ForTheRun>,
    pinned: impl FnOnce() -> Result<ProjectRules, Box<ErrorData>>,
) -> Judged {
    if inner.options.trust_project {
        return Judged {
            rules: Some(true),
            repository: true,
            problem: None,
            standing: TrustStanding::ForTheRun,
        };
    }
    if let Some(given) = for_the_run {
        // A yes the store could not keep holds for this session to the
        // rules the person saw, as a pin would, and to git as the yes gave
        // it: the store, which has no record of it, is not asked.
        let repository = given.git;
        return match pinned() {
            Ok(pins) if Definition::of_project_rules(&pins) == given.pin => Judged {
                rules: Some(true),
                repository,
                problem: None,
                standing: TrustStanding::ForTheRun,
            },
            Ok(_) => Judged {
                rules: Some(false),
                repository,
                problem: Some(
                    "This project's permission rules have changed since you trusted it for this \
                     session, so its settings and its allow rules are set aside."
                        .to_owned(),
                ),
                standing: TrustStanding::RulesChanged,
            },
            Err(_) => Judged {
                rules: None,
                repository,
                problem: None,
                standing: TrustStanding::ForTheRun,
            },
        };
    }
    let (stored, problem) = match state {
        Some(state) => {
            let store = TrustStore::new(state.trust_file(), Arc::clone(&inner.options.clock));
            match store.project(&root.project_key()) {
                Ok(trust) => (trust, None),
                Err(error) => (
                    cineris_policy::ProjectTrust::untrusted(),
                    Some(format!(
                        "The trust store could not be read, so this project is treated as not \
                         trusted: {error}"
                    )),
                ),
            }
        }
        None => (cineris_policy::ProjectTrust::untrusted(), None),
    };
    if !stored.is_trusted() {
        let standing = if stored.is_refused() {
            TrustStanding::Never
        } else {
            TrustStanding::Untrusted
        };
        return Judged {
            rules: Some(false),
            repository: false,
            problem,
            standing,
        };
    }
    // Git reads the repository's configuration only as it was shown and
    // trusted, on a reload as at the open (the follow-ups of step 23).
    let repository = stored.git_trusted();
    match pinned() {
        Ok(pins) if stored.project_rules_approved(&pins) => Judged {
            rules: Some(true),
            repository,
            problem,
            standing: TrustStanding::Trusted,
        },
        Ok(pins) => Judged {
            rules: Some(false),
            repository,
            problem: Some(if stored.rules_declined(&pins) {
                RULES_DECLINED.to_owned()
            } else {
                "This project's permission rules have changed since it was trusted, so its \
                 settings and its allow rules are set aside until the change is approved."
                    .to_owned()
            }),
            standing: TrustStanding::RulesChanged,
        },
        Err(_) => Judged {
            rules: None,
            repository,
            problem,
            standing: TrustStanding::Trusted,
        },
    }
}

/// Whether the programs an extension of this session would run are
/// confined: where the sandbox enforces, as the session's would (its
/// launcher confines them at every level there), and not where a write root
/// holds the harness's own files. What an approval says of them follows it
/// (the full review of step 28). Blocks.
pub(crate) fn programs_confined(
    inner: &Inner,
    loaded: &Loaded,
    root: &AbsPath,
    state: Option<&StateDir>,
) -> bool {
    let (sandbox, _) = inner.sandbox.state(loaded.settings.sandbox.enabled);
    sandbox == SandboxState::Enforcing
        && !holds_harness_files(
            &sandbox_grant(inner, loaded, root, &SessionId::new("s_question"), state),
            loaded,
            state,
            inner.sandbox.program(),
        )
}

/// A profile's configuration loaded with no trust and with trust, for
/// [`harness_write_warnings`].
pub(crate) type ProfileLoads<'a, E> = dyn Fn(&str) -> Result<(Loaded, Loaded), E> + 'a;

/// What the trust question says of each write root a yes would add that
/// holds the harness's own files, and so is left out of the project's
/// sessions' grant while the sandbox stays on ([`left_out_writes`]; the
/// maintainers, after the fresh review of step 28, when such a root turned
/// the sandbox off; worded as it is since the third security fix): of the
/// configuration as `loads` holds it, `(held, trusted)`, and of each
/// profile the project defines, as `with_profile` loads it the same two
/// ways, since a yes trusts every profile (the last look at the warning: a
/// profile's root was not said). A profile's root the configuration as a
/// whole holds is said once, of the whole. A profile that cannot be
/// selected (the managed file's lock) never applies.
///
/// # Errors
///
/// What loading a profile gives.
pub(crate) fn harness_write_warnings<E>(
    inner: &Inner,
    root: &AbsPath,
    state: Option<&StateDir>,
    loads: (&Loaded, &Loaded),
    with_profile: &ProfileLoads<'_, E>,
) -> Result<Vec<String>, E> {
    let (held, trusted) = loads;
    let mut said: Vec<String> = harness_write_roots(inner, held, trusted, root, state)
        .into_iter()
        .map(|(_, shown)| {
            format!(
                "This project's configuration asks to write in `{}`, which holds Cineris's own \
                 files. That entry is left out, so commands do not write there, and its sessions \
                 keep the sandbox on.",
                cineris_base::in_code(&shown)
            )
        })
        .collect();
    for name in &trusted.project_profiles {
        // The profile selected is the configuration as a whole, said above.
        if trusted.profile.as_deref() == Some(name.as_str()) {
            continue;
        }
        let (held, of) = with_profile(name)?;
        if of.profile.as_deref() != Some(name.as_str()) {
            continue;
        }
        for (entry, shown) in harness_write_roots(inner, &held, &of, root, state) {
            if trusted.settings.sandbox.write.contains(&entry) {
                continue;
            }
            said.push(format!(
                "This project's configuration asks, in its profile `{}`, to write in `{}`, which \
                 holds Cineris's own files. That entry is left out, so commands do not write there, \
                 and its sessions with that profile keep the sandbox on.",
                cineris_base::in_code(name),
                cineris_base::in_code(&shown)
            ));
        }
    }
    Ok(said)
}

/// Each write root a yes would add that holds the harness's own files, as
/// it is written and as it is shown, under `~`: one the load `trusted`
/// leaves out ([`left_out_writes`]) and the load `held`, the one in force
/// without that yes, does not name. None where the sandbox would not
/// enforce anyway: it is off, there is none, or the workspace or its
/// temporary folder, which cannot be left out, turns it off.
fn harness_write_roots(
    inner: &Inner,
    held: &Loaded,
    trusted: &Loaded,
    root: &AbsPath,
    state: Option<&StateDir>,
) -> Vec<(String, String)> {
    let (sandbox, _) = inner.sandbox.state(trusted.settings.sandbox.enabled);
    if sandbox != SandboxState::Enforcing
        || holds_harness_files(
            &sandbox_grant(inner, held, root, &SessionId::new("s_question"), state),
            held,
            state,
            inner.sandbox.program(),
        )
    {
        return Vec::new();
    }
    left_out_writes(inner, trusted, root, state)
        .into_iter()
        .filter(|entry| !held.settings.sandbox.write.contains(entry))
        .map(|entry| {
            let shown = shown_entry(inner, &entry, root);
            (entry, shown)
        })
        .collect()
}

/// Whether a write root of `policy` holds the harness's configuration,
/// state or data directory, or is one, or lies inside one; or holds the
/// user's configuration file, as named or where a link leads (a dotfiles
/// repository), which holds the level and rules the next session starts
/// with; or holds the program started as the launcher, which a confined
/// command could then replace with one that confines nothing (found by the
/// second review of step 26). A root that cannot be read as an absolute
/// path counts as holding them: what cannot be judged is not granted (on
/// Windows a root in the file system's form, `\\?\C:\…`, was passed over,
/// found by the review of step 28's process starts).
pub(crate) fn holds_harness_files(
    policy: &cineris_protocol::lease::SandboxPolicy,
    loaded: &Loaded,
    state: Option<&StateDir>,
    launcher: Option<&Path>,
) -> bool {
    let own = HarnessFilesAt::of(loaded, state, launcher);
    policy.write.all || policy.write.roots.iter().any(|root| own.held_by(root))
}

/// Where the harness's own files are, to judge a write root against
/// ([`holds_harness_files`], [`left_out_writes`]).
struct HarnessFilesAt {
    /// The configuration, state and data directories, resolved.
    dirs: Vec<AbsPath>,
    /// The launcher's program and the user's configuration file, each as
    /// it is named and as it really is: its directory resolved, and its own
    /// name.
    files: Vec<AbsPath>,
}

impl HarnessFilesAt {
    fn of(loaded: &Loaded, state: Option<&StateDir>, launcher: Option<&Path>) -> Self {
        // Resolved paths against the lease's roots, which are resolved too,
        // in one spelling; names compared as the system compares them,
        // folding case on Windows, since a match here only takes a root
        // away.
        let dirs: Vec<AbsPath> = [
            loaded.dirs.config.clone(),
            state.map(|state| state.root().to_path_buf()),
            loaded.data_dir(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|dir| resolve_dir(&dir))
        .collect();
        let files: Vec<AbsPath> = launcher
            .into_iter()
            .chain(loaded.dirs.user_config.as_deref())
            .flat_map(|program| {
                [
                    Some(program.to_path_buf()),
                    std::fs::canonicalize(program).ok(),
                ]
            })
            .flatten()
            .filter_map(|program| {
                let name = program.file_name()?.to_owned();
                resolve_dir(program.parent()?)?.child(&name).ok()
            })
            .collect();
        Self { dirs, files }
    }

    /// Whether the write root `root` holds one of the harness's files, is
    /// one of its directories, or lies inside one. A root that cannot be
    /// read as an absolute path counts as holding them: what cannot be
    /// judged is not granted.
    fn held_by(&self, root: &str) -> bool {
        match AbsPath::parse(root, cineris_base::PathStyle::NATIVE) {
            Ok(root) => {
                self.dirs
                    .iter()
                    .any(|dir| dir.starts_with(&root) || root.starts_with(dir))
                    || self.files.iter().any(|file| file.starts_with(&root))
            }
            Err(_) => true,
        }
    }
}

/// Starts an engine, with its failure as the error a front end is shown.
pub(crate) fn start_engine(parts: Parts) -> Result<Engine, Box<ErrorData>> {
    Engine::start(parts).map_err(|failure| match failure {
        // The level cannot be held here: the session does not start.
        StartError::Level(level) => Box::new(level.to_error_data()),
        other => error(ErrorKind::Config, "invalid_permissions", other.to_string()),
    })
}

/// The models of a session, by role, from its settings.
pub(crate) fn models(
    inner: &Inner,
    settings: &Settings,
    images: &Arc<dyn ImageSource>,
) -> Result<Models, Box<ErrorData>> {
    models_lenient(inner, settings, images, &[])
}

/// [`models`], where each role of `lenient` whose reasoning level the
/// model has not runs at the model's own default rather than refuse: a
/// level nobody asked that model for, carried by the files (a reload's, for
/// a model chosen in the session; `session/set` does the same).
pub(crate) fn models_lenient(
    inner: &Inner,
    settings: &Settings,
    images: &Arc<dyn ImageSource>,
    lenient: &[ModelRole],
) -> Result<Models, Box<ErrorData>> {
    let env = &inner.options.env;
    let providers = &inner.providers;
    let role = |role: &ModelRole, name: &str| {
        let entry = providers.entry(name, settings.model.provider_for(role))?;
        let make = |effort| {
            providers.role_model(
                settings,
                env,
                Arc::clone(&entry),
                effort,
                // The output limit is the main role's: a utility role's
                // requests set their own.
                (*role == ModelRole::Main)
                    .then_some(settings.model.max_output)
                    .flatten(),
                Arc::clone(images),
            )
        };
        match make(settings.model.effort_for(role)) {
            Err(refused)
                if lenient.contains(role)
                    && matches!(
                        refused.code.as_str(),
                        "unknown_effort" | "reasoning_budget_too_large"
                    ) =>
            {
                make(None)
            }
            made => made,
        }
    };
    let Some(main) = settings.model.model_for(&ModelRole::Main) else {
        return Err(error(
            ErrorKind::Config,
            "no_model",
            "No model is set. Set `model.main` in the configuration, or name one with `--model`.",
        ));
    };
    let mut models = Models::new(
        role(&ModelRole::Main, main)?,
        providers.catalog.version().to_owned(),
    );
    for utility in [ModelRole::Fast, ModelRole::Judge] {
        if let Some(name) = settings.model.model_for(&utility) {
            let model = role(&utility, name)?;
            models = models.with_role(utility, model);
        }
    }
    Ok(models)
}

/// The speeds a session starts at.
pub(crate) fn speeds(settings: &Settings) -> SpeedSettings {
    let mut speeds = SpeedSettings {
        session: settings.model.speed.clone(),
        ..SpeedSettings::default()
    };
    for (role, role_settings) in &settings.model.roles {
        if let Some(speed) = &role_settings.speed {
            speeds
                .roles
                .insert(ModelRole::from_wire(role), speed.clone());
        }
    }
    speeds
}

/// The shell a session's commands run in: the one the options give, or
/// the one found on this machine, asked its version the first time.
///
/// `avoid` is where a shell found on `PATH` is never taken from (the
/// workspace, and the write roots while confining: [`harness_avoid`]);
/// `confine_roots` is where even a configured absolute one is refused.
/// `removed` is the shell tool's own removal list, which the version's
/// probe is kept from as a command is.
fn find_shell(
    inner: &Inner,
    settings: &Settings,
    (avoid, confine_roots): (&[PathBuf], &[PathBuf]),
    removed: &[String],
    told: &mut Vec<String>,
) -> Result<Shell, cineris_tools::shell::ShellError> {
    if let Some(shell) = &inner.options.shell {
        return Ok(shell.clone());
    }
    let key = (settings.shell.program.clone(), settings.shell.args.clone());
    let known = inner
        .shells
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
        .cloned();
    // A shell cached by an earlier session is used only when its real path
    // is still outside what this session avoids. The cache is shared across
    // the process's sessions and keyed by the program and its arguments,
    // not by the workspace or the roots, so a shell another session
    // resolved must not be inherited without the check (found by the
    // independent review of the step 26 fixes). On a stale hit the search
    // runs again below.
    if let Some(shell) = known
        && cached_shell_is_usable(&shell, avoid)
    {
        return Ok(shell);
    }
    // A shell whose real path is inside the workspace, or inside a write
    // root while the session would confine, is passed over: a command or a
    // tool the model ran could have put it there, and it would then run
    // every command, and be started unconfined for its version (found by
    // the third review of step 26 for the write roots; the workspace since
    // the harness's own processes' fixes). The search goes on down `PATH`,
    // so a system shell behind a project's `.venv/bin` is still found; the
    // shell tool is unavailable only when nothing outside is left.
    // Looked for in the environment the harness's processes inherit from:
    // this process's own, or a test's.
    let search = match &inner.options.shell_env {
        Some(inherited) => ShellSearch::from_vars(inherited.iter().cloned()),
        None => ShellSearch::from_env(),
    };
    let (shell, passed) = shell_outside_roots(
        search,
        key.0.as_deref(),
        key.1.as_deref(),
        avoid,
        confine_roots,
    )?;
    if let Some(passed) = passed {
        told.push(format!(
            "The shell `{}` was passed over: it is inside the workspace or a writable root, where \
             a command could have put it; the search went on.",
            cineris_base::in_code(&passed.display().to_string())
        ));
    }
    let mut shell = shell;
    // The probe is given what a command is given: the environment commands
    // inherit from, without the keys and the removal list.
    let inherited = inner.options.shell_env.clone();
    let removed = removed.to_vec();
    let avoided = avoid.to_vec();
    // The description the model is given names the shell and its version.
    // Asking for it starts the shell, which for PowerShell is most of a
    // headless run's start-up on Windows (step 13b). So the answer is kept
    // in the state directory, under the shell's path, size and time; on a
    // miss the session goes on without the version, and the shell is asked
    // on a thread of its own for the sessions after this one.
    let asks = matches!(shell.kind(), ShellKind::Bash | ShellKind::Pwsh);
    let cache = StateDir::from_dirs(&inner.user().dirs)
        .map(|state| (state.root().join(shell_cache::FILE), state));
    let stamp = shell_cache::Stamp::of(shell.program());
    match (asks, cache, stamp) {
        (true, Some((file, state)), Some(stamp)) => match shell_cache::lookup(&file, &stamp) {
            Some(version) => shell = shell.with_version(version),
            None => {
                // Under the test hooks, a test that may not start a process
                // is stopped here, where the probe is decided, and not by a
                // race with the probe's thread.
                cineris_base::hook::starting(shell.program().as_os_str());
                let mut asked = shell.clone();
                let clock = Arc::clone(&inner.options.clock);
                let probe = std::thread::Builder::new()
                    .name("cineris-shell-version".to_owned())
                    .spawn(move || {
                        probe(&mut asked, inherited, &removed, &avoided);
                        if let Some(version) = asked.version()
                            && state.ensure().is_ok()
                        {
                            shell_cache::store(&file, &stamp, version, clock.as_ref());
                        }
                    });
                match probe {
                    Ok(probe) => inner
                        .probes
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(probe),
                    Err(failure) => {
                        tracing::debug!(%failure, "the shell's version is not asked for");
                    }
                }
            }
        },
        // Nowhere to keep it, a shell that cannot be looked at, or one that
        // is not asked (asking it starts nothing).
        _ => {
            if asks {
                cineris_base::hook::starting(shell.program().as_os_str());
            }
            probe(&mut shell, inherited, &removed, &avoided);
        }
    }
    inner
        .shells
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, shell.clone());
    Ok(shell)
}

/// Asks `shell` its version, from `inherited` (the options' environment
/// for commands) or this process's own, kept from what `removed` names and
/// with no directory of `avoid` on its `PATH`.
fn probe(
    shell: &mut Shell,
    inherited: Option<Vec<(OsString, OsString)>>,
    removed: &[String],
    avoid: &[PathBuf],
) {
    let limit = std::time::Duration::from_secs(2);
    match inherited {
        Some(inherited) => shell.probe_version_from(limit, inherited, removed, avoid),
        None => shell.probe_version(limit, removed, avoid),
    }
}

/// The shell `search` finds outside `avoid`, and the one it would have
/// taken with nothing avoided, when that was another: to tell the user.
/// The search goes on down `PATH` past a candidate inside one of `avoid`.
/// A configured absolute shell is taken as it is named, except inside one
/// of `confine_roots`, where it is an error: nothing could be searched
/// past it.
fn shell_outside_roots(
    mut search: ShellSearch,
    program: Option<&str>,
    arguments: Option<&[String]>,
    avoid: &[PathBuf],
    confine_roots: &[PathBuf],
) -> Result<(Shell, Option<PathBuf>), cineris_tools::shell::ShellError> {
    search.avoid = avoid.to_vec();
    let found = search.find(program, arguments)?;
    if under_a_write_root(found.program(), confine_roots) {
        return Err(cineris_tools::shell::ShellError::NoneFound(
            "every shell found was inside a writable root".to_owned(),
        ));
    }
    search.avoid.clear();
    let passed = search
        .find(program, arguments)
        .ok()
        .map(|first| first.program().to_path_buf())
        .filter(|first| first != found.program());
    Ok((found, passed))
}

/// Whether `path` is reached through any of `roots` on its way
/// (`cineris_base::program::passes_through`): a program there, or a link
/// there, is one a command that can write the root could have planted or
/// can change, so the harness does not start it.
fn under_a_write_root(path: &Path, roots: &[PathBuf]) -> bool {
    cineris_base::program::passes_through(path, &cineris_base::program::resolved(roots))
}

/// Whether a shell cached by an earlier session may be reused by a session
/// that avoids `roots` ([`harness_avoid`]): only when the shell's real path
/// is outside them. The cache is shared across the process's sessions and
/// keyed by the program and its arguments, not by the workspace or the
/// roots, so a shell another session resolved must not be inherited
/// without the check (found by the independent review of the step 26
/// fixes). A configured absolute shell inside the workspace is searched
/// for again, and found as it is named.
fn cached_shell_is_usable(shell: &Shell, roots: &[PathBuf]) -> bool {
    !under_a_write_root(shell.program(), roots)
}

/// Where a program the harness starts for itself (the shell, its git) is
/// never taken from by a search of `PATH`: the workspace, which a command
/// or a tool the model ran can write wherever commands are confined or
/// not, and the write roots while they are confined. Resolved, as
/// `confine_roots` are.
fn harness_avoid(root: &AbsPath, confine_roots: &[PathBuf]) -> Vec<PathBuf> {
    #[allow(
        clippy::disallowed_methods,
        reason = "resolving the workspace's path, as the write roots are, on the blocking pool"
    )]
    let workspace = std::fs::canonicalize(root.to_fs_path()).unwrap_or_else(|_| root.to_fs_path());
    let mut avoid = vec![workspace];
    for root in confine_roots {
        if !avoid.contains(root) {
            avoid.push(root.clone());
        }
    }
    avoid
}

/// The write roots a session would confine commands to, resolved, when it
/// would confine at all; empty otherwise. A program the harness starts
/// (the shell, git) that lies inside one of these is passed over: a
/// confined command could have planted it (found by the third review of
/// step 26).
fn confine_roots(inner: &Inner, loaded: &Loaded, root: &AbsPath) -> Vec<PathBuf> {
    if inner.sandbox.state(loaded.settings.sandbox.enabled).0 != SandboxState::Enforcing {
        return Vec::new();
    }
    resolved_write_roots(
        &loaded.settings.sandbox.write,
        inner.options.env.home().as_deref(),
        root,
    )
}

/// The workspace and the `sandbox.write` entries, placed as the sandbox's
/// grant places them ([`placed_entry`]) and resolved: an entry that names a
/// home there is none of is left out, as the grant cannot give it either.
/// They only keep the harness's own programs out; nothing is granted from
/// them.
fn resolved_write_roots(entries: &[String], home: Option<&Path>, root: &AbsPath) -> Vec<PathBuf> {
    let resolve = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut roots = vec![resolve(&root.to_fs_path())];
    for entry in entries {
        let Some(placed) = placed_entry(entry, home, root) else {
            continue;
        };
        match resolve_dir(&placed) {
            Some(resolved) => roots.push(resolve(&resolved.to_fs_path())),
            None => roots.push(resolve(&placed)),
        }
    }
    roots
}

/// A command's default time limit and the longest it may ask for. The
/// default is held to the longest: a project's file may lower
/// `shell.max_timeout` below `shell.timeout`, and the tool's description
/// must not then offer the model a default it may not have.
pub(crate) fn shell_limits(settings: &Settings) -> (std::time::Duration, std::time::Duration) {
    let longest = settings.shell.max_timeout;
    (settings.shell.timeout.min(longest), longest)
}

/// The shell tool's process runner.
fn runner(inner: &Inner) -> Runner {
    if let Some(runner) = &inner.options.runner {
        return runner.clone();
    }
    let mut runner = Runner::new()
        .with_timer(Arc::clone(&inner.options.timer))
        .with_clock(Arc::clone(&inner.options.clock));
    if let Some((program, arguments)) = &inner.options.guard {
        runner = runner.with_guard(Guard::new(program.clone(), arguments.iter().cloned()));
    }
    runner
}

/// Puts a session together. Blocks.
#[expect(
    clippy::too_many_lines,
    reason = "one pass over what a session is made of, in the order it is made"
)]
pub(crate) fn assemble(
    inner: &Arc<Inner>,
    wanted: Wanted,
    cancel: CancelToken,
    looked: Option<Trust>,
) -> Result<Assembled, Box<ErrorData>> {
    let options = &inner.options;
    let env = &options.env;
    let clock = Arc::clone(&options.clock);
    let mut told: Vec<String> = Vec::new();

    // ---------------------------------- the workspace, configuration, trust
    let Place {
        resolver,
        root,
        load,
        state,
    } = place(inner, &wanted)?;
    let project = root.project_key();
    // What a reload works from: the session's own choices, apart from the
    // rest of what it was loaded with, and where its state is.
    let wanted_choices = settings_overrides(&wanted.settings);
    let state_dir = state.clone();
    // Where the repository index keeps its cache: the project's, by its key.
    let index_dir = state.as_ref().and_then(|state| state.index_dir(&project));
    // Where the project's knowledge is kept: named here, read at the first
    // turn.
    let knowledge_file = state
        .as_ref()
        .and_then(|state| state.knowledge_file(&project));
    // The trust a look found (and an answer settled), or found now.
    let Trust {
        rules: rules_trusted,
        repository: repository_trusted,
        loaded,
        watched,
        for_the_run,
        problem,
        git_note,
        standing,
        found: project_found,
        stored,
        ..
    } = match looked {
        Some(trust) => trust,
        None => trust(inner, state.as_ref(), &root, &load)?,
    };
    if wanted.no_raise {
        // The level the session would start at without what the client
        // asked for, under the same answer about trust.
        let mut without = load.clone();
        without.overrides.clone_from(&options.overrides);
        without.trust = if rules_trusted {
            cineris_config::ProjectTrust::Trusted
        } else {
            cineris_config::ProjectTrust::Untrusted
        };
        let start = cineris_config::load(&without).map_err(|failure| {
            error(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(env),
            )
        })?;
        let (asked, start) = (
            &loaded.settings.permissions.mode,
            &start.settings.permissions.mode,
        );
        if !cineris_policy::level::no_higher_than(asked, start) {
            return Err(error(
                ErrorKind::Permission,
                cineris_protocol::error::code::ROLE_NOT_ALLOWED,
                format!(
                    "Only a `user` client may start a session above the level it would start \
                     at: `{}` is above `{}`.",
                    asked.as_str(),
                    start.as_str()
                ),
            ));
        }
    }
    // A core that may not give `unrestricted` (a server not started at it)
    // refuses a session that would start there, on what the configuration
    // comes to, however the request spelled it; before anything of the
    // session is written.
    inner.may_hold(&loaded.settings.permissions.mode)?;
    told.extend(problem);
    told.extend(git_note);
    for diagnostic in &loaded.diagnostics {
        told.push(diagnostic.render(env));
    }
    if !loaded.withheld.is_empty() {
        told.push(format!(
            "This project is not trusted, so {} of its settings {} not applied: {}.",
            loaded.withheld.len(),
            if loaded.withheld.len() == 1 {
                "is"
            } else {
                "are"
            },
            loaded
                .withheld
                .iter()
                .map(|withheld| format!("`{}`", cineris_base::in_code(&withheld.key.to_string())))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let loaded = Arc::new(loaded);
    let settings = &loaded.settings;

    // ---------------------------------------------------------- the models
    let late = Arc::new(LateImages::default());
    let images: Arc<dyn ImageSource> = Arc::clone(&late) as Arc<dyn ImageSource>;
    let models = models(inner, settings, &images)?;

    // -------------------------------------------- the store and the plan
    // Every key the core has found, and every one it finds from now on,
    // is masked by value in this session's log and artifacts: the filter
    // is the core's, and the session reads it each time it writes.
    let filter = Arc::clone(&inner.providers.filter);

    let (id, title, ephemeral, resume) = match &wanted.which {
        Which::New {
            id,
            title,
            ephemeral,
            ..
        } => (id.clone(), title.clone(), *ephemeral, false),
        Which::Resume { id } => (id.clone(), None, false, true),
    };
    // Where commands start: a new session's `cwd`, below the root; a
    // resumed one's, as its log keeps it.
    let mut start = match &wanted.which {
        Which::New { cwd: Some(cwd), .. } => start_below(&root, cwd)?,
        _ => None,
    };
    // The write roots a confined command of this session could write: a
    // shell or a git found inside one is passed over, so the harness never
    // starts a program a confined command planted.
    let confine_roots = confine_roots(inner, &loaded, &root);
    // Where the shell and the harness's git found on `PATH` are never taken
    // from: the workspace, and the write roots while confining.
    let avoid = harness_avoid(&root, &confine_roots);
    // The providers' key variables, as one value: the shell tool's
    // commands, the harness's git and the shell's probe are all kept from
    // it, and a reload sets it for all three.
    let withheld = Arc::new(cineris_base::child_env::Withheld::new(
        provider::key_variables(settings),
    ));
    // The harness's own git, for the base state and a file's earlier
    // content: inside the session's sandbox, once it is made, while
    // commands are confined; never a git inside the workspace or a write
    // root; and without the keys.
    let session_git = Arc::new(SessionGit::new(
        avoid.clone(),
        Arc::clone(&withheld),
        options.shell_env.clone(),
    ));
    let git: Arc<dyn Git> = options.git.clone().unwrap_or_else(|| {
        Arc::new(SystemGit::within(
            Arc::clone(&session_git) as Arc<dyn cineris_base::git::Confinement>
        ))
    });
    let mut created_at = clock.now().to_timestamp();
    let mut parent = None;
    let mut stored_title = title.clone();
    let mut lease_count = 0u64;
    let mut restored_plan: Option<Plan> = None;
    let mut base_capture = false;

    // Where the put-back keeps what it writes over or removes: the
    // session's own directory, with its other state (the third security
    // fix). None for a session kept in memory.
    let mut put_back_into: Option<PathBuf> = None;
    let (kept, content, rounds): (Kept, Arc<dyn ContentStore>, Option<Arc<Rounds>>) = if ephemeral {
        (Kept::memory(), Arc::new(MemoryStore::new()), None)
    } else {
        let Some(data) = loaded.data_dir() else {
            return Err(error(
                ErrorKind::Config,
                "no_data_dir",
                "There is nowhere to keep sessions: no data directory could be worked out. Set \
                 `session.data_dir`, or start the session with no log.",
            ));
        };
        let Some(sessions) = SessionsRoot::in_data_dir(&data).project(&project) else {
            return Err(error(
                ErrorKind::Internal,
                "invalid_project_key",
                "The project's key cannot be a directory name.",
            ));
        };
        let session_options = SessionOptions {
            durability: match settings.session.durability {
                cineris_config::schema::SessionDurability::Turn => Durability::Turn,
                cineris_config::schema::SessionDurability::Never => Durability::Never,
                _ => Durability::Item,
            },
            clock: Arc::clone(&clock),
            redactor: Arc::new(LogRedactor(Arc::clone(&filter))),
            harness: options.harness_version.clone(),
        };
        // A fork is written now, its level having passed the gate above.
        if let Some(fork) = &wanted.fork {
            let mut kept = fork
                .log
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(disk) = kept.disk() else {
                return Err(error(
                    ErrorKind::Usage,
                    "session_not_stored",
                    "A session with no log cannot be forked.",
                ));
            };
            let (forked, _) = disk
                .session
                .fork(
                    fork.at.as_ref(),
                    id.clone(),
                    SessionOptions::new(Arc::clone(&clock), options.harness_version.clone()),
                )
                .map_err(|failure| session_error(&failure))?;
            // Its lock is let go: it is opened below, as any stored session.
            drop(forked);
        }
        let session = if resume {
            let (session, report) =
                cineris_session::Session::resume(&sessions, &id, session_options)
                    .map_err(|failure| session_error(&failure))?;
            // The lease in force is read strictly: one this version does
            // not understand is a refusal, never a lease read in part.
            session.lease_in_force()?;
            lease_count = session
                .records(&cineris_session::Kind::Lease)
                .map(|records| records.len() as u64)
                .unwrap_or(0);
            created_at = session.created().clone();
            // Kept by this harness, below the root; anything else (a log
            // edited by hand) starts at the root.
            start = session
                .header()
                .start
                .clone()
                .filter(|start| is_below(start));
            parent = session
                .header()
                .forked_from
                .as_ref()
                .map(|origin| origin.session.clone());
            stored_title = session.title().ok().flatten();
            restored_plan = session
                .read_branch()
                .ok()
                .and_then(|entries| crate::store::plan_of(&entries));
            if !report.repaired.is_empty() {
                told.push(format!(
                    "{} tool call{} of this session never finished, and {} marked as \
                     interrupted. None was run again.",
                    report.repaired.len(),
                    if report.repaired.len() == 1 { "" } else { "s" },
                    if report.repaired.len() == 1 {
                        "was"
                    } else {
                        "were"
                    },
                ));
            }
            if !report.damaged_lines.is_empty() || report.cut_tail.is_some() {
                told.push(
                    "Part of this session's log could not be read and was skipped.".to_owned(),
                );
            }
            session
        } else {
            base_capture = options.base_capture;
            cineris_session::Session::create_starting_in(
                &sessions,
                id.clone(),
                &root,
                start.clone(),
                session_options,
            )
            .map_err(|failure| session_error(&failure))?
        };
        if !session.not_private().is_empty() {
            told.push(format!(
                "The session's files are not private: other users of this machine can read {}.",
                session
                    .not_private()
                    .iter()
                    .map(|path| env.display_path(path))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        put_back_into = Some(session.log().dir().path().join("put-back"));
        let store = session.store();
        let rounds = Arc::new(Rounds::new(
            Arc::clone(&store),
            root.clone(),
            KEEP_LIMIT,
            Arc::clone(&git),
        ));
        let content: Arc<dyn ContentStore> = Arc::new(DiskContent(Arc::clone(&store)));
        (
            Kept::Disk(Disk {
                session,
                rounds: Arc::clone(&rounds),
                filter: Arc::clone(&filter),
                base: None,
            }),
            content,
            Some(rounds),
        )
    };
    let _ = late.0.set(StoredImages(Arc::clone(&content)));

    let board = Arc::new(match restored_plan {
        Some(plan) => PlanBoard::with_plan(plan),
        None => PlanBoard::new(),
    });
    let hooks = Arc::new(Hooks::new(Arc::clone(&board)));
    let sink = Arc::new(SessionSink::new(id.clone()));

    // ------------------------------------------------- the shell, if any
    // The shell tool's removal list: the keys the session starts with,
    // and what the configuration lists. The probe of the shell's version
    // is kept from it too.
    let mut env_remove = provider::key_variables(settings);
    env_remove.extend(settings.shell.env_remove.iter().cloned());
    let shell_found = match find_shell(
        inner,
        settings,
        (&avoid, &confine_roots),
        &env_remove,
        &mut told,
    ) {
        Ok(shell) => Some(shell),
        Err(failure) if settings.shell.program.is_some() => {
            return Err(error(
                ErrorKind::Config,
                "no_shell",
                format!("The configured shell cannot be used: {failure}"),
            ));
        }
        Err(failure) => {
            told.push(format!(
                "No shell was found, so the `shell` tool is not available: {failure}"
            ));
            None
        }
    };
    let shell = shell_found;
    // The shell as the system prompt names it.
    let shell_named = shell.as_ref().map(|shell| match shell.version() {
        Some(version) => format!("{} {version}", shell.name()),
        None => shell.name(),
    });
    let syntax = shell.as_ref().map_or(ShellSyntax::Unknown, |shell| {
        ShellSyntax::of_program(&shell.program().to_string_lossy())
    });

    // -------------------------------------------- the permission engine
    let lease_id = LeaseId::new(format!("l_{}", lease_count + 1));
    let mut parts = engine_parts(
        inner,
        &loaded,
        state.as_ref(),
        &resolver,
        &id,
        lease_id.clone(),
        rules_trusted,
        repository_trusted,
        syntax,
        &mut told,
    );
    // A write root that holds the harness's own files would let a confined
    // command change what it is allowed: the sandbox does not enforce there
    // (found by the review of step 26). Since the third security fix only
    // the workspace or its temporary folder can: a `sandbox.write` entry
    // that does is left out of the grant ([`left_out_writes`]).
    let (mut sandbox_state, mut enforcing_with) =
        inner.sandbox.state(loaded.settings.sandbox.enabled);
    if sandbox_state == SandboxState::Enforcing
        && holds_harness_files(
            &parts.sandbox_policy,
            &loaded,
            state.as_ref(),
            inner.sandbox.program(),
        )
    {
        sandbox_state = SandboxState::NotEnforcing(NoSandbox::HarnessFilesWritable);
        enforcing_with = None;
    }
    parts.sandbox = sandbox_state;
    let engine = start_engine(parts.clone())?;
    let template = Arc::new(std::sync::Mutex::new(EngineTemplate { parts }));
    let lease = engine.lease().clone();
    let sandbox_notice = engine.sandbox_notice();
    let level = engine.level().clone();

    // ------------------------------------------------------- the sandbox
    let state = sandbox_state;
    let temp_path = resolve_dir(&sandbox::temp_path(&id))
        .map_or_else(|| sandbox::temp_path(&id), |path| path.to_fs_path());
    let made = match sandbox::make_temp(&temp_path) {
        Ok(_) => true,
        Err(failure) => {
            told.push(format!(
                "The session's private temporary directory could not be made, so commands are \
                 not given one: {failure}"
            ));
            false
        }
    };
    let landlock = enforcing_with.as_ref().map(|(landlock, _)| *landlock);
    if let Some((_, gaps)) = &enforcing_with {
        told.extend(gaps.iter().map(|gap| format!("sandbox: {gap}")));
    }
    let sandbox = Arc::new(SessionSandbox::new(
        &inner.sandbox,
        state,
        landlock,
        &level,
        &lease.grant.sandbox,
        Temp {
            path: temp_path,
            made,
        },
        &root.to_fs_path(),
        options.protect_limits.unwrap_or_default(),
    ));
    session_git.set(Arc::clone(&sandbox));
    if let Some(dir) = put_back_into {
        sandbox.keep_copies_in(dir);
    }
    let policy = Arc::new(EnginePolicy::new(PolicyParts {
        engine,
        clients: Arc::clone(&inner.clients),
        sink: Arc::clone(&sink),
        hooks: Arc::clone(&hooks),
        template: Arc::clone(&template),
        timer: Arc::clone(&options.timer),
        timeout: options.approval_timeout,
        fail_on_denial: options.fail_on_denial,
        stops_from_questions: options.answers_may_stop,
        waits_for_a_client: options.keep_sessions,
        sandboxed: sandbox.confines(),
    }));

    // ------------------------------------------ the project's knowledge
    // Made here and read nowhere yet: the first read of its files is at
    // the start of the session's first turn, or a client's `project/list`
    // before it (the interface asks as the session opens, to show what
    // waits for review), each on the blocking pool.
    let knowledge = knowledge_file.map(|file| {
        crate::knowledge::Knowledge::new(crate::knowledge::Parts {
            project: cineris_knowledge::Project::new(file, root.to_fs_path(), Arc::clone(&clock)),
            key: project.clone(),
            shared: inner.knowledge_of(&project),
            session: id.clone(),
            policy: Arc::clone(&policy),
            hooks: Arc::clone(&hooks),
            syntax,
            filter: Arc::clone(&filter),
            tokens: u32::try_from(settings.project.profile_tokens).unwrap_or(u32::MAX),
            learn: settings.project.learn,
            rounds: rounds.clone(),
            shared_file: root.join(".cineris/profile.toml").ok(),
            on_read: options.knowledge_read.clone(),
        })
    });
    if let Some(knowledge) = &knowledge {
        // Held weakly: the knowledge holds the policy and the rounds, and
        // a callback that held it back would keep a closed session's
        // parts for as long as the process runs.
        let before = Arc::downgrade(knowledge);
        policy.on_command(Arc::new(move || {
            if let Some(knowledge) = before.upgrade() {
                knowledge.before_command();
            }
        }));
        if let Some(rounds) = &rounds {
            let changed = Arc::downgrade(knowledge);
            rounds.on_change(Arc::new(move || {
                if let Some(knowledge) = changed.upgrade() {
                    knowledge.file_changed();
                }
            }));
        }
    }

    // ------------------------------------------------------------ the tools
    let files = Arc::new(FileTable::new());
    let registry = Arc::new(ToolRegistry::new());
    let editing = match &rounds {
        Some(rounds) => Editing::new(
            Arc::clone(rounds) as Arc<dyn cineris_tools::Checkpoints>,
            Arc::clone(&clock),
        )
        .with_limit(rounds.limit()),
        // With no session on disk there is nothing to undo from: what the
        // tools changed is kept in memory, for the session's own use.
        None => Editing::new(Arc::new(MemoryCheckpoints::new()), Arc::clone(&clock))
            .with_limit(KEEP_LIMIT),
    }
    .with_sync(settings.session.durability != cineris_config::schema::SessionDurability::Never);
    // What of the shell a reload changes: the repository's trust and the
    // providers' key variables, for the next command (and for an
    // extension's program's look at what it changed).
    let shell_live = Arc::new(cineris_tools::ShellLive::sharing(
        repository_trusted,
        Arc::clone(&withheld),
    ));
    // What the session's extensions keep (`cineris.state`): their `session`
    // values in the log, given to it once the log is made below, and their
    // `project` values in the state directory. Nothing is read until an
    // extension asks.
    #[cfg(feature = "scripts")]
    let ext_state = Arc::new(crate::ext::ExtState::new(
        state_dir
            .as_ref()
            .and_then(|state| crate::ext::ProjectStore::new(state, &project)),
        Arc::clone(&clock),
        options.ext_session_budget,
    ));
    // The extensions' tools, in the registry behind `tool_search` and
    // `tool_call`: registered from what each registered when it was
    // approved, with no engine started (step 28). Those a caller hands in
    // (`CoreOptions::extensions`, a test's) here; those the session finds,
    // by a task of their own once its loop runs (`ext::found`).
    #[cfg(feature = "scripts")]
    let ext_parts = {
        let tell_hooks = Arc::clone(&hooks);
        Arc::new(crate::ext::HostParts {
            policy: Arc::clone(&policy),
            timer: Arc::clone(&options.timer),
            clock: Arc::clone(&options.clock),
            workspace: root.clone(),
            write: Arc::new(WriteTool::new(editing.clone())),
            list: Arc::new(cineris_tools::ListTool::new()),
            search: Arc::new(cineris_tools::SearchTool::new()),
            // An extension's programs: found, decided, confined, looked
            // around and run as the shell's commands are, with the same
            // runner, environment and removal list.
            exec: crate::ext::ExecParts {
                sandbox: Arc::clone(&sandbox),
                runner: runner(inner),
                shell: shell.clone(),
                inherited: options.shell_env.clone(),
                env_remove: env_remove.clone(),
                withheld: Arc::clone(&withheld),
                avoid: avoid.clone(),
                rounds: rounds.clone(),
                harness_files: {
                    let loaded = loaded.clone();
                    let state = state_dir.clone();
                    let launcher = inner.sandbox.program().map(Path::to_path_buf);
                    let policy = template
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .parts
                        .sandbox_policy
                        .clone();
                    crate::ext::HarnessFiles(Arc::new(move |roots: &[String]| {
                        let mut policy = policy.clone();
                        policy.write = cineris_protocol::lease::PathSet {
                            all: false,
                            roots: roots.to_vec(),
                        };
                        holds_harness_files(&policy, &loaded, state.as_ref(), launcher.as_deref())
                    }))
                },
                live: Arc::clone(&shell_live),
                git: Arc::clone(&session_git),
            },
            fetcher: options.fetcher.clone().unwrap_or_else(|| {
                cineris_llm::Fetcher::new(
                    Arc::new(cineris_llm::SystemResolver),
                    Arc::clone(&options.timer),
                )
            }),
            tell: Arc::new(move |message: String| tell_hooks.tell(message)),
            state: Arc::clone(&ext_state),
            trust_project: options.trust_project,
        })
    };
    // What runs the session's hooks: its hook commands, and the hooks of the
    // extensions it loads, which are put on their events as each is.
    #[cfg(feature = "scripts")]
    let hook_commands = Arc::new(crate::ext::HookCommands::new(
        Arc::clone(&ext_parts),
        Arc::clone(&content),
        cancel.child(),
        {
            let sink = Arc::clone(&sink);
            Arc::new(move || sink.running_turn())
        },
    ));
    #[cfg(feature = "scripts")]
    let script_hooks = Arc::new(crate::ext::ScriptHooks::new(
        Arc::clone(&hook_commands),
        Arc::clone(&resolver),
    ));
    #[cfg(feature = "scripts")]
    for (_, left_out) in crate::ext::register(
        options.extensions.clone(),
        &ext_parts,
        &registry,
        &script_hooks,
    ) {
        told.push(left_out);
    }
    // The session's hook commands (step 28): the user's own, and the
    // project's that are approved as they are now, while it is trusted.
    // The store's reading the trust was worked out from decides a
    // project's; where none was kept (`--trust-project`), it is read now.
    #[cfg(feature = "scripts")]
    let command_hooks = {
        let trusted = matches!(
            standing,
            TrustStanding::ForTheRun | TrustStanding::Trusted | TrustStanding::RulesChanged
        );
        let configured = crate::ext::hooks::Configured::of(&loaded);
        // Read again only under `--trust-project`, which reads it not at
        // all: a store that could not be read when the trust was worked
        // out approves nothing of the project's here either.
        let stored = match stored {
            Some(stored) => Some(stored),
            None if options.trust_project && !configured.project.is_empty() => {
                state_dir.as_ref().and_then(|state| {
                    TrustStore::new(state.trust_file(), Arc::clone(&clock))
                        .project(&project)
                        .ok()
                })
            }
            None => None,
        };
        let in_session = crate::ext::hooks::in_session(
            configured,
            trusted,
            stored.as_ref(),
            for_the_run
                .as_ref()
                .map(|run| run.hooks.as_slice())
                .unwrap_or_default(),
        );
        told.extend(in_session.told);
        Arc::new(crate::ext::SessionHooks {
            hooks: in_session.hooks,
            refused: in_session.refused,
            commands: Arc::clone(&hook_commands),
            paths: Arc::clone(&resolver),
            starting: tokio::sync::Mutex::new(None),
            scripts: Arc::clone(&script_hooks),
        })
    };
    #[cfg(not(feature = "scripts"))]
    drop(stored);
    // Sections 6 and 7 of the system prompt: what the session's skills and
    // extensions give it, settled by the same finding, which the first
    // turn waits for, and answered from memory when the loop asks.
    #[cfg(feature = "scripts")]
    let prompt_parts = Arc::new(crate::ext::PromptParts::new(Arc::clone(&registry)));
    #[cfg(feature = "scripts")]
    let extensions = Some(crate::ext::found::Finding {
        resolver: Arc::clone(&resolver),
        config_dir: load.dirs.config.clone(),
        home: env.home(),
        state: state_dir.clone(),
        // A project's folder of extensions is looked in only while the
        // project is trusted.
        project: matches!(
            standing,
            TrustStanding::Trusted | TrustStanding::RulesChanged | TrustStanding::ForTheRun
        )
        .then(|| project.clone()),
        project_found,
        for_the_run: for_the_run
            .as_ref()
            .map(|run| run.extensions.clone())
            .unwrap_or_default(),
        trust_project: options.trust_project,
        disabled: settings
            .extensions
            .iter()
            .filter(|(_, extension)| !extension.enabled)
            .map(|(name, _)| name.clone())
            .collect(),
        registry: Arc::clone(&registry),
        parts: Arc::clone(&ext_parts),
        // The user's three roots of skills always, and the project's three
        // whether or not it is trusted: skills are text, as instruction
        // files are. `compat.skills` off leaves out `.agents/` and
        // `.claude/`.
        skills: cineris_ext::skills::Root::standard(settings.compat.skills),
        // The user's alone (a project's file may not set it), and off by
        // default: read once, for the session.
        read_skill_folders: settings.permissions.read_skill_folders,
        prompt: Arc::clone(&prompt_parts),
        scripts: Arc::clone(&script_hooks),
        // Set by the core once the session is open.
        session_start: None,
        cancel: options.cancel.child(),
    });
    // Without the script engine there is nothing to load.
    #[cfg(not(feature = "scripts"))]
    drop(project_found);
    #[cfg(not(feature = "scripts"))]
    let extensions = ();
    let mut toolbox = Toolbox::read_only(Arc::clone(&registry))
        .with(WriteTool::new(editing.clone()))
        .with(EditTool::new(editing))
        .with(PlanTool::new(Arc::clone(&board)))
        .with(RememberTool::new(match &knowledge {
            Some(knowledge) => Arc::clone(knowledge) as Arc<dyn cineris_tools::Proposals>,
            // With no state directory there is no project store: what is
            // proposed is kept for the session and no longer.
            None => Arc::new(MemoryProposals::new()),
        }));
    // The session's shell, which the project's checks run through too.
    let mut checks_shell: Option<Arc<RecordingShell>> = None;
    let repository = Arc::new(std::sync::atomic::AtomicBool::new(repository_trusted));
    if let Some(shell) = shell {
        let (timeout, max_timeout) = shell_limits(settings);
        let config = ShellConfig {
            timeout,
            max_timeout,
            env_remove,
            detect_changes: true,
            // Without trust, the look for what a command changed does not
            // ask git, which the repository's own configuration could make
            // run a program. The tool reads it from `shell_live`, given
            // below, which a reload sets again: this field is read only by
            // a shell with no `ShellLive`, and is kept the same here.
            repository_trusted,
            start: start.clone(),
        };
        let mut tool = ShellTool::new(shell, config)
            .with_runner(runner(inner))
            .with_live(Arc::clone(&shell_live));
        tool = tool
            .with_launcher(Arc::clone(&sandbox) as Arc<dyn cineris_tools::process::Launcher>)
            .with_git(Arc::clone(&session_git) as Arc<dyn cineris_base::git::Confinement>);
        if let Some(inherited) = &options.shell_env {
            tool = tool.with_inherited_env(inherited.iter().cloned());
        }
        // A check's output keeps the lines that report a problem, as a
        // known build's or test run's does (step 19).
        tool = tool.with_profile(Arc::new(crate::verify::ChecksProfile {
            inner: knowledge.as_ref().map(|knowledge| {
                Arc::clone(knowledge) as Arc<dyn cineris_tools::shell::CommandProfile>
            }),
            lines: settings.verify.commands.clone(),
        }));
        let recording = Arc::new(RecordingShell::new(
            tool,
            rounds.clone(),
            (Arc::clone(&sandbox), Arc::clone(&session_git)),
            Arc::clone(&policy),
            knowledge.clone(),
        ));
        toolbox.insert(Arc::clone(&recording) as Arc<dyn cineris_tools::Tool>);
        checks_shell = Some(recording);
    }

    // ------------------------------------------- what the log starts with
    let mut kept = kept;
    if let Some(disk) = kept.disk() {
        disk.session
            .append(
                None,
                Body::Lease(LeaseRecord {
                    event: LeaseEvent::Granted,
                    lease: lease.clone(),
                }),
            )
            .map_err(|failure| session_error(&failure))?;
        if let (false, Some(title)) = (resume, &title) {
            disk.session
                .append(
                    None,
                    Body::Meta(Meta {
                        title: Some(title.clone()),
                        labels: Vec::new(),
                        note: None,
                    }),
                )
                .map_err(|failure| session_error(&failure))?;
        }
        if base_capture {
            // Taken on a thread of its own, with the answer about the
            // repository's trust, and appended when it is ready: the first
            // prompt never waits for it.
            match BaseCapture::start_watching(
                root.clone(),
                Arc::clone(&repository),
                disk.session.store(),
                Arc::clone(&git),
                Arc::clone(&clock),
                BaseLimits::default(),
                policy.keep_gate(),
            ) {
                Ok(capture) => disk.base = Some(capture),
                Err(failure) => told.push(format!(
                    "The workspace's starting state is not being recorded: {failure}"
                )),
            }
        }
    }
    let log: SharedLog = Arc::new(std::sync::Mutex::new(kept));
    #[cfg(feature = "scripts")]
    ext_state.attach(Arc::clone(&log));

    // ------------------------------------------------ the repository index
    // Made here, touching no file: its first pass starts once the loop runs
    // (`Core::open`). What it indexes is held to what the agent could read
    // unasked, as the base state is, by a gate that a pass and a map take
    // once (`EnginePolicy::index_gate`).
    let repo = match &index_dir {
        Some(dir) if options.repo_index => {
            let index_settings = &settings.index;
            let limits = crate::repo_map::limits(
                index_settings.max_files,
                index_settings.max_file_bytes,
                index_settings.pass_time,
            );
            let gate: Arc<dyn cineris_index::Gate> = options
                .index_gate
                .clone()
                .unwrap_or_else(|| policy.index_gate());
            let index = cineris_index::Index::new(
                root.clone(),
                dir.clone(),
                limits,
                gate,
                Arc::clone(&clock),
            );
            Some(
                crate::repo_map::RepoIndex::new(
                    index,
                    usize::try_from(index_settings.map_tokens).unwrap_or(usize::MAX),
                    // The files of recent sessions come from the statistics
                    // (step 16), when they exist.
                    Arc::new(cineris_index::NoRecentFiles),
                    cancel.clone(),
                )
                .probed(options.index_started.clone()),
            )
        }
        _ => None,
    };

    // ------------------------------------------------------------ the loop
    let main = models.main().clone();
    let workspace = Workspace {
        paths: Arc::clone(&resolver),
        content: Arc::clone(&content),
        files,
        limits: OutputLimits::default(),
        outliner: repo
            .as_ref()
            .map(|repo| Arc::new(repo.clone()) as Arc<dyn cineris_tools::Outliner>),
        images: provider::image_support(&main.entry),
    };
    // Where the session's commands may write: the workspace, its private
    // temporary directory and the configuration's `sandbox.write`, for a
    // front end that keeps the programs it runs out of them (step 23).
    let mut write_roots = vec![root.to_native_string()];
    write_roots.push(sandbox::temp_path(&id).to_string_lossy().into_owned());
    let home = options.env.home();
    for entry in &settings.sandbox.write {
        if let Some(path) = placed_entry(entry, home.as_deref(), &root) {
            write_roots.push(path.to_string_lossy().into_owned());
        }
    }
    let info = SessionInfo {
        id: id.clone(),
        workspace: root.to_native_string(),
        title: stored_title,
        created_at,
        parent,
        level,
        lease_id: Some(lease_id),
        // Nothing is confined at `unrestricted`, wherever the sandbox
        // enforces.
        sandbox: if sandbox.confines() {
            cineris_protocol::session::SandboxStatus::Enforcing
        } else {
            cineris_protocol::session::SandboxStatus::Off
        },
        write_roots,
        trust: Some(standing.clone()),
    };
    let lease_account = Account::for_lease(&lease, Arc::clone(&clock));
    let mut accounts = vec![lease_account.clone()];
    let mut ledger = Ledger::new(Arc::clone(&clock))
        .with_account(lease_account)
        .with_turn_budget(budget(&settings.limits.turn));
    if let Some(day) = crate::day::day_account(inner, &settings.limits.day)? {
        accounts.push(day.clone());
        ledger = ledger.with_account(day);
    }
    // Every tool is held to a stop a confined command made.
    let names: Vec<String> = toolbox.names().map(str::to_owned).collect();
    for name in names {
        if let Some(tool) = toolbox.get(&name).cloned() {
            toolbox.insert(Arc::new(StopsWithTurn::new(tool, Arc::clone(&policy))));
        }
    }
    // What the session's context is made from. The instruction files are
    // read through the resolver and the engine's gate; which names are
    // read is the context's settings' (`context_settings`).
    let user_dir = load.dirs.config.as_deref().and_then(resolve_dir);
    let instructions: Arc<dyn cineris_agent::InstructionFiles> = Arc::new(DiskInstructions {
        resolver: Arc::clone(&resolver),
        user_dir: user_dir.clone(),
        gate: policy.keep_gate(),
    });
    let context = ContextParts {
        environment: Environment {
            os: if cfg!(windows) { "Windows" } else { "Linux" }.to_owned(),
            shell: shell_named,
            workspace: root.to_native_string(),
            start: start.clone(),
            git: in_git(&root),
        },
        append: inner.options.system_append.clone(),
        instructions,
        user_dir,
        settings: context_settings(settings),
        plan: Some(Arc::clone(&board)),
        cache_key: Some(id.to_string()),
        map: repo
            .as_ref()
            .filter(|_| settings.index.map)
            .map(|repo| cineris_agent::RepoMap {
                source: Arc::new(repo.clone()) as Arc<dyn cineris_agent::MapSource>,
                timer: Arc::clone(&options.timer),
                wait: settings.index.map_wait.unwrap_or(options.map_wait),
            }),
        profile: knowledge
            .as_ref()
            .map(|knowledge| Arc::clone(knowledge) as Arc<dyn cineris_agent::ProfileSource>),
        #[cfg(feature = "scripts")]
        extensions: Some(prompt_parts as Arc<dyn cineris_agent::ExtensionsSource>),
        // Without the script engine nothing is found: no section.
        #[cfg(not(feature = "scripts"))]
        extensions: None,
    };
    let verifier = Arc::new(crate::verify::CoreVerifier::new(crate::verify::Parts {
        settings: crate::verify::Settings::of(&settings.verify, shell_limits(settings).1),
        shell: checks_shell,
        policy: Arc::clone(&policy),
        knowledge: knowledge.clone(),
        trusted: repository_trusted,
        lowered_by_project: {
            let origin = loaded.origin(&cineris_config::KeyPath::new(["verify", "mode"]));
            matches!(
                rule_source(origin.layer, origin.defined_in),
                RuleSource::Project
            )
        },
        sandbox: Arc::clone(&sandbox),
        hooks: Arc::clone(&hooks),
        paths: Arc::clone(&resolver),
        content: Arc::clone(&workspace.content),
        files: Arc::clone(&workspace.files),
        limits: workspace.limits,
        clock: Arc::clone(&clock),
    }));
    let parts = SessionParts {
        info,
        models,
        speeds: speeds(settings),
        tools: Arc::new(toolbox),
        policy: Arc::clone(&policy) as Arc<dyn cineris_agent::Policy>,
        store: Box::new(LogStore::new(Arc::clone(&log), Arc::clone(&hooks))),
        context,
        events: Arc::clone(&sink) as Arc<dyn cineris_agent::EventSink>,
        stats: match &inner.stats {
            Some(recorder) => {
                // The writer starts off the core thread, the first time a
                // session opens; a later call does nothing.
                let starting = recorder.clone();
                tokio::task::spawn_blocking(move || {
                    if let Err(error) = starting.start() {
                        tracing::warn!(%error, "the statistics' writer did not start");
                    }
                });
                Arc::new(crate::stats::SessionStats {
                    recorder: recorder.clone(),
                    project: root.project_key(),
                    clock: Arc::clone(&clock),
                }) as Arc<dyn cineris_agent::Stats>
            }
            None => Arc::new(NoStats),
        },
        verifier: Arc::clone(&verifier) as Arc<dyn cineris_agent::Verifier>,
        clock,
        cooldowns: inner.providers.cooldowns.clone(),
        ledger,
        workspace,
        config: LoopConfig {
            verify_rounds: u32::try_from(settings.verify.rounds).unwrap_or(u32::MAX),
            ..LoopConfig::default()
        },
        cancel,
        observer: None,
        // The session's hooks, as the loop's points reach them (step 28).
        #[cfg(feature = "scripts")]
        hooks: Some(cineris_agent::LoopHooks::new(
            Arc::new(crate::ext::PointHooks::of(&command_hooks)),
            Arc::clone(&options.timer),
        )),
        #[cfg(not(feature = "scripts"))]
        hooks: None,
    };

    let mut notices: Vec<Event> = Vec::new();
    if let Some(message) = sandbox_notice {
        notices.push(Event::Notice(Notice::Sandbox(SandboxNotice {
            message,
            status: status_of(state),
        })));
    }
    notices.extend(told.into_iter().map(|message| {
        Event::Notice(Notice::Info(cineris_protocol::event::InfoNotice {
            message,
        }))
    }));

    Ok(Assembled {
        parts,
        sink,
        log,
        hooks,
        policy,
        board,
        sandbox,
        template,
        loaded: Arc::clone(&loaded),
        images,
        content,
        notices,
        lease_count: lease_count + 1,
        repo,
        knowledge,
        config: crate::reload::SessionConfig::new(
            load,
            wanted_choices,
            Arc::clone(&loaded),
            crate::reload::Trusted {
                rules: rules_trusted,
                repository: repository_trusted,
            },
            standing,
            for_the_run,
            root.clone(),
            state_dir.clone(),
        ),
        watched: crate::reload::Watched::new(watched, state_dir.as_ref(), &inner.options),
        verifier,
        accounts,
        shell_live,
        repository,
        extensions,
        #[cfg(feature = "scripts")]
        command_hooks,
    })
}

#[cfg(all(test, unix))]
#[allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "the unit test writes planted shells to a scratch directory (its cfg is not the \
              plain `test` one clippy reads for the in-test allowances)"
)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use cineris_base::Rng;
    use cineris_tools::shell::ShellSearch;

    use super::shell_outside_roots;

    /// Writes an executable file named `name` in `dir`.
    fn program(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A shell inside what a session avoids is passed over and the next on
    /// `PATH` is used, and the user is told which; when nothing else is
    /// left, there is none.
    #[test]
    fn a_shell_inside_a_write_root_is_passed_over() {
        let dir = std::env::temp_dir().join(format!(
            "cineris-core-shell-skip-{:016x}",
            cineris_base::rng::SystemRng::new().next_u64()
        ));
        let planted = dir.join("ws").join("bin");
        let clean = dir.join("usr").join("bin");
        program(&planted, "bash");
        let real = program(&clean, "bash");
        let search = |path: Vec<std::path::PathBuf>| ShellSearch {
            windows: false,
            path,
            system_root: None,
            comspec: None,
            avoid: Vec::new(),
        };
        let both = search(vec![planted.clone(), clean.clone()]);
        let roots = vec![dir.join("ws")];

        // The planted shell is passed over, the real one is used, and the
        // passed one is named. The same for a configured name.
        let (shell, passed) = shell_outside_roots(both.clone(), None, None, &roots, &[]).unwrap();
        assert_eq!(shell.program(), real);
        assert_eq!(passed, Some(planted.join("bash")));
        let (named, _) =
            shell_outside_roots(both.clone(), Some("bash"), None, &roots, &[]).unwrap();
        assert_eq!(named.program(), real);

        // With nothing avoided, the first is taken, and nothing is said.
        let (first, passed) = shell_outside_roots(both, None, None, &[], &[]).unwrap();
        assert_eq!(first.program(), planted.join("bash"));
        assert_eq!(passed, None);

        // With only the planted shell, and its root avoided, there is none.
        let only = search(vec![planted.clone()]);
        assert!(shell_outside_roots(only.clone(), None, None, &roots, &[]).is_err());

        // A configured absolute shell is taken as it is named inside the
        // workspace, which is avoided, and refused inside a write root of a
        // session that confines.
        let configured = planted.join("bash");
        let configured = configured.to_str().unwrap();
        let (taken, _) =
            shell_outside_roots(only.clone(), Some(configured), None, &roots, &[]).unwrap();
        assert_eq!(taken.program(), planted.join("bash"));
        assert!(shell_outside_roots(only.clone(), Some(configured), None, &roots, &roots).is_err());

        // A shell an earlier session cached inside what this one avoids is
        // not reused; one outside it still is.
        let planted_shell = shell_outside_roots(only, None, None, &[], &[]).unwrap().0;
        assert!(!super::cached_shell_is_usable(&planted_shell, &roots));
        assert!(super::cached_shell_is_usable(&planted_shell, &[]));
        assert!(super::cached_shell_is_usable(&shell, &roots));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A `sandbox.write` entry is kept out of as the sandbox grants it: `~`
    /// is the home, and a relative entry is under the workspace. Read as
    /// written, `~/.cargo` was a relative name that matched nothing.
    #[test]
    fn the_write_roots_kept_out_of_are_placed_as_the_grant_places_them() {
        let dir = std::env::temp_dir().join(format!(
            "cineris-core-roots-{:016x}",
            cineris_base::rng::SystemRng::new().next_u64()
        ));
        let (ws, home) = (dir.join("ws"), dir.join("home"));
        std::fs::create_dir_all(home.join(".cargo")).unwrap();
        std::fs::create_dir_all(ws.join("out")).unwrap();
        let root = cineris_base::AbsPath::from_os(&ws).unwrap();
        let real = |path: std::path::PathBuf| std::fs::canonicalize(path).unwrap();
        let roots = super::resolved_write_roots(
            &["~/.cargo".to_owned(), "out".to_owned()],
            Some(&home),
            &root,
        );
        assert_eq!(
            roots,
            vec![
                real(ws.clone()),
                real(home.join(".cargo")),
                real(ws.join("out"))
            ]
        );
        // With no home, the `~` entry cannot be placed and is left out.
        let roots = super::resolved_write_roots(&["~/.cargo".to_owned()], None, &root);
        assert_eq!(roots, vec![real(ws)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The workspace is avoided whether the session confines or not: with
    /// no sandbox a file the model wrote there would otherwise become the
    /// shell, or the harness's git (the harness's own processes' fixes).
    /// The write roots are added while it confines.
    #[test]
    fn the_workspace_is_avoided_with_or_without_a_sandbox() {
        let dir = std::env::temp_dir().join(format!(
            "cineris-core-avoid-{:016x}",
            cineris_base::rng::SystemRng::new().next_u64()
        ));
        let ws = dir.join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        let root = cineris_base::AbsPath::from_os(&ws).unwrap();
        let real = std::fs::canonicalize(&ws).unwrap();
        assert_eq!(super::harness_avoid(&root, &[]), vec![real.clone()]);
        let extra = dir.join("extra");
        assert_eq!(
            super::harness_avoid(&root, &[real.clone(), extra.clone()]),
            vec![real, extra]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod resolve_dir_tests {
    use cineris_base::{AbsPath, PathOptions, PathResolver};
    use cineris_testkit::Scratch;

    use super::resolve_dir;

    /// What `resolve_dir` gave before the seventh tooling step: a resolver
    /// tried at each level, from the path up, until one could be made.
    fn the_old_way(path: &std::path::Path) -> Option<AbsPath> {
        let mut missing: Vec<&std::ffi::OsStr> = Vec::new();
        let mut existing = path;
        loop {
            if let Ok(resolver) = PathResolver::new(existing, PathOptions::new()) {
                let mut resolved = resolver.root().clone();
                for name in missing.iter().rev() {
                    resolved = resolved.child(name).ok()?;
                }
                return Some(resolved);
            }
            missing.push(existing.file_name()?);
            existing = existing.parent()?;
        }
    }

    #[test]
    fn a_folder_missing_several_levels_deep_resolves_as_it_did() {
        let scratch = Scratch::outside("cineris-core-unit", "resolve_dir_deep");
        scratch.dir("there");
        scratch.file("there/a-file", "not a folder");
        for path in [
            scratch.path().to_path_buf(),
            scratch.join("there"),
            scratch.join("there/one"),
            scratch.join("there/one/two/three"),
            scratch.join("not/there/at/all"),
            // A file on the way is not a folder: the names below it are
            // kept as names, as before.
            scratch.join("there/a-file/below"),
        ] {
            let resolved = resolve_dir(&path);
            assert!(resolved.is_some(), "{}", path.display());
            assert_eq!(resolved, the_old_way(&path), "{}", path.display());
        }
        let deep = resolve_dir(&scratch.join("there/one/two/three")).unwrap();
        assert!(
            deep.as_str().ends_with("one/two/three") || deep.as_str().ends_with("one\\two\\three")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_on_the_way_is_followed_to_where_it_leads() {
        let scratch = Scratch::outside("cineris-core-unit", "resolve_dir_link");
        let real = scratch.dir("real");
        std::os::unix::fs::symlink(&real, scratch.join("link")).unwrap();
        let through = resolve_dir(&scratch.join("link/missing/deeper")).unwrap();
        let direct = resolve_dir(&real.join("missing/deeper")).unwrap();
        assert_eq!(through, direct, "the link is followed");
        assert_eq!(
            through,
            the_old_way(&scratch.join("link/missing/deeper")).unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn an_ancestor_spelt_in_another_case_resolves_to_its_spelling_on_disk() {
        let scratch = Scratch::outside("cineris-core-unit", "resolve_dir_case");
        scratch.dir("Folder");
        let asked = scratch.join("FOLDER").join("missing").join("deeper");
        let resolved = resolve_dir(&asked).unwrap();
        assert!(
            resolved.as_str().contains("Folder"),
            "the on-disk spelling: {resolved}"
        );
        assert_eq!(Some(resolved), the_old_way(&asked));
    }
}

#[cfg(test)]
mod shell_limit_tests {
    use std::time::Duration;

    use cineris_config::Settings;

    #[test]
    fn a_commands_default_limit_is_held_to_the_longest() {
        let mut settings = Settings::default();
        settings.shell.max_timeout = Duration::from_secs(10);
        assert_eq!(
            super::shell_limits(&settings),
            (Duration::from_secs(10), Duration::from_secs(10))
        );
        settings.shell.max_timeout = Duration::from_secs(600);
        assert_eq!(
            super::shell_limits(&settings),
            (Duration::from_secs(120), Duration::from_secs(600))
        );
    }
}

#[cfg(test)]
mod trust_and_start_tests {
    use super::{git_after_an_unkept_yes, is_below};

    /// A yes the store could not keep gives git only what the person was
    /// shown or had trusted before.
    #[test]
    fn an_unkept_yes_gives_git_only_what_was_shown_or_trusted_before() {
        assert!(!git_after_an_unkept_yes(false, false));
        assert!(git_after_an_unkept_yes(true, false));
        assert!(git_after_an_unkept_yes(false, true));
    }

    /// A start kept in a log names a directory below the root, and nothing
    /// else.
    #[test]
    fn a_kept_start_is_below_the_root_or_not_taken() {
        for good in ["src", "src/deep", "a b/c"] {
            assert!(is_below(good), "{good}");
        }
        for bad in ["", "/", "/etc", "..", "src/..", "./src", "src//x", "src/"] {
            assert!(!is_below(bad), "{bad:?}");
        }
        if cfg!(windows) {
            assert!(!is_below("C:"));
            assert!(!is_below("C:\\x"));
        }
    }
}

#[cfg(test)]
mod harness_files_tests {
    use cineris_config::{Env, LoadOptions, Platform};
    use cineris_protocol::lease::{NetworkAccess, PathSet, SandboxPolicy};

    use super::holds_harness_files;

    /// A write root that cannot be read as an absolute path counts as
    /// holding the harness's files: what cannot be judged is not granted.
    /// On Windows a root in the file system's form (`\\?\C:\…`) was such a
    /// root, and was passed over (the review of step 28's process starts).
    /// A plain root that holds none of them does not count.
    #[test]
    fn a_root_that_cannot_be_read_counts_as_holding_the_harness_files() {
        // No home and no managed file: the harness's own folders are none
        // of the machine's.
        let mut options = LoadOptions::new(Env::empty(Platform::CURRENT));
        options.dirs.managed_config = None;
        let loaded = cineris_config::load(&options).unwrap();
        let writing = |root: &str| SandboxPolicy {
            read: PathSet::default(),
            write: PathSet {
                all: false,
                roots: vec![root.to_owned()],
            },
            protect: PathSet::default(),
            network: NetworkAccess::Off,
        };
        let plain = if cfg!(windows) {
            r"C:\nowhere\of\the\harness"
        } else {
            "/nowhere/of/the/harness"
        };
        assert!(!holds_harness_files(&writing(plain), &loaded, None, None));
        for unreadable in ["relative/root", r"\\?\C:\nowhere\of\the\harness"] {
            assert!(
                holds_harness_files(&writing(unreadable), &loaded, None, None),
                "{unreadable}"
            );
        }
    }
}
