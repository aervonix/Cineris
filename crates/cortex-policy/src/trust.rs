//! Project trust, pinned by hash
//! (`docs/architecture/06-permissions-and-sandbox.md`, "Project trust").
//!
//! A repository is not trusted until the user says so, and then only for
//! the definitions they saw. What needs trust is whatever in a project can
//! run code or grant permissions: its rules, hooks, extensions, MCP
//! servers, and the learned or shared commands used for the safe list.
//!
//! **Trust is pinned.** When the user trusts a project, the hash of each
//! definition shown to them is stored with the trust. Afterwards a
//! definition stands in one of four ways ([`Standing`]):
//!
//! | Standing | Meaning | Used? |
//! |---|---|---|
//! | `Approved` | Its hash is the one the user approved. | Yes. |
//! | `Changed` | It was approved, and its content has changed since. | No, until the user approves the change. |
//! | `New` | The project is trusted, and this definition was not there to see. | No, until approved. |
//! | `Untrusted` | The project is not trusted. | No. |
//!
//! That closes "approved once, changed later": a hook that was harmless
//! when the user looked is not run after someone edits it.
//!
//! # The stored file
//!
//! `trust.toml` in the state directory:
//!
//! ```toml
//! version = 2
//!
//! [[project]]
//! key = "9f2c4a1b7d3e…"
//! root = "~/work/app"
//! trusted_at = "2026-11-13T10:15:02Z"
//! git = "trusted"
//! declined_rules = "0b7c…"
//!
//! [[project.pinned]]
//! kind = "hook"
//! name = "format"
//! hash = "5e884898da28…"
//! ```
//!
//! - **Projects are known by their key** (`AbsPath::project_key`), which is
//!   made from the resolved root in its stored case, so a folder that
//!   differs from a trusted one only in case is another project. `root` is
//!   for showing a person which project an entry is, and is never compared.
//! - **Written owner-only, atomically, on every write,** under a lock held
//!   from the read to the write, so that two sessions that change it at
//!   once both keep their change: a `revoke` is never undone by a `trust`
//!   that read the file before it (`crate::store_lock`).
//! - **Read strictly.** A key this build does not know, or a newer
//!   `version`, refuses the whole file, and then **nothing is trusted**: a
//!   file that cannot be read in full must not be read in part.
//! - **A project the user will never trust** (since step 23, the owner's
//!   answer of 2026-11-15) is a `[[refused]]` table with its key, root and
//!   time: the interactive application does not ask about it at later
//!   starts. A project is never both: trusting it clears the refusal, and
//!   refusing it takes its trust away. A build from before step 23 does not
//!   know the table, and so reads such a file as trusting nothing, which
//!   fails safe.
//! - **Whether git was shown is pinned** (version 2, since the follow-ups
//!   of step 23, the owner's decision of 2026-11-16): `git = "trusted"`
//!   when the person trusted the project, or approved what was new in it,
//!   with git in the list, which it is in a repository; `git = "declined"`
//!   when they chose to keep trusting it without git. Absent, git was never
//!   shown, and a trusted folder that has since become a repository is
//!   asked about again, git new. Until git is trusted it does not read the
//!   repository's configuration ([`ProjectTrust::git_trusted`]).
//! - **Rules a person kept trusting the project without** (version 2):
//!   `declined_rules`, the hash of the project's rules as they were when the
//!   person answered "keep trusting it, without the new rules". They stay
//!   held back, and are not asked about again while they are those rules;
//!   a further change asks again. Approving rules clears it.
//! - **Versions.** This build writes version 2 and reads 1 and 2. A
//!   version 1 file is read as it always meant, every project with git
//!   never shown, and is written as version 2 at its next change. A build
//!   that reads only version 1 refuses a version 2 file whole, by its
//!   version (decision 0028's rule: a file a build cannot read in full is
//!   not read in part), and so trusts nothing until it is updated.

#![allow(
    clippy::disallowed_methods,
    reason = "blocking file calls; the store is used at start-up and on the blocking pool"
)]

use std::io;
use std::path::{Path, PathBuf};

use cortex_base::{
    AtomicWriteError, AtomicWriteOptions, ContentHash, ContentHasher, SharedClock, write_atomic,
};
use cortex_protocol::ids::{ProjectKey, Timestamp};
use cortex_protocol::lease::Rule;
use cortex_protocol::methods::TrustResourceKind;
use serde::{Deserialize, Serialize};

