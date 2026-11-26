//! Resolving a path against the real file system.

#![allow(
    clippy::disallowed_methods,
    reason = "resolving a path asks the file system; async callers run this on the blocking pool"
)]

use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};

use super::lexical::{self, Part};
use super::{AbsPath, PathError, PathStyle, RelPath, root_path, steps};

/// How many links one path may lead through before it is taken for a loop.
/// The same number Linux uses.
const MAX_LINKS: usize = 40;

/// What a [`PathResolver`] needs to know beyond the workspace root.
#[derive(Debug, Clone, Default)]
pub struct PathOptions {
    home: Option<AbsPath>,
    shares: Vec<(String, String)>,
}

impl PathOptions {
    /// No home directory, and no network share allowed.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the directory that a leading `~` stands for.
    #[must_use]
    pub fn home(mut self, home: AbsPath) -> Self {
        self.home = Some(home);
        self
    }

    /// Takes the home directory from the environment: `HOME`, or
    /// `USERPROFILE` on Windows. If it is not set, or is not an absolute
    /// path, `~` stays unknown and a path that uses it is refused.
    #[must_use]
    pub fn home_from_env(mut self) -> Self {
        let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        self.home =
            std::env::var_os(variable).and_then(|home| AbsPath::from_os(Path::new(&home)).ok());
        self
    }

    /// Allows paths on one network share, `\\server\share`. Every other
    /// share stays refused. The names are compared without regard to case.
    #[must_use]
    pub fn allow_share(mut self, server: impl Into<String>, share: impl Into<String>) -> Self {
        self.shares.push((server.into(), share.into()));
        self
    }

    /// Refuses a path on a network share that is not on the allow-list.
    /// `written` is the path as the caller wrote it, for the error.
    fn check_share(&self, path: &AbsPath, written: &str) -> Result<(), PathError> {
        let Some((server, share)) = path.share() else {
            return Ok(());
        };
        let style = path.style();
        let allowed = self.shares.iter().any(|(allowed_server, allowed_share)| {
            style.same_name(allowed_server, server) && style.same_name(allowed_share, share)
        });
        if allowed {
            Ok(())
        } else {
            Err(PathError::ShareNotAllowed {
                path: written.to_owned(),
                server: server.to_owned(),
                share: share.to_owned(),
            })
        }
    }
}

/// Resolves paths as written into real locations, for one workspace.
///
/// It always works in the style of the OS it runs on, because it asks that
/// OS's file system.
#[derive(Debug, Clone)]
pub struct PathResolver {
    root: AbsPath,
    options: PathOptions,
}

/// A path after resolution: the real location, and where it is in relation
/// to the workspace.
///
/// "Real" means every symbolic link and Windows junction on the way has been
/// followed, and on Windows every name is spelled as the file system has it
/// and short names (`PROGRA~1`) have been replaced by the long ones. Two
/// spellings of a path to the same existing file therefore resolve to equal
/// `ResolvedPath`s, so a rule cannot be dodged through another spelling.
///
/// Two limits, both for whoever writes rules:
///
/// - **A file with two names is two paths.** Hard links are not detected.
/// - **Linux is taken to compare names exactly.** On a mount that ignores
///   case there (a FAT or NTFS volume, a directory with case folding turned
///   on), `SECRET.ENV` and `secret.env` are one file and resolve to two
///   paths.
///
/// It describes the file system at the moment of resolution. See the
/// [module](super) documentation for what that does and does not promise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPath {
    abs: AbsPath,
    relative: Option<RelPath>,
    exists: bool,
    names_a_directory: bool,
}

impl ResolvedPath {
    /// The real, absolute location. Permission rules are matched against
    /// this.
    #[must_use]
    pub fn abs(&self) -> &AbsPath {
        &self.abs
    }

    /// The location relative to the workspace root, when it is inside the
    /// workspace. `None` means outside: allowed only by permission.
    #[must_use]
    pub fn relative(&self) -> Option<&RelPath> {
        self.relative.as_ref()
    }

    /// Whether the real location is the workspace root or below it.
    ///
    /// The workspace root and the part of the path that exists are both in
    /// the file system's own spelling, so they are compared exactly, on
    /// Windows too. Nothing here depends on how this crate folds case.
    #[must_use]
    pub fn is_inside_workspace(&self) -> bool {
        self.relative.is_some()
    }

