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
//!   would trust ([`asked`]), and what it read is handed on to the session
//!   rather than read again.
//!
//! # How each stands
//!
//! | Found | Its pin | In the session |
//! |---|---|---|
//! | The user's | in `pins.toml`, with its hash | loaded |
//! | The user's | in `pins.toml`, with another hash, or none | not loaded; the session says so, with `cineris ext add <folder>` |
//! | The user's, with `pins.toml` unreadable | any | not loaded; the session says the store is why |
//! | The user's, `[extensions.<name>] enabled = false` | any | not loaded, and nothing is said or asked |
//! | The project's | in the project's entry of `trust.toml`, with its hash, or for the session by a yes the store could not keep | loaded |
//! | The project's | with another hash, or none | not loaded; the session says so, with the trust question, or `cineris permissions trust` under `--trust-project` |
//! | The project's, of a name a folder of the user's has | any | not used; the session says so |
//!
//! **A name is the user's** when the user's folder of extensions holds a
//! folder of that name, with ASCII's case folded, whether or not that
//! folder could be used: a user's extension left out does not free its
//! name for a project's, to which the user's rules about it would then
//! apply. A user's folder that could not be listed whole takes every name.
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
//! waits for it:** `turn/start` waits for [`Extensions::wait`] before it
//! hands its prompt to the loop, where it already waits for the look at
//! the configuration's files, so that no turn starts while an approved
//! extension's registration is missing. It waits for the tools, not for
//! the cache: the cache is written and pruned after the turn may start. A
//! session closed while the task runs ends it.
//!
//! # A change, at the next turn
//!
//! What was found is stamped (`super::watched`), and at each turn's start
//! ([`Extensions::recheck`]) the stamps are looked at again: a change has
//! every folder read and judged again. An extension that no longer stands
//! approved as it was loaded is withdrawn (its tools leave the registry,
//! and the session is told); one that now stands approved is loaded then,
//! before the turn starts, registered first if the cache misses it.
//!
//! # The cache, pruned
//!
//! A cache set aside (unreadable, or of another version) is said once, and
//! every approved extension is registered again into a new one. Once the
//! session's own entries are written, the cache is pruned of the hashes no
//! pin names: the user's own pins and **every trusted project's**, since
//! the cache is shared by every project. When either store cannot be read,
//! nothing is pruned.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use cineris_base::{
    ContentHash, PathResolver, SharedClock, in_code, strip_location_prefix, visible,
};
use cineris_ext::discover::{self, Discovered, Found, LeftOut, Root, Seen};
use cineris_ext::folder::Snapshot;
use cineris_ext::manifest::{HostPattern, Manifest};
use cineris_ext::source::Source;
use cineris_policy::{Definition, ProjectTrust, Standing};
use cineris_protocol::methods::{TrustResource, TrustResourceKind};

use super::watched::Watched;

#[cfg(not(feature = "scripts"))]
pub(crate) use super::without_engine::SessionView;
#[cfg(feature = "scripts")]
pub(crate) use pipeline::{Finding, SessionView, start};

impl crate::Core {
    /// Ends the session `session`'s finding of its extensions where it
    /// stands, as the end of its work before it is done would (a panic in
    /// it): for the test that a session whose finding failed starts no turn.
    ///
    /// # Errors
    ///
    /// When the session is not open.
    #[doc(hidden)]
    pub fn end_the_finding_of(
        &self,
        session: &cineris_protocol::SessionId,
    ) -> Result<(), crate::OpError> {
        self.live(session)?.extensions.stop();
        Ok(())
    }
}

/// Where an approved extension that the registration cache does not hold
/// is about to be evaluated, to register it: a point a test holds the
/// session's finding of its extensions at, to see that the first turn
/// waits (`cineris_base::hook`).
pub const BEFORE_AN_EXTENSION_IS_REGISTERED: &str =
    "ext: an approved extension missing from the registration cache is about to be registered";

/// Where a session's extensions are settled, and the registration cache is
/// about to be written and pruned: a point a test holds to see that a turn
/// does not wait for the cache.
pub const BEFORE_THE_CACHE_IS_KEPT: &str =
    "ext: the session's extensions are settled, and the registration cache is not yet written";

/// Where the trust question has read the project's trust, and not yet
/// shaped the question from it: a point a test holds to change the store
/// there (`cineris_base::hook`).
pub const THE_QUESTIONS_TRUST_IS_READ: &str =
    "ext: the trust question has read the project's trust, and is not yet shaped";

/// How long a turn waits for the session's extensions before the session
/// says what it waits for (the full review of step 28).
pub const WAIT_SAID_AFTER: Duration = Duration::from_secs(3);

