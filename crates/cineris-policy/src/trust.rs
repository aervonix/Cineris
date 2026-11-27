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
//! version = 3
//!
//! [[project]]
//! key = "9f2c4a1b7d3e…"
//! root = "~/work/app"
//! trusted_at = "2026-11-13T10:15:02Z"
//! revision = 2
//! git = "trusted"
//! declined_rules = "0b7c…"
//! signature = "1f0c…"   # over the key, revision, root, git and every pin
//!
//! [[project.pinned]]
//! kind = "hook"
//! name = "format"
//! hash = "5e884898da28…"
//! ```
//!
//! - **Each project's trust is signed whole** (version 3, since the signed
//!   approvals step): one Ed25519 signature, by the key the person's
//!   password makes, over the project's key, its revision, its root,
//!   whether git is trusted, and every pin's kind, name, hash and
//!   registration, in the order of kind and name ([`crate::signed`]). A
//!   project counts as trusted, and a pin of it toward a [`Standing`], only
//!   while its signature checks with the session's [`Verifier`]: one written
//!   by another program, one with a pin taken out or put back, one copied
//!   from another project, and one of an older version count for nothing.
//! - **Every change of what is signed is the person's.** Trusting a
//!   project, approving what is new in it and taking pins away are each
//!   proposed ([`TrustStore::propose`]), signed by the person at their yes,
//!   and committed ([`TrustStore::commit`]), which writes nothing unless the
//!   signature is over exactly what the project's entry would then hold.
//!   Taking a pin away is signed too: a pin can hold a gate, and a gate
//!   dropped quietly is a widening (the maintainers' answer to the design's
//!   call 4).
//! - **What restricts is read unsigned:** `declined_rules`, `git =
//!   "declined"` and `[[refused]]` only hold something back or keep a
//!   question from being asked, so another program that writes them can
//!   only make the harness stricter or quieter about a project that is not
//!   trusted. Every pin of an entry, signed or not, is in
//!   [`ProjectTrust::held`]: a session holds the gates of what it names. And
//!   a whole entry taken away leaves the project untrusted, as never trusted:
//!   asked about again.
//! - **The revision** goes up by one at each signed change of the entry.
//!   An earlier entry, signed whole, still checks: putting a whole earlier
//!   entry back is the stated rollback (`docs/security.md`, section 3).
//! - **Projects are known by their key** (`AbsPath::project_key`), which is
//!   made from the resolved root in its stored case, so a folder that
//!   differs from a trusted one only in case is another project. `root` is
//!   for showing a person which project an entry is, and is never compared;
//!   it is signed, so that what a person is shown of a trusted project is
//!   what they trusted.
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
//!   repository's configuration ([`ProjectTrust::git_trusted`]). Only
//!   `trusted` is signed.
//! - **Rules a person kept trusting the project without** (version 2):
//!   `declined_rules`, the hash of the project's rules as they were when the
//!   person answered "keep trusting it, without the new rules". They stay
//!   held back, and are not asked about again while they are those rules;
//!   a further change asks again. Approving rules clears it.
//! - **Versions.** This build writes version 3 and reads 1 to 3. Versions
//!   1 and 2 signed nothing: their projects are read as unsigned, trusted
//!   for nothing and asked about again, while their refusals and declines
//!   still hold. Nothing unsigned is signed in bulk, since an entry written
//!   by the model before the upgrade would be laundered. A build that reads
//!   only up to version 2 refuses a version 3 file whole, by its version
//!   (decision 0028's rule: a file a build cannot read in full is not read
//!   in part), and so trusts nothing until it is updated.
//!
//! # The user's own pins
//!
//! What runs code and is the user's own, not a project's, belongs to no
//! project, and is pinned in a file of its own beside this one (since step
//! 28): `pins.toml`, the [`PinStore`], whose module documentation says why it
//! is not a version 3 of this file. The user's own extensions are pinned
//! there; a [`Definition`] stands with it as with a trusted project
//! ([`UserPins::standing`]), never `Untrusted`.

#![allow(
    clippy::disallowed_methods,
    reason = "blocking file calls; the store is used at start-up and on the blocking pool"
)]

use std::io;
use std::path::{Path, PathBuf};

use cineris_base::{
    AtomicWriteError, AtomicWriteOptions, ContentHash, ContentHasher, SharedClock, write_atomic,
};
use cineris_protocol::ids::{ProjectKey, Timestamp};
use cineris_protocol::lease::Rule;
use cineris_protocol::methods::TrustResourceKind;
use serde::{Deserialize, Serialize};

use cineris_protocol::methods::{ApprovalProposal, ProposedPin};

use crate::signed::{Signature, SignedPin, Statement, Verifier, project_statement};
use crate::store_lock;

mod pins;

pub use pins::{
    BEFORE_THE_PINS_ARE_WRITTEN, PinChange, PinError, PinProposal, PinStore, UserPin, UserPins,
};

/// The version of the file's format this build writes: the first that
/// signs.
const VERSION: u32 = 3;

/// The oldest version this build reads: version 1, which has no record of
/// git or of declined rules.
const OLDEST: u32 = 1;

/// How an entry of a store stands with the session's key (the signed
/// approvals step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checked {
    /// Signed by the session's key, over exactly what it holds: it counts.
    Signed,
    /// There is nothing to check: no file, no pin, no entry.
    Empty,
    /// The session has no key to check with (the person did not open their
    /// approvals, or there is nobody to ask): it holds its gates and
    /// approves nothing.
    NoKey,
    /// Written by an older version, which signed nothing: asked again.
    Unsigned,
    /// Signed by another key, or changed since it was signed: written by
    /// something other than the person's Cineris. Asked again.
    NotTheirs,
}

impl Checked {
    /// How `signature`, over `statement`, stands with `verifier`.
    fn of(
        signature: Option<&Signature>,
        statement: &Statement,
        verifier: Option<&Verifier>,
    ) -> Self {
        let Some(signature) = signature else {
            return Self::Unsigned;
        };
        let Some(verifier) = verifier else {
            return Self::NoKey;
        };
        if verifier.verify(statement, signature) {
            Self::Signed
        } else {
            Self::NotTheirs
        }
    }
}

