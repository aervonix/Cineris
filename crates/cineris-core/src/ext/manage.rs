//! Managing extensions from the command line: `cineris ext list`,
//! `inspect`, `disable`, `remove` and `add <path>` (since step 28's third
//! pull request; `docs/architecture/12-extensions.md`, "As built in step 28:
//! managing extensions"), and what `extensions/status` says of a session's.
//!
//! Each standing is the one a session would give (the table in 12, "As
//! built in step 28's second pull request"), judged from the same stores:
//! the user's pins in `pins.toml`, a project's in `trust.toml`, and what each
//! registered as the registration cache keeps it. Nothing here runs an
//! extension but `add`'s one evaluation, after the person's yes.
//!
//! **`add` pins what it showed.** The folder is read once, through handles
//! that follow no link (`Snapshot::read_unfollowed`), within discovery's
//! caps, and its manifest read strictly; the person is shown that
//! snapshot's manifest in words and every file; on a yes that snapshot, and
//! never a second read of the source (which may be a folder the model can
//! write), is evaluated, written into `<config dir>/extensions/<name>`,
//! pinned, and its registration kept in the cache.
//!
//! These block on the file system, and run on the command line's main
//! thread, outside the core's runtime, as the rest of the core's management
//! does; `add`'s evaluation is a future, on a runtime the caller makes.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "these run on the binary's main thread, for a subcommand, and never on the core thread"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_base::{ContentHash, in_code, same_location, visible};
use cineris_config::schema::Effect;
use cineris_ext::cache::{Cache, Entry, RegistrationCache};
use cineris_ext::discover::{self, Discovered, Root};
use cineris_ext::folder::Snapshot;
use cineris_ext::manifest::Manifest;
use cineris_ext::source::Source;
use cineris_policy::{Definition, PinStore, ProjectTrust, UserPins};
use cineris_protocol::ErrorKind;
use cineris_protocol::methods::TrustResourceKind;

use super::found::{UserNames, definition, in_words, shown_path};
use crate::core::{Core, OpError, error};
use crate::manage::Project;

/// How an extension stands, as a session would judge it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExtensionStanding {
    /// Approved at the hash it is found at: a session loads it.
    Approved,
    /// Approved at another hash: changed since, and not loaded until it is
    /// approved again; its gating hooks hold, failing.
    Changed,
    /// Never approved: not loaded.
    NotApproved,
    /// The user turned it off (`[extensions.<name>] enabled = false`): not
    /// loaded, nothing said or asked.
    TurnedOff,
    /// The store that holds its approval cannot be read, as the text says:
    /// not loaded.
    Unjudged(String),
    /// A project's of a name the user's folder of extensions takes: not
    /// used, as the text says.
    NotUsed(String),
    /// A project's, and the project is not trusted: nothing of it is used.
    ProjectNotTrusted,
    /// Pinned, and its folder is gone: its gating hooks hold, failing,
    /// until it is turned off or removed.
    Gone,
    /// Its folder was found and could not be taken, as the text says.
    LeftOut(String),
}

impl ExtensionStanding {
    /// The standing in words, for a line of a listing.
    #[must_use]
    pub fn words(&self) -> String {
        match self {
            Self::Approved => "approved: a session loads it".to_owned(),
            Self::Changed => "changed since you approved it: not loaded, and its gating \
                              hooks hold, failing, until you approve it again or turn it off"
                .to_owned(),
            Self::NotApproved => "not approved: not loaded".to_owned(),
            Self::TurnedOff => "turned off in your configuration: not loaded".to_owned(),
            Self::Unjudged(why) => format!("cannot be judged: {why}"),
            Self::NotUsed(why) => format!("not used: {why}"),
            Self::ProjectNotTrusted => {
                "the project is not trusted: nothing of it is looked at in a session".to_owned()
            }
            Self::Gone => "its folder is gone, and its pin stands: its gating hooks hold, \
                           failing, until you turn it off or remove it"
                .to_owned(),
            Self::LeftOut(why) => format!("left out: {why}"),
        }
    }
}

/// What an extension registered, as the registration cache keeps it: the
/// names of its tools and commands, and each hook's event and tools.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Registered {
    /// Its tools' names.
    pub tools: Vec<String>,
    /// Its commands' names.
    pub commands: Vec<String>,
    /// Its hooks, each its event and, for a tool event, the tools it is on.
    pub hooks: Vec<String>,
}

impl Registered {
    fn of(entry: &Entry) -> Self {
        let registration = &entry.registration;
        Self {
            tools: registration
                .tools
                .iter()
                .map(|tool| tool.name.clone())
                .collect(),
            commands: registration
                .commands
                .iter()
                .map(|command| command.name.clone())
                .collect(),
            hooks: registration
                .hooks
                .iter()
                .map(|hook| {
                    if hook.tools.is_empty() {
                        hook.event.clone()
                    } else {
                        format!("{} on {}", hook.event, hook.tools.join(", "))
                    }
                })
                .collect(),
        }
    }
}

/// One extension, as `cineris ext list` and `inspect` show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionListed {
    /// The user's own, or the project's.
    pub source: Source,
    /// Its name: its folder's.
    pub name: String,
    /// Its folder as the user is shown it (under `~`, or relative to the
    /// workspace), escaped as code: `None` when it is gone.
    pub folder: Option<String>,
    /// The hash of its folder read whole, when it was.
    pub hash: Option<ContentHash>,
    /// How it stands.
    pub standing: ExtensionStanding,
    /// Its pin, when it has one: the hash and when it was pinned.
    pub pin: Option<(ContentHash, String)>,
    /// What it registered, as the cache keeps it: at the hash found, or else
    /// at its pin's.
    pub registered: Option<Registered>,
    /// What its manifest lets it do, in words, as an approval shows it.
    pub words: Option<String>,
}

