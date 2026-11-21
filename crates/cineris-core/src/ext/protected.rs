//! The protected paths no extension's script may write, at any level
//! (`docs/architecture/12-extensions.md`, "Tier 4";
//! `docs/steps/28/parts/protected-writes.md`).
//!
//! What decides what runs later, outside any sandbox (git's configuration
//! and hooks, CI files, build and registry settings, a shell's start-up
//! files, the project's own `.cineris`), is refused to `cineris.fs.write`
//! at every level, `unrestricted` included, on every system, before the
//! permission engine is asked: as an extension's programs are confined at
//! every level where the sandbox enforces (where it does not, on Windows
//! today, they are not, and this refusal is of `fs.write` alone). The
//! engine asks about such a write short of `unrestricted` and allows it
//! there, and nothing looks at a host write afterwards as the look at the
//! protected paths does after a command: so the harness's own git, where
//! the sandbox does not enforce, and the model's `git status` at
//! `unrestricted`, would run a filter or a hook a script wrote, with the
//! user's rights.
//!
//! **What is protected** is the table of protected paths
//! (`docs/architecture/06-permissions-and-sandbox.md`, "Protected paths"),
//! which the look after a command watches (`cineris_sandbox::protect`):
//!
//! - the lease's protected paths, each with everything under it: the names
//!   of `cineris_policy::lease::PROTECTED_IN_WORKSPACE` under the workspace
//!   and `sandbox.protect`. Each is judged where it is and, when a link on
//!   the way leads elsewhere, where it leads, a link to nothing by its
//!   target's name: a `.git/hooks` that is a link to `hooks` protects
//!   `hooks`. One in the workspace is also matched name by name without
//!   regard to case, for a mount on Linux that ignores case.
//! - at any depth, the workspace's own folders included: everything in a
//!   folder named `.git`, a file named `.git` or `HEAD`, and
//!   `.cargo/config.toml`, `.cargo/config`, `.npmrc`, `.yarnrc`,
//!   `.yarnrc.yml` and `.envrc`, their names compared without regard to
//!   case (protecting too much is the safe side); and everything in a
//!   folder, the workspace's or one above it, that holds `HEAD`, `objects`
//!   and `refs`, which git takes for a repository, or that holds `HEAD`
//!   and would be given `objects` or `refs` by the write.
//!
//! **Judged by place.** The write's target is where the session's path
//! resolver led the script's path (`..`, every link followed, the spelling
//! on disk), and it is compared with each protected path by place
//! ([`PathStyle::location_within`]): on Windows the extended-length form,
//! either separator and another case are one place.

use std::path::{Component, Path, PathBuf};

use cineris_base::PathStyle;
use cineris_base::path::{PathResolver, ResolvedPath};
use cineris_policy::lease::PROTECTED_IN_WORKSPACE;
use cineris_protocol::lease::PathSet;

/// The files protected wherever they are: what npm, yarn and `direnv` read
/// from the folder they start in and upward.
const BUILD_FILES: [&str; 4] = [".npmrc", ".yarnrc", ".yarnrc.yml", ".envrc"];

/// The files of a `.cargo` folder that cargo reads, wherever it is.
const CARGO_FILES: [&str; 2] = ["config.toml", "config"];

/// What git needs beside a folder's `HEAD` to take the folder for a
/// repository.
const BESIDE_HEAD: [&str; 2] = ["objects", "refs"];

/// A protected entry that a write would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    /// In the workspace: its names from the workspace's root, joined with
    /// `/` (empty for the workspace itself).
    Inside(String),
    /// A protected path that holds the workspace.
    Around(PathBuf),
}

/// A protected path, at a place.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    at: PathBuf,
    entry: Entry,
}

