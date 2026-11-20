//! The shipped binary's build (decision 0036): the `dist` profile, with the
//! build machine's folders taken out of the paths the binary holds, and the
//! check that none is left.
//!
//! A binary holds the source paths of every crate it was built from, in its
//! panic messages: the third-party crates' sources are under cargo's home
//! (`~/.cargo/registry/src/…`), and code that a build script generated is
//! under the target directory. Built at a desk, those paths name the
//! desk's user (rule 3). Cargo's `trim-paths` profile setting would take
//! them out, and is not stable on the pinned toolchain, so every build of
//! the shipped binary through `xtask` (`size`, `startup --profile dist`,
//! and step 34's `dist`) passes rustc a `--remap-path-prefix` for each of
//! those folders ([`environment_of`]), and the C compiler a
//! `-ffile-prefix-map` for each (since step 28: QuickJS's asserts name
//! their source files, which its binding compiles from its build folder;
//! MSVC has no such flag, [`c_flags`]). `size` then reads the binary for
//! any path still under a home folder ([`home_paths`]). `cargo xtask
//! home-paths FILE…` reads any built file the same way.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::Result;
use crate::rules::home_paths;
use crate::workspace;

/// The profile published binaries are built with (decision 0012). The size
/// budget is defined on it.
pub(crate) const PROFILE: &str = "dist";

/// The separator of `CARGO_ENCODED_RUSTFLAGS`, which, unlike `RUSTFLAGS`,
/// can carry a path with a space in it.
const FLAG_SEPARATOR: char = '\u{1f}';

/// The folders whose paths are rewritten, each with what it becomes, from
/// the most general to the most specific: rustc applies the last one that
/// matches, so a target directory inside the workspace, or a workspace
/// inside the home folder, is rewritten by its own entry.
#[derive(Debug, Default)]
pub(crate) struct Folders {
    /// The user's home folder: anything else under it.
    pub(crate) home: Option<PathBuf>,
    /// Cargo's home: the registry's sources and git checkouts.
    pub(crate) cargo_home: Option<PathBuf>,
    /// The toolchain's sysroot, should a path of the standard library's
    /// own sources reach the binary.
    pub(crate) sysroot: Option<PathBuf>,
    /// The workspace's root. Cargo names our crates' files relative to it
    /// already; this catches a path it gives whole.
    pub(crate) workspace: Option<PathBuf>,
    /// The target directory: the files build scripts generate.
    pub(crate) target: Option<PathBuf>,
}

impl Folders {
    /// The folders of this machine and a build of the tree at `root`, whose
    /// build directory is `target`.
    fn of_tree(root: &Path, target: &Path) -> Self {
        Self {
            home: home_folder(),
            cargo_home: cargo_home(),
            sysroot: sysroot(),
            workspace: Some(root.to_path_buf()),
            target: Some(target.to_path_buf()),
        }
    }

    /// Each folder and what it becomes, in the order rustc is given them.
    /// A folder is given as it is, and also as its canonical form where
    /// that differs (a path through a symbolic link).
    pub(crate) fn remaps(&self) -> Vec<(String, &'static str)> {
        let mut remaps: Vec<(String, &'static str)> = Vec::new();
        for (folder, to) in [
            (&self.home, "~"),
            (&self.cargo_home, "cargo"),
            (&self.sysroot, "toolchain"),
            (&self.workspace, "."),
            (&self.target, "target"),
        ] {
            let Some(folder) = folder else { continue };
            let mut spellings = vec![folder.clone()];
            if let Ok(canonical) = fs::canonicalize(folder)
                && !canonical.as_os_str().to_string_lossy().starts_with(r"\\?\")
                && canonical != *folder
            {
                spellings.push(canonical);
            }
            for spelling in spellings {
                let text = spelling.to_string_lossy().into_owned();
                // The root of a drive would rewrite every path.
                if Path::new(&text).parent().is_some() {
                    remaps.push((text, to));
                }
            }
        }
        remaps
    }
}

/// The user's home folder, as the environment gives it.
fn home_folder() -> Option<PathBuf> {
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Cargo's home, as cargo finds it: `CARGO_HOME`, or `.cargo` in the home
/// folder.
pub(crate) fn cargo_home() -> Option<PathBuf> {
    env::var_os("CARGO_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace::root().join(path)
            }
        })
        .or_else(|| home_folder().map(|home| home.join(".cargo")))
}

/// The toolchain's sysroot, from rustc.
fn sysroot() -> Option<PathBuf> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .current_dir(workspace::root())
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| PathBuf::from(text))
}

