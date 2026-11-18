//! `config/reload` through the core (`docs/steps/24/answered.md`, "The
//! design, before building", point 1; `docs/architecture/08-configuration.md`,
//! "Reloading without a restart").
//!
//! **The configuration in force is a mix.** Each open session keeps the
//! configuration its parts were made from (in force) and the one last
//! loaded. A reload takes the `Now` keys at once, by key
//! ([`Loaded::taking`]); the `NextEpoch` keys when the session's next
//! compaction begins (or the session is opened again); the `next session`
//! and `Restart` keys never, for that session. The core's own configuration
//! (the user's, with no project) is a mix the same way, so that a reload
//! never moves the data directory or turns the statistics on or off under
//! a running core.
//!
//! **The order, for each session.** Look (the configuration's files, and
//! the trust store's file); judge the project's trust first, as a start
//! judges it, from loads that are never put in force; load once with that
//! trust (lowering trust holds even when a file is broken); build
//! everything the change needs and check it whole, refusing that session's
//! reload when anything fails; apply; tell every client of the session
//! (`config.changed`).
//!
//! **What the person chose in the session outranks the files.** The
//! level, models, reasoning levels and speeds set with `session/set` are
//! kept as overrides of the session's load ([`SessionConfig::choose`]), so
//! that no reload undoes them; `/profile` starts from the profile, and
//! drops them, which its notice says.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the blocking calls here are inside closures run on the blocking pool"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use cineris_agent::{Compaction, EpochChange, EpochContext, Reconfigure, RoleModel};
use cineris_base::{AbsPath, SharedClock, WatchList};
use cineris_config::schema::Effect;
use cineris_config::{KeyPath, LiveConfig, LoadOptions, Loaded, Override, ProjectTrust, Reload};
use cineris_protocol::ErrorKind;
use cineris_protocol::event::{ConfigChanged, Event};
use cineris_protocol::methods::{ClientRole, ConfigReloadParams, ConfigReloadResult};
use cineris_protocol::model::ModelRole;
use cineris_protocol::session::TrustStanding;

use crate::assemble;
use crate::clients::ClientId;
use crate::core::{Core, CoreOptions, Inner, Live, OpError, error};

/// Where `session/set` has put a choice in force and not yet recorded it in
/// the session's configuration: a point a test can hold it at, to show a
/// reload never comes in between (`cineris_base::hook`).
pub const CHOICE_BEING_KEPT: &str = "reload: a choice in force, not yet kept";

/// Where a reload has judged the project's trust and not yet loaded with
/// it: a point a test can hold the reload at (`cineris_base::hook`).
pub const TRUST_JUDGED: &str =
    "reload: the project's trust is judged, and the load that uses it has not begun";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The trust a session's configuration is in force under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Trusted {
    /// The project may loosen what is asked and raise what is spent.
    pub(crate) rules: bool,
    /// Git may read the repository's own configuration.
    pub(crate) repository: bool,
}

/// What a session's configuration is loaded from, and what is in force: a
/// lock held only to read or to commit, never across a file's read.
pub(crate) struct SessionConfig {
    /// How it is loaded, without the session's own choices.
    options: LoadOptions,
    /// The session's own choices (`session/set`), which outrank the files.
    choices: Vec<Override>,
    /// What the session's parts were made from.
    in_force: Arc<Loaded>,
    /// The configuration last loaded: what waits is taken from it.
    latest: Arc<Loaded>,
    /// The keys whose values in `latest` wait, and for what.
    waiting: BTreeMap<KeyPath, Effect>,
    /// The trust in force.
    trusted: Trusted,
    standing: TrustStanding,
    root: AbsPath,
    state: Option<cineris_config::StateDir>,
    /// The error last said, so that a broken file is said once, and again
    /// only when asked (`/reload`) or once it has changed.
    reported: Option<String>,
    /// The rules of a yes the store could not keep, which trust for this
    /// session only holds to.
    for_the_run: Option<crate::assemble::ForTheRun>,
    /// What waits for the next epoch as the loop was given it, kept so that
    /// a choice made in the session can take its roles out without building
    /// anything again.
    epoch: Option<EpochKept>,
    /// Counts the changes of `in_force`: a reload that finds it changed
    /// under it (the loop took what waited for an epoch) builds again.
    generation: u64,
}

/// What waits for the next epoch, as given to the loop.
#[derive(Clone)]
struct EpochKept {
    models: Vec<(ModelRole, RoleModel)>,
    context: Option<EpochContext>,
    latest: Arc<Loaded>,
    keys: Vec<KeyPath>,
    profile_tokens: u64,
    map_tokens: u64,
}

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfig")
            .field("trusted", &self.trusted)
            .field("waiting", &self.waiting)
            .finish_non_exhaustive()
    }
}

/// The files a session's configuration was read from, and the trust
/// store's, each stamped: what a reload looks at. Only a reload, which holds
/// the session's reload lock, takes this lock, across the reads it makes.
pub(crate) struct Watched {
    live: Option<LiveConfig>,
    trust_file: Option<WatchList>,
    clock: SharedClock,
}

impl Watched {
    /// The session's, made from the files its configuration was read from
    /// (`None`: the first reload reads them again) and the trust store's.
    pub(crate) fn new(
        live: Option<LiveConfig>,
        state: Option<&cineris_config::StateDir>,
        core: &CoreOptions,
    ) -> Self {
        let clock = Arc::clone(&core.clock);
        let trust_file = state.map(|state| {
            let mut watch = WatchList::new(Arc::clone(&clock));
            watch.watch(state.trust_file());
            watch
        });
        Self {
            live,
            trust_file,
            clock,
        }
    }
}