/// Why `target` may not be written by an extension: the refusal's words,
/// naming the path, when it is a protected path or lies under one. `None`
/// when it may be written. Blocks: it resolves each protected path, and
/// looks for a repository on the way to the target.
pub(crate) fn refusal(
    paths: &PathResolver,
    target: &ResolvedPath,
    protect: &PathSet,
) -> Option<String> {
    let style = PathStyle::NATIVE;
    let workspace = paths.root().to_fs_path();
    let above: Vec<String> = paths.root().names().map(str::to_owned).collect();
    let at = target.to_fs_path();
    let entry = if protect.all {
        Some(Entry::Inside(target.data()))
    } else {
        let mut places = written(&workspace, &protect.roots, style);
        leading(paths, &workspace, &mut places, style);
        entry_of(&at, &workspace, &above, &places, style)
            .or_else(|| repository_on_the_way(&at, &workspace, style))
    }?;
    let home = matches!(entry, Entry::Around(_))
        .then(|| paths.resolve("~").ok())
        .flatten()
        .map(|home| home.to_fs_path());
    Some(words(target, &entry, home.as_deref()))
}

/// The protected paths as they are written: the names of
/// [`PROTECTED_IN_WORKSPACE`] under `workspace`, and the lease's `roots`
/// (absolute, as the engine placed them), which hold those names too.
pub(crate) fn written(workspace: &Path, roots: &[String], style: PathStyle) -> Vec<Place> {
    let mut places: Vec<Place> = PROTECTED_IN_WORKSPACE
        .iter()
        .map(|name| Place {
            at: joined(workspace, name, style),
            entry: Entry::Inside((*name).to_owned()),
        })
        .collect();
    for root in roots {
        let at = PathBuf::from(root);
        if places
            .iter()
            .any(|known| style.same_location(&known.at, &at))
        {
            continue;
        }
        let entry = match style.strip_location_prefix(&at, workspace) {
            Some(below) => Entry::Inside(names_of(&below, style).join("/")),
            None => Entry::Around(at.clone()),
        };
        places.push(Place { at, entry });
    }
    places
}

/// Adds where each protected path leads, when a link on the way takes it
/// elsewhere: the target is where the resolver led it, so a protected path
/// is judged there too. A link to nothing, which the resolver refuses, is
/// read by its name, as the look reads a `.cargo` link: a write may make
/// what it leads to.
fn leading(paths: &PathResolver, workspace: &Path, places: &mut Vec<Place>, style: PathStyle) {
    let mut led = Vec::new();
    for place in places.iter() {
        // A name in the workspace leads elsewhere only through a link below
        // the workspace's real root, which is looked for first: a name with
        // none costs a look at each of its names that is there, no more.
        let (asked, link) = match &place.entry {
            Entry::Inside(names) => match first_link(workspace, names) {
                Some(link) => (names.clone(), Some(link)),
                None => continue,
            },
            Entry::Around(at) => (at.to_string_lossy().into_owned(), None),
        };
        let leads = match paths.resolve(&asked) {
            Ok(resolved) => resolved.to_fs_path(),
            Err(_) => match link.and_then(|(link, rest)| lexical_target(&link, &rest)) {
                Some(leads) => leads,
                None => continue,
            },
        };
        if !style.same_location(&leads, &place.at) {
            led.push(Place {
                at: leads,
                entry: place.entry.clone(),
            });
        }
    }
    places.extend(led);
}

/// The first of `names` (separated by `/`) that is a link, from
/// `workspace` down, up to the first that is not there, and the names
/// after it.
#[allow(
    clippy::disallowed_methods,
    reason = "it looks at the file system on the blocking pool"
)]
fn first_link(workspace: &Path, names: &str) -> Option<(PathBuf, Vec<String>)> {
    let names: Vec<&str> = names.split('/').filter(|name| !name.is_empty()).collect();
    let mut at = workspace.to_path_buf();
    for (index, name) in names.iter().enumerate() {
        at.push(name);
        match std::fs::symlink_metadata(&at) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let rest = names[index + 1..].iter().map(|name| (*name).to_owned());
                return Some((at, rest.collect()));
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

/// Where the link at `link` leads by its target's name, with `rest` below
/// it, `.` and `..` taken by name: for a link to nothing.
#[allow(
    clippy::disallowed_methods,
    reason = "it reads a link on the blocking pool"
)]
fn lexical_target(link: &Path, rest: &[String]) -> Option<PathBuf> {
    let target = std::fs::read_link(link).ok()?;
    let mut joined = if target.is_absolute() {
        target
    } else {
        link.parent()?.join(target)
    };
    for name in rest {
        joined.push(name);
    }
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) if name.to_str() == Some("..") => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

