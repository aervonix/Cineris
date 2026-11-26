//! A file a rewind puts back has the mode it had
//! (`docs/architecture/07-sessions-and-context.md`, "Snapshots and undo").
//!
//! Since the step 25 fixes a kept version records the file's mode, read
//! from the handle its content was read through, and a file made again is
//! made with it: the review of step 25 found every file made again with
//! the default mode, a script no longer executable and a private file
//! readable by others. A log written before the field reads back exactly
//! as before, and its files are made as they always were.

use std::fs;

use cineris_base::ContentHash;
use cineris_protocol::ids::TurnId;
use cineris_session::entry::{Before, Body, Checkpoint, FileChange, FileVersion};
use cineris_session::{Restore, RoundCheckpoint, Session};

use crate::{World, user};

/// A tool changing (`Some`) or removing (`None`) a file, through a round:
/// what it held, and its mode, are read through the file's handle first.
fn change(
    world: &World,
    round: &mut RoundCheckpoint,
    session: &Session,
    path: &str,
    to: Option<&[u8]>,
) {
    let file = world.abs(path);
    round.before(&session.store(), &file).unwrap();
    match to {
        Some(content) => {
            world.write(path, content);
            round.after(
                &file,
                Some(FileVersion::new(
                    ContentHash::of(content),
                    content.len() as u64,
                    true,
                )),
            );
        }
        None => {
            fs::remove_file(world.file(path)).unwrap();
            round.after(&file, None);
        }
    }
}

/// One turn, one round, of `changes`.
fn a_turn(world: &World, session: &mut Session, changes: &[(&str, Option<&[u8]>)]) {
    let turn = TurnId::new("t_1");
    session.append(Some(&turn), user("change them")).unwrap();
    let mut round = RoundCheckpoint::new(world.root.clone());
    for (path, to) in changes {
        change(world, &mut round, session, path, *to);
    }
    if let Some(body) = round.finish().unwrap() {
        session.append(Some(&turn), body).unwrap();
    }
}

/// Undoes the turn, files and all, overwriting what was edited since.
fn undo(session: &mut Session) -> cineris_session::RewindDone {
    let plan = session.plan_undo().unwrap().unwrap();
    let everything = plan.files.iter().map(|file| file.path.clone()).collect();
    session
        .rewind(
            &plan,
            &Restore {
                files: true,
                overwrite_edited: everything,
            },
        )
        .unwrap()
}

