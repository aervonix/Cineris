//! Finding extensions: each folder directly under the user's folder of
//! extensions and the project's, read whole and its manifest checked
//! (`docs/architecture/12-extensions.md`, "Tier 4: script extensions";
//! `docs/steps/28/answered.md`, points 4 and 5).
//!
//! [`discover`] is given the roots, `<config dir>/extensions` for the
//! user's ([`Root::user`]) and `<workspace>/.cineris/extensions` for the
//! project's ([`Root::project`]), and the session's path resolver. Each
//! folder directly under a root is read into a [`Snapshot`] (every file, and
//! the hash of all of them, which is what is pinned) and its manifest read
//! from that snapshot: a [`Found`]. What cannot be taken is a [`LeftOut`],
//! whose reason names the folder and says why. Nothing here approves or
//! runs anything: whether a found extension's hash is pinned is the
//! caller's question.
//!
//! **What an extension is.** A folder directly under a root whose manifest
//! ([`manifest::FILE`]) gives the folder's own name, exactly: a manifest
//! that gives another is left out with both names. A file beside the
//! folders is passed over without a word, and so is a folder whose name
//! begins with `.` (a `.git` of the user's dotfiles, an editor's folder):
//! no extension's name begins with one, and such a folder is not read.
//!
//! **A root that is not there gives nothing, at the cost of one look.**
//! With no extension, discovery must cost a session nothing measurable, so
//! the common case is decided before the path resolver is asked (which
//! looks at every name on the way and then asks for the real spelling):
//!
//! - a user's root given as an absolute path is looked at once, as it is
//!   (`symlink_metadata`): the user's own configuration directory, which is
//!   asked as it is wherever the harness reads the user's files;
//! - a project's root below the workspace is looked at one name at a time
//!   from the workspace's real location, never through a link: one look
//!   when the workspace has no `.cineris`, two when it has one with no
//!   `extensions` in it. A name that is a link sends the root to the
//!   resolver, which checks where a link leads before it follows it, so
//!   that no link a repository holds (on Windows, one to a network share,
//!   which a look through it would send the account's credentials to) is
//!   followed by the system on the harness's behalf.
//!
//! Any other root (one given with `~`, `..` or relative to the workspace,
//! and every root that is there) is resolved in full. [`Discovered::resolved`]
//! counts the roots that were.
//!
//! **Paths are judged where they lead.** A root is resolved by the
//! session's [`PathResolver`], which takes `~`, a relative path and `..`,
//! follows each link and, on Windows, puts every name in the case the file
//! system stores; two places are then compared by place
//! ([`location_within`], [`same_location`]), never by their text.
//!
//! - **A user's root may be a link,** and is followed: the user made it.
//! - **A project's root must lead inside the workspace.** A
//!   `.cineris/extensions`, or a `.cineris`, that is a link leading out of
//!   it gives nothing, and a [`LeftOut`] that says so: what a repository's
//!   link reaches is not the repository's. One that leads to another place
//!   inside the workspace is followed.
//! - **An extension's folder reached through a link is refused,** as
//!   [`Snapshot::read`] refuses a link inside one: the folder is read where
//!   it is, so that what is pinned is the folder in the root. The folder is
//!   looked at again after it is read, and one that has become a link by
//!   then is refused too.
//! - **The same folder given as two roots** is looked in once, as the first
//!   root given; the second is left out with the first one's name.
//!
//! **Two extensions of one name.** From two roots of different sources (the
//! user's `tracker` and the project's `tracker`) both are found: they are
//! different extensions, pinned in different stores and keyed by
//! [`Source`]. Within one source, a name is one extension's:
//!
//! - **Two roots of one source that each hold the same name** give it once,
//!   from the first root given; the second is left out with the first one's
//!   folder.
//! - **Names that differ only in case** (`Tracker` and `tracker`, in one
//!   root or in two of one source) **are all refused, on every system**, each
//!   left out naming the others: on Windows they are one name, where they
//!   would share one folder and what an extension keeps under its name (its
//!   `project` state file among it), and only a folder made case-sensitive
//!   can hold both. None is kept, so that nothing depends on which was
//!   listed first, and a set of extensions behaves the same on every
//!   system. The names are compared with ASCII's case folded, which is the
//!   whole of Unicode's folding for them: a manifest's name is ASCII
//!   letters, digits, `_`, `-` and `.` (the registry's rules), and a found
//!   extension's name is its folder's, exactly. Windows' own table of case
//!   agrees on ASCII.
//!
//! **Caps.** At most [`MAX_ENTRIES`] entries of a root are listed (a file
//! counts, since each costs a look); past it the rest of the root is not
//! looked at, and that is said. Of those, at most [`MAX_PER_ROOT`] folders
//! are read, the first by name; the rest are left out with their count, in
//! one [`LeftOut`] for the root. Each folder is read within
//! [`Snapshot::read`]'s caps.
//!
//! **The order is fixed:** the roots in the order given, and in each the
//! folders by their names' bytes. What was left out follows the same
//! order, a root's count of folders past its cap after its folders, and
//! then the extensions left out for their names, in the order they were
//! found.
//!
//! **What the user is shown.** A folder is shown relative to the workspace
//! for the project's, under `~` for the user's (absolute only for a user's
//! root that leads out of the home directory), with `/`, and every
//! character that acts unseen (a control character, a line break, a
//! bidirectional control) written as an escape ([`visible`]), since a
//! folder's name comes from a repository. The reasons are escaped the same
//! way: a manifest's refusal quotes what the manifest says.
//!
//! [`discover`] blocks: it reads folders and files. The core calls it on
//! the blocking pool, off the start-up path.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "finding extensions reads folders and files; callers run it on the blocking pool"
)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, FileType};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use cineris_base::{
    AbsPath, PathError, PathResolver, location_within, same_location, strip_location_prefix,
    visible,
};