impl SessionConfig {
    /// A session's, as it was put together: `options` with the session's
    /// own `choices` last among its overrides, the configuration it was
    /// loaded as, and the trust it was judged with.
    #[expect(
        clippy::too_many_arguments,
        reason = "each is a part of how the session was put together, named where it is made"
    )]
    pub(crate) fn new(
        mut options: LoadOptions,
        choices: Vec<Override>,
        in_force: Arc<Loaded>,
        trusted: Trusted,
        standing: TrustStanding,
        for_the_run: Option<crate::assemble::ForTheRun>,
        root: AbsPath,
        state: Option<cineris_config::StateDir>,
    ) -> Self {
        // The choices were put last among the overrides (`place`): kept
        // apart, so that a profile can drop them.
        if options.overrides.ends_with(&choices) {
            let base = options.overrides.len() - choices.len();
            options.overrides.truncate(base);
        }
        Self {
            options,
            choices,
            latest: Arc::clone(&in_force),
            in_force,
            waiting: BTreeMap::new(),
            trusted,
            standing,
            root,
            state,
            reported: None,
            for_the_run,
            epoch: None,
            generation: 0,
        }
    }

    /// The configuration in force.
    pub(crate) fn in_force(&self) -> Arc<Loaded> {
        Arc::clone(&self.in_force)
    }

    /// Puts `in_force` in force, counting the change.
    fn set_in_force(&mut self, in_force: Arc<Loaded>) {
        self.in_force = in_force;
        self.generation += 1;
    }

    /// Records choices made in the session: they outrank the files from now
    /// on. Each replaces an earlier one of the same key.
    pub(crate) fn choose(&mut self, choices: Vec<Override>) {
        for choice in choices {
            self.choices.retain(|earlier| earlier.key != choice.key);
            self.choices.push(choice);
        }
    }

    /// Whether the project's rules are trusted, and the profile chosen for
    /// the session (`/profile`), when one was.
    pub(crate) fn profile_and_trust(&self) -> (bool, Option<String>) {
        (self.trusted.rules, self.options.profile.clone())
    }

    /// The keys the person chose in the session.
    fn chosen_keys(&self) -> BTreeSet<String> {
        self.choices
            .iter()
            .map(|choice| choice.key.clone())
            .collect()
    }

    /// How it loads now: the options, the choices, and `trust`.
    fn options_with(&self, trust: bool, profile: Option<&str>, keep_choices: bool) -> LoadOptions {
        let mut options = self.options.clone();
        if keep_choices {
            options.overrides.extend(self.choices.iter().cloned());
        }
        if let Some(profile) = profile {
            options.profile = Some(profile.to_owned());
        }
        options.trust = if trust {
            ProjectTrust::Trusted
        } else {
            ProjectTrust::Untrusted
        };
        options
    }
}

/// The roles a set of chosen keys sets the model, provider or reasoning
/// level of.
fn chosen_roles(chosen: &BTreeSet<String>) -> BTreeSet<String> {
    chosen
        .iter()
        .filter_map(|key| {
            let parts: Vec<&str> = key.split('.').collect();
            match parts.as_slice() {
                ["model", role] if !role.starts_with("speed") && *role != "effort" => {
                    Some((*role).to_owned())
                }
                ["model", "effort"] => Some("main".to_owned()),
                ["model", "roles", role, "provider" | "effort"] => Some((*role).to_owned()),
                _ => None,
            }
        })
        .collect()
}

/// The roles whose model the person chose in the session and whose
/// reasoning level they did not: built again, such a role falls back to
/// the model's own default where the level the files carry is not one it
/// has, as `session/set` does (the second review of step 24's first part).
fn lenient_roles(chosen: &BTreeSet<String>) -> Vec<ModelRole> {
    [ModelRole::Main, ModelRole::Fast, ModelRole::Judge]
        .into_iter()
        .filter(|role| {
            let name = role.as_str();
            let model = chosen.contains(&format!("model.{name}"));
            let effort = chosen.contains(&format!("model.roles.{name}.effort"))
                || (*role == ModelRole::Main && chosen.contains("model.effort"));
            model && !effort
        })
        .collect()
}

/// The core's own configuration: the user's, with no project. A lock held
/// only to read or to commit.
pub(crate) struct CoreConfig {
    in_force: Arc<Loaded>,
    /// The error last said, so that a broken file is said once.
    reported: Option<String>,
}

/// The files the core's own configuration was read from, stamped: locked
/// only by the core's reload, which is one at a time, across its reads.
pub(crate) struct CoreWatched {
    options: LoadOptions,
    live: Option<LiveConfig>,
    clock: SharedClock,
}

impl CoreWatched {
    pub(crate) fn new(options: LoadOptions, clock: SharedClock) -> Self {
        Self {
            options,
            live: None,
            clock,
        }
    }
}

impl CoreConfig {
    pub(crate) fn new(in_force: Loaded) -> Self {
        Self {
            in_force: Arc::new(in_force),
            reported: None,
        }
    }

    /// The configuration in force.
    pub(crate) fn in_force(&self) -> Arc<Loaded> {
        Arc::clone(&self.in_force)
    }
}

/// The keys the core's own configuration acts on: the log, the statistics'
/// pruning, and the usage reports and refresh.
const CORE_KEYS: &[&str] = &["log.level", "stats.retention", "usage"];

/// What a reload asks for.
#[derive(Debug, Clone, Default)]
pub(crate) struct Asked {
    /// Only where something changed.
    pub(crate) changed_only: bool,
    /// Another profile, for one session.
    pub(crate) profile: Option<String>,
    /// The person asked for it (`/reload`): a standing error is said again.
    pub(crate) explicit: bool,
}

/// What a reload did, over the sessions it reloaded.
#[derive(Debug, Default)]
pub(crate) struct Done {
    /// Something was new: applied, waiting since, or wrong.
    news: bool,
    applied: BTreeSet<String>,
    pending: BTreeSet<String>,
    errors: Vec<String>,
    notes: Vec<String>,
}

impl Done {
    fn add(&mut self, changed: &ConfigChanged, news: bool) {
        self.news |= news;
        self.applied.extend(changed.applied.iter().cloned());
        self.pending.extend(changed.pending.iter().cloned());
        for error in &changed.errors {
            if !self.errors.contains(error) {
                self.errors.push(error.clone());
            }
        }
        for note in &changed.notes {
            if !self.notes.contains(note) {
                self.notes.push(note.clone());
            }
        }
    }

    fn result(self) -> ConfigReloadResult {
        ConfigReloadResult {
            applied: self.applied.into_iter().collect(),
            pending: self.pending.into_iter().collect(),
            errors: self.errors,
            notes: self.notes,
        }
    }
}