use crate::store_lock;

/// The version of the file's format this build writes.
const VERSION: u32 = 2;

/// The oldest version this build reads: version 1, which has no record of
/// git or of declined rules.
const OLDEST: u32 = 1;

/// The largest file read.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// One thing in a project that can run code or grant permissions, as it is
/// now: what kind of thing, its name, and the hash of its definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Definition {
    /// What kind of thing it is.
    pub kind: TrustResourceKind,
    /// Its name, unique within its kind: a hook's or a server's configured
    /// name.
    pub name: String,
    /// The hash of everything that decides what it does.
    pub hash: ContentHash,
}

impl Definition {
    /// A definition whose content is these parts, in this order: a hook's
    /// command and each argument; an extension's manifest and its script;
    /// an MCP server's command, arguments and each tool definition.
    ///
    /// The parts are hashed with their lengths, so that moving a character
    /// from one part to the next is a different definition: `["rm", "-rf
    /// x"]` is not `["rm -rf", "x"]`.
    #[must_use]
    pub fn of<'a>(
        kind: TrustResourceKind,
        name: impl Into<String>,
        parts: impl IntoIterator<Item = &'a str>,
    ) -> Self {
        let mut hasher = ContentHasher::new();
        for part in parts {
            hasher.update(&(part.len() as u64).to_le_bytes());
            hasher.update(part.as_bytes());
        }
        Self {
            kind,
            name: name.into(),
            hash: hasher.finish(),
        }
    }

    /// The definition of a project's rules, taken together: all of them,
    /// the ones that deny and ask as well as the ones that allow. The order
    /// of the rules does not matter to what they mean, so it does not
    /// matter to the hash, and neither does a rule written twice.
    ///
    /// The rules that deny and ask take effect without trust, and are
    /// pinned all the same: what the user trusted was the allow rules *with*
    /// what held them back. A project that drops `deny exec make deploy`
    /// beside `allow exec make *` has loosened, and must be approved again
    /// (found by the review of step 12).
    #[must_use]
    pub fn of_rules(rules: &[Rule]) -> Self {
        let mut lines: Vec<String> = rules
            .iter()
            .map(|rule| {
                format!(
                    "{}\u{0}{}\u{0}{}",
                    rule.action.as_str(),
                    rule.effect.as_str(),
                    rule.pattern
                )
            })
            .collect();
        lines.sort();
        lines.dedup();
        Self::of(
            TrustResourceKind::Rules,
            "permissions.rules",
            lines.iter().map(String::as_str),
        )
    }

    /// The definition of a project's rules as trust pins them (since step
    /// 23, the owner's answer of 2026-11-15): those its files hold outside
    /// any profile, and each profile's own, with the profile's name, so
    /// that the pin is the same whichever profile is selected and a change
    /// to any of them is a change. A project whose files define no profile
    /// has the definition [`of_rules`](Self::of_rules) gives its rules.
    #[must_use]
    pub fn of_project_rules(rules: &ProjectRules) -> Self {
        let line = |rule: &Rule| {
            format!(
                "{}\u{0}{}\u{0}{}",
                rule.action.as_str(),
                rule.effect.as_str(),
                rule.pattern
            )
        };
        let mut lines: Vec<String> = rules.base.iter().map(line).collect();
        for (profile, own) in &rules.profiles {
            // A profile's line has a field more than the base's: neither
            // can be read as the other.
            lines.extend(
                own.iter()
                    .map(|rule| format!("{profile}\u{0}{}", line(rule))),
            );
        }
        lines.sort();
        lines.dedup();
        Self::of(
            TrustResourceKind::Rules,
            "permissions.rules",
            lines.iter().map(String::as_str),
        )
    }
}

/// A project's permission rules as trust takes them (since step 23): those
/// its files hold outside any profile, and those each profile its files
/// define adds, by the profile's name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectRules {
    /// Outside any profile.
    pub base: Vec<Rule>,
    /// Each profile's own, by name, in order.
    pub profiles: Vec<(String, Vec<Rule>)>,
}

impl ProjectRules {
    /// Whether the project has no rule at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.base.is_empty() && self.profiles.iter().all(|(_, own)| own.is_empty())
    }

    /// Every rule with the profile it is in (`None`: outside any).
    pub fn each(&self) -> impl Iterator<Item = (Option<&str>, &Rule)> {
        self.base.iter().map(|rule| (None, rule)).chain(
            self.profiles.iter().flat_map(|(profile, own)| {
                own.iter().map(move |rule| (Some(profile.as_str()), rule))
            }),
        )
    }
}

