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

use cortex_agent::{Compaction, EpochChange, EpochContext, Reconfigure};
use cortex_base::{AbsPath, SharedClock, WatchList};
use cortex_config::schema::Effect;
use cortex_config::{KeyPath, LiveConfig, LoadOptions, Loaded, Override, ProjectTrust, Reload};
use cortex_protocol::ErrorKind;
use cortex_protocol::event::{ConfigChanged, Event};
use cortex_protocol::methods::{ClientRole, ConfigReloadParams, ConfigReloadResult};
use cortex_protocol::session::TrustStanding;

use crate::assemble;
use crate::clients::ClientId;
use crate::core::{Core, CoreOptions, Inner, Live, OpError, error};

/// Where a reload has judged the project's trust and not yet loaded with
/// it: a point a test can hold the reload at (`cortex_base::hook`).
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

/// What a session's configuration is loaded from, and what is in force.
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
    /// The files of the last load, stamped: made by the first reload.
    live: Option<LiveConfig>,
    /// The trust store's file, stamped.
    trust_file: Option<WatchList>,
    /// The trust in force.
    trusted: Trusted,
    standing: TrustStanding,
    root: AbsPath,
    state: Option<cortex_config::StateDir>,
    clock: SharedClock,
    /// The error last said, so that a broken file is said once, and again
    /// only when asked (`/reload`) or once it has changed.
    reported: Option<String>,
}

impl std::fmt::Debug for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionConfig")
            .field("trusted", &self.trusted)
            .field("waiting", &self.waiting)
            .finish_non_exhaustive()
    }
}

