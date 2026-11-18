//! The two files, as text (`docs/architecture/14-adaptation.md`, "Where it
//! lives").
//!
//! | File | Read by | Written by |
//! |---|---|---|
//! | `knowledge.toml`, the private store | [`parse_private`] | [`write_private`], whole, on every change |
//! | `.cineris/profile.toml`, the shared file | [`parse_shared`] | [`append_shared`], which adds one entry to the end and keeps every byte a person wrote |
//!
//! # Both are read strictly
//!
//! A key or a value this build does not know, a newer `version`, a file or
//! an entry past its cap: the whole file is refused, with a [`Problem`]
//! that says where. An accepted command becomes an allow rule, so an entry
//! is a value the harness enforces
//! (`docs/decisions/0028-enforced-values-fail-closed.md`): a field that is
//! not understood could be the one that narrows it.
//!
//! # What the shared file cannot say
//!
//! An entry of the shared file has a kind, a text, and optionally evidence,
//! a command and a check. It has no status, no source and no acceptance: a
//! repository cannot say that its entry is in use. Each is read as a
//! proposal to this user, whose own decision is in the private store,
//! against the entry's [content hash](Entry::content_hash).

use cineris_base::{ContentHash, UtcTime};
use cineris_protocol::ids::{EntryId, ItemId, SessionId};
use serde::{Deserialize, Serialize};
use toml::Spanned;

use crate::entry::{Basis, Check, Command, Entry, Kind, Learned, Purpose, Scope, Status};

/// The version of both formats this build writes and reads.
pub const VERSION: u32 = 1;

/// The largest file read, and the largest written, in bytes. It binds
/// before the cap on entries does when the entries are long: 243 of the
/// longest fill it.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;

/// The most entries the private store holds.
pub const MAX_PRIVATE_ENTRIES: usize = 500;

/// The most entries the shared file holds.
pub const MAX_SHARED_ENTRIES: usize = 200;

/// The most decisions about shared entries the private store keeps. Past
/// it, the oldest go.
pub const MAX_DECISIONS: usize = 1_000;

/// The largest number an id or the counter may hold: past it a file is
/// refused, so that no arithmetic on a number a person wrote can wrap round
/// to an id in use.
pub const MAX_NUMBER: u64 = 1 << 53;

/// The line the private store begins with.
const PRIVATE_HEADER: &str = "\
# What Cineris has learned about this project, and what you told it.
# You may edit this file: `cineris project list` shows how it is read.
# It is written again whole when something is learned, so comments are not kept.
";

/// The line a new shared file begins with.
const SHARED_HEADER: &str = "\
# This project's shared profile for Cineris: commands, conventions and facts.
# Each entry is a proposal to whoever opens the project, until they accept it.
";

/// Why a file is not used as it is.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{file}` cannot be read as it is: {message}")]
pub struct Problem {
    /// The file, as a person names it: `knowledge.toml`,
    /// `.cineris/profile.toml`.
    pub file: String,
    /// What is wrong, with the line where one is known.
    pub message: String,
}

impl Problem {
    /// A problem with `file`. The message is shown to a person, and may
    /// quote what the file holds (a key's name, a value): whatever would
    /// act unseen in it is written as an escape.
    pub(crate) fn new(file: &str, message: impl Into<String>) -> Self {
        Self {
            file: file.to_owned(),
            message: cineris_base::visible(&message.into()).into_owned(),
        }
    }
}

/// What this user decided about an entry of the shared file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// The shared entry's content hash when the decision was made.
    pub hash: ContentHash,
    /// Accepted (`Active`) or turned off (`Disabled`).
    pub status: Status,
    /// When.
    pub at: UtcTime,
}

/// The private store, read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivateState {
    /// Its entries, in the file's order.
    pub entries: Vec<Entry>,
    /// What the user decided about shared entries.
    pub decisions: Vec<Decision>,
    /// The number the next new entry's id takes, when it is past every
    /// number in use. It never goes down: the id of an entry that was
    /// deleted is not given again, so a change sent for an id names the
    /// entry the sender was shown, or nothing.
    pub next: u64,
}

impl PrivateState {
    /// The number the next new entry's id takes.
    #[must_use]
    pub fn next_number(&self) -> u64 {
        self.next.max(largest_number(&self.entries) + 1)
    }

    /// Takes the id for a new entry: `k_<n>`, a number no entry of this
    /// store has had.
    pub fn take_id(&mut self) -> EntryId {
        let number = self.next_number();
        self.next = number + 1;
        EntryId::new(format!("k_{number}"))
    }

    /// The decision about the shared entry with this content hash.
    #[must_use]
    pub fn decision(&self, hash: &ContentHash) -> Option<&Decision> {
        self.decisions
            .iter()
            .find(|decision| decision.hash == *hash)
    }

    /// Records a decision about a shared entry, in place of any earlier
    /// one about the same content, and lets the oldest go past the cap.
    pub fn decide(&mut self, hash: ContentHash, status: Status, at: UtcTime) {
        self.decisions.retain(|decision| decision.hash != hash);
        self.decisions.push(Decision { hash, status, at });
        if self.decisions.len() > MAX_DECISIONS {
            let extra = self.decisions.len() - MAX_DECISIONS;
            self.decisions.drain(..extra);
        }
    }
}

