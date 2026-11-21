//! The session's side of finding its extensions: the task that finds,
//! judges and registers them when the session opens, and the look at a
//! turn's start that does it again when what they were found in changed
//! (the [parent module](super) says what each rule is).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use cineris_base::{CancelToken, ContentHash, PathResolver, SharedClock, visible};
use cineris_config::StateDir;
use cineris_ext::cache::{Entry, RegistrationCache};
use cineris_ext::discover::{self, Discovered, Found, Root};
use cineris_ext::source::Source;
use cineris_policy::{Definition, PinStore, ProjectTrust, Standing, TrustStore, UserPins};
use cineris_protocol::ProjectKey;
use cineris_protocol::methods::TrustResourceKind;
use cineris_tools::ToolRegistry;

use super::{
    BEFORE_AN_EXTENSION_IS_REGISTERED, BEFORE_THE_CACHE_IS_KEPT, Extensions, Pending, UserNames,
    definition, in_project, sentence, with_paths_shown,
};
use crate::ext::watched::Watched;
use crate::ext::{ApprovedExtension, HostParts};

/// What a session finds its extensions with.
pub(crate) struct Finding {
    /// The session's path resolver: its workspace, and the home directory
    /// `~` stands for.
    pub(crate) resolver: Arc<PathResolver>,
    /// The configuration directory, as it was given: the user's folder of
    /// extensions is in it.
    pub(crate) config_dir: Option<PathBuf>,
    /// The home directory, which a path the user is shown is under.
    pub(crate) home: Option<PathBuf>,
    /// The state directory: the pins, the trust store and the cache.
    pub(crate) state: Option<StateDir>,
    /// The project's key, when the project is trusted (in the store, or for
    /// the run): its folder of extensions is looked in.
    pub(crate) project: Option<ProjectKey>,
    /// What the trust question found in the project's folder, when it
    /// looked: used in place of a second reading of it.
    pub(crate) project_found: Option<Discovered>,
    /// The project's extensions a yes the trust store could not keep
    /// approved, by the hashes shown: approved for this session.
    pub(crate) for_the_run: Vec<Definition>,
    /// The project is trusted for the run by the command line
    /// (`--trust-project`), which approves no extension.
    pub(crate) trust_project: bool,
    /// The names the user's configuration turns off
    /// (`[extensions.<name>] enabled = false`).
    pub(crate) disabled: Vec<String>,
    /// Where the approved extensions' tools go.
    pub(crate) registry: Arc<ToolRegistry>,
    /// What their calls are served with, and how the session is told.
    pub(crate) parts: Arc<HostParts>,
    /// Ends the work when the session or the core ends.
    pub(crate) cancel: CancelToken,
}

impl std::fmt::Debug for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Finding")
            .field("config_dir", &self.config_dir)
            .field("project", &self.project)
            .finish_non_exhaustive()
    }
}

