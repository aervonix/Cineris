//! One entry of the project profile or memory
//! (`docs/architecture/14-adaptation.md`, "Entries").
//!
//! An [`Entry`] is what both files hold once they are read: the private
//! store's entries as they are written, and the shared file's with the
//! standing this user gave each one. Everything that decides whether an
//! entry is used is here, in three questions:
//!
//! | Question | Asked by | True when |
//! |---|---|---|
//! | [`Entry::in_use`] | The prompt, verification | It is active and not past its expiry. |
//! | [`Entry::is_accepted`] | The safe list | The user accepted it, or wrote it in their own store. |
//! | [`Entry::runs_unasked`] | The safe list | It is a command, in use and accepted. |
//!
//! An entry that became active by itself (a command seen to succeed twice)
//! is in use and not accepted: it is in the prompt, and it still asks.

use std::time::Duration;

use cineris_base::{ContentHash, ContentHasher, UtcTime};
use cineris_protocol::ids::{EntryId, ItemId, SessionId};
use cineris_protocol::project as wire;

/// The longest text of an entry, in characters: one or two sentences. The
/// limit the `remember` tool holds a proposal to.
pub const MAX_TEXT: usize = 600;

/// The longest evidence, command line, pattern or path, in characters.
pub const MAX_DETAIL: usize = 400;

/// The longest id of an entry, in characters.
pub const MAX_ID: usize = 64;

const DAY: u64 = 24 * 60 * 60;

/// What an entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// A named command: build, test, check, format, run.
    Command,
    /// A rule in a sentence or two.
    Convention,
    /// A test that fails intermittently.
    Flaky,
    /// Anything else worth keeping.
    Fact,
}

impl Kind {
    /// Every kind, in the order the prompt lists them.
    pub const ALL: [Self; 4] = [Self::Command, Self::Convention, Self::Flaky, Self::Fact];

    /// The kind as the files and the protocol write it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Convention => "convention",
            Self::Flaky => "flaky",
            Self::Fact => "fact",
        }
    }

    /// The kind a file names, or `None` for a name this build does not
    /// know.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }

    /// How long a learned entry of this kind is used after it was last
    /// confirmed: commands 90 days, flaky tests 30, facts and conventions
    /// 60. An entry the user wrote does not expire.
    #[must_use]
    pub fn lifetime(self) -> Duration {
        let days = match self {
            Self::Command => 90,
            Self::Flaky => 30,
            Self::Convention | Self::Fact => 60,
        };
        Duration::from_secs(days * DAY)
    }

    fn to_wire(self) -> wire::EntryKind {
        match self {
            Self::Command => wire::EntryKind::Command,
            Self::Convention => wire::EntryKind::Convention,
            Self::Flaky => wire::EntryKind::Flaky,
            Self::Fact => wire::EntryKind::Fact,
        }
    }

    /// The kind the protocol names, or `None` for one this build does not
    /// know.
    #[must_use]
    pub fn from_wire(kind: &wire::EntryKind) -> Option<Self> {
        Self::parse(kind.as_str())
    }
}

/// Whether an entry is in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    /// Waiting for the user to accept it, or for a second observation.
    /// Never in the prompt.
    Proposed,
    /// In use, until its expiry.
    Active,
    /// Kept, and not in use.
    Disabled,
}

impl Status {
    /// The status as the files and the protocol write it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    /// The status a file names.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Proposed, Self::Active, Self::Disabled]
            .into_iter()
            .find(|status| status.as_str() == text)
    }

    fn to_wire(self) -> wire::EntryStatus {
        match self {
            Self::Proposed => wire::EntryStatus::Proposed,
            Self::Active => wire::EntryStatus::Active,
            Self::Disabled => wire::EntryStatus::Disabled,
        }
    }
}

/// How the harness came to hold a learned entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Basis {
    /// The harness saw it happen.
    Observed,
    /// A model inferred it.
    Proposed,
}