/// The protected entry `target` is, or lies under, judged by its place and
/// by its names, the workspace's own (`above`, from its root) and those
/// below it. Touches nothing: the style is a parameter, so that the
/// Windows rules are tested on every system.
pub(crate) fn entry_of(
    target: &Path,
    workspace: &Path,
    above: &[String],
    places: &[Place],
    style: PathStyle,
) -> Option<Entry> {
    let below = style
        .strip_location_prefix(target, workspace)
        .map(|below| names_of(&below, style));
    let same = |name: &str, protected: &str| {
        name.eq_ignore_ascii_case(protected) || style.same_name(name, protected)
    };
    // A protected path in the workspace is also matched by its names, so
    // that a mount on Linux that ignores case (`/mnt/c` under WSL, an exFAT
    // or NTFS drive, a folder with case folding on) does not let
    // `.GITHUB/workflows` past `.github/workflows`: the resolver keeps the
    // script's spelling there.
    let folded = |entry: &Entry| match (entry, &below) {
        (Entry::Inside(names), Some(below)) if !names.is_empty() => {
            let protected: Vec<&str> = names.split('/').collect();
            below.len() >= protected.len()
                && below
                    .iter()
                    .zip(&protected)
                    .all(|(name, protected)| same(name, protected))
        }
        _ => false,
    };
    if let Some(place) = places
        .iter()
        .find(|place| style.location_within(target, &place.at) || folded(&place.entry))
    {
        return Some(place.entry.clone());
    }
    let below = below?;
    let names: Vec<&str> = above
        .iter()
        .chain(below.iter())
        .map(String::as_str)
        .collect();
    let last = names.len().checked_sub(1)?;
    for (at, name) in names.iter().enumerate() {
        let protected = same(name, ".git")
            || (at == last
                && (same(name, "HEAD")
                    || BUILD_FILES.iter().any(|file| same(name, file))
                    || (at > 0
                        && same(names[at - 1], ".cargo")
                        && CARGO_FILES.iter().any(|file| same(name, file)))));
        if !protected {
            continue;
        }
        return Some(match at.checked_sub(above.len()) {
            Some(inside) => Entry::Inside(below[..=inside].join("/")),
            // One of the workspace's own folders: the workspace is in it.
            None => Entry::Around(
                workspace
                    .ancestors()
                    .nth(above.len() - 1 - at)
                    .unwrap_or(workspace)
                    .to_path_buf(),
            ),
        });
    }
    None
}

/// The first folder git takes for a repository on the way to `target`:
/// one above the workspace or the workspace itself, or one between it and
/// the target (the target not), that holds `HEAD`, `objects` and `refs`,
/// as a bare repository made of plain files does; or one below the
/// workspace that holds `HEAD` where the write would make `objects` or
/// `refs`, so that a repository is not put together one write at a time.
#[allow(
    clippy::disallowed_methods,
    reason = "it looks at the file system on the blocking pool"
)]
fn repository_on_the_way(target: &Path, workspace: &Path, style: PathStyle) -> Option<Entry> {
    let there = |path: PathBuf| std::fs::symlink_metadata(path).is_ok();
    let repository = |folder: &Path| {
        there(folder.join("HEAD")) && BESIDE_HEAD.iter().all(|part| there(folder.join(part)))
    };
    let names = names_of(&style.strip_location_prefix(target, workspace)?, style);
    // Git looks for a repository in each folder up from where it starts.
    if let Some(folder) = workspace
        .ancestors()
        .skip(1)
        .find(|folder| repository(folder))
    {
        return Some(Entry::Around(folder.to_path_buf()));
    }
    let mut folder = workspace.to_path_buf();
    for (at, name) in names.iter().enumerate() {
        let making = BESIDE_HEAD
            .iter()
            .any(|part| name.eq_ignore_ascii_case(part))
            && there(folder.join("HEAD"));
        if making || repository(&folder) {
            return Some(Entry::Inside(names[..at].join("/")));
        }
        folder.push(name);
    }
    None
}