    /// Whether something existed there when the path was resolved. A path
    /// to a file that is about to be created resolves with `false`: its
    /// existing directories are real, and the rest is as written.
    #[must_use]
    pub fn exists(&self) -> bool {
        self.exists
    }

    /// Whether the path was written with a separator at its end, as in
    /// `out/`. Such a path names a directory. One that leads to an existing
    /// file is refused when it is resolved. For one that does not exist yet,
    /// this is how a caller knows not to create a file there.
    #[must_use]
    pub fn names_a_directory(&self) -> bool {
        self.names_a_directory
    }

    /// The path as it is shown to a model or a client, in text: relative to
    /// the workspace when inside it, absolute otherwise, always with `/`,
    /// and every character that would act unseen (a line break, a
    /// bidirectional control, a tag character) written as an escape
    /// ([`crate::visible`]), so that a name cannot start lines of its own
    /// in a result, a notice or a prompt.
    #[must_use]
    pub fn shown(&self) -> String {
        crate::visible(&self.data()).into_owned()
    }

    /// The path as data, for a program to read back: in a tool's payload,
    /// as a key. The same as [`ResolvedPath::shown`] but with nothing
    /// escaped. Never put it in text a model or a client reads.
    #[must_use]
    pub fn data(&self) -> String {
        match &self.relative {
            Some(relative) => relative.to_string(),
            None => self.abs.to_string(),
        }
    }

    /// The path for the standard library's file functions. See
    /// [`AbsPath::to_fs_path`].
    #[must_use]
    pub fn to_fs_path(&self) -> PathBuf {
        self.abs.to_fs_path()
    }
}

impl PathResolver {
    /// A resolver for the workspace at `root`, which must be an absolute
    /// path to an existing directory. The root itself is resolved, so a
    /// workspace opened through a symbolic link is known by its real
    /// location.
    pub fn new(root: &Path, options: PathOptions) -> Result<Self, PathError> {
        let Some(text) = root.to_str() else {
            return Err(PathError::NotUnicode {
                path: root.to_string_lossy().into_owned(),
            });
        };
        let style = PathStyle::NATIVE;
        let text = match style {
            PathStyle::Posix => std::borrow::Cow::Borrowed(text),
            PathStyle::Windows => lexical::from_extended_length(text)?,
        };
        let (start, parts) = match steps(&text, style)? {
            (Some(start), parts) => (root_path(style, &start), parts),
            (None, _) => {
                return Err(PathError::NotAbsolute {
                    path: text.to_string(),
                });
            }
        };
        let (real, missing) = locate(&options, &start, &parts, &text)?;
        if !missing.is_empty() {
            return Err(PathError::Io {
                path: text.to_string(),
                source: io::ErrorKind::NotFound.into(),
            });
        }
        if !is_dir(&real) {
            return Err(PathError::NotADirectory {
                path: real.to_string(),
            });
        }
        Ok(Self {
            root: real,
            options,
        })
    }

    /// The workspace root's real location.
    #[must_use]
    pub fn root(&self) -> &AbsPath {
        &self.root
    }

    /// The directory a leading `~` stands for, as it was given, not
    /// resolved: for a look at a name below it that follows no link the
    /// resolver would follow. `None` when there is none.
    #[must_use]
    pub fn home(&self) -> Option<&AbsPath> {
        self.options.home.as_ref()
    }