/// A definition as the person approves it: with, for an extension evaluated
/// before the question, the hash of what it registered (the registration
/// cache's entry is used only while it is that one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approved {
    /// What is approved.
    pub definition: Definition,
    /// What it registered, where that was known when it was approved.
    pub registration: Option<ContentHash>,
}

impl From<Definition> for Approved {
    fn from(definition: Definition) -> Self {
        Self {
            definition,
            registration: None,
        }
    }
}

/// The largest file read.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// One thing in a project that can run code or grant permissions, or one of
/// the user's own that runs code (an extension in the user's configuration
/// directory, since step 28), as it is now: what kind of thing, its name,
/// and the hash of its definition.
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
    /// Of the user's own ([`UserPins`]): nothing of its kind and name is
    /// pinned.
    New,
    /// The project is not trusted. Never said of the user's own.
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
    /// The project's entry changed between the question and the answer:
    /// nothing was written, and the person is asked again.
    #[error(
        "this project's trust in `{path}` changed while you were asked, so nothing was written: \
         asked again"
    )]
    Moved {
        /// The file.
        path: String,
    },
    /// The signature given is not the session's key's over what would be
    /// written: nothing was written.
    #[error(
        "the approval for `{path}` is not signed by your key over what it would hold, so nothing \
         was written"
    )]
    NotSigned {
        /// The file.
        path: String,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registration: Option<ContentHash>,
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
    /// The number of signed changes of the entry (version 3).
    #[serde(default, skip_serializing_if = "is_zero")]
    revision: u64,
    /// Absent: git was never shown (and every project of a version 1
    /// file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git: Option<GitAnswer>,
    /// The project's rules as they were when the person kept trusting it
    /// without them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    declined_rules: Option<ContentHash>,
    /// The person's signature over the entry (version 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<Signature>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pinned: Vec<Pin>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl Project {
    /// What the entry's signature is over: the project's key, the
    /// revision, the root, whether git is trusted, and each pin in the
    /// order of kind and name.
    fn statement(&self) -> Statement {
        let mut pins: Vec<SignedPin<'_>> = self
            .pinned
            .iter()
            .map(|pin| SignedPin {
                kind: &pin.kind,
                name: &pin.name,
                hash: *pin.hash.as_bytes(),
                registration: pin.registration.map(|hash| *hash.as_bytes()),
            })
            .collect();
        project_statement(
            self.key.as_str(),
            self.revision,
            &self.root,
            self.git == Some(GitAnswer::Trusted),
            &mut pins,
        )
    }

    /// The entry as the protocol shows it to a front end that signs it.
    fn view(&self) -> ApprovalProposal {
        ApprovalProposal::Project {
            key: self.key.clone(),
            revision: self.revision,
            root: self.root.clone(),
            git_trusted: self.git == Some(GitAnswer::Trusted),
            pins: self
                .pinned
                .iter()
                .map(|pin| ProposedPin {
                    kind: TrustResourceKind::from_wire(&pin.kind),
                    name: pin.name.clone(),
                    hash: pin.hash.to_hex(),
                    registration: pin.registration.map(|hash| hash.to_hex()),
                })
                .collect(),
        }
    }

    /// How the entry stands with `verifier`.
    fn checked(&self, verifier: Option<&Verifier>) -> Checked {
        Checked::of(self.signature.as_ref(), &self.statement(), verifier)
    }
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTrust {
    /// The project's entry, signed or not.
    project: Option<Project>,
    /// How the entry stands with the session's key: `Empty` with none.
    checked: Checked,
    refused: bool,
    /// The file read was of an older version, which kept no record of git.
    older: bool,
}

impl Default for ProjectTrust {
    fn default() -> Self {
        Self {
            project: None,
            checked: Checked::Empty,
            refused: false,
            older: false,
        }
    }
}

impl ProjectTrust {
    /// A project nobody has trusted.
    #[must_use]
    pub fn untrusted() -> Self {
        Self::default()
    }

    /// The entry, when it is signed by the session's key: what counts.
    fn signed(&self) -> Option<&Project> {
        self.project
            .as_ref()
            .filter(|_| self.checked == Checked::Signed)
    }

    /// Whether the user has trusted the project at all: its entry is signed
    /// by the session's key.
    #[must_use]
    pub fn is_trusted(&self) -> bool {
        self.signed().is_some()
    }

    /// How the project's entry stands with the session's key: `Empty` when
    /// it has none.
    #[must_use]
    pub fn checked(&self) -> Checked {
        self.checked
    }

    /// Whether the store holds an entry for the project, signed by the
    /// session's key or not: one that does not count is not asked about as
    /// a new project in a session the person opened without their
    /// approvals.
    #[must_use]
    pub fn has_entry(&self) -> bool {
        self.project.is_some()
    }

    /// The revision of the project's entry: 0 for none.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.project.as_ref().map_or(0, |project| project.revision)
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
        self.signed()
            .is_some_and(|project| project.git == Some(GitAnswer::Trusted))
    }

    /// Whether the person kept trusting the project without git: it is not
    /// asked about again.
    #[must_use]
    pub fn git_declined(&self) -> bool {
        self.signed()
            .is_some_and(|project| project.git == Some(GitAnswer::Declined))
    }

    /// Whether the person kept trusting the project without exactly these
    /// rules ([`Definition::of_project_rules`]): they stay held back, and
    /// are not asked about again.
    #[must_use]
    pub fn rules_declined(&self, rules: &ProjectRules) -> bool {
        let definition = Definition::of_project_rules(rules);
        self.signed()
            .is_some_and(|project| project.declined_rules == Some(definition.hash))
    }

    /// Whether the store was of an older version, which kept no record of
    /// whether git was shown: a session says why git is asked about again.
    #[must_use]
    pub fn from_older_store(&self) -> bool {
        self.older
    }

    /// The names of the project's `kind` that the user approved, each with
    /// the hash it was pinned at: none for a project that is not trusted.
    /// What a session holds a changed extension's gates by (step 28: its
    /// pinned registration is kept).
    #[must_use]
    pub fn pinned_of(&self, kind: &TrustResourceKind) -> Vec<(String, ContentHash)> {
        self.signed()
            .map(|project| pins_of_kind(project, kind))
            .unwrap_or_default()
    }

    /// The names of the project's `kind` that its entry pins, signed or
    /// not, each with its hash: what restricts. A session holds the gates of
    /// each as it holds those of an approved thing that is not loaded,
    /// whatever the entry's signature (the signed approvals step).
    #[must_use]
    pub fn held(&self, kind: &TrustResourceKind) -> Vec<(String, ContentHash)> {
        self.project
            .as_ref()
            .map(|project| pins_of_kind(project, kind))
            .unwrap_or_default()
    }

    /// The hash of what the pin of this kind and name registered, when the
    /// entry is signed and holds one.
    #[must_use]
    pub fn registration(&self, kind: &TrustResourceKind, name: &str) -> Option<ContentHash> {
        self.signed().and_then(|project| {
            project
                .pinned
                .iter()
                .find(|pin| pin.kind == kind.as_str() && pin.name == name)
                .and_then(|pin| pin.registration)
        })
    }

    /// How a definition stands.
    #[must_use]
    pub fn standing(&self, definition: &Definition) -> Standing {
        let Some(project) = self.signed() else {
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

fn pins_of_kind(project: &Project, kind: &TrustResourceKind) -> Vec<(String, ContentHash)> {
    project
        .pinned
        .iter()
        .filter(|pin| pin.kind == kind.as_str())
        .map(|pin| (pin.name.clone(), pin.hash))
        .collect()
}

/// One change to a project's trust, as the person is asked to sign it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustChange {
    /// Trust the project for exactly these definitions, replacing whatever
    /// was pinned for it before. `root` is the workspace root as it should
    /// be shown to a person (`~/work/app`). `git_shown`: git was in what
    /// the person was shown (the project is a repository), and so is
    /// trusted with it. `lift_refusal`: a never said of the project is
    /// lifted (`cineris permissions trust`, the person's own command); when
    /// false, a never said while the question was open outranks the yes,
    /// and nothing is written (the answer to a project's first question).
    Trust {
        /// The root, as shown.
        root: String,
        /// Everything approved.
        approved: Vec<Approved>,
        /// Whether git was shown.
        git_shown: bool,
        /// Whether a never is lifted.
        lift_refusal: bool,
    },
    /// Approve definitions of a project that is already trusted: new ones,
    /// and changed ones as they are now. Everything else that was pinned
    /// stays. Rules kept out before are no longer when rules are approved
    /// (a definition of kind `rules` among them). `git_shown`: git was in
    /// what the person was shown, and is trusted now; otherwise what was
    /// said of git stays.
    Approve {
        /// What is approved.
        approved: Vec<Approved>,
        /// Whether git was shown.
        git_shown: bool,
    },
    /// Take away the pins of these kinds and names from a trusted project:
    /// what was removed, at the person's yes.
    Drop {
        /// Each pin's kind and name.
        pins: Vec<(TrustResourceKind, String)>,
    },
}

/// A change to a project's trust, worked out before the person is asked
/// ([`TrustStore::propose`]): what the project's entry would then hold, the
/// statement the person signs, and what changes, for them to read.
#[derive(Debug, Clone)]
pub struct TrustProposal {
    key: ProjectKey,
    /// The project's entry when the change was worked out: committed only
    /// while it still is.
    base: Option<Project>,
    next: Project,
    statement: Statement,
    summary: Vec<String>,
    lift_refusal: bool,
    /// Whether the change approves rules, which clears rules declined.
    approves_rules: bool,
}

impl TrustProposal {
    /// The statement the person signs.
    #[must_use]
    pub fn statement(&self) -> &Statement {
        &self.statement
    }

    /// What changes, one line each, for the person to read before they
    /// sign.
    #[must_use]
    pub fn summary(&self) -> &[String] {
        &self.summary
    }

    /// The project it is of.
    #[must_use]
    pub fn key(&self) -> &ProjectKey {
        &self.key
    }

    /// What the person signs, as the protocol carries it to a front end:
    /// [`crate::signed::statement_of`] makes this proposal's statement from
    /// it.
    #[must_use]
    pub fn view(&self) -> ApprovalProposal {
        self.next.view()
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
    /// The store at `file`: `StateDir::trust_file` of `cineris-config`.
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

    /// What the user has trusted of the project `key`, as it stands with
    /// `verifier`, the session's key: with none, nothing is trusted, and
    /// the entry's pins hold their gates ([`ProjectTrust::held`]).
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood in full. The
    /// caller treats the project as not trusted, and shows the error.
    pub fn project(
        &self,
        key: &ProjectKey,
        verifier: Option<&Verifier>,
    ) -> Result<ProjectTrust, TrustError> {
        let file = self.read()?;
        Ok(trust_of(file, key, verifier))
    }

    /// Works out `change` to the project `key` against its entry as it
    /// stands with `verifier`, the session's key, when the person's
    /// approvals are open: what the entry would then hold, and the statement
    /// the person signs for it. `None` when the change changes nothing (a
    /// drop of what is not pinned), so that nobody is asked.
    ///
    /// A trust replaces the whole entry, so it is worked out with no key
    /// too: a project's first question is put before the person has opened
    /// their approvals, and the yes is signed, and committed, once they
    /// have. An approval and a drop keep what the entry holds, so each needs
    /// the key the entry is checked with.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood; or when an
    /// approval or a drop is of a project that is not trusted, which
    /// approving one definition must not make it, or is worked out with no
    /// key.
    pub fn propose(
        &self,
        key: &ProjectKey,
        change: TrustChange,
        verifier: Option<&Verifier>,
    ) -> Result<Option<TrustProposal>, TrustError> {
        if verifier.is_none() && !matches!(change, TrustChange::Trust { .. }) {
            return Err(self.not_understood(
                "your approvals are not open, so what the project's trust holds cannot be kept \
                 or changed"
                    .to_owned(),
            ));
        }
        let file = self.read()?;
        let base = file
            .projects
            .iter()
            .find(|project| project.key == *key)
            .cloned();
        let signed = base
            .as_ref()
            .filter(|project| project.checked(verifier) == Checked::Signed);
        let revision = base.as_ref().map_or(0, |project| project.revision) + 1;
        let mut summary = Vec::new();
        let (next, lift_refusal, approves_rules) = match change {
            TrustChange::Trust {
                root,
                approved,
                git_shown,
                lift_refusal,
            } => {
                if base.is_some() && signed.is_none() {
                    summary.push(if verifier.is_some() {
                        "its earlier trust, not signed by your key, is replaced".to_owned()
                    } else {
                        "its earlier trust is replaced".to_owned()
                    });
                }
                summary.push(format!("trust the project at {root}"));
                summary.extend(approved.iter().map(approve_line));
                if git_shown {
                    summary.push("let git read the repository's own configuration".to_owned());
                }
                let next = Project {
                    key: key.clone(),
                    root,
                    trusted_at: self.clock.now().to_timestamp(),
                    revision,
                    git: git_shown.then_some(GitAnswer::Trusted),
                    declined_rules: None,
                    signature: None,
                    pinned: pins_of(&approved),
                };
                (next, lift_refusal, false)
            }
            TrustChange::Approve {
                approved,
                git_shown,
            } => {
                let Some(project) = signed else {
                    return Err(self.not_understood(
                        "the project is not trusted, so no definition of it can be approved"
                            .to_owned(),
                    ));
                };
                let mut next = project.clone();
                next.revision = revision;
                next.signature = None;
                for pin in pins_of(&approved) {
                    next.pinned
                        .retain(|old| !(old.kind == pin.kind && old.name == pin.name));
                    next.pinned.push(pin);
                }
                summary.extend(approved.iter().map(approve_line));
                let approves_rules = approved
                    .iter()
                    .any(|item| item.definition.kind == TrustResourceKind::Rules);
                if approves_rules {
                    next.declined_rules = None;
                }
                if git_shown && next.git != Some(GitAnswer::Trusted) {
                    next.git = Some(GitAnswer::Trusted);
                    summary.push("let git read the repository's own configuration".to_owned());
                }
                (next, false, approves_rules)
            }
            TrustChange::Drop { pins } => {
                let Some(project) = signed else {
                    return Err(self.not_understood(
                        "the project is not trusted, so no pin of it can be taken away".to_owned(),
                    ));
                };
                let mut next = project.clone();
                next.revision = revision;
                next.signature = None;
                for (kind, name) in &pins {
                    let before = next.pinned.len();
                    next.pinned
                        .retain(|pin| !(pin.kind == kind.as_str() && pin.name == *name));
                    if next.pinned.len() != before {
                        summary.push(format!(
                            "take away the approval of the {kind} `{}`",
                            name.escape_debug()
                        ));
                    }
                }
                if summary.is_empty() {
                    return Ok(None);
                }
                (next, false, false)
            }
        };
        Ok(Some(TrustProposal {
            key: key.clone(),
            base,
            statement: next.statement(),
            next,
            summary,
            lift_refusal,
            approves_rules,
        }))
    }

    /// Writes what `proposal` worked out, signed with `signature`, when the
    /// signature is `verifier`'s over exactly that and the project's entry
    /// is still what it was when the proposal was made. A trust lifts a
    /// never said of the project when the proposal says so, and otherwise is
    /// refused by one. Gives the project's trust after.
    ///
    /// # Errors
    ///
    /// [`TrustError::Moved`] when the project's entry changed since the
    /// proposal (ask again); [`TrustError::NotSigned`] when the signature is
    /// not `verifier`'s over the proposal; [`TrustError::NotUnderstood`]
    /// when the project was marked never while the question was open;
    /// otherwise when the file cannot be read, is not understood, or cannot
    /// be written. Nothing is written in any of these.
    pub fn commit(
        &self,
        proposal: &TrustProposal,
        signature: &Signature,
        verifier: &Verifier,
    ) -> Result<ProjectTrust, TrustError> {
        if !verifier.verify(&proposal.statement, signature) {
            return Err(TrustError::NotSigned {
                path: self.file.display().to_string(),
            });
        }
        self.update(|file| {
            let now = file
                .projects
                .iter()
                .find(|project| project.key == proposal.key);
            if now != proposal.base.as_ref() {
                return Err(TrustError::Moved {
                    path: self.file.display().to_string(),
                });
            }
            let refused = file
                .refused
                .iter()
                .any(|refused| refused.key == proposal.key);
            if refused && proposal.base.is_none() && !proposal.lift_refusal {
                return Err(self.not_understood(
                    "the project was marked never while it was asked about, so it is not \
                     trusted"
                        .to_owned(),
                ));
            }
            // Trusting a project the user had refused lifts the refusal.
            file.refused.retain(|refused| refused.key != proposal.key);
            let mut next = Project {
                signature: Some(*signature),
                ..proposal.next.clone()
            };
            if proposal.approves_rules {
                next.declined_rules = None;
            }
            file.projects.retain(|project| project.key != proposal.key);
            file.projects.push(next);
            Ok((trust_of(file.clone(), &proposal.key, Some(verifier)), true))
        })
    }

    /// Keeps trusting a project as it was trusted, without what is new in
    /// it (since the follow-ups of step 23, the owner's decision of
    /// 2026-11-16): `rules`, its rules as they are now, which stay held back
    /// and are not asked about while they are these; `git`, git, which does
    /// not read the repository's configuration and is not asked about
    /// again. What was trusted before (pins, git) is not touched. Neither is
    /// signed: each only holds something back, and the entry's signature is
    /// over neither.
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
    ) -> Result<(), TrustError> {
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
            // declined. So the signed part of the entry does not change.
            if git && project.git != Some(GitAnswer::Trusted) {
                project.git = Some(GitAnswer::Declined);
            }
            Ok(((), true))
        })
    }

    /// Marks a project as one the user will never trust: it is not asked
    /// about again, and any trust it had is taken away. `root` is for
    /// showing a person which project it is. Not signed: a project taken
    /// out of trust whole is asked about again as never trusted.
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

    /// Takes trust away from a project. Says whether it had an entry.
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

    /// The projects the store holds an entry for, as their key, the root
    /// they were trusted at, and how the entry stands with `verifier`, for
    /// `cineris permissions`.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood.
    pub fn list(
        &self,
        verifier: Option<&Verifier>,
    ) -> Result<Vec<(ProjectKey, String, Checked)>, TrustError> {
        Ok(self
            .read()?
            .projects
            .into_iter()
            .map(|project| {
                let checked = project.checked(verifier);
                (project.key, project.root, checked)
            })
            .collect())
    }

    /// Every hash that a project's entry pins for a definition of `kind`,
    /// of every project and whatever its name or signature (since step 28):
    /// what a store kept by hash and shared by projects, the registration
    /// cache of extensions, may keep. Keeping more than counts is the safe
    /// side: the cache only restricts what a pin that counts may use.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood. The caller keeps
    /// what it holds, rather than drop another project's.
    pub fn pinned_hashes(&self, kind: &TrustResourceKind) -> Result<Vec<ContentHash>, TrustError> {
        Ok(self
            .read()?
            .projects
            .into_iter()
            .flat_map(|project| project.pinned)
            .filter(|pin| pin.kind == kind.as_str())
            .map(|pin| pin.hash)
            .collect())
    }

    /// Takes the signature off every project's entry, at a reset of the
    /// person's key: each entry stays, unsigned, so that the project is
    /// trusted for nothing and asked about again, and its pins still hold
    /// their gates ([`ProjectTrust::held`]) until the person's next yes
    /// replaces the entry. Deletes nothing, so that a reset run by
    /// something other than the person lifts no gate. Refusals stay. Gives
    /// how many entries there are.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood, or cannot be
    /// written.
    pub fn strip_signatures(&self) -> Result<usize, TrustError> {
        self.update(|file| {
            let count = file.projects.len();
            let signed = file
                .projects
                .iter()
                .any(|project| project.signature.is_some());
            for project in &mut file.projects {
                project.signature = None;
            }
            Ok((count, signed))
        })
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
                "its version is {}, and this version of Cineris reads versions {OLDEST} \
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
        // Nor has a version before 3 a revision, a signature or a pin's
        // registration.
        if file.version < VERSION
            && file.projects.iter().any(|project| {
                project.revision != 0
                    || project.signature.is_some()
                    || project.pinned.iter().any(|pin| pin.registration.is_some())
            })
        {
            return Err(self.not_understood(format!(
                "it is version {}, which has no `revision`, `signature` or `registration`",
                file.version
            )));
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

/// The project `key`'s trust in `file`, as it stands with `verifier`.
fn trust_of(file: File, key: &ProjectKey, verifier: Option<&Verifier>) -> ProjectTrust {
    let refused = file.refused.iter().any(|refused| refused.key == *key);
    let project = file
        .projects
        .into_iter()
        .find(|project| project.key == *key);
    let checked = project
        .as_ref()
        .map_or(Checked::Empty, |project| project.checked(verifier));
    ProjectTrust {
        older: file.version < 2,
        checked,
        project,
        refused,
    }
}

/// One line of what a proposal approves.
fn approve_line(item: &Approved) -> String {
    let hash: String = item.definition.hash.to_hex().chars().take(12).collect();
    format!(
        "approve the {} `{}` ({hash})",
        item.definition.kind,
        item.definition.name.escape_debug()
    )
}

fn pins_of(approved: &[Approved]) -> Vec<Pin> {
    let mut pins: Vec<Pin> = Vec::with_capacity(approved.len());
    for item in approved {
        let pin = Pin {
            kind: item.definition.kind.as_str().to_owned(),
            name: item.definition.name.clone(),
            hash: item.definition.hash,
            registration: item.registration,
        };
        pins.retain(|old| !(old.kind == pin.kind && old.name == pin.name));
        pins.push(pin);
    }
    pins
}

#[cfg(test)]
mod tests {
    use cineris_protocol::lease::Effect;

    use super::*;
    use crate::test_support::Person;

    fn hook(name: &str, parts: &[&str]) -> Definition {
        Definition::of(TrustResourceKind::Hook, name, parts.iter().copied())
    }

    /// A project's trust as a session that checked its signature holds it.
    fn trusted_with(definitions: &[Definition]) -> ProjectTrust {
        let approved: Vec<Approved> = definitions.iter().cloned().map(Into::into).collect();
        ProjectTrust {
            project: Some(Project {
                key: ProjectKey::new("k"),
                root: String::new(),
                trusted_at: Timestamp::new("2026-11-13T10:00:00Z"),
                revision: 1,
                git: None,
                declined_rules: None,
                signature: None,
                pinned: pins_of(&approved),
            }),
            checked: Checked::Signed,
            refused: false,
            older: false,
        }
    }

    fn store_in(scratch: &crate::store_lock::tests::Scratch) -> TrustStore {
        let clock: SharedClock = std::sync::Arc::new(
            cineris_base::FakeClock::starting_at("2026-11-27T10:00:00Z").unwrap(),
        );
        TrustStore::new(scratch.0.join("trust.toml"), clock)
    }

    #[test]
    fn a_refused_project_is_known_as_such_until_trusted_or_forgotten() {
        let dir =
            std::env::temp_dir().join(format!("cineris-policy-refused-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let clock: SharedClock = std::sync::Arc::new(
            cineris_base::FakeClock::starting_at("2026-11-15T10:00:00Z").unwrap(),
        );
        let store = TrustStore::new(dir.join("trust.toml"), clock);
        let key = ProjectKey::new("k");
        let person = Person::new();
        let mine = Some(person.verifier());
        assert!(!store.project(&key, mine.as_ref()).unwrap().is_refused());
        // Refusing takes trust away.
        person.trust(&store, &key, "~/app", &[], false);
        store.refuse(&key, "~/app").unwrap();
        let refused = store.project(&key, mine.as_ref()).unwrap();
        assert!(refused.is_refused() && !refused.is_trusted());
        assert_eq!(store.refused().unwrap().len(), 1);
        // It survives a new store over the same file: a restart.
        let again = TrustStore::new(
            store.path().to_owned(),
            std::sync::Arc::new(cineris_base::SystemClock::new()),
        );
        assert!(again.project(&key, mine.as_ref()).unwrap().is_refused());
        // Trusting lifts it.
        person.trust(&again, &key, "~/app", &[], false);
        let trusted = again.project(&key, mine.as_ref()).unwrap();
        assert!(trusted.is_trusted() && !trusted.is_refused());
        // Forgetting clears both.
        again.refuse(&key, "~/app").unwrap();
        assert!(again.forget(&key).unwrap());
        let forgotten = again.project(&key, mine.as_ref()).unwrap();
        assert!(!forgotten.is_trusted() && !forgotten.is_refused());
        assert!(!again.forget(&key).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hashes_pinned_of_a_kind_are_every_trusted_projects() {
        use crate::store_lock::tests::Scratch;

        let scratch = Scratch::new();
        let clock: SharedClock = std::sync::Arc::new(
            cineris_base::FakeClock::starting_at("2026-11-21T10:00:00Z").unwrap(),
        );
        let store = TrustStore::new(scratch.0.join("trust.toml"), clock);
        let person = Person::new();
        let extension =
            |name: &str, text: &str| Definition::of(TrustResourceKind::Extension, name, [text]);
        let (one, other, gone) = (
            extension("tracker", "one"),
            extension("tracker", "other"),
            extension("deploy", "gone"),
        );
        let (first, second, third) = (
            ProjectKey::new("first"),
            ProjectKey::new("second"),
            ProjectKey::new("third"),
        );
        person.trust(
            &store,
            &first,
            "~/a",
            &[one.clone(), hook("format", &["x"])],
            false,
        );
        person.trust(&store, &second, "~/b", std::slice::from_ref(&other), false);
        person.trust(&store, &third, "~/c", std::slice::from_ref(&gone), false);
        // A project whose trust is taken away pins nothing.
        store.revoke(&third).unwrap();
        let mut pinned = store.pinned_hashes(&TrustResourceKind::Extension).unwrap();
        pinned.sort();
        let mut expected = vec![one.hash, other.hash];
        expected.sort();
        assert_eq!(
            pinned, expected,
            "of every trusted project, of the kind asked"
        );
        assert_eq!(
            store.pinned_hashes(&TrustResourceKind::Hook).unwrap(),
            [hook("format", &["x"]).hash]
        );
        // A file that is not understood says so: nothing is taken as pinned
        // by a caller that would drop what it holds.
        std::fs::write(store.path(), "version = 9\n").unwrap();
        assert!(store.pinned_hashes(&TrustResourceKind::Extension).is_err());
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
        use cineris_protocol::lease::RuleAction;
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
        use cineris_base::FakeClock;

        use crate::store_lock::tests::{Scratch, is_held};

        let scratch = Scratch::new();
        let file = scratch.0.join("trust.toml");
        let clock: SharedClock =
            std::sync::Arc::new(FakeClock::starting_at("2026-11-13T10:00:00Z").unwrap());
        let store = TrustStore::new(&file, clock);
        let key = ProjectKey::new("k");
        let person = Person::new();
        let mine = person.verifier();
        // Every public change goes through `update`, and `update` holds the
        // lock from the read to the write.
        let held = store.update(|_| Ok((is_held(&file), false))).unwrap();
        assert!(held, "the lock is held while the file is changed");
        assert!(!is_held(&file), "and let go after");
        person.trust(&store, &key, "~/x", &[], false);
        person.approve(&store, &key, &[hook("h", &["x"])], false);
        assert!(store.revoke(&key).unwrap());
        assert!(!store.project(&key, Some(&mine)).unwrap().is_trusted());
        assert!(!is_held(&file));

        // A lock that cannot be taken stops every change, and no read: so
        // each change takes it. (A directory stands where the lock goes.)
        person.trust(&store, &key, "~/x", &[], false);
        let approve = store
            .propose(
                &key,
                TrustChange::Approve {
                    approved: vec![hook("h", &["y"]).into()],
                    git_shown: false,
                },
                Some(&mine),
            )
            .unwrap()
            .unwrap();
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
        refused(
            store
                .commit(&approve, &person.sign(approve.statement()), &mine)
                .map(drop),
        );
        refused(store.keep_without(&key, None, true));
        refused(store.revoke(&key).map(drop));
        assert!(store.project(&key, Some(&mine)).unwrap().is_trusted());
        assert_eq!(store.list(Some(&mine)).unwrap().len(), 1);
    }

    #[test]
    fn a_project_counts_only_with_a_signature_by_the_sessions_key() {
        let scratch = crate::store_lock::tests::Scratch::new();
        let store = store_in(&scratch);
        let key = ProjectKey::new("k");
        let person = Person::new();
        let model = Person::with_password("the model's own password");
        let format = hook("format", &["cargo", "fmt"]);
        person.trust(&store, &key, "~/app", std::slice::from_ref(&format), true);
        let mine = store.project(&key, Some(&person.verifier())).unwrap();
        assert!(mine.is_trusted() && mine.git_trusted());
        assert_eq!(mine.standing(&format), Standing::Approved);

        // With no key, nothing is trusted, and its pins still restrict.
        let keyless = store.project(&key, None).unwrap();
        assert!(!keyless.is_trusted() && !keyless.git_trusted());
        assert_eq!(keyless.checked(), Checked::NoKey);
        assert_eq!(keyless.standing(&format), Standing::Untrusted);
        assert_eq!(keyless.held(&TrustResourceKind::Hook).len(), 1);
        assert!(keyless.has_entry());

        // Another key's session: not theirs.
        let theirs = store.project(&key, Some(&model.verifier())).unwrap();
        assert_eq!(theirs.checked(), Checked::NotTheirs);
        assert!(!theirs.is_trusted());

        // The model writes an entry of its own, signed by its own key: the
        // person's session trusts nothing of it.
        let other = ProjectKey::new("other");
        model.trust(&store, &other, "~/evil", &[hook("x", &["curl evil"])], true);
        let forged = store.project(&other, Some(&person.verifier())).unwrap();
        assert!(!forged.is_trusted());
        assert_eq!(forged.checked(), Checked::NotTheirs);
    }

    #[test]
    fn an_entry_changed_by_hand_or_copied_to_another_project_counts_for_nothing() {
        let scratch = crate::store_lock::tests::Scratch::new();
        let store = store_in(&scratch);
        let key = ProjectKey::new("k");
        let person = Person::new();
        let mine = person.verifier();
        let gate = hook("gate", &["refuse", "git push"]);
        let format = hook("format", &["cargo", "fmt"]);
        person.trust(
            &store,
            &key,
            "~/app",
            &[gate.clone(), format.clone()],
            false,
        );
        let written = std::fs::read_to_string(store.path()).unwrap();

        let check = |text: String, what: &str| {
            std::fs::write(store.path(), &text).unwrap();
            let trust = store.project(&key, Some(&mine));
            let copied = store.project(&ProjectKey::new("copy"), Some(&mine));
            match (trust, copied) {
                (Ok(trust), Ok(copied)) => assert!(
                    !trust.is_trusted() && !copied.is_trusted(),
                    "{what}: still trusted\n{text}"
                ),
                (trust, copied) => {
                    assert!(trust.is_err() || copied.is_err(), "{what}");
                }
            }
        };
        // A pin taken out: the gate's pin, so that its gate would lapse.
        let without_gate = {
            let start = written.find("[[project.pinned]]").unwrap();
            let gate_at = written[start..].find("name = \"gate\"").unwrap() + start;
            let block = written[..gate_at].rfind("[[project.pinned]]").unwrap();
            let next = written[gate_at..]
                .find("[[project.pinned]]")
                .map_or(written.len(), |at| at + gate_at);
            format!("{}{}", &written[..block], &written[next..])
        };
        assert!(!without_gate.contains("gate"), "{without_gate}");
        check(without_gate, "a pin taken out");
        // A hash changed to the model's.
        check(
            written.replace(
                &format.hash.to_hex(),
                &hook("format", &["curl evil"]).hash.to_hex(),
            ),
            "a hash changed",
        );
        // Git turned on.
        check(
            written.replace("revision = 1", "revision = 1\ngit = \"trusted\""),
            "git turned on",
        );
        // The whole entry copied to another project's key.
        check(
            written.replace("key = \"k\"", "key = \"copy\""),
            "copied to another project",
        );
        // The revision moved, or the root changed.
        check(
            written.replace("revision = 1", "revision = 2"),
            "the revision moved",
        );
        check(written.replace("~/app", "~/other"), "the root changed");
        // The signature taken off: unsigned.
        let unsigned: String = written
            .lines()
            .filter(|line| !line.starts_with("signature"))
            .map(|line| format!("{line}\n"))
            .collect();
        std::fs::write(store.path(), &unsigned).unwrap();
        assert_eq!(
            store.project(&key, Some(&mine)).unwrap().checked(),
            Checked::Unsigned
        );
        // And as written, it counts.
        std::fs::write(store.path(), &written).unwrap();
        assert!(store.project(&key, Some(&mine)).unwrap().is_trusted());
    }

    #[test]
    fn a_store_of_an_older_version_is_read_as_unsigned_and_keeps_its_refusals() {
        let scratch = crate::store_lock::tests::Scratch::new();
        let store = store_in(&scratch);
        std::fs::create_dir_all(&scratch.0).unwrap();
        std::fs::write(
            store.path(),
            "version = 2\n\n[[project]]\nkey = \"k\"\nroot = \"~/app\"\ntrusted_at = \
             \"2026-11-13T10:00:00Z\"\ngit = \"trusted\"\n\n[[project.pinned]]\nkind = \
             \"hook\"\nname = \"format\"\nhash = \"{}\"\n\n[[refused]]\nkey = \"no\"\nroot = \
             \"~/no\"\nrefused_at = \"2026-11-13T10:00:00Z\"\n"
                .replace("{}", &hook("format", &["x"]).hash.to_hex()),
        )
        .unwrap();
        let person = Person::new();
        let mine = person.verifier();
        let old = store.project(&ProjectKey::new("k"), Some(&mine)).unwrap();
        assert!(!old.is_trusted() && !old.git_trusted());
        assert_eq!(old.checked(), Checked::Unsigned);
        assert_eq!(
            old.held(&TrustResourceKind::Hook).len(),
            1,
            "its pins restrict"
        );
        assert!(
            store
                .project(&ProjectKey::new("no"), Some(&mine))
                .unwrap()
                .is_refused()
        );
        // An older version that says it signed is not one a build wrote.
        std::fs::write(
            store.path(),
            "version = 2\n\n[[project]]\nkey = \"k\"\ntrusted_at = \"2026-11-13T10:00:00Z\"\n\
             revision = 1\n",
        )
        .unwrap();
        assert!(matches!(
            store.project(&ProjectKey::new("k"), Some(&mine)),
            Err(TrustError::NotUnderstood { .. })
        ));
    }

    #[test]
    fn an_approval_needs_a_trusted_project_and_its_entry_as_it_was() {
        let scratch = crate::store_lock::tests::Scratch::new();
        let store = store_in(&scratch);
        let key = ProjectKey::new("k");
        let person = Person::new();
        let mine = person.verifier();
        let approve = |definition: Definition| TrustChange::Approve {
            approved: vec![definition.into()],
            git_shown: false,
        };
        // Not trusted: approving one definition must not make it so.
        assert!(matches!(
            store.propose(&key, approve(hook("a", &["x"])), Some(&mine)),
            Err(TrustError::NotUnderstood { .. })
        ));
        person.trust(&store, &key, "~/app", &[], false);
        let proposal = store
            .propose(&key, approve(hook("a", &["x"])), Some(&mine))
            .unwrap()
            .unwrap();
        // The entry changes in between: the answer is not used.
        person.approve(&store, &key, &[hook("b", &["y"])], false);
        assert!(matches!(
            store.commit(&proposal, &person.sign(proposal.statement()), &mine),
            Err(TrustError::Moved { .. })
        ));
        let trust = store.project(&key, Some(&mine)).unwrap();
        assert_eq!(trust.standing(&hook("a", &["x"])), Standing::New);
        assert_eq!(trust.standing(&hook("b", &["y"])), Standing::Approved);
        assert_eq!(trust.revision(), 2);

        // A drop is signed too, and takes only what it names.
        let drop = store
            .propose(
                &key,
                TrustChange::Drop {
                    pins: vec![(TrustResourceKind::Hook, "b".to_owned())],
                },
                Some(&mine),
            )
            .unwrap()
            .unwrap();
        let model = Person::with_password("the model's own password");
        assert!(matches!(
            store.commit(&drop, &model.sign(drop.statement()), &mine),
            Err(TrustError::NotSigned { .. })
        ));
        store
            .commit(&drop, &person.sign(drop.statement()), &mine)
            .unwrap();
        let trust = store.project(&key, Some(&mine)).unwrap();
        assert_eq!(trust.standing(&hook("b", &["y"])), Standing::New);
        // A drop of what is not pinned asks nobody.
        assert!(
            store
                .propose(
                    &key,
                    TrustChange::Drop {
                        pins: vec![(TrustResourceKind::Hook, "none".to_owned())],
                    },
                    Some(&mine),
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_first_trust_is_refused_by_a_never_said_while_it_was_asked() {
        let scratch = crate::store_lock::tests::Scratch::new();
        let store = store_in(&scratch);
        let key = ProjectKey::new("k");
        let person = Person::new();
        let mine = person.verifier();
        let first = store
            .propose(
                &key,
                TrustChange::Trust {
                    root: "~/app".to_owned(),
                    approved: Vec::new(),
                    git_shown: false,
                    lift_refusal: false,
                },
                Some(&mine),
            )
            .unwrap()
            .unwrap();
        store.refuse(&key, "~/app").unwrap();
        assert!(matches!(
            store.commit(&first, &person.sign(first.statement()), &mine),
            Err(TrustError::NotUnderstood { .. })
        ));
        assert!(!store.project(&key, Some(&mine)).unwrap().is_trusted());
        // The person's own command lifts it.
        person.trust(&store, &key, "~/app", &[], false);
        let trust = store.project(&key, Some(&mine)).unwrap();
        assert!(trust.is_trusted() && !trust.is_refused());
    }

    /// A first question is put before the person opens their approvals: a
    /// trust is worked out with no key, and signed and committed once they
    /// have. What keeps the entry's pins needs the key it is checked with.
    #[test]
    fn a_trust_is_proposed_without_a_key_and_an_approval_or_a_drop_is_not() {
        let scratch = crate::store_lock::tests::Scratch::new();
        let store = store_in(&scratch);
        let key = ProjectKey::new("k");
        let person = Person::new();
        let mine = person.verifier();
        let first = store
            .propose(
                &key,
                TrustChange::Trust {
                    root: "~/app".to_owned(),
                    approved: vec![hook("a", &["x"]).into()],
                    git_shown: false,
                    lift_refusal: false,
                },
                None,
            )
            .unwrap()
            .unwrap();
        store
            .commit(&first, &person.sign(first.statement()), &mine)
            .unwrap();
        assert!(store.project(&key, Some(&mine)).unwrap().is_trusted());
        for change in [
            TrustChange::Approve {
                approved: vec![hook("b", &["y"]).into()],
                git_shown: false,
            },
            TrustChange::Drop {
                pins: vec![(TrustResourceKind::Hook, "a".to_owned())],
            },
        ] {
            assert!(matches!(
                store.propose(&key, change, None),
                Err(TrustError::NotUnderstood { .. })
            ));
        }
        // An entry there already is replaced, and the summary says so
        // without saying whose it was.
        let again = store
            .propose(
                &key,
                TrustChange::Trust {
                    root: "~/app".to_owned(),
                    approved: Vec::new(),
                    git_shown: false,
                    lift_refusal: false,
                },
                None,
            )
            .unwrap()
            .unwrap();
        assert_eq!(again.summary()[0], "its earlier trust is replaced");
    }

    #[test]
    fn a_proposal_shown_to_a_front_end_is_signed_as_the_store_checks_it() {
        let scratch = crate::store_lock::tests::Scratch::new();
        let store = store_in(&scratch);
        let key = ProjectKey::new("k");
        let person = Person::new();
        let mine = person.verifier();
        let proposal = store
            .propose(
                &key,
                TrustChange::Trust {
                    root: "~/app".to_owned(),
                    approved: vec![
                        Approved {
                            definition: hook("b", &["x"]),
                            registration: Some(ContentHash::of(b"what it registered")),
                        },
                        hook("a", &["y"]).into(),
                    ],
                    git_shown: true,
                    lift_refusal: false,
                },
                Some(&mine),
            )
            .unwrap()
            .unwrap();
        // The bytes a front end makes from what it is shown are the store's.
        let rebuilt = crate::signed::statement_of(&proposal.view()).unwrap();
        assert_eq!(&rebuilt, proposal.statement());
        store
            .commit(&proposal, &person.sign(&rebuilt), &mine)
            .unwrap();
        assert!(store.project(&key, Some(&mine)).unwrap().git_trusted());
        // A view with a pin the front end did not show signs other bytes.
        let mut padded = proposal.view();
        if let ApprovalProposal::Project { pins, .. } = &mut padded {
            pins.push(ProposedPin {
                kind: TrustResourceKind::Hook,
                name: "slipped in".to_owned(),
                hash: ContentHash::of(b"x").to_hex(),
                registration: None,
            });
        }
        assert_ne!(
            &crate::signed::statement_of(&padded).unwrap(),
            proposal.statement()
        );
        // A view with a pin twice, or a hash that is not one, makes nothing.
        let mut twice = proposal.view();
        if let ApprovalProposal::Project { pins, .. } = &mut twice {
            let first = pins[0].clone();
            pins.push(first);
        }
        assert!(crate::signed::statement_of(&twice).is_err());
        let mut bad = proposal.view();
        if let ApprovalProposal::Project { pins, .. } = &mut bad {
            pins[0].hash = "XYZ".to_owned();
        }
        assert!(crate::signed::statement_of(&bad).is_err());
    }
}