use crate::folder::Snapshot;
use crate::manifest::{self, Manifest};
use crate::source::Source;

/// The most extension folders read from one root. The rest are left out,
/// counted.
///
/// Every folder found is read whole, so a root's cost is this many times
/// [`Snapshot::read`]'s caps at most. A user keeps a handful of
/// extensions, and a project fewer; 64 is more than either needs, and
/// bounds what a repository with a folder of thousands can make a session
/// read.
pub const MAX_PER_ROOT: usize = 64;

/// The most entries listed in one root, files and folders together. Past
/// it the rest of the root is not looked at.
///
/// A root is listed whole before its folders are sorted by name, so the
/// listing is bounded on its own: a folder of a great many files costs no
/// more than this many entries.
pub const MAX_ENTRIES: usize = 1024;

/// The user's folder of extensions, in the configuration directory.
pub const USER_FOLDER: &str = "extensions";

/// The project's folder of extensions, in the workspace: `.cineris/extensions`.
pub const PROJECT_FOLDER: [&str; 2] = [".cineris", "extensions"];

/// A folder to look for extensions in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// Whose extensions it holds.
    pub source: Source,
    /// Where it is: absolute, relative to the workspace, or beginning with
    /// `~` for the home directory, as the session's resolver takes a path.
    /// `..` and links are taken as the resolver takes them. An ordinary
    /// path, not a Windows `\\?\` one.
    pub dir: PathBuf,
}

impl Root {
    /// A root of `source`'s at `dir`.
    #[must_use]
    pub fn new(source: Source, dir: impl Into<PathBuf>) -> Self {
        Self {
            source,
            dir: dir.into(),
        }
    }

    /// The user's root, `<config dir>/extensions`.
    #[must_use]
    pub fn user(config_dir: &Path) -> Self {
        Self::new(Source::User, config_dir.join(USER_FOLDER))
    }

    /// The project's root, `<workspace>/.cineris/extensions`.
    #[must_use]
    pub fn project(workspace: &Path) -> Self {
        let mut dir = workspace.to_path_buf();
        dir.extend(PROJECT_FOLDER);
        Self::new(Source::Project, dir)
    }
}