/// What a session keeps of its extensions from one turn to the next.
#[derive(Default)]
pub(crate) struct Shared {
    held: tokio::sync::Mutex<Option<Held>>,
    cancel: Mutex<Option<CancelToken>>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

/// An extension as the session holds it: where it comes from, its name,
/// and the hash it was loaded at.
type Key = (Source, String, ContentHash);

/// What the session found, and loaded.
struct Held {
    inputs: Arc<Inputs>,
    registry: Arc<ToolRegistry>,
    parts: Arc<HostParts>,
    /// What is loaded, by the hash it was loaded at.
    loaded: Vec<Key>,
    /// What the session was told of what was found: not said again.
    told: HashSet<String>,
    /// What it was found in, stamped.
    watched: Watched,
}

/// What the blocking look is given.
struct Inputs {
    resolver: Arc<PathResolver>,
    config_dir: Option<PathBuf>,
    home: Option<PathBuf>,
    state: Option<StateDir>,
    project: Option<ProjectKey>,
    for_the_run: Vec<Definition>,
    trust_project: bool,
    disabled: Vec<String>,
    clock: SharedClock,
}

/// What the blocking look decided.
#[derive(Default)]
struct Decided {
    /// What to tell the session.
    told: Vec<String>,
    /// Approved, with what they registered from the cache.
    approved: Vec<ApprovedExtension>,
    /// Approved, and missing from the cache: registered now.
    missing: Vec<Found>,
    /// The cache, when it was read: written and pruned after.
    cache: Option<RegistrationCache>,
    /// What everything was found in, stamped.
    watched: Watched,
}

/// The cache's part of the work, done after the session's extensions are
/// settled: what was registered written, and the cache pruned.
struct Kept {
    cache: RegistrationCache,
    written: Vec<(ContentHash, Entry)>,
    state: StateDir,
    clock: SharedClock,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts finding a session's extensions, on a task of its own: what the
/// session's first turn waits for.
pub(crate) fn start(pending: Pending) -> Arc<Extensions> {
    let Some(finding) = pending else {
        return Extensions::none();
    };
    let extensions = Arc::new(Extensions::default());
    *lock(&extensions.held.cancel) = Some(finding.cancel.clone());
    let task = Arc::clone(&extensions);
    tokio::spawn(async move {
        let _settles = Settles(Arc::clone(&task));
        let cancel = finding.cancel.clone();
        let _ = cancel.race(first(&task, finding)).await;
    });
    extensions
}

/// Settles a session's extensions when it is dropped: when the work ends,
/// however it ends (done, cancelled, or a panic), so that a turn never
/// waits for work that is gone. What did not get registered is then not
/// loaded.
struct Settles(Arc<Extensions>);

impl Drop for Settles {
    fn drop(&mut self) {
        self.0.ready.settle();
    }
}

/// The session's first finding: settled as soon as its tools are in the
/// registry, and the cache written and pruned after that, which no turn
/// waits for.
async fn first(extensions: &Extensions, finding: Finding) {
    let Finding {
        resolver,
        config_dir,
        home,
        state,
        project,
        project_found,
        for_the_run,
        trust_project,
        disabled,
        registry,
        parts,
        cancel: _,
    } = finding;
    let inputs = Arc::new(Inputs {
        resolver,
        config_dir,
        home,
        state,
        project,
        for_the_run,
        trust_project,
        disabled,
        clock: Arc::clone(&parts.clock),
    });
    let kept = {
        let mut slot = extensions.held.held.lock().await;
        let mut held = Held {
            inputs,
            registry,
            parts,
            loaded: Vec::new(),
            told: HashSet::new(),
            watched: Watched::default(),
        };
        let kept = held.settle(project_found).await;
        *slot = Some(held);
        kept
    };
    extensions.ready.settle();
    if let Some(kept) = kept {
        cineris_base::hook::await_point!(BEFORE_THE_CACHE_IS_KEPT);
        let _ = blocking(move || kept.keep()).await;
    }
}

impl Shared {
    /// At a turn's start: when what the extensions were found in changed,
    /// they are found and judged again, what no longer stands approved as
    /// it was loaded is withdrawn, and what now stands approved is loaded.
    pub(crate) async fn recheck(&self) {
        let kept = {
            let mut slot = self.held.lock().await;
            let Some(held) = slot.as_mut() else {
                return;
            };
            let watched = std::mem::take(&mut held.watched);
            let Some((changed, watched)) = blocking(move || (watched.changed(), watched)).await
            else {
                return;
            };
            held.watched = watched;
            if changed.is_none() {
                return;
            }
            held.settle(None).await
        };
        if let Some(kept) = kept {
            let _ = blocking(move || kept.keep()).await;
        }
    }

    /// Ends the session's finding, if it is still under way.
    pub(crate) fn stop(&self) {
        if let Some(cancel) = lock(&self.cancel).as_ref() {
            cancel.cancel();
        }
    }
}

impl Held {
    /// Says `message`, unless the session was told it already.
    fn tell(&mut self, message: String) {
        if self.told.insert(message.clone()) {
            (self.parts.tell)(message);
        }
    }

