//! The extensions a session loads (`docs/architecture/12-extensions.md`,
//! "Tier 4: script extensions"; `docs/steps/28/answered.md`, points 4 and
//! 5): found in the user's folder of extensions and, in a project the user
//! trusts, the project's; each one's definition judged against its pin; and
//! what each approved one registered, taken from the registration cache.
//!
//! # What is looked in
//!
//! - **The user's folder**, `<config dir>/extensions`, given as the
//!   configuration directory is given: a plain path, whose `~`, `..`, link
//!   or relative form the session's path resolver takes
//!   (`cineris_ext::discover`).
//! - **The project's**, `.cineris/extensions` under the workspace's real
//!   location, **only while the project is trusted** (in the trust store,
//!   or for the run). A session of a project that is not trusted does not
//!   look in it at all; only the trust question does, to list what a yes
//!   would trust ([`asked`]).
//!
//! # How each stands
//!
//! | Found | Its pin | In the session |
//! |---|---|---|
//! | The user's | in `pins.toml`, with its hash | loaded |
//! | The user's | in `pins.toml`, with another hash | not loaded; the session says so, with `cineris ext add <folder>` |
//! | The user's | none | the same |
//! | The user's, `[extensions.<name>] enabled = false` | any | not loaded, and nothing is said or asked |
//! | The project's | in the project's entry of `trust.toml`, with its hash | loaded |
//! | The project's | with another hash, or none | not loaded; the session says so, and the trust question lists it |
//! | The project's, of a name one of the user's has | any | not used; the session says so |
//!
//! # Everything is decided before anything runs
//!
//! The folders are read, the pins and the trust store read, every standing
//! decided and the registration cache read, before any script is evaluated.
//! Then only an extension that stands approved and whose registration the
//! cache does not hold is evaluated: once, in a throwaway runtime
//! (`cineris_ext::engine::register`), and its registration written to the
//! cache. **A missing entry fails closed**: the extension is registered
//! then, rather than taken as registering nothing. No runtime of an
//! extension's is made for the session before its first call
//! (`super::tools`).
//!
//! # Where it runs, and what waits for it
//!
//! A task the core spawns once the session's loop runs ([`start`]), off the
//! path that opens the session: the reads and writes on the blocking pool,
//! each evaluation on a thread of its own. **The session's first turn
//! waits for it:** `turn/start` waits for [`Ready`] before it hands its
//! prompt to the loop, where it already waits for the look at the
//! configuration's files, so that no turn starts while an approved
//! extension's registration is missing. With no extension the task costs a
//! look at each folder that is not there.
//!
//! # The cache, pruned
//!
//! A cache set aside (unreadable, or of another version) is said once, and
//! every approved extension is registered again into a new one. Once the
//! session's own entries are written, the cache is pruned of the hashes no
//! pin names: the user's own pins and **every trusted project's**, since
//! the cache is shared by every project. When either store cannot be read,
//! nothing is pruned.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cineris_base::{PathResolver, visible};
use cineris_ext::discover::{self, Discovered, Found, Root};
use cineris_ext::manifest::{HostPattern, Manifest};
use cineris_policy::{Definition, ProjectTrust, Standing};
use cineris_protocol::methods::{TrustResource, TrustResourceKind};

#[cfg(feature = "scripts")]
pub(crate) use pipeline::Finding;

/// Where an approved extension that the registration cache does not hold
/// is about to be evaluated, to register it: a point a test holds the
/// session's finding of its extensions at, to see that the first turn
/// waits (`cineris_base::hook`).
pub const BEFORE_AN_EXTENSION_IS_REGISTERED: &str =
    "ext: an approved extension missing from the registration cache is about to be registered";

/// Whether a session's extensions are settled: found, judged, registered
/// where the cache missed one, and their tools in the session's registry.
/// The first turn waits for it.
#[derive(Debug, Default)]
pub(crate) struct Ready {
    done: AtomicBool,
    settled: tokio::sync::Notify,
}

impl Ready {
    /// Settled already: a session with nothing to find.
    pub(crate) fn settled() -> Arc<Self> {
        let ready = Self::default();
        ready.done.store(true, Ordering::SeqCst);
        Arc::new(ready)
    }

    #[cfg(feature = "scripts")]
    fn settle(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.settled.notify_waiters();
    }