/// What the session is told when a turn has waited [`WAIT_SAID_AFTER`] for
/// its extensions.
pub const WAITING: &str = "This turn waits for the session's extensions to be found and \
                           registered, so that a gate one of them holds is not skipped. It \
                           starts once they are.";

/// What the session is told when a turn has waited [`WAIT_SAID_AFTER`] for
/// the `session.start` hooks of its extensions, which the first turn waits
/// for once they are found.
pub const WAITING_FOR_HOOKS: &str = "This turn waits for the `session.start` hooks of the \
                                     session's extensions, which are heard before its first \
                                     turn. It starts once each has answered, or its time is up.";

/// Whether a session's extensions are settled: found, judged, registered
/// where the cache missed one, and their tools in the session's registry;
/// or why their finding failed, which no turn passes over: a gate one of
/// them holds would be skipped.
#[derive(Debug, Default)]
struct Ready {
    done: AtomicBool,
    /// Why the finding failed, when it did: set before `done`.
    failed: std::sync::Mutex<Option<String>>,
    settled: tokio::sync::Notify,
}

impl Ready {
    fn settle(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.settled.notify_waiters();
    }

    /// The finding failed, and why: what every turn's start is told from now
    /// on. A finding that settled already stays settled.
    #[cfg_attr(
        not(feature = "scripts"),
        expect(
            dead_code,
            reason = "without the script engine nothing is found to load, so no finding fails"
        )
    )]
    fn fail(&self, why: String) {
        if self.done.load(Ordering::SeqCst) {
            return;
        }
        *self
            .failed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(why);
        self.settle();
    }

    fn is_settled(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    /// How it settled: `Err` with why the finding failed.
    fn outcome(&self) -> Result<(), String> {
        match self
            .failed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            Some(why) => Err(why.clone()),
            None => Ok(()),
        }
    }

    async fn wait(&self) -> Result<(), String> {
        loop {
            // Made before the look, so that a settling in between wakes it.
            let settled = self.settled.notified();
            if self.done.load(Ordering::SeqCst) {
                return self.outcome();
            }
            settled.await;
        }
    }
}

/// A session's extensions: what its first turn waits for, and what a turn's
/// start looks at again.
#[derive(Debug, Default)]
pub(crate) struct Extensions {
    ready: Ready,
    #[cfg(feature = "scripts")]
    held: pipeline::Shared,
    /// Without the script engine, what the session's extensions were judged
    /// from as it opened.
    #[cfg(not(feature = "scripts"))]
    view: Option<SessionView>,
}

impl Extensions {
    /// A session with nothing to find.
    #[cfg(feature = "scripts")]
    pub(crate) fn none() -> Arc<Self> {
        let extensions = Self::default();
        extensions.ready.settle();
        Arc::new(extensions)
    }

    /// Waits until the session's extensions are settled: at once when they
    /// are. A wait of more than [`WAIT_SAID_AFTER`], on the session's timer,
    /// is said, once, and goes on: a turn does not start while an approved
    /// extension's registration is missing, and each evaluation is bounded
    /// by the engine's own time.
    ///
    /// # Errors
    ///
    /// Why their finding failed: no turn starts then, since a gate one of
    /// them holds would be skipped.
    pub(crate) async fn wait(&self) -> Result<(), String> {
        if self.ready.is_settled() {
            return self.ready.outcome();
        }
        #[cfg(feature = "scripts")]
        if let Some((timer, tell)) = self.held.waiting() {
            let mut said = timer.sleep(WAIT_SAID_AFTER);
            let mut ready = std::pin::pin!(self.ready.wait());
            let done = std::future::poll_fn(|context| {
                if ready.as_mut().poll(context).is_ready() {
                    return std::task::Poll::Ready(true);
                }
                if said.as_mut().poll(context).is_ready() {
                    return std::task::Poll::Ready(false);
                }
                std::task::Poll::Pending
            })
            .await;
            if !done {
                tell(self.held.waiting_said());
                return ready.await;
            }
            return self.ready.outcome();
        }
        self.ready.wait().await
    }

    /// Looks again at what the extensions were found in, at a turn's
    /// start: a change has them read and judged again, and the turn waits
    /// for what that loads.
    ///
    /// # Errors
    ///
    /// Why their finding failed, as [`Self::wait`].
    pub(crate) async fn recheck(&self) -> Result<(), String> {
        self.wait().await?;
        #[cfg(feature = "scripts")]
        self.held.recheck().await;
        Ok(())
    }

