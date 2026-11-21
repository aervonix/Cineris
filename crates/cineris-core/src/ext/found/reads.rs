//! The session's rules for reading the user's own skills' folders (step 28,
//! the owner's call on reading a skill, as the maintainers narrowed it).
//!
//! **Which folders.** A skill of the home directory's roots, whose folder
//! and the workspace are apart, neither within the other, and whose folder
//! is not the home directory or a folder that holds it, judged by place
//! (`cineris_base::location_within`), never by text ([`readable`]). So a
//! project's skills give none, and nor does a user's whose folder is in the
//! workspace (its root a link there, or the workspace the home directory)
//! or holds it (a skill's author working in its own `scripts/`): the
//! workspace's reads, and its look for secrets, stay as they were. A folder
//! that is the home directory or holds it would be a rule for all of it.
//!
//! **The rules follow the skills found** ([`SkillReads::follow`]): one
//! found again keeps its rule, one gone or moved where it may not have one
//! loses it, and one new gains one. One the user revoked is never given
//! again in the session, though its folder drops out of a finding and is
//! found again (a checkout, an editor that writes `SKILL.md` anew).

use std::path::Path;

use cineris_base::{AbsPath, location_within, same_location};
use cineris_ext::skills::{Owner, Skill};
use cineris_protocol::ids::ApprovalId;

use crate::policy::EnginePolicy;

/// The folders of `skills` that may be read unasked, in their order: the
/// user's own skills', each folder apart from `workspace` (neither within
/// the other), and neither `home` nor a folder that holds it. `workspace`
/// and `home` are where they really are, as each skill's folder is.
pub(super) fn readable(skills: &[Skill], workspace: &Path, home: Option<&Path>) -> Vec<AbsPath> {
    skills
        .iter()
        .filter(|skill| skill.owner == Owner::User)
        .filter(|skill| {
            let folder = skill.folder.to_fs_path();
            !location_within(&folder, workspace)
                && !location_within(workspace, &folder)
                && home.is_none_or(|home| !location_within(home, &folder))
        })
        .map(|skill| skill.folder.clone())
        .collect()
}

/// The session's rules for reading its skills' folders, as it holds them.
#[derive(Debug, Default)]
pub(super) struct SkillReads {
    /// Each folder given a rule, with the id of the session's approval
    /// that allows reading it, when the skills made it (one an approval
    /// held already is not theirs to take back).
    held: Vec<(AbsPath, Option<ApprovalId>)>,
    /// The folders whose rule the user revoked: given none again in the
    /// session.
    revoked: Vec<AbsPath>,
    /// Whether the user was told that the skills made rules ([`told`]).
    told: bool,
}

/// What the user is told, once a session, when the skills' first rules are
/// made, so that they are not taken for approvals the user gave: `count`
/// folders may be read unasked, why, and where they are.
pub(super) fn told(count: usize) -> String {
    format!(
        "{count} of your skill folders may be read without asking, because \
         `permissions.read_skill_folders` is on. Each is an approval of this session's, which \
         /permissions lists and can revoke."
    )
}