/// The refusal's words: the target, as code, relative to the workspace, and
/// the protected entry it is in, relative to the workspace or under `~`.
fn words(target: &ResolvedPath, entry: &Entry, home: Option<&Path>) -> String {
    let shown = cineris_base::in_code(&target.data()).into_owned();
    let what = match entry {
        Entry::Inside(names) if names.eq_ignore_ascii_case(&target.data()) => {
            format!("`{shown}` is a protected path")
        }
        Entry::Inside(names) if names.is_empty() => {
            format!("`{shown}` is in the workspace, which git takes for a repository")
        }
        Entry::Inside(names) => format!(
            "`{shown}` is in `{}`, a protected path",
            cineris_base::in_code(names)
        ),
        Entry::Around(root) => format!(
            "`{shown}` is in `{}`, a protected path",
            cineris_base::in_code(&super::found::shown_path(root, home))
        ),
    };
    format!(
        "{what}, and extensions may not write the harness's or the repository's protected files, \
         at any level (`unrestricted` included): whoever writes one can run code later, outside \
         the sandbox"
    )
}

/// `relative`, a path of names separated by `/`, under `base`, with the
/// style's own separator.
fn joined(base: &Path, relative: &str, style: PathStyle) -> PathBuf {
    let separator = match style {
        PathStyle::Posix => "/",
        PathStyle::Windows => "\\",
    };
    let base = base.to_string_lossy();
    let base = base.trim_end_matches(['/', '\\']);
    PathBuf::from(format!(
        "{base}{separator}{}",
        relative.replace('/', separator)
    ))
}

/// The names of a relative path, by either separator on Windows.
fn names_of(relative: &Path, style: PathStyle) -> Vec<String> {
    relative
        .to_string_lossy()
        .split(|character| character == '/' || (style == PathStyle::Windows && character == '\\'))
        .filter(|name| !name.is_empty() && *name != ".")
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "the tests plant folders, files and links in a scratch folder"
)]
mod tests {
    use cineris_base::path::{PathOptions, PathResolver};
    use cineris_testkit::Scratch;

    use super::*;

    /// The places of a workspace with a lease that names its own protected
    /// paths, as the engine places them, and others of the user's.
    fn placed(workspace: &str, style: PathStyle, also: &[&str]) -> Vec<Place> {
        let roots: Vec<String> = also.iter().map(|root| (*root).to_owned()).collect();
        written(Path::new(workspace), &roots, style)
    }

    /// What [`entry_of`] finds for `target` in the workspace `workspace`,
    /// whose own names are `above`: the entry's names, or the path that
    /// holds the workspace.
    fn judged(
        target: &str,
        (workspace, above): (&str, &[&str]),
        places: &[Place],
        style: PathStyle,
    ) -> Option<String> {
        let above: Vec<String> = above.iter().map(|name| (*name).to_owned()).collect();
        entry_of(
            Path::new(target),
            Path::new(workspace),
            &above,
            places,
            style,
        )
        .map(|entry| match entry {
            Entry::Inside(names) => names,
            Entry::Around(root) => root.to_string_lossy().into_owned(),
        })
    }

