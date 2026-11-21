//! Skills: folders of instructions the model reads when a task calls for
//! them (`docs/architecture/12-extensions.md`, "Tier 2: skills";
//! `docs/steps/28/answered.md`, point 7).
//!
//! A skill is a folder holding a `SKILL.md` whose front matter gives a
//! `name` and a `description`: the open Agent Skills layout, read as it is,
//! so that people's existing skills work. [`find`] looks for them in the
//! roots it is given ([`Root::standard`] gives the six), and
//! [`render`] writes the part of section 6 of the system prompt that lists
//! them. Skills are text for the model, as instruction files are, and need
//! no trust: nothing here runs anything.
//!
//! **What is a skill.** `<root>/<folder>/SKILL.md`, one level down. A
//! `SKILL.md` directly in a root, or deeper than one folder, is not one, and
//! a folder with no `SKILL.md` is passed over without a word.
//!
//! **Paths are judged where they lead.** Every root, folder and file is
//! found through the session's path resolver ([`PathResolver`]), which takes
//! `~`, a relative path and `..`, follows each link and, on Windows, puts
//! every name in the case the file system stores. Two places are then
//! compared with [`location_within`] and its friends, never by their text.
//!
//! - **A root may be a link** the user made (`~/.claude/skills` often is),
//!   and is followed: the user named it. **A project's root must lead to a
//!   place inside the workspace;** one that leads out is left out with its
//!   reason, since what a repository's link reaches is not the repository's.
//! - **A skill's folder or its `SKILL.md` reached through a link** is
//!   followed only where it stays inside its root. A link that leads out, or
//!   to nothing, is left out with its reason.
//! - **The file is opened with [`open_regular`]**, after resolving, which
//!   refuses a link, a FIFO or a device at the last name, so what is read is
//!   what was checked.
//!
//! **The front matter** is read by [`FrontMatter::read`], a small reader of
//! the plain YAML skills are written in, with no YAML library: `key: value`
//! lines, quoted strings in both kinds of quotes, a value folded over
//! lines, and a `>` or `|` block. A file with no front matter, or without a
//! name, is left out with its reason. A missing description is an empty
//! one.
//!
//! **For the system prompt** each name and description is made one line,
//! with nothing in it that acts unseen ([`one_line`]): a line break or a
//! tab becomes a space, every other control character and every format
//! character (`Cf`: the bidirectional controls, the zero-width characters,
//! U+FEFF) is taken out, and the text is cut at a character's edge to
//! [`MAX_NAME_BYTES`] and [`MAX_DESCRIPTION_BYTES`]. The path shown is
//! relative to the workspace for a project's skill and under `~` for the
//! user's, never the home directory written out, with every character that
//! acts unseen written as an escape ([`cineris_base::visible`]), which the
//! resolver reads back when the model asks for the file.
//!
//! **Caps:** at most [`MAX_ENTRIES`] entries looked at in a root, a
//! `SKILL.md` of at most [`MAX_SKILL_BYTES`], and at most [`MAX_SKILLS`]
//! skills listed, the rest counted. **One name, one skill:** where two
//! skills have one name, the project's is taken over the user's, and
//! `.cineris/` over `.agents/` over `.claude/`; the other is left out with
//! its reason. **The order is fixed:** by name, then by root.
//!
//! **A root that is not there costs a look at each name on the way to it,
//! and is not resolved;** the home directory is resolved only when a
//! user's root is shown. [`find_seeing`] names each place to the caller
//! before it is looked at or read, where it really is and through no link
//! the walk does not follow, so that a session can stamp what its skills
//! were found in and see a change.
//!
//! Finding skills blocks: it reads folders and files. The caller runs
//! [`find`] on the blocking pool.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "finding skills reads folders and files; callers run it on the blocking pool"
)]

use std::ffi::{OsStr, OsString};
use std::fs::FileType;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use cineris_base::{
    AbsPath, OpenError, PathError, PathResolver, ResolvedPath, location_within, open_regular,
    same_location, strip_location_prefix, visible,
};

/// The file that makes a folder a skill.
pub const FILE_NAME: &str = "SKILL.md";
/// The most skills listed. The rest are counted, not listed.
pub const MAX_SKILLS: usize = 100;
/// The largest `SKILL.md` that is read. A larger one is left out, by its
/// length, unread.
pub const MAX_SKILL_BYTES: u64 = 64 * 1024;
/// The most bytes of `SKILL.md` files read in one root, every byte read
/// counting (of a file left out after its read too). Past it the root's
/// other skills are left out, unread, and that is said: a repository's
/// three roots can hold no more than this each, whether or not it is
/// trusted, and the first turn waits for what is read.
pub const MAX_ROOT_BYTES: u64 = 8 * 1024 * 1024;
/// The most entries looked at in one root. Every entry counts, a file
/// beside the skills' folders too, since each costs a look; past it the
/// rest of the root is not read, and that is said.
pub const MAX_ENTRIES: usize = 1_000;
/// The longest a skill's name is shown, in bytes.
pub const MAX_NAME_BYTES: usize = 64;
/// The longest a skill's description is shown, in bytes.
pub const MAX_DESCRIPTION_BYTES: usize = 300;
/// The cap on section 6 of the system prompt, in bytes.
pub const INDEX_CAP: usize = 8 * 1024;