    /// Resolves a path as a tool received it: relative to the workspace
    /// root, absolute, or starting with `~` for the home directory (`~` or
    /// `~/notes`; `~name` and `./~` are ordinary names).
    ///
    /// - The path is read by the rules of this OS, and refused if it breaks
    ///   them ([`PathError`]).
    /// - Links are followed one at a time, and each link's target is read
    ///   and checked by the same rules **before** it is followed. A link to
    ///   a network share that is not allowed is refused without the share
    ///   being contacted, like a path that names the share outright.
    /// - `..` is resolved as the OS resolves it: against the real directory
    ///   on Linux, where `link/..` is the parent of the link's target, and
    ///   as text on Windows.
    /// - The part of the path that exists is replaced by its real location.
    ///   The part that does not exist yet is kept as written, name-checked.
    /// - A path that leads outside the workspace is not an error. It
    ///   resolves, and [`ResolvedPath::is_inside_workspace`] says so.
    /// - **A name the model was shown escaped** ([`ResolvedPath::shown`],
    ///   [`crate::visible`]) leads back to its file: when the path as written
    ///   leads to nothing, or is refused, the same path with the escapes
    ///   read back ([`crate::unescape`]) is tried, and taken when all of it
    ///   exists, when it reaches further into directories that exist than
    ///   the path as written does, or when the path as written was refused.
    ///   A file named with a zero-width joiner or a right-to-left mark,
    ///   which is shown as `\u{200d}` or `\u{200f}`, is reached by that
    ///   spelling, and so is a new file in a directory so named (found by
    ///   the independent review of step 18's fixes). How far each reaches
    ///   is counted in the names of its real location, not in the names
    ///   missing: on Windows `\` parts the path as written, so it has more
    ///   names than the path read back.
    pub fn resolve(&self, input: &str) -> Result<ResolvedPath, PathError> {
        let literal = self.resolve_written(input);
        let literal_reach = match &literal {
            Ok((found, _)) if found.exists() => return literal.map(|(found, _)| found),
            Ok((_, reach)) => Some(*reach),
            Err(_) => None,
        };
        if let std::borrow::Cow::Owned(unescaped) = crate::visible::unescape(input)
            && let Ok((found, reach)) = self.resolve_written(&unescaped)
            && (found.exists() || literal_reach.is_none_or(|literal| reach > literal))
        {
            return Ok(found);
        }
        literal.map(|(found, _)| found)
    }

    /// [`PathResolver::resolve`] of the path exactly as written, with how
    /// far it reaches into what exists: the number of names of the real
    /// location of its part that exists.
    fn resolve_written(&self, input: &str) -> Result<(ResolvedPath, usize), PathError> {
        let style = PathStyle::NATIVE;
        let (root, mut parts) = steps(input, style)?;
        let start = match root {
            Some(root) => root_path(style, &root),
            // `~` alone, or `~` and a separator, at the very start. `./~`
            // is a file named `~`.
            None if parts.first() == Some(&Part::Name("~")) && input.starts_with('~') => {
                parts.remove(0);
                self.options.home.clone().ok_or_else(|| PathError::NoHome {
                    path: input.to_owned(),
                })?
            }
            None => self.root.clone(),
        };

        let (real, missing) = locate(&self.options, &start, &parts, input)?;
        let exists = missing.is_empty();
        let names_a_directory =
            input.ends_with('/') || (style == PathStyle::Windows && input.ends_with('\\'));
        if names_a_directory && exists && !is_dir(&real) {
            return Err(PathError::Io {
                path: input.to_owned(),
                source: io::ErrorKind::NotADirectory.into(),
            });
        }
        // Inside the workspace or not is decided on the part that exists,
        // which is in the file system's own spelling, as the root is.
        let relative = real.strip_prefix_exactly(&self.root).map(|existing| {
            let names: Vec<&str> = existing
                .names()
                .chain(missing.iter().map(String::as_str))
                .collect();
            RelPath::from_text(style, names.join("/"))
        });
        let reach = real.names().count();
        let mut abs = real;
        for name in &missing {
            // Each name was checked when the path was read.
            abs = abs.join_rel(&RelPath::from_text(style, name.clone()));
        }
        Ok((
            ResolvedPath {
                abs,
                relative,
                exists,
                names_a_directory,
            },
            reach,
        ))
    }
}

/// One step still to take.
#[derive(Debug)]
enum Step {
    Name(String),
    Parent,
}

impl From<&Part<'_>> for Step {
    fn from(part: &Part<'_>) -> Self {
        match part {
            Part::Name(name) => Self::Name((*name).to_owned()),
            Part::Parent => Self::Parent,
        }
    }
}

