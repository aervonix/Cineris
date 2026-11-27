# Signed approvals, part: the managed configuration is the administrator's

The managed configuration file (an administrator's settings, which outrank the user's own: its rules, its level and its locks) is the one layer the model's code is meant not to be able to write. The review of the design ([answered.md](../answered.md), "The trust anchor") found two ways it was not what it claimed, and this part closes both ([08-configuration.md](../../../architecture/08-configuration.md), "The managed file is an administrator's").

## What was built

- **No variable moves the managed file.** On Windows its folder came from the `ProgramData` variable, so after `setx ProgramData …` every later run of the user's read a "managed" file the model wrote. It is now the program data folder the system records: the value `ProgramData` of `HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList` (the one the system's known folder and every logon's `ProgramData` and `ALLUSERSPROFILE` variables are made from), read with `RRF_NOEXPAND`, its `%SystemDrive%` put in from the system's Windows directory (`GetSystemWindowsDirectoryW`) and any other variable refused (`cineris_base::program_data`, a new module, the third of `cineris-base` allowed `unsafe`). `Env::from_process` asks for it, `Env::program_data` gives it, and `Dirs::from_env` places the managed file in it; a test gives a folder with `Env::with_program_data`, and no variable does. On Linux the place was and is fixed.
- **A managed file the user could change is not used.** Before a load reads the managed file `Dirs::from_env` found, `cineris-config`'s `managed` module looks at who could change it. One that fails is left out with a warning that names what the user could change, and the run goes on. A test's managed file, in a folder of its own, is read unchecked unless the test asks for the check (`Dirs::check_managed_config`): no test can make a file root's or the administrators'.
- **Windows's own access check, in `cineris-base`:** `private::standing_without_administrators` reads an object's owner and asks `AccessCheck`, with the process's token and the Administrators group made deny-only (`CreateRestrictedToken`), what the user may do to it, without following a link.

## The rules

- **Linux:** the file, and every folder the system looks it up through (the folders of each link on the way included, its target walked as the system walks it, at most 40 links), must be owned by root and not writable by the group or by others. A folder that others may write and that has the sticky bit passes when the name looked up in it is root's (the Nix store's `1775`, which a NixOS `/etc` links into). A file or folder the user owns, unless the user is root, is refused. A link's own mode and owner are not asked.
- **Windows:** the file and its folder must each be owned by SYSTEM, the Administrators group or TrustedInstaller, and be no link or junction (neither is followed: the folder is looked at first, without following it, so that a junction to a network share sends no credentials). With the Administrators group set aside, the user may not write to the file, append to it, delete it, or change its access list or owner; nor delete files in the folder, delete or rename it, or change its access list or owner. Setting the group aside makes an administrator's elevated run judge the file as their ordinary one. **Adding a file to the folder is allowed:** an administrator's folder under `C:\ProgramData` lets every user add files by default (an inherited entry for `Users`), the file the user would add is theirs and refused by the file's check, and the file read cannot be replaced without deleting it. A folder the user made under `C:\ProgramData` is theirs and refused by its owner.
- **A file that is not there** is no managed layer, as before. On Linux a file that is there and cannot be looked at, under folders all root's, is an error, as before. On Windows what cannot be looked at is refused with the warning, since a file the user made can deny the user the right to read its owner.

**Not used, not stopped (recommended, and built).** A refused file leaves the run where it would be with no managed file, which the user's own configuration already allows. Stopping instead would hand the model's code a way to stop every later run: on Windows any user may make `C:\ProgramData\cineris\config.toml` where an administrator has made none. The warning is among what a session is told when it opens, and `cineris config check` prints it.

## What this does not close

- **Elevation without the person.** On Linux under WSL with Windows interop on (the default), `wsl.exe -u root` gives any program of the user's a root shell with no password, and `/etc/cineris` is then the model's to write; a `sudo` rule with `NOPASSWD` is the same. On a Windows administrator's account with UAC at its default level, ways to elevate without a prompt are publicly known, and Microsoft does not treat UAC as a boundary. Both are residuals of the design.
- **A harness the model starts itself,** in a user namespace of its own where what it mounts appears as root's: a run the model started, which the cap on nested runs covers, not the person's.
- **A file that is not there** is no layer, whoever could have put one there: an administrator who wants a policy writes the file.

## Tests

