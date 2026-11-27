//! The user's own pins (since step 28): the hash of each thing of the
//! user's own that runs code and that the user approved, in `pins.toml` in
//! the state directory (`docs/architecture/06-permissions-and-sandbox.md`,
//! "Project trust", "As built in step 28", and "Signed approvals").
//!
//! A project's definitions are pinned with its trust, in `trust.toml`. The
//! user's own extensions, in the configuration directory's `extensions/`,
//! belong to no project, and are pinned here: the user approves one, its
//! hash is pinned, and it is used only while it is still what was approved.
//! Step 30 keeps the user's MCP servers' pins of each tool here too, so a
//! pin has a kind, as the pins of `trust.toml` have.
//!
//! # The stored file
//!
//! ```toml
//! version = 2
//! revision = 4
//! signature = "1f0c…"       # over the revision and every pin
//!
//! [[pin]]
//! kind = "extension"
//! name = "tracker"
//! hash = "5e884898da28…"
//! registration = "a3b1…"    # what it registered when it was approved
//! at = "2026-11-20T10:15:02Z"
//!
//! [[held]]                  # not signed: kept only for its gates
//! kind = "extension"
//! name = "gate"
//! hash = "0b7c…"
//! at = "2026-11-18T09:00:00Z"
//! ```
//!
//! - **The pins are signed together** (since the signed approvals step):
//!   one Ed25519 signature, by the key the person's password makes, over
//!   the revision and every pin's kind, name, hash and registration, in the
//!   order of kind and name ([`crate::signed`]). A pin counts toward a
//!   [`Standing`] only while the whole set checks with the session's
//!   [`Verifier`]: an unsigned pin, one written by another program, a set
//!   with a pin taken out or put back, and a file of an older version count
//!   for nothing.
//! - **Every change is the person's.** Adding a pin and taking one away
//!   both change the signed set, so each is proposed
//!   ([`PinStore::propose`]), signed by the person at their yes, and
//!   committed ([`PinStore::commit`]), which writes nothing unless the
//!   signature is over exactly what the file would then hold. Taking an
//!   approval away is signed too: a pin can hold a gate, and a gate
//!   dropped quietly is a widening (the maintainers' answer to the design's
//!   call 4).
//! - **What a pin restricts counts whether or not it is signed.** Every pin
//!   in the file, signed or not, is in [`UserPins::held`]: the session
//!   holds the gates of what it names, as it does for one changed or gone.
//!   Only what a pin allows waits for a signature.
//! - **Pins that do not count are carried forward as restrictions, never
//!   signed and never dropped by a signed change** (the full review of the
//!   signed approvals: a yes to one extension, after a reset, an upgrade or
//!   one write by the model, signed a set without the pins that held gates,
//!   and the gates lapsed). A change proposed over a set that does not count
//!   moves its pins into `[[held]]`, an unsigned list beside the signed
//!   pins: what earlier pins named, kept only for their gates. Nothing reads
//!   it as a grant: [`UserPins::standing`], [`UserPins::get`] and
//!   [`UserPins::list`] count only the signed pins, and the statement signs
//!   only them. A pin approved again moves from the list to the signed set;
//!   a signed unpin of a name takes it out of both, the person's yes naming
//!   it being what lets its gates go. The list is unsigned by design:
//!   another program can only add to it, which holds a gate the person
//!   lets go with a signed unpin (a denial), or delete the file, the stated
//!   residual.
//! - **The revision** goes up by one at each change, so that a set signed
//!   once is never the same bytes as a later one. An earlier set, signed
//!   whole, still checks: putting a whole earlier file back is the stated
//!   rollback (`docs/security.md`, section 3).
//! - **One pin for each kind and name.** Pinning a definition again
//!   replaces its pin, in its place; `at` is when it was last pinned, and is
//!   not signed.
//! - **Written owner-only, atomically, on every write,** under a lock held
//!   from the read to the write (`pins.toml.lock`, `crate::store_lock`), as
//!   `trust.toml` is.
//! - **Read strictly.** A key this build does not know, a value of the
//!   wrong type, a `version` other than 1 or 2, a hash that is not one, or
//!   two pins of one kind and name, signed or held, refuse the whole file,
//!   and then
//!   **nothing is taken as pinned**: a file that cannot be read in full is
//!   not read in part (decision 0028's rule,
//!   `docs/decisions/0028-enforced-values-fail-closed.md`). The error says
//!   why, and the file is left as it is: a change through the store fails
//!   rather than write over it.
//! - **Version 1** (step 28's, never signed) is read as unsigned: its pins
//!   hold their gates, and approve nothing. The person's next change writes
//!   version 2 with only what they sign, and its pins held; an unsigned pin
//!   is never signed in bulk, since one written by the model before the
//!   upgrade would be laundered.
//! - **A kind this build does not know** is kept as it was written, and
//!   written back: a later build's pin, which approves nothing this build
//!   asks about.
//! - **A missing file pins nothing,** and the first pin makes it, with its
//!   folder, owner-only.
//! - **A link at its name is not followed,** as at the registration
//!   cache's and an extension's values' (since the review of step 28): the
//!   store is not read through it, so it pins nothing and says why, and no
//!   change writes through it, so what it names is left as it is.
//! - **No tool can reach it.** It is in the state directory, which the
//!   built-in protection closes to every tool, to reading and to writing,
//!   at every level.