    /// What the session's finding holds of its extensions, once it has
    /// settled: `None` before, or with nothing to find.
    #[cfg(feature = "scripts")]
    pub(crate) async fn view(&self) -> Option<pipeline::SessionView> {
        self.held.view().await
    }

    /// What the session's extensions were judged from as it opened: `None`
    /// with nothing to look in.
    #[cfg(not(feature = "scripts"))]
    pub(crate) async fn view(&self) -> Option<SessionView> {
        self.view.clone()
    }

    /// The session has ended: work still under way for it ends too.
    pub(crate) fn stop(&self) {
        #[cfg(feature = "scripts")]
        self.held.stop();
    }
}

/// What a session put together finds once its loop runs: started by the
/// core. `None` when there is nothing to look in.
#[cfg(feature = "scripts")]
pub(crate) type Pending = Option<Finding>;

/// Without the script engine no extension is loaded, and there is nothing
/// to find: what the session's extensions were judged from as it opened is
/// kept, for `extensions/status`.
#[cfg(not(feature = "scripts"))]
pub(crate) type Pending = Option<SessionView>;

/// Without the script engine there is nothing to find.
#[cfg(not(feature = "scripts"))]
pub(crate) fn start(pending: Pending) -> Arc<Extensions> {
    let extensions = Extensions {
        view: pending,
        ..Extensions::default()
    };
    extensions.ready.settle();
    Arc::new(extensions)
}

/// An extension as the trust question found it, handed to the session
/// without its bytes: where it is, how it is shown, the hash it was read
/// at, and its manifest (the early review: a project's folders were held
/// whole, up to 64 of 4 MiB each, from the question to the session's
/// registration). The session reads it again only to register it, and then
/// only at this hash.
#[cfg_attr(
    not(feature = "scripts"),
    expect(
        dead_code,
        reason = "without the script engine nothing takes the question's findings on"
    )
)]
#[derive(Debug, Clone)]
pub(crate) struct Listed {
    pub(crate) source: Source,
    pub(crate) folder: PathBuf,
    pub(crate) shown: String,
    pub(crate) hash: ContentHash,
    pub(crate) manifest: Manifest,
    /// The manifest's prompt file alone, as it was read ([`prompt_alone`]):
    /// what a cached entry is checked against when the rest of the folder's
    /// bytes are not kept.
    pub(crate) prompt: Snapshot,
}

impl Listed {
    /// What is kept of a found extension.
    pub(crate) fn of(found: &Found) -> Self {
        Self {
            source: found.source,
            folder: found.folder.clone(),
            shown: found.shown.clone(),
            hash: found.snapshot.hash(),
            manifest: found.manifest.clone(),
            prompt: prompt_alone(&found.manifest, &found.snapshot),
        }
    }

    /// The definition it is pinned by.
    #[cfg(feature = "scripts")]
    pub(crate) fn definition(&self) -> Definition {
        Definition {
            kind: TrustResourceKind::Extension,
            name: self.manifest.name.clone(),
            hash: self.hash,
        }
    }
}

/// What the trust question found in the project's folder of extensions,
/// handed to the session: each extension without its bytes, what was left
/// out, and every place stamped as it was **before** it was read, so that a
/// change made while the question was open is a change at the session's
/// first look (the full review of step 28).
#[cfg_attr(
    not(feature = "scripts"),
    expect(
        dead_code,
        reason = "without the script engine nothing takes the question's findings on"
    )
)]
#[derive(Debug, Default)]
pub(crate) struct Looked {
    pub(crate) found: Vec<Listed>,
    pub(crate) left_out: Vec<LeftOut>,
    pub(crate) watched: Watched,
}

