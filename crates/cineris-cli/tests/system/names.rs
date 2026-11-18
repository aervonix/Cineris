//! The program under both of its names (`docs/decisions/0013-naming.md`):
//! the same behaviour, and help and usage text in the name it was started
//! with.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{CINERIS, Machine};

/// Puts the binary under its second name, `cin`, in a directory of this
/// test's own, the way an installation does: a hard link, or a copy where
/// the file system cannot link.
pub fn install_alias(machine: &Machine) -> PathBuf {
    let dir = machine.home.join("bin");
    fs::create_dir_all(&dir).expect("create the alias directory");
    let alias = dir.join(format!("cin{}", std::env::consts::EXE_SUFFIX));
    if !alias.exists() && fs::hard_link(CINERIS, &alias).is_err() {
        fs::copy(CINERIS, &alias).expect("copy the binary where it cannot be linked");
    }
    alias
}

/// Asserts that `cineris <args>` and `cin <args>` exit with the same status and
/// print the same text, once the name each was started with is accounted for.
fn assert_same_under_both_names(machine: &Machine, args: &[&str], expected_code: i32) {
    let alias = install_alias(machine);
    let as_cineris = crate::Run::of(machine.command(args).output().unwrap());
    let as_cx = crate::Run::of(machine.command_as(&alias, args).output().unwrap());

    as_cineris.expect_code(expected_code);
    as_cx.expect_code(expected_code);

    // The only permitted difference is the name the program echoes back:
    // "cineris" as a whole word in usage and error text. The product name
    // "Cineris" is capitalised and so is left alone.
    let swap = |text: &str| {
        text.replace("cineris ", "cin ")
            .replace("cineris:", "cin:")
            .replace("`cineris", "`cin")
            .replace("'cineris ", "'cin ")
    };
    assert_eq!(
        as_cx.stdout,
        swap(&as_cineris.stdout),
        "standard output of cx {args:?}"
    );
    assert_eq!(
        as_cx.stderr,
        swap(&as_cineris.stderr),
        "standard error of cx {args:?}"
    );
}

#[test]
fn sys_version_is_byte_identical_under_both_names() {
    let machine = Machine::new("sys_version");
    let alias = install_alias(&machine);
    let as_cineris = machine.command(&["--version"]).output().unwrap();
    let as_cx = machine.command_as(&alias, &["--version"]).output().unwrap();

    assert!(as_cineris.status.success());
    assert!(as_cx.status.success());
    assert_eq!(as_cineris.stdout, as_cx.stdout);
    assert_eq!(
        String::from_utf8(as_cineris.stdout).unwrap().trim_end(),
        concat!("cineris ", env!("CARGO_PKG_VERSION"))
    );
    assert!(as_cineris.stderr.is_empty() && as_cx.stderr.is_empty());
}

#[test]
fn sys_help_differs_only_in_the_name_used() {
    let machine = Machine::new("sys_help");
    for args in [
        &["--help"][..],
        &["config", "--help"],
        &["sessions", "--help"],
        &["permissions", "--help"],
        &["auth", "--help"],
    ] {
        assert_same_under_both_names(&machine, args, 0);
    }
    let alias = install_alias(&machine);
    let help =
        |program: &Path| crate::Run::of(machine.command_as(program, &["--help"]).output().unwrap());
    assert!(help(&alias).stdout.contains("Usage: cin [OPTIONS]"));
    assert!(
        help(Path::new(CINERIS))
            .stdout
            .contains("Usage: cineris [OPTIONS]")
    );
    assert!(
        !help(&alias).stdout.contains("cineris "),
        "{}",
        help(&alias).stdout
    );
}

#[test]
fn sys_no_arguments_and_usage_errors_are_the_same_under_both_names() {
    let machine = Machine::new("sys_usage");
    // No prompt and no -p is the interactive application, which is not
    // built yet: a usage error, said in the name used.
    assert_same_under_both_names(&machine, &[], 2);
    assert_same_under_both_names(&machine, &["--no-such-flag"], 2);

    let alias = install_alias(&machine);
    let output = crate::Run::of(
        machine
            .command_as(&alias, &["--no-such-flag"])
            .output()
            .unwrap(),
    );
    assert!(output.stdout.is_empty());
    assert!(
        output
            .stderr
            .starts_with("cin: unexpected argument '--no-such-flag'"),
        "{}",
        output.stderr
    );
}
