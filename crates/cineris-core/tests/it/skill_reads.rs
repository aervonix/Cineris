//! The folders of the user's own skills outside the workspace may be read
//! unasked (step 28, the owner's call on reading a skill, as the maintainers
//! narrowed it): each such skill found is given to the session as a rule of
//! its own that allows reading its folder, with the source `session`. Reads
//! only; a `deny` still wins; a folder that is a link out of its root has
//! none, nor do the roots `compat.skills` leaves out; a project's skills,
//! and a user's whose folder is in the workspace or holds it, have none, so
//! that the workspace's reads stay as they were, nor has a folder that is
//! the home directory or holds it; the rules follow the skills found at each
//! turn's start, and one the user revoked is not given again. End to end,
//! through the core, on in-memory sessions.

use std::path::Path;

use cineris_protocol::lease::{Effect, RuleAction};
use cineris_protocol::methods::{
    ApprovalScope, Op, OpResult, PermissionsRevokeParams, SessionRef, StoredApproval,
};
use serde_json::json;

use crate::extensions_found::start;
use crate::{Front, World, calls, says};

/// Writes the skill `name` in the folder of skills `root`.
fn skill(root: &Path, name: &str) {
    std::fs::create_dir_all(root.join(name)).unwrap();
    std::fs::write(
        root.join(name).join("SKILL.md"),
        format!("---\nname: {name}\ndescription: Does {name}.\n---\n# {name}\n\nThe steps.\n"),
    )
    .unwrap();
}

/// Writes a file, and the folders it is in.
fn put(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Sets the folder `dir`'s modification time to a day after the epoch: an
/// entry made in it is then a change at once, whatever the step of the file
/// system's clock, which a test does not wait for.
fn age_folder(dir: &Path) {
    let day = std::time::UNIX_EPOCH + std::time::Duration::from_secs(86_400);
    #[cfg(unix)]
    let folder = std::fs::File::open(dir).unwrap();
    #[cfg(windows)]
    let folder = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            // FILE_WRITE_ATTRIBUTES, and FILE_FLAG_BACKUP_SEMANTICS, without
            // which a folder is not opened.
            .access_mode(0x100)
            .custom_flags(0x0200_0000)
            .open(dir)
            .unwrap()
    };
    folder.set_modified(day).unwrap();
}

/// A path as the model writes it in a call.
fn text(path: &Path) -> String {
    path.to_str().unwrap().to_owned()
}

/// A call of the read tool for `path`.
fn read(id: &str, path: &Path) -> (String, &'static str, serde_json::Value) {
    (id.to_owned(), "read", json!({"path": text(path)}))
}

