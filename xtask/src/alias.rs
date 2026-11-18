//! `cargo xtask alias`: builds `cineris` and puts its alias `cin` beside it, so
//! that both names can be run from the target directory.
//!
//! One executable, two names (`docs/decisions/0038-the-project-is-named-cineris.md`). `cin` is a
//! link to `cineris`, never a second build target.

use std::env::consts::EXE_SUFFIX;
use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::steps;
use crate::workspace;

pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let release = match arguments {
        [] => false,
        [flag] if flag == "--release" => true,
        _ => return Err("usage: cargo xtask alias [--release]".to_owned()),
    };

    let mut build = vec!["build", "--package", "cineris-cli"];
    if release {
        build.push("--release");
    }
    steps::cargo(&build)?;

    let metadata = workspace::metadata(&workspace::root())?;
    let directory = metadata
        .target_directory
        .join(if release { "release" } else { "debug" });
    let (alias, how) = link(&directory)?;
    println!(
        "{} is {how} {}",
        alias.display(),
        directory.join(binary("cineris")).display()
    );
    if cfg!(windows) {
        println!("Run this command again after a rebuild: a rebuild replaces cineris.exe, and the");
        println!("link keeps pointing at the old file.");
    }
    Ok(())
}

fn binary(name: &str) -> String {
    format!("{name}{EXE_SUFFIX}")
}

/// Creates `cin` beside the `cineris` in `directory`, replacing an alias left
/// by an earlier run. Returns the alias and how it was made.
///
/// On Unix it is a relative symbolic link, which stays right across rebuilds.
/// On Windows it is a hard link, or a copy where the file system cannot
/// link: symbolic links there need a privilege most accounts do not have.
fn link(directory: &Path) -> Result<(PathBuf, &'static str)> {
    let cineris = directory.join(binary("cineris"));
    let alias = directory.join(binary("cin"));
    if !cineris.is_file() {
        return Err(format!("{} does not exist", cineris.display()));
    }
    if alias.symlink_metadata().is_ok() {
        fs::remove_file(&alias)
            .map_err(|error| format!("could not replace {}: {error}", alias.display()))?;
    }
    let how = make_link(&cineris, &alias)
        .map_err(|error| format!("could not create {}: {error}", alias.display()))?;
    Ok((alias, how))
}

#[cfg(unix)]
fn make_link(cineris: &Path, alias: &Path) -> std::io::Result<&'static str> {
    let target = cineris.file_name().unwrap_or(cineris.as_os_str());
    std::os::unix::fs::symlink(target, alias)?;
    Ok("a symbolic link to")
}

#[cfg(not(unix))]
fn make_link(cineris: &Path, alias: &Path) -> std::io::Result<&'static str> {
    if fs::hard_link(cineris, alias).is_ok() {
        return Ok("a hard link to");
    }
    fs::copy(cineris, alias)?;
    Ok("a copy of")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{binary, link};

    #[test]
    fn the_alias_is_the_same_file_under_a_second_name() {
        let directory =
            std::env::temp_dir().join(format!("cineris-xtask-alias-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(binary("cineris")), b"first build").unwrap();

        let (alias, _) = link(&directory).unwrap();
        assert_eq!(alias, directory.join(binary("cin")));
        assert_eq!(fs::read(&alias).unwrap(), b"first build");

        // Running it again replaces the alias instead of failing on it.
        fs::remove_file(directory.join(binary("cineris"))).unwrap();
        fs::write(directory.join(binary("cineris")), b"second build").unwrap();
        let (alias, _) = link(&directory).unwrap();
        assert_eq!(fs::read(&alias).unwrap(), b"second build");

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn there_is_nothing_to_link_before_the_first_build() {
        let directory =
            std::env::temp_dir().join(format!("cineris-xtask-no-build-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();

        let error = link(&directory).unwrap_err();
        assert!(error.contains("does not exist"));

        fs::remove_dir_all(&directory).unwrap();
    }
}