    /// Finds and judges the extensions, withdraws what no longer stands
    /// approved as it was loaded, and loads what now stands approved: from
    /// the cache, or registered first. What is left of the cache's work.
    async fn settle(&mut self, project_found: Option<Discovered>) -> Option<Kept> {
        let inputs = Arc::clone(&self.inputs);
        let Decided {
            told,
            approved,
            missing,
            cache,
            watched,
        } = blocking(move || decide(&inputs, project_found)).await?;
        self.watched = watched;
        for message in told {
            self.tell(message);
        }
        let wanted: Vec<Key> = approved
            .iter()
            .map(|one| (one.source, one.manifest.name.clone(), one.hash))
            .chain(
                missing
                    .iter()
                    .map(|one| (one.source, one.manifest.name.clone(), one.snapshot.hash())),
            )
            .collect();
        for gone in self.loaded.iter().filter(|key| !wanted.contains(key)) {
            self.registry.remove_source(&gone.1);
            (self.parts.tell)(format!(
                "The extension `{}` is withdrawn from this session: its tools are no longer \
                 offered.",
                visible(&gone.1)
            ));
        }
        self.loaded.retain(|key| wanted.contains(key));
        let mut new: Vec<ApprovedExtension> = approved
            .into_iter()
            .filter(|one| {
                !self
                    .loaded
                    .contains(&(one.source, one.manifest.name.clone(), one.hash))
            })
            .collect();
        let mut written: Vec<(ContentHash, Entry)> = Vec::new();
        for found in missing {
            let hash = found.snapshot.hash();
            if self
                .loaded
                .contains(&(found.source, found.manifest.name.clone(), hash))
            {
                continue;
            }
            let name = visible(&found.manifest.name).into_owned();
            // A turn that waits says why.
            (self.parts.tell)(format!(
                "Registering the extension `{name}` (`{}`): what it registers is not in the \
                 registration cache, so it is evaluated once before the turn starts.",
                found.shown
            ));
            cineris_base::hook::await_point!(BEFORE_AN_EXTENSION_IS_REGISTERED);
            match cineris_ext::engine::register(
                Arc::clone(&found.snapshot),
                self.parts.timer.as_ref(),
            )
            .await
            {
                Ok((manifest, registration)) => {
                    // The prompt's text as the runtime reads it: from the
                    // snapshot that was evaluated.
                    let prompt = manifest
                        .prompt
                        .as_deref()
                        .and_then(|file| found.snapshot.file(file))
                        .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                    written.push((hash, Entry::new(&manifest, registration.clone(), prompt)));
                    new.push(ApprovedExtension {
                        source: found.source,
                        folder: found.folder,
                        hash,
                        manifest,
                        registration,
                    });
                }
                Err(error) => (self.parts.tell)(format!(
                    "The extension `{name}` (`{}`) could not be registered, so it is not \
                     loaded: {error}",
                    found.shown
                )),
            }
        }
        let keys: Vec<Key> = new
            .iter()
            .map(|one| (one.source, one.manifest.name.clone(), one.hash))
            .collect();
        for message in crate::ext::register(new, &self.parts, &self.registry) {
            (self.parts.tell)(message);
        }
        self.loaded.extend(keys);
        let state = self.inputs.state.clone()?;
        cache.map(|cache| Kept {
            cache,
            written,
            state,
            clock: Arc::clone(&self.inputs.clock),
        })
    }
}

/// Runs `work` on the blocking pool. `None` when it could not finish.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    cineris_tools::run_blocking(&CancelToken::new(), move |_| work())
        .await
        .ok()
}

/// Finds the extensions, decides how each stands, takes what the approved
/// ones registered from the cache, and stamps what everything was found in.
/// Blocks.
fn decide(inputs: &Inputs, project_found: Option<Discovered>) -> Decided {
    let mut decided = Decided::default();
    let user = inputs
        .config_dir
        .as_ref()
        .map(|config| discover::discover(&[Root::user(config)], &inputs.resolver));
    // The project's, while it is trusted: what the trust question read of
    // it, or a look now.
    let project = inputs
        .project
        .as_ref()
        .map(|_| project_found.unwrap_or_else(|| in_project(&inputs.resolver)));
    decided.watched = Watched::stamp(
        places(inputs, user.as_ref(), project.as_ref()),
        &inputs.clock,
    );
    for discovered in [&user, &project].into_iter().flatten() {
        decided.told.extend(
            discovered
                .left_out
                .iter()
                .map(|left| sentence(&left.reason)),
        );
    }
    let users = UserNames::of(user.as_ref());
    let mine: Vec<(String, String)> = user
        .iter()
        .flat_map(|user| &user.found)
        .map(|found| {
            (
                found.manifest.name.to_ascii_lowercase(),
                found.shown.clone(),
            )
        })
        .collect();
    let mut pins: Option<Option<UserPins>> = None;
    let mut trust: Option<ProjectTrust> = None;
    let mut standing_approved = Vec::new();
    let found = user
        .into_iter()
        .chain(project)
        .flat_map(|discovered| discovered.found);
    for found in found {
        let name = visible(&found.manifest.name).into_owned();
        let definition = definition(&found);
        let standing = match found.source {
            Source::User => {
                // Turned off by the user: not loaded, and nothing asked.
                if inputs.disabled.contains(&found.manifest.name) {
                    continue;
                }
                let Some(pins) = pins
                    .get_or_insert_with(|| user_pins(inputs, &mut decided.told))
                    .as_ref()
                else {
                    decided.told.push(format!(
                        "Your extension `{name}` (`{}`) is not loaded: the pin store, which \
                         holds what you approved of your own, could not be read (said above).",
                        found.shown
                    ));
                    continue;
                };
                pins.standing(&definition)
            }
            Source::Project => {
                if users.takes(&found.manifest.name) {
                    decided.told.push(taken(&found, &name, &mine, users.cut()));
                    continue;
                }
                project_standing(
                    trust.get_or_insert_with(|| project_trust(inputs)),
                    &inputs.for_the_run,
                    &definition,
                )
            }
        };
        if standing == Standing::Approved {
            standing_approved.push(found);
        } else {
            decided
                .told
                .push(not_loaded(&found, &name, standing, inputs.trust_project));
        }
    }
    if standing_approved.is_empty() {
        return decided;
    }
    // Nothing is approved without a pin, nor pinned without a state
    // directory: a yes the store could not keep approves for the session,
    // and leaves nothing to keep.
    let Some(state) = &inputs.state else {
        decided.missing = standing_approved;
        return decided;
    };
    let cache = RegistrationCache::new(state.registration_cache_file())
        .with_clock(Arc::clone(&inputs.clock));
    let loaded = cache.load();
    if let Some(why) = &loaded.set_aside {
        let file = cache.path().to_path_buf();
        let aside = cache.set_aside_path();
        decided.told.push(sentence(&with_paths_shown(
            why,
            &[&file, &aside],
            inputs.home.as_deref(),
        )));
    }
    for found in standing_approved {
        let hash = found.snapshot.hash();
        match loaded.cache.get(&hash) {
            // Keyed by the hash of the folder, whose manifest names it: an
            // entry of another name is no entry of this one.
            Some(entry) if entry.name == found.manifest.name => {
                decided.approved.push(ApprovedExtension {
                    source: found.source,
                    folder: found.folder,
                    hash,
                    manifest: found.manifest,
                    registration: entry.registration.clone(),
                });
            }
            _ => decided.missing.push(found),
        }
    }
    decided.cache = Some(cache);
    decided
}

/// What everything was found in, in an order in which each folder comes
/// before what is in it: the user's folder of extensions, the names on the
/// way to the project's (no further than a link or a name that is not
/// there), each extension's folder, the folders in it and its files, and
/// each extension folder left out. Blocks.
#[expect(
    clippy::disallowed_methods,
    reason = "a look at each name on the way to the project's folder, on the blocking pool"
)]
fn places(
    inputs: &Inputs,
    user: Option<&Discovered>,
    project: Option<&Discovered>,
) -> Vec<PathBuf> {
    let mut places = Vec::new();
    // The stores of the pins: an approval made, or taken back, in another
    // process is a change.
    if let Some(state) = &inputs.state {
        places.push(state.pins_file());
        places.push(state.trust_file());
    }
    if let Some(config) = &inputs.config_dir {
        let looked = user.and_then(|user| {
            user.roots
                .iter()
                .find(|(source, _)| *source == Source::User)
                .map(|(_, real)| real.clone())
        });
        let root = Root::user(config).dir;
        let place = looked.or_else(|| {
            if root.is_absolute() {
                Some(root.clone())
            } else {
                let text = root.to_str()?;
                inputs
                    .resolver
                    .resolve(text)
                    .ok()
                    .map(|resolved| resolved.to_fs_path())
            }
        });
        places.extend(place);
    }
    if project.is_some() {
        // The names on the way, each looked at itself: one that is a link,
        // or not there, is the last (a look through it would follow it).
        let mut at = inputs.resolver.root().to_fs_path();
        let mut way = vec![at.clone()];
        for name in discover::PROJECT_FOLDER {
            at.push(name);
            way.push(at.clone());
            match std::fs::symlink_metadata(&at) {
                Ok(metadata) if metadata.is_dir() => {}
                _ => break,
            }
        }
        // The workspace itself only while it has no `.cineris`: creating
        // one is a change to it.
        if way.len() > 2 || std::fs::symlink_metadata(&way[1]).is_ok() {
            way.remove(0);
        }
        places.extend(way);
    }
    for discovered in [user, project].into_iter().flatten() {
        for found in &discovered.found {
            places.push(found.folder.clone());
            let mut folders: Vec<PathBuf> = Vec::new();
            let mut files: Vec<PathBuf> = Vec::new();
            for path in found.snapshot.paths() {
                let names: Vec<&str> = path.split('/').collect();
                let mut inner = found.folder.clone();
                for (index, name) in names.iter().enumerate() {
                    inner.push(name);
                    if index + 1 < names.len() {
                        if !folders.contains(&inner) {
                            folders.push(inner.clone());
                        }
                    } else {
                        files.push(inner.clone());
                    }
                }
            }
            folders.sort();
            places.extend(folders);
            places.extend(files);
        }
        for left in &discovered.left_out {
            if !places.contains(&left.folder) {
                places.push(left.folder.clone());
            }
        }
    }
    places
}