/// The model's response that makes `reads`.
fn reading(reads: &[(String, &'static str, serde_json::Value)]) -> cineris_testkit::Response {
    let reads: Vec<(&str, &str, serde_json::Value)> = reads
        .iter()
        .map(|(id, tool, input)| (id.as_str(), *tool, input.clone()))
        .collect();
    calls(&reads)
}

/// The outcome of the one result so far whose text holds `words`, as
/// results finish in their own order.
fn outcome_with(front: &Front, words: &str) -> String {
    let results = front.results();
    let found: Vec<_> = results
        .iter()
        .filter(|result| result.2.contains(words))
        .collect();
    assert_eq!(found.len(), 1, "{words}: {results:#?}");
    found[0].1.to_string()
}

/// The session's approvals, as `permissions/list` gives them.
async fn approvals(front: &Front) -> Vec<StoredApproval> {
    let listed = front
        .op(Op::PermissionsList(SessionRef {
            session_id: front.session.clone(),
        }))
        .await
        .unwrap();
    let OpResult::PermissionsList(listed) = listed else {
        panic!("not approvals");
    };
    listed.approvals
}

/// The one rule of the session's, which must be a skill's: a session rule
/// that allows reading below a folder.
async fn the_rule(front: &Front) -> String {
    let approvals = approvals(front).await;
    assert_eq!(approvals.len(), 1, "{approvals:#?}");
    let approval = &approvals[0];
    assert_eq!(approval.scope, ApprovalScope::Session);
    assert_eq!(approval.rule.effect, Effect::Read);
    assert_eq!(approval.rule.action, RuleAction::Allow);
    approval.rule.pattern.clone()
}

/// Where a folder really is, as the session's resolver gives it: the
/// file system's own spelling.
fn real(folder: &Path) -> String {
    let real = std::fs::canonicalize(folder).unwrap();
    cineris_base::AbsPath::from_os(&real)
        .unwrap()
        .as_str()
        .to_owned()
}

/// Makes a link to the folder `target` at `link`. On Windows that needs a
/// privilege an ordinary account lacks unless Developer Mode is on; the CI
/// runner has it, and there the link must be made. Elsewhere `false` when
/// it could not be, and the test checks the rest.
fn link_folder(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(target, link);
    match made {
        Ok(()) => true,
        Err(_) if std::env::var_os("GITHUB_ACTIONS").is_none() => false,
        Err(error) => panic!("the CI runner cannot make a link: {error}"),
    }
}

/// Points the link to a folder at `link` to `target` instead.
fn relink(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::fs::remove_file(link).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(link).unwrap();
    assert!(link_folder(target, link), "the link was made before");
}

/// Takes back the session's approval `id`, as the user does.
async fn revoke(front: &Front, id: cineris_protocol::ids::ApprovalId) {
    front
        .op(Op::PermissionsRevoke(PermissionsRevokeParams {
            session_id: front.session.clone(),
            ids: vec![id],
        }))
        .await
        .unwrap();
}

/// A user's skill, outside the workspace, in one session: its files are
/// read unasked, and one a `deny` rule names is refused; a write there is
/// asked as before; its root's own files and a folder beside it whose name
/// begins as its does are not covered. Another skill added beside it gains
/// a rule at the next turn's start, and it keeps its own; its `SKILL.md`
/// removed, the turn's start after takes its rule back. The user revokes the
/// other's rule, and it is not given again, though its skill drops out of a
/// finding and is found again (an editor that writes `SKILL.md` anew).
#[test]
fn a_users_skill_folder_is_read_unasked_and_its_rule_follows_the_skills_found() {
    let world = World::new("skill_reads_story", []);
    world.user_config(
        "[[permissions.rules]]\neffect = \"read\"\nmatch = \"~/.claude/skills/review/private.md\"\n\
         action = \"deny\"\n",
    );
    let root = world.home.join(".claude/skills");
    skill(&root, "review");
    let folder = root.join("review");
    put(&folder.join("notes.md"), "Notes on reviewing.\n");
    put(&folder.join("private.md"), "Kept back.\n");
    put(&root.join("notes.md"), "Beside the skills.\n");
    put(
        &root.join("reviewer/notes.md"),
        "Beside, in a folder that is no skill.\n",
    );
    world.provider.push(reading(&[
        read("c1", &folder.join("SKILL.md")),
        read("c2", &folder.join("notes.md")),
        read("c3", &folder.join("private.md")),
    ]));
    world.provider.push(says("Read."));
    world.provider.push(calls(&[(
        "c4",
        "write",
        json!({"path": text(&folder.join("notes.md")), "content": "Changed.\n"}),
    )]));
    world.provider.push(says("Asked."));
    world.provider.push(reading(&[
        read("c5", &root.join("notes.md")),
        read("c6", &root.join("reviewer/notes.md")),
    ]));
    world.provider.push(says("Asked."));
    world
        .provider
        .push(reading(&[read("c7", &root.join("pdf/SKILL.md"))]));
    world.provider.push(says("Read."));
    world
        .provider
        .push(reading(&[read("c8", &folder.join("notes.md"))]));
    world.provider.push(says("Asked."));
    world
        .provider
        .push(reading(&[read("c9", &root.join("pdf/notes.md"))]));
    world.provider.push(says("Asked."));
    world
        .provider
        .push(reading(&[read("c10", &root.join("pdf/SKILL.md"))]));
    world.provider.push(says("Asked."));
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        // The rule is in force before the first turn's calls: none of them
        // is asked about (the rig fails a turn at once on a question nobody
        // answers), and the one a `deny` names is refused.
        front.turn("read the skill").await;
        assert_eq!(front.results().len(), 3);
        assert_eq!(outcome_with(&front, "The steps."), "success");
        assert_eq!(outcome_with(&front, "Notes on reviewing."), "success");
        assert_eq!(outcome_with(&front, "private.md"), "denied");
        assert!(
            !front
                .results()
                .iter()
                .any(|result| result.2.contains("Kept back.")),
            "the denied file was read"
        );
        assert_eq!(the_rule(&front).await, format!("{}/**", real(&folder)));
        // Reads only: a write there is asked about, as it was.
        assert_eq!(front.turn_denying("write there").await, ["write"]);
        assert_eq!(
            std::fs::read_to_string(folder.join("notes.md")).unwrap(),
            "Notes on reviewing.\n"
        );
        // Beside it: the root's own file, and a folder whose name begins
        // with the skill's.
        assert_eq!(front.turn_denying("read beside").await, ["read", "read"]);
        // Another skill comes: at the next turn's start it gains its rule,
        // read unasked, and the first keeps its own.
        skill(&root, "pdf");
        age_folder(&root);
        front.turn("read the new one").await;
        assert_eq!(outcome_with(&front, "# pdf"), "success");
        let mut rules: Vec<String> = approvals(&front)
            .await
            .into_iter()
            .map(|approval| approval.rule.pattern)
            .collect();
        rules.sort();
        let mut both = [
            format!("{}/**", real(&folder)),
            format!("{}/**", real(&root.join("pdf"))),
        ];
        both.sort();
        assert_eq!(rules, both);
        // The first goes (its folder stays, with no `SKILL.md`): at the next
        // turn's start its rule is taken back.
        std::fs::remove_file(folder.join("SKILL.md")).unwrap();
        assert_eq!(front.turn_denying("read again").await, ["read"]);
        assert_eq!(
            the_rule(&front).await,
            format!("{}/**", real(&root.join("pdf")))
        );
        // The user revokes the other's rule. Its `SKILL.md` goes and comes
        // back: it is not given a rule again.
        let pdf = approvals(&front).await.remove(0).id;
        revoke(&front, pdf).await;
        put(&root.join("pdf/notes.md"), "Notes on forms.\n");
        let written = std::fs::read_to_string(root.join("pdf/SKILL.md")).unwrap();
        std::fs::remove_file(root.join("pdf/SKILL.md")).unwrap();
        assert_eq!(front.turn_denying("read its notes").await, ["read"]);
        std::fs::write(root.join("pdf/SKILL.md"), written).unwrap();
        age_folder(&root.join("pdf"));
        assert_eq!(front.turn_denying("read it again").await, ["read"]);
        let approvals = approvals(&front).await;
        assert!(approvals.is_empty(), "{approvals:#?}");
        front.close().await;
    });
}

