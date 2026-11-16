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
//!   does not lose its base state for it.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "assembling a session reads files, on the blocking pool"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use cortex_agent::{
    Account, ContextParts, ContextSettings, Environment, Ledger, LoopConfig, Models, NoStats,
    SessionInfo, SessionParts, SpeedSettings, Workspace,
};
use cortex_base::{AbsPath, CancelToken, PathOptions, PathResolver};
use cortex_config::{LayerKind, LoadOptions, Loaded, Override, Settings, StateDir, TrustClass};
use cortex_llm::{BoxFuture, ImageError, ImageSource};
use cortex_policy::{
    ApprovalStore, Definition, Engine, NoSandbox, Parts, ProjectRules, Protected, RuleSource,
    SandboxState, ShellSyntax, SourcedRule, StartError, TrustStore, default_sandbox,
};
use cortex_protocol::event::{Event, Notice, SandboxNotice};
use cortex_protocol::lease::{Budget, HardLimits, Rule};
use cortex_protocol::methods::{TrustResource, TrustResourceKind};
use cortex_protocol::model::ModelRole;
use cortex_protocol::session::{SessionSettings, TrustStanding};
use cortex_protocol::{ArtifactId, ErrorData, ErrorKind, LeaseId, Plan, ProjectKey, SessionId};
use cortex_session::entry::{LeaseEvent, LeaseRecord, Meta};
use cortex_session::{
    BaseCapture, BaseLimits, Body, Durability, Git, KEEP_LIMIT, SessionError, SessionOptions,
    SessionsRoot, SystemGit,
};
use cortex_tools::process::{Guard, Runner};
use cortex_tools::{
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
use cortex_tools::shell::ShellKind;

/// Whether the workspace is in a git repository: a `.git` in it or in a
/// directory above it. Looked at, not asked of git, which starts nothing.
fn in_git(root: &AbsPath) -> bool {
    root.to_fs_path()
        .ancestors()
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
fn context_settings(settings: &Settings) -> ContextSettings {
    let context = &settings.context;
    let mut names: Vec<String> = if settings.compat.instructions {
        cortex_agent::context::DEFAULT_NAMES
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    } else {
        Vec::new()
    };
    // Only plain documents: a name with a separator, or one that is not a
    // `.md` file, could make the harness read and send something that is
    // not instructions (`cortex_agent::context::is_instruction_name`).
    for name in &context.instruction_names {
        if cortex_agent::context::is_instruction_name(name) && !names.contains(name) {
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
    Box::new(cortex_base::Classified::to_error_data(error))
}

/// Which session to put together.
#[derive(Debug, Clone)]
pub(crate) enum Which {
    /// A new one, with this id.
    New {
        id: SessionId,
        title: Option<String>,
        ephemeral: bool,
    },
    /// A stored one.
    Resume { id: SessionId },
}

/// What a session is asked to be.
#[derive(Debug, Clone)]
pub(crate) struct Wanted {
    pub(crate) which: Which,
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
    pub(crate) template: EngineTemplate,
    pub(crate) loaded: Arc<Loaded>,
    pub(crate) images: Arc<dyn ImageSource>,
    /// What to tell front ends once the session is open.
    pub(crate) notices: Vec<Event>,
    pub(crate) lease_count: u64,
    /// The repository index, whose first pass starts once the loop runs.
    pub(crate) repo: Option<crate::repo_map::RepoIndex>,
    /// The project's knowledge as this session has it. `None` with no
    /// state directory to keep it in.
    pub(crate) knowledge: Option<Arc<crate::knowledge::Knowledge>>,
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
fn rule_source(layer: LayerKind, defined_in: Option<LayerKind>) -> RuleSource {
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

/// What was learned about the project's trust.
pub(crate) struct Trust {
    /// Whether the project may loosen what is asked and raise what is
    /// spent: it is trusted, and its rules are the ones the user saw.
    rules: bool,
    /// Whether git may read the repository's own configuration: the user
    /// trusts the project.
    repository: bool,
    loaded: Loaded,
    /// What went wrong reading the trust store, for the user.
    problem: Option<String>,
    /// What the user has said of the project, for a front end's status.
    standing: TrustStanding,
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
) -> Result<Vec<(Option<String>, cortex_config::Withheld)>, Box<ErrorData>> {
    let project = |kind: LayerKind| matches!(kind, LayerKind::Project | LayerKind::ProjectLocal);
    let in_profile = |withheld: &cortex_config::Withheld| {
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
    /// The project is trusted and only its rules changed: a yes approves
    /// them again.
    pub(crate) rules_only: bool,
}

/// A person's answer to the trust question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrustAnswer {
    /// Trust it: pin what was shown.
    Yes,
    /// Not now: not asked again by this core, and nothing stored.
    NotNow,
    /// Never for this project: stored, and not asked at later starts.
    Never,
}

/// What one setting held back for trust can do, said to the person asked.
fn held_back(withheld: &cortex_config::Withheld) -> String {
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

fn place(inner: &Inner, wanted: &Wanted) -> Result<Place, Box<ErrorData>> {
    let options = &inner.options;
    let env = &options.env;
    let home = env.home().as_deref().and_then(resolve_dir);
    let mut path_options = PathOptions::new();
    if let Some(home) = home {
        path_options = path_options.home(home);
    }
    let resolver = PathResolver::new(&wanted.workspace, path_options).map_err(|failure| {
        error(
            ErrorKind::Usage,
            "invalid_workspace",
            format!(
                "`{}` cannot be a workspace: {failure}",
                wanted.workspace.display()
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
    let trust = trust(inner, place.state.as_ref(), &place.root, &place.load)?;
    if trust.rules || inner.options.trust_project {
        return Ok((trust, None));
    }
    let key = place.root.project_key();
    let Some(state) = place.state.as_ref() else {
        return Ok((trust, None));
    };
    let store = TrustStore::new(state.trust_file(), Arc::clone(&inner.options.clock));
    let Ok(stored) = store.project(&key) else {
        // A store that cannot be read is said, and nothing is asked: an
        // answer could not be kept.
        return Ok((trust, None));
    };
    if stored.is_refused()
        || inner
            .not_now
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&key)
    {
        return Ok((trust, None));
    }
    let rules_only = stored.is_trusted();
    let trusted_load = |profile: Option<&str>| {
        let mut options = place.load.clone();
        options.trust = cortex_config::ProjectTrust::Trusted;
        if let Some(profile) = profile {
            options.profile = Some(profile.to_owned());
        }
        cortex_config::load(&options).map_err(|failure| {
            Box::new(ErrorData::new(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&inner.options.env),
            ))
        })
    };
    let peek = trusted_load(None)?;
    let rules = pinned_rules(&peek, &|profile| trusted_load(Some(profile)))?;
    let mut resources = Vec::new();
    if !rules_only && in_git(&place.root) {
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
    for (profile, rule) in rules.each() {
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
            changed: rules_only,
        });
    }
    let untrusted_load = |profile: &str| {
        let mut options = place.load.clone();
        options.trust = cortex_config::ProjectTrust::Untrusted;
        options.profile = Some(profile.to_owned());
        cortex_config::load(&options).map_err(|failure| {
            Box::new(ErrorData::new(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&inner.options.env),
            ))
        })
    };
    for (profile, withheld) in held_back_settings(&trust.loaded, &untrusted_load)? {
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
    if resources.is_empty() {
        return Ok((trust, None));
    }
    let question = TrustQuestion {
        root: shown_root(&place.root, inner.options.env.home().as_deref()),
        key,
        resources,
        pin: Definition::of_project_rules(&rules),
        rules_only,
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
/// ones shown; a never is taken as not now.
pub(crate) fn answer_trust(
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
    match answer {
        TrustAnswer::Yes => {
            let pin = [question.pin.clone()];
            let kept = if question.rules_only {
                store.approve(&question.key, &pin)
            } else {
                store.trust(&question.key, &question.root, &pin)
            };
            match kept {
                Ok(_) => self::trust(inner, place.state.as_ref(), &place.root, &place.load),
                // The project's trust was taken away while the question was
                // open (revoked, forgotten or refused, by another client or
                // process): a yes to its changed rules is no yes to the
                // whole project, nor past a never. Nothing is written, and
                // the session opens with the trust as it stands now (the
                // review of step 23's fixes).
                Err(cortex_policy::TrustError::NotUnderstood { .. })
                    if question.rules_only
                        && store
                            .project(&question.key)
                            .is_ok_and(|now| !now.is_trusted()) =>
                {
                    let mut now =
                        self::trust(inner, place.state.as_ref(), &place.root, &place.load)?;
                    now.problem = Some(
                        "This project's trust changed while you were asked, so your answer was \
                         not used and nothing of the project that needs trust is used in this \
                         session. A session opened again asks afresh, unless the project was \
                         marked never."
                            .to_owned(),
                    );
                    Ok(now)
                }
                Err(failure) => trusted_unkept(inner, &place.load, question, &failure),
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
/// aside, as it is after a yes the store kept.
fn trusted_unkept(
    inner: &Inner,
    options: &LoadOptions,
    question: &TrustQuestion,
    failure: &cortex_policy::TrustError,
) -> Result<Trust, Box<ErrorData>> {
    let load = |trusted: bool, profile: Option<&str>| {
        let mut options = options.clone();
        options.trust = if trusted {
            cortex_config::ProjectTrust::Trusted
        } else {
            cortex_config::ProjectTrust::Untrusted
        };
        if let Some(profile) = profile {
            options.profile = Some(profile.to_owned());
        }
        cortex_config::load(&options).map_err(|failure| {
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
            repository: true,
            loaded: peek,
            problem: Some(format!(
                "Your answer could not be kept, so this project is trusted for this session \
                 only, and will be asked about again: {failure}"
            )),
            standing: TrustStanding::ForTheRun,
        })
    } else {
        Ok(Trust {
            rules: false,
            repository: true,
            loaded: load(false, None)?,
            problem: Some(format!(
                "Your answer could not be kept, so this project is trusted for this session \
                 only; its permission rules changed while you were asked, so its settings and \
                 its allow rules are set aside: {failure}"
            )),
            standing: TrustStanding::RulesChanged,
        })
    }
}

/// A root as a person reads it: under the home directory with `~`, and
/// otherwise as the path's own text, never the system's canonical form
/// (`\\?\C:\…` on Windows).
fn shown_root(root: &AbsPath, home: Option<&Path>) -> String {
    let path = root.to_fs_path();
    let home = home.and_then(resolve_dir).map(|home| home.to_fs_path());
    match home
        .as_deref()
        .and_then(|home| cortex_base::strip_location_prefix(&path, home))
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
    let load = |trusted: bool| {
        let mut options = options.clone();
        options.trust = if trusted {
            cortex_config::ProjectTrust::Trusted
        } else {
            cortex_config::ProjectTrust::Untrusted
        };
        cortex_config::load(&options).map_err(|failure| {
            Box::new(ErrorData::new(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&inner.options.env),
            ))
        })
    };
    if inner.options.trust_project {
        // The user's own command line says so, for this run.
        return Ok(Trust {
            rules: true,
            repository: true,
            loaded: load(true)?,
            problem: None,
            standing: TrustStanding::ForTheRun,
        });
    }
    let (stored, problem) = match state {
        Some(state) => {
            let store = TrustStore::new(state.trust_file(), Arc::clone(&inner.options.clock));
            match store.project(&root.project_key()) {
                Ok(trust) => (trust, None),
                // A trust store that cannot be read trusts nothing.
                Err(error) => (
                    cortex_policy::ProjectTrust::untrusted(),
                    Some(format!(
                        "The trust store could not be read, so this project is treated as not \
                         trusted: {error}"
                    )),
                ),
            }
        }
        None => (cortex_policy::ProjectTrust::untrusted(), None),
    };
    if !stored.is_trusted() {
        // A project the user said never to trust is not asked about: the
        // session says so, with the command that changes it.
        let problem = problem.or_else(|| {
            stored.is_refused().then(|| {
                "You said never to trust this project, so nothing of it that needs trust is \
                 used, and it is not asked about. `cortex permissions trust` trusts it."
                    .to_owned()
            })
        });
        let standing = if stored.is_refused() {
            TrustStanding::Never
        } else {
            TrustStanding::Untrusted
        };
        return Ok(Trust {
            rules: false,
            repository: false,
            loaded: load(false)?,
            problem,
            standing,
        });
    }
    // Trusted when the user last looked. The rules the project's files hold
    // now are read as a trusted project's would be, so that none is held
    // back before it is compared with what the user saw, in every profile
    // the project defines.
    let peek = load(true)?;
    let with_profile = |profile: &str| {
        let mut options = options.clone();
        options.trust = cortex_config::ProjectTrust::Trusted;
        options.profile = Some(profile.to_owned());
        cortex_config::load(&options).map_err(|failure| {
            Box::new(ErrorData::new(
                ErrorKind::Config,
                "invalid_configuration",
                failure.render(&inner.options.env),
            ))
        })
    };
    if stored.project_rules_approved(&pinned_rules(&peek, &with_profile)?) {
        Ok(Trust {
            rules: true,
            repository: true,
            loaded: peek,
            problem,
            standing: TrustStanding::Trusted,
        })
    } else {
        // The user still trusts the project; what they have not seen is
        // its rules as they are now. What the rules could loosen is set
        // aside, and what has nothing to do with them is not.
        Ok(Trust {
            rules: false,
            repository: true,
            loaded: load(false)?,
            problem: Some(
                "This project's permission rules have changed since it was trusted, so its \
                 settings and its allow rules are set aside until the change is approved."
                    .to_owned(),
            ),
            standing: TrustStanding::RulesChanged,
        })
    }
}

/// The session's settings as configuration overrides, which rank as the
/// command line does: they are the user's own.
fn settings_overrides(settings: &SessionSettings) -> Vec<Override> {
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
pub(crate) fn budget(limits: &cortex_config::schema::LimitSet) -> Budget {
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

    // A path the user wrote, as where it really is. One that cannot be
    // placed is passed on as written, for the engine to refuse by name.
    let place = |entry: &String| -> String {
        let expanded = cortex_config::load::expand_home(entry, home_dir.as_deref());
        let absolute = match expanded {
            Some(path) if path.is_absolute() => Some(path),
            Some(path) => Some(root.to_fs_path().join(path)),
            None => None,
        };
        absolute
            .as_deref()
            .and_then(resolve_dir)
            .map_or_else(|| entry.clone(), |resolved| resolved.as_str().to_owned())
    };
    let temporary = sandbox::temp_path(session);
    let mut writable: Vec<String> = resolve_dir(&temporary)
        .map(|path| path.as_str().to_owned())
        .into_iter()
        .collect();
    writable.extend(settings.sandbox.write.iter().map(place));
    let protect: Vec<String> = settings.sandbox.protect.iter().map(place).collect();

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
    // The user's configuration file, which `CORTEX_CONFIG` can put outside
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

    Parts {
        lease_id,
        level: settings.permissions.mode.clone(),
        rules,
        approved,
        sandbox_policy: {
            let mut policy = default_sandbox(&root, &writable, &protect);
            policy.network = settings.sandbox.network;
            policy
        },
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
        locate: Arc::clone(resolver) as Arc<dyn cortex_policy::Locate>,
        store,
        clock: Arc::clone(&inner.options.clock),
    }
}

/// Whether a write root of `policy` holds the harness's configuration,
/// state or data directory, or is one, or lies inside one; or holds the
/// user's configuration file, as named or where a link leads (a dotfiles
/// repository), which holds the level and rules the next session starts
/// with; or holds the program started as the launcher, which a confined
/// command could then replace with one that confines nothing (found by the
/// second review of step 26).
fn holds_harness_files(
    policy: &cortex_protocol::lease::SandboxPolicy,
    loaded: &Loaded,
    state: Option<&StateDir>,
    launcher: Option<&Path>,
) -> bool {
    // Resolved paths against the lease's roots, which are resolved too, in
    // one spelling; names compared as the system compares them, folding
    // case on Windows, since a match here only takes the sandbox away.
    let own: Vec<AbsPath> = [
        loaded.dirs.config.clone(),
        state.map(|state| state.root().to_path_buf()),
        loaded.data_dir(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|dir| resolve_dir(&dir))
    .collect();
    // The launcher's program and the user's configuration file, each as
    // it is named and as it really is: its directory resolved, and its own
    // name.
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
    policy.write.all
        || policy
            .write
            .roots
            .iter()
            .filter_map(|root| AbsPath::parse(root, cortex_base::PathStyle::NATIVE).ok())
            .any(|root| {
                own.iter()
                    .any(|dir| dir.starts_with(&root) || root.starts_with(dir))
                    || files.iter().any(|file| file.starts_with(&root))
            })
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
fn models(
    inner: &Inner,
    settings: &Settings,
    images: &Arc<dyn ImageSource>,
) -> Result<Models, Box<ErrorData>> {
    let env = &inner.options.env;
    let providers = &inner.providers;
    let role = |role: &ModelRole, name: &str| {
        let entry = providers.entry(name, settings.model.provider_for(role))?;
        providers.role_model(
            settings,
            env,
            entry,
            settings.model.effort_for(role),
            // The output limit is the main role's: a utility role's
            // requests set their own.
            (*role == ModelRole::Main)
                .then_some(settings.model.max_output)
                .flatten(),
            Arc::clone(images),
        )
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
fn speeds(settings: &Settings) -> SpeedSettings {
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
fn find_shell(
    inner: &Inner,
    settings: &Settings,
    confine_roots: &[PathBuf],
    told: &mut Vec<String>,
) -> Result<Shell, cortex_tools::shell::ShellError> {
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
    // is still outside this session's write roots. The cache is shared
    // across the process's sessions and keyed by the program and its
    // arguments, not by the roots, so a shell a non-confining session (or
    // one with other roots) resolved must not be inherited by a confining
    // one without the write-root check (found by the independent review of
    // the step 26 fixes). On a stale hit the search runs again below.
    if let Some(shell) = known
        && cached_shell_is_usable(&shell, confine_roots)
    {
        return Ok(shell);
    }
    // When the session would confine commands, a shell whose real path is
    // inside a write root is passed over: a confined command could have
    // planted it there, and starting it to ask its version, or to run a
    // command the user let leave the sandbox, would run that unconfined
    // (found by the third review of step 26). The search goes on down
    // `PATH`, so a system shell behind a project's `.venv/bin` is still
    // found; the shell tool is unavailable only when nothing outside the
    // write roots is left.
    let (shell, skipped) = shell_outside_roots(
        ShellSearch::from_env(),
        key.0.as_deref(),
        key.1.as_deref(),
        confine_roots,
    )?;
    for passed in skipped {
        told.push(format!(
            "sandbox: the shell `{}` was passed over: it is inside a writable root, where a \
             confined command could have planted it; the search went on.",
            passed.display()
        ));
    }
    let mut shell = shell;
    // The description the model is given names the shell and its version.
    // Asking for it starts the shell, which for PowerShell is most of a
    // headless run's start-up on Windows (step 13b). So the answer is kept
    // in the state directory, under the shell's path, size and time; on a
    // miss the session goes on without the version, and the shell is asked
    // on a thread of its own for the sessions after this one.
    let asks = matches!(shell.kind(), ShellKind::Bash | ShellKind::Pwsh);
    let cache = StateDir::from_dirs(&inner.user.dirs)
        .map(|state| (state.root().join(shell_cache::FILE), state));
    let stamp = shell_cache::Stamp::of(shell.program());
    match (asks, cache, stamp) {
        (true, Some((file, state)), Some(stamp)) => match shell_cache::lookup(&file, &stamp) {
            Some(version) => shell = shell.with_version(version),
            None => {
                let mut asked = shell.clone();
                let clock = Arc::clone(&inner.options.clock);
                let probe = std::thread::Builder::new()
                    .name("cortex-shell-version".to_owned())
                    .spawn(move || {
                        asked.probe_version(std::time::Duration::from_secs(2));
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
        _ => shell.probe_version(std::time::Duration::from_secs(2)),
    }
    inner
        .shells
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(key, shell.clone());
    Ok(shell)
}

/// The shell found by `search` whose real path is outside every root in
/// `roots`, and the paths of those passed over for being inside one. The
/// search goes on down `PATH` past a candidate inside a root; a configured
/// absolute shell inside a root cannot be searched past, and is an error.
fn shell_outside_roots(
    mut search: ShellSearch,
    program: Option<&str>,
    arguments: Option<&[String]>,
    roots: &[PathBuf],
) -> Result<(Shell, Vec<PathBuf>), cortex_tools::shell::ShellError> {
    let mut skipped = Vec::new();
    loop {
        let candidate = search.find(program, arguments)?;
        if under_a_write_root(candidate.program(), roots).is_none() {
            return Ok((candidate, skipped));
        }
        let program_path = candidate.program().to_path_buf();
        match program_path.parent() {
            Some(dir) if search.path.iter().any(|entry| entry == dir) => {
                search.path.retain(|entry| entry != dir);
            }
            _ => {
                return Err(cortex_tools::shell::ShellError::NoneFound(
                    "every shell found was inside a writable root".to_owned(),
                ));
            }
        }
        skipped.push(program_path);
    }
}

/// The resolved write root `path` is inside, or `None` when it is under
/// none of them: a program there is one a confined command could have
/// planted, so the harness does not start it. `path` and `roots` are
/// compared by their real paths, so a link into a root is caught.
#[allow(
    clippy::disallowed_methods,
    reason = "resolving a program's path is part of choosing it, on the blocking pool"
)]
fn under_a_write_root(path: &Path, roots: &[PathBuf]) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // By place: a root that could not be resolved stays plain, and the real
    // path is extended-length on Windows.
    roots
        .iter()
        .find(|root| cortex_base::location_within(&real, root))
        .cloned()
}

/// Whether a shell cached by an earlier session may be reused by a session
/// that confines commands to `roots`: only when the shell's real path is
/// outside them. The cache is shared across the process's sessions and keyed
/// by the program and its arguments, not by the roots, so a shell a
/// non-confining session (or one with other roots) resolved must not be
/// inherited by a confining one without the write-root check (found by the
/// independent review of the step 26 fixes).
fn cached_shell_is_usable(shell: &Shell, roots: &[PathBuf]) -> bool {
    under_a_write_root(shell.program(), roots).is_none()
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
    let resolve = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut roots = vec![resolve(&root.to_fs_path())];
    for entry in &loaded.settings.sandbox.write {
        if let Some(resolved) = resolve_dir(Path::new(entry)) {
            roots.push(resolve(&resolved.to_fs_path()));
        } else {
            roots.push(resolve(Path::new(entry)));
        }
    }
    roots
}

/// A command's default time limit and the longest it may ask for. The
/// default is held to the longest: a project's file may lower
/// `shell.max_timeout` below `shell.timeout`, and the tool's description
/// must not then offer the model a default it may not have.
fn shell_limits(settings: &Settings) -> (std::time::Duration, std::time::Duration) {
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
        problem,
        standing,
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
            cortex_config::ProjectTrust::Trusted
        } else {
            cortex_config::ProjectTrust::Untrusted
        };
        let start = cortex_config::load(&without).map_err(|failure| {
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
        if !cortex_policy::level::no_higher_than(asked, start) {
            return Err(error(
                ErrorKind::Permission,
                cortex_protocol::error::code::ROLE_NOT_ALLOWED,
                format!(
                    "Only a `user` client may start a session above the level it would start \
                     at: `{}` is above `{}`.",
                    asked.as_str(),
                    start.as_str()
                ),
            ));
        }
    }
    told.extend(problem);
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
                .map(|withheld| format!("`{}`", withheld.key))
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
        } => (id.clone(), title.clone(), *ephemeral, false),
        Which::Resume { id } => (id.clone(), None, false, true),
    };
    // The write roots a confined command of this session could write: a
    // shell or a git found inside one is passed over, so the harness never
    // starts a program a confined command planted.
    let confine_roots = confine_roots(inner, &loaded, &root);
    // The harness's own git, for the base state and a file's earlier
    // content: inside the session's sandbox, once it is made, while
    // commands are confined, and never a git a confined command could have
    // planted inside a write root.
    let session_git = Arc::new(SessionGit::new(confine_roots.clone()));
    let git: Arc<dyn Git> = options.git.clone().unwrap_or_else(|| {
        Arc::new(SystemGit::within(
            Arc::clone(&session_git) as Arc<dyn cortex_base::git::Confinement>
        ))
    });
    let mut created_at = clock.now().to_timestamp();
    let mut parent = None;
    let mut stored_title = title.clone();
    let mut lease_count = 0u64;
    let mut restored_plan: Option<Plan> = None;
    let mut base_capture = false;

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
                cortex_config::schema::SessionDurability::Turn => Durability::Turn,
                cortex_config::schema::SessionDurability::Never => Durability::Never,
                _ => Durability::Item,
            },
            clock: Arc::clone(&clock),
            redactor: Arc::new(LogRedactor(Arc::clone(&filter))),
            harness: options.harness_version.clone(),
        };
        let session = if resume {
            let (session, report) =
                cortex_session::Session::resume(&sessions, &id, session_options)
                    .map_err(|failure| session_error(&failure))?;
            // The lease in force is read strictly: one this version does
            // not understand is a refusal, never a lease read in part.
            session.lease_in_force()?;
            lease_count = session
                .records(&cortex_session::Kind::Lease)
                .map(|records| records.len() as u64)
                .unwrap_or(0);
            created_at = session.created().clone();
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
            cortex_session::Session::create(&sessions, id.clone(), &root, session_options)
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
    let shell_found = match find_shell(inner, settings, &confine_roots, &mut told) {
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
    // (found by the review of step 26).
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
    let template = EngineTemplate { parts };
    let engine = start_engine(template.parts.clone())?;
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
    let policy = Arc::new(EnginePolicy::new(PolicyParts {
        engine,
        clients: Arc::clone(&inner.clients),
        sink: Arc::clone(&sink),
        hooks: Arc::clone(&hooks),
        timer: Arc::clone(&options.timer),
        timeout: options.approval_timeout,
        fail_on_denial: options.fail_on_denial,
        stops_from_questions: options.answers_may_stop,
        sandboxed: sandbox.confines(),
    }));

    // ------------------------------------------ the project's knowledge
    // Made here and read nowhere yet: the first read of its files is at
    // the start of the session's first turn.
    let knowledge = knowledge_file.map(|file| {
        crate::knowledge::Knowledge::new(crate::knowledge::Parts {
            project: cortex_knowledge::Project::new(file, root.to_fs_path(), Arc::clone(&clock)),
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
            shared_file: root.join(".cortex/profile.toml").ok(),
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
            Arc::clone(rounds) as Arc<dyn cortex_tools::Checkpoints>,
            Arc::clone(&clock),
        )
        .with_limit(rounds.limit()),
        // With no session on disk there is nothing to undo from: what the
        // tools changed is kept in memory, for the session's own use.
        None => Editing::new(Arc::new(MemoryCheckpoints::new()), Arc::clone(&clock))
            .with_limit(KEEP_LIMIT),
    }
    .with_sync(settings.session.durability != cortex_config::schema::SessionDurability::Never);
    let mut toolbox = Toolbox::read_only(registry)
        .with(WriteTool::new(editing.clone()))
        .with(EditTool::new(editing))
        .with(PlanTool::new(Arc::clone(&board)))
        .with(RememberTool::new(match &knowledge {
            Some(knowledge) => Arc::clone(knowledge) as Arc<dyn cortex_tools::Proposals>,
            // With no state directory there is no project store: what is
            // proposed is kept for the session and no longer.
            None => Arc::new(MemoryProposals::new()),
        }));
    // The session's shell, which the project's checks run through too.
    let mut checks_shell: Option<Arc<RecordingShell>> = None;
    if let Some(shell) = shell {
        let mut env_remove = provider::key_variables(settings);
        env_remove.extend(settings.shell.env_remove.iter().cloned());
        let (timeout, max_timeout) = shell_limits(settings);
        let config = ShellConfig {
            timeout,
            max_timeout,
            env_remove,
            detect_changes: true,
            // Without trust, the look for what a command changed does not
            // ask git, which the repository's own configuration could make
            // run a program.
            repository_trusted,
        };
        let mut tool = ShellTool::new(shell, config).with_runner(runner(inner));
        tool = tool
            .with_launcher(Arc::clone(&sandbox) as Arc<dyn cortex_tools::process::Launcher>)
            .with_git(Arc::clone(&session_git) as Arc<dyn cortex_base::git::Confinement>);
        if let Some(inherited) = &options.shell_env {
            tool = tool.with_inherited_env(inherited.iter().cloned());
        }
        // A check's output keeps the lines that report a problem, as a
        // known build's or test run's does (step 19).
        tool = tool.with_profile(Arc::new(crate::verify::ChecksProfile {
            inner: knowledge.as_ref().map(|knowledge| {
                Arc::clone(knowledge) as Arc<dyn cortex_tools::shell::CommandProfile>
            }),
            lines: settings.verify.commands.clone(),
        }));
        let recording = Arc::new(RecordingShell::new(
            tool,
            rounds.clone(),
            Arc::clone(&sandbox),
            Arc::clone(&policy),
            knowledge.clone(),
        ));
        toolbox.insert(Arc::clone(&recording) as Arc<dyn cortex_tools::Tool>);
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
            match BaseCapture::start(
                root.clone(),
                repository_trusted,
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
            let gate: Arc<dyn cortex_index::Gate> = options
                .index_gate
                .clone()
                .unwrap_or_else(|| policy.index_gate());
            let index = cortex_index::Index::new(
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
                    Arc::new(cortex_index::NoRecentFiles),
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
        content,
        files,
        limits: OutputLimits::default(),
        outliner: repo
            .as_ref()
            .map(|repo| Arc::new(repo.clone()) as Arc<dyn cortex_tools::Outliner>),
        images: provider::image_support(&main.entry),
    };
    // Where the session's commands may write: the workspace, its private
    // temporary directory and the configuration's `sandbox.write`, for a
    // front end that keeps the programs it runs out of them (step 23).
    let mut write_roots = vec![root.to_native_string()];
    write_roots.push(sandbox::temp_path(&id).to_string_lossy().into_owned());
    for entry in &settings.sandbox.write {
        let home = options.env.home();
        if let Some(path) = cortex_config::load::expand_home(entry, home.as_deref()) {
            let path = if path.is_absolute() {
                path
            } else {
                root.to_fs_path().join(path)
            };
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
            cortex_protocol::session::SandboxStatus::Enforcing
        } else {
            cortex_protocol::session::SandboxStatus::Off
        },
        write_roots,
        trust: Some(standing),
    };
    let mut ledger = Ledger::new(Arc::clone(&clock))
        .with_account(Account::for_lease(&lease, Arc::clone(&clock)))
        .with_turn_budget(budget(&settings.limits.turn));
    if let Some(day) = crate::day::day_account(inner, &settings.limits.day)? {
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
    let instructions: Arc<dyn cortex_agent::InstructionFiles> = Arc::new(DiskInstructions {
        resolver: Arc::clone(&resolver),
        user_dir: user_dir.clone(),
        gate: policy.keep_gate(),
    });
    let context = ContextParts {
        environment: Environment {
            os: if cfg!(windows) { "Windows" } else { "Linux" }.to_owned(),
            shell: shell_named,
            workspace: root.to_native_string(),
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
            .map(|repo| cortex_agent::RepoMap {
                source: Arc::new(repo.clone()) as Arc<dyn cortex_agent::MapSource>,
                timer: Arc::clone(&options.timer),
                wait: settings.index.map_wait.unwrap_or(options.map_wait),
            }),
        profile: knowledge
            .as_ref()
            .map(|knowledge| Arc::clone(knowledge) as Arc<dyn cortex_agent::ProfileSource>),
    };
    let parts = SessionParts {
        info,
        models,
        speeds: speeds(settings),
        tools: Arc::new(toolbox),
        policy: Arc::clone(&policy) as Arc<dyn cortex_agent::Policy>,
        store: Box::new(LogStore::new(Arc::clone(&log), Arc::clone(&hooks))),
        context,
        events: Arc::clone(&sink) as Arc<dyn cortex_agent::EventSink>,
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
                }) as Arc<dyn cortex_agent::Stats>
            }
            None => Arc::new(NoStats),
        },
        verifier: Arc::new(crate::verify::CoreVerifier::new(crate::verify::Parts {
            settings: crate::verify::Settings::from(&settings.verify),
            shell: checks_shell,
            policy: Arc::clone(&policy),
            knowledge: knowledge.clone(),
            trusted: repository_trusted,
            lowered_by_project: {
                let origin = loaded.origin(&cortex_config::KeyPath::new(["verify", "mode"]));
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
        })),
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
    };

    let mut notices: Vec<Event> = Vec::new();
    if let Some(message) = sandbox_notice {
        notices.push(Event::Notice(Notice::Sandbox(SandboxNotice {
            message,
            status: status_of(state),
        })));
    }
    notices.extend(told.into_iter().map(|message| {
        Event::Notice(Notice::Info(cortex_protocol::event::InfoNotice { message }))
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
        notices,
        lease_count: lease_count + 1,
        repo,
        knowledge,
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

    use cortex_base::Rng;
    use cortex_tools::shell::ShellSearch;

    use super::shell_outside_roots;

    /// Writes an executable file named `name` in `dir`.
    fn program(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A shell inside a write root is passed over and the next on `PATH` is
    /// used; when nothing else is left, there is none.
    #[test]
    fn a_shell_inside_a_write_root_is_passed_over() {
        let dir = std::env::temp_dir().join(format!(
            "cortex-core-shell-skip-{:016x}",
            cortex_base::rng::SystemRng::new().next_u64()
        ));
        let planted = dir.join("ws").join("bin");
        let clean = dir.join("usr").join("bin");
        program(&planted, "bash");
        let real = program(&clean, "bash");
        let search = ShellSearch {
            windows: false,
            path: vec![planted.clone(), clean.clone()],
            system_root: None,
            comspec: None,
        };
        let roots = vec![dir.join("ws")];

        // The planted shell is passed over; the real one is used.
        let (shell, skipped) = shell_outside_roots(search.clone(), None, None, &roots).unwrap();
        assert_eq!(shell.program(), real);
        assert_eq!(skipped, vec![planted.join("bash")]);

        // With no roots avoided, the first is taken.
        let (first, skipped) = shell_outside_roots(search, None, None, &[]).unwrap();
        assert_eq!(first.program(), planted.join("bash"));
        assert!(skipped.is_empty());

        // With only the planted shell, and its root avoided, there is none.
        let only = ShellSearch {
            windows: false,
            path: vec![planted.clone()],
            system_root: None,
            comspec: None,
        };
        assert!(shell_outside_roots(only, None, None, &roots).is_err());

        // A shell an earlier session cached inside what is now a write root
        // is not reused by a session that confines to that root; one outside
        // it still is.
        let planted_shell = shell_outside_roots(
            ShellSearch {
                windows: false,
                path: vec![planted.clone()],
                system_root: None,
                comspec: None,
            },
            None,
            None,
            &[],
        )
        .unwrap()
        .0;
        assert!(!super::cached_shell_is_usable(&planted_shell, &roots));
        assert!(super::cached_shell_is_usable(&planted_shell, &[]));
        assert!(super::cached_shell_is_usable(&shell, &roots));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod resolve_dir_tests {
    use cortex_base::{AbsPath, PathOptions, PathResolver};
    use cortex_testkit::Scratch;

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
        let scratch = Scratch::outside("cortex-core-unit", "resolve_dir_deep");
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
        let scratch = Scratch::outside("cortex-core-unit", "resolve_dir_link");
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
        let scratch = Scratch::outside("cortex-core-unit", "resolve_dir_case");
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

    use cortex_config::Settings;

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