    /// Each protected path and what is below it is refused, a protected
    /// folder itself included; so is each name protected at any depth, in
    /// another case too, and a protected path named in another case, for a
    /// mount that ignores case; and what only begins like one, or sits
    /// beside one, is not. A protected name among the workspace's own
    /// folders counts as one below it.
    #[test]
    fn every_protected_path_and_what_is_below_it_is_refused_and_nothing_beside_it() {
        let style = PathStyle::Posix;
        let ws = ("/work/app", &["work", "app"][..]);
        let places = placed(ws.0, style, &["/work/app/deploy/keys", "/srv/keys"]);
        let refused = |target: &str| judged(&format!("{}/{target}", ws.0), ws, &places, style);
        for name in PROTECTED_IN_WORKSPACE {
            assert_eq!(refused(name).as_deref(), Some(name), "{name}");
            assert_eq!(
                refused(&format!("{name}/below/it")).as_deref(),
                Some(name),
                "{name}"
            );
        }
        for (target, entry) in [
            (".git/hooks/pre-commit", ".git/hooks"),
            (".git/objects/ab/cd", ".git"),
            (".GIT/CONFIG", ".git/config"),
            (".GITHUB/workflows/ci.yml", ".github/workflows"),
            (".Husky/pre-commit", ".husky"),
            (".Pre-Commit-Config.yaml", ".pre-commit-config.yaml"),
            (".CINERIS/config.toml", ".cineris"),
            ("vendor/lib/.git/config", "vendor/lib/.git"),
            ("vendor/lib/.GIT", "vendor/lib/.GIT"),
            ("fake/HEAD", "fake/HEAD"),
            ("crates/x/.cargo/config.toml", "crates/x/.cargo/config.toml"),
            ("crates/x/.CARGO/Config", "crates/x/.CARGO/Config"),
            ("web/.npmrc", "web/.npmrc"),
            ("web/app/.envrc", "web/app/.envrc"),
            ("deploy/keys/id", "deploy/keys"),
            ("Deploy/Keys/id", "deploy/keys"),
        ] {
            assert_eq!(refused(target).as_deref(), Some(entry), "{target}");
        }
        for target in [
            "src/main.rs",
            ".gitignore",
            ".gitattributes",
            ".github/dependabot.yml",
            ".cinerisrc",
            ".cargo/registry.txt",
            "docs/config.toml",
            "notes/HEAD.md",
            "src/git/config",
            "deploy/keys.txt",
        ] {
            assert_eq!(refused(target), None, "{target}");
        }
        // The workspace inside a protected path of the user's is protected
        // whole.
        let around = placed(ws.0, style, &["/work"]);
        assert_eq!(
            judged("/work/app/src/a.rs", ws, &around, style).as_deref(),
            Some("/work")
        );
        // A workspace inside a `.git` folder, and one that is a `.cargo`.
        let in_git = ("/work/.git/app", &["work", ".git", "app"][..]);
        let places = placed(in_git.0, style, &[]);
        assert_eq!(
            judged("/work/.git/app/src/a.rs", in_git, &places, style).as_deref(),
            Some("/work/.git")
        );
        let cargo = ("/srv/u/.cargo", &["srv", "u", ".cargo"][..]);
        let places = placed(cargo.0, style, &[]);
        assert_eq!(
            judged("/srv/u/.cargo/config.toml", cargo, &places, style).as_deref(),
            Some("config.toml")
        );
        assert_eq!(
            judged("/srv/u/.cargo/registry/x", cargo, &places, style),
            None
        );
    }