/// Only the user's skills found have a rule: none for a skill's folder that
/// is a link out of its root, read where it leads or through the link, nor
/// for a skill in `.claude/skills/` while `compat.skills` is off. A folder
/// whose name holds what a path glob reads as a pattern (`notes[1]`) is
/// covered by its rule as it is named.
#[test]
fn a_users_skill_that_is_not_found_has_no_rule() {
    let world = World::new("skill_reads_not_found", []);
    world.user_config("[compat]\nskills = false\n");
    let mine = world.home.join(".cineris/skills");
    skill(&mine, "kept");
    skill(&mine, "notes[1]");
    skill(&world.home.join(".claude/skills"), "other");
    let outside = world.dir.join("outside");
    skill(&outside, "linked");
    let linked = link_folder(&outside.join("linked"), &mine.join("linked"));
    let mut asked: Vec<(String, &'static str, serde_json::Value)> = vec![read(
        "c2",
        &world.home.join(".claude/skills/other/SKILL.md"),
    )];
    if linked {
        asked.push(read("c3", &outside.join("linked/SKILL.md")));
        asked.push(read("c4", &mine.join("linked/SKILL.md")));
    }
    world.provider.push(reading(&[
        read("c1", &mine.join("kept/SKILL.md")),
        read("c5", &mine.join("notes[1]/SKILL.md")),
    ]));
    world.provider.push(says("Read."));
    world.provider.push(reading(&asked));
    world.provider.push(says("Asked."));
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("read the ones found").await;
        assert_eq!(outcome_with(&front, "# kept"), "success");
        assert_eq!(outcome_with(&front, "# notes[1]"), "success");
        let questions = front.turn_denying("read the others").await;
        assert_eq!(questions, vec!["read"; asked.len()]);
        // The user's two skills that were found.
        let rules: Vec<String> = approvals(&front)
            .await
            .into_iter()
            .map(|approval| approval.rule.pattern)
            .collect();
        assert_eq!(rules.len(), 2, "{rules:#?}");
        let kept = format!("{}/**", real(&mine.join("kept")));
        assert!(rules.contains(&kept), "{kept}: {rules:#?}");
        front.close().await;
    });
}