#![allow(
    clippy::disallowed_methods,
    reason = "blocking file calls; the store is used at start-up and on the blocking pool"
)]

use std::io;
use std::path::{Path, PathBuf};

use cineris_base::{
    AtomicWriteError, AtomicWriteOptions, ContentHash, OpenError, SharedClock, write_atomic,
};
use cineris_protocol::ids::Timestamp;
use cineris_protocol::methods::TrustResourceKind;
use serde::{Deserialize, Serialize};

use super::{Approved, Checked, Definition, Standing};
use cineris_protocol::methods::{ApprovalProposal, ProposedPin};

use crate::signed::{Signature, SignedPin, Statement, Verifier, user_pins_statement};
use crate::store_lock;

/// The version of the file's form this build writes.
const VERSION: u32 = 2;

/// Step 28's version, never signed: read as unsigned.
const UNSIGNED_VERSION: u32 = 1;

/// The largest file read.
const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// A point of the store's (`cineris_base::hook`): a change has read the
/// file and checked it, under the lock, and is about to write it. A test
/// holds it to bring a second writer in between a read and its write.
pub const BEFORE_THE_PINS_ARE_WRITTEN: &str =
    "pins: the file is read and changed, and is about to be written";

/// One thing of the user's own that the user approved: what kind of thing,
/// its name, the hash of what was approved, what it registered, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserPin {
    /// What kind of thing it is: `extension`.
    pub kind: TrustResourceKind,
    /// Its name, unique within its kind.
    pub name: String,
    /// The hash of what the user approved ([`Definition::hash`]).
    pub hash: ContentHash,
    /// The hash of what it registered when the person approved it, for an
    /// extension evaluated before the question (the registration cache's
    /// entry is used only while its hash is this one); `None` where it was
    /// not known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration: Option<ContentHash>,
    /// When it was pinned. Not signed: it only says when.
    pub at: Timestamp,
}

impl UserPin {
    /// The definition the user approved.
    #[must_use]
    pub fn definition(&self) -> Definition {
        Definition {
            kind: self.kind.clone(),
            name: self.name.clone(),
            hash: self.hash,
        }
    }

    /// Whether this is the pin of the thing of this kind and name. Kinds
    /// are compared as they are written, so that a known kind held as an
    /// unknown one is still the same kind.
    fn is_of(&self, kind: &TrustResourceKind, name: &str) -> bool {
        self.kind.as_str() == kind.as_str() && self.name == name
    }
}

/// What the user has pinned, read once: a value to ask about each
/// definition as it is met, as [`ProjectTrust`](super::ProjectTrust) is of a
/// project's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPins {
    /// The file's signed set (`[[pin]]`), whether or not it checks.
    pins: Vec<UserPin>,
    /// The file's held list (`[[held]]`): never signed.
    kept: Vec<UserPin>,
    /// Both, the signed set first: what restricts.
    every: Vec<UserPin>,
    checked: Checked,
    revision: u64,
}

impl Default for UserPins {
    fn default() -> Self {
        Self::of(File::empty(), Checked::Empty)
    }
}

impl UserPins {
    /// What `file` holds, standing as `checked` with the session's key.
    fn of(file: File, checked: Checked) -> Self {
        let every = file.pins.iter().chain(&file.held).cloned().collect();
        Self {
            pins: file.pins,
            kept: file.held,
            every,
            checked,
            revision: file.revision,
        }
    }

    /// Whether the pins count: the set is signed by the session's key, or
    /// there is none.
    #[must_use]
    pub fn counts(&self) -> bool {
        matches!(self.checked, Checked::Signed | Checked::Empty)
    }

    /// How the pins stand with the session's key.
    #[must_use]
    pub fn checked(&self) -> Checked {
        self.checked
    }

    /// The pin of the thing of this kind and name, if there is one and it
    /// counts.
    #[must_use]
    pub fn get(&self, kind: &TrustResourceKind, name: &str) -> Option<&UserPin> {
        self.list().iter().find(|pin| pin.is_of(kind, name))
    }

    /// How a definition of the user's own stands: `Approved` when its hash
    /// is the pinned one, `Changed` when a pin of its kind and name holds
    /// another hash, and `New` when nothing of its kind and name is pinned,
    /// or the pins do not count. Never `Untrusted`: the user's own things
    /// belong to no project.
    #[must_use]
    pub fn standing(&self, definition: &Definition) -> Standing {
        match self.get(&definition.kind, &definition.name) {
            Some(pin) if pin.hash == definition.hash => Standing::Approved,
            Some(_) => Standing::Changed,
            None => Standing::New,
        }
    }

    /// Every pin that counts, in the order each was first pinned: none
    /// unless the set is signed by the session's key.
    #[must_use]
    pub fn list(&self) -> &[UserPin] {
        if self.counts() { &self.pins } else { &[] }
    }