/// Every extension `cineris ext list` shows: the user's and the current
/// workspace's project's, trusted or not, with what could not be read, and
/// the skills found and left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionsListing {
    /// The extensions, the user's first, each source's by name.
    pub extensions: Vec<ExtensionListed>,
    /// Whether the project is trusted (in the store): its extensions are
    /// looked at by its sessions only then.
    pub project_trusted: bool,
    /// What could not be read, each a sentence: a store of approvals, the
    /// registration cache, a folder of extensions.
    pub problems: Vec<String>,
    /// The skills found, each its name and its path as shown.
    pub skills: Vec<(String, String)>,
    /// The skills left out, each its path as shown and why.
    pub skills_left_out: Vec<(String, String)>,
}

/// What [`judged`] takes.
pub(crate) struct Judging<'a> {
    pub(crate) user: Option<&'a Discovered>,
    pub(crate) project: Option<&'a Discovered>,
    pub(crate) project_trusted: bool,
    pub(crate) pins: Result<&'a UserPins, String>,
    pub(crate) trust: Result<&'a ProjectTrust, String>,
    pub(crate) for_the_run: &'a [Definition],
    pub(crate) disabled: &'a [String],
    pub(crate) cache: Option<&'a Cache>,
    pub(crate) confined: bool,
}

/// A left-out folder's name, when it is an extension's folder directly in a
/// root that was looked in, and not a note of a root's own.
fn left_out_name(discovered: &Discovered, folder: &Path) -> Option<String> {
    let parent = folder.parent()?;
    if !discovered
        .roots
        .iter()
        .any(|(_, root)| same_location(root, parent))
    {
        return None;
    }
    folder
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// How each extension found, left out or pinned stands, as a session would
/// judge it, in the order of a listing.
pub(crate) fn judged(judging: &Judging<'_>) -> Vec<ExtensionListed> {
    let users = UserNames::of(judging.user);
    let mut out = Vec::new();
    let registered = |hash: Option<ContentHash>, name: &str| {
        let cache = judging.cache?;
        hash.and_then(|hash| cache.get(&hash))
            .filter(|entry| entry.name == name)
            .map(Registered::of)
    };
    for (source, discovered) in [(Source::User, judging.user), (Source::Project, judging.project)]
    {
        let Some(discovered) = discovered else {
            continue;
        };
        let mut seen: Vec<String> = Vec::new();
        for found in discovered.found.iter().filter(|found| found.source == source) {
            let name = found.manifest.name.clone();
            let definition = definition(found);
            let pin = match (source, &judging.pins, &judging.trust) {
                (Source::User, Ok(pins), _) => pins
                    .get(&TrustResourceKind::Extension, &name)
                    .map(|pin| (pin.hash, pin.at.as_str().to_owned())),
                (Source::Project, _, Ok(trust)) => judging
                    .for_the_run
                    .iter()
                    .find(|run| run.kind == TrustResourceKind::Extension && run.name == name)
                    .map(|run| (run.hash, "for this session".to_owned()))
                    .or_else(|| {
                        trust
                            .pinned_of(&TrustResourceKind::Extension)
                            .into_iter()
                            .find(|(pinned, _)| *pinned == name)
                            .map(|(_, hash)| (hash, "with the project's trust".to_owned()))
                    }),
                _ => None,
            };
            let standing = match source {
                Source::User if judging.disabled.contains(&name) => ExtensionStanding::TurnedOff,
                Source::User => match &judging.pins {
                    Err(why) => ExtensionStanding::Unjudged(why.clone()),
                    Ok(pins) => of_policy(pins.standing(&definition)),
                },
                Source::Project if users.takes(&name) => ExtensionStanding::NotUsed(
                    "a folder of your own extensions has its name, and the name is yours"
                        .to_owned(),
                ),
                Source::Project if !judging.project_trusted => {
                    ExtensionStanding::ProjectNotTrusted
                }
                Source::Project => match &judging.trust {
                    Err(why) => ExtensionStanding::Unjudged(why.clone()),
                    Ok(_) if judging.for_the_run.contains(&definition) => {
                        ExtensionStanding::Approved
                    }
                    Ok(trust) => of_policy(trust.standing(&definition)),
                },
            };
            let hash = found.snapshot.hash();
            out.push(ExtensionListed {
                source,
                registered: registered(Some(hash), &name)
                    .or_else(|| registered(pin.as_ref().map(|pin| pin.0), &name)),
                words: Some(in_words(&found.manifest, judging.confined)),
                folder: Some(found.shown.clone()),
                hash: Some(hash),
                standing,
                pin,
                name: name.clone(),
            });
            seen.push(name);
        }
        for left in discovered.left_out.iter().filter(|left| left.source == source) {
            let Some(name) = left_out_name(discovered, &left.folder) else {
                continue;
            };
            if seen.contains(&name) {
                continue;
            }
            out.push(ExtensionListed {
                source,
                folder: Some(left.shown.clone()),
                hash: None,
                standing: ExtensionStanding::LeftOut(left.reason.clone()),
                pin: None,
                registered: None,
                words: None,
                name: name.clone(),
            });
            seen.push(name);
        }
        // Pinned and found nowhere: gone, its gates held.
        let pinned: Vec<(String, ContentHash, String)> = match (source, &judging.pins) {
            (Source::User, Ok(pins)) => pins
                .list()
                .iter()
                .filter(|pin| pin.kind == TrustResourceKind::Extension)
                .map(|pin| (pin.name.clone(), pin.hash, pin.at.as_str().to_owned()))
                .collect(),
            (Source::Project, _) if judging.project_trusted => match &judging.trust {
                Ok(trust) => trust
                    .pinned_of(&TrustResourceKind::Extension)
                    .into_iter()
                    .map(|(name, hash)| (name, hash, "with the project's trust".to_owned()))
                    .collect(),
                Err(_) => Vec::new(),
            },
            _ => Vec::new(),
        };
        for (name, hash, at) in pinned {
            if seen.contains(&name) {
                continue;
            }
            let standing = if source == Source::User && judging.disabled.contains(&name) {
                ExtensionStanding::TurnedOff
            } else {
                ExtensionStanding::Gone
            };
            out.push(ExtensionListed {
                source,
                registered: registered(Some(hash), &name),
                folder: None,
                hash: None,
                standing,
                pin: Some((hash, at)),
                words: None,
                name: name.clone(),
            });
            seen.push(name);
        }
    }
    out
}

fn of_policy(standing: cineris_policy::Standing) -> ExtensionStanding {
    match standing {
        cineris_policy::Standing::Approved => ExtensionStanding::Approved,
        cineris_policy::Standing::Changed => ExtensionStanding::Changed,
        _ => ExtensionStanding::NotApproved,
    }
}

/// Why `name` cannot be the folder of an extension of the user's: it must
/// be one name a session would find, the same on every system. `None` when
/// it can.
#[must_use]
pub fn folder_name_refused(name: &str) -> Option<&'static str> {
    const DEVICES: [&str; 4] = ["con", "prn", "aux", "nul"];
    if cineris_ext::registration::check_name(name).is_err() {
        return Some(
            "it is not an extension's name: 1 to 64 of ASCII letters, digits, `_`, `-` and `.`",
        );
    }
    if name.starts_with('.') {
        return Some(
            "it begins with `.`, and a folder of extensions whose name begins with `.` is never \
             looked in",
        );
    }
    if name.ends_with('.') {
        return Some("it ends with `.`, which Windows takes off a folder's name");
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let numbered = base.len() == 4
        && (base.starts_with("com") || base.starts_with("lpt"))
        && base.as_bytes()[3].is_ascii_digit();
    if DEVICES.contains(&base.as_str()) || numbered {
        return Some("it is the name of a device on Windows, which no folder can have");
    }
    None
}

/// What [`Core::extension_to_add`] read, to show before asking: the
/// snapshot that a yes writes, pins and registers, and nothing read again.
#[derive(Debug)]
pub struct ToAdd {
    /// Its name, the manifest's.
    pub name: String,
    /// Where it was read from, as the user is shown it.
    pub source: String,
    /// Where it is written, under `~`.
    pub destination: String,
    /// The hash of the snapshot: what is pinned.
    pub hash: ContentHash,
    /// What its manifest lets it do, in words.
    pub words: String,
    /// Every file of the snapshot, by its path, with its size.
    pub files: Vec<(String, u64)>,
    /// What is at the destination now that a yes replaces, in words naming
    /// it; `None` when nothing is, or it is the source itself.
    pub replaces: Option<String>,
    /// The source is the destination: it is approved where it is kept.
    pub in_place: bool,
    /// The destination holds this snapshot already: nothing is written.
    pub unchanged: bool,
    /// It is pinned already at this hash.
    pub pinned: bool,
    /// Its earlier pin, at another hash, which a yes replaces.
    pub pinned_before: Option<(ContentHash, String)>,
    /// What else the person should know before the yes, each a sentence.
    pub notes: Vec<String>,
    snapshot: Arc<Snapshot>,
    manifest: Manifest,
    root: PathBuf,
    target: PathBuf,
}

/// What [`Core::add_extension`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    /// Its name.
    pub name: String,
    /// Where it is, under `~`.
    pub destination: String,
    /// Whether its folder was written (`false`: it held this already).
    pub written: bool,
    /// What it registered.
    pub registered: Registered,
    /// Why its registration was not kept in the cache, when it was not: a
    /// session then registers it again.
    pub not_cached: Option<String>,
}