/// Whose a root is.
///
/// The order is the order a name is taken in: the project's skill over the
/// user's of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Owner {
    /// The workspace's: its own folder of skills, text from the repository.
    Project,
    /// The user's, in their home directory.
    User,
}

/// Which tool's layout a root follows.
///
/// The order is the order a name is taken in: `.cineris/` over `.agents/`
/// over `.claude/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layout {
    /// `.cineris/skills/`, Cineris's own.
    Cineris,
    /// `.agents/skills/`, read when `compat.skills` is on.
    Agents,
    /// `.claude/skills/`, read when `compat.skills` is on.
    Claude,
}

impl Layout {
    /// Every layout, in the order a name is taken in.
    pub const ALL: [Self; 3] = [Self::Cineris, Self::Agents, Self::Claude];

    /// The folder its skills are in, relative to the workspace or the home
    /// directory, with `/`.
    #[must_use]
    pub fn folder(self) -> &'static str {
        match self {
            Self::Cineris => ".cineris/skills",
            Self::Agents => ".agents/skills",
            Self::Claude => ".claude/skills",
        }
    }

    /// Whether it is another tool's, read only when `compat.skills` is on.
    #[must_use]
    pub fn is_compat(self) -> bool {
        self != Self::Cineris
    }
}

/// A folder to look for skills in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// Whose it is.
    pub owner: Owner,
    /// Which layout it follows.
    pub layout: Layout,
    /// Where it is, as a path is given to the session's resolver: relative
    /// to the workspace, beginning with `~` for the home directory, or
    /// absolute. `..` and links are taken as the resolver takes them.
    pub path: String,
}

impl Root {
    /// A root at `path`.
    #[must_use]
    pub fn new(owner: Owner, layout: Layout, path: impl Into<String>) -> Self {
        Self {
            owner,
            layout,
            path: path.into(),
        }
    }

    /// The roots skills are looked for in: `.cineris/skills/`,
    /// `.agents/skills/` and `.claude/skills/` in the workspace, then the
    /// same three in the home directory. Without `compat` (the setting
    /// `compat.skills` off) only the two `.cineris/skills/`. In the order a
    /// name is taken in.
    #[must_use]
    pub fn standard(compat: bool) -> Vec<Self> {
        let mut roots = Vec::new();
        for owner in [Owner::Project, Owner::User] {
            for layout in Layout::ALL {
                if layout.is_compat() && !compat {
                    continue;
                }
                let path = match owner {
                    Owner::Project => layout.folder().to_owned(),
                    Owner::User => format!("~/{}", layout.folder()),
                };
                roots.push(Self::new(owner, layout, path));
            }
        }
        roots
    }
}

/// A skill that was found, as section 6 lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Its name, one line ([`one_line`]), at most [`MAX_NAME_BYTES`].
    pub name: String,
    /// Its description, one line, at most [`MAX_DESCRIPTION_BYTES`]. Empty
    /// when the front matter gives none.
    pub description: String,
    /// Its `SKILL.md` as the model is shown it: relative to the workspace
    /// for a project's skill, under `~` for the user's (absolute only for a
    /// user's root that leads out of the home directory), with `/`, and
    /// every character that acts unseen written as an escape.
    pub path: String,
    /// Whose root it was found in.
    pub owner: Owner,
    /// Which layout that root follows.
    pub layout: Layout,
    /// Where its folder really is, every link followed.
    pub folder: AbsPath,
}

/// Something that was looked at and is not listed, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    /// What it is, as the user is shown it: a root, a folder in one, or a
    /// `SKILL.md`, in the form [`Skill::path`] has.
    pub path: String,
    /// Why it is not listed.
    pub reason: Reason,
}

/// Why a root, a folder or a `SKILL.md` was left out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Reason {
    /// It could not be read.
    #[error("it could not be read: {0}")]
    Unreadable(String),
    /// A project's root that leads out of the workspace.
    #[error("it leads out of the workspace, and a project's skills are read only from inside it")]
    OutsideWorkspace,
    /// A root that is not a folder.
    #[error("it is not a folder")]
    NotAFolder,
    /// A root that is the same folder as one looked at before it.
    #[error("it is the same folder as `{0}`, which was looked at instead")]
    SameFolder(String),
    /// A root with more than [`MAX_ENTRIES`] entries.
    #[error("it holds more than {MAX_ENTRIES} entries, and the rest were not looked at")]
    TooManyEntries,
    /// A name that is not valid Unicode.
    #[error("its name is not valid Unicode")]
    NotUnicode,
    /// A link whose target is not there.
    #[error("it is a link to something that is not there")]
    Dangling,
    /// A link that leads out of the root it is in, or to the root itself.
    #[error("it is a link that leads out of the folder of skills it is in")]
    LinkOut,
    /// A `SKILL.md` that is not a regular file: a folder, a FIFO, a device.
    #[error("it is not a regular file")]
    NotAFile,
    /// A `SKILL.md` larger than [`MAX_SKILL_BYTES`].
    #[error("it is larger than 64 KiB")]
    TooLarge,
    /// A `SKILL.md` past what its root may read ([`MAX_ROOT_BYTES`]).
    #[error("its folder of skills had 8 MiB of SKILL.md files read before it, and it was not read")]
    PastTheRootsBytes,
    /// A `SKILL.md` that does not begin with front matter.
    #[error("it has no front matter: a block between two `---` lines at its top")]
    NoFrontMatter,
    /// Front matter with no `name`, or one that is empty once it is one
    /// line.
    #[error("its front matter gives no `name`")]
    NoName,
    /// A skill whose name another skill has, which was taken first.
    #[error("the skill at `{kept}` has the name `{name}` too, and is taken first")]
    Duplicate {
        /// The name, as it is shown.
        name: String,
        /// The path of the skill taken.
        kept: String,
    },
}