/// How a definition stands with the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Standing {
    /// The user approved exactly this.
    Approved,
    /// The user approved it, and it has changed since. Disabled until they
    /// approve the change.
    Changed,
    /// The project is trusted, and this was not among what the user saw.
    New,
    /// The project is not trusted.
    Untrusted,
}

impl Standing {
    /// Whether the definition may be used.
    #[must_use]
    pub fn is_approved(self) -> bool {
        self == Self::Approved
    }
}

/// Why the trust store could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum TrustError {
    /// The file could not be read.
    #[error("the trust store `{path}` could not be read, so no project is trusted: {source}")]
    Read {
        /// The file.
        path: String,
        /// Why.
        source: io::Error,
    },
    /// The file is not one this build understands in full. Nothing in it
    /// is trusted, and it is left as it is.
    #[error(
        "the trust store `{path}` is not understood, so no project is trusted: {problem}. The \
         file was left as it is"
    )]
    NotUnderstood {
        /// The file.
        path: String,
        /// What is wrong with it.
        problem: String,
    },
    /// The file could not be written.
    #[error(transparent)]
    Write(#[from] AtomicWriteError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    kind: String,
    name: String,
    hash: ContentHash,
}

/// What a person said of git in a trusted project (version 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum GitAnswer {
    /// Git was shown and trusted.
    Trusted,
    /// Git was shown, and the person kept trusting the project without it.
    Declined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Project {
    key: ProjectKey,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    root: String,
    trusted_at: Timestamp,
    /// Absent: git was never shown (and every project of a version 1
    /// file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git: Option<GitAnswer>,
    /// The project's rules as they were when the person kept trusting it
    /// without them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    declined_rules: Option<ContentHash>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pinned: Vec<Pin>,
}

/// A project the user said never to trust.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Refused {
    key: ProjectKey,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    root: String,
    refused_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    version: u32,
    #[serde(default, rename = "project", skip_serializing_if = "Vec::is_empty")]
    projects: Vec<Project>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    refused: Vec<Refused>,
}

/// What the user has trusted of one project, read once: a value to ask
/// about each definition as it is met.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectTrust {
    project: Option<Project>,
    refused: bool,
    /// The file read was of an older version, which kept no record of git.
    older: bool,
}

impl ProjectTrust {
    /// A project nobody has trusted.
    #[must_use]
    pub fn untrusted() -> Self {
        Self::default()
    }

    /// Whether the user has trusted the project at all.
    #[must_use]
    pub fn is_trusted(&self) -> bool {
        self.project.is_some()
    }

    /// Whether the user said never to trust the project: it is not asked
    /// about (since step 23).
    #[must_use]
    pub fn is_refused(&self) -> bool {
        self.refused
    }

    /// Whether git may read the repository's own configuration: the
    /// project is trusted, and git was shown and trusted (version 2 of the
    /// store). A project trusted before git was pinned, or while it was
    /// not a repository, has not had git shown.
    #[must_use]
    pub fn git_trusted(&self) -> bool {
        self.project
            .as_ref()
            .is_some_and(|project| project.git == Some(GitAnswer::Trusted))
    }

    /// Whether the person kept trusting the project without git: it is not
    /// asked about again.
    #[must_use]
    pub fn git_declined(&self) -> bool {
        self.project
            .as_ref()
            .is_some_and(|project| project.git == Some(GitAnswer::Declined))
    }

    /// Whether the person kept trusting the project without exactly these
    /// rules ([`Definition::of_project_rules`]): they stay held back, and
    /// are not asked about again.
    #[must_use]
    pub fn rules_declined(&self, rules: &ProjectRules) -> bool {
        let definition = Definition::of_project_rules(rules);
        self.project
            .as_ref()
            .is_some_and(|project| project.declined_rules == Some(definition.hash))
    }

    /// Whether the store was of an older version, which kept no record of
    /// whether git was shown: a session says why git is asked about again.
    #[must_use]
    pub fn from_older_store(&self) -> bool {
        self.older
    }

    /// How a definition stands.
    #[must_use]
    pub fn standing(&self, definition: &Definition) -> Standing {
        let Some(project) = &self.project else {
            return Standing::Untrusted;
        };
        let pinned = project
            .pinned
            .iter()
            .find(|pin| pin.kind == definition.kind.as_str() && pin.name == definition.name);
        match pinned {
            Some(pin) if pin.hash == definition.hash => Standing::Approved,
            Some(_) => Standing::Changed,
            None => Standing::New,
        }
    }