/// How a project's extension stands: approved for this session by a yes
/// the store could not keep, by the hash shown; otherwise as the project's
/// entry in the store pins it (approved for the session at another hash is
/// changed).
fn project_standing(
    trust: &ProjectTrust,
    for_the_run: &[Definition],
    definition: &Definition,
) -> Standing {
    if for_the_run.contains(definition) {
        return Standing::Approved;
    }
    match trust.standing(definition) {
        Standing::Approved => Standing::Approved,
        _ if for_the_run.iter().any(|run| run.name == definition.name) => Standing::Changed,
        other => other,
    }
}

/// The user's own pins. `None` when the store cannot be read: nothing is
/// taken as pinned, and the session is told why, with the file under `~`.
fn user_pins(inputs: &Inputs, told: &mut Vec<String>) -> Option<UserPins> {
    let Some(state) = &inputs.state else {
        return Some(UserPins::default());
    };
    let file = state.pins_file();
    match PinStore::new(&file, Arc::clone(&inputs.clock)).pins() {
        Ok(pins) => Some(pins),
        Err(error) => {
            told.push(sentence(&with_paths_shown(
                &error.to_string(),
                &[&file],
                inputs.home.as_deref(),
            )));
            None
        }
    }
}

/// The project's trust as the store holds it. A store that cannot be read
/// trusts nothing: the session was told so when it was put together.
fn project_trust(inputs: &Inputs) -> ProjectTrust {
    match (&inputs.state, &inputs.project) {
        (Some(state), Some(key)) => TrustStore::new(state.trust_file(), Arc::clone(&inputs.clock))
            .project(key)
            .unwrap_or_default(),
        _ => ProjectTrust::untrusted(),
    }
}