/// `config/reload`.
pub(crate) async fn reload(
    core: &Core,
    client: ClientId,
    params: ConfigReloadParams,
) -> Result<ConfigReloadResult, OpError> {
    let asked = Asked {
        changed_only: params.changed_only,
        profile: params.profile.clone(),
        explicit: !params.changed_only,
    };
    let mut done = Done::default();
    let sessions: Vec<Arc<Live>> = match &params.session_id {
        Some(session) => vec![core.open_for(client, session)?],
        None => {
            if params.profile.is_some() {
                return Err(error(
                    ErrorKind::Usage,
                    "profile_needs_session",
                    "A profile is chosen for one session: name it.",
                ));
            }
            core.open_sessions_of(client)
        }
    };
    if let (Some(name), Some(live)) = (&params.profile, sessions.first()) {
        // A profile only a project not trusted defines is not one to put in
        // force: what it holds was never shown to the person.
        let (trusted, _) = lock(&live.config).profile_and_trust();
        let loaded = live.loaded();
        if !trusted && !loaded.profiles.contains(name) && loaded.project_profiles.contains(name) {
            return Err(error(
                ErrorKind::Permission,
                "untrusted_profile",
                format!(
                    "The profile `{}` is this project's, and the project is not trusted: trust \
                     it first (`cineris permissions trust`), or choose a profile of your own.",
                    cineris_base::visible(name)
                ),
            ));
        }
    }
    if params.profile.is_some() {
        // A profile can raise the level, which only a person may (as
        // `session/set` holds it).
        let role = core.inner.clients.info(client).map(|info| info.role);
        if role != Some(ClientRole::User) {
            return Err(error(
                ErrorKind::Permission,
                cineris_protocol::error::code::ROLE_NOT_ALLOWED,
                "Only a `user` client may choose a session's profile.",
            ));
        }
    }
    if params.session_id.is_none() {
        let changed = reload_core(core, &asked).await;
        let news = !(changed.applied.is_empty() && changed.errors.is_empty());
        done.add(&changed, news);
    }
    for live in sessions {
        let (changed, news) = reload_session(core, &live, &asked).await?;
        done.add(&changed, news);
    }
    if asked.explicit && !done.news {
        done.notes.insert(0, "Nothing changed.".to_owned());
    }
    Ok(done.result())
}

/// The look at the start of a turn: a reload of the session where a file,
/// or the trust store, changed.
pub(crate) async fn at_turn_start(core: &Core, live: &Arc<Live>) {
    let asked = Asked {
        changed_only: true,
        ..Asked::default()
    };
    if let Err(failure) = reload_session(core, live, &asked).await.map(|_| ()) {
        tracing::warn!(error = %failure.message, "the configuration could not be looked at");
    }
}

// ------------------------------------------------------------- the core's

async fn reload_core(core: &Core, asked: &Asked) -> ConfigChanged {
    // One at a time, from the look to the commit: an older one never
    // commits over a newer one.
    let _one = core.inner.core_reloading.lock().await;
    let inner = Arc::clone(&core.inner);
    let changed_only = asked.changed_only;
    let looked = Core::blocking(move || {
        // The files' own lock: `config/get` and the usage reports, which
        // read what is in force, never wait for a disk.
        let mut config = lock(&inner.user_watched);
        let changed = match config.live.as_mut() {
            Some(live) => live.has_changed(),
            None => true,
        };
        if changed_only && !changed {
            return Ok(None);
        }
        let options = config.options.clone();
        let clock = Arc::clone(&config.clock);
        let reload = match config.live.as_mut() {
            Some(live) => live.update(|now| *now = options),
            None => match LiveConfig::start(options, clock) {
                Ok(live) => {
                    config.live = Some(live);
                    Reload::Unchanged
                }
                Err(failure) => Reload::Rejected(failure),
            },
        };
        Ok(Some((
            reload,
            config.live.as_ref().map(LiveConfig::current),
        )))
    })
    .await;
    let Ok(Some((reload, latest))) = looked else {
        return ConfigChanged::default();
    };
    if let Reload::Rejected(failure) = &reload {
        let text = failure.render(&core.inner.options.env);
        let mut config = lock(&core.inner.user_config);
        let said = config.reported.as_deref() == Some(text.as_str());
        config.reported = Some(text.clone());
        if said && !asked.explicit {
            return ConfigChanged::default();
        }
        return ConfigChanged {
            errors: vec![text],
            ..ConfigChanged::default()
        };
    }
    lock(&core.inner.user_config).reported = None;
    let Some(latest) = latest else {
        return ConfigChanged::default();
    };
    let in_force = lock(&core.inner.user_config).in_force();
    let changes = latest.changes_since(&in_force);
    let now: Vec<KeyPath> = changes
        .iter()
        .filter(|change| change.effect == Effect::Now)
        .map(|change| change.key.clone())
        .collect();
    let Ok(mix) = in_force.taking(&latest, &now) else {
        return ConfigChanged::default();
    };
    let mix = Arc::new(mix);
    if touches(&now, &["log.level"])
        && let (Some(log), Some(level)) = (&core.inner.options.log, &mix.settings.log.level)
        && let Ok(filter) = cineris_base::log::Filter::parse(level)
    {
        log.set_filter(filter);
    }
    if touches(&now, &["stats.retention"])
        && let Some(recorder) = &core.inner.stats
    {
        recorder.set_retention(mix.settings.stats.retention);
    }
    lock(&core.inner.user_config).in_force = mix;
    // The core's own configuration says only what it acts on itself; each
    // session's says the rest, and what waits, for the session.
    ConfigChanged {
        applied: now
            .iter()
            .filter(|key| touches(std::slice::from_ref(*key), CORE_KEYS))
            .map(ToString::to_string)
            .collect(),
        ..ConfigChanged::default()
    }
}

// ----------------------------------------------------------- a session's

/// What the judgement of trust found, before the load that uses it.
struct Judging {
    judged: assemble::Judged,
    trusted: Trusted,
}

/// What the load found, off the core's thread.
#[derive(Clone)]
struct Looked {
    trusted: Trusted,
    standing: TrustStanding,
    problem: Option<String>,
    /// The configuration loaded, when one was.
    latest: Option<Arc<Loaded>>,
    errors: Vec<String>,
    /// The profile this reload loads with (`/profile`), committed with the
    /// rest only once it is applied.
    profile: Option<String>,
}