impl Basis {
    /// The basis as the files and the protocol write it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::Proposed => "proposed",
        }
    }

    /// The basis a file names.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Observed, Self::Proposed]
            .into_iter()
            .find(|basis| basis.as_str() == text)
    }
}

/// What a command entry is for. The order is the order the prompt lists
/// commands in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Purpose {
    /// Builds the project.
    Build,
    /// Runs its tests.
    Test,
    /// Runs its checks: lints, types.
    Check,
    /// Formats it.
    Format,
    /// Runs it.
    Run,
}

impl Purpose {
    /// Every purpose, in order.
    pub const ALL: [Self; 5] = [
        Self::Build,
        Self::Test,
        Self::Check,
        Self::Format,
        Self::Run,
    ];

    /// The purpose as the files and the protocol write it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Test => "test",
            Self::Check => "check",
            Self::Format => "format",
            Self::Run => "run",
        }
    }

    /// The purpose a file names.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|purpose| purpose.as_str() == text)
    }

    /// The purpose as the protocol has it.
    #[must_use]
    pub fn to_wire(self) -> wire::CommandPurpose {
        match self {
            Self::Build => wire::CommandPurpose::Build,
            Self::Test => wire::CommandPurpose::Test,
            Self::Check => wire::CommandPurpose::Check,
            Self::Format => wire::CommandPurpose::Format,
            Self::Run => wire::CommandPurpose::Run,
        }
    }
}

/// Which file an entry lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Scope {
    /// This user's private store for the project.
    Private,
    /// The project's shared profile, in the repository.
    Shared,
}

/// Where and when a learned entry was learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Learned {
    /// The session.
    pub session: Option<SessionId>,
    /// The item of that session.
    pub item: Option<ItemId>,
    /// When.
    pub at: UtcTime,
}

/// The command of an entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// What it is for.
    pub purpose: Purpose,
    /// The command line.
    pub run: String,
    /// The directory it runs in, relative to the workspace root, with `/`.
    /// `None` is the root.
    pub cwd: Option<String>,
    /// How long it usually takes, in milliseconds.
    pub usual_ms: Option<u64>,
    /// Whether it only reads.
    pub read_only: bool,
}

/// A cheap, read-only test that shows an entry still holds. It is stored
/// and shown; nothing here runs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// The path exists.
    Exists {
        /// The path, relative to the workspace root.
        path: String,
    },
    /// A search in the file or folder finds the pattern.
    Contains {
        /// The path, relative to the workspace root.
        path: String,
        /// The pattern.
        pattern: String,
    },
    /// The command exits with success. The text is whoever wrote the
    /// entry's, unverified.
    Command {
        /// The command line.
        command: String,
    },
}

impl Check {
    /// The check as the protocol writes it.
    #[must_use]
    pub fn to_wire(&self) -> wire::EntryCheck {
        let (kind, path, pattern, command) = match self {
            Self::Exists { path } => (wire::EntryCheckKind::Exists, Some(path), None, None),
            Self::Contains { path, pattern } => (
                wire::EntryCheckKind::Contains,
                Some(path),
                Some(pattern),
                None,
            ),
            Self::Command { command } => (wire::EntryCheckKind::Command, None, None, Some(command)),
        };
        wire::EntryCheck {
            kind,
            path: path.cloned(),
            pattern: pattern.cloned(),
            command: command.cloned(),
        }
    }