/// What [`find`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The skills, by name, at most [`MAX_SKILLS`].
    pub skills: Vec<Skill>,
    /// How many more skills there were past [`MAX_SKILLS`].
    pub not_listed: usize,
    /// What was left out, and why: in the order the roots were looked at,
    /// each root's entries by name, then the duplicates.
    pub left_out: Vec<LeftOut>,
}

/// Looks for skills in `roots`, through `resolver`, which is the session's:
/// its workspace is the project's, and its home directory is what `~`
/// stands for (a resolver with none finds none of the user's).
///
/// It reads the roots' folders and the `SKILL.md` files, and nothing else.
/// It blocks: call it on the blocking pool.
#[must_use]
pub fn find(resolver: &PathResolver, roots: &[Root]) -> Found {
    find_seeing(resolver, roots, &mut |_| {})
}

/// A place [`find_seeing`] is about to look at or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen<'a> {
    /// A name on the way to a root, or where a root that is not there
    /// would be: only its kind matters (there, not there, a folder, a
    /// link), and a file made in it beside the root is no change of it.
    Way(&'a Path),
    /// A place what was found depends on: a root where it really is, an
    /// entry of it, the folder a link in it leads to, a `SKILL.md`.
    Place(&'a Path),
}

/// A point of [`find`]: a name on the way to a root, looked at itself to
/// rule the root out (`cineris_base::hook`). A test counts what a session
/// looks at with it.
pub const A_NAME_IS_LOOKED_AT: &str = "skills: a name on the way to a root is looked at";

/// A point of [`find`]: a root is resolved through the path resolver,
/// since a look could not rule it out.
pub const A_ROOT_IS_RESOLVED: &str = "skills: a root is resolved";

/// A point of [`find`]: a `SKILL.md` is about to be read.
pub const A_SKILL_IS_READ: &str = "skills: a SKILL.md is about to be read";

/// [`find`], with each place given to `seen` **before** it is looked at or
/// read, so that the caller can stamp it for a change (the core's
/// hot-reload): each name on the way to a root as written, looked at
/// itself; where each root really is, before it is listed; each entry of
/// it that may be a skill's folder, itself, before it is looked into; where
/// such a folder reached through a link really is; and each `SKILL.md`
/// where it really is, before it is read. A place is never one reached
/// through a link the walk does not follow. A root gives at most 3 names on
/// the way, itself, and three places for each of its [`MAX_ENTRIES`]
/// entries.
///
/// **A root that is not there costs a look at each name on the way to it,
/// and is not resolved:** the names below the workspace, or below the home
/// directory as it was given to the resolver, are looked at one at a time,
/// each itself, and the first that is not there rules the root out. One
/// that is a link, or anything but a folder, leaves it to the resolver,
/// which follows a link and judges where it leads. The home directory is
/// resolved only when a user's root is shown.
pub fn find_seeing(
    resolver: &PathResolver,
    roots: &[Root],
    seen: &mut dyn FnMut(Seen<'_>),
) -> Found {
    find_within(resolver, roots, seen, MAX_ROOT_BYTES)
}

/// [`find_seeing`], a root reading at most `max_root_bytes` of `SKILL.md`
/// files: [`MAX_ROOT_BYTES`], or less for a test.
fn find_within(
    resolver: &PathResolver,
    roots: &[Root],
    seen: &mut dyn FnMut(Seen<'_>),
    max_root_bytes: u64,
) -> Found {
    let places = Places {
        workspace: resolver.root().to_fs_path(),
        home: std::cell::OnceCell::new(),
        resolver,
    };
    let mut left_out = Vec::new();
    let mut candidates = Vec::new();
    let mut looked: Vec<(PathBuf, String)> = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        if certainly_absent(resolver, root, seen) {
            continue;
        }
        cineris_base::hook::point(A_ROOT_IS_RESOLVED);
        let Some((real, shown)) = open_root(resolver, root, &places, &looked, &mut left_out, seen)
        else {
            continue;
        };
        seen(Seen::Place(&real.to_fs_path()));
        look_in(
            &Look {
                resolver,
                root,
                index,
                real: &real,
                shown: &shown,
                left: std::cell::Cell::new(max_root_bytes),
            },
            &mut candidates,
            &mut left_out,
            seen,
        );
        looked.push((real.to_fs_path(), shown));
    }
    // By name, then by the root a name is taken from first.
    candidates.sort_by(|one: &Candidate, other: &Candidate| {
        one.skill
            .name
            .cmp(&other.skill.name)
            .then_with(|| one.skill.owner.cmp(&other.skill.owner))
            .then_with(|| one.skill.layout.cmp(&other.skill.layout))
            .then_with(|| one.root.cmp(&other.root))
            .then_with(|| one.skill.path.cmp(&other.skill.path))
    });
    let mut skills: Vec<Skill> = Vec::new();
    for Candidate { skill, .. } in candidates {
        if let Some(kept) = skills.last()
            && kept.name == skill.name
        {
            left_out.push(LeftOut {
                path: skill.path,
                reason: Reason::Duplicate {
                    name: skill.name,
                    kept: kept.path.clone(),
                },
            });
            continue;
        }
        skills.push(skill);
    }
    let not_listed = skills.len().saturating_sub(MAX_SKILLS);
    skills.truncate(MAX_SKILLS);
    Found {
        skills,
        not_listed,
        left_out,
    }
}

/// Whether a root is certainly not there, decided with a look at each name
/// on the way to it, each looked at itself and given to `seen` first.
/// `false` when it is there, or when the looks cannot tell (a link, a
/// name that is not a folder, a `..`): the resolver decides then. A user's
/// root with no home directory is not there.
fn certainly_absent(resolver: &PathResolver, root: &Root, seen: &mut dyn FnMut(Seen<'_>)) -> bool {
    // The separators the resolver reads: `/`, and `\` too on Windows.
    let separator = |character: char| character == '/' || (cfg!(windows) && character == '\\');
    let path = Path::new(&root.path);
    let home_rest = root
        .path
        .strip_prefix('~')
        .filter(|rest| rest.is_empty() || rest.starts_with(separator));
    let (mut at, rest) = if let Some(rest) = home_rest {
        match resolver.home() {
            Some(home) => (home.to_fs_path(), rest),
            None => return true,
        }
    } else if path.is_absolute() {
        let at = PathBuf::from(&root.path);
        seen(Seen::Way(&at));
        cineris_base::hook::point(A_NAME_IS_LOOKED_AT);
        return missing(std::fs::symlink_metadata(&at));
    } else if path.has_root() || matches!(path.components().next(), Some(Component::Prefix(_))) {
        // Rooted on Windows without being absolute (`\x`, `C:x`): the
        // resolver reads it.
        return false;
    } else {
        (resolver.root().to_fs_path(), root.path.as_str())
    };
    for name in rest.split(separator) {
        match name {
            "" | "." => continue,
            ".." => return false,
            name => at.push(name),
        }
        seen(Seen::Way(&at));
        cineris_base::hook::point(A_NAME_IS_LOOKED_AT);
        match std::fs::symlink_metadata(&at) {
            // A folder, and no link: the next name is looked at in it.
            Ok(found) if found.is_dir() => {}
            found => return missing(found),
        }
    }
    false
}

/// Whether a look found nothing there.
fn missing(found: std::io::Result<std::fs::Metadata>) -> bool {
    matches!(found, Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

/// The places a path is shown relative to, as the file system's calls
/// take them.
struct Places<'a> {
    workspace: PathBuf,
    /// The home directory's real location, resolved the first time a
    /// user's root is shown: `None` within when there is none.
    home: std::cell::OnceCell<Option<PathBuf>>,
    resolver: &'a PathResolver,
}

impl Places<'_> {
    /// Where the home directory really is.
    fn home(&self) -> Option<&Path> {
        self.home
            .get_or_init(|| {
                self.resolver
                    .resolve("~")
                    .ok()
                    .filter(ResolvedPath::exists)
                    .map(|home| home.to_fs_path())
            })
            .as_deref()
    }

    /// `path` as the model and the user are shown it: relative to the
    /// workspace for a project's, under `~` for the user's, escaped.
    fn show(&self, owner: Owner, path: &AbsPath) -> String {
        let real = path.to_fs_path();
        let below = |base: &Path| strip_location_prefix(&real, base).map(|rest| slashed(&rest));
        let text = match owner {
            Owner::Project => below(&self.workspace).map(|rest| {
                if rest.is_empty() {
                    ".".to_owned()
                } else {
                    rest
                }
            }),
            Owner::User => self.home().and_then(below).map(|rest| {
                if rest.is_empty() {
                    "~".to_owned()
                } else {
                    format!("~/{rest}")
                }
            }),
        };
        visible(&text.unwrap_or_else(|| path.as_str().to_owned())).into_owned()
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

/// A skill found, before the duplicates are taken out.
struct Candidate {
    skill: Skill,
    /// The index of its root among those given.
    root: usize,
}

/// One root being looked in.
struct Look<'a> {
    resolver: &'a PathResolver,
    root: &'a Root,
    index: usize,
    /// Where the root really is.
    real: &'a AbsPath,
    /// The root as it is shown.
    shown: &'a str,
    /// What the root may still read of its `SKILL.md` files.
    left: std::cell::Cell<u64>,
}

/// Resolves a root: its real location and how it is shown, or `None` when
/// there is nothing to look in (said in `left_out` when it is more than an
/// absent folder).
fn open_root(
    resolver: &PathResolver,
    root: &Root,
    places: &Places,
    looked: &[(PathBuf, String)],
    left_out: &mut Vec<LeftOut>,
    seen: &mut dyn FnMut(Seen<'_>),
) -> Option<(AbsPath, String)> {
    let written = || visible(&root.path).into_owned();
    let resolved = match resolver.resolve(&root.path) {
        Ok(resolved) => resolved,
        // No home directory is known: there is no user's root to look in.
        Err(PathError::NoHome { .. }) => return None,
        Err(PathError::DanglingLink { path }) => {
            // Where the link leads, which is not there: named, so that the
            // root made there later is seen.
            let link = Path::new(&path);
            if let Ok(target) = std::fs::read_link(link) {
                let target = link
                    .parent()
                    .map_or(target.clone(), |parent| parent.join(&target));
                seen(Seen::Way(&target));
            }
            left_out.push(LeftOut {
                path: written(),
                reason: Reason::Dangling,
            });
            return None;
        }
        Err(error) => {
            left_out.push(LeftOut {
                path: written(),
                reason: path_reason(error),
            });
            return None;
        }
    };
    if !resolved.exists() {
        // Where it would be, through the links on the way: named, so that a
        // root made there later is seen.
        seen(Seen::Way(&resolved.to_fs_path()));
        return None;
    }
    let real = resolved.to_fs_path();
    if root.owner == Owner::Project && !location_within(&real, &places.workspace) {
        left_out.push(LeftOut {
            path: written(),
            reason: Reason::OutsideWorkspace,
        });
        return None;
    }
    let shown = places.show(root.owner, resolved.abs());
    match std::fs::metadata(&real) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            left_out.push(LeftOut {
                path: shown,
                reason: Reason::NotAFolder,
            });
            return None;
        }
        Err(error) => {
            left_out.push(LeftOut {
                path: shown,
                reason: Reason::Unreadable(error.to_string()),
            });
            return None;
        }
    }
    if let Some((_, first)) = looked
        .iter()
        .find(|(earlier, _)| same_location(earlier, &real))
    {
        left_out.push(LeftOut {
            path: written(),
            reason: Reason::SameFolder(first.clone()),
        });
        return None;
    }
    Some((resolved.abs().clone(), shown))
}

/// Why a path could not be resolved, as a reason.
fn path_reason(error: PathError) -> Reason {
    match error {
        PathError::DanglingLink { .. } => Reason::Dangling,
        PathError::Io { source, .. } => Reason::Unreadable(source.to_string()),
        other => Reason::Unreadable(other.to_string()),
    }
}

/// Looks at each entry of one root, by name, up to [`MAX_ENTRIES`].
fn look_in(
    look: &Look<'_>,
    candidates: &mut Vec<Candidate>,
    left_out: &mut Vec<LeftOut>,
    seen: &mut dyn FnMut(Seen<'_>),
) {
    let entries = match std::fs::read_dir(look.real.to_fs_path()) {
        Ok(entries) => entries,
        Err(error) => {
            left_out.push(LeftOut {
                path: look.shown.to_owned(),
                reason: Reason::Unreadable(error.to_string()),
            });
            return;
        }
    };
    let mut names: Vec<(OsString, FileType)> = Vec::new();
    for entry in entries {
        if names.len() == MAX_ENTRIES {
            left_out.push(LeftOut {
                path: look.shown.to_owned(),
                reason: Reason::TooManyEntries,
            });
            break;
        }
        let kind = entry.and_then(|entry| Ok((entry.file_name(), entry.file_type()?)));
        match kind {
            Ok(named) => names.push(named),
            Err(error) => {
                left_out.push(LeftOut {
                    path: look.shown.to_owned(),
                    reason: Reason::Unreadable(error.to_string()),
                });
                break;
            }
        }
    }
    names.sort_by(|one, other| one.0.cmp(&other.0));
    for (name, kind) in names {
        // A file beside the skills' folders, a FIFO or a socket: no skill.
        if !kind.is_dir() && !kind.is_symlink() {
            continue;
        }
        let named = format!("{}/{}", look.shown, visible(&name.to_string_lossy()));
        // The entry itself, before it is looked into: a `SKILL.md` made in
        // it, or the folder swapped for a link, is a change.
        seen(Seen::Place(&look.real.to_fs_path().join(&name)));
        match skill_in(look, &name, &named, seen) {
            Ok(Some(skill)) => candidates.push(Candidate {
                skill,
                root: look.index,
            }),
            Ok(None) => {}
            Err(left) => left_out.push(left),
        }
    }
}

/// The skill in the folder `name` of a root: `None` when the folder is not
/// a skill's (no `SKILL.md`, or a link to a file), and what was left out
/// and why when it is one that cannot be taken.
fn skill_in(
    look: &Look<'_>,
    name: &OsStr,
    named: &str,
    seen: &mut dyn FnMut(Seen<'_>),
) -> Result<Option<Skill>, LeftOut> {
    let left = |path: &str, reason: Reason| LeftOut {
        path: path.to_owned(),
        reason,
    };
    if name.to_str().is_none() {
        return Err(left(named, Reason::NotUnicode));
    }
    let entry = look
        .real
        .child(name)
        .map_err(|error| left(named, path_reason(error)))?;
    let folder = look
        .resolver
        .resolve(entry.as_str())
        .map_err(|error| left(named, path_reason(error)))?;
    if !folder.exists() {
        // Gone since the folder was listed.
        return Ok(None);
    }
    let root = look.real.to_fs_path();
    // Below the root: a link to a place outside it, or to the root itself,
    // is not followed.
    let inside = strip_location_prefix(&folder.to_fs_path(), &root)
        .is_some_and(|rest| rest.components().next().is_some());
    if !inside {
        return Err(left(named, Reason::LinkOut));
    }
    // Where a folder reached through a link really is: what is made in it
    // is a change too.
    seen(Seen::Place(&folder.to_fs_path()));
    match std::fs::metadata(folder.to_fs_path()) {
        Ok(metadata) if metadata.is_dir() => {}
        // A link to a file inside the root: not a skill's folder.
        Ok(_) => return Ok(None),
        Err(error) => return Err(left(named, Reason::Unreadable(error.to_string()))),
    }
    let file_shown = format!("{named}/{FILE_NAME}");
    let file = folder
        .abs()
        .join(FILE_NAME)
        .map_err(|error| left(&file_shown, path_reason(error)))?;
    let file = look
        .resolver
        .resolve(file.as_str())
        .map_err(|error| left(&file_shown, path_reason(error)))?;
    if !file.exists() {
        // A folder with no `SKILL.md` is not a skill.
        return Ok(None);
    }
    if !location_within(&file.to_fs_path(), &root) {
        return Err(left(&file_shown, Reason::LinkOut));
    }
    seen(Seen::Place(&file.to_fs_path()));
    cineris_base::hook::point(A_SKILL_IS_READ);
    let text = match read_capped(&file.to_fs_path(), &look.left) {
        Ok(Some(text)) => text,
        Ok(None) => return Ok(None),
        Err(reason) => return Err(left(&file_shown, reason)),
    };
    let Some(front) = FrontMatter::read(&text) else {
        return Err(left(&file_shown, Reason::NoFrontMatter));
    };
    let skill_name = one_line(front.get("name").unwrap_or_default(), MAX_NAME_BYTES);
    if skill_name.is_empty() {
        return Err(left(&file_shown, Reason::NoName));
    }
    Ok(Some(Skill {
        name: skill_name,
        description: one_line(
            front.get("description").unwrap_or_default(),
            MAX_DESCRIPTION_BYTES,
        ),
        path: file_shown,
        owner: look.root.owner,
        layout: look.root.layout,
        folder: folder.abs().clone(),
    }))
}

/// Reads a `SKILL.md` already resolved, opened so that nothing at the last
/// name but a regular file is read. `None` when it is gone. One larger than
/// [`MAX_SKILL_BYTES`] by its length, or than what its root may still read
/// (`left`), is refused unread; every byte read is taken from `left`.
fn read_capped(path: &Path, left: &std::cell::Cell<u64>) -> Result<Option<String>, Reason> {
    let (file, metadata) = match open_regular(path) {
        Ok(opened) => opened,
        Err(OpenError::NotFound) => return Ok(None),
        Err(OpenError::Directory | OpenError::NotAFile) => return Err(Reason::NotAFile),
        // Swapped for a link since it was resolved.
        Err(OpenError::Link) => return Err(Reason::LinkOut),
        Err(error) => return Err(Reason::Unreadable(error.to_string())),
    };
    // One test of size for the length and for what is read past it.
    let too_large = |len: u64| len > MAX_SKILL_BYTES;
    if too_large(metadata.len()) {
        return Err(Reason::TooLarge);
    }
    let before = left.get();
    if metadata.len() > before {
        return Err(Reason::PastTheRootsBytes);
    }
    // One byte past either cap says the file is larger, whatever its
    // metadata said a moment before.
    let mut bytes = Vec::new();
    let read = file
        .take(MAX_SKILL_BYTES.min(before) + 1)
        .read_to_end(&mut bytes);
    left.set(before.saturating_sub(bytes.len() as u64));
    read.map_err(|error| Reason::Unreadable(error.to_string()))?;
    if too_large(bytes.len() as u64) {
        return Err(Reason::TooLarge);
    }
    if bytes.len() as u64 > before {
        // It grew past what the root may read since its length was taken.
        return Err(Reason::PastTheRootsBytes);
    }
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}

/// `text` as one line for the system prompt, at most `max_bytes` long: the
/// rule's one home is `cineris_base`, which the agent's index of tool
/// sources is shown by too.
pub use cineris_base::one_line;

/// The front matter at the top of a Markdown file: a block between two
/// `---` lines, the first line of the file, holding `key: value` lines.
///
/// A small reader of the plain YAML such blocks are written in, not of
/// YAML: no library is pulled in for two strings. What it reads:
///
/// - **A field** is a line that begins at its first column with a key and
///   a `:` followed by white space or the line's end. The key may be in
///   quotes. Lines that begin with white space after it, and blank lines,
///   are its value's. A line that begins with `#` is a comment. A key given
///   twice keeps its first value.
/// - **A plain value** is the rest of the line and its value's lines, each
///   trimmed and joined by a space (a blank line between them is a line
///   break), with a comment (` #` and what follows) taken off each.
/// - **A quoted value,** in `"` or `'`, may run over lines, which are
///   joined as YAML folds them. In `"`, the escapes are read (`\"`, `\\`,
///   `\n`, `\t`, `\xXX`, `\uXXXX`, `\UXXXXXXXX` and the others); in `'`, a
///   doubled `''` is one quote. What follows the closing quote is ignored;
///   a value whose quote is never closed is taken as far as it goes.
/// - **A block,** `|` or `>` with any chomping (`+`, `-`) and indentation
///   (a digit) after it: its lines, less their indentation, kept as lines
///   for `|` and folded into one for `>` (a blank line being a line break).
///
/// Flow collections (`[a, b]`, `{a: b}`), anchors, tags and nested maps
/// are not read: such a value is the text as it stands, and a nested
/// map's lines belong to its key.
///
/// The file may begin with a byte-order mark, and its lines may end with
/// `\r\n`. The closing line is `---` or `...`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrontMatter {
    fields: Vec<(String, String)>,
}

impl FrontMatter {
    /// The front matter of `text`: `None` when its first line is not
    /// `---`, or the block is never closed.
    #[must_use]
    pub fn read(text: &str) -> Option<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut lines = text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line));
        if lines.next()?.trim_end() != "---" {
            return None;
        }
        let mut body = Vec::new();
        for line in lines {
            if matches!(line.trim_end(), "---" | "...") {
                return Some(Self {
                    fields: fields_of(&body),
                });
            }
            body.push(line);
        }
        None
    }

    /// The value of the field `key`, as text.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// Every field's key and value, in the order the block gives them.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

/// The fields of a block's lines.
fn fields_of(body: &[&str]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut at = 0;
    while at < body.len() {
        let line = body[at];
        at += 1;
        if line.trim().is_empty() || indented(line) || line.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = split_key(line) else {
            continue;
        };
        let start = at;
        while at < body.len() && (body[at].trim().is_empty() || indented(body[at])) {
            at += 1;
        }
        if out.iter().any(|(earlier, _)| *earlier == key) {
            continue;
        }
        let value = scalar(rest, &body[start..at]);
        out.push((key, value));
    }
    out
}

fn indented(line: &str) -> bool {
    line.starts_with([' ', '\t'])
}

/// A field's key, and the rest of its line after the `:`.
fn split_key(line: &str) -> Option<(String, &str)> {
    let (key, after) = match line.chars().next() {
        Some(quote @ ('"' | '\'')) => {
            let inner = &line[1..];
            let end = inner.find(quote)?;
            (
                &inner[..end],
                inner[end + 1..].trim_start_matches([' ', '\t']),
            )
        }
        _ => {
            let bytes = line.as_bytes();
            let colon = (0..bytes.len()).find(|&index| {
                bytes[index] == b':'
                    && bytes
                        .get(index + 1)
                        .is_none_or(|next| matches!(next, b' ' | b'\t'))
            })?;
            (&line[..colon], &line[colon..])
        }
    };
    let after = after.strip_prefix(':')?;
    if !(after.is_empty() || after.starts_with([' ', '\t'])) {
        return None;
    }
    Some((key.trim().to_owned(), after.trim_start()))
}

/// A value: the rest of its key's line, and the lines that follow it.
fn scalar(first: &str, more: &[&str]) -> String {
    match first.chars().next() {
        Some('|' | '>') if block_header(first) => block(first, more),
        Some(quote @ ('"' | '\'')) => quoted(&first[1..], more, quote),
        _ => plain(first, more),
    }
}

/// Whether `first` is a block's header: `|` or `>`, then only chomping and
/// indentation indicators, then nothing but a comment.
fn block_header(first: &str) -> bool {
    let rest =
        first[1..].trim_start_matches(['+', '-', '1', '2', '3', '4', '5', '6', '7', '8', '9']);
    let rest = rest.trim_start();
    rest.is_empty() || rest.starts_with('#')
}

/// A `|` or `>` block.
fn block(header: &str, lines: &[&str]) -> String {
    let literal = header.starts_with('|');
    let given = header[1..]
        .chars()
        .take_while(|character| matches!(character, '+' | '-' | '1'..='9'))
        .find_map(|character| character.to_digit(10))
        .and_then(|digit| usize::try_from(digit).ok());
    let leading = |line: &str| line.len() - line.trim_start_matches([' ', '\t']).len();
    let indent = given.unwrap_or_else(|| {
        lines
            .iter()
            .find(|line| !line.trim().is_empty())
            .map_or(0, |line| leading(line))
    });
    let mut content: Vec<&str> = lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                ""
            } else {
                // At most the block's indentation, never into the text.
                &line[leading(line).min(indent)..]
            }
        })
        .collect();
    while content.last() == Some(&"") {
        content.pop();
    }
    if literal {
        return content.join("\n");
    }
    let mut out = String::new();
    let mut breaks = 0;
    for line in content {
        if line.is_empty() {
            breaks += 1;
            continue;
        }
        if !out.is_empty() {
            if breaks == 0 {
                out.push(' ');
            }
            for _ in 0..breaks {
                out.push('\n');
            }
        }
        breaks = 0;
        out.push_str(line);
    }
    out
}