/// The text of the session's log.
fn log_of(session: &Session) -> String {
    fs::read_to_string(session.log().dir().log()).unwrap()
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::Path;

    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    fn set(path: &Path, bits: u32) {
        fs::set_permissions(path, fs::Permissions::from_mode(bits)).unwrap();
    }

    /// What the process's umask takes from a file it makes.
    fn umask(world: &World) -> u32 {
        let probe = world.scratch.join("umask-probe");
        fs::create_dir(&probe).unwrap();
        let mask = !mode(&probe) & 0o777;
        fs::remove_dir(&probe).unwrap();
        mask
    }

    /// Removed files, one in a folder removed with it, come back with the
    /// mode they had, less the umask as any new file; a set-id bit never
    /// comes back from a log. The log records each mode in octal.
    #[test]
    fn a_removed_file_is_made_again_with_the_mode_it_had() {
        let world = World::new("put_back_removed");
        let files = [
            ("bin/run.sh", 0o755),
            ("private.txt", 0o600),
            ("shared.txt", 0o640),
            ("tool", 0o4755),
            ("read-only.txt", 0o444),
        ];
        for (path, bits) in files {
            world.write(path, path.as_bytes());
            set(&world.file(path), bits);
        }
        let mut session = world.create("s_1");
        let removed: Vec<(&str, Option<&[u8]>)> =
            files.iter().map(|(path, _)| (*path, None)).collect();
        a_turn(&world, &mut session, &removed);
        fs::remove_dir(world.file("bin")).unwrap();
        let log = log_of(&session);
        for text in ["755", "600", "640", "444"] {
            assert!(
                log.contains(&format!(r#""mode":"{text}""#)),
                "{text}: {log}"
            );
        }
        assert!(!log.contains("4755"), "{log}");

        let done = undo(&mut session);
        assert!(done.not_restored.is_empty(), "{done:?}");
        let umask = umask(&world);
        for (path, bits) in files {
            assert_eq!(fs::read(world.file(path)).unwrap(), path.as_bytes());
            assert_eq!(mode(&world.file(path)), bits & 0o777 & !umask, "{path}");
        }
    }

    /// Over a file that is still there, the mode only narrows: a file a
    /// change left readable by others is private again, and one the person
    /// made private since is not opened up again.
    #[test]
    fn a_file_put_back_over_one_still_there_is_never_left_wider() {
        let world = World::new("put_back_over");
        world.write("private.txt", b"mine");
        set(&world.file("private.txt"), 0o600);
        world.write("open.txt", b"ours");
        set(&world.file("open.txt"), 0o644);
        let mut session = world.create("s_1");
        a_turn(
            &world,
            &mut session,
            &[
                ("private.txt", Some(b"changed")),
                ("open.txt", Some(b"changed")),
            ],
        );
        // What the round left: one opened up, as a replace by a program
        // that does not keep the mode would leave it; the other closed by
        // the person.
        set(&world.file("private.txt"), 0o644);
        set(&world.file("open.txt"), 0o600);

        let done = undo(&mut session);
        assert!(done.not_restored.is_empty(), "{done:?}");
        assert_eq!(fs::read(world.file("private.txt")).unwrap(), b"mine");
        assert_eq!(mode(&world.file("private.txt")), 0o600);
        assert_eq!(fs::read(world.file("open.txt")).unwrap(), b"ours");
        assert_eq!(mode(&world.file("open.txt")), 0o600);
    }

    /// A checkpoint written before versions had a mode: the line is as it
    /// was (no `mode`), it reads back the same, and the file is made again
    /// with the default mode, as it always was.
    #[test]
    fn a_version_from_a_log_written_before_modes_is_made_again_as_before() {
        let world = World::new("put_back_older");
        let mut session = world.create("s_1");
        let turn = TurnId::new("t_1");
        session.append(Some(&turn), user("remove it")).unwrap();
        let content = b"#!/bin/sh\n";
        let hash = session.store().put_blob(content).unwrap();
        let old = FileVersion::new(hash, content.len() as u64, true);
        session
            .append(
                Some(&turn),
                Body::Checkpoint(Checkpoint {
                    files: vec![FileChange {
                        path: "run.sh".into(),
                        before: Before::Version(old),
                        after: None,
                    }],
                }),
            )
            .unwrap();
        let log = log_of(&session);
        let line: serde_json::Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(
            line["files"],
            serde_json::json!([{"path": "run.sh", "before": {"hash": hash, "bytes": 10}, "after": null}]),
            "the line is as a build from before the field wrote it"
        );
        drop(session);
        let (mut session, _) = world.resume("s_1");
        let plan = session.plan_undo().unwrap().unwrap();
        assert_eq!(plan.files[0].restore, Before::Version(old));
        let done = undo(&mut session);
        assert_eq!(done.restored, ["run.sh"]);
        assert_eq!(fs::read(world.file("run.sh")).unwrap(), content);
        assert_eq!(mode(&world.file("run.sh")), 0o666 & !umask(&world));
    }

    /// A file whose way is now a link (a folder swapped for a link to
    /// another folder in the root, or the file itself) is not put back, its
    /// mode or not, and nothing where the link leads is touched.
    #[test]
    fn a_file_reached_through_a_link_is_not_put_back_with_its_mode() {
        let world = World::new("put_back_links");
        world.write("bin/run.sh", b"#!/bin/sh\n");
        set(&world.file("bin/run.sh"), 0o755);
        world.write("private.txt", b"mine");
        set(&world.file("private.txt"), 0o600);
        world.write("elsewhere/run.sh", b"other");
        set(&world.file("elsewhere/run.sh"), 0o644);
        world.write("elsewhere/private.txt", b"other");
        set(&world.file("elsewhere/private.txt"), 0o644);
        let mut session = world.create("s_1");
        a_turn(
            &world,
            &mut session,
            &[("bin/run.sh", None), ("private.txt", None)],
        );
        fs::remove_dir(world.file("bin")).unwrap();
        symlink(world.file("elsewhere"), world.file("bin")).unwrap();
        symlink(
            world.file("elsewhere/private.txt"),
            world.file("private.txt"),
        )
        .unwrap();

        let done = undo(&mut session);
        assert!(done.restored.is_empty(), "{done:?}");
        assert_eq!(done.not_restored.len(), 2, "{done:?}");
        for path in ["elsewhere/run.sh", "elsewhere/private.txt"] {
            assert_eq!(fs::read(world.file(path)).unwrap(), b"other", "{path}");
            assert_eq!(mode(&world.file(path)), 0o644, "{path}");
        }
    }
}

/// On Windows a file kept from everyone but its user (its own access
/// list, cut off from its folder) comes back owner-only; any other comes
/// back with its folder's rules, as a new file does. Runs only on a
/// Windows machine (CI's Windows job).
#[cfg(windows)]
#[test]
fn a_private_file_made_again_on_windows_is_owner_only() {
    use std::io::Write;

    use cineris_base::private::{access_list, create_new};

    let world = World::new("put_back_windows");
    let private = world.file("private.txt");
    create_new(&private).unwrap().write_all(b"mine").unwrap();
    assert!(access_list(&private).unwrap().is_owner_only());
    world.write("plain.txt", b"ours");
    let mut session = world.create("s_1");
    a_turn(
        &world,
        &mut session,
        &[("private.txt", None), ("plain.txt", None)],
    );
    let log = log_of(&session);
    assert_eq!(
        log.matches(r#""mode":"#).count(),
        1,
        "the private file's alone: {log}"
    );
    assert!(log.contains(r#""mode":"600""#), "{log}");

    let done = undo(&mut session);
    assert!(done.not_restored.is_empty(), "{done:?}");
    assert_eq!(fs::read(&private).unwrap(), b"mine");
    let list = access_list(&private).unwrap();
    assert!(list.is_owner_only(), "{list:?}");
    let plain = access_list(&world.file("plain.txt")).unwrap();
    assert!(plain.inherits, "{plain:?}");
    assert!(!plain.is_kept_from_others(), "{plain:?}");
}