    /// Whether the project's rules, as its files hold them now, are the
    /// ones the user approved: `rules` is **every** rule of the project,
    /// whatever it does ([`Definition::of_rules`]). Any change, a rule that
    /// denies or asks added or dropped included, is a change to approve.
    ///
    /// A trusted project that has no rules, and had none pinned, needs no
    /// approval for them. One whose pinned rules are all gone has changed.
    #[must_use]
    pub fn rules_approved(&self, rules: &[Rule]) -> bool {
        match self.standing(&Definition::of_rules(rules)) {
            Standing::Approved => true,
            Standing::New => rules.is_empty(),
            Standing::Changed | Standing::Untrusted => false,
        }
    }

    /// Whether the project's rules, outside any profile and in each of its
    /// profiles, are the ones the user approved
    /// ([`Definition::of_project_rules`]), as
    /// [`rules_approved`](Self::rules_approved) judges a project's rules.
    #[must_use]
    pub fn project_rules_approved(&self, rules: &ProjectRules) -> bool {
        match self.standing(&Definition::of_project_rules(rules)) {
            Standing::Approved => true,
            Standing::New => rules.is_empty(),
            Standing::Changed | Standing::Untrusted => false,
        }
    }
}

/// The trust store: which projects the user has trusted, and the hashes of
/// what they trusted.
#[derive(Debug, Clone)]
pub struct TrustStore {
    file: PathBuf,
    clock: SharedClock,
}

impl TrustStore {
    /// The store at `file`: `StateDir::trust_file` of `cortex-config`.
    #[must_use]
    pub fn new(file: impl Into<PathBuf>, clock: SharedClock) -> Self {
        Self {
            file: file.into(),
            clock,
        }
    }