| Test | What it shows | Ran |
|---|---|---|
| `cineris-config managed::tests::on_linux_*` (four) | Linux's rule from its facts: root's file and folders pass, read-only or not; anyone else's, or one written by the group or others, at any place, is refused; a sticky folder passes only over a name of root's; a link's own mode is not asked | Passed, Linux, on the reference machine (`cargo xtask test -p cineris-config`) |
| `cineris-config managed::tests::on_windows_*` (three) | Windows's rule from its facts, on every system: the three owners pass with the default rights; every other owner and a link are refused; each right to change the file or its folder refuses alone | Passed on Linux; on Windows on the runners after the push |
| `cineris-config managed::tests::on_linux_the_walk_lists_every_folder_and_link_on_the_way` | The walk follows a relative and an absolute link and lists every folder and link; a missing name, a file used as a folder, a loop of links and a relative path stop it | Passed, Linux, on the reference machine (`cargo xtask test -p cineris-config`) |
| `cineris-config managed::tests::on_linux_a_file_of_the_users_is_refused_and_one_of_roots_passes` | A real file of the test user's is refused; `/etc/passwd` passes | Passed, Linux, on the reference machine (`cargo xtask test -p cineris-config`) |
| `cineris-config::it managed::a_managed_file_the_user_owns_is_not_used` (Linux) | Through `load`: the same file applies unchecked and is left out checked, with one warning; the load goes on | Passed, Linux, on the reference machine (`cargo xtask test -p cineris-config`) |
| `cineris-config::it managed::a_missing_managed_file_is_no_layer_checked_or_not` | No file, or no folder: no layer and nothing said | Passed on Linux; on Windows on the runners after the push |
| `cineris-config dirs::tests::no_variable_moves_the_managed_file`, `windows_follows_its_own_folders` | `ProgramData`, `ALLUSERSPROFILE` and `SystemDrive` move nothing; without the system's folder there is no managed file | Passed on Linux; on Windows on the runners after the push |
| `cineris-base program_data::tests::the_stored_folder_takes_the_systems_drive_and_no_variable_else` | The stored value's expansion: the system's drive and root, nothing else | Passed on Linux (`cargo xtask test -p cineris-base`); on Windows on the runners after the push |
| `cineris-base program_data::windows::tests::the_folder_is_the_machines_program_data` (Windows) | The folder read from the system is on the system's drive, ends `\ProgramData` and exists | Not run: Windows, on the runners after the push |
| `cineris-config::it managed::windows::sys_who_can_change_the_managed_file_decides_whether_it_is_used` (Windows) | With `icacls`: an administrator's file in an administrator's folder is used; the user's own file, one the user may write, one in a folder the user may delete from, and one in a folder the user owns are each left out, with the reason | Not run: Windows, on the runners after the push |
| `cineris-config::it managed::windows::sys_the_program_data_folder_is_the_systems_whatever_the_variables_say` (Windows) | A process whose `ProgramData`, `ALLUSERSPROFILE` and `SystemDrive` name a folder of the user's finds the same folder, and its managed file there | Not run: Windows, on the runners after the push |

Besides: `cargo xtask test -p cineris-base` (339 passed) and `cargo xtask test --dependents -p cineris-config` (`cineris-cli`, `cineris-core` and `cineris-rpc` with it: 1,189 passed), on the reference machine. The Windows code is linted from Linux (`cargo xwin clippy --target x86_64-pc-windows-msvc --workspace --all-targets --locked -- -D warnings`, clean); it has not run.

## Breaks

`mutations/signed-approvals-managed-config.breaks`: `programdata-variable-moves-the-managed-file`, `load-reads-the-managed-file-unchecked`, `linux-owner-not-asked`, `windows-folder-delete-child-not-asked`, each naming the tests that must catch it. The pre-flight passes, and every `caught-by` names a test that exists (`cargo xtask mutate --check --tests` with the four names: 8 names checked). The breaks have not been run: `linux-owner-not-asked` and `load-reads-the-managed-file-unchecked` on Linux, the other two on either system.

## Measured

What the check adds to a load, on the reference machine with other builds running beside it (not an otherwise idle machine), release build, the median of 4,000 loads, three rounds (a throwaway test run with `cargo xtask scratch-test cineris-config FILE --cargo-profile release --ignored`):

| Load | Median |
|---|---|
| No managed file named at all | 8.30 to 8.37 µs |
| The system's place, `/etc/cineris/config.toml`, checked and not there (what every Linux run without a managed file does: `lstat` of `/`, `/etc` and `/etc/cineris`) | 9.36 to 9.39 µs, so 1.0 µs more, the failed open that came before the check included |
| A file of the user's, refused (its walk from `/`) | 11.7 to 11.8 µs |
| The same file read unchecked, as tests read it | 12.7 to 14.0 µs |

Within the noise of the start-up budgets. **Windows is not measured:** each `Env::from_process` reads one registry value and the Windows directory (in libraries every start loads already; `SHGetKnownFolderPath` would have added `shell32.dll` and `combase.dll` to every start), and where there is a managed folder, a load opens it and the file to read their access lists and asks the access check twice. A `try` run of `startup -f profile=dist -f against=main` on Windows would show it.