/// Follows `parts` from `start` through the file system. Returns the real
/// location of the part that exists, and the names after it that do not.
/// `written` is the path as the caller wrote it, for errors.
///
/// No link is ever followed by the operating system on our behalf. Each
/// step is looked at without following it; a link's target is read, checked
/// like any path, and put in the link's place. So a link cannot lead
/// anywhere, a network share included, that the rules have not seen first.
fn locate(
    options: &PathOptions,
    start: &AbsPath,
    parts: &[Part<'_>],
    written: &str,
) -> Result<(AbsPath, Vec<String>), PathError> {
    let style = PathStyle::NATIVE;
    // Before anything is asked of the file system, and so of the network.
    options.check_share(start, written)?;

    let mut pending: VecDeque<Step> = if style == PathStyle::Windows {
        // Windows resolves `..` as text, against the whole path and
        // stopping at the drive's root, before it opens anything.
        let whole: Vec<Part<'_>> = start
            .names()
            .map(Part::Name)
            .chain(parts.iter().copied())
            .collect();
        let (_, names) = lexical::collapse(&whole);
        names
            .into_iter()
            .map(|name| Step::Name(name.to_owned()))
            .collect()
    } else {
        start
            .names()
            .map(|name| Step::Name(name.to_owned()))
            .chain(parts.iter().map(Step::from))
            .collect()
    };

    // Every name in `current` exists and is not a link.
    let mut current = start.root();
    // The names after the point where the path stops existing.
    let mut missing: Vec<String> = Vec::new();
    // How many steps at the front of `pending` came out of a link's target,
    // and which link that was.
    let mut steps_from_link = 0usize;
    let mut last_link: Option<AbsPath> = None;
    let mut links_followed = 0usize;

    while let Some(step) = pending.pop_front() {
        let from_link = steps_from_link > 0;
        steps_from_link = steps_from_link.saturating_sub(1);
        let name = match step {
            Step::Parent => {
                // Out of the part that does not exist first, then up the
                // real directories. The root is its own parent.
                if missing.pop().is_none()
                    && let Some(parent) = current.parent()
                {
                    current = parent;
                }
                continue;
            }
            Step::Name(name) => name,
        };
        if !missing.is_empty() {
            missing.push(name);
            continue;
        }

        let candidate = current.join_rel(&RelPath::from_text(style, name.clone()));
        let is_link = match is_link(&candidate.to_fs_path()) {
            Ok(is_link) => is_link,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // A link whose target is not there: writing "through" it
                // would create a file at a place the path does not show.
                if from_link && let Some(link) = &last_link {
                    return Err(PathError::DanglingLink {
                        path: link.to_string(),
                    });
                }
                // New names can only be created inside a directory.
                if !is_dir(&current) {
                    return Err(PathError::Io {
                        path: candidate.to_string(),
                        source: io::ErrorKind::NotADirectory.into(),
                    });
                }
                missing.push(name);
                continue;
            }
            Err(source) => {
                return Err(PathError::Io {
                    path: candidate.to_string(),
                    source,
                });
            }
        };
        if !is_link {
            current = candidate;
            continue;
        }

        // A symbolic link, or on Windows a junction. Read where it points,
        // check that as a path in its own right, and go on from there.
        links_followed += 1;
        if links_followed > MAX_LINKS {
            return Err(PathError::Io {
                path: candidate.to_string(),
                source: io::Error::other("too many levels of symbolic links"),
            });
        }
        let target =
            std::fs::read_link(candidate.to_fs_path()).map_err(|source| PathError::Io {
                path: candidate.to_string(),
                source,
            })?;
        let Some(target) = target.to_str() else {
            return Err(PathError::NotUnicode {
                path: target.to_string_lossy().into_owned(),
            });
        };
        let target = match style {
            PathStyle::Posix => std::borrow::Cow::Borrowed(target),
            // Windows hands back an absolute target in the extended-length
            // form. Anything but a drive or a share is refused here.
            PathStyle::Windows => lexical::from_extended_length(target)?,
        };
        let (target_root, target_parts) = steps(&target, style)?;
        if let Some(target_root) = target_root {
            let target_root = root_path(style, &target_root);
            options.check_share(&target_root, written)?;
            current = target_root;
        }
        // A relative target starts from the directory the link is in,
        // which is `current`.
        steps_from_link += target_parts.len();
        for part in target_parts.iter().rev() {
            pending.push_front(Step::from(part));
        }
        last_link = Some(candidate);
    }

    // `current` exists and has no link in it. On Windows it may still be
    // written in another case or with short names: ask for its own spelling.
    let real = match std::fs::canonicalize(current.to_fs_path()) {
        Ok(real) => AbsPath::from_os(&real)?,
        Err(source) => {
            return Err(PathError::Io {
                path: current.to_string(),
                source,
            });
        }
    };
    // Again on the real location: a drive letter can be mapped to a share.
    options.check_share(&real, written)?;
    Ok((real, missing))
}