/// The skills in the workspace give no rule, so the workspace's reads, and
/// its look for files that look like secrets, stay as they were: a
/// project's skill, not trusted, whose `SKILL.md` is read unasked and whose
/// `.env` is asked about, as any file of the workspace's is; and a user's
/// whose root is a link pointed into the workspace during the session, so
/// that its folder really is there from then on: the rule it had outside is
/// taken back at the next turn's start, and its files are asked about read
/// through the link or where it leads.
#[test]
fn the_skills_in_the_workspace_have_no_rule() {
    let world = World::new("skill_reads_in_the_workspace", []);
    let theirs = world.workspace.join(".cineris/skills");
    skill(&theirs, "theirs");
    put(&theirs.join("theirs/.env"), "TOKEN=project\n");
    let away = world.dir.join("away/skills");
    skill(&away, "team");
    let shared = world.workspace.join("shared/skills");
    skill(&shared, "team");
    put(&shared.join("team/.env"), "TOKEN=team\n");
    std::fs::create_dir_all(world.home.join(".claude")).unwrap();
    let users = world.home.join(".claude/skills");
    let linked = link_folder(&away, &users);
    let mut first = vec![read("c1", &theirs.join("theirs/SKILL.md"))];
    let mut asked = vec![read("c2", &theirs.join("theirs/.env"))];
    if linked {
        first.push(read("c5", &users.join("team/SKILL.md")));
        asked.push(read("c3", &users.join("team/.env")));
        asked.push(read("c4", &shared.join("team/.env")));
    }
    world.provider.push(reading(&first));
    world.provider.push(says("Read."));
    world.provider.push(reading(&asked));
    world.provider.push(says("Asked."));
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("read the skills").await;
        assert_eq!(outcome_with(&front, "# theirs"), "success");
        if linked {
            // Outside the workspace, the user's skill has its rule.
            assert_eq!(outcome_with(&front, "# team"), "success");
            assert_eq!(
                the_rule(&front).await,
                format!("{}/**", real(&away.join("team")))
            );
            relink(&shared, &users);
        }
        let questions = front.turn_denying("read the secrets beside them").await;
        assert_eq!(questions, vec!["read"; asked.len()]);
        let approvals = approvals(&front).await;
        assert!(approvals.is_empty(), "{approvals:#?}");
        front.close().await;
    });
    // Both were found: the prompt lists them.
    let system = &world.provider.requests()[0].system;
    assert!(system.contains("\n- theirs: "), "{system}");
    if linked {
        assert!(system.contains("\n- team: "), "{system}");
    }
}