/// One session's reload, start to end. Says whether anything was new:
/// applied, waiting since this reload, or wrong. Only then are the
/// session's clients told.
async fn reload_session(
    core: &Core,
    live: &Arc<Live>,
    asked: &Asked,
) -> Result<(ConfigChanged, bool), OpError> {
    // One reload of a session at a time, from its look to its commit: two
    // (another client's, the look before a turn) never interleave, and an
    // older one never commits over a newer one (the review of step 24's
    // first part).
    let _one = live.reloading.lock().await;
    let (inner, held, asked_now) = (Arc::clone(&core.inner), Arc::clone(live), asked.clone());
    let judging = Core::blocking(move || Ok(judge(&inner, &held, &asked_now))).await?;
    let Some(judging) = judging else {
        return Ok((ConfigChanged::default(), false));
    };
    // A test holds the reload here, to change the trust store between the
    // judgement and the load: the load uses the trust just judged.
    cineris_base::hook::await_point!(TRUST_JUDGED);
    let (inner, held, asked_now) = (Arc::clone(&core.inner), Arc::clone(live), asked.clone());
    let looked = Core::blocking(move || Ok(load(&inner, &held, &asked_now, judging))).await?;
    let Some(looked) = looked else {
        return Ok((ConfigChanged::default(), false));
    };
    let (waited, trusted) = {
        let config = lock(&live.config);
        let waited: BTreeSet<String> = config.waiting.keys().map(ToString::to_string).collect();
        (waited, config.trusted)
    };
    let mut looked = looked;
    let changed = loop {
        let again = looked.clone();
        match apply(core, live, looked).await {
            Ok(changed) => break changed,
            // Only the loop's taking of an epoch moves the configuration
            // while this reload holds the session's lock, and once per
            // compaction: applied again from what is in force now.
            Err(Moved) => looked = again,
        }
    };
    let waits: BTreeSet<String> = changed.pending.iter().cloned().collect();
    // Only what is new is told: what still waits was said when it began
    // to.
    let news = !changed.applied.is_empty()
        || !changed.errors.is_empty()
        || waits != waited
        || lock(&live.config).trusted != trusted;
    if news {
        live.hooks.emit(Event::ConfigChanged(changed.clone()));
    }
    Ok((changed, news))
}

/// Looks at the session's files and the trust store and judges trust, from
/// loads that are never in force. `None`: nothing changed. Blocks.
fn judge(inner: &Inner, live: &Live, asked: &Asked) -> Option<Judging> {
    let keep_choices = asked.profile.is_none();
    let (judging, state, root, in_force, for_the_run) = {
        let config = lock(&live.config);
        (
            config.options_with(true, asked.profile.as_deref(), keep_choices),
            config.state.clone(),
            config.root.clone(),
            config.trusted,
            config.for_the_run.clone(),
        )
    };
    {
        let mut watched = lock(&live.watched);
        let files = match watched.live.as_mut() {
            Some(files) => files.has_changed(),
            None => true,
        };
        let trust_file = watched
            .trust_file
            .as_mut()
            .is_some_and(|watch| !watch.check().is_empty());
        if asked.changed_only && !files && !trust_file && asked.profile.is_none() {
            return None;
        }
    }
    // Judged first: what the project may do is decided before anything of
    // it is loaded to be used.
    let judged = assemble::judge(inner, state.as_ref(), &root, &judging, for_the_run.as_ref());
    let trusted = Trusted {
        rules: judged.rules.unwrap_or(in_force.rules),
        repository: judged.repository,
    };
    Some(Judging { judged, trusted })
}

/// The sentence of a trusted project whose rules are not the ones pinned.
const RULES_CHANGED: &str = "This project's permission rules have changed since it was trusted, so \
                             its settings and its allow rules are set aside until the change is \
                             approved.";

/// Loads the session's configuration once, with the trust `judging` found;
/// a change of the trust store since is the next look's. A load trusted is
/// checked against the pins once more, on what was loaded: a file written
/// since the judgement's look-ahead is never loaded as trusted unchecked.
/// Blocks.
fn load(inner: &Inner, live: &Live, asked: &Asked, judging: Judging) -> Option<Looked> {
    let Judging {
        mut judged,
        mut trusted,
    } = judging;
    let keep_choices = asked.profile.is_none();
    let (options, untrusted, in_force, state, root, for_the_run) = {
        let config = lock(&live.config);
        (
            config.options_with(trusted.rules, asked.profile.as_deref(), keep_choices),
            config.options_with(false, asked.profile.as_deref(), keep_choices),
            config.trusted,
            config.state.clone(),
            config.root.clone(),
            config.for_the_run.clone(),
        )
    };
    let mut watched = lock(&live.watched);
    let clock = Arc::clone(&watched.clock);
    let start_or_update = |watched: &mut Watched, options: LoadOptions| match watched.live.as_mut()
    {
        Some(files) => files.update(|now| *now = options),
        None => match LiveConfig::start(options, Arc::clone(&clock)) {
            Ok(files) => {
                watched.live = Some(files);
                Reload::Unchanged
            }
            Err(failure) => Reload::Rejected(failure),
        },
    };
    let mut reload = start_or_update(&mut watched, options.clone());
    if trusted.rules
        && !inner.options.trust_project
        && !matches!(reload, Reload::Rejected(_))
        && let Some(loaded) = watched.live.as_ref().map(LiveConfig::current)
    {
        let again = assemble::judge_loaded(
            inner,
            state.as_ref(),
            &root,
            &loaded,
            &options,
            for_the_run.as_ref(),
        );
        if again.rules != Some(true) {
            trusted.rules = false;
            trusted.repository = trusted.repository && again.repository;
            if again.rules.is_some() {
                // The store no longer trusts the project, or its rules
                // changed: said as the judgement says it.
                judged.standing = again.standing;
                judged.problem = again.problem;
            } else {
                // Cannot be told: set aside, as a change would be.
                judged.standing = TrustStanding::RulesChanged;
                judged.problem = Some(RULES_CHANGED.to_owned());
            }
            reload = start_or_update(&mut watched, untrusted);
        }
    }
    let latest = watched.live.as_ref().map(LiveConfig::current);
    drop(watched);
    let env = &inner.options.env;
    let mut config = lock(&live.config);
    let (latest, errors) = match reload {
        Reload::Rejected(failure) => {
            // Nothing loaded: what is in force stays. Trust the judgement
            // lowered is lowered all the same, where nothing needs loading
            // for it: git's reading of the repository, the verifier, and the
            // engine's setting aside of the project's allow rules.
            trusted = Trusted {
                rules: in_force.rules && trusted.rules,
                repository: in_force.repository && trusted.repository,
            };
            let text = failure.render(env);
            let said = config.reported.as_deref() == Some(text.as_str());
            config.reported = Some(text.clone());
            if said && !asked.explicit && trusted == in_force {
                return None;
            }
            (None, vec![text])
        }
        Reload::TrustLowered { failure, .. } => (latest, vec![failure.render(env)]),
        Reload::Unchanged | Reload::Applied { .. } => {
            config.reported = None;
            (latest, Vec::new())
        }
    };
    Some(Looked {
        trusted,
        standing: judged.standing,
        problem: judged.problem,
        profile: latest.is_some().then(|| asked.profile.clone()).flatten(),
        latest,
        errors,
    })
}

/// Whether any of `keys` is at or under one of `prefixes`.
fn touches(keys: &[KeyPath], prefixes: &[&str]) -> bool {
    keys.iter().any(|key| {
        let text = key.to_string();
        prefixes
            .iter()
            .any(|prefix| text == *prefix || text.starts_with(&format!("{prefix}.")))
    })
}