/// What [`Core::disable_extension`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisableOutcome {
    /// The user's file says so now, or said so already.
    Done {
        /// The file, under `~`.
        file: String,
        /// The file was written (`false`: it said so already).
        written: bool,
        /// The file was made, as it was not there.
        made: bool,
        /// When the key takes effect, as it is declared.
        effect: Effect,
        /// Whether the configuration in force here turns it off: `false`
        /// when a managed file holds it on.
        in_force: bool,
    },
    /// It is a project's: a project's extension is kept out by its trust.
    Project,
}

/// What [`Core::extension_removal`] found to remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// Its name.
    pub name: String,
    /// Its pin, when it has one: the hash and when it was pinned.
    pub pin: Option<(ContentHash, String)>,
    /// Its folder, under `~`, when something is at its name, and whether
    /// that is a link (which is unlinked, never walked).
    pub folder: Option<(String, bool)>,
    /// Only a project's has the name: it is not removed.
    pub project_only: bool,
    target: PathBuf,
    root: PathBuf,
}

/// What [`Core::remove_extension`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Removed {
    /// A pin was taken away.
    pub unpinned: bool,
    /// The folder was deleted (a link, unlinked).
    pub deleted: bool,
}

/// Whether `target`, resolved, is a name directly in `root`, resolved: the
/// one place a removal deletes. A path that leads out of the user's folder
/// of extensions is refused, whatever its text.
pub(crate) fn within(root: &Path, target: &Path) -> bool {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return false;
    };
    if name == ".." || name == "." {
        return false;
    }
    match (std::fs::canonicalize(parent), std::fs::canonicalize(root)) {
        (Ok(parent), Ok(root)) => same_location(&parent, &root),
        _ => false,
    }
}