fn largest_number(entries: &[Entry]) -> u64 {
    entries
        .iter()
        .filter_map(|entry| entry.id.as_str().strip_prefix("k_")?.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
}

// ------------------------------------------------------------ the wire forms

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLearned {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    item: Option<String>,
    at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommand {
    purpose: String,
    run: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    usual_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "is_false")]
    read_only: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde calls it with a reference"
)]
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContains {
    path: String,
    pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheck {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exists: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    contains: Option<RawContains>,
}

/// An entry of the private store. Plain values come before tables, which
/// is the order TOML writes them in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPrivate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    kind: String,
    text: String,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirmed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lapsed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seen: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    learned: Option<RawLearned>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<RawCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    check: Option<RawCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecision {
    hash: String,
    status: String,
    at: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrivateRead {
    version: u32,
    #[serde(default)]
    next: Option<u64>,
    #[serde(default, rename = "entry")]
    entries: Vec<Spanned<RawPrivate>>,
    #[serde(default, rename = "shared")]
    decisions: Vec<Spanned<RawDecision>>,
}

#[derive(Debug, Serialize)]
struct PrivateWrite {
    version: u32,
    next: u64,
    #[serde(rename = "entry", skip_serializing_if = "Vec::is_empty")]
    entries: Vec<RawPrivate>,
    #[serde(rename = "shared", skip_serializing_if = "Vec::is_empty")]
    decisions: Vec<RawDecision>,
}

/// An entry of the shared file: what it says, and nothing about whether it
/// is in use.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawShared {
    kind: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    evidence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<RawCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    check: Option<RawCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedRead {
    version: u32,
    /// Each entry as the table it is: one this build does not understand
    /// is left out alone.
    #[serde(default, rename = "entry")]
    entries: Vec<Spanned<toml::Table>>,
}

/// The shared file, read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SharedFile {
    /// The entries this build understands, each a proposal.
    pub entries: Vec<Entry>,
    /// What is wrong with each entry that was left out, with its line.
    pub left_out: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SharedOne {
    entry: Vec<RawShared>,
}

// ------------------------------------------------------------------- reading

/// The line a byte of `text` is on, counted from 1.
fn line_of(text: &str, at: usize) -> usize {
    text.as_bytes()
        .iter()
        .take(at)
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}

fn syntax(file: &str, text: &str, error: &toml::de::Error) -> Problem {
    let message = error.message();
    match error.span() {
        Some(span) => Problem::new(
            file,
            format!("line {}: {message}", line_of(text, span.start)),
        ),
        None => Problem::new(file, message),
    }
}

fn check_version(file: &str, version: u32) -> Result<(), Problem> {
    if version == VERSION {
        Ok(())
    } else {
        Err(Problem::new(
            file,
            format!(
                "its version is {version}, and this version of Cineris reads version \
                 {VERSION}"
            ),
        ))
    }
}

fn time(name: &str, text: &str) -> Result<UtcTime, String> {
    UtcTime::parse(text).map_err(|error| format!("`{name}`: {error}"))
}

fn optional_time(name: &str, text: Option<&String>) -> Result<Option<UtcTime>, String> {
    text.map(|text| time(name, text)).transpose()
}

fn named<T>(
    name: &str,
    text: &str,
    parse: fn(&str) -> Option<T>,
    known: &str,
) -> Result<T, String> {
    parse(text).ok_or_else(|| {
        format!(
            "`{name}` is `{}`, which is none of {known}",
            cineris_base::visible(text)
        )
    })
}

fn command_of(raw: &RawCommand) -> Result<Command, String> {
    Ok(Command {
        purpose: named(
            "command.purpose",
            &raw.purpose,
            Purpose::parse,
            "build, test, check, format and run",
        )?,
        run: raw.run.clone(),
        cwd: crate::entry::checked_dir(raw.cwd.as_deref())?,
        usual_ms: raw.usual_ms,
        read_only: raw.read_only,
    })
}

fn check_of(raw: &RawCheck) -> Result<Check, String> {
    match (&raw.exists, &raw.contains, &raw.command) {
        (Some(path), None, None) => Ok(Check::Exists { path: path.clone() }),
        (None, Some(contains), None) => Ok(Check::Contains {
            path: contains.path.clone(),
            pattern: contains.pattern.clone(),
        }),
        (None, None, Some(command)) => Ok(Check::Command {
            command: command.clone(),
        }),
        _ => Err("`check` takes exactly one of `exists`, `contains` and `command`".to_owned()),
    }
}

fn raw_command(command: &Command) -> RawCommand {
    RawCommand {
        purpose: command.purpose.as_str().to_owned(),
        run: command.run.clone(),
        cwd: command.cwd.clone(),
        usual_ms: command.usual_ms,
        read_only: command.read_only,
    }
}

fn raw_check(check: &Check) -> RawCheck {
    let mut raw = RawCheck {
        exists: None,
        command: None,
        contains: None,
    };
    match check {
        Check::Exists { path } => raw.exists = Some(path.clone()),
        Check::Contains { path, pattern } => {
            raw.contains = Some(RawContains {
                path: path.clone(),
                pattern: pattern.clone(),
            });
        }
        Check::Command { command } => raw.command = Some(command.clone()),
    }
    raw
}

fn private_entry(raw: &RawPrivate, id: EntryId) -> Result<Entry, String> {
    let learned = raw
        .learned
        .as_ref()
        .map(|learned| -> Result<Learned, String> {
            Ok(Learned {
                session: learned.session.clone().map(SessionId::new),
                item: learned.item.clone().map(ItemId::new),
                at: time("learned.at", &learned.at)?,
            })
        })
        .transpose()?;
    let entry = Entry {
        id,
        scope: Scope::Private,
        kind: named(
            "kind",
            &raw.kind,
            Kind::parse,
            "command, convention, flaky and fact",
        )?,
        text: raw.text.clone(),
        status: named(
            "status",
            &raw.status,
            Status::parse,
            "proposed, active and disabled",
        )?,
        basis: raw
            .basis
            .as_ref()
            .map(|basis| named("basis", basis, Basis::parse, "observed and proposed"))
            .transpose()?,
        learned,
        evidence: raw.evidence.clone(),
        confirmed: optional_time("confirmed", raw.confirmed.as_ref())?,
        expires: optional_time("expires", raw.expires.as_ref())?,
        accepted: optional_time("accepted", raw.accepted.as_ref())?,
        lapsed: optional_time("lapsed", raw.lapsed.as_ref())?,
        seen: raw.seen.unwrap_or(0),
        command: raw.command.as_ref().map(command_of).transpose()?,
        check: raw.check.as_ref().map(check_of).transpose()?,
    };
    entry.validate()?;
    Ok(entry)
}

/// Reads the private store's text.
///
/// An entry with no `id` (one a person added by hand) is given the next
/// one, in the file's order: the same one every time the same text is
/// read, until a write puts it in the file.
///
/// # Errors
///
/// Anything this build does not understand in full, with the line where it
/// is: the file is then not used at all.
pub fn parse_private(text: &str, file: &str) -> Result<PrivateState, Problem> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let read: PrivateRead = toml::from_str(text).map_err(|error| syntax(file, text, &error))?;
    check_version(file, read.version)?;
    if read.entries.len() > MAX_PRIVATE_ENTRIES {
        return Err(Problem::new(
            file,
            format!(
                "it holds {} entries, and may hold up to {MAX_PRIVATE_ENTRIES}",
                read.entries.len()
            ),
        ));
    }
    let largest = read
        .entries
        .iter()
        .filter_map(|entry| {
            entry
                .get_ref()
                .id
                .as_deref()?
                .strip_prefix("k_")?
                .parse::<u64>()
                .ok()
        })
        .max()
        .unwrap_or(0);
    if largest > MAX_NUMBER || read.next.is_some_and(|next| next > MAX_NUMBER) {
        return Err(Problem::new(
            file,
            format!("the number of an id, or `next`, is past {MAX_NUMBER}"),
        ));
    }
    let mut next = read.next.unwrap_or(0).max(largest + 1);
    let mut state = PrivateState::default();
    for spanned in &read.entries {
        let at = |message: String| {
            Problem::new(
                file,
                format!(
                    "the entry at line {}: {message}",
                    line_of(text, spanned.span().start)
                ),
            )
        };
        let raw = spanned.get_ref();
        let id = match &raw.id {
            // `s_` begins the id of a shared entry, which is made from what
            // the entry says: one written here would stand in front of it.
            Some(id) if id.starts_with("s_") => {
                return Err(at(format!(
                    "the id `{id}` begins with `s_`, which is how the shared file's entries \
                     are named"
                )));
            }
            Some(id) => EntryId::new(id.clone()),
            None => {
                next += 1;
                EntryId::new(format!("k_{}", next - 1))
            }
        };
        if state.entries.iter().any(|known| known.id == id) {
            return Err(at(format!("the id `{id}` is used twice")));
        }
        state.entries.push(private_entry(raw, id).map_err(at)?);
    }
    for spanned in &read.decisions {
        let at = |message: String| {
            Problem::new(
                file,
                format!(
                    "the `[[shared]]` table at line {}: {message}",
                    line_of(text, spanned.span().start)
                ),
            )
        };
        let raw = spanned.get_ref();
        let hash = ContentHash::from_hex(&raw.hash)
            .ok_or_else(|| at("`hash` is not a content hash".to_owned()))?;
        let status = match Status::parse(&raw.status) {
            Some(status @ (Status::Active | Status::Disabled)) => status,
            _ => {
                return Err(at(format!(
                    "`status` is `{}`, which is neither active nor disabled",
                    cineris_base::visible(&raw.status)
                )));
            }
        };
        state.decisions.push(Decision {
            hash,
            status,
            at: time("at", &raw.at).map_err(at)?,
        });
    }
    if state.decisions.len() > MAX_DECISIONS {
        return Err(Problem::new(
            file,
            format!(
                "it holds {} `[[shared]]` tables, and may hold up to {MAX_DECISIONS}",
                state.decisions.len()
            ),
        ));
    }
    // The counter after every id is given: past the bound, no change
    // could be written back.
    if next > MAX_NUMBER {
        return Err(Problem::new(
            file,
            format!("the number of an id, or `next`, is past {MAX_NUMBER}"),
        ));
    }
    state.next = next;
    Ok(state)
}

/// The private store as text, whole.
#[must_use]
pub fn write_private(state: &PrivateState) -> String {
    let stamp = |time: Option<UtcTime>| time.map(|time| time.to_timestamp().into_string());
    let entries = state
        .entries
        .iter()
        .map(|entry| RawPrivate {
            id: Some(entry.id.as_str().to_owned()),
            kind: entry.kind.as_str().to_owned(),
            text: entry.text.clone(),
            status: entry.status.as_str().to_owned(),
            basis: entry.basis.map(|basis| basis.as_str().to_owned()),
            evidence: entry.evidence.clone(),
            confirmed: stamp(entry.confirmed),
            expires: stamp(entry.expires),
            accepted: stamp(entry.accepted),
            lapsed: stamp(entry.lapsed),
            seen: (entry.seen > 0).then_some(entry.seen),
            learned: entry.learned.as_ref().map(|learned| RawLearned {
                session: learned
                    .session
                    .as_ref()
                    .map(|session| session.as_str().to_owned()),
                item: learned.item.as_ref().map(|item| item.as_str().to_owned()),
                at: learned.at.to_timestamp().into_string(),
            }),
            command: entry.command.as_ref().map(raw_command),
            check: entry.check.as_ref().map(raw_check),
        })
        .collect();
    let decisions = state
        .decisions
        .iter()
        .map(|decision| RawDecision {
            hash: decision.hash.to_hex(),
            status: decision.status.as_str().to_owned(),
            at: decision.at.to_timestamp().into_string(),
        })
        .collect();
    let body = toml::to_string(&PrivateWrite {
        version: VERSION,
        next: state.next_number(),
        entries,
        decisions,
    })
    // Strings, numbers and tables of them: there is nothing here that TOML
    // cannot write.
    .unwrap_or_default();
    format!("{PRIVATE_HEADER}\n{body}")
}

/// Why an entry was not added to the shared file. The file is as it was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppendError {
    /// The file holds as much as it may: entries, or bytes.
    #[error("{0}")]
    Full(Problem),
    /// The file cannot be read as it is, or already holds the entry.
    #[error("{0}")]
    Invalid(Problem),
}