/// A value in quotes, begun on its key's line (`first`, after the opening
/// quote) and perhaps running over `more`.
fn quoted(first: &str, more: &[&str], quote: char) -> String {
    let mut text = first.to_owned();
    for line in more {
        text.push('\n');
        text.push_str(line);
    }
    let mut chars = text.chars().peekable();
    let mut out = String::new();
    while let Some(character) = chars.next() {
        match character {
            '\'' if quote == '\'' && chars.peek() == Some(&'\'') => {
                chars.next();
                out.push('\'');
            }
            found if found == quote => return out,
            '\\' if quote == '"' => escape(&mut chars, &mut out),
            '\n' => {
                // A line break folds: the white space around it goes, and
                // it is a space, or a line break for each blank line.
                while out.ends_with([' ', '\t']) {
                    out.pop();
                }
                let mut breaks = 0;
                loop {
                    while matches!(chars.peek(), Some(' ' | '\t')) {
                        chars.next();
                    }
                    if chars.peek() != Some(&'\n') {
                        break;
                    }
                    chars.next();
                    breaks += 1;
                }
                if breaks == 0 {
                    out.push(' ');
                }
                for _ in 0..breaks {
                    out.push('\n');
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// One escape of a `"` string, its backslash read.
fn escape(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, out: &mut String) {
    let Some(code) = chars.next() else {
        out.push('\\');
        return;
    };
    let simple = match code {
        '0' => Some('\0'),
        'a' => Some('\u{7}'),
        'b' => Some('\u{8}'),
        't' | '\t' => Some('\t'),
        'n' => Some('\n'),
        'v' => Some('\u{b}'),
        'f' => Some('\u{c}'),
        'r' => Some('\r'),
        'e' => Some('\u{1b}'),
        ' ' => Some(' '),
        '"' => Some('"'),
        '/' => Some('/'),
        '\\' => Some('\\'),
        'N' => Some('\u{85}'),
        '_' => Some('\u{a0}'),
        'L' => Some('\u{2028}'),
        'P' => Some('\u{2029}'),
        _ => None,
    };
    if let Some(simple) = simple {
        out.push(simple);
        return;
    }
    let digits = match code {
        'x' => 2,
        'u' => 4,
        'U' => 8,
        '\n' => {
            // A line that ends in a backslash goes on with the next, with
            // no space between.
            while matches!(chars.peek(), Some(' ' | '\t')) {
                chars.next();
            }
            return;
        }
        other => {
            out.push('\\');
            out.push(other);
            return;
        }
    };
    let mut hex = String::new();
    while hex.len() < digits
        && let Some(digit) = chars.next_if(char::is_ascii_hexdigit)
    {
        hex.push(digit);
    }
    match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
        Some(decoded) if hex.len() == digits => out.push(decoded),
        _ => {
            out.push('\\');
            out.push(code);
            out.push_str(&hex);
        }
    }
}

/// A plain value: its lines trimmed and joined.
fn plain(first: &str, more: &[&str]) -> String {
    let mut out = String::new();
    let mut breaks = 0;
    for line in std::iter::once(first).chain(more.iter().copied()) {
        let text = without_comment(line.trim_start()).trim_end();
        if text.is_empty() {
            if !out.is_empty() {
                breaks += 1;
            }
            continue;
        }
        if !out.is_empty() {
            if breaks == 0 {
                out.push(' ');
            }
            for _ in 0..breaks {
                out.push('\n');
            }
        }
        breaks = 0;
        out.push_str(text);
    }
    out
}

/// A line without its comment: a `#` at its start, or after white space,
/// and what follows it.
fn without_comment(line: &str) -> &str {
    let mut after_space = true;
    for (index, character) in line.char_indices() {
        if character == '#' && after_space {
            return &line[..index];
        }
        after_space = character == ' ' || character == '\t';
    }
    line
}

/// The first line of section 6's list of skills.
pub const HEADING: &str = "Skills: instructions for particular kinds of task, each in a folder \
    of the user's or the project's with a SKILL.md file. When the task at hand matches a \
    skill's description, read its SKILL.md before you begin, and the files it points to as \
    you need them. Each line gives a skill's name, its description and the path to its \
    SKILL.md.";

/// Room kept for the line that says skills were not listed.
const CUT_ROOM: usize = 96;

/// Section 6's list of skills, rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    /// The text. Empty when there is no skill to list, or not even one fits.
    pub text: String,
    /// How many skills it lists.
    pub listed: usize,
    /// How many skills it counts and does not list: those past
    /// [`MAX_SKILLS`], and those the cap left out.
    pub not_listed: usize,
}

/// One skill's line: `- name: description (path)`, or `- name (path)` with
/// no description.
fn line(skill: &Skill) -> String {
    if skill.description.is_empty() {
        format!("- {} ({})", skill.name, skill.path)
    } else {
        format!("- {}: {} ({})", skill.name, skill.description, skill.path)
    }
}

/// Renders the skills of `found` as section 6 of the system prompt lists
/// them, in at most `cap` bytes ([`INDEX_CAP`] is the section's):
///
/// ```text
/// Skills: instructions for particular kinds of task, … the path to its SKILL.md.
/// - pdf: Fill in and merge PDF forms. (.cineris/skills/pdf/SKILL.md)
/// - review (~/.claude/skills/review/SKILL.md)
/// (2 more skills are not listed, to keep this within its size.)
/// ```
///
/// [`HEADING`], then one line a skill in the order of [`Found::skills`],
/// then, when any were not listed, a line that counts them. The same
/// `found` always gives the same bytes.
#[must_use]
pub fn render(found: &Found, cap: usize) -> Index {
    let mut index = Index::default();
    if found.skills.is_empty() {
        return index;
    }
    let lines: Vec<String> = found.skills.iter().map(line).collect();
    let whole = HEADING.len() + lines.iter().map(|line| 1 + line.len()).sum::<usize>();
    let room = if whole <= cap && found.not_listed == 0 {
        cap
    } else {
        cap.saturating_sub(CUT_ROOM)
    };
    let mut text = HEADING.to_owned();
    for line in &lines {
        if text.len() + 1 + line.len() > room {
            break;
        }
        text.push('\n');
        text.push_str(line);
        index.listed += 1;
    }
    index.not_listed = found.not_listed + (lines.len() - index.listed);
    if index.listed == 0 {
        // Not even one fits: a heading over nothing says nothing.
        return index;
    }
    if index.not_listed > 0 {
        text.push_str(&format!(
            "\n({} more {} not listed, to keep this within its size.)",
            index.not_listed,
            if index.not_listed == 1 {
                "skill is"
            } else {
                "skills are"
            }
        ));
    }
    index.text = text;
    index
}

#[cfg(test)]
mod tests;