    /// Every pin in the file, signed or not, the held list's included: what
    /// restricts. A session holds the gates of each as it holds those of an
    /// approved thing that is not loaded (changed, unreadable or gone),
    /// whatever its signature. Never what approves: [`list`](Self::list)
    /// and [`standing`](Self::standing) are.
    #[must_use]
    pub fn held(&self) -> &[UserPin] {
        &self.every
    }

    /// The pins that hold gates and approve nothing: the held list, beside a
    /// set that counts; every pin in the file, when the set does not count.
    /// A session holds the gates of each of these, whatever it finds at its
    /// name, since nothing the person's key signed approves it.
    #[must_use]
    pub fn unsigned(&self) -> &[UserPin] {
        if self.counts() {
            &self.kept
        } else {
            &self.every
        }
    }

    /// The pin of this kind and name among [`unsigned`](Self::unsigned):
    /// what holds its gates and approves nothing.
    #[must_use]
    pub fn unsigned_of(&self, kind: &TrustResourceKind, name: &str) -> Option<&UserPin> {
        self.unsigned().iter().find(|pin| pin.is_of(kind, name))
    }

    /// Whether nothing counts as pinned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list().is_empty()
    }

    /// The revision of the set read: 0 for none.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

/// Why the user's pins could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PinError {
    /// The file could not be read.
    #[error("the pin store `{path}` could not be read, so nothing is taken as pinned: {source}")]
    Read {
        /// The file.
        path: String,
        /// Why.
        source: io::Error,
    },
    /// The file is not one this build understands in full. Nothing in it
    /// is taken as pinned, and it is left as it is.
    #[error(
        "the pin store `{path}` is not understood, so nothing is taken as pinned: {problem}. The \
         file was left as it is"
    )]
    NotUnderstood {
        /// The file.
        path: String,
        /// What is wrong with it.
        problem: String,
    },
    /// The file changed between the question and the answer: nothing was
    /// written, and the person is asked again about what it holds now.
    #[error(
        "the pin store `{path}` changed while you were asked, so nothing was written: asked again"
    )]
    Moved {
        /// The file.
        path: String,
    },
    /// The signature given is not the session's key's over what would be
    /// written: nothing was written.
    #[error(
        "the approval for the pin store `{path}` is not signed by your key over what it would \
         hold, so nothing was written"
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
struct File {
    version: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<Signature>,
    #[serde(default, rename = "pin", skip_serializing_if = "Vec::is_empty")]
    pins: Vec<UserPin>,
    /// What earlier pins named that the person's key did not sign, kept
    /// only for their gates (since the full review of the signed approvals).
    /// Not signed, and never read as approving.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    held: Vec<UserPin>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl File {
    fn empty() -> Self {
        Self {
            version: VERSION,
            revision: 0,
            signature: None,
            pins: Vec::new(),
            held: Vec::new(),
        }
    }

    /// What the file's signature must be over.
    fn statement(&self) -> Statement {
        statement(self.revision, &self.pins)
    }

    /// How the file stands with `verifier`.
    fn checked(&self, verifier: Option<&Verifier>) -> Checked {
        if self.pins.is_empty() && self.signature.is_none() {
            return Checked::Empty;
        }
        Checked::of(self.signature.as_ref(), &self.statement(), verifier)
    }
}

/// The statement a set of pins is signed as: the revision, then each pin's
/// kind, name, hash and registration, in the order of kind and name.
fn statement(revision: u64, pins: &[UserPin]) -> Statement {
    let mut signed: Vec<SignedPin<'_>> = pins
        .iter()
        .map(|pin| SignedPin {
            kind: pin.kind.as_str(),
            name: &pin.name,
            hash: *pin.hash.as_bytes(),
            registration: pin.registration.map(|hash| *hash.as_bytes()),
        })
        .collect();
    user_pins_statement(revision, &mut signed)
}

/// One change to the user's pins, as the person is asked to sign it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinChange {
    /// Pin a definition as it is now, with what it registered where that is
    /// known: adds its pin, or replaces the pin of the same kind and name.
    Pin(Approved),
    /// Take away the pin of the thing of this kind and name.
    Unpin {
        /// Its kind.
        kind: TrustResourceKind,
        /// Its name.
        name: String,
    },
}

/// A change to the user's pins, worked out before the person is asked
/// ([`PinStore::propose`]): what the file would then hold, the statement the
/// person signs, and what changes, for them to read.
#[derive(Debug, Clone)]
pub struct PinProposal {
    /// What the file held when the change was worked out: committed only
    /// while it still holds that.
    base: Option<ContentHash>,
    next: File,
    statement: Statement,
    summary: Vec<String>,
}

impl PinProposal {
    /// The statement the person signs.
    #[must_use]
    pub fn statement(&self) -> &Statement {
        &self.statement
    }

    /// What changes, one line each, for the person to read before they
    /// sign: each pin added or taken away, and each pin not signed by their
    /// key that the change carries as held, holding its gates and approving
    /// nothing.
    #[must_use]
    pub fn summary(&self) -> &[String] {
        &self.summary
    }