    /// The check the protocol names.
    ///
    /// # Errors
    ///
    /// A kind this build does not know, a part the kind needs that is
    /// missing, or one it does not take.
    pub fn from_wire(check: &wire::EntryCheck) -> Result<Self, String> {
        let parts = (&check.path, &check.pattern, &check.command);
        match (&check.kind, parts) {
            (wire::EntryCheckKind::Exists, (Some(path), None, None)) => {
                Ok(Self::Exists { path: path.clone() })
            }
            (wire::EntryCheckKind::Contains, (Some(path), Some(pattern), None)) => {
                Ok(Self::Contains {
                    path: path.clone(),
                    pattern: pattern.clone(),
                })
            }
            (wire::EntryCheckKind::Command, (None, None, Some(command))) => Ok(Self::Command {
                command: command.clone(),
            }),
            (kind, _) => Err(format!(
                "a check of kind `{}` takes `path` for `exists`, `path` and `pattern` for \
                 `contains`, and `command` for `command`, and nothing else",
                kind.as_str()
            )),
        }
    }
}

/// One entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its id: `k_<n>` in the private store, `s_<hash>` in the shared file.
    pub id: EntryId,
    /// Which file it lives in.
    pub scope: Scope,
    /// What it is.
    pub kind: Kind,
    /// The entry in words.
    pub text: String,
    /// Whether it is in use. For a shared entry, what this user decided.
    pub status: Status,
    /// How the harness came to hold it. `None`: a person wrote it.
    pub basis: Option<Basis>,
    /// Where it was learned. `None`: a person wrote it.
    pub learned: Option<Learned>,
    /// Where it was learned, in words.
    pub evidence: Option<String>,
    /// When it was last seen to hold.
    pub confirmed: Option<UtcTime>,
    /// When it stops being used unless confirmed again.
    pub expires: Option<UtcTime>,
    /// When the user accepted it.
    pub accepted: Option<UtcTime>,
    /// When it was taken out of use for being past its expiry. An entry
    /// with this is disabled, and a new observation or the user's `accept`
    /// brings it back; one the user disabled has none, and is not learned
    /// again.
    pub lapsed: Option<UtcTime>,
    /// For an observed entry: how many times the harness saw it hold.
    pub seen: u32,
    /// For a `command` entry, and a `flaky` one that names the command
    /// that flipped: the command.
    pub command: Option<Command>,
    /// A test that shows it still holds.
    pub check: Option<Check>,
}

impl Entry {
    /// An entry a person wrote, active at once and never expiring.
    #[must_use]
    pub fn written(id: EntryId, scope: Scope, kind: Kind, text: impl Into<String>) -> Self {
        Self {
            id,
            scope,
            kind,
            text: text.into(),
            status: Status::Active,
            basis: None,
            learned: None,
            evidence: None,
            confirmed: None,
            expires: None,
            accepted: None,
            lapsed: None,
            seen: 0,
            command: None,
            check: None,
        }
    }

    /// Whether the entry is past its expiry at `now`.
    #[must_use]
    pub fn is_expired(&self, now: UtcTime) -> bool {
        self.expires.is_some_and(|at| at <= now)
    }

    /// Whether the entry is used at `now`: it is active and not past its
    /// expiry. Only such an entry reaches the prompt.
    #[must_use]
    pub fn in_use(&self, now: UtcTime) -> bool {
        self.status == Status::Active && !self.is_expired(now)
    }

    /// Whether the user stands behind the entry: they accepted it, or it is
    /// one they wrote in their own store. An entry of the shared file is
    /// never accepted for having been written: a repository's file is not
    /// the user's word.
    #[must_use]
    pub fn is_accepted(&self) -> bool {
        self.accepted.is_some() || (self.scope == Scope::Private && self.learned.is_none())
    }

    /// Whether the entry's command may run without asking at `now`, as far
    /// as the profile is concerned: a command entry, in use and accepted.
    /// The permission engine still holds the rule back until the project
    /// is trusted.
    #[must_use]
    pub fn runs_unasked(&self, now: UtcTime) -> bool {
        self.kind == Kind::Command
            && self.command.is_some()
            && self.in_use(now)
            && self.is_accepted()
    }