    /// The file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.file
    }

    /// What the user has trusted of the project `key`.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood in full. The
    /// caller treats the project as not trusted, and shows the error.
    pub fn project(&self, key: &ProjectKey) -> Result<ProjectTrust, TrustError> {
        let file = self.read()?;
        let refused = file.refused.iter().any(|refused| refused.key == *key);
        Ok(ProjectTrust {
            older: file.version < VERSION,
            project: file
                .projects
                .into_iter()
                .find(|project| project.key == *key),
            refused,
        })
    }

    /// Trusts a project for exactly these definitions, replacing whatever
    /// was pinned for it before. `root` is the workspace root as it should
    /// be shown to a person (`~/work/app`). `git_shown`: git was in what
    /// the person was shown (the project is a repository), and so is
    /// trusted with it.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood (it is then not
    /// overwritten), or cannot be written.
    pub fn trust(
        &self,
        key: &ProjectKey,
        root: &str,
        definitions: &[Definition],
        git_shown: bool,
    ) -> Result<ProjectTrust, TrustError> {
        self.update(|file| {
            // Trusting a project the user had refused lifts the refusal.
            file.refused.retain(|refused| refused.key != *key);
            Ok((self.trusted_in(file, key, root, definitions, git_shown), true))
        })
    }

    /// Writes into `file` the project trusted for exactly `definitions`.
    fn trusted_in(
        &self,
        file: &mut File,
        key: &ProjectKey,
        root: &str,
        definitions: &[Definition],
        git_shown: bool,
    ) -> ProjectTrust {
        file.projects.retain(|project| project.key != *key);
        let project = Project {
            key: key.clone(),
            root: root.to_owned(),
            trusted_at: self.clock.now().to_timestamp(),
            git: git_shown.then_some(GitAnswer::Trusted),
            declined_rules: None,
            pinned: pins_of(definitions),
        };
        file.projects.push(project.clone());
        ProjectTrust {
            project: Some(project),
            refused: false,
            older: false,
        }
    }

    /// Approves definitions of a project that is already trusted: new ones,
    /// and changed ones as they are now. Everything else that was pinned
    /// stays. Rules kept out before are no longer when rules are approved
    /// (a definition of kind `rules` among `definitions`): what is approved
    /// now is what stands; an approval of git alone leaves them kept out
    /// (the review of the follow-ups of step 23). `git_shown`: git was in
    /// what the person was shown, and is trusted now; otherwise what was
    /// said of git stays.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood, or cannot be
    /// written; or when the project is not trusted, which approving one
    /// definition must not make it.
    pub fn approve(
        &self,
        key: &ProjectKey,
        definitions: &[Definition],
        git_shown: bool,
    ) -> Result<ProjectTrust, TrustError> {
        self.update(|file| {
            let Some(project) = file.projects.iter_mut().find(|project| project.key == *key) else {
                return Err(self.not_understood(
                    "the project is not trusted, so no definition of it can be approved".to_owned(),
                ));
            };
            for pin in pins_of(definitions) {
                project
                    .pinned
                    .retain(|old| !(old.kind == pin.kind && old.name == pin.name));
                project.pinned.push(pin);
            }
            if definitions
                .iter()
                .any(|definition| definition.kind == TrustResourceKind::Rules)
            {
                project.declined_rules = None;
            }
            if git_shown {
                project.git = Some(GitAnswer::Trusted);
            }
            Ok((
                ProjectTrust {
                    project: Some(project.clone()),
                    refused: false,
                    older: false,
                },
                true,
            ))
        })
    }

    /// Keeps trusting a project as it was trusted, without what is new in
    /// it (since the follow-ups of step 23, the owner's decision of
    /// 2026-11-16): `rules`, its rules as they are now, which stay held back
    /// and are not asked about while they are these; `git`, git, which does
    /// not read the repository's configuration and is not asked about
    /// again. What was trusted before (pins, git) is not touched.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood, or cannot be
    /// written; or when the project is not trusted.
    pub fn keep_without(
        &self,
        key: &ProjectKey,
        rules: Option<&Definition>,
        git: bool,
    ) -> Result<ProjectTrust, TrustError> {
        self.update(|file| {
            let Some(project) = file.projects.iter_mut().find(|project| project.key == *key) else {
                return Err(self.not_understood(
                    "the project is not trusted, so it cannot be kept trusted".to_owned(),
                ));
            };
            if let Some(rules) = rules {
                project.declined_rules = Some(rules.hash);
            }
            // Git trusted before stays trusted: only git never shown is
            // declined.
            if git && project.git != Some(GitAnswer::Trusted) {
                project.git = Some(GitAnswer::Declined);
            }
            Ok((
                ProjectTrust {
                    project: Some(project.clone()),
                    refused: false,
                    older: false,
                },
                true,
            ))
        })
    }

    /// [`trust`](Self::trust), unless the project was marked never since it
    /// was looked at: the answer to a project's first question, which a
    /// never said meanwhile (by another client or process) outranks (the
    /// review of the follow-ups of step 23). `cortex permissions trust`,
    /// the person's own command, lifts a never with [`trust`](Self::trust).
    ///
    /// # Errors
    ///
    /// As [`trust`](Self::trust); and, writing nothing, when the project is
    /// marked never.
    pub fn trust_unless_refused(
        &self,
        key: &ProjectKey,
        root: &str,
        definitions: &[Definition],
        git_shown: bool,
    ) -> Result<ProjectTrust, TrustError> {
        self.update(|file| {
            if file.refused.iter().any(|refused| refused.key == *key) {
                return Err(self.not_understood(
                    "the project was marked never while it was asked about, so it is not \
                     trusted"
                        .to_owned(),
                ));
            }
            Ok((self.trusted_in(file, key, root, definitions, git_shown), true))
        })
    }

    /// Marks a project as one the user will never trust: it is not asked
    /// about again, and any trust it had is taken away. `root` is for
    /// showing a person which project it is.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood, or cannot be
    /// written.
    pub fn refuse(&self, key: &ProjectKey, root: &str) -> Result<(), TrustError> {
        self.update(|file| {
            file.projects.retain(|project| project.key != *key);
            file.refused.retain(|refused| refused.key != *key);
            file.refused.push(Refused {
                key: key.clone(),
                root: root.to_owned(),
                refused_at: self.clock.now().to_timestamp(),
            });
            Ok(((), true))
        })
    }

    /// Forgets what the user said of a project, trust and refusal alike, so
    /// that it is asked about again. Says whether there was anything.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood, or cannot be
    /// written.
    pub fn forget(&self, key: &ProjectKey) -> Result<bool, TrustError> {
        self.update(|file| {
            let before = file.projects.len() + file.refused.len();
            file.projects.retain(|project| project.key != *key);
            file.refused.retain(|refused| refused.key != *key);
            let changed = file.projects.len() + file.refused.len() != before;
            Ok((changed, changed))
        })
    }

    /// The projects the user said never to trust, as their key and root.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood.
    pub fn refused(&self) -> Result<Vec<(ProjectKey, String)>, TrustError> {
        Ok(self
            .read()?
            .refused
            .into_iter()
            .map(|refused| (refused.key, refused.root))
            .collect())
    }

    /// Takes trust away from a project. Says whether it was trusted.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood, or cannot be
    /// written.
    pub fn revoke(&self, key: &ProjectKey) -> Result<bool, TrustError> {
        self.update(|file| {
            let before = file.projects.len();
            file.projects.retain(|project| project.key != *key);
            let was_trusted = file.projects.len() != before;
            Ok((was_trusted, was_trusted))
        })
    }

    /// The projects the user has trusted, as their key and the root they
    /// were trusted at, for `cortex permissions`.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood.
    pub fn list(&self) -> Result<Vec<(ProjectKey, String)>, TrustError> {
        Ok(self
            .read()?
            .projects
            .into_iter()
            .map(|project| (project.key, project.root))
            .collect())
    }

    /// Reads the file, changes it and writes it if `change` says it changed
    /// (the second of what it returns), with the lock held throughout. Every
    /// change to the file goes through here.
    fn update<T>(
        &self,
        change: impl FnOnce(&mut File) -> Result<(T, bool), TrustError>,
    ) -> Result<T, TrustError> {
        let _lock = store_lock::hold(&self.file).map_err(|source| AtomicWriteError::Io {
            path: self.file.display().to_string(),
            step: "lock",
            source,
        })?;
        let mut file = self.read()?;
        let (answer, changed) = change(&mut file)?;
        if changed {
            self.write(&file)?;
        }
        Ok(answer)
    }

    fn not_understood(&self, problem: String) -> TrustError {
        TrustError::NotUnderstood {
            path: self.file.display().to_string(),
            problem,
        }
    }

    fn read(&self) -> Result<File, TrustError> {
        let read_error = |source| TrustError::Read {
            path: self.file.display().to_string(),
            source,
        };
        let size = match std::fs::metadata(&self.file) {
            Ok(found) => found.len(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(File {
                    version: VERSION,
                    projects: Vec::new(),
                    refused: Vec::new(),
                });
            }
            Err(error) => return Err(read_error(error)),
        };
        if size > MAX_BYTES {
            return Err(self.not_understood(format!(
                "it is {size} bytes, more than a trust store could be"
            )));
        }
        let text = std::fs::read_to_string(&self.file).map_err(read_error)?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let file: File = toml::from_str(text)
            .map_err(|error| self.not_understood(error.message().to_owned()))?;
        if !(OLDEST..=VERSION).contains(&file.version) {
            return Err(self.not_understood(format!(
                "its version is {}, and this version of Cortex-Harness reads versions {OLDEST} \
                 to {VERSION}",
                file.version
            )));
        }
        // A version 1 file has no record of git or of declined rules: one
        // that says it has is not one this build wrote.
        if file.version < 2
            && file
                .projects
                .iter()
                .any(|project| project.git.is_some() || project.declined_rules.is_some())
        {
            return Err(self.not_understood(
                "it is version 1, which has no `git` or `declined_rules`".to_owned(),
            ));
        }
        Ok(file)
    }

    fn write(&self, file: &File) -> Result<(), TrustError> {
        // Whatever version was read, what is written is this build's.
        let file = File {
            version: VERSION,
            ..file.clone()
        };
        // The directory was made when the lock was taken.
        let text = toml::to_string(&file).map_err(|error| {
            self.not_understood(format!("it could not be written out: {error}"))
        })?;
        let options = AtomicWriteOptions {
            owner_only: true,
            ..AtomicWriteOptions::default()
        };
        write_atomic(&self.file, text.as_bytes(), &options, &*self.clock)?;
        Ok(())
    }
}