    /// What the person signs, as the protocol carries it to a front end:
    /// [`crate::signed::statement_of`] makes this proposal's statement from
    /// it.
    #[must_use]
    pub fn view(&self) -> ApprovalProposal {
        ApprovalProposal::UserPins {
            revision: self.next.revision,
            pins: self
                .next
                .pins
                .iter()
                .map(|pin| ProposedPin {
                    kind: pin.kind.clone(),
                    name: pin.name.clone(),
                    hash: pin.hash.to_hex(),
                    registration: pin.registration.map(|hash| hash.to_hex()),
                })
                .collect(),
        }
    }
}

/// The user's pin store: the hashes of what the user approved of their own.
#[derive(Debug, Clone)]
pub struct PinStore {
    file: PathBuf,
    clock: SharedClock,
}

impl PinStore {
    /// The store at `file`: `StateDir::pins_file` of `cineris-config`.
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

    /// What the user has pinned, as it stands with `verifier`, the
    /// session's key: with none, the pins hold their gates and approve
    /// nothing ([`UserPins::held`]). A missing file pins nothing, and
    /// reading it creates nothing.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood in full. The
    /// caller takes nothing as pinned, and shows the error.
    pub fn pins(&self, verifier: Option<&Verifier>) -> Result<UserPins, PinError> {
        let (file, _) = self.read()?;
        let checked = file.checked(verifier);
        Ok(UserPins::of(file, checked))
    }