    /// On Windows the target is in the extended-length form the resolver
    /// gives, the lease's paths are written with `/`, and a script may
    /// write another case, one folded by NTFS's rules outside ASCII too:
    /// all are one place. A protected path that holds the workspace is
    /// found by place alone. A share is a place of its own.
    #[test]
    fn on_windows_a_protected_path_is_found_whatever_its_form_and_case() {
        let style = PathStyle::Windows;
        let ws = (r"\\?\C:\Work\App", &["Work", "App"][..]);
        let places = placed(
            ws.0,
            style,
            &["C:/Work/App/.git/config", "C:/Work/App/deploy"],
        );
        for (target, entry) in [
            (r"\\?\C:\Work\App\.git\config", ".git/config"),
            (r"\\?\c:\work\app\.GIT\CONFIG", ".git/config"),
            (r"C:/Work/App/.git/hooks/pre-commit", ".git/hooks"),
            (r"C:\WORK\APP\.Git\Hooks\post-checkout", ".git/hooks"),
            (r"\\?\C:\Work\App\.cargo\Config.TOML", ".cargo/config.toml"),
            (r"\\?\C:\Work\App\.Cineris\config.toml", ".cineris"),
            (
                r"\\?\C:\Work\App\.GITHUB\Workflows\ci.yml",
                ".github/workflows",
            ),
            ("\\\\?\\C:\\Work\\App\\.hu\u{17f}ky\\pre-commit", ".husky"),
            (r"\\?\C:\Work\App\Deploy\x", "deploy"),
            (r"\\?\C:\Work\App\sub\.GIT\config", "sub/.GIT"),
            (r"\\?\C:\Work\App\sub\.cargo\config", r"sub/.cargo/config"),
        ] {
            assert_eq!(
                judged(target, ws, &places, style).as_deref(),
                Some(entry),
                "{target}"
            );
        }
        for target in [
            r"\\?\C:\Work\App\src\main.rs",
            r"\\?\C:\Work\App2\.gitignore",
            r"\\?\D:\Work\App\.github\workflows\ci.yml",
        ] {
            assert_eq!(judged(target, ws, &places, style), None, "{target}");
        }
        let around = placed(ws.0, style, &["c:/work"]);
        assert_eq!(
            judged(r"\\?\C:\Work\App\src\a.rs", ws, &around, style).as_deref(),
            Some("c:/work")
        );
        let share = (r"\\server\share\app", &["app"][..]);
        let on_share = placed(share.0, style, &[]);
        assert_eq!(
            judged(
                r"\\?\UNC\Server\Share\App\.git\hooks\x",
                share,
                &on_share,
                style
            )
            .as_deref(),
            Some(".git/hooks")
        );
    }

    /// A new workspace in a scratch folder; and, below, a lease that
    /// protects nothing more than the names every workspace has.
    fn workspace(test: &str) -> (Scratch, PathBuf) {
        let scratch = Scratch::outside("cineris-core-unit", test);
        let ws = scratch.dir("ws");
        (scratch, ws)
    }

    const NONE_MORE: PathSet = PathSet {
        all: false,
        roots: Vec::new(),
    };

    /// Makes `link`, a link to the folder `target`, which need not be
    /// there. `false` on a Windows machine without the right to; a CI
    /// runner has it, and must.
    fn link(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(target, link);
        match made {
            Ok(()) => true,
            Err(error) => {
                assert!(
                    std::env::var_os("GITHUB_ACTIONS").is_none(),
                    "the runner cannot make a link: {error}"
                );
                false
            }
        }
    }