    /// Waits until the extensions are settled: at once when they are.
    pub(crate) async fn wait(&self) {
        loop {
            // Made before the look, so that a settling in between wakes it.
            let settled = self.settled.notified();
            if self.done.load(Ordering::SeqCst) {
                return;
            }
            settled.await;
        }
    }
}

/// Settles a [`Ready`] when it is dropped: when the work ends, however it
/// ends (done, cancelled, or a panic), so that a turn never waits for work
/// that is gone. What did not get registered is then not loaded.
#[cfg(feature = "scripts")]
struct Settles(Arc<Ready>);

#[cfg(feature = "scripts")]
impl Drop for Settles {
    fn drop(&mut self) {
        self.0.settle();
    }
}

/// What a session put together finds once its loop runs: [`start`]ed by
/// the core. `None` when there is nothing to look in.
#[cfg(feature = "scripts")]
pub(crate) type Pending = Option<Finding>;

/// Without the script engine, no extension's tool is registered, and there
/// is nothing to find.
#[cfg(not(feature = "scripts"))]
pub(crate) type Pending = ();

/// Starts finding a session's extensions, on a task of its own: what the
/// session's first turn waits for.
#[cfg(feature = "scripts")]
pub(crate) fn start(pending: Pending) -> Arc<Ready> {
    let Some(finding) = pending else {
        return Ready::settled();
    };
    let ready = Arc::new(Ready::default());
    let settles = Settles(Arc::clone(&ready));
    tokio::spawn(async move {
        let _settles = settles;
        let cancel = finding.cancel.clone();
        let _ = cancel.race(finding.run()).await;
    });
    ready
}

/// Without the script engine there is nothing to find.
#[cfg(not(feature = "scripts"))]
pub(crate) fn start(_pending: Pending) -> Arc<Ready> {
    Ready::settled()
}

/// The definition a found extension is pinned by: its kind, its name, and
/// the hash of its folder read whole, which is what was approved.
pub(crate) fn definition(found: &Found) -> Definition {
    Definition {
        kind: TrustResourceKind::Extension,
        name: found.manifest.name.clone(),
        hash: found.snapshot.hash(),
    }
}

/// The project's folder of extensions, looked in: `.cineris/extensions`
/// under the workspace's real location. Blocks.
pub(crate) fn in_project(resolver: &PathResolver) -> Discovered {
    discover::discover(&[Root::project(&resolver.root().to_fs_path())], resolver)
}

/// What the trust question lists of a project's extensions, and the
/// definitions a yes pins.
#[derive(Debug, Default)]
pub(crate) struct Asked {
    /// One resource of kind `extension` each.
    pub(crate) resources: Vec<TrustResource>,
    /// The definitions listed, as they were read: a yes pins these, not
    /// the folders as they are when the answer comes.
    pub(crate) pins: Vec<Definition>,
}

/// What the trust question lists of `project`, the project's extensions
/// found ([`in_project`]), for a project whose trust is `stored`: each one
/// not approved as it is now, in words, `changed` when it was approved with
/// another hash. One whose name one of the user's own extensions has is
/// not listed: it is not used whatever the answer, and the session says
/// so. Blocks: the user's folder is looked in when the project has any.
pub(crate) fn asked(
    project: Discovered,
    resolver: &PathResolver,
    config_dir: Option<&Path>,
    stored: &ProjectTrust,
) -> Asked {
    let mut asked = Asked::default();
    if project.found.is_empty() {
        return asked;
    }
    let users: Vec<String> = config_dir
        .map(|config| {
            discover::discover(&[Root::user(config)], resolver)
                .found
                .into_iter()
                .map(|found| found.manifest.name)
                .collect()
        })
        .unwrap_or_default();
    for found in project.found {
        if users.contains(&found.manifest.name) {
            continue;
        }
        let definition = definition(&found);
        let changed = match stored.standing(&definition) {
            Standing::Approved => continue,
            Standing::Changed => true,
            Standing::New | Standing::Untrusted => false,
        };
        asked.resources.push(TrustResource {
            kind: TrustResourceKind::Extension,
            name: visible(&found.manifest.name).into_owned(),
            detail: format!("In `{}`. {}", found.shown, in_words(&found.manifest)),
            changed,
        });
        asked.pins.push(definition);
    }
    asked
}