/// An extension that was found: its folder read whole, and its manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Whose root it was found in.
    pub source: Source,
    /// Where its folder is: the root's real location, every link on the way
    /// followed, and the folder's own name, as the file system's calls take
    /// it (on Windows, in the `\\?\` form).
    pub folder: PathBuf,
    /// The folder as the user is shown it: relative to the workspace for
    /// the project's, under `~` for the user's, with `/`, escaped.
    pub shown: String,
    /// Every file of the folder, as it was read, and their hash.
    pub snapshot: Arc<Snapshot>,
    /// The manifest, read from the snapshot. Its `name` is the folder's.
    pub manifest: Manifest,
}

/// A root, or a folder in one, that was not taken, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    /// Whose root it was in.
    pub source: Source,
    /// The folder: an extension's, or the root (for a root that was not
    /// looked in, as it was given; for its count of folders past the cap,
    /// where it really is).
    pub folder: PathBuf,
    /// The folder as the user is shown it, as [`Found::shown`] is, or the
    /// root as it was given, escaped, when it was not resolved.
    pub shown: String,
    /// Why, in words, naming the folder: a sentence for the user, with
    /// every character that acts unseen written as an escape.
    pub reason: String,
}

/// What [`discover`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discovered {
    /// The extensions, the roots in the order given and each root's by
    /// name.
    pub found: Vec<Found>,
    /// What was left out, in the same order, and then the extensions left
    /// out for their names.
    pub left_out: Vec<LeftOut>,
    /// How many roots were resolved through the path resolver: those that
    /// are there, and those that could not be ruled out with a look or two.
    /// A root that is not there, given as the core gives it, is not.
    pub resolved: usize,
}

/// Looks for extensions in `roots`, through `paths`, the session's
/// resolver: its workspace is the project's, and its home directory is
/// what `~` stands for (a resolver with none finds nothing in a root given
/// with `~`).
///
/// It reads the roots' folders and each extension's folder whole, and
/// nothing else. It blocks: call it on the blocking pool.
#[must_use]
pub fn discover(roots: &[Root], paths: &PathResolver) -> Discovered {
    let mut places = Places {
        workspace: paths.root().to_fs_path(),
        home: None,
    };
    let mut discovered = Discovered::default();
    let mut looked: Vec<(PathBuf, String)> = Vec::new();
    for root in roots {
        let below = below_workspace(&root.dir, &places.workspace);
        if certainly_absent(root, below.as_deref(), &places.workspace) {
            continue;
        }
        discovered.resolved += 1;
        let Some(opened) = open_root(
            root,
            below.as_deref(),
            paths,
            &mut places,
            &looked,
            &mut discovered.left_out,
        ) else {
            continue;
        };
        look_in(root, &opened, &mut discovered);
        looked.push((opened.real, opened.shown));
    }
    settle_names(&mut discovered);
    discovered
}

/// Leaves out the extensions one source's names make ambiguous, in the
/// order they were found (the [module](self) documentation says why): a
/// name a later root of the same source holds again (the first stands),
/// and every one of names that differ only in case (none stands).
fn settle_names(discovered: &mut Discovered) {
    let found = std::mem::take(&mut discovered.found);
    // Each source's names, their case folded: which extensions have each.
    let mut groups: BTreeMap<(Source, String), Vec<usize>> = BTreeMap::new();
    for (index, one) in found.iter().enumerate() {
        groups
            .entry((one.source, one.manifest.name.to_ascii_lowercase()))
            .or_default()
            .push(index);
    }
    let mut refused: Vec<Option<String>> = vec![None; found.len()];
    for members in groups.values() {
        let first = &found[members[0]];
        let whose = whose(first.source);
        let one_spelling = members
            .iter()
            .all(|&member| found[member].manifest.name == first.manifest.name);
        for (position, &member) in members.iter().enumerate() {
            let this = &found[member];
            if one_spelling {
                if position > 0 {
                    refused[member] = Some(format!(
                        "the {whose} extension `{}` was found first, in `{}`",
                        visible(&this.manifest.name),
                        first.shown
                    ));
                }
                continue;
            }
            let others: Vec<String> = members
                .iter()
                .filter(|&&other| other != member)
                .map(|&other| format!("`{}`", found[other].shown))
                .collect();
            let (those, none) = if others.len() == 1 {
                (format!("that of the {whose} extension"), "neither is used")
            } else {
                (
                    format!("those of the {whose} extensions"),
                    "none of them is used",
                )
            };
            refused[member] = Some(format!(
                "its name `{}` differs only in case from {those} {}: names that differ only in \
                 case are one name on Windows, where they would share what an extension keeps \
                 under its name, so {none}",
                visible(&this.manifest.name),
                listed(&others)
            ));
        }
    }
    for (one, refused) in found.into_iter().zip(refused) {
        match refused {
            None => discovered.found.push(one),
            Some(why) => discovered.left_out.push(LeftOut {
                reason: format!("the extension folder `{}` was left out: {why}", one.shown),
                source: one.source,
                folder: one.folder,
                shown: one.shown,
            }),
        }
    }
}