    /// A folder that holds `HEAD`, `objects` and `refs` is a repository to
    /// git, whatever its name: everything in it is refused; a folder that
    /// holds only some of them is not one, but one that holds `HEAD` is not
    /// given `objects` or `refs`. The refusal names the path as code a name
    /// cannot end, and everything is refused where the lease protects
    /// everything.
    #[test]
    fn everything_in_a_folder_git_takes_for_a_repository_is_refused() {
        let (_scratch, ws) = workspace("ext_protected_repository");
        for part in ["mirror/objects", "mirror/refs", "half/objects", "half/refs"] {
            std::fs::create_dir_all(ws.join(part)).unwrap();
        }
        std::fs::create_dir_all(ws.join("third/objects")).unwrap();
        std::fs::write(ws.join("mirror/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(ws.join("third/HEAD"), "ref: refs/heads/main\n").unwrap();
        let paths = PathResolver::new(&ws, PathOptions::new()).unwrap();
        let refused = |target: &str, protect: &PathSet| {
            refusal(&paths, &paths.resolve(target).unwrap(), protect)
        };
        let words = refused("mirror/hooks/pre-commit", &NONE_MORE).expect("refused");
        assert!(
            words.starts_with("`mirror/hooks/pre-commit` is in `mirror`, a protected path, and "),
            "{words}"
        );
        assert!(
            words.contains("at any level (`unrestricted` included)"),
            "{words}"
        );
        assert!(refused("mirror/config", &NONE_MORE).is_some());
        assert_eq!(refused("half/config", &NONE_MORE), None);
        assert_eq!(refused("third/config", &NONE_MORE), None);
        let making = refused("third/refs/heads/main", &NONE_MORE).expect("refused");
        assert!(
            making.starts_with("`third/refs/heads/main` is in `third`"),
            "{making}"
        );
        assert_eq!(refused("src/main.rs", &NONE_MORE), None);
        // A name is shown as code it cannot end.
        let odd = refused("a`b/.npmrc", &NONE_MORE).expect("refused");
        assert!(
            odd.starts_with(r"`a\`b/.npmrc` is a protected path"),
            "{odd}"
        );
        let all = PathSet {
            all: true,
            roots: Vec::new(),
        };
        assert!(refused("src/main.rs", &all).is_some());
    }

    /// A workspace that git takes for a repository, or that is in one,
    /// refuses every write.
    #[test]
    fn a_workspace_that_is_or_is_in_a_repository_refuses_every_write() {
        let scratch = Scratch::outside("cineris-core-unit", "ext_protected_repository_around");
        for root in ["bare", "outer"] {
            for part in ["objects", "refs"] {
                std::fs::create_dir_all(scratch.path().join(root).join(part)).unwrap();
            }
            std::fs::write(scratch.path().join(root).join("HEAD"), "ref: x\n").unwrap();
        }
        let bare = PathResolver::new(&scratch.path().join("bare"), PathOptions::new()).unwrap();
        let words = refusal(&bare, &bare.resolve("src/a.rs").unwrap(), &NONE_MORE).unwrap();
        assert!(
            words.starts_with("`src/a.rs` is in the workspace, which git takes for a repository"),
            "{words}"
        );
        let inner = scratch.dir("outer/ws");
        let paths = PathResolver::new(&inner, PathOptions::new()).unwrap();
        let words = refusal(&paths, &paths.resolve("src/a.rs").unwrap(), &NONE_MORE).unwrap();
        let outer = paths.root().to_fs_path();
        let outer = outer.parent().unwrap();
        assert!(
            words.starts_with(&format!(
                "`src/a.rs` is in `{}`, a protected path",
                cineris_base::in_code(&outer.display().to_string())
            )),
            "{words}"
        );
    }

    /// A protected path that is a link is judged where it leads too: a
    /// `.git/hooks` that leads to `hooks` protects `hooks`; a `.git/info`
    /// that leads to a folder not there yet protects that folder; and a
    /// `sandbox.protect` path that holds the workspace through a link holds
    /// it where it really is.
    #[test]
    fn a_protected_path_that_is_a_link_protects_where_it_leads() {
        let (scratch, ws) = workspace("ext_protected_link");
        std::fs::create_dir_all(ws.join(".git")).unwrap();
        std::fs::create_dir_all(ws.join("hooks")).unwrap();
        let alias = scratch.path().join("alias");
        let made = link(&ws.join("hooks"), &ws.join(".git/hooks"))
            && link(&Path::new("..").join("gitinfo"), &ws.join(".git/info"))
            && link(scratch.path(), &alias);
        if !made {
            return;
        }
        let paths = PathResolver::new(&ws, PathOptions::new()).unwrap();
        let refused = |target: &str, protect: &PathSet| {
            refusal(&paths, &paths.resolve(target).unwrap(), protect)
        };
        let words = refused("hooks/pre-commit", &NONE_MORE).expect("refused");
        assert!(
            words.starts_with("`hooks/pre-commit` is in `.git/hooks`, a protected path"),
            "{words}"
        );
        let words = refused("gitinfo/attributes", &NONE_MORE).expect("refused");
        assert!(
            words.starts_with("`gitinfo/attributes` is in `.git/info`, a protected path"),
            "{words}"
        );
        assert!(
            refused(".git/config", &NONE_MORE)
                .unwrap()
                .starts_with("`.git/config` is a protected path")
        );
        assert_eq!(refused("src/main.rs", &NONE_MORE), None);
        let around = PathSet {
            all: false,
            roots: vec![alias.to_string_lossy().into_owned()],
        };
        let words = refused("src/main.rs", &around).expect("refused");
        assert!(
            words.starts_with(&format!(
                "`src/main.rs` is in `{}`, a protected path",
                cineris_base::in_code(&alias.display().to_string())
            )),
            "{words}"
        );
    }
}