/// Removes what is at `target`, never through a link: a link is unlinked
/// itself, and a folder is removed with everything in it by the standard
/// library's `remove_dir_all`, which follows no link and, since Rust 1.58,
/// walks through handles. Says whether anything was there.
fn delete_unfollowed(target: &Path) -> std::io::Result<bool> {
    let found = match std::fs::symlink_metadata(target) {
        Ok(found) => found,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if found.file_type().is_symlink() {
        // A link to a folder is a folder's entry on Windows, a file's on
        // Linux; either way the link alone goes.
        if std::fs::remove_file(target).is_err() {
            std::fs::remove_dir(target)?;
        }
    } else if found.is_dir() {
        std::fs::remove_dir_all(target)?;
    } else {
        std::fs::remove_file(target)?;
    }
    Ok(true)
}

/// A name beside `target` in its folder, free, for a folder written or set
/// aside: it begins with `.`, so that no session looks in it.
fn beside(target: &Path, what: &str) -> PathBuf {
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let pid = std::process::id();
    for attempt in 0..64_u32 {
        let path = target.with_file_name(format!(".{name}.{pid}-{attempt}.{what}"));
        if std::fs::symlink_metadata(&path).is_err() {
            return path;
        }
    }
    target.with_file_name(format!(".{name}.{pid}.{what}"))
}

/// Writes `snapshot` as the folder `target`, whole or not at all: into a
/// folder of its own beside it, then put in its place, what was there set
/// aside first and removed after, never through a link.
fn write_folder(target: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    let new = beside(target, "new");
    let written = (|| {
        cineris_base::private::create_dir_all(&new)?;
        for path in snapshot.paths() {
            let at = path
                .split('/')
                .fold(new.clone(), |folder, name| folder.join(name));
            if let Some(folder) = at.parent() {
                std::fs::create_dir_all(folder)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&at)?;
            std::io::Write::write_all(&mut file, snapshot.file(path).unwrap_or_default())?;
            file.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = written {
        let _ = delete_unfollowed(&new);
        return Err(error);
    }
    let aside = beside(target, "old");
    let set_aside = match std::fs::rename(target, &aside) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            let _ = delete_unfollowed(&new);
            return Err(error);
        }
    };
    if let Err(error) = std::fs::rename(&new, target) {
        if set_aside {
            let _ = std::fs::rename(&aside, target);
        }
        let _ = delete_unfollowed(&new);
        return Err(error);
    }
    if set_aside {
        delete_unfollowed(&aside)?;
    }
    Ok(())
}

impl Core {
    fn config_dir(&self) -> Option<PathBuf> {
        let options = &self.inner.options;
        options
            .dirs
            .clone()
            .unwrap_or_else(|| cineris_config::Dirs::from_env(&options.env))
            .config
    }

    fn user_config_file(&self) -> Option<PathBuf> {
        let options = &self.inner.options;
        options
            .dirs
            .clone()
            .unwrap_or_else(|| cineris_config::Dirs::from_env(&options.env))
            .user_config
    }

    fn shown(&self, path: &Path) -> String {
        let home = self.inner.options.env.home();
        in_code(&shown_path(path, home.as_deref())).into_owned()
    }

    fn pin_store(&self) -> Result<PinStore, OpError> {
        Ok(PinStore::new(
            self.state()?.pins_file(),
            Arc::clone(&self.inner.options.clock),
        ))
    }

    /// Every extension of the user's and of the project of `project`,
    /// trusted or not, each as a session would judge it, with the skills
    /// found and left out (`cineris ext list`).
    ///
    /// # Errors
    ///
    /// When the workspace cannot be resolved, or its configuration loaded.
    pub fn extensions(&self, project: &Project) -> Result<ExtensionsListing, OpError> {
        let options = &self.inner.options;
        let resolver = self.resolver(&project.root)?;
        let loaded = self.configuration(project, false)?;
        let state = self.state_dir();
        let mut listing = ExtensionsListing::default();
        let home = options.env.home();
        let file_shown =
            |path: &Path| in_code(&shown_path(path, home.as_deref())).into_owned();
        let pins_read = match &state {
            Some(state) => PinStore::new(state.pins_file(), Arc::clone(&options.clock))
                .pins()
                .map_err(|failure| {
                    super::found::with_paths_shown(
                        &failure.to_string(),
                        &[&state.pins_file()],
                        home.as_deref(),
                    )
                }),
            None => Ok(UserPins::default()),
        };
        let trust_read = match &state {
            Some(state) => {
                cineris_policy::TrustStore::new(state.trust_file(), Arc::clone(&options.clock))
                    .project(&project.key)
                    .map_err(|_| {
                        format!(
                            "the trust store `{}` could not be read",
                            file_shown(&state.trust_file())
                        )
                    })
            }
            None => Ok(ProjectTrust::untrusted()),
        };
        for problem in [pins_read.as_ref().err(), trust_read.as_ref().err()]
            .into_iter()
            .flatten()
        {
            listing.problems.push(problem.clone());
        }
        listing.project_trusted = trust_read.as_ref().is_ok_and(ProjectTrust::is_trusted);
        let user = self
            .config_dir()
            .map(|config| discover::discover(&[Root::user(&config)], &resolver));
        let (project_found, _) = super::found::in_project(&resolver, &options.clock);
        // What was left out of a root itself, not of an extension's folder.
        for discovered in user.iter().chain([&project_found]) {
            for left in &discovered.left_out {
                if left_out_name(discovered, &left.folder).is_none()
                    && !listing.problems.contains(&left.reason)
                {
                    listing.problems.push(left.reason.clone());
                }
            }
        }
        let cache = state.as_ref().map(|state| {
            RegistrationCache::new(state.registration_cache_file())
                .with_clock(Arc::clone(&options.clock))
                .load()
        });
        if let (Some(state), Some(cache)) = (&state, &cache)
            && let Some(why) = &cache.set_aside
        {
            listing.problems.push(super::found::with_paths_shown(
                why,
                &[&state.registration_cache_file()],
                home.as_deref(),
            ));
        }
        let disabled: Vec<String> = loaded
            .settings
            .extensions
            .iter()
            .filter(|(_, entry)| !entry.enabled)
            .map(|(name, _)| name.clone())
            .collect();
        let confined = crate::assemble::programs_confined(
            &self.inner,
            &loaded,
            resolver.root(),
            state.as_ref(),
        );
        listing.extensions = judged(&Judging {
            user: user.as_ref(),
            project: Some(&project_found),
            project_trusted: listing.project_trusted,
            pins: pins_read.as_ref().map_err(Clone::clone),
            trust: trust_read.as_ref().map_err(Clone::clone),
            for_the_run: &[],
            disabled: &disabled,
            cache: cache.as_ref().map(|loaded| &loaded.cache),
            confined,
        });
        let skills = cineris_ext::skills::find(
            &resolver,
            &cineris_ext::skills::Root::standard(loaded.settings.compat.skills),
        );
        listing.skills = skills
            .skills
            .iter()
            .map(|skill| (visible(&skill.name).into_owned(), skill.path.clone()))
            .collect();
        listing.skills_left_out = skills
            .left_out
            .iter()
            .map(|left| (left.path.clone(), visible(&left.reason.to_string()).into_owned()))
            .collect();
        Ok(listing)
    }

    /// Reads the folder at `path` once, through handles that follow no link,
    /// as an extension of the user's own to add (`cineris ext add`): what is
    /// shown before the question, and the snapshot a yes takes. Nothing is
    /// written and nothing runs.
    ///
    /// # Errors
    ///
    /// Of kind `usage` when the folder cannot be read as an extension, its
    /// name cannot be one of the user's (the names' rules), or the pin store
    /// cannot be read.
    pub fn extension_to_add(&self, project: &Project, path: &Path) -> Result<ToAdd, OpError> {
        let usage = |message: String| error(ErrorKind::Usage, "extension_refused", message);
        let config = self.config_dir().ok_or_else(|| {
            usage("No configuration directory could be worked out: the home directory is not known.".to_owned())
        })?;
        let source_shown = self.shown(path);
        let (read, _) = Snapshot::read_unfollowed(path);
        let snapshot = read.map_err(|failure| {
            let why = match &failure {
                cineris_ext::folder::FolderError::Link(at) if at.is_empty() => {
                    "it is a link, and is not followed: name the folder it leads to".to_owned()
                }
                other => visible(&other.to_string()).into_owned(),
            };
            usage(format!("`{source_shown}` cannot be added as an extension: {why}"))
        })?;
        let manifest = Manifest::of(&snapshot).map_err(|failure| {
            usage(format!(
                "`{source_shown}` cannot be added as an extension: {}",
                visible(&failure.to_string())
            ))
        })?;
        let name = manifest.name.clone();
        let code = in_code(&name).into_owned();
        if let Some(why) = folder_name_refused(&name) {
            return Err(usage(format!(
                "The extension `{code}` cannot be added: {why}."
            )));
        }
        let resolver = self.resolver(&project.root)?;
        let root = config.join(discover::USER_FOLDER);
        let names = discover::names(&[Root::user(&config)], &resolver);
        if names.cut.contains(&Source::User) {
            return Err(usage(format!(
                "The extension `{code}` cannot be added: your folder of extensions `{}` could \
                 not be listed whole, so any name may be taken there.",
                self.shown(&root)
            )));
        }
        if let Some((_, other)) = names.names.iter().find(|(source, other)| {
            *source == Source::User && *other != name && other.eq_ignore_ascii_case(&name)
        }) {
            return Err(usage(format!(
                "The extension `{code}` cannot be added: your folder of extensions holds \
                 `{}`, whose name differs from it only in case. Names that differ only in case \
                 are one name on Windows, where they would share what an extension keeps, so \
                 neither would be used: rename or remove that folder first.",
                in_code(other)
            )));
        }
        let target = root.join(&name);
        let in_place = matches!(
            (std::fs::canonicalize(path), std::fs::canonicalize(&target)),
            (Ok(from), Ok(to)) if same_location(&from, &to)
        );
        let destination = self.shown(&target);
        let (replaces, unchanged) = match std::fs::symlink_metadata(&target) {
            Err(_) => (None, false),
            Ok(found) if found.file_type().is_symlink() => (
                Some(format!("`{destination}`, which is a link (it is unlinked, not followed)")),
                false,
            ),
            Ok(found) if found.is_dir() => match Snapshot::read_unfollowed(&target).0 {
                Ok(there) if there.hash() == snapshot.hash() => (None, true),
                _ if in_place => (None, false),
                Ok(there) => (
                    Some(format!(
                        "`{destination}`, which holds something else (its hash is {})",
                        there.hash()
                    )),
                    false,
                ),
                Err(why) => (
                    Some(format!(
                        "`{destination}`, which cannot be read as an extension ({})",
                        visible(&why.to_string())
                    )),
                    false,
                ),
            },
            Ok(_) => (
                Some(format!("`{destination}`, which is a file, not a folder")),
                false,
            ),
        };
        let pins = self.pin_store()?.pins().map_err(|failure| {
            usage(format!(
                "The extension `{code}` cannot be added: {}. Repair or remove the file, then add \
                 it again.",
                super::found::with_paths_shown(
                    &failure.to_string(),
                    &[&self.state().map(|state| state.pins_file()).unwrap_or_default()],
                    self.inner.options.env.home().as_deref()
                )
            ))
        })?;
        let pin = pins.get(&TrustResourceKind::Extension, &name);
        let pinned = pin.is_some_and(|pin| pin.hash == snapshot.hash());
        let pinned_before = pin
            .filter(|pin| pin.hash != snapshot.hash())
            .map(|pin| (pin.hash, pin.at.as_str().to_owned()));
        let mut notes = Vec::new();
        let projects = discover::names(&[Root::project(&resolver.root().to_fs_path())], &resolver);
        if let Some((_, theirs)) = projects
            .names
            .iter()
            .find(|(source, theirs)| *source == Source::Project && theirs.eq_ignore_ascii_case(&name))
        {
            notes.push(format!(
                "This project has an extension of its own named `{}`: once yours is added, the \
                 project's is not used, since the name is yours.",
                in_code(theirs)
            ));
        }
        let loaded = self.configuration(project, false)?;
        if loaded
            .settings
            .extensions
            .get(&name)
            .is_some_and(|entry| !entry.enabled)
        {
            notes.push(format!(
                "Your configuration turns it off (`[extensions.{}] enabled = false`): it is not \
                 loaded until you take that out.",
                toml_key(&name)
            ));
        }
        let confined = crate::assemble::programs_confined(
            &self.inner,
            &loaded,
            resolver.root(),
            self.state_dir().as_ref(),
        );
        Ok(ToAdd {
            words: in_words(&manifest, confined),
            files: snapshot
                .paths()
                .map(|path| {
                    (
                        in_code(path).into_owned(),
                        snapshot.file(path).map_or(0, |bytes| bytes.len() as u64),
                    )
                })
                .collect(),
            hash: snapshot.hash(),
            source: source_shown,
            destination,
            replaces,
            in_place,
            unchanged,
            pinned,
            pinned_before,
            notes,
            snapshot: Arc::new(snapshot),
            manifest,
            root,
            target,
            name,
        })
    }

    /// Approves `to_add` as it was shown, on the person's yes: its snapshot
    /// evaluated once (what it registers), written into the user's folder of
    /// extensions in place of what was there, pinned in `pins.toml`, and its
    /// registration kept in the cache. Nothing is read again from where it
    /// came from.
    ///
    /// # Errors
    ///
    /// When its evaluation fails (nothing is written or pinned then), its
    /// folder cannot be written, or the pin cannot be kept.
    #[cfg(feature = "scripts")]
    pub async fn add_extension(&self, to_add: ToAdd) -> Result<Added, OpError> {
        let failed = |message: String| error(ErrorKind::Internal, "extension_not_added", message);
        let code = in_code(&to_add.name).into_owned();
        // A process that left a thread of this hash running refuses to
        // start it again until it is approved anew: this is that approval.
        cineris_ext::engine::forget_abandoned(&to_add.hash);
        let (manifest, registration) = cineris_ext::engine::register(
            Arc::clone(&to_add.snapshot),
            self.inner.options.timer.as_ref(),
        )
        .await
        .map_err(|failure| {
            error(
                ErrorKind::Usage,
                "extension_not_registered",
                format!(
                    "The extension `{code}` was not added: its script could not be evaluated, \
                     so nothing was written or pinned: {}",
                    visible(&failure.to_string())
                ),
            )
        })?;
        if manifest != to_add.manifest {
            return Err(failed(format!(
                "The extension `{code}` was not added: what was evaluated is not what was shown."
            )));
        }
        let written = !to_add.unchanged;
        if written {
            cineris_base::private::create_dir_all(&to_add.root).map_err(|failure| {
                failed(format!(
                    "The extension `{code}` was not added: `{}` could not be made: {failure}",
                    self.shown(&to_add.root)
                ))
            })?;
            write_folder(&to_add.target, &to_add.snapshot).map_err(|failure| {
                failed(format!(
                    "The extension `{code}` was not added: `{}` could not be written: {failure}",
                    to_add.destination
                ))
            })?;
        }
        self.pin_store()?
            .pin(&Definition {
                kind: TrustResourceKind::Extension,
                name: to_add.name.clone(),
                hash: to_add.hash,
            })
            .map_err(|failure| {
                failed(format!(
                    "The extension `{code}` was written to `{}` and not pinned, so it is not \
                     loaded: {failure}",
                    to_add.destination
                ))
            })?;
        let prompt = manifest
            .prompt
            .as_deref()
            .and_then(|file| to_add.snapshot.file(file))
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
        let entry = Entry::new(&manifest, registration, prompt);
        let registered = Registered::of(&entry);
        let not_cached = self.state().ok().and_then(|state| {
            RegistrationCache::new(state.registration_cache_file())
                .with_clock(Arc::clone(&self.inner.options.clock))
                .put(&to_add.hash, entry)
                .err()
                .map(|failure| failure.to_string())
        });
        Ok(Added {
            name: to_add.name,
            destination: to_add.destination,
            written,
            registered,
            not_cached,
        })
    }

    /// Writes `[extensions.<name>] enabled = false` into the user's own
    /// configuration (`cineris ext disable`), for an extension of the user's
    /// own, found or pinned; for a project's, says it is one.
    ///
    /// # Errors
    ///
    /// Of kind `usage` when no extension of that name is the user's or the
    /// project's, or the file cannot take the change.
    pub fn disable_extension(
        &self,
        project: &Project,
        name: &str,
    ) -> Result<DisableOutcome, OpError> {
        let usage = |message: String| error(ErrorKind::Usage, "extension_refused", message);
        let code = in_code(name).into_owned();
        let resolver = self.resolver(&project.root)?;
        let mine = self.config_dir().is_some_and(|config| {
            discover::names(&[Root::user(&config)], &resolver)
                .names
                .iter()
                .any(|(source, found)| *source == Source::User && found == name)
        }) || self
            .pin_store()
            .ok()
            .and_then(|store| store.pins().ok())
            .is_some_and(|pins| pins.get(&TrustResourceKind::Extension, name).is_some());
        if !mine {
            let theirs = discover::names(&[Root::project(&resolver.root().to_fs_path())], &resolver)
                .names
                .iter()
                .any(|(source, found)| *source == Source::Project && found == name);
            if theirs {
                return Ok(DisableOutcome::Project);
            }
            return Err(usage(format!(
                "No extension of yours is named `{code}`: none is in your folder of extensions, \
                 and none is pinned."
            )));
        }
        let file = self.user_config_file().ok_or_else(|| {
            usage("Your configuration file could not be worked out: the home directory is not known.".to_owned())
        })?;
        let shown = self.shown(&file);
        let done = cineris_config::write::disable_extension(&file, name).map_err(|failure| {
            usage(format!(
                "`{shown}` was not changed: {}",
                visible(&failure.to_string())
            ))
        })?;
        let effect = cineris_config::schema::keys()
            .into_iter()
            .find(|key| key.path == "extensions.*.enabled")
            .and_then(|key| key.effect)
            .unwrap_or(Effect::NextSession);
        let in_force = self.configuration(project, false).is_ok_and(|loaded| {
            loaded
                .settings
                .extensions
                .get(name)
                .is_some_and(|entry| !entry.enabled)
        });
        let (written, made) = match done {
            cineris_config::write::Disabled::Written { made } => (true, made),
            cineris_config::write::Disabled::Already => (false, false),
        };
        Ok(DisableOutcome::Done {
            file: shown,
            written,
            made,
            effect,
            in_force,
        })
    }

    /// What removing the user's extension `name` would take away
    /// (`cineris ext remove`): its pin and its folder. Nothing is changed.
    ///
    /// # Errors
    ///
    /// Of kind `usage` when `name` cannot be an extension's, nothing of the
    /// name is the user's or the project's, or the pin store cannot be read.
    pub fn extension_removal(&self, project: &Project, name: &str) -> Result<Removal, OpError> {
        let usage = |message: String| error(ErrorKind::Usage, "extension_refused", message);
        let code = in_code(name).into_owned();
        if let Some(why) = folder_name_refused(name) {
            return Err(usage(format!("`{code}` cannot be removed: {why}.")));
        }
        let config = self.config_dir().ok_or_else(|| {
            usage("No configuration directory could be worked out: the home directory is not known.".to_owned())
        })?;
        let root = config.join(discover::USER_FOLDER);
        let target = root.join(name);
        let pins = self.pin_store()?.pins().map_err(|failure| {
            usage(format!(
                "`{code}` was not removed: {}. Repair or remove the file first.",
                visible(&failure.to_string())
            ))
        })?;
        let pin = pins
            .get(&TrustResourceKind::Extension, name)
            .map(|pin| (pin.hash, pin.at.as_str().to_owned()));
        let folder = std::fs::symlink_metadata(&target)
            .ok()
            .map(|found| (self.shown(&target), found.file_type().is_symlink()));
        let project_only = pin.is_none() && folder.is_none() && {
            let resolver = self.resolver(&project.root)?;
            discover::names(&[Root::project(&resolver.root().to_fs_path())], &resolver)
                .names
                .iter()
                .any(|(source, found)| *source == Source::Project && found == name)
        };
        if pin.is_none() && folder.is_none() && !project_only {
            return Err(usage(format!(
                "No extension of yours is named `{code}`: none is in your folder of extensions, \
                 and none is pinned."
            )));
        }
        Ok(Removal {
            name: name.to_owned(),
            pin,
            folder,
            project_only,
            target,
            root,
        })
    }

    /// Removes the user's extension `removal` names: its pin taken away,
    /// and its folder deleted when `delete` (the person's yes naming it),
    /// never through a link, and only where it is directly in the user's
    /// folder of extensions, resolved.
    ///
    /// # Errors
    ///
    /// When it is a project's, the pin store cannot be written (nothing is
    /// deleted then), or the folder is not where it should be or cannot be
    /// deleted.
    pub fn remove_extension(&self, removal: &Removal, delete: bool) -> Result<Removed, OpError> {
        let code = in_code(&removal.name).into_owned();
        if removal.project_only {
            return Err(error(
                ErrorKind::Usage,
                "extension_refused",
                format!(
                    "`{code}` is this project's extension, which is not removed here: `cineris \
                     permissions trust` approves the project's extensions, and `cineris \
                     permissions untrust` takes its trust away."
                ),
            ));
        }
        let failed = |message: String| error(ErrorKind::Internal, "extension_not_removed", message);
        let unpinned = self
            .pin_store()?
            .unpin(&TrustResourceKind::Extension, &removal.name)
            .map_err(|failure| {
                failed(format!(
                    "`{code}` was not removed, and nothing was deleted: its pin could not be \
                     taken away: {failure}"
                ))
            })?;
        let mut deleted = false;
        if delete && std::fs::symlink_metadata(&removal.target).is_ok() {
            if !within(&removal.root, &removal.target) {
                return Err(failed(format!(
                    "`{code}` was unpinned and its folder not deleted: `{}` does not lead to a \
                     folder directly in your folder of extensions.",
                    self.shown(&removal.target)
                )));
            }
            deleted = delete_unfollowed(&removal.target).map_err(|failure| {
                failed(format!(
                    "`{code}` was unpinned, and its folder `{}` could not be deleted: {failure}",
                    self.shown(&removal.target)
                ))
            })?;
        }
        Ok(Removed { unpinned, deleted })
    }
}

/// What `extensions/status` says of each extension: of a session, as its
/// finding loaded it and with the gates it holds; with none, of the user's
/// own, as a session with no project would find them (since step 28's third
/// pull request). Blocks.
pub(crate) fn status(
    core: &Core,
    #[cfg(feature = "scripts")] view: Option<&super::found::SessionView>,
    gates: &[(String, String, String, String)],
) -> cineris_protocol::methods::ExtensionsStatusResult {
    use cineris_protocol::methods::{ComponentStatus, ExtensionsStatusResult, HeldGate};
    let options = &core.inner.options;
    let home = options.env.home();
    #[cfg(feature = "scripts")]
    let (resolver, config, state, project, for_the_run, disabled) = match view {
        Some(view) => (
            Some(Arc::clone(&view.resolver)),
            view.config_dir.clone(),
            view.state.clone(),
            view.project.clone(),
            view.for_the_run.clone(),
            view.disabled.clone(),
        ),
        None => none_of(core),
    };
    #[cfg(not(feature = "scripts"))]
    let (resolver, config, state, project, for_the_run, disabled) = none_of(core);
    let Some(resolver) = resolver else {
        return ExtensionsStatusResult {
            extensions: Vec::new(),
            problems: Vec::new(),
        };
    };
    let mut problems = Vec::new();
    let pins = match &state {
        Some(state) => PinStore::new(state.pins_file(), Arc::clone(&options.clock))
            .pins()
            .map_err(|failure| {
                super::found::with_paths_shown(
                    &failure.to_string(),
                    &[&state.pins_file()],
                    home.as_deref(),
                )
            }),
        None => Ok(UserPins::default()),
    };
    let trust = match (&state, &project) {
        (Some(state), Some(key)) => {
            cineris_policy::TrustStore::new(state.trust_file(), Arc::clone(&options.clock))
                .project(key)
                .map_err(|_| {
                    format!(
                        "the trust store `{}` could not be read",
                        in_code(&shown_path(&state.trust_file(), home.as_deref()))
                    )
                })
        }
        _ => Ok(ProjectTrust::untrusted()),
    };
    for problem in [pins.as_ref().err(), trust.as_ref().err()]
        .into_iter()
        .flatten()
    {
        problems.push(problem.clone());
    }
    let user = config.map(|config| discover::discover(&[Root::user(&config)], &resolver));
    let project_found = project
        .as_ref()
        .map(|_| super::found::in_project(&resolver, &options.clock).0);
    let cache = state.as_ref().map(|state| {
        RegistrationCache::new(state.registration_cache_file())
            .with_clock(Arc::clone(&options.clock))
            .load()
    });
    let listed = judged(&Judging {
        user: user.as_ref(),
        project: project_found.as_ref(),
        project_trusted: project.is_some(),
        pins: pins.as_ref().map_err(Clone::clone),
        trust: trust.as_ref().map_err(Clone::clone),
        for_the_run: &for_the_run,
        disabled: &disabled,
        cache: cache.as_ref().map(|loaded| &loaded.cache),
        confined: false,
    });
    let extensions = listed
        .into_iter()
        .map(|one| {
            #[cfg(feature = "scripts")]
            let (loaded, unloaded) = match view {
                Some(view) => (
                    Some(view.loaded.iter().any(|(source, name, hash)| {
                        *source == one.source && *name == one.name && Some(*hash) == one.hash
                    })),
                    view.unloaded
                        .iter()
                        .find(|(source, name, _)| *source == one.source && *name == one.name)
                        .map(|(_, _, why)| why.clone()),
                ),
                None => (None, None),
            };
            #[cfg(not(feature = "scripts"))]
            let (loaded, unloaded): (Option<bool>, Option<String>) = (None, None);
            let (state, error) = component_state(&one.standing, loaded, unloaded);
            let held: Vec<HeldGate> = gates
                .iter()
                .filter(|(extension, ..)| *extension == one.name)
                .map(|(_, hook, event, why)| HeldGate {
                    hook: hook.clone(),
                    event: event.clone(),
                    why: why.clone(),
                })
                .collect();
            let registered = one.registered.clone().unwrap_or_default();
            ComponentStatus {
                name: visible(&one.name).into_owned(),
                state,
                tools: u32::try_from(registered.tools.len()).unwrap_or(u32::MAX),
                error,
                source: Some(match one.source {
                    Source::User => cineris_protocol::methods::ExtensionSource::User,
                    Source::Project => cineris_protocol::methods::ExtensionSource::Project,
                }),
                standing: Some(wire_standing(&one.standing)),
                folder: one.folder.clone(),
                hash: one.hash.map(|hash| hash.to_string()),
                commands: registered.commands,
                hooks: registered.hooks,
                gates: held,
            }
        })
        .collect();
    ExtensionsStatusResult {
        extensions,
        problems,
    }
}

/// The state an extension is in, from its standing and, in a session,
/// whether the session loaded it (`loaded`) and why its gate refuses where
/// it stands approved and could not be loaded (`unloaded`).
pub(crate) fn component_state(
    standing: &ExtensionStanding,
    loaded: Option<bool>,
    unloaded: Option<String>,
) -> (cineris_protocol::methods::ComponentState, Option<String>) {
    use cineris_protocol::methods::ComponentState;
    match (standing, loaded, unloaded) {
        (_, _, Some(why)) => (ComponentState::Failed, Some(why)),
        (ExtensionStanding::Approved, Some(true) | None, _) => (ComponentState::Ready, None),
        // Approved as found, and not loaded yet: at the next turn.
        (ExtensionStanding::Approved, Some(false), _) => (
            ComponentState::Starting,
            Some("approved as it is found now, and loaded at the session's next turn".to_owned()),
        ),
        (ExtensionStanding::Changed | ExtensionStanding::NotApproved, ..) => {
            (ComponentState::NeedsApproval, Some(standing.words()))
        }
        (
            ExtensionStanding::TurnedOff
            | ExtensionStanding::NotUsed(_)
            | ExtensionStanding::ProjectNotTrusted,
            ..,
        ) => (ComponentState::Disabled, Some(standing.words())),
        _ => (ComponentState::Failed, Some(standing.words())),
    }
}

/// What a status with no session looks in.
type Lookin = (
    Option<Arc<cineris_base::PathResolver>>,
    Option<PathBuf>,
    Option<cineris_config::StateDir>,
    Option<cineris_protocol::ProjectKey>,
    Vec<Definition>,
    Vec<String>,
);

/// What a status with no session looks in: the user's own folder, the
/// core's own configuration, no project.
fn none_of(core: &Core) -> Lookin {
    let resolver = core
        .config_dir()
        .and_then(|config| core.resolver(&config).ok())
        .map(Arc::new);
    let disabled = core
        .inner
        .user()
        .settings
        .extensions
        .iter()
        .filter(|(_, entry)| !entry.enabled)
        .map(|(name, _)| name.clone())
        .collect();
    (
        resolver,
        core.config_dir(),
        core.state_dir(),
        None,
        Vec::new(),
        disabled,
    )
}

fn wire_standing(standing: &ExtensionStanding) -> cineris_protocol::methods::ExtensionStanding {
    use cineris_protocol::methods::ExtensionStanding as Wire;
    match standing {
        ExtensionStanding::Approved => Wire::Approved,
        ExtensionStanding::Changed => Wire::Changed,
        ExtensionStanding::NotApproved => Wire::NotApproved,
        ExtensionStanding::TurnedOff => Wire::TurnedOff,
        ExtensionStanding::Unjudged(_) => Wire::Unjudged,
        ExtensionStanding::NotUsed(_) => Wire::NotUsed,
        ExtensionStanding::ProjectNotTrusted => Wire::ProjectNotTrusted,
        ExtensionStanding::Gone => Wire::Gone,
        ExtensionStanding::LeftOut(_) => Wire::LeftOut,
    }
}

/// `name` as a key of a TOML table's header: bare when it can be, quoted
/// otherwise (`acme.lint` is one key, `"acme.lint"`).
fn toml_key(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        name.to_owned()
    } else {
        format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[cfg(test)]
mod tests;
