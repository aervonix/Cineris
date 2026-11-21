//! The protected paths no extension's script may write, at any level
//! (`docs/architecture/12-extensions.md`, "Tier 4";
//! `docs/steps/28/parts/protected-writes.md`).
//!
//! What decides what runs later, outside any sandbox (git's configuration
//! and hooks, CI files, build and registry settings, a shell's start-up
//! files, the project's own `.cineris`), is refused to `cineris.fs.write`
//! at every level, `unrestricted` included, before the permission engine
//! is asked, as an extension's programs are confined at every level. The
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
//!   the way leads elsewhere, where it leads: a `.git/hooks` that is a link
//!   to `hooks` protects `hooks`.
//! - at any depth: everything in a folder named `.git`, a file named
//!   `.git` or `HEAD`, and `.cargo/config.toml`, `.cargo/config`, `.npmrc`,
//!   `.yarnrc`, `.yarnrc.yml` and `.envrc`, their names compared without
//!   regard to case (protecting too much is the safe side); and everything
//!   in a folder that holds `HEAD`, `objects` and `refs`, which git takes
//!   for a repository.
//!
//! **Judged by place.** The write's target is where the session's path
//! resolver led the script's path (`..`, every link followed, the spelling
//! on disk), and it is compared with each protected path by place
//! ([`PathStyle::location_within`]): on Windows the extended-length form,
//! either separator and another case are one place.

use std::path::{Path, PathBuf};

use cineris_base::PathStyle;
use cineris_base::path::{PathResolver, ResolvedPath};
use cineris_policy::lease::PROTECTED_IN_WORKSPACE;
use cineris_protocol::lease::PathSet;

/// The files protected wherever they are below the workspace: what npm,
/// yarn and `direnv` read from the folder they start in and upward.
const BUILD_FILES: [&str; 4] = [".npmrc", ".yarnrc", ".yarnrc.yml", ".envrc"];

/// The files of a `.cargo` folder that cargo reads, wherever it is.
const CARGO_FILES: [&str; 2] = ["config.toml", "config"];

/// What git needs side by side in a folder to take it for a repository.
const REPOSITORY: [&str; 3] = ["HEAD", "objects", "refs"];