/// Whether any of `keys` is a provider's key named `field`.
fn provider_field(keys: &[KeyPath], fields: &[&str]) -> bool {
    keys.iter().any(|key| {
        let parts = key.parts();
        parts.len() == 3 && parts[0] == "providers" && fields.contains(&parts[2].as_str())
    })
}

/// The keys whose building can fail for reasons of their own (a credential
/// missing, statistics off): what a lowering of trust leaves out, rather
/// than not lower trust at all.
const MAY_FAIL: &[&str] = &["model", "providers", "adapt", "retry", "limits.day"];

/// What waits for the next epoch, built with the rest.
struct EpochPrepared {
    models: Vec<(ModelRole, RoleModel)>,
    context: Option<EpochContext>,
    /// The configuration the keys are taken from when the epoch begins.
    latest: Arc<Loaded>,
    keys: Vec<KeyPath>,
    profile_tokens: u64,
    map_tokens: u64,
}

/// What the changes need, built before anything is applied.
#[derive(Default)]
struct Prepared {
    engine: Option<cineris_policy::Parts>,
    policy: Option<cineris_sandbox::policy::Policy>,
    models: Option<cineris_agent::Models>,
    speeds: Option<cineris_agent::SpeedSettings>,
    accounts: Option<Vec<cineris_agent::Account>>,
    reconfigure: Reconfigure,
    verify: Option<crate::verify::VerifyLive>,
    key_variables: Option<Vec<String>>,
    repository: Option<bool>,
    epoch: Option<EpochPrepared>,
}

/// What a reload builds before it applies anything.
struct Building<'a> {
    mix: &'a Arc<Loaded>,
    now: &'a [KeyPath],
    trusted: Trusted,
    was: Trusted,
    /// The keys that wait for the next epoch, and what they are taken from.
    epoch: Option<(&'a Arc<Loaded>, &'a [KeyPath])>,
    /// The keys the person chose in the session.
    chosen: &'a BTreeSet<String>,
}

/// Builds what applying the changes needs, checking it whole: off the core's
/// thread wherever a file or a credential is read.
async fn prepare(
    core: &Core,
    live: &Arc<Live>,
    building: Building<'_>,
) -> Result<Prepared, String> {
    let Building {
        mix,
        now,
        trusted,
        was,
        epoch,
        chosen,
    } = building;
    let chosen_roles = chosen_roles(chosen);
    let inner = &core.inner;
    let mut prepared = Prepared::default();

    if trusted != was
        || touches(
            now,
            &[
                "permissions",
                "sandbox.write",
                "sandbox.network",
                "limits.session",
            ],
        )
    {
        let template = lock(&live.template).parts.clone();
        let session = live.handle.id().clone();
        let state = lock(&live.config).state.clone();
        let (held, loaded, sandbox) = (
            Arc::clone(inner),
            Arc::clone(mix),
            Arc::clone(&live.sandbox),
        );
        let built = Core::blocking(move || {
            let parts = assemble::reloaded_parts(
                &held,
                &template,
                &loaded,
                trusted.rules,
                trusted.repository,
                &session,
            );
            let mut policy = None;
            if parts.sandbox_policy != template.sandbox_policy && sandbox.enforces() {
                if assemble::holds_harness_files(
                    &parts.sandbox_policy,
                    &loaded,
                    state.as_ref(),
                    held.sandbox.program(),
                ) {
                    return Err(error(
                        ErrorKind::Config,
                        "harness_files_writable",
                        "`sandbox.write` would let a confined command write the harness's own \
                         files, so the session's sandbox is left as it was.",
                    ));
                }
                policy = sandbox.policy_for(&parts.sandbox_policy);
            }
            Ok((parts, policy))
        })
        .await
        .map_err(|failure| failure.message)?;
        prepared.engine = Some(built.0);
        prepared.policy = built.1;
    }

    let models_now = touches(now, &["retry", "model", "adapt"])
        || provider_field(
            now,
            &["key_env", "headers", "idle_timeout", "first_event_timeout"],
        );
    if models_now {
        let settings = mix.settings.clone();
        let images = Arc::clone(&live.images);
        let held = Arc::clone(inner);
        let lenient = lenient_roles(chosen);
        let models =
            Core::blocking(move || assemble::models_lenient(&held, &settings, &images, &lenient))
                .await
                .map_err(|failure| failure.message)?;
        prepared.models = Some(models);
    }
    if touches(now, &["model.speed", "model.roles"]) {
        prepared.speeds = Some(assemble::speeds(&mix.settings));
    }

    if touches(now, &["limits.session", "limits.day"]) {
        let mut accounts = lock(&live.accounts).clone();
        if let Some(lease) = accounts.first_mut() {
            *lease = lease.with_budget(assemble::budget(&mix.settings.limits.session));
        }
        if touches(now, &["limits.day"]) {
            accounts.truncate(1);
            let held = Arc::clone(inner);
            let day = mix.settings.limits.day.clone();
            if let Some(day) = Core::blocking(move || crate::day::day_account(&held, &day))
                .await
                .map_err(|failure| failure.message)?
            {
                accounts.push(day);
            }
        }
        prepared.accounts = Some(accounts);
    }
    if touches(now, &["limits.turn"]) {
        prepared.reconfigure.turn_budget = Some(assemble::budget(&mix.settings.limits.turn));
    }
    if touches(
        now,
        &[
            "context.auto_compact",
            "context.background",
            "context.compact_at",
            "context.prepare_at",
        ],
    ) {
        let context = &mix.settings.context;
        prepared.reconfigure.compaction = Some(Compaction {
            auto_compact: context.auto_compact,
            background: context.background,
            compact_at: context.compact_at,
            prepare_at: context.prepare_at,
        });
    }
    if touches(now, &["verify.rounds"]) {
        prepared.reconfigure.verify_rounds =
            Some(u32::try_from(mix.settings.verify.rounds).unwrap_or(u32::MAX));
    }
    if trusted != was || touches(now, &["verify"]) {
        let in_force = lock(&live.config).in_force();
        let origin = mix.origin(&KeyPath::new(["verify", "mode"]));
        prepared.verify = Some(crate::verify::VerifyLive {
            settings: crate::verify::Settings::of(
                &mix.settings.verify,
                // The shell's longest limit is the session's own: it moves
                // only with a new session.
                assemble::shell_limits(&in_force.settings).1,
            ),
            trusted: trusted.repository,
            lowered_by_project: matches!(
                assemble::rule_source(origin.layer, origin.defined_in),
                cineris_policy::RuleSource::Project
            ),
        });
    }
    if provider_field(now, &["key_env"]) {
        prepared.key_variables = Some(crate::provider::key_variables(&mix.settings));
    }
    if trusted.repository != was.repository {
        prepared.repository = Some(trusted.repository);
    }

    // What waits for the next epoch is built now too, so that a model whose
    // credential is missing refuses the reload rather than wait to fail.
    if let Some((latest, keys)) = epoch
        && !keys.is_empty()
    {
        let at_epoch = Arc::new(
            mix.taking(latest, keys)
                .map_err(|failure| failure.render(&inner.options.env))?,
        );
        let mut models = Vec::new();
        if touches(keys, &["model", "providers", "adapt"]) {
            let settings = at_epoch.settings.clone();
            let images = Arc::clone(&live.images);
            let held = Arc::clone(inner);
            let built = Core::blocking(move || assemble::models(&held, &settings, &images))
                .await
                .map_err(|failure| failure.message)?;
            for (role, model) in built.roles() {
                // A role the person set in the session keeps what they set.
                if !chosen_roles.contains(role.as_str()) {
                    models.push((role.clone(), model.clone()));
                }
            }
        }
        let context = touches(keys, &["context"]).then(|| {
            let settings = assemble::context_settings(&at_epoch.settings);
            EpochContext {
                keep_recent_tokens: settings.keep_recent_tokens,
                instruction_names: settings.instruction_names,
                instructions_cap: settings.instructions_cap,
            }
        });
        prepared.epoch = Some(EpochPrepared {
            models,
            context,
            latest: Arc::clone(latest),
            keys: keys.to_vec(),
            profile_tokens: at_epoch.settings.project.profile_tokens,
            map_tokens: at_epoch.settings.index.map_tokens,
        });
    }
    Ok(prepared)
}