fn pins_of(definitions: &[Definition]) -> Vec<Pin> {
    let mut pins: Vec<Pin> = Vec::with_capacity(definitions.len());
    for definition in definitions {
        let pin = Pin {
            kind: definition.kind.as_str().to_owned(),
            name: definition.name.clone(),
            hash: definition.hash,
        };
        pins.retain(|old| !(old.kind == pin.kind && old.name == pin.name));
        pins.push(pin);
    }
    pins
}

#[cfg(test)]
mod tests {
    use cortex_protocol::lease::Effect;

    use super::*;

    fn hook(name: &str, parts: &[&str]) -> Definition {
        Definition::of(TrustResourceKind::Hook, name, parts.iter().copied())
    }

    fn trusted_with(definitions: &[Definition]) -> ProjectTrust {
        ProjectTrust {
            project: Some(Project {
                key: ProjectKey::new("k"),
                root: String::new(),
                trusted_at: Timestamp::new("2026-11-13T10:00:00Z"),
                git: None,
                declined_rules: None,
                pinned: pins_of(definitions),
            }),
            refused: false,
            older: false,
        }
    }

    #[test]
    fn a_refused_project_is_known_as_such_until_trusted_or_forgotten() {
        let dir =
            std::env::temp_dir().join(format!("cortex-policy-refused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let clock: SharedClock = std::sync::Arc::new(
            cortex_base::FakeClock::starting_at("2026-11-15T10:00:00Z").unwrap(),
        );
        let store = TrustStore::new(dir.join("trust.toml"), clock);
        let key = ProjectKey::new("k");
        assert!(!store.project(&key).unwrap().is_refused());
        // Refusing takes trust away.
        store.trust(&key, "~/app", &[], false).unwrap();
        store.refuse(&key, "~/app").unwrap();
        let refused = store.project(&key).unwrap();
        assert!(refused.is_refused() && !refused.is_trusted());
        assert_eq!(store.refused().unwrap().len(), 1);
        // It survives a new store over the same file: a restart.
        let again = TrustStore::new(
            store.path().to_owned(),
            std::sync::Arc::new(cortex_base::SystemClock::new()),
        );
        assert!(again.project(&key).unwrap().is_refused());
        // Trusting lifts it.
        again.trust(&key, "~/app", &[], false).unwrap();
        let trusted = again.project(&key).unwrap();
        assert!(trusted.is_trusted() && !trusted.is_refused());
        // Forgetting clears both.
        again.refuse(&key, "~/app").unwrap();
        assert!(again.forget(&key).unwrap());
        let forgotten = again.project(&key).unwrap();
        assert!(!forgotten.is_trusted() && !forgotten.is_refused());
        assert!(!again.forget(&key).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_definition_is_its_parts_and_where_they_divide() {
        let base = hook("format", &["cargo", "fmt"]);
        assert_eq!(base, hook("format", &["cargo", "fmt"]));
        assert_ne!(base.hash, hook("format", &["cargo", "fmt", "--all"]).hash);
        assert_ne!(base.hash, hook("format", &["cargo fmt"]).hash);
        assert_ne!(base.hash, hook("format", &["cargofmt"]).hash);
        assert_ne!(base.hash, hook("format", &["cargo", "fm", "t"]).hash);
        assert_ne!(
            hook("x", &["rm", "-rf x"]).hash,
            hook("x", &["rm -rf", "x"]).hash
        );
        assert_ne!(hook("x", &[]).hash, hook("x", &[""]).hash);
    }

    #[test]
    fn a_definition_stands_by_its_hash() {
        let approved = hook("format", &["cargo", "fmt"]);
        let trust = trusted_with(std::slice::from_ref(&approved));
        assert!(trust.is_trusted());
        assert_eq!(trust.standing(&approved), Standing::Approved);
        // Changed since it was approved: disabled.
        let changed = hook("format", &["cargo", "fmt", "&&", "curl", "evil"]);
        assert_eq!(trust.standing(&changed), Standing::Changed);
        assert!(!trust.standing(&changed).is_approved());
        // Not among what the user saw.
        assert_eq!(
            trust.standing(&hook("deploy", &["./deploy"])),
            Standing::New
        );
        // The same name of another kind is another thing.
        let server = Definition::of(TrustResourceKind::McpServer, "format", ["cargo", "fmt"]);
        assert_eq!(trust.standing(&server), Standing::New);
        // And without trust nothing stands.
        let nobody = ProjectTrust::untrusted();
        assert!(!nobody.is_trusted());
        assert_eq!(nobody.standing(&approved), Standing::Untrusted);
    }

    #[test]
    fn a_projects_rules_are_pinned_together_whatever_their_order() {
        use cortex_protocol::lease::RuleAction;
        let rule = |effect: Effect, pattern: &str, action| Rule {
            effect,
            pattern: pattern.to_owned(),
            action,
        };
        let test = rule(Effect::Exec, "cargo test *", RuleAction::Allow);
        let build = rule(Effect::Exec, "cargo build *", RuleAction::Allow);
        let deny = rule(Effect::Write, "**/.env*", RuleAction::Deny);
        let seen = [test.clone(), build.clone(), deny.clone()];
        let trust = trusted_with(&[Definition::of_rules(&seen)]);

        assert!(trust.rules_approved(&seen));
        // Order and repeats do not matter.
        assert!(trust.rules_approved(&[deny.clone(), build.clone(), test.clone(), test.clone()]));
        // An added allow, a dropped one or a changed one is a change.
        let more = [
            test.clone(),
            build.clone(),
            deny.clone(),
            rule(Effect::Exec, "rm *", RuleAction::Allow),
        ];
        assert!(!trust.rules_approved(&more));
        assert!(!trust.rules_approved(&[test.clone(), deny.clone()]));
        assert!(!trust.rules_approved(&[
            test.clone(),
            rule(Effect::Exec, "cargo build", RuleAction::Allow),
            deny.clone()
        ]));
        // The same pattern for another effect is another rule.
        assert!(!trust.rules_approved(&[
            test.clone(),
            rule(Effect::Write, "cargo build *", RuleAction::Allow),
            deny.clone()
        ]));
        // So is the same pattern with another action.
        assert!(!trust.rules_approved(&[
            test.clone(),
            build.clone(),
            rule(Effect::Write, "**/.env*", RuleAction::Ask)
        ]));
        // A rule that holds back, dropped or added, is a change too: the
        // user trusted the allow rules with what held them back. Found by
        // the review of step 12.
        assert!(!trust.rules_approved(&[test.clone(), build.clone()]));
        assert!(!trust.rules_approved(&[
            test.clone(),
            build.clone(),
            deny.clone(),
            rule(Effect::Exec, "make deploy", RuleAction::Deny)
        ]));
        // All of them gone is a change.
        assert!(!trust.rules_approved(&[]));
        // A trusted project that had no rules pinned, and has none, needs
        // no approval; any rule it gains, of any action, does.
        let plain = trusted_with(&[]);
        assert!(plain.rules_approved(&[]));
        assert!(!plain.rules_approved(std::slice::from_ref(&deny)));
        assert!(!plain.rules_approved(std::slice::from_ref(&test)));
        assert!(!ProjectTrust::untrusted().rules_approved(&seen));
        assert!(!ProjectTrust::untrusted().rules_approved(&[]));
    }

    #[test]
    fn every_change_to_the_store_is_made_under_its_lock() {
        use cortex_base::FakeClock;

        use crate::store_lock::tests::{Scratch, is_held};

        let scratch = Scratch::new();
        let file = scratch.0.join("trust.toml");
        let clock: SharedClock =
            std::sync::Arc::new(FakeClock::starting_at("2026-11-13T10:00:00Z").unwrap());
        let store = TrustStore::new(&file, clock);
        let key = ProjectKey::new("k");
        // Every public change goes through `update`, and `update` holds the
        // lock from the read to the write.
        let held = store.update(|_| Ok((is_held(&file), false))).unwrap();
        assert!(held, "the lock is held while the file is changed");
        assert!(!is_held(&file), "and let go after");
        store.trust(&key, "~/x", &[], false).unwrap();
        store.approve(&key, &[], false).unwrap();
        assert!(store.revoke(&key).unwrap());
        assert!(!store.project(&key).unwrap().is_trusted());
        assert!(!is_held(&file));

        // A lock that cannot be taken stops every change, and no read: so
        // each change takes it. (A directory stands where the lock goes.)
        store.trust(&key, "~/x", &[], false).unwrap();
        let lock = crate::store_lock::lock_path(&file);
        std::fs::remove_file(&lock).unwrap();
        std::fs::create_dir(&lock).unwrap();
        let refused = |result: Result<(), TrustError>| {
            assert!(
                matches!(
                    &result,
                    Err(TrustError::Write(AtomicWriteError::Io { step: "lock", .. }))
                ),
                "{result:?}"
            );
        };
        refused(store.trust(&ProjectKey::new("other"), "~/y", &[], false).map(drop));
        refused(store.approve(&key, &[], false).map(drop));
        refused(store.revoke(&key).map(drop));
        assert!(store.project(&key).unwrap().is_trusted());
        assert_eq!(store.list().unwrap().len(), 1);
    }
}