/// What an extension's manifest lets it do, in words for the person asked:
/// its version and description, the programs it may run (and whether they
/// may reach the network), the files it may read and write, the hosts it
/// may reach, what each event it may hook lets it see, and whether it adds
/// to the system prompt. Every text of the manifest's is shown with what
/// acts unseen escaped ([`visible`]).
pub(crate) fn in_words(manifest: &Manifest) -> String {
    let listed = |items: &[String]| {
        items
            .iter()
            .map(|item| format!("`{}`", visible(item)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let declared = &manifest.capabilities;
    let mut words = Vec::new();
    let description = visible(manifest.description.trim());
    let version = visible(&manifest.version);
    words.push(if description.is_empty() {
        format!("Version {version}.")
    } else {
        format!("Version {version}, described by its author as: {description}")
    });
    let programs: Vec<String> = declared
        .exec
        .iter()
        .map(|program| program.as_str().to_owned())
        .collect();
    words.push(if programs.is_empty() {
        "It runs no program.".to_owned()
    } else {
        format!(
            "It may run {}, {}.",
            listed(&programs),
            if declared.exec_network {
                "which may reach the network"
            } else {
                "which may not reach the network"
            }
        )
    });
    words.push(if declared.read.is_empty() {
        "It reads no file.".to_owned()
    } else {
        format!(
            "It may read the workspace's files that match {}.",
            listed(&declared.read)
        )
    });
    words.push(if declared.write.is_empty() {
        "It writes no file.".to_owned()
    } else {
        format!(
            "It may write the workspace's files that match {}.",
            listed(&declared.write)
        )
    });
    words.push(if declared.net.is_empty() {
        "It reaches no host.".to_owned()
    } else {
        let hosts: Vec<String> = declared
            .net
            .iter()
            .map(|host| match host {
                HostPattern::Any => "any host".to_owned(),
                HostPattern::Under(domain) => format!("`*{}`", visible(domain)),
                HostPattern::Exact(name) => format!("`{}`", visible(name)),
            })
            .collect();
        format!("It may reach {}.", hosts.join(", "))
    });
    words.push(if declared.hooks.is_empty() {
        "It hooks no event.".to_owned()
    } else {
        let sees: Vec<String> = declared
            .hooks
            .iter()
            .map(|event| format!("{} (`{}`)", event.sees(), event.as_str()))
            .collect();
        format!("It may see {}.", sees.join("; "))
    });
    if manifest.prompt.is_some() {
        words.push("It adds text of its author's to the system prompt.".to_owned());
    }
    words.join(" ")
}

/// A reason the extension module wrote as a clause (`the extension folder
/// … was left out: …`), as a sentence for a notice.
#[cfg(feature = "scripts")]
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut said: String = chars
        .next()
        .map(|first| first.to_uppercase().collect())
        .unwrap_or_default();
    said.push_str(chars.as_str());
    if !said.ends_with('.') {
        said.push('.');
    }
    said
}

#[cfg(feature = "scripts")]
mod pipeline {
    use std::path::PathBuf;
    use std::sync::Arc;

    use cineris_base::{CancelToken, ContentHash, PathResolver, SharedClock, visible};
    use cineris_config::StateDir;
    use cineris_ext::cache::{Entry, RegistrationCache};
    use cineris_ext::discover::{self, Found, Root};
    use cineris_ext::source::Source;
    use cineris_policy::{PinStore, ProjectTrust, Standing, TrustStore, UserPins};
    use cineris_protocol::ProjectKey;
    use cineris_protocol::methods::TrustResourceKind;
    use cineris_tools::ToolRegistry;

    use super::{BEFORE_AN_EXTENSION_IS_REGISTERED, definition, sentence};
    use crate::ext::{ApprovedExtension, HostParts};

    /// What a session finds its extensions with.
    pub(crate) struct Finding {
        /// The session's path resolver: its workspace, and the home
        /// directory `~` stands for.
        pub(crate) resolver: Arc<PathResolver>,
        /// The configuration directory, as it was given: the user's folder
        /// of extensions is in it.
        pub(crate) config_dir: Option<PathBuf>,
        /// The state directory: the pins, the trust store and the cache.
        pub(crate) state: Option<StateDir>,
        /// The project's key, when the project is trusted (in the store, or
        /// for the run): its folder of extensions is looked in.
        pub(crate) project: Option<ProjectKey>,
        /// The names the user's configuration turns off
        /// (`[extensions.<name>] enabled = false`).
        pub(crate) disabled: Vec<String>,
        /// Where the approved extensions' tools go.
        pub(crate) registry: Arc<ToolRegistry>,
        /// What their calls are served with, and how the session is told.
        pub(crate) parts: Arc<HostParts>,
        /// Ends the work when the core is ending.
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
    }

    /// What the blocking look is given.
    struct Inputs {
        resolver: Arc<PathResolver>,
        config_dir: Option<PathBuf>,
        state: Option<StateDir>,
        project: Option<ProjectKey>,
        disabled: Vec<String>,
        clock: SharedClock,
    }

    impl Finding {
        /// Finds, judges, registers what the cache misses, and puts the
        /// approved extensions' tools in the registry.
        pub(super) async fn run(self) {
            let tell = Arc::clone(&self.parts.tell);
            let inputs = Inputs {
                resolver: Arc::clone(&self.resolver),
                config_dir: self.config_dir.clone(),
                state: self.state.clone(),
                project: self.project.clone(),
                disabled: self.disabled.clone(),
                clock: Arc::clone(&self.parts.clock),
            };
            let Some(decided) = blocking(move || decide(&inputs)).await else {
                return;
            };
            for message in decided.told {
                tell(message);
            }
            let mut approved = decided.approved;
            let mut written: Vec<(ContentHash, Entry)> = Vec::new();
            for found in decided.missing {
                cineris_base::hook::await_point!(BEFORE_AN_EXTENSION_IS_REGISTERED);
                let hash = found.snapshot.hash();
                let name = visible(&found.manifest.name).into_owned();
                match cineris_ext::engine::register(
                    Arc::clone(&found.snapshot),
                    self.parts.timer.as_ref(),
                )
                .await
                {
                    Ok((manifest, registration)) => {
                        // The prompt's text as the runtime reads it: from
                        // the snapshot that was evaluated.
                        let prompt = manifest
                            .prompt
                            .as_deref()
                            .and_then(|file| found.snapshot.file(file))
                            .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
                        written.push((hash, Entry::new(&manifest, registration.clone(), prompt)));
                        approved.push(ApprovedExtension {
                            source: found.source,
                            folder: found.folder,
                            hash,
                            manifest,
                            registration,
                        });
                    }
                    Err(error) => tell(format!(
                        "The extension `{name}` (`{}`) could not be registered, so it is not \
                         loaded: {error}",
                        found.shown
                    )),
                }
            }
            if let (Some(cache), Some(state)) = (decided.cache, self.state.clone()) {
                let clock = Arc::clone(&self.parts.clock);
                let _ = blocking(move || keep(&cache, written, &state, &clock)).await;
            }
            for message in crate::ext::register(approved, &self.parts, &self.registry) {
                tell(message);
            }
        }
    }

    /// Runs `work` on the blocking pool. `None` when it could not finish.
    async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        cineris_tools::run_blocking(&CancelToken::new(), move |_| work())
            .await
            .ok()
    }

    /// Finds the extensions, decides how each stands, and takes what the
    /// approved ones registered from the cache. Blocks.
    fn decide(inputs: &Inputs) -> Decided {
        let mut decided = Decided::default();
        let mut roots = Vec::new();
        if let Some(config) = &inputs.config_dir {
            roots.push(Root::user(config));
        }
        if inputs.project.is_some() {
            roots.push(Root::project(&inputs.resolver.root().to_fs_path()));
        }
        let discovered = discover::discover(&roots, &inputs.resolver);
        decided.told.extend(
            discovered
                .left_out
                .iter()
                .map(|left| sentence(&left.reason)),
        );
        if discovered.found.is_empty() {
            return decided;
        }
        // The user's own names: a project's extension of one is not used.
        let users: Vec<(String, String)> = discovered
            .found
            .iter()
            .filter(|found| found.source == Source::User)
            .map(|found| (found.manifest.name.clone(), found.shown.clone()))
            .collect();
        let mut pins: Option<UserPins> = None;
        let mut trust: Option<ProjectTrust> = None;
        let mut standing_approved = Vec::new();
        for found in discovered.found {
            let name = visible(&found.manifest.name).into_owned();
            let definition = definition(&found);
            let standing = match found.source {
                Source::User => {
                    // Turned off by the user: not loaded, and nothing asked.
                    if inputs.disabled.contains(&found.manifest.name) {
                        continue;
                    }
                    pins.get_or_insert_with(|| user_pins(inputs, &mut decided.told))
                        .standing(&definition)
                }
                Source::Project => {
                    if let Some((_, mine)) =
                        users.iter().find(|(user, _)| *user == found.manifest.name)
                    {
                        decided.told.push(format!(
                            "This project's extension `{name}` (`{}`) is not used: your own \
                             extension `{name}` (`{mine}`) has its name, and the name is yours.",
                            found.shown
                        ));
                        continue;
                    }
                    trust
                        .get_or_insert_with(|| project_trust(inputs))
                        .standing(&definition)
                }
            };
            if standing == Standing::Approved {
                standing_approved.push(found);
            } else {
                decided.told.push(not_loaded(&found, &name, standing));
            }
        }
        if standing_approved.is_empty() {
            return decided;
        }
        // Nothing is approved without a pin, nor pinned without a state
        // directory.
        let Some(state) = &inputs.state else {
            return decided;
        };
        let cache = RegistrationCache::new(state.registration_cache_file())
            .with_clock(Arc::clone(&inputs.clock));
        let loaded = cache.load();
        if let Some(why) = &loaded.set_aside {
            decided.told.push(sentence(why));
        }
        for found in standing_approved {
            let hash = found.snapshot.hash();
            match loaded.cache.get(&hash) {
                // Keyed by the hash of the folder, whose manifest names it:
                // an entry of another name is no entry of this one.
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

    /// The user's own pins. A store that cannot be read pins nothing, and
    /// the session is told why.
    fn user_pins(inputs: &Inputs, told: &mut Vec<String>) -> UserPins {
        let Some(state) = &inputs.state else {
            return UserPins::default();
        };
        match PinStore::new(state.pins_file(), Arc::clone(&inputs.clock)).pins() {
            Ok(pins) => pins,
            Err(error) => {
                told.push(sentence(&error.to_string()));
                UserPins::default()
            }
        }
    }

    /// The project's trust as the store holds it. A store that cannot be
    /// read trusts nothing: the session was told so when it was put
    /// together.
    fn project_trust(inputs: &Inputs) -> ProjectTrust {
        match (&inputs.state, &inputs.project) {
            (Some(state), Some(key)) => {
                TrustStore::new(state.trust_file(), Arc::clone(&inputs.clock))
                    .project(key)
                    .unwrap_or_default()
            }
            _ => ProjectTrust::untrusted(),
        }
    }

    /// What the session is told of an extension it does not load, and how
    /// to approve it.
    fn not_loaded(found: &Found, name: &str, standing: Standing) -> String {
        let shown = &found.shown;
        match (found.source, standing) {
            (Source::User, Standing::Changed) => format!(
                "Your extension `{name}` (`{shown}`) has changed since you approved it, so it is \
                 not loaded. `cineris ext add {shown}` shows it as it is now, to approve it."
            ),
            (Source::User, _) => format!(
                "Your extension `{name}` (`{shown}`) is not loaded: you have not approved it. \
                 `cineris ext add {shown}` shows it, to approve it."
            ),
            (Source::Project, Standing::Changed) => format!(
                "This project's extension `{name}` (`{shown}`) has changed since you approved it, \
                 so it is not loaded. The project's trust question lists the change, to approve \
                 it, when a session opens."
            ),
            (Source::Project, _) => format!(
                "This project's extension `{name}` (`{shown}`) is not loaded: you have not \
                 approved it. The project's trust question lists it, to approve it, when a \
                 session opens."
            ),
        }
    }

    /// Writes what the session registered into the cache, then prunes the
    /// cache of the hashes no pin names. Blocks.
    fn keep(
        cache: &RegistrationCache,
        written: Vec<(ContentHash, Entry)>,
        state: &StateDir,
        clock: &SharedClock,
    ) {
        for (hash, entry) in written {
            // Not kept, the extension is registered again by the next
            // session: it fails closed.
            if let Err(error) = cache.put(&hash, entry) {
                tracing::warn!(%error, "an extension's registration was not kept");
            }
        }
        // Read after the writes, so that a pin made meanwhile is kept.
        let Some(pinned) = pinned(state, clock) else {
            return;
        };
        if let Err(error) = cache.prune(|hash| pinned.contains(hash)) {
            tracing::warn!(%error, "the registration cache was not pruned");
        }
    }

    /// Every hash a pin of an extension names: the user's own and every
    /// trusted project's, whose cache this is too. `None` when either
    /// store cannot be read: nothing is pruned then.
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
}

#[cfg(all(test, feature = "scripts"))]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_said_as_a_sentence() {
        assert_eq!(
            sentence("the folder `x` was left out"),
            "The folder `x` was left out."
        );
        assert_eq!(sentence("done."), "Done.");
        assert_eq!(sentence(""), ".");
    }
}