/// Puts what was prepared in force. Only the engine's lease records can
/// fail, and they are written first.
async fn put(core: &Core, live: &Arc<Live>, prepared: Prepared) -> Result<(), OpError> {
    if let Some(parts) = prepared.engine {
        core.put_engine(live, parts).await?;
    }
    if let Some(policy) = prepared.policy {
        live.sandbox.set_policy(policy);
    }
    if let Some(models) = prepared.models {
        for (role, model) in models.roles() {
            live.handle.set_role_model(role.clone(), model.clone());
        }
    }
    if let Some(speeds) = prepared.speeds {
        live.handle.set_speed(speeds.session.clone());
        for (role, speed) in speeds.roles {
            live.handle.set_role_speed(role, speed);
        }
    }
    let mut reconfigure = prepared.reconfigure;
    if let Some(accounts) = prepared.accounts {
        // Kept, so that the next change of a limit starts from these.
        lock(&live.accounts).clone_from(&accounts);
        reconfigure.accounts = Some(accounts);
    }
    live.handle.reconfigure(reconfigure);
    if let Some(verify) = prepared.verify {
        live.verifier.set_live(verify);
    }
    if let Some(variables) = prepared.key_variables {
        live.shell_live.set_key_variables(variables);
    }
    if let Some(repository) = prepared.repository {
        live.shell_live.set_repository_trusted(repository);
        live.repository.store(repository, Ordering::Release);
    }
    Ok(())
}

/// Builds and puts in force; on a failure, the reason. `None` when the loop
/// took what waited for an epoch while it built (`generation` is no longer
/// the configuration's), and nothing was put.
async fn build_and_put(
    core: &Core,
    live: &Arc<Live>,
    building: Building<'_>,
    generation: u64,
) -> Result<Option<Option<EpochPrepared>>, String> {
    let mut prepared = prepare(core, live, building).await?;
    if lock(&live.config).generation != generation {
        return Ok(None);
    }
    let epoch = prepared.epoch.take();
    put(core, live, prepared)
        .await
        .map_err(|failure| failure.message)?;
    Ok(Some(epoch))
}