    /// The hash of what the entry says: its kind, its text, its command
    /// and its check. It is what a user's acceptance of a shared entry is
    /// tied to, so that a shared entry that changes is a proposal again.
    /// How long a command usually takes is not part of it.
    #[must_use]
    pub fn content_hash(&self) -> ContentHash {
        let mut hasher = ContentHasher::new();
        let mut part = |text: &str| {
            hasher.update(&(text.len() as u64).to_le_bytes());
            hasher.update(text.as_bytes());
        };
        part("cineris-knowledge entry 1");
        part(self.kind.as_str());
        part(&one_spaced(&self.text));
        match &self.command {
            Some(command) => {
                part("command");
                part(command.purpose.as_str());
                part(&command.run);
                part(command.cwd.as_deref().unwrap_or(""));
                part(if command.read_only { "reads" } else { "writes" });
            }
            None => part("no command"),
        }
        match &self.check {
            Some(Check::Exists { path }) => {
                part("exists");
                part(path);
            }
            Some(Check::Contains { path, pattern }) => {
                part("contains");
                part(path);
                part(pattern);
            }
            Some(Check::Command { command }) => {
                part("check command");
                part(command);
            }
            None => part("no check"),
        }
        hasher.finish()
    }

    /// What is wrong with the entry, if anything is: a text that is empty
    /// or too long, a command where the kind takes none or none where it
    /// needs one, a path that leaves the workspace.
    ///
    /// # Errors
    ///
    /// The first thing that is wrong, in a sentence.
    pub fn validate(&self) -> Result<(), String> {
        within("id", self.id.as_str(), MAX_ID)?;
        if !self
            .id
            .as_str()
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        {
            return Err("`id` may hold letters, digits, `_` and `-` only".to_owned());
        }
        within("text", &self.text, MAX_TEXT)?;
        if let Some(evidence) = &self.evidence {
            within("evidence", evidence, MAX_DETAIL)?;
        }
        match (self.kind, &self.command) {
            (Kind::Command, None) => {
                return Err("an entry of kind `command` needs a `command`".to_owned());
            }
            (Kind::Convention | Kind::Fact, Some(_)) => {
                return Err(format!(
                    "an entry of kind `{}` takes no `command`",
                    self.kind.as_str()
                ));
            }
            _ => {}
        }
        if let Some(command) = &self.command {
            within("command.run", &command.run, MAX_DETAIL)?;
            if command.run.chars().any(char::is_control) {
                return Err("`command.run` is one line, with no control character".to_owned());
            }
            if let Some(cwd) = &command.cwd {
                inside("command.cwd", cwd)?;
            }
        }
        match &self.check {
            Some(Check::Exists { path }) => inside("check.exists", path)?,
            Some(Check::Contains { path, pattern }) => {
                inside("check.contains.path", path)?;
                one_line("check.contains.pattern", pattern)?;
            }
            Some(Check::Command { command }) => one_line("check.command", command)?,
            None => {}
        }
        if self.basis.is_some() != self.learned.is_some() {
            return Err(
                "`basis` and `learned` go together: an entry the harness learned has both, and \
                 one a person wrote has neither"
                    .to_owned(),
            );
        }
        if self.lapsed.is_some() && self.status != Status::Disabled {
            return Err("`lapsed` is for an entry whose `status` is `disabled`".to_owned());
        }
        Ok(())
    }