impl SessionConfig {
    /// A session's, as it was put together: `options` with the session's
    /// own `choices` last among its overrides, the configuration it was
    /// loaded as, and the trust it was judged with.
    #[expect(
        clippy::too_many_arguments,
        reason = "each is one thing a session's configuration was made from"
    )]
    pub(crate) fn new(
        mut options: LoadOptions,
        choices: Vec<Override>,
        in_force: Arc<Loaded>,
        trusted: Trusted,
        standing: TrustStanding,
        root: AbsPath,
        state: Option<cortex_config::StateDir>,
        core: &CoreOptions,
    ) -> Self {
        // The choices were put last among the overrides (`place`): kept
        // apart, so that a profile can drop them.
        if options.overrides.ends_with(&choices) {
            let base = options.overrides.len() - choices.len();
            options.overrides.truncate(base);
        }
        let clock = Arc::clone(&core.clock);
        let trust_file = state.as_ref().map(|state| {
            let mut watch = WatchList::new(Arc::clone(&clock));
            watch.watch(state.trust_file());
            watch
        });
        Self {
            options,
            choices,
            latest: Arc::clone(&in_force),
            in_force,
            waiting: BTreeMap::new(),
            live: None,
            trust_file,
            trusted,
            standing,
            root,
            state,
            clock,
            reported: None,
        }
    }

    /// The configuration in force.
    pub(crate) fn in_force(&self) -> Arc<Loaded> {
        Arc::clone(&self.in_force)
    }

    /// Records choices made in the session: they outrank the files from now
    /// on. Each replaces an earlier one of the same key.
    pub(crate) fn choose(&mut self, choices: Vec<Override>) {
        for choice in choices {
            self.choices.retain(|earlier| earlier.key != choice.key);
            self.choices.push(choice);
        }
    }

    /// The roles whose model, provider or reasoning level the person chose
    /// in the session.
    fn chosen_roles(&self) -> BTreeSet<String> {
        self.choices
            .iter()
            .filter_map(|choice| {
                let parts: Vec<&str> = choice.key.split('.').collect();
                match parts.as_slice() {
                    ["model", role] if !role.starts_with("speed") && *role != "effort" => {
                        Some((*role).to_owned())
                    }
                    ["model", "roles", role, "provider" | "effort"] => Some((*role).to_owned()),
                    _ => None,
                }
            })
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

/// The core's own configuration: the user's, with no project.
pub(crate) struct CoreConfig {
    options: LoadOptions,
    in_force: Arc<Loaded>,
    live: Option<LiveConfig>,
    clock: SharedClock,
    /// The error last said, so that a broken file is said once.
    reported: Option<String>,
}

impl CoreConfig {
    pub(crate) fn new(options: LoadOptions, in_force: Loaded, clock: SharedClock) -> Self {
        Self {
            options,
            in_force: Arc::new(in_force),
            live: None,
            clock,
            reported: None,
        }
    }

    /// The configuration in force.
    pub(crate) fn in_force(&self) -> Arc<Loaded> {
        Arc::clone(&self.in_force)
    }
}

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
    if params.profile.is_some() {
        // A profile can raise the level, which only a person may (as
        // `session/set` holds it).
        let role = core.inner.clients.info(client).map(|info| info.role);
        if role != Some(ClientRole::User) {
            return Err(error(
                ErrorKind::Permission,
                cortex_protocol::error::code::ROLE_NOT_ALLOWED,
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
    let inner = Arc::clone(&core.inner);
    let changed_only = asked.changed_only;
    let looked = Core::blocking(move || {
        let mut config = lock(&inner.user_config);
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
    let mut config = lock(&core.inner.user_config);
    let changes = latest.changes_since(&config.in_force);
    let now: Vec<KeyPath> = changes
        .iter()
        .filter(|change| change.effect == Effect::Now)
        .map(|change| change.key.clone())
        .collect();
    let Ok(mix) = config.in_force.taking(&latest, &now) else {
        return ConfigChanged::default();
    };
    let mix = Arc::new(mix);
    if touches(&now, &["log.level"])
        && let (Some(log), Some(level)) = (&core.inner.options.log, &mix.settings.log.level)
        && let Ok(filter) = cortex_base::log::Filter::parse(level)
    {
        log.set_filter(filter);
    }
    if touches(&now, &["stats.retention"])
        && let Some(recorder) = &core.inner.stats
    {
        recorder.set_retention(mix.settings.stats.retention);
    }
    config.in_force = mix;
    // The core's own configuration tells nobody of what waits: each
    // session's says it for the session.
    ConfigChanged {
        applied: now.iter().map(ToString::to_string).collect(),
        ..ConfigChanged::default()
    }
}

// ----------------------------------------------------------- a session's

/// What the look at a session's files found, off the core's thread.
struct Looked {
    trusted: Trusted,
    standing: TrustStanding,
    problem: Option<String>,
    /// The configuration loaded, when one was.
    latest: Option<Arc<Loaded>>,
    errors: Vec<String>,
    /// The session's own choices were dropped for a profile.
    dropped: Vec<Override>,
}

/// One session's reload, start to end.
async fn reload_session(
    core: &Core,
    live: &Arc<Live>,
    asked: &Asked,
) -> Result<(ConfigChanged, bool), OpError> {
    let (inner, held, asked_now) = (Arc::clone(&core.inner), Arc::clone(live), asked.clone());
    let judging = Core::blocking(move || Ok(judge(&inner, &held, &asked_now))).await?;
    let Some(judging) = judging else {
        return Ok((ConfigChanged::default(), false));
    };
    // A test holds the reload here, to change the trust store between the
    // judgement and the load: the load uses the trust just judged.
    cortex_base::hook::await_point!(TRUST_JUDGED);
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
    let changed = apply(core, live, looked).await;
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

/// What the judgement of trust found, before the load that uses it.
struct Judging {
    judged: assemble::Judged,
    trusted: Trusted,
}

/// Looks at the session's files and the trust store and judges trust,
/// from loads that are never in force. `None`: nothing changed. Blocks.
fn judge(inner: &Inner, live: &Live, asked: &Asked) -> Option<Judging> {
    let mut config = lock(&live.config);
    let files = match config.live.as_mut() {
        Some(watched) => watched.has_changed(),
        None => true,
    };
    let trust_file = config
        .trust_file
        .as_mut()
        .is_some_and(|watch| !watch.check().is_empty());
    if asked.changed_only && !files && !trust_file && asked.profile.is_none() {
        return None;
    }
    let keep_choices = asked.profile.is_none();
    // Judged first: what the project may do is decided before anything of
    // it is loaded to be used.
    let judging = config.options_with(true, asked.profile.as_deref(), keep_choices);
    let judged = assemble::judge(inner, config.state.as_ref(), &config.root, &judging);
    let trusted = Trusted {
        rules: judged.rules.unwrap_or(config.trusted.rules),
        repository: judged.repository,
    };
    Some(Judging { judged, trusted })
}

/// Loads the session's configuration once, with the trust `judging` found:
/// a change of the trust store since is the next look's. Blocks.
fn load(inner: &Inner, live: &Live, asked: &Asked, judging: Judging) -> Option<Looked> {
    let Judging { judged, trusted } = judging;
    let mut config = lock(&live.config);
    let keep_choices = asked.profile.is_none();
    let options = config.options_with(trusted.rules, asked.profile.as_deref(), keep_choices);
    let clock = Arc::clone(&config.clock);
    let reload = match config.live.as_mut() {
        Some(watched) => watched.update(|now| *now = options),
        None => match LiveConfig::start(options, clock) {
            Ok(watched) => {
                config.live = Some(watched);
                Reload::Unchanged
            }
            Err(failure) => Reload::Rejected(failure),
        },
    };
    let env = &inner.options.env;
    let (latest, errors, trusted) = match reload {
        Reload::Rejected(failure) => {
            // Nothing loaded: what is in force stays, its trust with it.
            // The same error is said once, unless the person asks again.
            let text = failure.render(env);
            let said = config.reported.as_deref() == Some(text.as_str());
            config.reported = Some(text.clone());
            if said && !asked.explicit {
                return None;
            }
            (None, vec![text], config.trusted)
        }
        Reload::TrustLowered { failure, .. } => (
            config.live.as_ref().map(LiveConfig::current),
            vec![failure.render(env)],
            trusted,
        ),
        Reload::Unchanged | Reload::Applied { .. } => {
            config.reported = None;
            (
                config.live.as_ref().map(LiveConfig::current),
                Vec::new(),
                trusted,
            )
        }
    };
    let mut dropped = Vec::new();
    if latest.is_some()
        && let Some(profile) = &asked.profile
    {
        config.options.profile = Some(profile.clone());
        dropped = std::mem::take(&mut config.choices);
    }
    Some(Looked {
        trusted,
        standing: if latest.is_some() {
            judged.standing
        } else {
            config.standing.clone()
        },
        problem: judged.problem,
        latest,
        errors,
        dropped,
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

/// What the `Now` keys need, built before anything is applied.
#[derive(Default)]
struct Prepared {
    engine: Option<cortex_policy::Parts>,
    grant: bool,
    models: Option<cortex_agent::Models>,
    speeds: Option<cortex_agent::SpeedSettings>,
    reconfigure: Reconfigure,
    verify: Option<crate::verify::VerifyLive>,
    key_variables: Option<Vec<String>>,
    repository: Option<bool>,
}

/// Builds what applying `now` and the trust needs, checking it whole.
async fn prepare(
    core: &Core,
    live: &Arc<Live>,
    mix: &Arc<Loaded>,
    now: &[KeyPath],
    trusted: Trusted,
    was: Trusted,
) -> Result<Prepared, String> {
    let inner = &core.inner;
    let trust_changed = trusted != was;
    let mut prepared = Prepared::default();
    let session = live.handle.id().clone();

    if trust_changed
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
        let parts = assemble::reloaded_parts(
            inner,
            &template,
            mix,
            trusted.rules,
            trusted.repository,
            &session,
        );
        let grant = parts.sandbox_policy != template.sandbox_policy;
        if grant
            && live.sandbox.enforces()
            && assemble::holds_harness_files(
                &parts.sandbox_policy,
                mix,
                lock(&live.config).state.as_ref(),
                inner.sandbox.program(),
            )
        {
            return Err(
                "`sandbox.write` would let a confined command write the harness's own files, so \
                 the session's sandbox is left as it was."
                    .to_owned(),
            );
        }
        prepared.grant = grant;
        prepared.engine = Some(parts);
    }

    let models_now = touches(now, &["retry", "model.speed_fallback", "model.speed_wait"])
        || touches(now, &["model.speed_cooldown", "model.speed_cooldown_min"])
        || provider_field(
            now,
            &["key_env", "headers", "idle_timeout", "first_event_timeout"],
        )
        || touches(
            now,
            &["model.main", "model.fast", "model.judge", "model.effort"],
        )
        || touches(now, &["model.max_output", "model.reasoning", "model.roles"])
        || touches(now, &["adapt"]);
    if models_now {
        let settings = mix.settings.clone();
        let images = Arc::clone(&live.images);
        let held = Arc::clone(inner);
        let models = Core::blocking(move || assemble::models(&held, &settings, &images))
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
        prepared.reconfigure.accounts = Some(accounts);
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
    if trust_changed || touches(now, &["verify"]) {
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
                cortex_policy::RuleSource::Project
            ),
        });
    }
    if provider_field(now, &["key_env"]) {
        prepared.key_variables = Some(crate::provider::key_variables(&mix.settings));
    }
    if trusted.repository != was.repository {
        prepared.repository = Some(trusted.repository);
    }
    Ok(prepared)
}

/// Puts what was prepared in force.
async fn put(core: &Core, live: &Arc<Live>, prepared: Prepared) -> Result<(), OpError> {
    if let Some(parts) = prepared.engine {
        let grant = parts.sandbox_policy.clone();
        core.put_engine(live, parts).await?;
        if prepared.grant {
            live.sandbox.set_grant(&grant);
        }
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
    live.handle.reconfigure(prepared.reconfigure);
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

/// Applies what a look loaded to a running session, and says what it did.
async fn apply(core: &Core, live: &Arc<Live>, looked: Looked) -> ConfigChanged {
    let Looked {
        trusted,
        standing,
        problem,
        latest,
        mut errors,
        dropped,
    } = looked;
    let (in_force, was, chosen) = {
        let config = lock(&live.config);
        let chosen: BTreeSet<String> = config
            .choices
            .iter()
            .map(|choice| choice.key.clone())
            .collect();
        (config.in_force(), config.trusted, chosen)
    };
    let Some(latest) = latest else {
        return ConfigChanged {
            errors,
            ..ConfigChanged::default()
        };
    };
    let lowered = (was.rules && !trusted.rules) || (was.repository && !trusted.repository);
    let changes = latest.changes_since(&in_force);
    // What applies now: the keys that apply at once, and, when trust was
    // taken away, what it had given, whatever its effect (a project's
    // dearer model does not wait for the next epoch).
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
    let (mut now, mut waiting): (Vec<KeyPath>, BTreeMap<KeyPath, Effect>) =
        (Vec::new(), BTreeMap::new());
    for change in &changes {
        let held_back = newly_held
            .iter()
            .any(|held| change.key.is_at_or_under(held) || held.is_at_or_under(&change.key));
        if change.effect == Effect::Now || (held_back && change.effect == Effect::NextEpoch) {
            now.push(change.key.clone());
        } else {
            waiting.insert(change.key.clone(), change.effect);
        }
    }
    // A key the person chose in the session is in force already, as
    // `session/set` put it: taken into the configuration, and neither
    // applied again nor said.
    let applying: Vec<KeyPath> = now
        .iter()
        .filter(|key| !chosen.contains(&key.to_string()))
        .cloned()
        .collect();
    let mix = match in_force.taking(&latest, &now) {
        Ok(mix) => Arc::new(mix),
        Err(failure) => {
            errors.push(failure.render(&core.inner.options.env));
            return ConfigChanged {
                errors,
                ..ConfigChanged::default()
            };
        }
    };
    if !applying.is_empty() || trusted != was {
        let prepared = match prepare(core, live, &mix, &applying, trusted, was).await {
            Ok(prepared) => prepared,
            Err(reason) => {
                errors.push(format!(
                    "The configuration's changes were not applied to this session: {reason}"
                ));
                return ConfigChanged {
                    errors,
                    ..ConfigChanged::default()
                };
            }
        };
        if let Err(failure) = put(core, live, prepared).await {
            errors.push(format!(
                "The configuration's changes were not applied to this session: {}",
                failure.message
            ));
            return ConfigChanged {
                errors,
                ..ConfigChanged::default()
            };
        }
    }
    let mut notes = Vec::new();
    {
        let mut config = lock(&live.config);
        config.in_force = Arc::clone(&mix);
        config.latest = Arc::clone(&latest);
        config.waiting = waiting.clone();
        config.trusted = trusted;
        config.standing = standing.clone();
    }
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
    let epoch_keys: Vec<KeyPath> = waiting
        .iter()
        .filter(|(_, effect)| **effect == Effect::NextEpoch)
        .map(|(key, _)| key.clone())
        .collect();
    set_next_epoch(core, live, &mix, &latest, &epoch_keys).await;
    notes.extend(waiting_notes(&waiting, &mix, &latest));
    ConfigChanged {
        applied: applying.iter().map(ToString::to_string).collect(),
        pending: waiting.keys().map(ToString::to_string).collect(),
        errors,
        notes,
    }
}

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
    let mut notes = Vec::new();
    let epoch = named(Effect::NextEpoch);
    if !epoch.is_empty() {
        notes.push(format!(
            "{} take{} effect at this session's next context epoch: a compaction, or the session \
             opened again.",
            epoch.join(", "),
            if epoch.len() == 1 { "s" } else { "" }
        ));
    }
    let session = named(Effect::NextSession);
    if !session.is_empty() {
        notes.push(format!(
            "{} take{} effect in sessions opened from now on, not in this one.",
            session.join(", "),
            if session.len() == 1 { "s" } else { "" }
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
            "{} take{} effect when Cortex-Harness is next started.",
            restart.join(", "),
            if restart.len() == 1 { "s" } else { "" }
        ));
    }
    notes
}

/// Gives the session's loop what waits for its next epoch, or nothing.
async fn set_next_epoch(
    core: &Core,
    live: &Arc<Live>,
    in_force: &Arc<Loaded>,
    latest: &Arc<Loaded>,
    keys: &[KeyPath],
) {
    if keys.is_empty() {
        live.handle.at_next_epoch(None);
        return;
    }
    let Ok(at_epoch) = in_force.taking(latest, keys) else {
        live.handle.at_next_epoch(None);
        return;
    };
    let at_epoch = Arc::new(at_epoch);
    let chosen = lock(&live.config).chosen_roles();
    let mut models = Vec::new();
    if touches(keys, &["model", "providers", "adapt"]) {
        let settings = at_epoch.settings.clone();
        let images = Arc::clone(&live.images);
        let held = Arc::clone(&core.inner);
        if let Ok(built) = Core::blocking(move || assemble::models(&held, &settings, &images)).await
        {
            for (role, model) in built.roles() {
                // A role the person set in the session keeps what they set.
                if chosen.contains(role.as_str()) {
                    continue;
                }
                models.push((role.clone(), model.clone()));
            }
        }
    }
    let context = touches(keys, &["context", "compat"]).then(|| {
        let settings = assemble::context_settings(&at_epoch.settings);
        EpochContext {
            keep_recent_tokens: settings.keep_recent_tokens,
            instruction_names: settings.instruction_names,
            instructions_cap: settings.instructions_cap,
        }
    });
    let weak: Weak<Live> = Arc::downgrade(live);
    let taken_keys: Vec<KeyPath> = keys.to_vec();
    let profile_tokens = at_epoch.settings.project.profile_tokens;
    let map_tokens = at_epoch.settings.index.map_tokens;
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
        let applied: Vec<KeyPath> = {
            let mut config = lock(&live.config);
            let latest = Arc::clone(&config.latest);
            let keys: Vec<KeyPath> = taken_keys
                .iter()
                .filter(|key| config.waiting.get(*key) == Some(&Effect::NextEpoch))
                .cloned()
                .collect();
            if let Ok(mix) = config.in_force.taking(&latest, &keys) {
                config.in_force = Arc::new(mix);
                for key in &keys {
                    config.waiting.remove(key);
                }
                keys
            } else {
                Vec::new()
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

/// After `session/set`: what the person chose is kept as the session's own,
/// and a role they set no longer waits for the next epoch.
pub(crate) async fn chosen(core: &Core, live: &Arc<Live>, choices: Vec<Override>) {
    if choices.is_empty() {
        return;
    }
    let (in_force, latest, keys) = {
        let mut config = lock(&live.config);
        config.choose(choices);
        let keys: Vec<KeyPath> = config
            .waiting
            .iter()
            .filter(|(_, effect)| **effect == Effect::NextEpoch)
            .map(|(key, _)| key.clone())
            .collect();
        (config.in_force(), Arc::clone(&config.latest), keys)
    };
    set_next_epoch(core, live, &in_force, &latest, &keys).await;
}