/// Applies what a look loaded to a running session, and says what it did.
#[expect(
    clippy::too_many_lines,
    reason = "one reload from what was loaded to what is said, each way it ends in its place"
)]
async fn apply(core: &Core, live: &Arc<Live>, looked: Looked) -> Result<ConfigChanged, Moved> {
    let Looked {
        trusted,
        standing,
        problem,
        latest,
        mut errors,
        profile,
    } = looked;
    let (in_force, was, kept_choices, generation) = {
        let config = lock(&live.config);
        (
            config.in_force(),
            config.trusted,
            config.chosen_keys(),
            config.generation,
        )
    };
    let loaded = latest.is_some();
    let latest = latest.unwrap_or_else(|| Arc::clone(&in_force));
    // A level the core may not give (a server not started at `unrestricted`)
    // is never applied: it is refused in the errors and stays pending, and
    // the rest of the reload is applied, a tightening included. A profile at
    // such a level is refused whole: neither its keys nor its taking the
    // place of the session's own choices; only what trust taken away takes
    // goes (the reviews of step 27's second pull request).
    let refused_level = core
        .inner
        .may_hold(&latest.settings.permissions.mode)
        .is_err();
    let refused_profile = refused_level && profile.is_some();
    // A profile starts from itself: the session's own choices go.
    let chosen = if profile.is_some() && !refused_profile {
        BTreeSet::new()
    } else {
        kept_choices
    };
    let lowered = (was.rules && !trusted.rules) || (was.repository && !trusted.repository);
    let changes = latest.changes_since(&in_force);
    // When trust was taken away, what it gave goes at once, whatever its
    // effect: a project's dearer model does not wait for the next epoch.
    let newly_held: BTreeSet<KeyPath> = if lowered {
        latest
            .withheld
            .iter()
            .map(|withheld| withheld.key.clone())
            .filter(|key| !in_force.withheld.iter().any(|before| before.key == *key))
            .collect()
    } else {
        BTreeSet::new()
    };
    let mut now: Vec<KeyPath> = Vec::new();
    let mut silent: Vec<KeyPath> = Vec::new();
    let mut waiting: BTreeMap<KeyPath, Effect> = BTreeMap::new();
    let mut refused: Vec<KeyPath> = Vec::new();
    for change in &changes {
        let held_back = newly_held
            .iter()
            .any(|held| change.key.is_at_or_under(held) || held.is_at_or_under(&change.key));
        // Of a refused profile, only what trust taken away takes.
        if refused_profile && !held_back {
            continue;
        }
        if refused_level && touches(std::slice::from_ref(&change.key), &["permissions.mode"]) {
            refused.push(change.key.clone());
            continue;
        }
        if chosen.contains(&change.key.to_string()) {
            // In force already, as `session/set` put it: taken into the
            // configuration, neither applied again nor said, whatever its
            // effect.
            silent.push(change.key.clone());
            continue;
        }
        if change.effect == Effect::Now || (held_back && change.effect == Effect::NextEpoch) {
            now.push(change.key.clone());
        } else {
            waiting.insert(change.key.clone(), change.effect);
        }
    }
    if refused_profile {
        errors.push(format!(
            "The profile was not chosen for this session: it sets `permissions.mode` to \
             `unrestricted`. {}",
            unrestricted_reason()
        ));
        // Nothing of the session changes unless trust was taken away.
        if !lowered {
            return Ok(ConfigChanged {
                errors,
                ..ConfigChanged::default()
            });
        }
    } else if !refused.is_empty() {
        errors.push(format!(
            "`permissions.mode` = `unrestricted` was not applied to this session, and waits; the \
             rest was applied. {}",
            unrestricted_reason()
        ));
    }
    let mut taken: Vec<KeyPath> = now.iter().chain(&silent).cloned().collect();
    let mut mix = match in_force.taking(&latest, &taken) {
        Ok(mix) => Arc::new(mix),
        Err(failure) => {
            errors.push(failure.render(&core.inner.options.env));
            return Ok(ConfigChanged {
                errors,
                ..ConfigChanged::default()
            });
        }
    };
    let mut epoch_keys: Vec<KeyPath> = waiting
        .iter()
        .filter(|(_, effect)| **effect == Effect::NextEpoch)
        .map(|(key, _)| key.clone())
        .collect();
    let mut applied = now.clone();
    let mut epoch = None;
    let mut ready = now.is_empty() && trusted == was && epoch_keys.is_empty();
    if !ready {
        match build_and_put(
            core,
            live,
            Building {
                mix: &mix,
                now: &now,
                trusted,
                was,
                epoch: Some((&latest, &epoch_keys)),
                chosen: &chosen,
            },
            generation,
        )
        .await
        {
            Ok(Some(prepared)) => {
                epoch = prepared;
                ready = true;
            }
            Ok(None) => return Err(Moved),
            Err(reason) => {
                errors.push(format!(
                    "The configuration's changes were not applied to this session: {reason}"
                ));
            }
        }
    }
    if !ready && lowered {
        // Trust taken away is taken away even when something else of the
        // reload cannot be built: what can fail for reasons of its own
        // waits, and the rest, trust's part included, is applied.
        let safe: Vec<KeyPath> = now
            .iter()
            .filter(|key| !touches(std::slice::from_ref(*key), MAY_FAIL))
            .cloned()
            .collect();
        for key in now.iter().filter(|key| !safe.contains(key)) {
            waiting.insert(key.clone(), Effect::Now);
        }
        // What waits for the next epoch and can fail waits for the next
        // reload too; the rest is given to the loop as usual.
        for key in epoch_keys
            .iter()
            .filter(|key| touches(std::slice::from_ref(*key), MAY_FAIL))
        {
            waiting.insert(key.clone(), Effect::Now);
        }
        epoch_keys.retain(|key| !touches(std::slice::from_ref(key), MAY_FAIL));
        taken = safe.iter().chain(&silent).cloned().collect();
        if let Ok(trust_only) = in_force.taking(&latest, &taken) {
            mix = Arc::new(trust_only);
            match build_and_put(
                core,
                live,
                Building {
                    mix: &mix,
                    now: &safe,
                    trusted,
                    was,
                    epoch: Some((&latest, &epoch_keys)),
                    chosen: &chosen,
                },
                generation,
            )
            .await
            {
                Ok(Some(prepared)) => {
                    epoch = prepared;
                    applied = safe;
                    ready = true;
                }
                Ok(None) => return Err(Moved),
                Err(reason) => errors.push(format!(
                    "Trust taken away could not be applied to this session: {reason}"
                )),
            }
        }
    }
    if !ready {
        return Ok(ConfigChanged {
            errors,
            ..ConfigChanged::default()
        });
    }

    let mut notes = Vec::new();
    let dropped = {
        let mut config = lock(&live.config);
        // Taken on top of what is in force now, which is `in_force` unless
        // the loop took what waited for an epoch since: never a copy of an
        // older one written over it.
        let committed = if config.generation == generation {
            Arc::clone(&mix)
        } else {
            match config.in_force.taking(&latest, &taken) {
                Ok(on_top) => Arc::new(on_top),
                Err(_) => Arc::clone(&mix),
            }
        };
        config.set_in_force(Arc::clone(&committed));
        // A profile refused for its level leaves nothing of it: not what
        // was loaded with it, not its name, not the session's choices.
        if loaded && !refused_profile {
            config.latest = Arc::clone(&latest);
        }
        if refused_profile {
            // What waited before waits still, but for what trust's part
            // applied: the profile's load says nothing of it.
            config.waiting.retain(|key, _| !applied.contains(key));
            waiting = config.waiting.clone();
        } else {
            config.waiting = waiting.clone();
        }
        config.trusted = trusted;
        if trusted != was || loaded {
            config.standing = standing.clone();
        }
        match profile {
            Some(profile) if !refused_profile => {
                config.options.profile = Some(profile);
                std::mem::take(&mut config.choices)
            }
            _ => Vec::new(),
        }
    };
    if trusted != was {
        live.handle.set_trust(standing);
        if let Some(problem) = problem {
            notes.push(problem);
        }
    }
    if !dropped.is_empty() {
        let mut keys: Vec<String> = dropped
            .iter()
            .map(|choice| format!("`{}`", choice.key))
            .collect();
        keys.dedup();
        notes.push(format!(
            "The profile replaces what was chosen in this session: {}.",
            keys.join(", ")
        ));
    }
    install_epoch(live, epoch);
    if !refused_profile {
        notes.extend(waiting_notes(&waiting, &mix, &latest));
    }
    Ok(ConfigChanged {
        applied: applied.iter().map(ToString::to_string).collect(),
        pending: waiting
            .keys()
            .chain(&refused)
            .map(ToString::to_string)
            .collect(),
        errors,
        notes,
    })
}

/// Why a level of `unrestricted` is not given.
fn unrestricted_reason() -> String {
    crate::core::unrestricted_not_allowed().message
}

/// What `apply` says when the loop took what waited for an epoch while it
/// built: what it built is from an older configuration, and nothing of it
/// was put in force. It is applied again, from what is in force now.
struct Moved;