/// The manifest's prompt file alone, out of `snapshot`, for the cache's
/// check that an entry's prompt is the file's text (`Entry::fits`, which
/// looks at nothing else of the snapshot): the rest of the folder's bytes
/// are not kept. A file longer than any prompt an entry may hold is kept
/// to one byte past that, which no entry's prompt equals, as none equals
/// the whole file.
pub(crate) fn prompt_alone(manifest: &Manifest, snapshot: &Snapshot) -> Snapshot {
    let mut files = std::collections::BTreeMap::new();
    if let Some(path) = manifest.prompt.as_deref()
        && let Some(bytes) = snapshot.file(path)
    {
        let kept = &bytes[..bytes.len().min(cineris_ext::manifest::MAX_PROMPT_BYTES + 1)];
        files.insert(path.to_owned(), kept.to_vec());
    }
    Snapshot::from_files(files)
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
/// under the workspace's real location. Each place is stamped before it is
/// looked at or read (the names on the way, as written and to where the
/// folder really is; each extension's folder, the folders in it and its
/// files), no stamp following a link. Blocks.
pub(crate) fn in_project(resolver: &PathResolver, clock: &SharedClock) -> (Discovered, Watched) {
    let workspace = resolver.root().to_fs_path();
    let mut watched = Watched::default();
    // The names on the way as written, each looked at itself: one that is a
    // link, or not there, is the last (a look through it would follow it).
    let mut at = workspace.clone();
    for name in discover::PROJECT_FOLDER {
        at.push(name);
        watched.note(at.clone(), clock);
        match stat_itself(&at) {
            Some(metadata) if metadata.is_dir() => {}
            _ => break,
        }
    }
    let mut left = super::watched::MAX_PLACES_PER_ROOT;
    let discovered = discover::discover_seeing(
        &[Root::project(&workspace)],
        resolver,
        &mut |seen| match seen {
            // Where the folder really is, each name from the workspace down
            // to it: a folder on the way swapped for a link is a change,
            // and nothing is looked at through it.
            Seen::Root(_, real) => {
                if let Some(rest) = strip_location_prefix(real, &workspace) {
                    let mut at = workspace.clone();
                    for name in rest.components() {
                        at.push(name);
                        watched.note(at.clone(), clock);
                    }
                }
                watched.note(real.to_path_buf(), clock);
                true
            }
            Seen::Extension(_, folder) => watched.note_tree(folder, clock, &mut left),
        },
    );
    for left in &discovered.left_out {
        watched.note(left.folder.clone(), clock);
    }
    (discovered, watched)
}

/// A look at `path` itself, following no link. Blocks.
#[expect(
    clippy::disallowed_methods,
    reason = "a look at one name on the way to the project's folder, on the blocking pool"
)]
fn stat_itself(path: &Path) -> Option<std::fs::Metadata> {
    std::fs::symlink_metadata(path).ok()
}

/// What the trust question found, as the session is handed it.
pub(crate) fn looked(discovered: &Discovered, watched: Watched) -> Looked {
    Looked {
        found: discovered.found.iter().map(Listed::of).collect(),
        left_out: discovered.left_out.clone(),
        watched,
    }
}

/// The names the user's folder of extensions takes, with ASCII's case
/// folded: every folder in it, whether or not it could be used.
#[derive(Debug, Default)]
pub(crate) struct UserNames {
    names: Vec<String>,
    /// The folder could not be listed whole: every name may be taken.
    cut: bool,
}

impl UserNames {
    /// The names of the user's folders, as discovery listed them.
    pub(crate) fn of(user: Option<&Discovered>) -> Self {
        let Some(user) = user else {
            return Self::default();
        };
        Self {
            names: user
                .names
                .iter()
                .filter(|(source, _)| *source == Source::User)
                .map(|(_, name)| name.to_ascii_lowercase())
                .collect(),
            cut: user.cut.contains(&Source::User),
        }
    }

    /// The user's folder of extensions, listed and read no further. Blocks.
    pub(crate) fn listed(resolver: &PathResolver, config_dir: Option<&Path>) -> Self {
        Self::listed_within(resolver, config_dir, super::watched::MAX_PLACES_PER_ROOT)
    }

    /// [`UserNames::listed`], the user's root counting as not listed whole
    /// past `bound` places within its extensions, as the session's finding
    /// counts them: the question and the session then agree on which names
    /// are the user's (the re-check of step 28's fresh review).
    pub(crate) fn listed_within(
        resolver: &PathResolver,
        config_dir: Option<&Path>,
        bound: usize,
    ) -> Self {
        let clock: SharedClock = Arc::new(cineris_base::SystemClock::new());
        let mut watched = Watched::default();
        let mut left = bound;
        let listed = config_dir.map(|config| {
            discover::names_seeing(&[Root::user(config)], resolver, &mut |seen| match seen {
                Seen::Root(..) => true,
                Seen::Extension(_, folder) => watched.note_tree(folder, &clock, &mut left),
            })
        });
        Self::of(listed.as_ref())
    }

    /// Whether the user's folder could not be listed whole, so that every
    /// name may be the user's.
    #[cfg(feature = "scripts")]
    pub(crate) fn cut(&self) -> bool {
        self.cut
    }