/// The id of a shared entry: `s_` and the first 32 hexadecimal digits of
/// its content hash. The same on every machine, and another when the entry
/// changes, so an id names exactly what the user was shown.
#[must_use]
pub fn shared_id(hash: &ContentHash) -> EntryId {
    let hex = hash.to_hex();
    EntryId::new(format!("s_{}", hex.get(..32).unwrap_or(&hex)))
}

fn shared_entry(raw: &RawShared) -> Result<Entry, String> {
    let kind = named(
        "kind",
        &raw.kind,
        Kind::parse,
        "command, convention, flaky and fact",
    )?;
    let mut entry = Entry::written(EntryId::new("s_"), Scope::Shared, kind, raw.text.clone());
    // What this user decided is applied when the two files are put
    // together: until then it is a proposal.
    entry.status = Status::Proposed;
    entry.evidence.clone_from(&raw.evidence);
    entry.command = raw.command.as_ref().map(command_of).transpose()?;
    // How long a command takes is this machine's to say.
    if let Some(command) = &mut entry.command {
        command.usual_ms = None;
    }
    entry.check = raw.check.as_ref().map(check_of).transpose()?;
    entry.id = shared_id(&entry.content_hash());
    entry.validate()?;
    Ok(entry)
}

/// Reads the shared file's text. Every entry is a proposal: what this user
/// decided about each is in the private store.
///
/// An entry with a key or a value this build does not know is left out,
/// alone, and said ([`SharedFile::left_out`]): an entry only ever adds, so
/// leaving one out widens nothing, and a team on two builds keeps the
/// entries both understand.
///
/// # Errors
///
/// Text that is not TOML, a key outside the entries that this build does
/// not know, a newer version, or more entries than the file may hold: the
/// file is then not used at all.
pub fn parse_shared(text: &str, file: &str) -> Result<SharedFile, Problem> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let read: SharedRead = toml::from_str(text).map_err(|error| syntax(file, text, &error))?;
    check_version(file, read.version)?;
    if read.entries.len() > MAX_SHARED_ENTRIES {
        return Err(Problem::new(
            file,
            format!(
                "it holds {} entries, and may hold up to {MAX_SHARED_ENTRIES}",
                read.entries.len()
            ),
        ));
    }
    let mut shared = SharedFile::default();
    for spanned in &read.entries {
        let read = toml::Value::Table(spanned.get_ref().clone())
            .try_into::<RawShared>()
            .map_err(|error| error.message().to_owned())
            .and_then(|raw| shared_entry(&raw));
        match read {
            // The same entry written twice is one entry.
            Ok(entry) => {
                if shared.entries.iter().all(|known| known.id != entry.id) {
                    shared.entries.push(entry);
                }
            }
            // The message may quote a key's name, which is the file's
            // own text.
            Err(message) => shared.left_out.push(format!(
                "`{file}`: the entry at line {} is left out: {}",
                line_of(text, spanned.span().start),
                cineris_base::visible(&message)
            )),
        }
    }
    Ok(shared)
}