/// Whether the name at `path` is a link (a symbolic link, or on Windows a
/// junction or any other reparse point the standard library takes for
/// one), looked at without following it: what
/// `std::fs::symlink_metadata(path)?.file_type().is_symlink()` says, and
/// its error when it fails.
///
/// **On Windows the attributes are read first,** without opening the file
/// (`GetFileAttributesW`): a name that has no reparse point is no link,
/// which is the standard library's answer for it, at a third of its cost
/// on a hosted runner (`docs/steps/ci-budgets-2/answered.md`, "Where a
/// Windows session's time goes"). A name that is not there is not there to
/// both, and the error is of the kind `symlink_metadata`'s is, which is
/// all the walk asks of it. A reparse point, and a name whose attributes
/// cannot be read for any other reason, are looked at by `symlink_metadata`
/// as before, so that the answer, or the error, is the one it gives.
///
/// Public for the other walks of a path that ask each name the same
/// question (the program search's, the checkpoints', the protected paths'
/// look and the snapshots'), so that each asks it at the same cost.
pub fn is_link(path: &Path) -> io::Result<bool> {
    #[cfg(windows)]
    match crate::private::attributes(path) {
        Ok(attributes) if !attributes.is_reparse_point() => return Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    std::fs::symlink_metadata(path).map(|found| found.file_type().is_symlink())
}

/// Whether `path`, which has no link in it, is a directory.
///
/// On Windows from its attributes, as [`is_link`] looks, where they show no
/// reparse point; otherwise, and on Linux, the standard library's
/// `metadata`, which follows a link at the end.
fn is_dir(path: &AbsPath) -> bool {
    let path = path.to_fs_path();
    #[cfg(windows)]
    if let Ok(attributes) = crate::private::attributes(&path)
        && !attributes.is_reparse_point()
    {
        return attributes.is_directory();
    }
    std::fs::metadata(&path).is_ok_and(|metadata| metadata.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn windows(path: &str) -> AbsPath {
        AbsPath::parse(path, PathStyle::Windows).unwrap()
    }

    /// The walk's look at a name ([`is_link`], [`is_dir`]) gives what the
    /// standard library's `symlink_metadata` and `metadata` gave, for every
    /// kind of name a Windows runner can make (`docs/steps/ci-budgets-2/
    /// answered.md`): the same answer, or an error of the same kind where
    /// they fail. A link this machine may not make is skipped away from CI,
    /// and fails the test on CI, whose runners may.
    #[cfg(windows)]
    #[test]
    fn on_windows_the_walks_look_at_a_name_is_the_standard_librarys() {
        use std::os::windows::fs::{symlink_dir, symlink_file};

        fn agrees(path: &Path) {
            let before =
                std::fs::symlink_metadata(path).map(|found| found.file_type().is_symlink());
            match (before, is_link(path)) {
                (Ok(before), Ok(now)) => assert_eq!(before, now, "is a link: {}", path.display()),
                (Err(before), Err(now)) => {
                    assert_eq!(
                        before.kind(),
                        now.kind(),
                        "{}: {before} / {now}",
                        path.display()
                    );
                }
                (before, now) => panic!("{}: {before:?} / {now:?}", path.display()),
            }
            let directory = std::fs::metadata(path).is_ok_and(|found| found.is_dir());
            if let Ok(path) = AbsPath::from_os(path) {
                assert_eq!(directory, is_dir(&path), "is a directory: {path}");
            }
        }
        fn made(made: std::io::Result<()>) -> bool {
            match made {
                Ok(()) => true,
                Err(error) => {
                    assert!(std::env::var_os("CI").is_none(), "on CI: {error}");
                    false
                }
            }
        }

        let dir = std::env::temp_dir().join(format!("cineris-is-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("folder")).unwrap();
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        let mut names = vec![
            dir.clone(),
            dir.join("folder"),
            dir.join("file.txt"),
            // Not there, and a parent that is not there.
            dir.join("missing.txt"),
            dir.join("missing").join("file.txt"),
            // Another case of the same names.
            dir.join("FOLDER"),
            dir.join("File.TXT"),
        ];
        // Links to a folder, to a file, and to nothing.
        if made(symlink_dir(dir.join("folder"), dir.join("to-folder"))) {
            names.push(dir.join("to-folder"));
            names.push(dir.join("to-folder").join("inside"));
        }
        if made(symlink_file(dir.join("file.txt"), dir.join("to-file.txt"))) {
            names.push(dir.join("to-file.txt"));
        }
        if made(symlink_file(dir.join("gone.txt"), dir.join("dangling.txt"))) {
            names.push(dir.join("dangling.txt"));
        }
        // Trailing dots and spaces: a name Windows would read as another
        // unless the path is in its extended-length form.
        let verbatim = |name: &str| PathBuf::from(format!(r"\\?\{}\{name}", dir.display()));
        for name in ["dot.", "space ", "both. "] {
            std::fs::write(verbatim(name), "x").unwrap();
            names.push(verbatim(name));
            names.push(dir.join(name));
        }
        // The extended-length form of the others, and a path past 260
        // characters.
        names.push(verbatim("folder"));
        names.push(verbatim("missing.txt"));
        let long = verbatim(&"d".repeat(120))
            .join("e".repeat(120))
            .join("f".repeat(40));
        std::fs::create_dir_all(&long).unwrap();
        names.push(long.clone());
        names.push(long.join("missing.txt"));
        // Alternate data streams of a file.
        std::fs::write(dir.join("file.txt:stream"), "x").unwrap();
        names.push(dir.join("file.txt:stream"));
        names.push(dir.join("file.txt::$DATA"));
        names.push(dir.join("file.txt:missing"));
        // The page file, which no one may open.
        for page in [r"C:\pagefile.sys", r"D:\pagefile.sys"] {
            names.push(PathBuf::from(page));
        }
        // A short name of the system's program folder, where the volume
        // makes them.
        names.push(PathBuf::from(r"C:\PROGRA~1"));
        // App execution aliases: reparse points that are not links.
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let aliases = PathBuf::from(local).join("Microsoft").join("WindowsApps");
            names.extend(
                std::fs::read_dir(&aliases)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .take(5)
                    .map(|entry| entry.path()),
            );
        }
        for name in &names {
            agrees(name);
        }
        // A name below a file: an error to both, as the walk takes it
        // (a name it finds missing below a file is refused as one under no
        // directory).
        let below = dir.join("file.txt").join("below");
        assert!(std::fs::symlink_metadata(&below).is_err());
        assert!(is_link(&below).is_err());
        // Device names, which the resolver refuses as text before it looks
        // at anything: never taken for a link.
        for device in [r"\\.\NUL", "NUL", "CON"] {
            assert!(!is_link(Path::new(device)).unwrap_or(false), "{device}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_an_allowed_share_passes_the_share_check() {
        let none = PathOptions::new();
        let one = PathOptions::new().allow_share("FileServer", "Projects");

        // A local path is never a share.
        for options in [&none, &one] {
            assert!(options.check_share(&windows(r"C:\Work\app"), "x").is_ok());
            let posix = AbsPath::parse("/srv/app", PathStyle::Posix).unwrap();
            assert!(options.check_share(&posix, "x").is_ok());
        }

        let on_share = windows(r"\\fileserver\PROJECTS\app\src");
        match none.check_share(&on_share, r"S:\app\src").unwrap_err() {
            PathError::ShareNotAllowed {
                path,
                server,
                share,
            } => {
                assert_eq!(path, r"S:\app\src", "the path as it was written");
                assert_eq!(
                    (server.as_str(), share.as_str()),
                    ("fileserver", "PROJECTS")
                );
            }
            other => panic!("unexpected refusal: {other}"),
        }
        assert!(
            one.check_share(&on_share, "x").is_ok(),
            "compared without case"
        );
        assert!(
            one.check_share(&windows(r"\\fileserver\projects"), "x")
                .is_ok()
        );

        // Another share on the same server, and the same share name on
        // another server, stay refused.
        for other in [
            r"\\fileserver\Private\x",
            r"\\elsewhere\Projects\x",
            r"\\fileserver\Projects2\x",
            r"\\fileserver.example\Projects\x",
        ] {
            assert!(
                matches!(
                    one.check_share(&windows(other), other),
                    Err(PathError::ShareNotAllowed { .. })
                ),
                "{other}"
            );
        }
    }
}
