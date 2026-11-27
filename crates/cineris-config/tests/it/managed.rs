//! The managed file is an administrator's (since the signed approvals
//! step): one the user could change is not used, and the load says so; one
//! that is not there is no layer, as before; and on Windows its folder is
//! the one the system records, whatever the variables say.
//!
//! The rules themselves are tested in `cineris-config`'s `managed` module,
//! from the facts, on every system. Here the real file system gives them:
//! a test can make a file the user owns, and on Windows, where the runner
//! is an administrator, `icacls` can hand a file to the administrators.

use cineris_config::LayerKind;

use crate::Machine;

/// What the managed file of these tests says: a level, and a lock on it.
const MANAGED: &str = "[managed]\nlocked = [\"permissions.mode\"]\n\
                       [permissions]\nmode = \"read-only\"\n";

/// The user's own managed file, checked as the system's is: not used, with
/// a warning that says why. Unchecked, as every other test reads it, the
/// same file applies, so it is the check that leaves it out.
///
/// On Windows the runner is an administrator, and what it makes may be an
/// administrator's: `sys_who_can_change_the_managed_file_decides_whether_it_is_used`
/// gives the file to the user there.
#[cfg(unix)]
#[test]
fn a_managed_file_the_user_owns_is_not_used() {
    use std::os::unix::fs::MetadataExt;

    use crate::value;

    let mut machine = Machine::new("managed_users_own");
    machine.layer(LayerKind::Managed, MANAGED);
    let file = machine.file(LayerKind::Managed);
    if std::fs::metadata(&file).unwrap().uid() == 0 {
        println!("skipped: run as root, every file the test makes is root's");
        return;
    }
    assert_eq!(value(&machine.load(), "permissions.mode"), "\"read-only\"");

    machine.options.dirs.check_managed_config();
    let loaded = machine.load();
    assert_eq!(value(&loaded, "permissions.mode"), "\"supervised\"");
    let said = machine.said(&loaded);
    assert_eq!(said.len(), 1, "{said:?}");
    // The first name on the way the user could change: a folder of theirs,
    // or one written by others.
    assert!(
        said[0].starts_with("warning: ~/managed/config.toml: is not used: `")
            && (said[0].contains(", not by root.") || said[0].contains("` can be written by "))
            && said[0].ends_with(". A managed file must be one only an administrator can change"),
        "{said:?}"
    );
    // It is there: what the load looked for says so.
    assert!(
        loaded
            .files
            .iter()
            .any(|(layer, _, there)| *layer == LayerKind::Managed && *there)
    );
}