/// The shared file's text with `entry` added at its end. Everything a
/// person wrote stays as it is, comments included. `existing` is the
/// file's text now, or `None` when there is no file.
///
/// # Errors
///
/// When the file as it is cannot be read (it is then left alone), is full,
/// or already holds the entry.
pub fn append_shared(
    existing: Option<&str>,
    entry: &Entry,
    file: &str,
) -> Result<String, AppendError> {
    let held = match existing {
        Some(text) => parse_shared(text, file).map_err(AppendError::Invalid)?,
        None => SharedFile::default(),
    };
    let held = held.entries.len() + held.left_out.len();
    if held >= MAX_SHARED_ENTRIES {
        return Err(AppendError::Full(Problem::new(
            file,
            format!("it holds {MAX_SHARED_ENTRIES} entries already, the most it may"),
        )));
    }
    let mut command = entry.command.as_ref().map(raw_command);
    if let Some(command) = &mut command {
        command.usual_ms = None;
    }
    let one = SharedOne {
        entry: vec![RawShared {
            kind: entry.kind.as_str().to_owned(),
            text: entry.text.clone(),
            evidence: entry.evidence.clone(),
            command,
            check: entry.check.as_ref().map(raw_check),
        }],
    };
    let block = toml::to_string(&one).unwrap_or_default();
    let mut text = match existing {
        Some(text) => text.to_owned(),
        None => format!("{SHARED_HEADER}\nversion = {VERSION}\n"),
    };
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push('\n');
    text.push_str(&block);
    // What is written must be a file every reader takes: one past the cap
    // would be refused whole by everyone who has the repository.
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(AppendError::Full(Problem::new(
            file,
            format!(
                "with this entry it would be {} bytes, and it may be up to {MAX_FILE_BYTES}",
                text.len()
            ),
        )));
    }
    // And it must read back with exactly the one entry more.
    let after = parse_shared(&text, file).map_err(AppendError::Invalid)?;
    if after.entries.len() + after.left_out.len() != held + 1 {
        return Err(AppendError::Invalid(Problem::new(
            file,
            "it already holds this entry",
        )));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: &str = "knowledge.toml";
    const SHARED: &str = ".cineris/profile.toml";

    const FULL: &str = r#"
version = 1

[[entry]]
id = "k_3"
kind = "command"
text = "The project's test command."
status = "active"
basis = "observed"
evidence = "Ran and succeeded twice."
confirmed = "2026-11-16T10:00:00Z"
expires = "2027-02-14T10:00:00Z"
accepted = "2026-11-16T10:05:00Z"
seen = 2
learned = { session = "s_1", item = "i_4", at = "2026-11-16T10:00:00Z" }
command = { purpose = "test", run = "cargo test", cwd = "crates/app", usual_ms = 4200 }
check = { exists = "Cargo.toml" }

[[entry]]
kind = "convention"
text = "Errors are returned, never panicked on."
status = "active"

[[shared]]
hash = "9f2c000000000000000000000000000000000000000000000000000000000000"
status = "active"
at = "2026-11-16T10:06:00Z"
"#;

    #[test]
    fn the_private_store_reads_and_writes_back_to_the_same_state() {
        let state = parse_private(FULL, NAME).unwrap();
        assert_eq!(state.entries.len(), 2);
        let command = &state.entries[0];
        assert_eq!(command.id.as_str(), "k_3");
        assert_eq!(command.kind, Kind::Command);
        assert_eq!(command.basis, Some(Basis::Observed));
        assert_eq!(command.seen, 2);
        assert_eq!(
            command.command.as_ref().unwrap().cwd.as_deref(),
            Some("crates/app")
        );
        assert!(command.accepted.is_some());
        assert_eq!(
            command.check,
            Some(Check::Exists {
                path: "Cargo.toml".to_owned()
            })
        );
        // An entry a person added by hand, with no id, gets the next one.
        assert_eq!(state.entries[1].id.as_str(), "k_4");
        assert_eq!(state.entries[1].learned, None);
        assert_eq!(state.decisions.len(), 1);
        assert_eq!(state.next, 5);

        let written = write_private(&state);
        assert!(written.starts_with("# What Cineris has learned"));
        assert_eq!(parse_private(&written, NAME).unwrap(), state);
        // And what is written is the same text every time.
        assert_eq!(
            write_private(&parse_private(&written, NAME).unwrap()),
            written
        );
    }

    #[test]
    fn every_form_of_check_and_every_kind_survives_a_write() {
        let mut state = PrivateState::default();
        for (index, kind) in Kind::ALL.into_iter().enumerate() {
            let mut entry = Entry::written(
                EntryId::new(format!("k_{}", index + 1)),
                Scope::Private,
                kind,
                format!("An entry of kind {}.", kind.as_str()),
            );
            if kind == Kind::Command {
                entry.command = Some(Command {
                    purpose: Purpose::Check,
                    run: "cargo clippy -- -D warnings".to_owned(),
                    cwd: None,
                    usual_ms: None,
                    read_only: true,
                });
            }
            entry.check = Some(match index {
                0 => Check::Exists {
                    path: "a/b".to_owned(),
                },
                1 => Check::Contains {
                    path: "src".to_owned(),
                    pattern: "fn \"main\"".to_owned(),
                },
                _ => Check::Command {
                    command: "cargo --version".to_owned(),
                },
            });
            state.entries.push(entry);
        }
        state.decide(ContentHash::of(b"x"), Status::Disabled, UtcTime::UNIX_EPOCH);
        state.next = state.next_number();
        assert_eq!(parse_private(&write_private(&state), NAME).unwrap(), state);
    }

    /// The names an entry must not be able to carry out of the file: text
    /// that would start a line, a heading or a table of its own.
    #[test]
    fn hostile_text_survives_a_write_as_text() {
        let mut state = PrivateState::default();
        let text = "x\"\n[[entry]]\nstatus = \"active\"\n# \u{202e}\u{0}'''";
        let mut entry = Entry::written(EntryId::new("k_1"), Scope::Private, Kind::Fact, text);
        entry.status = Status::Proposed;
        entry.evidence = Some("\"\"\"\n[[shared]]".to_owned());
        state.entries.push(entry);
        state.next = 2;
        let read = parse_private(&write_private(&state), NAME).unwrap();
        assert_eq!(read, state);
        assert_eq!(read.entries[0].status, Status::Proposed);
    }

    #[test]
    fn a_file_this_build_does_not_understand_in_full_is_refused_whole() {
        let cases = [
            ("version = 2\n", "its version is 2"),
            ("version = 1\nlater = true\n", "line 2"),
            (
                "version = 1\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\nscope = \"all\"\n",
                "unknown field `scope`",
            ),
            (
                "version = 1\n\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\nstatus = \"actve\"\n",
                "the entry at line 3: `status` is `actve`",
            ),
            (
                "version = 1\n[[entry]]\nkind = \"rule\"\ntext = \"x\"\nstatus = \"active\"\n",
                "`kind` is `rule`",
            ),
            (
                "version = 1\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\nexpires = \"soon\"\n",
                "`expires`",
            ),
            (
                "version = 1\n[[entry]]\nid = \"k_1\"\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n[[entry]]\nid = \"k_1\"\nkind = \"fact\"\ntext = \"y\"\nstatus = \"active\"\n",
                "the entry at line 7: the id `k_1` is used twice",
            ),
            (
                "version = 1\n[[entry]]\nkind = \"command\"\ntext = \"x\"\nstatus = \"active\"\ncommand = { purpose = \"deploy\", run = \"make\" }\n",
                "`command.purpose` is `deploy`",
            ),
            (
                "version = 1\n[[entry]]\nkind = \"command\"\ntext = \"x\"\nstatus = \"active\"\ncommand = { purpose = \"test\", run = \"make\", sudo = true }\n",
                "unknown field `sudo`",
            ),
            (
                "version = 1\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\ncheck = { exists = \"a\", command = \"b\" }\n",
                "exactly one of",
            ),
            (
                "version = 1\n[[shared]]\nhash = \"zz\"\nstatus = \"active\"\nat = \"2026-11-16T10:06:00Z\"\n",
                "`hash` is not a content hash",
            ),
            (
                "version = 1\n[[shared]]\nhash = \"9f2c000000000000000000000000000000000000000000000000000000000000\"\nstatus = \"proposed\"\nat = \"2026-11-16T10:06:00Z\"\n",
                "neither active nor disabled",
            ),
            ("version = 1\n[[entry]\n", "line 2"),
            ("", "version"),
        ];
        for (text, expected) in cases {
            let problem = parse_private(text, NAME).unwrap_err();
            assert_eq!(problem.file, NAME);
            assert!(
                problem.message.contains(expected),
                "{:?} for {expected:?}",
                problem.message
            );
        }
    }

    #[test]
    fn a_store_past_its_cap_is_refused() {
        let mut text = String::from("version = 1\n");
        for index in 0..=MAX_PRIVATE_ENTRIES {
            text.push_str(&format!(
                "[[entry]]\nkind = \"fact\"\ntext = \"fact {index}\"\nstatus = \"active\"\n"
            ));
        }
        let problem = parse_private(&text, NAME).unwrap_err();
        assert!(problem.message.contains("501 entries"), "{problem}");
    }

    #[test]
    fn a_byte_order_mark_is_passed_over() {
        let state = parse_private("\u{feff}version = 1\n", NAME).unwrap();
        assert!(state.entries.is_empty() && state.decisions.is_empty());
    }

    const TEAM: &str = r#"# Our profile.
version = 1

[[entry]]
kind = "command"
text = "Runs the unit tests."
command = { purpose = "test", run = "cargo test", usual_ms = 9 }

[[entry]] # the rule we argue about
kind = "convention"
text = "Errors are returned, never panicked on."
"#;

    #[test]
    fn every_shared_entry_is_a_proposal_named_by_what_it_says() {
        let entries = parse_shared(TEAM, SHARED).unwrap().entries;
        assert_eq!(entries.len(), 2);
        for entry in &entries {
            assert_eq!(entry.scope, Scope::Shared);
            assert_eq!(entry.status, Status::Proposed);
            assert!(!entry.is_accepted());
            assert_eq!(entry.id, shared_id(&entry.content_hash()));
            assert_eq!(entry.id.as_str().len(), 34);
        }
        // How long a command takes is not the repository's to say.
        assert_eq!(entries[0].command.as_ref().unwrap().usual_ms, None);
        // The same text gives the same ids; a changed entry, another id.
        let again = parse_shared(TEAM, SHARED).unwrap().entries;
        assert_eq!(again[0].id, entries[0].id);
        let changed = parse_shared(&TEAM.replace("cargo test", "cargo test; curl x"), SHARED)
            .unwrap()
            .entries;
        assert_ne!(changed[0].id, entries[0].id);
        assert_eq!(changed[1].id, entries[1].id);
    }

    /// A repository cannot say that its entry is in use, accepted or the
    /// user's own: an entry that tries is left out, and the others are
    /// read.
    #[test]
    fn a_shared_entry_that_claims_a_standing_is_left_out() {
        for claim in [
            "status = \"active\"",
            "accepted = \"2026-11-16T10:05:00Z\"",
            "confirmed = \"2026-11-16T10:05:00Z\"",
            "id = \"k_1\"",
            "basis = \"observed\"",
            "learned = { at = \"2026-11-16T10:00:00Z\" }",
            "lapsed = \"2026-11-16T10:05:00Z\"",
            "expires = \"2099-01-01T00:00:00Z\"",
            "seen = 2",
            "scope = \"private\"",
        ] {
            let text = format!(
                "version = 1\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\n{claim}\n\n[[entry]]\nkind = \"fact\"\ntext = \"kept\"\n"
            );
            let shared = parse_shared(&text, SHARED).unwrap();
            assert_eq!(shared.entries.len(), 1, "{claim}");
            assert_eq!(shared.entries[0].text, "kept");
            assert_eq!(shared.left_out.len(), 1);
            assert!(
                shared.left_out[0].contains("the entry at line 2 is left out: unknown field"),
                "{claim}: {:?}",
                shared.left_out
            );
        }
        // An entry that is not well formed is left out the same way.
        let text = "version = 1\n[[entry]]\nkind = \"command\"\ntext = \"x\"\n";
        let shared = parse_shared(text, SHARED).unwrap();
        assert!(shared.entries.is_empty());
        assert!(shared.left_out[0].contains("needs a `command`"));
        // A table of decisions is not an entry: the file is refused whole.
        let text = "version = 1\n[[shared]]\nhash = \"00\"\nstatus = \"active\"\nat = \"x\"\n";
        assert!(parse_shared(text, SHARED).is_err());
        assert!(parse_shared("version = 1\nnext = 4\n", SHARED).is_err());
    }

    #[test]
    fn a_shared_file_past_its_cap_or_of_a_later_version_is_refused() {
        let mut text = String::from("version = 1\n");
        for index in 0..=MAX_SHARED_ENTRIES {
            text.push_str(&format!(
                "[[entry]]\nkind = \"fact\"\ntext = \"fact {index}\"\n"
            ));
        }
        assert!(
            parse_shared(&text, SHARED)
                .unwrap_err()
                .message
                .contains("201 entries")
        );
        assert!(parse_shared("version = 3\n", SHARED).is_err());
    }

    #[test]
    fn an_entry_is_added_to_the_end_of_the_shared_file_and_nothing_else_moves() {
        let mut entry = Entry::written(
            EntryId::new("k_7"),
            Scope::Private,
            Kind::Command,
            "Builds \"everything\".",
        );
        entry.command = Some(Command {
            purpose: Purpose::Build,
            run: "cargo build --workspace".to_owned(),
            cwd: Some("crates".to_owned()),
            usual_ms: Some(1_000),
            read_only: false,
        });
        entry.accepted = Some(UtcTime::UNIX_EPOCH);
        let text = append_shared(Some(TEAM), &entry, SHARED).unwrap();
        assert!(text.starts_with(TEAM), "every byte a person wrote is kept");
        let entries = parse_shared(&text, SHARED).unwrap().entries;
        assert_eq!(entries.len(), 3);
        let added = &entries[2];
        assert_eq!(added.text, entry.text);
        assert_eq!(added.content_hash(), {
            let mut expected = entry.clone();
            expected.command.as_mut().unwrap().usual_ms = None;
            expected.content_hash()
        });
        // Nothing of its standing went with it.
        assert!(!text.contains("accepted") && !text.contains("k_7"));

        // A file that is not there is begun.
        let fresh = append_shared(None, &entry, SHARED).unwrap();
        assert!(fresh.contains("version = 1"));
        assert_eq!(parse_shared(&fresh, SHARED).unwrap().entries.len(), 1);
        // The same entry is not added twice.
        let twice = append_shared(Some(&fresh), &entry, SHARED).unwrap_err();
        assert!(twice.to_string().contains("already holds"), "{twice}");
        assert!(matches!(twice, AppendError::Invalid(_)));
        // A file that cannot be read is left alone.
        assert!(append_shared(Some("version = 1\n[[entry]\n"), &entry, SHARED).is_err());
        // A file with no line break at its end.
        let joined = append_shared(Some("version = 1"), &entry, SHARED).unwrap();
        assert_eq!(parse_shared(&joined, SHARED).unwrap().entries.len(), 1);
    }

    #[test]
    fn decisions_replace_each_other_and_the_oldest_go_past_the_cap() {
        let mut state = PrivateState::default();
        let hash = ContentHash::of(b"one");
        state.decide(hash, Status::Active, UtcTime::UNIX_EPOCH);
        state.decide(hash, Status::Disabled, UtcTime::UNIX_EPOCH);
        assert_eq!(state.decisions.len(), 1);
        assert_eq!(state.decision(&hash).unwrap().status, Status::Disabled);
        for index in 0..MAX_DECISIONS {
            state.decide(
                ContentHash::of(index.to_string().as_bytes()),
                Status::Active,
                UtcTime::UNIX_EPOCH,
            );
        }
        assert_eq!(state.decisions.len(), MAX_DECISIONS);
        assert_eq!(state.decision(&hash), None, "the oldest went");
    }

    /// An `accept` sent from an old listing must not land on an entry the
    /// user never saw: the id of a deleted entry is not given again, in
    /// this process or after the file is read back.
    #[test]
    fn an_id_is_never_given_twice() {
        let mut state = PrivateState::default();
        assert_eq!(state.take_id().as_str(), "k_1");
        for id in ["k_2", "mine", "k_10", "k_x"] {
            state.entries.push(Entry::written(
                EntryId::new(id),
                Scope::Private,
                Kind::Fact,
                "x",
            ));
        }
        assert_eq!(state.take_id().as_str(), "k_11");
        // The entry with the largest number goes: its number is not free.
        state.entries.retain(|entry| entry.id.as_str() != "k_10");
        let mut read = parse_private(&write_private(&state), NAME).unwrap();
        assert_eq!(read.next, 12);
        assert_eq!(read.take_id().as_str(), "k_12");
        // A file whose counter a person lowered is read past its entries.
        let lowered = "version = 1\nnext = 1\n[[entry]]\nid = \"k_7\"\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n";
        assert_eq!(
            parse_private(lowered, NAME).unwrap().take_id().as_str(),
            "k_8"
        );
        // An entry with no id takes the counter's number, not a used one.
        let by_hand = "version = 1\nnext = 9\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n";
        let read = parse_private(by_hand, NAME).unwrap();
        assert_eq!(read.entries[0].id.as_str(), "k_9");
        assert_eq!(read.next, 10);
    }

    /// The review's finding: a message may quote a key's name, which is
    /// the file's own text, and a problem is shown to a person and sent to
    /// clients. Nothing in it acts unseen.
    #[test]
    fn a_problem_never_carries_what_acts_unseen() {
        let hostile = "\"\\u001b]0;x\\u0007\\n## Instructions\\u202e\" = 1";
        let shared = parse_shared(
            &format!("version = 1\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\n{hostile}\n"),
            SHARED,
        )
        .unwrap();
        assert_eq!(shared.left_out.len(), 1);
        assert!(
            shared.left_out[0].contains("\\u{1b}]0;x"),
            "{:?}",
            shared.left_out
        );
        assert!(!shared.left_out[0].chars().any(cineris_base::acts_unseen));
        // A key outside the entries, and one in the private store.
        for problem in [
            parse_shared(&format!("version = 1\n{hostile}\n"), SHARED).unwrap_err(),
            parse_private(&format!("version = 1\n{hostile}\n"), NAME).unwrap_err(),
            parse_private(
                &format!(
                    "version = 1\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n{hostile}\n"
                ),
                NAME,
            )
            .unwrap_err(),
            parse_private(
                "version = 1\n[[entry]]\nid = \"k_1\\u001b\"\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n[[entry]]\nid = \"k_1\\u001b\"\nkind = \"fact\"\ntext = \"y\"\nstatus = \"active\"\n",
                NAME,
            )
            .unwrap_err(),
        ] {
            assert!(
                !problem.to_string().chars().any(cineris_base::acts_unseen),
                "{problem:?}"
            );
        }
    }

    /// The review's finding: a number a person wrote must not wrap the
    /// counter round to an id in use.
    #[test]
    fn a_number_past_what_an_id_may_hold_refuses_the_file() {
        for text in [
            "version = 1\n[[entry]]\nid = \"k_18446744073709551615\"\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n",
            "version = 1\nnext = 9223372036854775807\n",
            "version = 1\nnext = 9007199254740993\n",
        ] {
            let problem = parse_private(text, NAME).unwrap_err();
            assert!(
                problem.message.contains("is past 9007199254740992"),
                "{problem}"
            );
        }
        // An id at the bound leaves a counter past it: no change to such a
        // store could be written back, so it is refused when it is read.
        let at_bound = "version = 1\n[[entry]]\nid = \"k_9007199254740992\"\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n";
        assert!(parse_private(at_bound, NAME).is_err());
        let by_hand = "version = 1\nnext = 9007199254740992\n[[entry]]\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n";
        assert!(parse_private(by_hand, NAME).is_err());
        // The largest counter that is read: every store that is read can be
        // written back as it is.
        let state = parse_private("version = 1\nnext = 9007199254740992\n", NAME).unwrap();
        assert_eq!(parse_private(&write_private(&state), NAME).unwrap(), state);
        // An id whose number is no number is a name like any other.
        let named = "version = 1\n[[entry]]\nid = \"k_99999999999999999999999\"\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n";
        assert_eq!(
            parse_private(named, NAME).unwrap().take_id().as_str(),
            "k_1"
        );
    }

    /// An id that begins `s_` is a shared entry's, made from what the entry
    /// says: one written in the private store would stand in front of it.
    #[test]
    fn a_private_id_cannot_take_a_shared_entrys_name() {
        let text = "version = 1\n[[entry]]\nid = \"s_0123456789abcdef0123456789abcdef\"\nkind = \"fact\"\ntext = \"x\"\nstatus = \"active\"\n";
        let problem = parse_private(text, NAME).unwrap_err();
        assert!(problem.message.contains("begins with `s_`"), "{problem}");
    }

    /// The shared file's cap on its size is held when it is written.
    #[test]
    fn an_entry_that_would_take_the_shared_file_past_its_size_is_not_added() {
        let mut text = String::from("version = 1\n");
        text.push_str(&format!("# {}\n", "p".repeat(256 * 1024 - 200)));
        let entry = Entry::written(
            EntryId::new("k_1"),
            Scope::Private,
            Kind::Fact,
            "x".repeat(300),
        );
        let problem = append_shared(Some(&text), &entry, SHARED).unwrap_err();
        assert!(
            problem.to_string().contains("may be up to 262144"),
            "{problem}"
        );
        assert!(matches!(problem, AppendError::Full(_)));
        // One that fits is added.
        let small = Entry::written(EntryId::new("k_1"), Scope::Private, Kind::Fact, "x");
        assert!(append_shared(Some(&text), &small, SHARED).is_ok());
    }
}