    /// Works out `changes`, in order, against the pins as they stand with
    /// `verifier`, the key the person's password just made: what the file
    /// would then hold, and the statement the person signs for it. Only the
    /// pins that count are signed again. Pins not signed by the session's
    /// key are never dropped: they are moved into the held list, which keeps
    /// their gates and approves nothing, and the summary names each (the
    /// full review of the signed approvals). A pin approved again leaves
    /// the held list for the signed set; an unpin takes its name out of
    /// both. `None` when the changes change nothing (an unpin of what is
    /// neither pinned nor held), so that nobody is asked.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or is not understood.
    pub fn propose(
        &self,
        changes: &[PinChange],
        verifier: &Verifier,
    ) -> Result<Option<PinProposal>, PinError> {
        let (file, base) = self.read()?;
        let checked = file.checked(Some(verifier));
        let counts = matches!(checked, Checked::Signed | Checked::Empty);
        // A set that does not count is signed again by nobody: what it
        // pinned is held, beside what was held already.
        let (mut pins, mut held) = if counts {
            (file.pins.clone(), file.held.clone())
        } else {
            (
                Vec::new(),
                file.pins.iter().chain(&file.held).cloned().collect(),
            )
        };
        let mut summary = Vec::new();
        let mut changed = false;
        let at = self.clock.now().to_timestamp();
        for change in changes {
            match change {
                PinChange::Pin(approved) => {
                    let pin = UserPin {
                        kind: approved.definition.kind.clone(),
                        name: approved.definition.name.clone(),
                        hash: approved.definition.hash,
                        registration: approved.registration,
                        at: at.clone(),
                    };
                    summary.push(format!(
                        "approve the {} `{}` ({})",
                        pin.kind,
                        pin.name.escape_debug(),
                        short(&pin.hash)
                    ));
                    held.retain(|old| !old.is_of(&pin.kind, &pin.name));
                    match pins.iter().position(|old| old.is_of(&pin.kind, &pin.name)) {
                        Some(index) => pins[index] = pin,
                        None => pins.push(pin),
                    }
                    changed = true;
                }
                PinChange::Unpin { kind, name } => {
                    let (signed, kept) = (pins.len(), held.len());
                    pins.retain(|pin| !pin.is_of(kind, name));
                    held.retain(|pin| !pin.is_of(kind, name));
                    if pins.len() != signed {
                        summary.push(format!(
                            "take away the approval of the {kind} `{}`",
                            name.escape_debug()
                        ));
                        changed = true;
                    } else if held.len() != kept {
                        summary.push(format!(
                            "let go of the {kind} `{}`, approved earlier but not signed by your \
                             key: the gates it held no longer hold",
                            name.escape_debug()
                        ));
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            return Ok(None);
        }
        summary.extend(held.iter().map(|pin| held_line(&pin.kind, &pin.name)));
        let next = File {
            version: VERSION,
            revision: file.revision.saturating_add(1),
            signature: None,
            pins,
            held,
        };
        Ok(Some(PinProposal {
            base,
            statement: next.statement(),
            next,
            summary,
        }))
    }

    /// Writes what `proposal` worked out, signed with `signature`, when the
    /// signature is `verifier`'s over exactly that and the file still holds
    /// what it held when the proposal was made. Gives what is pinned after.
    ///
    /// # Errors
    ///
    /// [`PinError::Moved`] when the file changed since the proposal (ask
    /// again); [`PinError::NotSigned`] when the signature is not
    /// `verifier`'s over the proposal; otherwise when the file cannot be
    /// read, is not understood, or cannot be written. Nothing is written in
    /// any of these.
    pub fn commit(
        &self,
        proposal: &PinProposal,
        signature: &Signature,
        verifier: &Verifier,
    ) -> Result<UserPins, PinError> {
        if !verifier.verify(&proposal.statement, signature) {
            return Err(PinError::NotSigned {
                path: self.file.display().to_string(),
            });
        }
        let _lock = store_lock::hold(&self.file).map_err(|source| AtomicWriteError::Io {
            path: self.file.display().to_string(),
            step: "lock",
            source,
        })?;
        let (_, base) = self.read()?;
        if base != proposal.base {
            return Err(PinError::Moved {
                path: self.file.display().to_string(),
            });
        }
        let next = File {
            signature: Some(*signature),
            ..proposal.next.clone()
        };
        cineris_base::hook::point(BEFORE_THE_PINS_ARE_WRITTEN);
        self.write(&next)?;
        let checked = next.checked(Some(verifier));
        Ok(UserPins::of(next, checked))
    }

    /// Takes the signature off the pins, at a reset of the person's key:
    /// every pin stays, unsigned, so that it approves nothing and asks
    /// again, and still holds its gates ([`UserPins::held`]): the person's
    /// next yes over the pins carries it as held, and only a signed unpin of
    /// its name, or its approval again, lets its gates go. Deletes nothing,
    /// so that a
    /// reset run by something other than the person lifts no gate. Gives
    /// how many pins there are.
    ///
    /// # Errors
    ///
    /// When the file cannot be read, is not understood, or cannot be
    /// written.
    pub fn strip_signature(&self) -> Result<usize, PinError> {
        let _lock = store_lock::hold(&self.file).map_err(|source| AtomicWriteError::Io {
            path: self.file.display().to_string(),
            step: "lock",
            source,
        })?;
        let (mut file, base) = self.read()?;
        let count = file.pins.len();
        if base.is_none() || file.signature.is_none() {
            return Ok(count);
        }
        file.version = VERSION;
        file.signature = None;
        self.write(&file)?;
        Ok(count)
    }

    fn not_understood(&self, problem: String) -> PinError {
        PinError::NotUnderstood {
            path: self.file.display().to_string(),
            problem,
        }
    }

    /// The file, and the hash of its bytes (`None` for no file): what a
    /// proposal is committed against.
    fn read(&self) -> Result<(File, Option<ContentHash>), PinError> {
        let read_error = |source| PinError::Read {
            path: self.file.display().to_string(),
            source,
        };
        // A link at the store's name is not followed: what it names is not
        // the user's pins, and it is refused, as a folder there is.
        let (opened, found) = match cineris_base::open_regular(&self.file) {
            Ok(opened) => opened,
            Err(OpenError::NotFound) => return Ok((File::empty(), None)),
            Err(OpenError::Io(error)) => return Err(read_error(error)),
            Err(other) => return Err(read_error(io::Error::other(other.to_string()))),
        };
        let size = found.len();
        if size > MAX_BYTES {
            return Err(self.not_understood(format!(
                "it is {size} bytes, more than a pin store could be"
            )));
        }
        let mut text = String::new();
        io::Read::read_to_string(&mut io::Read::take(opened, MAX_BYTES), &mut text)
            .map_err(read_error)?;
        let base = ContentHash::of(text.as_bytes());
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let file: File = toml::from_str(text)
            .map_err(|error| self.not_understood(error.message().to_owned()))?;
        if !(UNSIGNED_VERSION..=VERSION).contains(&file.version) {
            return Err(self.not_understood(format!(
                "its version is {}, and this version of Cineris reads versions \
                 {UNSIGNED_VERSION} and {VERSION}",
                file.version
            )));
        }
        // Version 1 had no revision, signature, registration or held list:
        // one that says it has is not one a build wrote.
        if file.version == UNSIGNED_VERSION
            && (file.revision != 0
                || file.signature.is_some()
                || !file.held.is_empty()
                || file.pins.iter().any(|pin| pin.registration.is_some()))
        {
            return Err(self.not_understood(
                "it is version 1, which has no `revision`, `signature`, `registration` or \
                 `[[held]]`"
                    .to_owned(),
            ));
        }
        // A file this build wrote has one pin for each kind and name, signed
        // or held: one with two would leave it to the order which of them
        // stands.
        let every: Vec<&UserPin> = file.pins.iter().chain(&file.held).collect();
        for (index, pin) in every.iter().enumerate() {
            if every[..index]
                .iter()
                .any(|earlier| earlier.is_of(&pin.kind, &pin.name))
            {
                return Err(self.not_understood(format!(
                    "it pins the {} `{}` twice",
                    pin.kind,
                    pin.name.escape_debug()
                )));
            }
        }
        Ok((file, Some(base)))
    }

    fn write(&self, file: &File) -> Result<(), PinError> {
        // The directory was made when the lock was taken.
        let text = toml::to_string(file).map_err(|error| {
            self.not_understood(format!("it could not be written out: {error}"))
        })?;
        // A link at the store's name is not written through: the write is
        // refused, and what the link names is left as it is.
        let options = AtomicWriteOptions {
            owner_only: true,
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        write_atomic(&self.file, text.as_bytes(), &options, &*self.clock)?;
        Ok(())
    }
}

/// The first twelve digits of a hash: enough for a person to match it
/// with what they were shown.
fn short(hash: &ContentHash) -> String {
    hash.to_hex().chars().take(12).collect()
}

/// The line of a proposal's summary that says a pin is carried as held.
fn held_line(kind: &TrustResourceKind, name: &str) -> String {
    super::held_line(kind.as_str(), name)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cineris_base::FakeClock;

    use super::*;
    use crate::signed::Unit;
    use crate::store_lock::tests::{Scratch, is_held};
    use crate::test_support::Person;

    fn store(scratch: &Scratch) -> PinStore {
        let clock: SharedClock = Arc::new(FakeClock::starting_at("2026-11-20T10:00:00Z").unwrap());
        PinStore::new(scratch.0.join("pins.toml"), clock)
    }

    fn tracker(text: &str) -> Definition {
        Definition::of(TrustResourceKind::Extension, "tracker", [text])
    }

    #[test]
    fn a_commit_holds_the_lock_and_checks_the_file_did_not_move() {
        let scratch = Scratch::new();
        let store = store(&scratch);
        let person = Person::new();
        let proposal = store
            .propose(&[PinChange::Pin(tracker("v1").into())], &person.verifier())
            .unwrap()
            .unwrap();
        // Another change lands between the question and the answer.
        person.pin(
            &store,
            &Definition::of(TrustResourceKind::Extension, "other", ["x"]),
        );
        let moved = store.commit(
            &proposal,
            &person.sign(proposal.statement()),
            &person.verifier(),
        );
        assert!(matches!(moved, Err(PinError::Moved { .. })), "{moved:?}");
        assert_eq!(
            store
                .pins(Some(&person.verifier()))
                .unwrap()
                .standing(&tracker("v1")),
            Standing::New,
            "nothing of the moved proposal was written"
        );
        // Asked again, it lands; and the lock is let go after.
        let pins = person.pin(&store, &tracker("v1"));
        assert_eq!(pins.standing(&tracker("v1")), Standing::Approved);
        assert!(!is_held(store.path()));
    }

    #[test]
    fn a_signature_over_anything_else_writes_nothing() {
        let scratch = Scratch::new();
        let store = store(&scratch);
        let person = Person::new();
        let stranger = Person::with_password("someone else's password");
        let proposal = store
            .propose(&[PinChange::Pin(tracker("v1").into())], &person.verifier())
            .unwrap()
            .unwrap();
        // Signed by another key, checked with that key: refused all the same,
        // since the session checks with its own.
        let theirs = stranger.sign(proposal.statement());
        assert!(matches!(
            store.commit(&proposal, &theirs, &person.verifier()),
            Err(PinError::NotSigned { .. })
        ));
        // Signed by the person, over another statement.
        let other = person.sign(&Statement::build(Unit::UserPins).finish());
        assert!(matches!(
            store.commit(&proposal, &other, &person.verifier()),
            Err(PinError::NotSigned { .. })
        ));
        assert!(!store.path().exists(), "nothing was written");
    }

    /// A link put at the store's name after it was read, before it is
    /// written, is not written through: the change fails, and what the link
    /// names is left as it is.
    #[test]
    fn a_link_put_at_the_stores_name_after_its_read_is_not_written_through() {
        let scratch = Scratch::new();
        std::fs::create_dir_all(&scratch.0).unwrap();
        let store = store(&scratch);
        let named = scratch.0.join("elsewhere.toml");
        std::fs::write(&named, "the user's own file").unwrap();
        let person = Person::new();
        let proposal = store
            .propose(&[PinChange::Pin(tracker("v1").into())], &person.verifier())
            .unwrap()
            .unwrap();
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&named, store.path());
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(&named, store.path());
        match made {
            Ok(()) => {}
            Err(error) if std::env::var_os("GITHUB_ACTIONS").is_some() => {
                panic!("the CI runner cannot make a link: {error}")
            }
            Err(error) => {
                println!("skipped: cannot make a link here ({error})");
                return;
            }
        }
        let written = store.commit(
            &proposal,
            &person.sign(proposal.statement()),
            &person.verifier(),
        );
        assert!(written.is_err(), "{written:?}");
        assert_eq!(
            std::fs::read_to_string(&named).unwrap(),
            "the user's own file"
        );
    }

    #[test]
    fn a_proposal_shown_to_a_front_end_is_signed_as_the_store_checks_it() {
        let scratch = Scratch::new();
        let store = store(&scratch);
        let person = Person::new();
        person.pin(&store, &tracker("v1"));
        let proposal = store
            .propose(
                &[
                    PinChange::Pin(Approved {
                        definition: Definition::of(TrustResourceKind::Extension, "a", ["x"]),
                        registration: Some(ContentHash::of(b"registered")),
                    }),
                    PinChange::Unpin {
                        kind: TrustResourceKind::Extension,
                        name: "tracker".to_owned(),
                    },
                ],
                &person.verifier(),
            )
            .unwrap()
            .unwrap();
        let rebuilt = crate::signed::statement_of(&proposal.view()).unwrap();
        assert_eq!(&rebuilt, proposal.statement());
        let pins = store
            .commit(&proposal, &person.sign(&rebuilt), &person.verifier())
            .unwrap();
        assert!(pins.counts() && pins.list().len() == 1);
        assert_eq!(pins.revision(), 2);
    }

    /// The permission path's review, finding 3a: a pin the person's key did
    /// not sign is never carried into the set the person signs next. Since
    /// the full review it is carried as held: never signed, and never
    /// dropped, so that its gates hold.
    #[test]
    fn a_pin_not_signed_by_the_person_is_never_signed_with_their_next_pin() {
        let scratch = Scratch::new();
        let store = store(&scratch);
        let person = Person::new();
        let model = Person::with_password("the model's own password");
        let evil = Definition::of(TrustResourceKind::Extension, "evil", ["curl evil"]);
        // Written by the model under its own key, and, apart, by an older
        // version that signed nothing.
        for planted in ["theirs", "older"] {
            if planted == "theirs" {
                model.pin(&store, &evil);
            } else {
                std::fs::write(
                    store.path(),
                    format!(
                        "version = 1\n\n[[pin]]\nkind = \"extension\"\nname = \"evil\"\nhash = \
                         \"{}\"\nat = \"2026-11-20T10:00:00Z\"\n",
                        evil.hash
                    ),
                )
                .unwrap();
            }
            let proposal = store
                .propose(&[PinChange::Pin(tracker("v1").into())], &person.verifier())
                .unwrap()
                .unwrap();
            assert!(
                proposal.summary().iter().any(|line| line
                    == "the extension `evil`: approved earlier but not signed by your key; kept, \
                        holding its gates, approving nothing"),
                "{planted}: {:?}",
                proposal.summary()
            );
            // What the person signs, and what a front end is shown to sign,
            // name the pin they approved alone.
            let ApprovalProposal::UserPins { pins: shown, .. } = proposal.view() else {
                panic!("{planted}: not the user's pins");
            };
            assert_eq!(shown.len(), 1, "{planted}: {shown:?}");
            assert_eq!(shown[0].name, "tracker", "{planted}");
            let pins = store
                .commit(
                    &proposal,
                    &person.sign(proposal.statement()),
                    &person.verifier(),
                )
                .unwrap();
            assert!(pins.counts(), "{planted}");
            assert_eq!(pins.standing(&evil), Standing::New, "{planted}");
            assert!(
                pins.get(&TrustResourceKind::Extension, "evil").is_none()
                    && pins.list().iter().all(|pin| pin.name != "evil"),
                "{planted}: it approves nothing"
            );
            assert!(
                pins.held().iter().any(|pin| pin.name == "evil")
                    && pins
                        .unsigned_of(&TrustResourceKind::Extension, "evil")
                        .is_some_and(|pin| pin.hash == evil.hash),
                "{planted}: it holds its gates"
            );
            assert_eq!(
                pins.standing(&tracker("v1")),
                Standing::Approved,
                "{planted}"
            );
            std::fs::remove_file(store.path()).unwrap();
        }
    }

    /// The permission path's review, finding 3d: the registration and the
    /// revision are signed.
    #[test]
    fn a_registration_or_a_revision_changed_by_hand_counts_for_nothing() {
        let scratch = Scratch::new();
        let store = store(&scratch);
        let person = Person::new();
        let registered = ContentHash::of(b"what it registered");
        person
            .change_pins(
                &store,
                &[PinChange::Pin(Approved {
                    definition: tracker("v1"),
                    registration: Some(registered),
                })],
            )
            .unwrap();
        let written = std::fs::read_to_string(store.path()).unwrap();
        assert!(written.contains(&registered.to_hex()), "{written}");
        let other = ContentHash::of(b"a registration that drops a gate").to_hex();
        for (what, text) in [
            (
                "the registration",
                written.replace(&registered.to_hex(), &other),
            ),
            (
                "the revision",
                written.replace("revision = 1", "revision = 7"),
            ),
        ] {
            std::fs::write(store.path(), &text).unwrap();
            let pins = store.pins(Some(&person.verifier())).unwrap();
            assert_eq!(pins.checked(), Checked::NotTheirs, "{what}:\n{text}");
            assert!(!pins.counts(), "{what}");
            // Listed for nothing and approving nothing (the full review, L3:
            // a break that listed the unsigned pins was not caught here).
            assert!(pins.list().is_empty(), "{what}");
            assert_eq!(pins.standing(&tracker("v1")), Standing::New, "{what}");
            assert_eq!(pins.held().len(), 1, "{what}: it still holds its gates");
        }
    }

    /// The full review of the signed approvals: a held pin approved again
    /// leaves the held list for the signed set, and a signed unpin of a held
    /// name lets it go, saying so; an unpin of what is neither asks nobody.
    #[test]
    fn a_held_pin_is_approved_again_or_let_go_only_by_the_persons_yes() {
        let scratch = Scratch::new();
        let store = store(&scratch);
        let person = Person::new();
        let gate = Definition::of(TrustResourceKind::Extension, "gate", ["refuse"]);
        person.pin(&store, &gate);
        // The signature taken off, as a reset takes it: it counts for
        // nothing, and the next yes carries it as held.
        store.strip_signature().unwrap();
        let pins = person.pin(&store, &tracker("v1"));
        assert_eq!(pins.standing(&gate), Standing::New);
        assert_eq!(pins.unsigned().len(), 1);
        let written = std::fs::read_to_string(store.path()).unwrap();
        assert!(written.contains("[[held]]"), "{written}");
        // Approved again: signed, and held no more.
        let pins = person.pin(&store, &gate);
        assert_eq!(pins.standing(&gate), Standing::Approved);
        assert!(pins.unsigned().is_empty() && pins.held().len() == 2);
        // Held again, then let go by a signed unpin that names it.
        store.strip_signature().unwrap();
        person.pin(&store, &tracker("v2"));
        assert!(
            store
                .pins(Some(&person.verifier()))
                .unwrap()
                .unsigned_of(&TrustResourceKind::Extension, "gate")
                .is_some()
        );
        let unpin = |name: &str| PinChange::Unpin {
            kind: TrustResourceKind::Extension,
            name: name.to_owned(),
        };
        let proposal = store
            .propose(&[unpin("gate")], &person.verifier())
            .unwrap()
            .unwrap();
        assert!(
            proposal
                .summary()
                .iter()
                .any(|line| line.starts_with("let go of the extension `gate`")),
            "{:?}",
            proposal.summary()
        );
        let pins = store
            .commit(
                &proposal,
                &person.sign(proposal.statement()),
                &person.verifier(),
            )
            .unwrap();
        assert!(pins.held().iter().all(|pin| pin.name != "gate"));
        assert_eq!(pins.standing(&tracker("v2")), Standing::Approved);
        assert!(
            store
                .propose(&[unpin("none")], &person.verifier())
                .unwrap()
                .is_none()
        );
    }

    /// The maintainers, on the full review: a held entry another program
    /// adds can only deny. The signed set still counts, the entry approves
    /// nothing, and it holds its gates until the person's signed unpin.
    #[test]
    fn a_held_entry_written_by_another_program_only_holds_a_gate() {
        let scratch = Scratch::new();
        let store = store(&scratch);
        let person = Person::new();
        person.pin(&store, &tracker("v1"));
        let planted = Definition::of(TrustResourceKind::Extension, "planted", ["x"]);
        let mut text = std::fs::read_to_string(store.path()).unwrap();
        text.push_str(&format!(
            "\n[[held]]\nkind = \"extension\"\nname = \"planted\"\nhash = \"{}\"\nat = \
             \"2026-11-20T10:00:00Z\"\n",
            planted.hash
        ));
        std::fs::write(store.path(), &text).unwrap();
        let pins = store.pins(Some(&person.verifier())).unwrap();
        assert!(pins.counts(), "the signature is over the signed pins alone");
        assert_eq!(pins.standing(&tracker("v1")), Standing::Approved);
        assert_eq!(pins.standing(&planted), Standing::New);
        assert!(pins.get(&TrustResourceKind::Extension, "planted").is_none());
        assert!(pins.list().iter().all(|pin| pin.name != "planted"));
        assert!(
            pins.unsigned_of(&TrustResourceKind::Extension, "planted")
                .is_some()
        );
        // Without the key the same.
        let keyless = store.pins(None).unwrap();
        assert_eq!(keyless.standing(&planted), Standing::New);
        assert_eq!(keyless.unsigned().len(), 2);
    }

    /// A held list is read as strictly as the signed set: a name held twice,
    /// or both signed and held, refuses the whole file, and so does one in a
    /// file of version 1.
    #[test]
    fn a_held_list_is_read_strictly() {
        let scratch = Scratch::new();
        let store = store(&scratch);
        let person = Person::new();
        person.pin(&store, &tracker("v1"));
        let written = std::fs::read_to_string(store.path()).unwrap();
        let held = |name: &str| {
            format!(
                "\n[[held]]\nkind = \"extension\"\nname = \"{name}\"\nhash = \"{}\"\nat = \
                 \"2026-11-20T10:00:00Z\"\n",
                tracker("v0").hash
            )
        };
        for (what, text) in [
            ("signed and held", format!("{written}{}", held("tracker"))),
            (
                "held twice",
                format!("{written}{}{}", held("other"), held("other")),
            ),
            (
                "held in version 1",
                format!("version = 1\n{}", held("other")),
            ),
        ] {
            std::fs::write(store.path(), &text).unwrap();
            assert!(
                matches!(
                    store.pins(Some(&person.verifier())),
                    Err(PinError::NotUnderstood { .. })
                ),
                "{what}:\n{text}"
            );
        }
    }
}