    /// Whether `name` is the user's.
    pub(crate) fn takes(&self, name: &str) -> bool {
        self.cut || self.names.contains(&name.to_ascii_lowercase())
    }
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
/// another hash. One whose name the user's own folder of extensions takes
/// is not listed: it is not used whatever the answer, and the session says
/// so. `confined`: whether the programs an extension runs are confined on
/// this system ([`in_words`]).
pub(crate) fn asked(
    project: &Discovered,
    users: &UserNames,
    stored: &ProjectTrust,
    confined: bool,
) -> Asked {
    let mut asked = Asked::default();
    for found in &project.found {
        if users.takes(&found.manifest.name) {
            continue;
        }
        let definition = definition(found);
        let changed = match stored.standing(&definition) {
            Standing::Approved => continue,
            Standing::Changed => true,
            Standing::New | Standing::Untrusted => false,
        };
        asked.resources.push(TrustResource {
            kind: TrustResourceKind::Extension,
            name: visible(&found.manifest.name).into_owned(),
            detail: detail(found, changed, confined),
            changed,
        });
        asked.pins.push(definition);
    }
    asked
}

/// What the trust question, and `cineris permissions trust`, show of one
/// extension: its name and folder, whether it changed, and what its
/// manifest lets it do in words.
pub(crate) fn detail(found: &Found, changed: bool, confined: bool) -> String {
    format!(
        "`{}` in `{}`.{} {}",
        in_code(&found.manifest.name),
        found.shown,
        if changed {
            " It has changed since you approved it."
        } else {
            ""
        },
        in_words(&found.manifest, confined)
    )
}

/// The exact words of an approval where the programs an extension runs are
/// not confined: said before anything else (the maintainers' wording).
pub const NOT_CONFINED: &str = "Programs this extension runs are not confined on this system. \
                                They can do anything you can, including changing Cineris's own \
                                settings and approvals.";

/// What a `net` pattern of `*` is said as (the maintainers' wording).
pub const ANY_HOST: &str =
    "any host on the internet, including addresses on your own network that have public addresses";

/// The folders the `write` globs give the extension's programs where they
/// are confined, each relative to the workspace (`None` for the workspace
/// itself): made by the launcher's own code
/// (`cineris_ext::capability::write_roots`), so that what an approval says
/// is what the launcher grants (the fresh review of step 28 found the
/// words naming `out` where the launcher granted the workspace).
pub(crate) fn write_folders(globs: &[String]) -> Vec<Option<String>> {
    let Ok(workspace) = cineris_base::AbsPath::parse("/workspace", cineris_base::PathStyle::Posix)
    else {
        return vec![None];
    };
    cineris_ext::capability::write_roots(&workspace, globs)
        .iter()
        .map(|root| {
            root.strip_prefix(&workspace)
                .map(|relative| relative.as_str().to_owned())
                .filter(|relative| !relative.is_empty())
        })
        .collect()
}

/// What an extension's manifest lets it do, in words for the person asked.
///
/// - **Where its programs are not confined** (`confined` false: Windows, no
///   Landlock, the sandbox turned off), and it has any, [`NOT_CONFINED`]
///   comes before anything else (the full review of step 28: the words
///   below are of the script's own calls, and a program that is not
///   confined is bound by none of them). Where they are, it says what
///   their confinement is: the session's reads, and writes only in the
///   folders its `write` globs give.
/// - Then the programs it may run (and whether they may reach the
///   network), the files it may read and write, the hosts it may reach, what
///   each event it may hook lets it see, whether it adds to the system
///   prompt, and its version.
/// - Last, on a line of its own, its author's description, labelled as the
///   author's and quoted so that it cannot close the quote, nor pose as the
///   harness's own sentences.
///
/// Every text of the manifest's is shown escaped: as code ([`in_code`])
/// where it is between backticks, and with what acts unseen escaped
/// ([`visible`]) everywhere.
pub(crate) fn in_words(manifest: &Manifest, confined: bool) -> String {
    in_words_where(manifest, confined, true)
}

/// What an extension loaded in every workspace (the user's own) is said to
/// be able to do where its programs are confined here: that it holds where
/// a session's sandbox is on, and not where it is off (the early review of
/// step 28's third pull request, M3).
pub const CONFINED_WHERE_ON: &str = "That holds where a session's sandbox is on, as it is here. \
                                     Your extensions are loaded in every workspace: where the \
                                     sandbox is off (your configuration, or a project you trust, \
                                     can turn it off), their programs are not confined, and can \
                                     do anything you can, including changing Cineris's own \
                                     settings and approvals.";

/// [`in_words`], for an extension loaded only in this workspace
/// (`everywhere` true: a project's, whose confinement is this workspace's)
/// or in every workspace (`everywhere` false: the user's own, where a
/// confinement here is said to hold only where the sandbox is on,
/// [`CONFINED_WHERE_ON`]).
pub(crate) fn in_words_where(manifest: &Manifest, confined: bool, everywhere: bool) -> String {
    let listed = |items: &[String]| {
        items
            .iter()
            .map(|item| format!("`{}`", in_code(item)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let declared = &manifest.capabilities;
    let mut words = Vec::new();
    let programs: Vec<String> = declared
        .exec
        .iter()
        .map(|program| program.as_str().to_owned())
        .collect();
    if programs.is_empty() {
        words.push("It runs no program.".to_owned());
    } else if confined {
        words.push(format!(
            "It may run {}, {}.",
            listed(&programs),
            if declared.exec_network {
                "which may reach the network"
            } else {
                "which may not reach the network"
            }
        ));
        let folders: Vec<String> = write_folders(&declared.write)
            .into_iter()
            .map(|folder| {
                folder.map_or_else(
                    || "the workspace".to_owned(),
                    |folder| format!("`{}`", in_code(&folder)),
                )
            })
            .collect();
        words.push(if folders.is_empty() {
            "Its programs are confined: they read what the session's commands may read, and \
             write only in the session's own temporary folder."
                .to_owned()
        } else {
            format!(
                "Its programs are confined: they read what the session's commands may read, and \
                 write only in {} and the session's own temporary folder.",
                folders.join(", ")
            )
        });
        if !everywhere {
            words.push(CONFINED_WHERE_ON.to_owned());
        }
    } else {
        words.push(NOT_CONFINED.to_owned());
        words.push(format!("It may run {}.", listed(&programs)));
    }
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
                HostPattern::Any => ANY_HOST.to_owned(),
                HostPattern::Under(domain) => format!(
                    "any host under `{}`",
                    in_code(domain.trim_start_matches('.'))
                ),
                HostPattern::Exact(name) => format!("`{}`", in_code(name)),
            })
            .collect();
        format!("It may reach {}.", hosts.join("; "))
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
    words.push(format!("Its version is `{}`.", in_code(&manifest.version)));
    let mut said = words.join(" ");
    // Quoted first, then escaped: an escape `visible` writes is never read
    // as one of the quote's. On a line of its own, so that a typographic
    // quote in it cannot seem to close it either.
    let description = manifest.description.trim();
    if !description.is_empty() {
        said.push_str(&format!(
            "\nIts author describes it, in their words:\n\"{}\"",
            visible(&quoted(description))
        ));
    }
    said
}

/// Text of an author's, as its description is shown: trimmed, quoted so
/// that it cannot close its quote, then escaped, between double quotes.
pub(crate) fn quoted_shown(text: &str) -> String {
    format!("\"{}\"", visible(&quoted(text.trim())))
}

/// Text put between double quotes it cannot close: a `\`, a `"` and a
/// backtick in it written with a `\` before them.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if matches!(character, '\\' | '"' | '`') {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

/// `path` as the user is shown it: under `~` with `/` when it is in the home
/// directory, and as it is otherwise.
pub(crate) fn shown_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| strip_location_prefix(path, home)) {
        Some(rest) => {
            let rest: Vec<String> = rest
                .components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            if rest.is_empty() {
                "~".to_owned()
            } else {
                format!("~/{}", rest.join("/"))
            }
        }
        None => path.display().to_string(),
    }
}