    /// The entry as the protocol shows it.
    #[must_use]
    pub fn to_wire(&self) -> wire::ProjectEntry {
        let source = match &self.learned {
            Some(learned) => wire::EntrySource {
                kind: wire::EntrySourceKind::Learned,
                session_id: learned.session.clone(),
                item_id: learned.item.clone(),
                at: Some(learned.at.to_timestamp()),
            },
            None => wire::EntrySource {
                kind: wire::EntrySourceKind::User,
                session_id: None,
                item_id: None,
                at: None,
            },
        };
        wire::ProjectEntry {
            id: self.id.clone(),
            kind: self.kind.to_wire(),
            text: self.text.clone(),
            command: self.command.as_ref().map(|command| wire::EntryCommand {
                purpose: command.purpose.to_wire(),
                command: command.run.clone(),
                cwd: command.cwd.clone(),
                usual_duration_ms: command.usual_ms,
                read_only: command.read_only,
            }),
            evidence: self.evidence.clone(),
            source,
            basis: self.basis.map(|basis| match basis {
                Basis::Observed => wire::EntryBasis::Observed,
                Basis::Proposed => wire::EntryBasis::Proposed,
            }),
            status: self.status.to_wire(),
            scope: match self.scope {
                Scope::Private => wire::EntryScope::Private,
                Scope::Shared => wire::EntryScope::Shared,
            },
            confirmed_at: self.confirmed.map(UtcTime::to_timestamp),
            expires_at: self.expires.map(UtcTime::to_timestamp),
            check: self.check.as_ref().map(Check::to_wire),
            accepted_at: self.accepted.map(UtcTime::to_timestamp),
            observations: (self.basis == Some(Basis::Observed)).then_some(self.seen),
        }
    }
}

/// The command the protocol names, for an entry a user writes.
///
/// # Errors
///
/// A purpose this build does not know.
pub fn command_from_wire(command: &wire::NewCommand) -> Result<Command, String> {
    let purpose = Purpose::parse(command.purpose.as_str()).ok_or_else(|| {
        format!(
            "`{}` is not a purpose a command has: build, test, check, format or run",
            command.purpose.as_str()
        )
    })?;
    Ok(Command {
        purpose,
        run: command.command.trim().to_owned(),
        cwd: checked_dir(command.cwd.as_deref())?,
        usual_ms: None,
        read_only: command.read_only,
    })
}

/// A directory relative to the workspace root as an entry keeps it: with
/// `/`, without `./` at its start or a separator at its end, and `None`
/// for the root itself.
#[must_use]
pub fn normal_dir(dir: Option<&str>) -> Option<String> {
    let dir = dir?.trim().replace('\\', "/");
    let dir = dir.trim_end_matches('/');
    let mut names: Vec<&str> = dir.split('/').collect();
    names.retain(|name| !name.is_empty() && *name != ".");
    if names.is_empty() {
        None
    } else {
        Some(names.join("/"))
    }
}