/// `CARGO_ENCODED_RUSTFLAGS` for the shipped build: the flags already set
/// for the build (`CARGO_ENCODED_RUSTFLAGS`, else `RUSTFLAGS`, which cargo
/// would otherwise read and which this replaces), then one
/// `--remap-path-prefix` for each of `remaps`.
///
/// Set, it also takes the place of every `rustflags` in cargo's
/// configuration (`build.rustflags`, `target.<triple>.rustflags` and their
/// variables). The repository sets none (`.cargo/config.toml` holds only the
/// `xtask` alias); one added there would not reach the shipped build unless
/// it is read here too.
pub(crate) fn encoded_rustflags(
    encoded: Option<&str>,
    plain: Option<&str>,
    remaps: &[(String, &'static str)],
) -> String {
    let mut flags: Vec<String> = match (encoded, plain) {
        (Some(encoded), _) if !encoded.is_empty() => {
            encoded.split(FLAG_SEPARATOR).map(str::to_owned).collect()
        }
        (_, Some(plain)) => plain.split_whitespace().map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    for (from, to) in remaps {
        flags.push(format!("--remap-path-prefix={from}={to}"));
    }
    flags.join(&FLAG_SEPARATOR.to_string())
}

/// The C compiler's flags for the shipped build for `triple`: the flags
/// already set for it, then `-ffile-prefix-map=FROM=TO` for each of
/// `remaps`, so that a C file's `__FILE__` names no folder of the build
/// machine. QuickJS's asserts name their source file, and the engine's
/// binding compiles its sources from its build folder (step 28's engine
/// check found the target folder four times in a Linux binary).
///
/// `None` for MSVC, whose compiler has no such flag, and when there is
/// nothing to add. A folder with white space in its path is left out:
/// the `cc` crate splits these flags at white space. The home-path check
/// of `size` still finds what is left.
pub(crate) fn c_flags(
    triple: &str,
    already: Option<&str>,
    remaps: &[(String, &'static str)],
) -> Option<String> {
    if triple.contains("msvc") {
        return None;
    }
    let mut flags: Vec<String> = already
        .map(|already| already.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default();
    let before = flags.len();
    for (from, to) in remaps {
        if !from.contains(char::is_whitespace) {
            flags.push(format!("-ffile-prefix-map={from}={to}"));
        }
    }
    (flags.len() > before).then(|| flags.join(" "))
}

/// The C flags the `cc` crate would use for `triple`: the first of the
/// variables it reads, in its order.
fn c_flags_already(triple: &str) -> Option<String> {
    [
        format!("CFLAGS_{triple}"),
        format!("CFLAGS_{}", triple.replace('-', "_")),
        "TARGET_CFLAGS".to_owned(),
        "CFLAGS".to_owned(),
    ]
    .iter()
    .find_map(|name| env::var(name).ok())
}

/// The environment every build of the shipped binary through `xtask` is
/// made in, for the tree at `root` with its build directory at `target`,
/// for the target `triple`.
fn environment_of(root: &Path, target: &Path, triple: &str) -> Vec<(String, String)> {
    let folders = Folders::of_tree(root, target);
    let remaps = folders.remaps();
    let encoded = env::var("CARGO_ENCODED_RUSTFLAGS").ok();
    let plain = env::var("RUSTFLAGS").ok();
    let mut environment = vec![(
        "CARGO_ENCODED_RUSTFLAGS".to_owned(),
        encoded_rustflags(encoded.as_deref(), plain.as_deref(), &remaps),
    )];
    if let Some(flags) = c_flags(triple, c_flags_already(triple).as_deref(), &remaps) {
        // The first name `cc` looks for, so that it is the one it takes.
        environment.push((format!("CFLAGS_{triple}"), flags));
    }
    environment
}

/// Builds `cineris` in the shipped profile, with the build's environment.
/// `target` is a target triple, or `None` for the host's.
pub(crate) fn build(target: Option<&str>) -> Result<()> {
    build_with(&Options {
        profile: PROFILE,
        target,
        tree: None,
        target_dir: None,
        variables: &[],
    })
}

/// How [`build_with`] builds `cineris`.
pub(crate) struct Options<'a> {
    /// The cargo profile: `dist`, or `release` for a look that is not of the
    /// shipped binary.
    pub(crate) profile: &'a str,
    /// A target triple, or `None` for the host's.
    pub(crate) target: Option<&'a str>,
    /// Another checkout of the workspace to build, or `None` for this one.
    pub(crate) tree: Option<&'a Path>,
    /// A build directory of its own, or `None` for the tree's.
    pub(crate) target_dir: Option<&'a Path>,
    /// More environment: a profile setting, as `CARGO_PROFILE_DIST_STRIP`.
    pub(crate) variables: &'a [(&'a str, &'a str)],
}

/// Builds `cineris` as `options` say, with the build machine's folders taken
/// out of its paths.
pub(crate) fn build_with(options: &Options) -> Result<()> {
    let root = options.tree.map_or_else(workspace::root, Path::to_path_buf);
    let target_dir = match options.target_dir {
        Some(directory) => directory.to_path_buf(),
        None => workspace::metadata(&root)?.target_directory,
    };
    let mut arguments = vec![
        "build",
        "--locked",
        "--profile",
        options.profile,
        "--package",
        "cineris-cli",
        "--bin",
        "cineris",
    ];
    if let Some(target) = options.target {
        arguments.extend(["--target", target]);
    }
    let triple = match options.target {
        Some(target) => target.to_owned(),
        None => crate::nonblocking::host_target()?,
    };
    println!("$ cargo {}", arguments.join(" "));
    let mut command = Command::new(workspace::cargo());
    command
        .args(&arguments)
        .envs(environment_of(&root, &target_dir, &triple))
        .envs(options.variables.iter().copied())
        // A token for `gh` (a `try` run of `size` has one) is no build
        // script's or procedural macro's business.
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .current_dir(&root);
    if options.target_dir.is_some() {
        // Whatever the caller's environment says: a build of its own never
        // shares, or rebuilds, another's.
        command
            .env("CARGO_TARGET_DIR", &target_dir)
            .env_remove("CARGO_BUILD_TARGET_DIR");
    }
    let status = command
        .status()
        .map_err(|error| format!("could not run cargo {}: {error}", arguments.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", arguments.join(" ")))
    }
}

/// How many places a file holds a path under a home folder, each kind
/// once: `/home/<user>`, `C:\Users\<user>`, and each of `own`, this
/// build's own folders by name (its cargo home, its home folder, which may
/// be of no common shape: `/root` in a container), as descriptions with the
/// user's name left out, each with how often it occurs and an example of
/// what follows it.
pub(crate) fn home_paths(bytes: &[u8], own: &[(&str, &Path)]) -> Vec<String> {
    let mut found: Vec<(String, usize, String)> = Vec::new();
    let mut note = |kind: String, end: usize| {
        let example: String = String::from_utf8_lossy(&bytes[end..bytes.len().min(end + 60)])
            .chars()
            .take_while(|character| !character.is_control())
            .collect();
        match found.iter_mut().find(|(seen, _, _)| *seen == kind) {
            Some((_, count, _)) => *count += 1,
            None => found.push((kind, 1, example)),
        }
    };
    for (start, path) in home_paths::find_in_binary(bytes) {
        note(without_the_user(&path), start + path.len());
    }
    for (name, folder) in own {
        let text = folder.to_string_lossy();
        // A drive's root, or a short name, would be found in ordinary text.
        if folder.parent().is_none() || text.len() < 4 {
            continue;
        }
        let mut spellings = vec![text.to_string(), text.replace('\\', "/")];
        spellings.dedup();
        for spelling in spellings {
            for start in find_all(bytes, spelling.as_bytes()) {
                let end = start + spelling.len();
                // The folder itself, not a longer name that starts with it.
                if bytes
                    .get(end)
                    .is_none_or(|&byte| !is_folder_name_byte(byte))
                {
                    note((*name).to_owned(), end);
                }
            }
        }
    }
    found
        .into_iter()
        .map(|(kind, count, example)| {
            format!("{kind}, {count} times; the first followed by `{example}`")
        })
        .collect()
}

/// A byte that can continue a folder's name: the end of a match is not
/// the end of the folder when one follows.
fn is_folder_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') || byte >= 0x80
}

/// This build's own folders, for [`home_paths`].
fn own_folders() -> Vec<(&'static str, PathBuf)> {
    [
        ("the build's CARGO_HOME", cargo_home()),
        ("the build's home folder", home_folder()),
    ]
    .into_iter()
    .filter_map(|(name, folder)| Some((name, folder?)))
    .collect()
}

/// A home path with the user's name taken out: `/home/<user>`.
fn without_the_user(path: &str) -> String {
    let cut = path.rfind(['/', '\\']).map_or(0, |separator| separator + 1);
    format!("{}<user>", &path[..cut])
}

/// Every offset at which `needle` starts in `bytes`, ignoring ASCII case
/// (Windows' paths are not told apart by it).
fn find_all(bytes: &[u8], needle: &[u8]) -> Vec<usize> {
    if needle.is_empty() || bytes.len() < needle.len() {
        return Vec::new();
    }
    let first = needle[0].to_ascii_lowercase();
    (0..=bytes.len() - needle.len())
        .filter(|&start| {
            bytes[start].to_ascii_lowercase() == first
                && bytes[start..start + needle.len()].eq_ignore_ascii_case(needle)
        })
        .collect()
}

/// Fails, saying where, when the file at `path` holds a path under a home
/// folder or under this build's cargo home.
pub(crate) fn check_file(path: &Path) -> Result<()> {
    let bytes =
        fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let own = own_folders();
    let own: Vec<(&str, &Path)> = own
        .iter()
        .map(|(name, folder)| (*name, folder.as_path()))
        .collect();
    let found = home_paths(&bytes, &own);
    if found.is_empty() {
        println!("  paths    none under a home folder, cargo's home or this build's home");
        return Ok(());
    }
    Err(format!(
        "{} holds paths of the machine it was built on (rule 3 in docs/build-plan.md):\n    {}\n\
         Build it with `cargo xtask size`, which takes them out (decision 0036), and look for \
         what passed them by: a `RUSTFLAGS` that replaced the build's, a folder that is not \
         remapped.",
        path.display(),
        found.join("\n    ")
    ))
}

/// `cargo xtask home-paths FILE…`
pub(crate) fn home_paths_command(arguments: &[String]) -> Result<()> {
    if arguments.is_empty() || arguments.iter().any(|argument| argument.starts_with('-')) {
        return Err(
            "usage: cargo xtask home-paths FILE...\n\nFails when a built file holds a path under \
             a home folder (/home/, /Users/, C:\\Users\\) or under cargo's home."
                .to_owned(),
        );
    }
    let mut failed = Vec::new();
    for file in arguments {
        println!("{file}");
        if let Err(message) = check_file(Path::new(file)) {
            eprintln!("{message}");
            failed.push(file.as_str());
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("home paths in {}", failed.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The paths are assembled from pieces, so that this file holds none.
    fn joined(parts: &[&str], separator: &str) -> String {
        parts.join(separator)
    }

    #[test]
    fn each_folder_is_remapped_from_the_most_general_to_the_most_specific() {
        let home = joined(&["", "home", "ada"], "/");
        let folders = Folders {
            home: Some(PathBuf::from(&home)),
            cargo_home: Some(PathBuf::from(format!("{home}/.cargo"))),
            sysroot: Some(PathBuf::from(format!("{home}/.rustup/toolchains/x"))),
            workspace: Some(PathBuf::from(format!("{home}/src/cineris"))),
            target: Some(PathBuf::from(format!("{home}/src/cineris/target"))),
        };
        let remaps = folders.remaps();
        let order: Vec<&str> = remaps.iter().map(|(_, to)| *to).collect();
        assert_eq!(order, ["~", "cargo", "toolchain", ".", "target"]);
        assert_eq!(remaps[1].0, format!("{home}/.cargo"));
    }

    #[test]
    fn a_folder_at_the_root_of_a_drive_is_not_remapped() {
        let folders = Folders {
            home: Some(PathBuf::from("/")),
            cargo_home: Some(PathBuf::from("/cargo")),
            ..Folders::default()
        };
        assert_eq!(folders.remaps(), [("/cargo".to_owned(), "cargo")]);
    }

    #[test]
    fn the_remaps_follow_the_flags_already_set() {
        let remaps = vec![("/x/.cargo".to_owned(), "cargo"), ("/x/w".to_owned(), ".")];
        let separator = FLAG_SEPARATOR.to_string();
        let ours =
            format!("--remap-path-prefix=/x/.cargo=cargo{separator}--remap-path-prefix=/x/w=.");
        assert_eq!(encoded_rustflags(None, None, &remaps), ours);
        // `RUSTFLAGS` is read when nothing encoded is set, and kept.
        assert_eq!(
            encoded_rustflags(None, Some(" -C  target-cpu=native "), &remaps),
            format!("-C{separator}target-cpu=native{separator}{ours}")
        );
        // The encoded form wins, as it does in cargo.
        assert_eq!(
            encoded_rustflags(Some("-Cdebuginfo=1"), Some("-C x"), &remaps),
            format!("-Cdebuginfo=1{separator}{ours}")
        );
        // A path with a space survives.
        let spaced = vec![("/x/my files".to_owned(), ".")];
        assert_eq!(
            encoded_rustflags(None, None, &spaced),
            "--remap-path-prefix=/x/my files=."
        );
    }

    #[test]
    fn the_c_compiler_is_given_the_same_remaps_except_on_msvc() {
        let remaps = vec![
            ("/x/.cargo".to_owned(), "cargo"),
            ("/x/w/target".to_owned(), "target"),
        ];
        let ours = "-ffile-prefix-map=/x/.cargo=cargo -ffile-prefix-map=/x/w/target=target";
        assert_eq!(
            c_flags("x86_64-unknown-linux-musl", None, &remaps).as_deref(),
            Some(ours)
        );
        assert_eq!(
            c_flags("x86_64-unknown-linux-gnu", None, &remaps).as_deref(),
            Some(ours)
        );
        // MSVC's compiler has no such flag; given one, it fails the build.
        assert_eq!(c_flags("x86_64-pc-windows-msvc", None, &remaps), None);
        // The flags already set are kept, ahead of ours.
        assert_eq!(
            c_flags("x86_64-unknown-linux-gnu", Some(" -O2  -g "), &remaps).as_deref(),
            Some(format!("-O2 -g {ours}").as_str())
        );
        // `cc` splits at white space, so a folder with a space is left out,
        // and with nothing to add nothing is set.
        let spaced = vec![("/x/my files".to_owned(), ".")];
        assert_eq!(c_flags("x86_64-unknown-linux-gnu", None, &spaced), None);
    }

    #[test]
    fn home_paths_in_a_binary_are_found_without_the_users_name() {
        let linux = joined(&["", "home", "ada", ".cargo", "registry"], "/");
        let windows = joined(&["C:", "Users", "ada", "src"], "\\");
        let mut bytes = b"\x7fELF\0\0".to_vec();
        for _ in 0..3 {
            bytes.extend_from_slice(linux.as_bytes());
            bytes.push(0);
        }
        bytes.extend_from_slice(windows.as_bytes());
        bytes.push(0);
        let found = home_paths(&bytes, &[]);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(
            found[0].starts_with("/home/<user>, 3 times")
                && found[0].ends_with("`/.cargo/registry`"),
            "{found:?}"
        );
        assert!(
            found[1].starts_with("C:\\Users\\<user>, 1 times"),
            "{found:?}"
        );
        assert!(!found.concat().contains("ada"), "{found:?}");
    }

    #[test]
    fn a_path_packed_against_the_string_before_it_is_found() {
        // A binary's strings have nothing between them.
        let bytes = format!("src/lib.rs{}/src/x.rs", joined(&["", "home", "ada"], "/"));
        let found = home_paths(bytes.as_bytes(), &[]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].starts_with("/home/<user>, 1 times"), "{found:?}");
    }

    #[test]
    fn the_builds_cargo_home_is_found_in_either_spelling_and_any_case() {
        let cargo_home = PathBuf::from(joined(&["D:", "a", "_temp", "cargo"], "\\"));
        let backslashes = joined(&["d:", "A", "_temp", "cargo", "registry"], "\\");
        let forward = joined(&["D:", "a", "_temp", "cargo", "git"], "/");
        let bytes = format!("x{backslashes}\0{forward}\0").into_bytes();
        let found = home_paths(&bytes, &[("the build's CARGO_HOME", cargo_home.as_path())]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("the build's CARGO_HOME, 2 times"),
            "{found:?}"
        );
    }

    #[test]
    fn a_name_outside_ascii_and_the_builds_own_home_are_found() {
        let accented = joined(&["", "home", "\u{e9}mile", "src"], "/");
        let root = joined(&["", "root", ".cargo", "registry"], "/");
        let bytes = format!("{accented}\0{root}\0/rootfs/etc\0").into_bytes();
        let found = home_paths(&bytes, &[("the build's home folder", Path::new("/root"))]);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].starts_with("/home/<user>, 1 times"), "{found:?}");
        // `/rootfs` is another folder, not the home folder.
        assert!(
            found[1].starts_with("the build's home folder, 1 times")
                && found[1].ends_with("`/.cargo/registry`"),
            "{found:?}"
        );
    }

    #[test]
    fn the_builds_own_folders_are_its_cargo_home_and_its_home_folder() {
        // What `check_file` looks for by name, from this process's
        // environment: both are set wherever the tests run.
        let own = own_folders();
        let named = |name: &str| own.iter().find(|(own, _)| *own == name).map(|(_, f)| f);
        assert_eq!(named("the build's home folder"), home_folder().as_ref());
        assert_eq!(named("the build's CARGO_HOME"), cargo_home().as_ref());
        assert!(home_folder().is_some(), "no home folder in the environment");
    }

    #[test]
    fn a_binary_with_the_folders_remapped_passes() {
        let bytes =
            b"\0cargo/registry/src/index.crates.io-1949cf8c6b5b557f/serde-1.0.229/src/de.rs\0\
            ./target/dist/build/x/out/generated.rs\0/rustc/58cf8f9c3/library/core/src/str/mod.rs\0\
            ~/.config/cineris\0";
        assert_eq!(
            home_paths(
                bytes,
                &[("the build's CARGO_HOME", Path::new("/opt/cargo"))]
            ),
            Vec::<String>::new()
        );
    }
}
