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

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cineris_base::{PathResolver, strip_location_prefix, visible};
use cineris_ext::discover::{self, Discovered, Found, Root};
use cineris_ext::manifest::{HostPattern, Manifest};
use cineris_ext::source::Source;
use cineris_policy::{Definition, ProjectTrust, Standing};
use cineris_protocol::methods::{TrustResource, TrustResourceKind};

#[cfg(feature = "scripts")]
pub(crate) use pipeline::{Finding, start};

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

/// Whether a session's extensions are settled: found, judged, registered
/// where the cache missed one, and their tools in the session's registry.
#[derive(Debug, Default)]
struct Ready {
    done: AtomicBool,
    settled: tokio::sync::Notify,
}

impl Ready {
    fn settle(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.settled.notify_waiters();
    }

    async fn wait(&self) {
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

/// A session's extensions: what its first turn waits for, and what a turn's
/// start looks at again.
#[derive(Debug, Default)]
pub(crate) struct Extensions {
    ready: Ready,
    #[cfg(feature = "scripts")]
    held: pipeline::Shared,
}

impl Extensions {
    /// A session with nothing to find.
    pub(crate) fn none() -> Arc<Self> {
        let extensions = Self::default();
        extensions.ready.settle();
        Arc::new(extensions)
    }

    /// Waits until the session's extensions are settled: at once when they
    /// are.
    pub(crate) async fn wait(&self) {
        self.ready.wait().await;
    }

    /// Looks again at what the extensions were found in, at a turn's
    /// start: a change has them read and judged again, and the turn waits
    /// for what that loads.
    pub(crate) async fn recheck(&self) {
        self.wait().await;
        #[cfg(feature = "scripts")]
        self.held.recheck().await;
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

/// Without the script engine, no extension's tool is registered, and there
/// is nothing to find.
#[cfg(not(feature = "scripts"))]
pub(crate) type Pending = ();

/// Without the script engine there is nothing to find.
#[cfg(not(feature = "scripts"))]
pub(crate) fn start(_pending: Pending) -> Arc<Extensions> {
    Extensions::none()
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
        let listed = config_dir.map(|config| discover::names(&[Root::user(config)], resolver));
        Self::of(listed.as_ref())
    }

    /// Whether the user's folder could not be listed whole, so that every
    /// name may be the user's.
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
/// so.
pub(crate) fn asked(project: &Discovered, users: &UserNames, stored: &ProjectTrust) -> Asked {
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
            detail: detail(found, changed),
            changed,
        });
        asked.pins.push(definition);
    }
    asked
}

/// What the trust question, and `cineris permissions trust`, show of one
/// extension: its name and folder, whether it changed, and what its
/// manifest lets it do in words.
pub(crate) fn detail(found: &Found, changed: bool) -> String {
    format!(
        "`{}` in `{}`.{} {}",
        visible(&found.manifest.name),
        found.shown,
        if changed {
            " It has changed since you approved it."
        } else {
            ""
        },
        in_words(&found.manifest)
    )
}

/// What an extension's manifest lets it do, in words for the person asked:
/// the programs it may run (and whether they may reach the network), the
/// files it may read and write, the hosts it may reach, what each event it
/// may hook lets it see, whether it adds to the system prompt, its version;
/// and last its author's description, labelled as the author's and quoted,
/// so that it cannot pose as the harness's own sentences. Every text of the
/// manifest's is shown with what acts unseen escaped ([`visible`]); the
/// manifest refuses a backtick in a program, a glob or the version, which
/// would end the code each is shown as.
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
    words.push(format!("Its version is `{}`.", visible(&manifest.version)));
    let description = manifest.description.trim();
    if !description.is_empty() {
        // Quoted first, then escaped: an escape `visible` writes is never
        // read as one of the quote's.
        words.push(format!(
            "Its author describes it, in their words: \"{}\"",
            visible(&quoted(description))
        ));
    }
    words.join(" ")
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