impl SkillReads {
    /// Follows `folders`, the folders that may be read unasked as the
    /// skills are found now ([`readable`]): the rule of each folder held
    /// and not among them is taken back, and each of them that has none,
    /// and was not revoked, is given one, through `policy` in one change of
    /// the engine in force ([`EnginePolicy::follow_reads`]). First, each
    /// rule of the skills' that the engine no longer holds, which the user
    /// revoked, has its folder kept as revoked. Folders are told apart by
    /// where they are (`cineris_base::same_location`). What it makes is
    /// held before it returns. The first time in the session that a rule
    /// is held, it says how many are, for the user to be told ([`told`]).
    /// Blocks: call this off the core's thread.
    pub(super) fn follow(&mut self, policy: &EnginePolicy, folders: Vec<AbsPath>) -> Option<usize> {
        let same =
            |one: &AbsPath, other: &AbsPath| same_location(&one.to_fs_path(), &other.to_fs_path());
        let ours: Vec<ApprovalId> = self.held.iter().filter_map(|(_, id)| id.clone()).collect();
        let taken = policy.not_held(&ours);
        if !taken.is_empty() {
            let (revoked, held): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held)
                .into_iter()
                .partition(|(_, id)| id.as_ref().is_some_and(|id| taken.contains(id)));
            self.held = held;
            self.revoked
                .extend(revoked.into_iter().map(|(folder, _)| folder));
        }
        let (kept, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.held)
            .into_iter()
            .partition(|(held, _)| folders.iter().any(|folder| same(folder, held)));
        self.held = kept;
        let mut added: Vec<AbsPath> = Vec::new();
        for folder in folders {
            let known = self.held.iter().any(|(held, _)| same(held, &folder))
                || self.revoked.iter().any(|revoked| same(revoked, &folder))
                || added.iter().any(|new| same(new, &folder));
            if !known {
                added.push(folder);
            }
        }
        if gone.is_empty() && added.is_empty() {
            return None;
        }
        let take_back: Vec<ApprovalId> = gone.into_iter().filter_map(|(_, id)| id).collect();
        let mut given = Vec::new();
        let mut rules = Vec::new();
        for folder in added {
            if let Some(rule) = crate::policy::reading_below(&folder) {
                rules.push(rule);
                given.push(folder);
            }
        }
        let ids = policy.follow_reads(&rules, &take_back);
        self.held.extend(given.into_iter().zip(ids));
        let made = self.held.iter().filter(|(_, id)| id.is_some()).count();
        if self.told || made == 0 {
            return None;
        }
        self.told = true;
        Some(made)
    }
}

#[cfg(test)]
mod tests {
    use cineris_ext::skills::Layout;

    use super::*;

    fn skill(owner: Owner, folder: &Path) -> Skill {
        Skill {
            name: "s".to_owned(),
            description: String::new(),
            path: "s/SKILL.md".to_owned(),
            owner,
            layout: Layout::Claude,
            folder: AbsPath::from_os(folder).unwrap(),
        }
    }

    /// Only a user's skill whose folder and the workspace are apart, and
    /// that is not the home directory or above it, may be read unasked: not
    /// a project's, not one in the workspace, not one that holds it (its
    /// author working in its own `scripts/`), not the home directory or a
    /// folder that holds it. Judged by place: the same folder written
    /// another way is the same.
    #[test]
    fn a_folder_may_be_read_unasked_only_apart_from_the_workspace_and_below_the_home() {
        // Places that need not be there: only their names are compared.
        let base = std::path::PathBuf::from(if cfg!(windows) { r"C:\r" } else { "/r" });
        let home = base.join("home");
        let workspace = base.join("work").join("app");
        let apart = home.join(".claude/skills/review");
        let cases = [
            (Owner::User, apart.clone(), true),
            (Owner::Project, apart.clone(), false),
            (Owner::User, workspace.join(".cineris/skills/pdf"), false),
            (Owner::User, workspace.clone(), false),
            (Owner::User, base.join("work"), false),
            (Owner::User, home.clone(), false),
            (Owner::User, base.clone(), false),
            (Owner::User, base.join("elsewhere"), true),
        ];
        for (owner, folder, wanted) in cases {
            let found = readable(&[skill(owner, &folder)], &workspace, Some(&home));
            assert_eq!(found.len(), usize::from(wanted), "{owner:?} {folder:?}");
        }
        // With no home directory known, the workspace alone decides.
        assert_eq!(
            readable(&[skill(Owner::User, &home)], &workspace, None).len(),
            1
        );
        // A skill's author working in its `scripts/`: the folder holds the
        // workspace.
        let scripts = apart.join("scripts");
        assert!(readable(&[skill(Owner::User, &apart)], &scripts, Some(&home)).is_empty());
    }

    /// On Windows the workspace written with `/` and in another case is
    /// the same place.
    #[cfg(windows)]
    #[test]
    fn on_windows_a_workspace_written_otherwise_is_the_same_place() {
        let workspace = std::path::PathBuf::from(r"C:\r\App");
        let written: std::path::PathBuf = workspace
            .to_str()
            .unwrap()
            .to_lowercase()
            .replace('\\', "/")
            .into();
        let inside = skill(Owner::User, &workspace.join("skills").join("pdf"));
        assert!(readable(std::slice::from_ref(&inside), &written, None).is_empty());
        let holding = skill(Owner::User, &workspace);
        assert!(readable(&[holding], &written.join("scripts"), None).is_empty());
    }
}