/// What the session is told of a project's extension whose name a folder
/// of the user's takes.
fn taken(found: &Found, name: &str, mine: &[(String, String)], cut: bool) -> String {
    let folded = found.manifest.name.to_ascii_lowercase();
    let shown = &found.shown;
    match mine.iter().find(|(user, _)| *user == folded) {
        None if cut => format!(
            "This project's extension `{name}` (`{shown}`) is not used: your own folder of \
             extensions could not be listed whole (said above), so any name may be yours."
        ),
        Some((_, theirs)) => format!(
            "This project's extension `{name}` (`{shown}`) is not used: your own extension \
             (`{theirs}`) has its name, and the name is yours."
        ),
        None => format!(
            "This project's extension `{name}` (`{shown}`) is not used: a folder of your own \
             extensions has its name, though it was left out (said above), and the name is \
             yours."
        ),
    }
}

/// What the session is told of an extension it does not load, and how to
/// approve it.
fn not_loaded(found: &Found, name: &str, standing: Standing, trust_project: bool) -> String {
    let shown = &found.shown;
    // `--trust-project` puts no question: the command approves.
    let project_remedy = if trust_project {
        "`cineris permissions trust` shows it, to approve it."
    } else {
        "The project's trust question lists it, to approve it, when a session opens."
    };
    match (found.source, standing) {
        (Source::User, Standing::Changed) => format!(
            "Your extension `{name}` (`{shown}`) has changed since you approved it, so it is not \
             loaded. `cineris ext add {shown}` shows it as it is now, to approve it."
        ),
        (Source::User, _) => format!(
            "Your extension `{name}` (`{shown}`) is not loaded: you have not approved it. \
             `cineris ext add {shown}` shows it, to approve it."
        ),
        (Source::Project, Standing::Changed) => format!(
            "This project's extension `{name}` (`{shown}`) has changed since you approved it, so \
             it is not loaded. {project_remedy}"
        ),
        (Source::Project, _) => format!(
            "This project's extension `{name}` (`{shown}`) is not loaded: you have not approved \
             it. {project_remedy}"
        ),
    }
}