/// A user's skill whose folder holds the workspace (its author working in
/// its own `scripts/`) gives no rule, which would cover the whole
/// workspace: its files are asked about as before, and the workspace's
/// `.env` is asked about as a secret; a skill beside it has its rule.
#[test]
fn a_skill_folder_that_holds_the_workspace_has_no_rule() {
    let mut world = World::new("skill_reads_holding_the_workspace", []);
    let root = world.home.join(".claude/skills");
    skill(&root, "pdf");
    skill(&root, "review");
    world.workspace = root.join("pdf/scripts");
    put(&world.workspace.join(".env"), "TOKEN=here\n");
    world
        .provider
        .push(reading(&[read("c1", &root.join("review/SKILL.md"))]));
    world.provider.push(says("Read."));
    world.provider.push(reading(&[
        read("c2", &root.join("pdf/SKILL.md")),
        read("c3", &world.workspace.join(".env")),
    ]));
    world.provider.push(says("Asked."));
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("read the one beside").await;
        assert_eq!(outcome_with(&front, "# review"), "success");
        assert_eq!(
            front.turn_denying("read the one around").await,
            ["read", "read"]
        );
        assert_eq!(
            the_rule(&front).await,
            format!("{}/**", real(&root.join("review")))
        );
        front.close().await;
    });
    let system = &world.provider.requests()[0].system;
    assert!(system.contains("\n- pdf: "), "found: {system}");
}

/// A skill whose folder is the home directory gives no rule, which would
/// be one for all of it: a user's root that is a link to the folder the
/// home directory is in, which holds a `SKILL.md`.
#[test]
fn a_skill_folder_that_is_the_home_directory_has_no_rule() {
    let world = World::new("skill_reads_the_home", []);
    put(
        &world.home.join("SKILL.md"),
        "---\nname: everything\ndescription: All of it.\n---\n",
    );
    put(&world.home.join("notes.txt"), "Private notes.\n");
    std::fs::create_dir_all(world.home.join(".cineris")).unwrap();
    if !link_folder(&world.dir, &world.home.join(".cineris/skills")) {
        return;
    }
    world
        .provider
        .push(reading(&[read("c1", &world.home.join("notes.txt"))]));
    world.provider.push(says("Asked."));
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        assert_eq!(front.turn_denying("read the notes").await, ["read"]);
        let approvals = approvals(&front).await;
        assert!(approvals.is_empty(), "{approvals:#?}");
        front.close().await;
    });
    let system = &world.provider.requests()[0].system;
    assert!(system.contains("\n- everything: "), "found: {system}");
}

/// On Windows the home directory may be given with `/` and in another
/// case than the file system's: the skills found in it are judged by
/// where they are. The rule is written in the file system's own spelling,
/// which an allow is matched in, and a read written either way is allowed
/// unasked.
#[cfg(windows)]
#[test]
fn on_windows_a_home_written_otherwise_is_judged_by_place() {
    let mut world = World::new("skill_reads_windows_spelling", []);
    let written: std::path::PathBuf = world
        .home
        .to_str()
        .unwrap()
        .to_lowercase()
        .replace('\\', "/")
        .into();
    assert_ne!(text(&written), text(&world.home));
    world.env = world.env.clone().with("USERPROFILE", text(&written));
    let folder = world.home.join(".claude/skills/review");
    skill(&world.home.join(".claude/skills"), "review");
    put(&folder.join("notes.md"), "Notes on reviewing.\n");
    world.provider.push(reading(&[
        read("c1", &folder.join("SKILL.md")),
        read("c2", &written.join(".claude/skills/review/notes.md")),
    ]));
    world.provider.push(says("Read."));
    let core = world.core();
    world.run(async {
        let front = start(&world, &core).await;
        front.turn("read the skill both ways").await;
        assert_eq!(front.results().len(), 2);
        assert_eq!(outcome_with(&front, "# review"), "success");
        assert_eq!(outcome_with(&front, "Notes on reviewing."), "success");
        assert_eq!(the_rule(&front).await, format!("{}/**", real(&folder)));
        front.close().await;
    });
}