/// A text with its ends trimmed and every run of blanks made one space:
/// how two texts are compared, how a new one is kept, and how a text is
/// hashed, so that a line ending a checkout changed is not another entry.
pub(crate) fn one_spaced(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A directory an entry's command runs in, as a person or a client wrote
/// it, made the one way an entry keeps it ([`normal_dir`]).
///
/// # Errors
///
/// A directory that names a place by its root, a drive or the home
/// directory: it is refused as written, never read as a path below the
/// workspace.
pub fn checked_dir(dir: Option<&str>) -> Result<Option<String>, String> {
    if let Some(dir) = dir {
        let dir = dir.trim();
        let mut characters = dir.chars();
        let rooted = matches!(characters.next(), Some('/' | '\\' | '~'))
            || matches!(characters.next(), Some(':'));
        if rooted {
            return Err(format!(
                "`cwd` is a directory inside the workspace, relative to its root: `{}` is not",
                cineris_base::visible(dir)
            ));
        }
    }
    Ok(normal_dir(dir))
}

/// A text that is not empty and within its length.
fn within(name: &str, text: &str, longest: usize) -> Result<(), String> {
    if text.trim().is_empty() {
        return Err(format!("`{name}` is empty"));
    }
    let length = text.chars().count();
    if length > longest {
        return Err(format!(
            "`{name}` is {length} characters long, and may be up to {longest}"
        ));
    }
    Ok(())
}

/// A pattern or a command line: within its length, and one line with no
/// control character.
fn one_line(name: &str, text: &str) -> Result<(), String> {
    within(name, text, MAX_DETAIL)?;
    if text.chars().any(char::is_control) {
        return Err(format!("`{name}` is one line, with no control character"));
    }
    Ok(())
}

/// A path that is relative and stays below where it starts: no root, no
/// drive, no `..`.
fn inside(name: &str, path: &str) -> Result<(), String> {
    within(name, path, MAX_DETAIL)?;
    let mut characters = path.chars();
    let rooted = matches!(characters.next(), Some('/' | '\\'))
        || matches!(characters.next(), Some(':'))
        || path.starts_with('~');
    let climbs = path.split(['/', '\\']).any(|part| part == "..");
    if rooted || climbs || path.chars().any(char::is_control) {
        return Err(format!(
            "`{name}` is a path inside the workspace, relative to its root: `{}` is not",
            cineris_base::visible(path)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: Kind, text: &str) -> Entry {
        Entry::written(EntryId::new("k_1"), Scope::Private, kind, text)
    }

    fn command(run: &str) -> Command {
        Command {
            purpose: Purpose::Test,
            run: run.to_owned(),
            cwd: None,
            usual_ms: None,
            read_only: false,
        }
    }

    fn at(text: &str) -> UtcTime {
        UtcTime::parse(text).unwrap()
    }

    #[test]
    fn an_entry_is_in_use_only_while_active_and_before_its_expiry() {
        let now = at("2026-11-16T10:00:00Z");
        let mut entry = entry(Kind::Fact, "The release script is scripts/release.sh.");
        assert!(entry.in_use(now));
        for status in [Status::Proposed, Status::Disabled] {
            entry.status = status;
            assert!(!entry.in_use(now), "{status:?}");
        }
        entry.status = Status::Active;
        entry.expires = Some(at("2026-11-16T10:00:01Z"));
        assert!(entry.in_use(now));
        // The moment of expiry is past it.
        entry.expires = Some(now);
        assert!(!entry.in_use(now));
        assert!(entry.is_expired(now));
    }

    #[test]
    fn only_an_accepted_command_in_use_runs_unasked() {
        let now = at("2026-11-16T10:00:00Z");
        let learned = Learned {
            session: None,
            item: None,
            at: now,
        };
        let mut observed = entry(Kind::Command, "The project's test command.");
        observed.command = Some(command("cargo test"));
        observed.basis = Some(Basis::Observed);
        observed.learned = Some(learned);
        // Active by itself: in use, and it still asks.
        assert!(observed.in_use(now));
        assert!(!observed.is_accepted());
        assert!(!observed.runs_unasked(now));
        observed.accepted = Some(now);
        assert!(observed.runs_unasked(now));
        // Not when disabled, proposed or past its expiry.
        for status in [Status::Proposed, Status::Disabled] {
            let mut other = observed.clone();
            other.status = status;
            assert!(!other.runs_unasked(now), "{status:?}");
        }
        let mut expired = observed.clone();
        expired.expires = Some(now);
        assert!(!expired.runs_unasked(now));
        // Not an entry of another kind, whatever it carries.
        let mut flaky = observed.clone();
        flaky.kind = Kind::Flaky;
        assert!(!flaky.runs_unasked(now));
    }

    #[test]
    fn a_shared_entry_is_not_accepted_for_having_been_written() {
        let now = at("2026-11-16T10:00:00Z");
        let mut shared = Entry::written(
            EntryId::new("s_1"),
            Scope::Shared,
            Kind::Command,
            "Runs the tests.",
        );
        shared.command = Some(command("cargo test"));
        assert!(!shared.is_accepted());
        assert!(!shared.runs_unasked(now));
        shared.accepted = Some(now);
        assert!(shared.runs_unasked(now));
        // The same entry in the user's own store is the user's word.
        let mut own = shared.clone();
        own.scope = Scope::Private;
        own.accepted = None;
        assert!(own.runs_unasked(now));
    }

    #[test]
    fn the_content_hash_follows_what_the_entry_says_and_nothing_else() {
        let mut one = entry(Kind::Command, "Runs the tests.");
        one.command = Some(command("cargo test"));
        let hash = one.content_hash();
        // Not its id, its standing, its dates or how long it takes.
        let mut same = one.clone();
        same.id = EntryId::new("k_9");
        same.status = Status::Disabled;
        same.accepted = Some(at("2026-11-16T10:00:00Z"));
        same.command.as_mut().unwrap().usual_ms = Some(4_200);
        assert_eq!(same.content_hash(), hash);
        // Its kind, text, command line, directory, purpose and check.
        let changes: [fn(&mut Entry); 7] = [
            |entry| entry.text.push('!'),
            |entry| entry.kind = Kind::Flaky,
            |entry| entry.command.as_mut().unwrap().run.push_str(" --all"),
            |entry| entry.command.as_mut().unwrap().cwd = Some("sub".to_owned()),
            |entry| entry.command.as_mut().unwrap().purpose = Purpose::Build,
            |entry| entry.command.as_mut().unwrap().read_only = true,
            |entry| {
                entry.check = Some(Check::Exists {
                    path: "Cargo.toml".to_owned(),
                });
            },
        ];
        for (index, change) in changes.iter().enumerate() {
            let mut other = one.clone();
            change(&mut other);
            assert_ne!(other.content_hash(), hash, "change {index}");
        }
        // How its text is spaced and where its lines end is not what it
        // says: a checkout that changed the line endings has the same
        // entry, under the same id.
        let mut spaced = one.clone();
        spaced.text = "  Runs\r\n the   tests.\n".to_owned();
        assert_eq!(spaced.content_hash(), hash);
        // A character moved from one part to the next is another entry.
        let mut moved = one.clone();
        moved.text = "Runs the tests.c".to_owned();
        moved.command.as_mut().unwrap().run = "argo test".to_owned();
        assert_ne!(moved.content_hash(), hash);
    }

    #[test]
    fn an_entry_that_is_not_well_formed_is_refused_with_the_reason() {
        let good = {
            let mut entry = entry(Kind::Command, "Runs the tests.");
            entry.command = Some(command("cargo test"));
            entry
        };
        assert_eq!(good.validate(), Ok(()));
        type Change = fn(&mut Entry);
        let cases: [(Change, &str); 14] = [
            (
                |entry| {
                    entry.check = Some(Check::Command {
                        command: "cargo --version\nrm -rf x".to_owned(),
                    });
                },
                "`check.command` is one line",
            ),
            (
                |entry| {
                    entry.check = Some(Check::Contains {
                        path: "src".to_owned(),
                        pattern: "a\u{0}b".to_owned(),
                    });
                },
                "`check.contains.pattern` is one line",
            ),
            (|entry| entry.text = "  ".to_owned(), "`text` is empty"),
            (
                |entry| entry.text = "x".repeat(MAX_TEXT + 1),
                "601 characters",
            ),
            (|entry| entry.command = None, "needs a `command`"),
            (
                |entry| entry.kind = Kind::Fact,
                "kind `fact` takes no `command`",
            ),
            (
                |entry| entry.command.as_mut().unwrap().run = "a\nb".to_owned(),
                "one line",
            ),
            (
                |entry| entry.command.as_mut().unwrap().cwd = Some("../other".to_owned()),
                "inside the workspace",
            ),
            (
                |entry| entry.command.as_mut().unwrap().cwd = Some("/etc".to_owned()),
                "inside the workspace",
            ),
            (
                |entry| {
                    entry.check = Some(Check::Exists {
                        path: "C:\\Windows".to_owned(),
                    });
                },
                "inside the workspace",
            ),
            (
                |entry| {
                    entry.check = Some(Check::Contains {
                        path: "~/notes".to_owned(),
                        pattern: "x".to_owned(),
                    });
                },
                "inside the workspace",
            ),
            (
                |entry| entry.basis = Some(Basis::Observed),
                "`basis` and `learned` go together",
            ),
            (
                |entry| entry.lapsed = Some(UtcTime::UNIX_EPOCH),
                "`lapsed` is for an entry whose `status` is `disabled`",
            ),
            (|entry| entry.id = EntryId::new("k 1"), "`id` may hold"),
        ];
        for (change, expected) in cases {
            let mut entry = good.clone();
            change(&mut entry);
            let problem = entry.validate().unwrap_err();
            assert!(problem.contains(expected), "{problem:?} for {expected:?}");
        }
        // A flaky entry may name the command that flipped, or not.
        let mut flaky = good.clone();
        flaky.kind = Kind::Flaky;
        assert_eq!(flaky.validate(), Ok(()));
        flaky.command = None;
        assert_eq!(flaky.validate(), Ok(()));
    }

    #[test]
    fn a_directory_that_names_a_root_is_refused_as_written() {
        for rooted in [
            "/etc",
            "\\\\server\\share",
            "\\x",
            "C:\\Windows",
            "c:x",
            "~/notes",
            " /etc ",
        ] {
            let problem = checked_dir(Some(rooted)).unwrap_err();
            assert!(
                problem.contains("inside the workspace"),
                "{rooted}: {problem}"
            );
        }
        assert_eq!(checked_dir(None), Ok(None));
        assert_eq!(
            checked_dir(Some("./crates//app/")),
            Ok(Some("crates/app".to_owned()))
        );
        // `..` is not a root: the entry's own check refuses it.
        assert_eq!(checked_dir(Some("../up")), Ok(Some("../up".to_owned())));
    }

    #[test]
    fn a_directory_is_kept_one_way() {
        for (written, kept) in [
            (None, None),
            (Some(""), None),
            (Some("."), None),
            (Some("./"), None),
            (Some("crates/app/"), Some("crates/app")),
            (Some("./crates//app"), Some("crates/app")),
            (Some("crates\\app"), Some("crates/app")),
        ] {
            assert_eq!(normal_dir(written).as_deref(), kept, "{written:?}");
        }
    }

    #[test]
    fn the_wire_form_says_who_wrote_an_entry_and_whether_it_was_accepted() {
        let now = at("2026-11-16T10:00:00Z");
        let mut observed = entry(Kind::Command, "The project's test command.");
        observed.command = Some(command("cargo test"));
        observed.basis = Some(Basis::Observed);
        observed.seen = 2;
        observed.learned = Some(Learned {
            session: Some(SessionId::new("s_1")),
            item: None,
            at: now,
        });
        let wire = observed.to_wire();
        assert_eq!(wire.source.kind, wire::EntrySourceKind::Learned);
        assert_eq!(wire.basis, Some(wire::EntryBasis::Observed));
        assert_eq!(wire.observations, Some(2));
        assert_eq!(wire.accepted_at, None);
        assert_eq!(wire.command.unwrap().command, "cargo test");

        let written = entry(Kind::Fact, "A fact.").to_wire();
        assert_eq!(written.source.kind, wire::EntrySourceKind::User);
        assert_eq!(written.basis, None);
        assert_eq!(written.observations, None);
    }

    #[test]
    fn a_check_goes_to_the_wire_and_back_and_a_malformed_one_is_refused() {
        for check in [
            Check::Exists {
                path: "Cargo.toml".to_owned(),
            },
            Check::Contains {
                path: "src".to_owned(),
                pattern: "/v2".to_owned(),
            },
            Check::Command {
                command: "cargo --version".to_owned(),
            },
        ] {
            assert_eq!(Check::from_wire(&check.to_wire()), Ok(check));
        }
        let mut both = Check::Exists {
            path: "a".to_owned(),
        }
        .to_wire();
        both.command = Some("x".to_owned());
        assert!(Check::from_wire(&both).is_err());
        let mut unknown = both;
        unknown.kind = wire::EntryCheckKind::from_wire("later");
        assert!(Check::from_wire(&unknown).is_err());
    }
}