impl Kept {
    /// Writes what the session registered into the cache, then prunes the
    /// cache of the hashes no pin names. Blocks.
    fn keep(self) {
        for (hash, entry) in self.written {
            // Not kept, the extension is registered again by the next
            // session: it fails closed.
            if let Err(error) = self.cache.put(&hash, entry) {
                tracing::warn!(%error, "an extension's registration was not kept");
            }
        }
        // Read after the writes, so that a pin made meanwhile is kept.
        let Some(pinned) = pinned(&self.state, &self.clock) else {
            return;
        };
        if let Err(error) = self.cache.prune(|hash| pinned.contains(hash)) {
            tracing::warn!(%error, "the registration cache was not pruned");
        }
    }
}

/// Every hash a pin of an extension names: the user's own and every
/// trusted project's, whose cache this is too. `None` when either store
/// cannot be read: nothing is pruned then.
fn pinned(state: &StateDir, clock: &SharedClock) -> Option<Vec<ContentHash>> {
    let kind = TrustResourceKind::Extension;
    let mine = PinStore::new(state.pins_file(), Arc::clone(clock))
        .pins()
        .ok()?;
    let mut pinned: Vec<ContentHash> = mine
        .list()
        .iter()
        .filter(|pin| pin.kind.as_str() == kind.as_str())
        .map(|pin| pin.hash)
        .collect();
    pinned.extend(
        TrustStore::new(state.trust_file(), Arc::clone(clock))
            .pinned_hashes(&kind)
            .ok()?,
    );
    Some(pinned)
}