/// `a`, `a and b`, `a, b and c`.
fn listed(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The names of `dir` below the workspace, when it is written as a place
/// in it with no `..` and no `~`: relative to it, or absolute and under its
/// real location. `None` for any other.
fn below_workspace(dir: &Path, workspace: &Path) -> Option<Vec<OsString>> {
    let rest = if dir.is_absolute() {
        strip_location_prefix(dir, workspace)?
    } else {
        // `~` at the start is the home directory, not a name.
        if dir.components().next() == Some(Component::Normal(OsStr::new("~"))) {
            return None;
        }
        dir.to_path_buf()
    };
    let mut names = Vec::new();
    for component in rest.components() {
        match component {
            Component::Normal(name) => names.push(name.to_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(names)
}

/// Whether a root is certainly not there, decided with a look or two
/// that follow no link a repository holds (the [module](self)
/// documentation says which). `false` when it is there, or when the looks
/// cannot tell: the resolver decides then.
fn certainly_absent(root: &Root, below: Option<&[OsString]>, workspace: &Path) -> bool {
    match (root.source, below) {
        // A project's root in the workspace: one name at a time from its
        // real location, each looked at itself.
        (Source::Project, Some(names)) => {
            let mut at = workspace.to_path_buf();
            for name in names {
                at.push(name);
                match fs::symlink_metadata(&at) {
                    // A folder, and no link: the next name is looked at in it.
                    Ok(found) if found.is_dir() => {}
                    found => return missing(found),
                }
            }
            false
        }
        // The user's own: asked as it is.
        (Source::User, _) if root.dir.is_absolute() => missing(fs::symlink_metadata(&root.dir)),
        _ => false,
    }
}

/// Whether a look found nothing there.
fn missing(found: io::Result<fs::Metadata>) -> bool {
    matches!(found, Err(error) if error.kind() == io::ErrorKind::NotFound)
}

/// The places a path is shown relative to.
struct Places {
    /// The workspace's real location.
    workspace: PathBuf,
    /// The home directory's, looked up the first time a user's root is
    /// shown: `Some(None)` when there is none.
    home: Option<Option<PathBuf>>,
}

impl Places {
    /// `real` as the user is shown it: relative to the workspace for the
    /// project's, under `~` for the user's, with `/`, escaped.
    fn show(&mut self, source: Source, real: &AbsPath, paths: &PathResolver) -> String {
        let path = real.to_fs_path();
        let below = |base: &Path| strip_location_prefix(&path, base).map(|rest| slashed(&rest));
        let text = match source {
            Source::Project => below(&self.workspace).map(|rest| {
                if rest.is_empty() {
                    ".".to_owned()
                } else {
                    rest
                }
            }),
            Source::User => {
                let home = self.home.get_or_insert_with(|| {
                    paths
                        .resolve("~")
                        .ok()
                        .filter(cineris_base::ResolvedPath::exists)
                        .map(|home| home.to_fs_path())
                });
                home.as_deref().and_then(below).map(|rest| {
                    if rest.is_empty() {
                        "~".to_owned()
                    } else {
                        format!("~/{rest}")
                    }
                })
            }
        };
        visible(&text.unwrap_or_else(|| real.as_str().to_owned())).into_owned()
    }
}

/// The names of a relative path joined with `/`.
fn slashed(relative: &Path) -> String {
    relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Whose a root is, in words.
fn whose(source: Source) -> &'static str {
    match source {
        Source::User => "user's",
        Source::Project => "project's",
    }
}

/// A root that is there and is a folder.
struct Opened {
    /// Where it really is, for the file system's calls.
    real: PathBuf,
    /// How it is shown.
    shown: String,
}

/// Resolves a root: where it really is and how it is shown, or `None` when
/// there is nothing to look in (said in `left_out` when that is more than
/// a folder that is not there).
fn open_root(
    root: &Root,
    below: Option<&[OsString]>,
    paths: &PathResolver,
    places: &mut Places,
    looked: &[(PathBuf, String)],
    left_out: &mut Vec<LeftOut>,
) -> Option<Opened> {
    let written = visible(&root.dir.to_string_lossy()).into_owned();
    let refused = |shown: &str, folder: PathBuf, why: &str| LeftOut {
        source: root.source,
        folder,
        shown: shown.to_owned(),
        reason: format!(
            "the {} folder of extensions `{shown}` was not looked in: {why}",
            whose(root.source)
        ),
    };
    // A root under the workspace is resolved by its names below it: an
    // absolute one may be in a form the resolver does not take, such as
    // the workspace's real location on Windows (`\\?\C:\…`).
    let text = match below {
        Some(names) if root.dir.is_absolute() => names
            .iter()
            .map(|name| name.to_str())
            .collect::<Option<Vec<_>>>()
            // The workspace itself is `.`: the resolver refuses an empty path.
            .map(|names| {
                if names.is_empty() {
                    ".".to_owned()
                } else {
                    names.join("/")
                }
            }),
        _ => root.dir.to_str().map(str::to_owned),
    };
    let Some(text) = text else {
        left_out.push(refused(
            &written,
            root.dir.clone(),
            "its path is not valid Unicode",
        ));
        return None;
    };
    let resolved = match paths.resolve(&text) {
        Ok(resolved) => resolved,
        // No home directory is known: nothing is under `~`.
        Err(PathError::NoHome { .. }) => return None,
        Err(PathError::DanglingLink { .. }) => {
            left_out.push(refused(
                &written,
                root.dir.clone(),
                "it is a link to something that is not there",
            ));
            return None;
        }
        Err(error) => {
            // The resolver's messages escape what they quote.
            left_out.push(refused(&written, root.dir.clone(), &error.to_string()));
            return None;
        }
    };
    if !resolved.exists() {
        return None;
    }
    let real = resolved.to_fs_path();
    if root.source == Source::Project && !location_within(&real, &places.workspace) {
        left_out.push(refused(
            &written,
            root.dir.clone(),
            "it leads out of the workspace, and a project's extensions are read only from inside it",
        ));
        return None;
    }
    let shown = places.show(root.source, resolved.abs(), paths);
    match fs::metadata(&real) {
        Ok(found) if found.is_dir() => {}
        Ok(_) => {
            left_out.push(refused(&shown, real, "it is not a folder"));
            return None;
        }
        Err(error) => {
            let why = format!("it could not be read: {error}");
            left_out.push(refused(&shown, real, &why));
            return None;
        }
    }
    if let Some((_, first)) = looked
        .iter()
        .find(|(earlier, _)| same_location(earlier, &real))
    {
        let why = format!("it is the same folder as `{first}`, which was looked in instead");
        left_out.push(refused(&written, root.dir.clone(), &why));
        return None;
    }
    Some(Opened { real, shown })
}

/// Reads each folder of one root, by name, within the caps.
fn look_in(root: &Root, opened: &Opened, discovered: &mut Discovered) {
    let of_root = |reason: String| LeftOut {
        source: root.source,
        folder: opened.real.clone(),
        shown: opened.shown.clone(),
        reason,
    };
    let whose = whose(root.source);
    let shown = &opened.shown;
    let entries = match fs::read_dir(&opened.real) {
        Ok(entries) => entries,
        Err(error) => {
            discovered.left_out.push(of_root(format!(
                "the {whose} folder of extensions `{shown}` was not looked in: it could not be read: {error}"
            )));
            return;
        }
    };
    let mut names: Vec<(OsString, FileType)> = Vec::new();
    for entry in entries {
        if names.len() == MAX_ENTRIES {
            discovered.left_out.push(of_root(format!(
                "the {whose} folder of extensions `{shown}` holds more than {MAX_ENTRIES} entries, and the rest of it was not looked at"
            )));
            break;
        }
        // The entry's own kind: a link is not followed to say it.
        match entry.and_then(|entry| Ok((entry.file_name(), entry.file_type()?))) {
            Ok(named) => names.push(named),
            Err(error) => {
                discovered.left_out.push(of_root(format!(
                    "the {whose} folder of extensions `{shown}` could not be read to its end: {error}"
                )));
                break;
            }
        }
    }
    names.sort_by(|one, other| one.0.cmp(&other.0));
    let mut read = 0;
    let mut past = 0;
    for (name, kind) in names {
        // A file beside the folders, a FIFO, a socket: no extension.
        if !kind.is_dir() && !kind.is_symlink() {
            continue;
        }
        // No extension's name begins with `.`: a `.git`, an editor's folder.
        if name.as_encoded_bytes().first() == Some(&b'.') {
            continue;
        }
        if read == MAX_PER_ROOT {
            past += 1;
            continue;
        }
        read += 1;
        match extension_in(root, opened, &name, kind) {
            Ok(found) => discovered.found.push(found),
            Err(left) => discovered.left_out.push(left),
        }
    }
    if past > 0 {
        let (count, were) = if past == 1 {
            ("1 more folder".to_owned(), "it was")
        } else {
            (format!("{past} more folders"), "they were")
        };
        discovered.left_out.push(of_root(format!(
            "the {whose} folder of extensions `{shown}` holds {count} than the {MAX_PER_ROOT} read from it, the first by name: {were} left out"
        )));
    }
}

/// The extension in the folder `name` of a root, or what was left out and
/// why.
fn extension_in(
    root: &Root,
    opened: &Opened,
    name: &OsStr,
    kind: FileType,
) -> Result<Found, LeftOut> {
    let folder = opened.real.join(name);
    let shown = format!("{}/{}", opened.shown, visible(&name.to_string_lossy()));
    let refused = |why: &str| LeftOut {
        source: root.source,
        folder: folder.clone(),
        shown: shown.clone(),
        reason: format!("the extension folder `{shown}` was left out: {why}"),
    };
    let Some(text) = name.to_str() else {
        return Err(refused("its name is not valid Unicode"));
    };
    if kind.is_symlink() {
        return Err(refused(
            "it is a link, and an extension is read from a folder of its own in the folder of extensions, never through a link",
        ));
    }
    let snapshot =
        Snapshot::read(&folder).map_err(|error| refused(&visible(&error.to_string())))?;
    // `Snapshot::read` follows the folder itself: a folder swapped for a
    // link while it was read is refused as a link is.
    match fs::symlink_metadata(&folder) {
        Ok(found) if found.is_dir() => {}
        _ => {
            return Err(refused(
                "it changed while it was read, and is no longer a folder of its own",
            ));
        }
    }
    let manifest =
        Manifest::of(&snapshot).map_err(|error| refused(&visible(&error.to_string())))?;
    if manifest.name != text {
        let why = format!(
            "its manifest gives the name `{}`, and an extension's manifest (`{}`) gives its folder's own name, `{}`",
            visible(&manifest.name),
            manifest::FILE,
            visible(text)
        );
        return Err(refused(&why));
    }
    Ok(Found {
        source: root.source,
        folder,
        shown,
        snapshot: Arc::new(snapshot),
        manifest,
    })
}