/// A protected entry that a write would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Entry {
    /// In the workspace: its names from the workspace's root, joined with
    /// `/` (empty for the workspace itself).
    Inside(String),
    /// A protected path that holds the workspace, as the lease names it.
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
    let at = target.to_fs_path();
    let entry = if protect.all {
        Some(Entry::Inside(target.data()))
    } else {
        let mut places = written(&workspace, &protect.roots, style);
        leading(paths, &workspace, &mut places, style);
        entry_of(&at, &workspace, &places, style)
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
/// is judged there too. One the resolver refuses (a link to nothing) is
/// judged where it is written.
fn leading(paths: &PathResolver, workspace: &Path, places: &mut Vec<Place>, style: PathStyle) {
    let mut led = Vec::new();
    for place in places.iter() {
        // A name in the workspace leads elsewhere only through a link below
        // the workspace's real root, which is looked for first: a name with
        // none costs a look at each of its names that is there, no more.
        let asked = match &place.entry {
            Entry::Inside(names) if through_a_link(workspace, names) => names.clone(),
            Entry::Inside(_) => continue,
            Entry::Around(at) => at.to_string_lossy().into_owned(),
        };
        let Ok(resolved) = paths.resolve(&asked) else {
            continue;
        };
        let leads = resolved.to_fs_path();
        if !style.same_location(&leads, &place.at) {
            led.push(Place {
                at: leads,
                entry: place.entry.clone(),
            });
        }
    }
    places.extend(led);
}

/// Whether one of `names` (separated by `/`) is a link, from `workspace`
/// down, up to the first that is not there.
#[allow(
    clippy::disallowed_methods,
    reason = "it looks at the file system on the blocking pool"
)]
fn through_a_link(workspace: &Path, names: &str) -> bool {
    let mut at = workspace.to_path_buf();
    for name in names.split('/').filter(|name| !name.is_empty()) {
        at.push(name);
        match std::fs::symlink_metadata(&at) {
            Ok(metadata) if metadata.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

/// The protected entry `target` is, or lies under, judged by its place and
/// by its names below `workspace`. Touches nothing: the style is a
/// parameter, so that the Windows rules are tested on every system.
pub(crate) fn entry_of(
    target: &Path,
    workspace: &Path,
    places: &[Place],
    style: PathStyle,
) -> Option<Entry> {
    if let Some(place) = places
        .iter()
        .find(|place| style.location_within(target, &place.at))
    {
        return Some(place.entry.clone());
    }
    let names = names_of(&style.strip_location_prefix(target, workspace)?, style);
    let same = |name: &str, protected: &str| {
        name.eq_ignore_ascii_case(protected) || style.same_name(name, protected)
    };
    let last = names.len().checked_sub(1)?;
    for (at, name) in names.iter().enumerate() {
        let protected = same(name, ".git")
            || (at == last
                && (same(name, "HEAD")
                    || BUILD_FILES.iter().any(|file| same(name, file))
                    || (at > 0
                        && same(&names[at - 1], ".cargo")
                        && CARGO_FILES.iter().any(|file| same(name, file)))));
        if protected {
            return Some(Entry::Inside(names[..=at].join("/")));
        }
    }
    None
}

/// The first folder from `workspace` down towards `target` (the workspace
/// itself included, the target not) that git takes for a repository: one
/// that holds `HEAD`, `objects` and `refs`, as a bare repository made of
/// plain files does.
#[allow(
    clippy::disallowed_methods,
    reason = "it looks at the file system on the blocking pool"
)]
fn repository_on_the_way(target: &Path, workspace: &Path, style: PathStyle) -> Option<Entry> {
    let names = names_of(&style.strip_location_prefix(target, workspace)?, style);
    let mut folder = workspace.to_path_buf();
    for (at, name) in names.iter().enumerate() {
        let repository = REPOSITORY
            .iter()
            .all(|part| std::fs::symlink_metadata(folder.join(part)).is_ok());
        if repository {
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
    reason = "the tests plant folders, files and a link in a scratch folder"
)]
mod tests {
    use cineris_base::path::{PathOptions, PathResolver};
    use cineris_testkit::Scratch;

    use super::*;

    /// The places of a workspace with a lease that names its own protected
    /// paths, as the engine places them, and one of the user's.
    fn placed(workspace: &str, style: PathStyle, also: &[&str]) -> Vec<Place> {
        let roots: Vec<String> = also.iter().map(|root| (*root).to_owned()).collect();
        written(Path::new(workspace), &roots, style)
    }

    fn judged(target: &str, workspace: &str, places: &[Place], style: PathStyle) -> Option<String> {
        entry_of(Path::new(target), Path::new(workspace), places, style).map(|entry| match entry {
            Entry::Inside(names) => names,
            Entry::Around(root) => root.to_string_lossy().into_owned(),
        })
    }

    /// Each protected path and what is below it is refused, a protected
    /// folder itself included; so is each name protected at any depth, in
    /// another case too; and what only begins like one, or sits beside one,
    /// is not.
    #[test]
    fn every_protected_path_and_what_is_below_it_is_refused_and_nothing_beside_it() {
        let style = PathStyle::Posix;
        let ws = "/work/app";
        let places = placed(ws, style, &["/work/app/deploy/keys", "/srv/keys"]);
        let refused = |target: &str| judged(&format!("{ws}/{target}"), ws, &places, style);
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
            (".GIT/CONFIG", ".GIT"),
            ("vendor/lib/.git/config", "vendor/lib/.git"),
            ("vendor/lib/.git", "vendor/lib/.git"),
            ("fake/HEAD", "fake/HEAD"),
            ("crates/x/.cargo/config.toml", "crates/x/.cargo/config.toml"),
            ("crates/x/.CARGO/Config", "crates/x/.CARGO/Config"),
            ("web/.npmrc", "web/.npmrc"),
            ("web/app/.envrc", "web/app/.envrc"),
            ("deploy/keys/id", "deploy/keys"),
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
        let around = placed("/work/app", style, &["/work"]);
        assert_eq!(
            judged("/work/app/src/a.rs", ws, &around, style).as_deref(),
            Some("/work")
        );
    }

    /// On Windows the target is in the extended-length form the resolver
    /// gives, the lease's paths are written with `/`, and a script may
    /// write another case: all are one place. A share is a place of its
    /// own.
    #[test]
    fn on_windows_a_protected_path_is_found_whatever_its_form_and_case() {
        let style = PathStyle::Windows;
        let ws = r"\\?\C:\Work\App";
        let places = placed(
            ws,
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
        let share = r"\\server\share\app";
        let on_share = placed(share, style, &[]);
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

    /// A folder that holds `HEAD`, `objects` and `refs` is a repository to
    /// git, whatever its name: everything in it is refused, and a folder
    /// that holds only some of them is not one. The refusal names the path
    /// as code a name cannot end, and everything is refused where the lease
    /// protects everything.
    #[test]
    fn everything_in_a_folder_git_takes_for_a_repository_is_refused() {
        let (_scratch, ws) = workspace("ext_protected_repository");
        for part in ["mirror/objects", "mirror/refs", "half/objects", "half/refs"] {
            std::fs::create_dir_all(ws.join(part)).unwrap();
        }
        std::fs::write(ws.join("mirror/HEAD"), "ref: refs/heads/main\n").unwrap();
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

    /// A protected path that is a link is judged where it leads too: a
    /// `.git/hooks` that leads to `hooks` protects `hooks`.
    #[test]
    fn a_protected_path_that_is_a_link_protects_where_it_leads() {
        let (_scratch, ws) = workspace("ext_protected_link");
        std::fs::create_dir_all(ws.join(".git")).unwrap();
        std::fs::create_dir_all(ws.join("hooks")).unwrap();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(ws.join("hooks"), ws.join(".git/hooks"));
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(ws.join("hooks"), ws.join(".git/hooks"));
        if let Err(error) = made {
            // A Windows machine without the right to make links; a CI
            // runner has it, and must.
            assert!(
                std::env::var_os("GITHUB_ACTIONS").is_none(),
                "the runner cannot make a link: {error}"
            );
            return;
        }
        let paths = PathResolver::new(&ws, PathOptions::new()).unwrap();
        let refused = |target: &str| refusal(&paths, &paths.resolve(target).unwrap(), &NONE_MORE);
        let words = refused("hooks/pre-commit").expect("refused");
        assert!(
            words.starts_with("`hooks/pre-commit` is in `.git/hooks`, a protected path"),
            "{words}"
        );
        assert!(
            refused(".git/config")
                .unwrap()
                .starts_with("`.git/config` is a protected path")
        );
        assert_eq!(refused("src/main.rs"), None);
    }
}