/// What the person is told of the keys that wait.
fn waiting_notes(
    waiting: &BTreeMap<KeyPath, Effect>,
    in_force: &Loaded,
    latest: &Loaded,
) -> Vec<String> {
    let named = |effect: Effect| -> Vec<String> {
        waiting
            .iter()
            .filter(|(_, waits)| **waits == effect)
            .map(|(key, _)| format!("`{key}`"))
            .collect()
    };
    let plural = |keys: &[String]| if keys.len() == 1 { "s" } else { "" };
    let mut notes = Vec::new();
    let now = named(Effect::Now);
    if !now.is_empty() {
        notes.push(format!(
            "{} could not be applied with trust taken away, and wait{} to be tried again: at \
             the next change of a configuration file, or with /reload.",
            now.join(", "),
            plural(&now)
        ));
    }
    let epoch = named(Effect::NextEpoch);
    if !epoch.is_empty() {
        notes.push(format!(
            "{} take{} effect at this session's next context epoch: a compaction, or the session \
             opened again.",
            epoch.join(", "),
            plural(&epoch)
        ));
    }
    let session = named(Effect::NextSession);
    if !session.is_empty() {
        notes.push(format!(
            "{} take{} effect in sessions opened from now on, not in this one.",
            session.join(", "),
            plural(&session)
        ));
        // What would tighten the sandbox is said with more weight: this
        // session's stays as it was.
        let turned_on = !in_force.settings.sandbox.enabled && latest.settings.sandbox.enabled;
        let added: Vec<&String> = latest
            .settings
            .sandbox
            .protect
            .iter()
            .filter(|path| !in_force.settings.sandbox.protect.contains(path))
            .collect();
        if turned_on || !added.is_empty() {
            let mut what = Vec::new();
            if turned_on {
                what.push("turns the sandbox on".to_owned());
            }
            if !added.is_empty() {
                what.push(format!(
                    "protects {}",
                    added
                        .iter()
                        .map(|path| format!("`{path}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            notes.push(format!(
                "This session's sandbox stays as it was: the change that {} holds only in a new \
                 session. Start one with /new for it to protect what you are working on.",
                what.join(" and ")
            ));
        }
    }
    let restart = named(Effect::Restart);
    if !restart.is_empty() {
        notes.push(format!(
            "{} take{} effect when Cineris is next started.",
            restart.join(", "),
            plural(&restart)
        ));
    }
    notes
}

/// Gives the session's loop what waits for its next epoch, or nothing.
fn install_epoch(live: &Arc<Live>, epoch: Option<EpochPrepared>) {
    let kept = epoch.map(|epoch| EpochKept {
        models: epoch.models,
        context: epoch.context,
        latest: epoch.latest,
        keys: epoch.keys,
        profile_tokens: epoch.profile_tokens,
        map_tokens: epoch.map_tokens,
    });
    lock(&live.config).epoch.clone_from(&kept);
    give_epoch(live, kept);
}

/// Gives the loop `kept`, or nothing.
fn give_epoch(live: &Arc<Live>, kept: Option<EpochKept>) {
    let Some(EpochKept {
        models,
        context,
        latest,
        keys,
        profile_tokens,
        map_tokens,
    }) = kept
    else {
        live.handle.at_next_epoch(None);
        return;
    };
    let weak: Weak<Live> = Arc::downgrade(live);
    let taken = Box::new(move || {
        let Some(live) = weak.upgrade() else {
            return;
        };
        if let Some(knowledge) = &live.knowledge {
            knowledge.set_tokens(u32::try_from(profile_tokens).unwrap_or(u32::MAX));
        }
        if let Some(repo) = &live.repo {
            repo.set_tokens(usize::try_from(map_tokens).unwrap_or(usize::MAX));
        }
        // What the loop took is what this change held: the configuration it
        // was built from, not one a later reload loaded.
        let applied: Vec<KeyPath> = {
            let mut config = lock(&live.config);
            let still: Vec<KeyPath> = keys
                .iter()
                .filter(|key| config.waiting.get(*key) == Some(&Effect::NextEpoch))
                .cloned()
                .collect();
            config.epoch = None;
            match config.in_force.taking(&latest, &still) {
                Ok(mix) => {
                    config.set_in_force(Arc::new(mix));
                    for key in &still {
                        config.waiting.remove(key);
                    }
                    still
                }
                Err(_) => Vec::new(),
            }
        };
        if !applied.is_empty() {
            live.hooks.emit(Event::ConfigChanged(ConfigChanged {
                applied: applied.iter().map(ToString::to_string).collect(),
                notes: vec![
                    "The session's new context epoch began: what waited for it is in force."
                        .to_owned(),
                ],
                ..ConfigChanged::default()
            }));
        }
    });
    live.handle.at_next_epoch(Some(EpochChange {
        models,
        context,
        taken: Some(taken),
    }));
}

/// After `session/set`, which holds the session's reload lock across its
/// change and this: what the person chose is kept as the session's own and
/// taken into the configuration in force (`config/get` says it, and a
/// profile then changes it), no longer waits, and a role they set is taken
/// out of what waits for the next epoch.
pub(crate) async fn chosen(live: &Arc<Live>, choices: Vec<Override>) {
    if choices.is_empty() {
        return;
    }
    cineris_base::hook::await_point!(CHOICE_BEING_KEPT);
    let (options, keys) = {
        let mut config = lock(&live.config);
        config.choose(choices);
        let keys: Vec<KeyPath> = config
            .chosen_keys()
            .iter()
            .filter_map(|key| KeyPath::parse(key).ok())
            .collect();
        (config.options_with(config.trusted.rules, None, true), keys)
    };
    // Loaded with the choices among its overrides, of which only the chosen
    // keys are taken: their values are the choices'.
    let loaded = Core::blocking(move || Ok(cineris_config::load(&options).ok())).await;
    let mut config = lock(&live.config);
    if let Ok(Some(loaded)) = loaded
        && let Ok(mix) = config.in_force.taking(&loaded, &keys)
    {
        config.set_in_force(Arc::new(mix));
    }
    let chosen = config.chosen_keys();
    config
        .waiting
        .retain(|key, _| !chosen.contains(&key.to_string()));
    let roles = chosen_roles(&chosen);
    let kept = config.epoch.take().and_then(|mut kept| {
        kept.models
            .retain(|(role, _)| !roles.contains(role.as_str()));
        kept.keys.retain(|key| !chosen.contains(&key.to_string()));
        (!kept.keys.is_empty()).then_some(kept)
    });
    config.epoch.clone_from(&kept);
    drop(config);
    give_epoch(live, kept);
}