/// A managed file that is not there is no layer and no warning, checked or
/// not.
#[test]
fn a_missing_managed_file_is_no_layer_checked_or_not() {
    let mut machine = Machine::new("managed_missing");
    machine.options.dirs.check_managed_config();
    let loaded = machine.load();
    assert!(loaded.diagnostics.is_empty(), "{:?}", machine.said(&loaded));
    assert!(
        loaded
            .files
            .iter()
            .any(|(layer, _, there)| *layer == LayerKind::Managed && !*there)
    );
    // Nor is one whose folder is not there.
    machine.options.dirs.managed_config = Some(machine.home().join("nowhere").join("config.toml"));
    machine.options.dirs.check_managed_config();
    assert!(machine.load().diagnostics.is_empty());
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use cineris_config::{Dirs, Env, LayerKind};

    use super::MANAGED;
    use crate::{Machine, value};

    fn in_ci() -> bool {
        std::env::var_os("GITHUB_ACTIONS").is_some()
    }

    /// Runs `icacls` on `path`. `false` when it could not do it, which off
    /// the runner is a machine where the test is not an administrator: the
    /// test then says so and stops.
    fn icacls(path: &Path, args: &[&str]) -> bool {
        let output = Command::new("icacls")
            .arg(path)
            .args(args)
            .output()
            .expect("run icacls");
        if !output.status.success() {
            let said = String::from_utf8_lossy(&output.stdout).into_owned();
            assert!(
                !in_ci(),
                "the CI runner could not run icacls {args:?}: {said}"
            );
            println!("skipped: icacls {args:?} failed here ({said})");
        }
        output.status.success()
    }

    /// SYSTEM, the administrators and every user, each as `icacls` grants.
    const SYSTEM: &str = "*S-1-5-18";
    const ADMINISTRATORS: &str = "*S-1-5-32-544";
    const USERS: &str = "*S-1-5-32-545";

    /// Makes the folder and the file an administrator's, as an
    /// administrator would: owned by the group, kept from the folder above,
    /// and every user may read them and nothing more.
    fn administrators_only(folder: &Path, file: &Path) -> bool {
        let full_folder = |who: &str| format!("{who}:(OI)(CI)F");
        let read_folder = format!("{USERS}:(OI)(CI)RX");
        let full_file = |who: &str| format!("{who}:F");
        let read_file = format!("{USERS}:RX");
        icacls(
            folder,
            &[
                "/inheritance:r",
                "/grant:r",
                &full_folder(SYSTEM),
                &full_folder(ADMINISTRATORS),
                &read_folder,
            ],
        ) && icacls(folder, &["/setowner", ADMINISTRATORS])
            && icacls(
                file,
                &[
                    "/inheritance:r",
                    "/grant:r",
                    &full_file(SYSTEM),
                    &full_file(ADMINISTRATORS),
                    &read_file,
                ],
            )
            && icacls(file, &["/setowner", ADMINISTRATORS])
    }

    /// The same file, each way the user could change it: the user's own,
    /// written by the user, in a folder the user may delete files from.
    /// An administrator's, in an administrator's folder, is used. The
    /// runner is an administrator, and the check sets that aside: what
    /// counts is what the user may do without elevating.
    #[test]
    fn sys_who_can_change_the_managed_file_decides_whether_it_is_used() {
        let mut machine = Machine::new("managed_windows_owners");
        machine.layer(LayerKind::Managed, MANAGED);
        machine.options.dirs.check_managed_config();
        let file = machine.file(LayerKind::Managed);
        let folder = file.parent().unwrap().to_path_buf();
        let user = format!("*{}", cineris_base::private::current_user_sid().unwrap());

        let used = |machine: &Machine| {
            let loaded = machine.load();
            let said = machine.said(&loaded);
            (value(&loaded, "permissions.mode") == "\"read-only\"", said)
        };

        if !administrators_only(&folder, &file) {
            return;
        }
        let (applied, said) = used(&machine);
        assert!(applied && said.is_empty(), "{said:?}");

        // The user's own file.
        assert!(icacls(&file, &["/setowner", &user]));
        let (applied, said) = used(&machine);
        assert!(!applied, "{said:?}");
        assert!(
            said[0].contains("config.toml` is owned by S-1-5-21-"),
            "{said:?}"
        );

        // The administrators', which the user may write.
        assert!(icacls(&file, &["/setowner", ADMINISTRATORS]));
        assert!(icacls(&file, &["/grant", &format!("{user}:(W)")]));
        let (applied, said) = used(&machine);
        assert!(!applied, "{said:?}");
        assert!(said[0].contains("you may write to `"), "{said:?}");

        // An administrator's file in a folder the user may delete it from.
        assert!(administrators_only(&folder, &file));
        assert!(icacls(&folder, &["/grant", &format!("{user}:(DC)")]));
        let (applied, said) = used(&machine);
        assert!(!applied, "{said:?}");
        assert!(said[0].contains("you may delete files in `"), "{said:?}");

        // A folder of the user's, as a user can make one under
        // `C:\ProgramData`.
        assert!(administrators_only(&folder, &file));
        assert!(icacls(&folder, &["/setowner", &user]));
        let (applied, said) = used(&machine);
        assert!(!applied, "{said:?}");
        assert!(
            said[0].contains("managed` is owned by S-1-5-21-"),
            "{said:?}"
        );

        // Each warning is one line, of the managed layer, and the load
        // goes on.
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("warning: ~/managed/config.toml: is not used: "));
        // Given back, so that the scratch folder can be removed.
        assert!(administrators_only(&folder, &file));
    }

    /// The program data folder is the one the system records: a process
    /// whose `ProgramData`, `ALLUSERSPROFILE` and `SystemDrive` name a
    /// folder of the user's finds the same folder as this one, and its
    /// managed file there.
    #[test]
    fn sys_the_program_data_folder_is_the_systems_whatever_the_variables_say() {
        let scratch = cineris_testkit::scratch!("managed_program_data");
        let ours = Env::from_process()
            .program_data()
            .map(Path::to_path_buf)
            .expect("the system records a program data folder");
        assert!(
            ours.to_string_lossy()
                .to_ascii_lowercase()
                .ends_with(r":\programdata"),
            "{}",
            ours.display()
        );

        let decoy = scratch.join("decoy");
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "managed::windows::prints_the_program_data_folder",
                "--ignored",
                "--nocapture",
            ])
            .env("ProgramData", &decoy)
            .env("ALLUSERSPROFILE", &decoy)
            .env("SystemDrive", "Q:")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{stdout}");
        let said = |label: &str| {
            stdout
                .lines()
                .find_map(|line| line.strip_prefix(label))
                .map(PathBuf::from)
                .unwrap_or_else(|| panic!("no `{label}` line: {stdout}"))
        };
        assert_eq!(said("program data: "), ours);
        assert_eq!(said("managed: "), ours.join("cineris").join("config.toml"));
    }

    /// Run by the test above in a process of its own, with the variables
    /// it names: what that process finds.
    #[test]
    #[ignore = "run by sys_the_program_data_folder_is_the_systems_whatever_the_variables_say"]
    fn prints_the_program_data_folder() {
        let env = Env::from_process();
        let shown = |path: Option<&Path>| path.map(|path| path.display().to_string());
        println!(
            "program data: {}",
            shown(env.program_data()).unwrap_or_default()
        );
        println!(
            "managed: {}",
            shown(Dirs::from_env(&env).managed_config.as_deref()).unwrap_or_default()
        );
    }
}