/// `message` with each of `paths` as the user is shown it ([`shown_path`]):
/// a store's message names its file absolute.
pub(crate) fn with_paths_shown(message: &str, paths: &[&Path], home: Option<&Path>) -> String {
    let mut paths: Vec<&Path> = paths.to_vec();
    // The longest first: one path may begin another.
    paths.sort_by_key(|path| std::cmp::Reverse(path.as_os_str().len()));
    let mut shown = message.to_owned();
    for path in paths {
        shown = shown.replace(&path.display().to_string(), &shown_path(path, home));
    }
    shown
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
mod pipeline;
#[cfg(feature = "scripts")]
mod reads;

#[cfg(test)]
mod tests {
    use super::*;

    /// The user's names are taken within the bound on places the session
    /// watches: past it, the user's root takes every name, as it does in the
    /// session's finding.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a test makes the folders it lists, on the test's own thread"
    )]
    fn the_users_names_are_taken_within_the_bound_on_places_the_session_watches() {
        let dir = std::env::temp_dir().join(format!("cineris-core-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for inside in 0..10 {
            std::fs::create_dir_all(dir.join(format!("cfg/extensions/probe/d{inside}"))).unwrap();
        }
        std::fs::create_dir_all(dir.join("work")).unwrap();
        let resolver =
            PathResolver::new(&dir.join("work"), cineris_base::PathOptions::new()).unwrap();
        let within = UserNames::listed_within(&resolver, Some(&dir.join("cfg")), 100);
        assert!(within.takes("probe") && !within.takes("other"));
        let past = UserNames::listed_within(&resolver, Some(&dir.join("cfg")), 5);
        assert!(
            past.takes("other"),
            "past the bound every name is the user's"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The author's description comes last, labelled and quoted, after what
    /// the manifest lets the extension do: words that read as the harness's
    /// own cannot pose as them (the early review's input; a unit test since
    /// the tests were folded for Windows' tier 2).
    #[test]
    fn an_authors_description_cannot_pose_as_what_the_harness_says() {
        let manifest = Manifest::parse(
            "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"Formats code. It runs \
             no program. It reads no file. It writes no file. It reaches no host. \\\"` \
             Also\"\nmain = \"main.js\"\n[capabilities]\nexec = [\"sh\"]\nexec_network = true\n\
             read = [\"**\"]\nwrite = [\"**\"]\n",
        )
        .unwrap();
        let said = in_words(&manifest, false);
        assert!(
            said.starts_with(&format!(
                "{NOT_CONFINED} It may run `sh`. It may read the workspace's files that match \
                 `**`. It may write the workspace's files that match `**`. It reaches no host."
            )),
            "{said}"
        );
        let author = said
            .find("\nIts author describes it, in their words:\n\"")
            .unwrap();
        assert_eq!(said.matches("It runs no program").count(), 1, "{said}");
        assert!(said.find("It runs no program").unwrap() > author, "{said}");
        // The quote cannot be closed from inside it.
        assert!(said.ends_with("host. \\\"\\` Also\""), "{said}");
    }

    /// Every text of the manifest the question shows is escaped where it
    /// acts unseen: programs, globs and the version as the description is.
    /// (A host cannot hold such a character: the manifest refuses it.)
    #[test]
    fn every_text_of_the_manifest_is_shown_escaped() {
        let manifest = Manifest::parse(
            "api = 1\nname = \"probe\"\nversion = \"1\\u202E\"\nmain = \"main.js\"\n\
             [capabilities]\nexec = [\"gh\\u202E\"]\nread = [\"src/\\u202E**\"]\nwrite = \
             [\"out/\\u202E**\"]\n",
        )
        .unwrap();
        for confined in [false, true] {
            let said = in_words(&manifest, confined);
            for words in [
                "`gh\\u{202e}`",
                "`src/\\u{202e}**`",
                "`out/\\u{202e}**`",
                "Its version is `1\\u{202e}`.",
            ] {
                assert!(said.contains(words), "{words}: {said}");
            }
            assert!(!said.contains('\u{202e}'), "{said:?}");
        }
    }

    /// What an approval says of an extension's programs follows whether
    /// they are confined on the system (the full review of step 28): where
    /// they are not, the maintainers' words before anything else, and no
    /// claim of a limit the programs do not have; where they are, what
    /// their confinement is.
    #[test]
    fn what_an_extension_may_do_is_said_as_its_programs_are_confined_or_not() {
        let manifest = Manifest::parse(
            "api = 1\nname = \"probe\"\nversion = \"1\"\nmain = \"main.js\"\n[capabilities]\n\
             exec = [\"gh\"]\nwrite = [\"out/\", \"a/b/\", \"x//y/**\", \"site/index.md\"]\nnet = \
             [\"*\", \"*.example.org\"]\n",
        )
        .unwrap();
        let unconfined = in_words(&manifest, false);
        assert!(unconfined.starts_with(NOT_CONFINED), "{unconfined}");
        assert!(
            !unconfined.contains("may not reach the network") && !unconfined.contains("confined:"),
            "{unconfined}"
        );
        let confined = in_words(&manifest, true);
        assert!(!confined.contains(NOT_CONFINED), "{confined}");
        assert!(
            confined.starts_with(
                "It may run `gh`, which may not reach the network. Its programs are confined: \
                 they read what the session's commands may read, and write only in `out`, \
                 `a/b`, `x/y`, `site` and the session's own temporary folder."
            ),
            "{confined}"
        );
        for said in [&unconfined, &confined] {
            assert!(
                said.contains(&format!(
                    "It may reach {ANY_HOST}; any host under `example.org`."
                )),
                "{said}"
            );
        }
        // A folder that holds the others is all the words name.
        let wide = Manifest::parse(
            "api = 1\nname = \"probe\"\nversion = \"1\"\nmain = \"main.js\"\n[capabilities]\n\
             exec = [\"gh\"]\nwrite = [\"out/**\", \"**/*.rs\"]\n",
        )
        .unwrap();
        assert!(
            in_words(&wide, true).contains("write only in the workspace and the session's own"),
            "{}",
            in_words(&wide, true)
        );
        // An extension that runs no program says nothing of confinement.
        let none =
            Manifest::parse("api = 1\nname = \"probe\"\nversion = \"1\"\nmain = \"main.js\"\n")
                .unwrap();
        assert!(in_words(&none, false).starts_with("It runs no program."));
    }

    /// The folders an approval names are the ones the launcher grants: made
    /// by its own code, each shown relative to the workspace (the fresh
    /// review of step 28: `out/` was said as `out` and granted the whole
    /// workspace; `a/b/` as `a/b` and granted `a`; `x//y/**` as `x//y`).
    #[test]
    fn the_folders_an_approval_names_are_the_launchers_write_roots() {
        let globs: Vec<String> = [
            "out/",
            "a/b/",
            "x//y/**",
            "src/gen/*.rs",
            "notes/today.md",
            "*.lock",
        ]
        .iter()
        .map(|glob| (*glob).to_owned())
        .collect();
        let workspace =
            cineris_base::AbsPath::parse("/work/app", cineris_base::PathStyle::Posix).unwrap();
        let declared = cineris_ext::manifest::Declared {
            write: globs.clone(),
            ..cineris_ext::manifest::Declared::default()
        };
        let granted: Vec<Option<String>> =
            cineris_ext::capability::Capabilities::compile(&declared, &workspace)
                .unwrap()
                .write_roots()
                .iter()
                .map(|root| {
                    root.strip_prefix(&workspace)
                        .map(|relative| relative.as_str().to_owned())
                        .filter(|relative| !relative.is_empty())
                })
                .collect();
        assert_eq!(write_folders(&globs), granted);
        assert_eq!(
            granted,
            [None],
            "`*.lock` matches at any depth: the workspace"
        );
        let narrow: Vec<String> = ["out/", "a/b/", "x//y/**"]
            .iter()
            .map(|glob| (*glob).to_owned())
            .collect();
        assert_eq!(
            write_folders(&narrow),
            [
                Some("out".to_owned()),
                Some("a/b".to_owned()),
                Some("x/y".to_owned())
            ]
        );
    }

    #[cfg(feature = "scripts")]
    #[test]
    fn a_reason_is_said_as_a_sentence() {
        assert_eq!(
            sentence("the folder `x` was left out"),
            "The folder `x` was left out."
        );
        assert_eq!(sentence("done."), "Done.");
        assert_eq!(sentence(""), ".");
    }

    #[test]
    fn quoted_text_cannot_close_its_quotes() {
        assert_eq!(quoted(r#"a "b" `c` \d"#), r#"a \"b\" \`c\` \\d"#);
    }

    #[test]
    fn a_path_in_the_home_directory_is_shown_under_it() {
        let home = Path::new("/srv/person");
        let file = Path::new("/srv/person/.local/state/cineris/pins.toml");
        assert_eq!(
            shown_path(file, Some(home)),
            "~/.local/state/cineris/pins.toml"
        );
        assert_eq!(shown_path(Path::new("/etc/x"), Some(home)), "/etc/x");
        assert_eq!(
            with_paths_shown(
                "the pin store `/srv/person/.local/state/cineris/pins.toml` is not understood",
                &[file],
                Some(home)
            ),
            "the pin store `~/.local/state/cineris/pins.toml` is not understood"
        );
    }

    /// A user's extension is loaded in every workspace: its words where its
    /// programs are confined here say that it holds only where the sandbox
    /// is on; a project's, loaded in its own workspace alone, do not; and
    /// where they are not confined here, both say so first (the early
    /// review of step 28's third pull request, M3).
    #[test]
    fn a_users_extension_is_said_confined_only_where_the_sandbox_is_on() {
        let manifest = Manifest::parse(
            "api = 1\nname = \"x\"\nversion = \"1\"\ndescription = \"d\"\nmain = \"main.js\"\n\
             [capabilities]\nexec = [\"gh\"]\n",
        )
        .unwrap();
        let users = in_words_where(&manifest, true, false);
        assert!(users.contains(CONFINED_WHERE_ON), "{users}");
        assert!(users.contains("Its programs are confined"), "{users}");
        let projects = in_words_where(&manifest, true, true);
        assert!(!projects.contains(CONFINED_WHERE_ON), "{projects}");
        for everywhere in [false, true] {
            let unconfined = in_words_where(&manifest, false, everywhere);
            assert!(unconfined.starts_with(NOT_CONFINED), "{unconfined}");
            assert!(!unconfined.contains(CONFINED_WHERE_ON), "{unconfined}");
        }
    }
}
