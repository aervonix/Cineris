//! The person's approvals in a core (the signed approvals step;
//! `docs/architecture/06-permissions-and-sandbox.md`, "Signed approvals").
//!
//! Every approval a session uses is signed by the key the person's password
//! makes (`cineris_policy::signed`). A core holds only the **public** half
//! of that key, once the person has typed their password in a front end of
//! this process ([`Core::open_approvals`]): what every session of the core
//! checks the stores' signatures with. Until then nothing in the stores
//! counts, and what they pin still restricts.
//!
//! - **The key is made from a typed password, never read.** A
//!   [`Verifier`] can only come from a [`PersonKey`], and a front end makes
//!   one only from what the person typed ([`Core::person_key`]): no file,
//!   flag, setting or variable opens the approvals.
//! - **No signing key is kept.** A [`PersonKey`] is made at each yes and
//!   dropped once it has signed (the owner's answer to the design's call 2).
//! - **One person to a process.** Once open, the approvals stay open with
//!   that key for every session of the core; a key from another password is
//!   refused ([`KeyRefusal::NotTheOpenKey`]).
//! - **A reset deletes nothing.** It takes the signature off every
//!   approval and makes a new key: what was approved asks again, and what
//!   it pins still holds its gates until the person's next yes over it. A
//!   reset run by something other than the person (past guard B) can only
//!   make the person's password fail to open their approvals: it lifts no
//!   gate.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use cineris_policy::signed::{
    Cost, KEY_FILE, KeyError, KeyFile, MIN_PASSWORD_CHARS, PersonKey, Verifier, fresh_salt,
};
use cineris_policy::{Checked, PinStore, TrustStore};
use cineris_protocol::ids::Timestamp;

use crate::core::Core;
use crate::manage::Project;

/// The key a core's sessions check signatures with, once the person has
/// opened their approvals.
#[derive(Debug, Default)]
pub(crate) struct Opened(RwLock<Option<Verifier>>);

impl Opened {
    /// The key, when the approvals are open.
    pub(crate) fn verifier(&self) -> Option<Verifier> {
        *self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn open(&self, verifier: Verifier) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = Some(verifier);
    }
}

/// Why the person's key could not be made, or was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyRefusal {
    /// No key has been set up: the first approval sets one up.
    NoKey,
    /// A key is set up already: set-up does not replace it (a reset does).
    KeyExists,
    /// The password typed does not make the key the approvals are signed
    /// with: mistyped, or the key was made again by something else.
    NotThePassword {
        /// The first bytes of the key the file is of, for the person to
        /// tell it apart.
        fingerprint: String,
        /// When the file says it was made.
        made_at: Timestamp,
    },
    /// The password typed makes another key than the one this core's
    /// sessions already check with.
    NotTheOpenKey,
    /// A password chosen with too few characters.
    TooShort,
    /// The key file could not be read or written, or Argon2 refused: said
    /// in full.
    Failed(String),
}

impl std::fmt::Display for KeyRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoKey => f.write_str(
                "no password has been set for your approvals yet: the first approval sets one",
            ),
            Self::KeyExists => f.write_str(
                "a password is set for your approvals already: `cineris approvals reset` sets a \
                 new one, and every approval asks again",
            ),
            Self::NotThePassword {
                fingerprint,
                made_at,
            } => write!(
                f,
                "that is not the password of your approvals (key {fingerprint}, made {made_at}). \
                 If you did not set it then, something else did: `cineris approvals reset` sets a \
                 new one, and every approval asks again"
            ),
            Self::NotTheOpenKey => f.write_str(
                "that password makes another key than the one your approvals were opened with in \
                 this session",
            ),
            Self::TooShort => write!(
                f,
                "a password for your approvals has at least {MIN_PASSWORD_CHARS} characters: a \
                 copy of the approvals lets anyone test guesses offline"
            ),
            Self::Failed(why) => f.write_str(why),
        }
    }
}

impl From<KeyError> for KeyRefusal {
    fn from(error: KeyError) -> Self {
        match error {
            KeyError::PasswordTooShort => Self::TooShort,
            other => Self::Failed(other.to_string()),
        }
    }
}

/// Why what a store holds approves nothing in this session, in words that
/// follow a colon (the session is told it): `NoKey`, `Unsigned` and
/// `NotTheirs`. Empty for the two that count.
pub(crate) fn uncounted(checked: Checked) -> &'static str {
    match checked {
        Checked::NoKey => "your approvals are not open in this session",
        Checked::Unsigned => {
            "written by an older version of Cineris, or before your password was reset: asked \
             again"
        }
        Checked::NotTheirs => "not signed by your key: changed by something other than Cineris",
        Checked::Signed | Checked::Empty => "",
    }
}

/// What the session is told, once, of a project whose entry in the trust
/// store does not count (`checked`): it is not trusted, and what its entry
/// pins holds its gates.
pub(crate) fn project_uncounted(checked: Checked) -> String {
    let out = match checked {
        Checked::NoKey => {
            "Open your approvals with your password when a session opens to use its trust"
        }
        _ => {
            "The project's trust question asks about it again when a session opens, and \
             `cineris permissions trust` trusts it"
        }
    };
    format!(
        "This project's trust does not count in this session ({}), so nothing of it that \
         needs trust is used: each hook and extension its trust approved holds its gates, \
         failing, refusing every call and prompt it gates. {out}.",
        uncounted(checked)
    )
}

/// What a reset took the signature off.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResetReport {
    /// How many of the user's own pins: each asks again.
    pub pins: usize,
    /// How many projects' trust: each is asked about again.
    pub projects: usize,
}

/// What the key file and the stores hold of the person's approvals: what
/// **carries** a signature, never what counts ([`Core::approvals_held`]).
/// Nothing is checked to make it: the stores are read with no key, so that
/// an entry with a signature is told from one without, whosever the
/// signature is and whatever it is over. It holds no secret and no key a
/// session could check with: the key file's fingerprint and its time only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalsHeld {
    /// The key file: none, what it says, or why it could not be read.
    pub key: HeldKey,
    /// How many pins the user's `pins.toml` holds, signed or not.
    pub pins: usize,
    /// Whether the user's pins carry a signature (one for the whole set).
    pub pins_carry_signature: bool,
    /// How many projects' entries in `trust.toml` carry a signature.
    pub projects_carrying: usize,
    /// How many carry none: written by an older version, or after a reset.
    pub projects_not_carrying: usize,
    /// Whether the entry of the project asked about carries a signature.
    pub project_carries: bool,
}

/// The key file, as [`ApprovalsHeld`] says it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeldKey {
    /// No key has been set up.
    None,
    /// A key is set up: its fingerprint and when it was made.
    Made {
        /// The first bytes of the public key the file is of, in
        /// hexadecimal (`KeyFile::fingerprint`).
        fingerprint: String,
        /// When the file says it was made.
        made_at: Timestamp,
    },
    /// The file could not be read or is not understood: why.
    Unreadable(String),
}

impl ApprovalsHeld {
    /// Whether a session opened in the project asked about has approvals to
    /// use: a key is set up, and the user's pins or the project's entry
    /// carry a signature. It decides only whether the person is asked for
    /// their password when the session opens: it opens nothing, and nothing
    /// it says counts until the typed password's key checks it.
    #[must_use]
    pub fn to_use(&self) -> bool {
        matches!(self.key, HeldKey::Made { .. })
            && (self.pins_carry_signature || self.project_carries)
    }
}

impl Core {
    /// What the key file and the stores hold of the person's approvals, and
    /// whether `project`'s entry carries a signature: one read of each of
    /// the three files, which checks nothing, opens nothing and writes
    /// nothing (`cineris approvals status`, and the application's question
    /// whether to ask for the password when a session opens). See
    /// [`ApprovalsHeld`]: a store's entry read with no key stands as
    /// `Checked::NoKey` when it carries a signature and `Checked::Unsigned`
    /// when it carries none.
    ///
    /// # Errors
    ///
    /// When there is no state directory, or a store cannot be read or is
    /// not understood. A key file that cannot be read is said in
    /// [`HeldKey::Unreadable`] instead.
    pub fn approvals_held(&self, project: Option<&Project>) -> Result<ApprovalsHeld, KeyRefusal> {
        let state = self
            .state_dir()
            .ok_or_else(|| KeyRefusal::Failed("no state directory is known".to_owned()))?;
        let key = match self.approval_key() {
            Ok(None) => HeldKey::None,
            Ok(Some(file)) => HeldKey::Made {
                fingerprint: file.fingerprint(),
                made_at: file.made_at().clone(),
            },
            Err(why) => HeldKey::Unreadable(why.to_string()),
        };
        let clock = Arc::clone(&self.inner.options.clock);
        let pins = PinStore::new(state.pins_file(), Arc::clone(&clock))
            .pins(None)
            .map_err(|failure| KeyRefusal::Failed(failure.to_string()))?;
        let projects = TrustStore::new(state.trust_file(), clock)
            .list(None)
            .map_err(|failure| KeyRefusal::Failed(failure.to_string()))?;
        // Read with no key: a signature is `NoKey`, none is `Unsigned`.
        let carries = |checked: Checked| checked == Checked::NoKey;
        let projects_carrying = projects
            .iter()
            .filter(|(_, _, checked)| carries(*checked))
            .count();
        Ok(ApprovalsHeld {
            key,
            pins: pins.held().len(),
            pins_carry_signature: carries(pins.checked()),
            projects_carrying,
            projects_not_carrying: projects.len() - projects_carrying,
            project_carries: project.is_some_and(|project| {
                projects
                    .iter()
                    .any(|(key, _, checked)| *key == project.key && carries(*checked))
            }),
        })
    }

    /// The key file's place in the state directory. `None` with no state
    /// directory.
    #[must_use]
    pub fn approval_key_file(&self) -> Option<PathBuf> {
        self.state_dir().map(|state| state.root().join(KEY_FILE))
    }

    /// What the key file holds: `None` when no key has been set up.
    ///
    /// # Errors
    ///
    /// When there is no state directory, or the file cannot be read or is
    /// not understood.
    pub fn approval_key(&self) -> Result<Option<KeyFile>, KeyRefusal> {
        let file = self
            .approval_key_file()
            .ok_or_else(|| KeyRefusal::Failed("no state directory is known".to_owned()))?;
        Ok(KeyFile::read(&file)?)
    }

    /// The person's key from the password they typed: refused when no key
    /// is set up, when the password does not make the key the file is of,
    /// or when this core's approvals are open with another key. A
    /// [`PersonKey`] signs; drop it as soon as it has.
    ///
    /// # Errors
    ///
    /// A [`KeyRefusal`] that says which.
    pub fn person_key(&self, password: &[u8]) -> Result<PersonKey, KeyRefusal> {
        let file = self.approval_key()?.ok_or(KeyRefusal::NoKey)?;
        let key = PersonKey::derive(password, &file)?;
        if !file.opens(&key) {
            return Err(KeyRefusal::NotThePassword {
                fingerprint: file.fingerprint(),
                made_at: file.made_at().clone(),
            });
        }
        match self.inner.person.verifier() {
            Some(open) if open != key.verifier() => Err(KeyRefusal::NotTheOpenKey),
            _ => Ok(key),
        }
    }

    /// Opens the person's approvals for every session of this core, with
    /// the key their typed password made: from now on the stores' signatures
    /// are checked with it. A later call with the same key does nothing.
    ///
    /// # Errors
    ///
    /// [`KeyRefusal::NotTheOpenKey`] when the approvals are open with
    /// another key.
    pub fn open_approvals(&self, key: &PersonKey) -> Result<(), KeyRefusal> {
        match self.inner.person.verifier() {
            Some(open) if open != key.verifier() => Err(KeyRefusal::NotTheOpenKey),
            Some(_) => Ok(()),
            None => {
                self.inner.person.open(key.verifier());
                Ok(())
            }
        }
    }

    /// Whether the person's approvals are open in this core.
    #[must_use]
    pub fn approvals_open(&self) -> bool {
        self.inner.person.verifier().is_some()
    }

    /// The key a session checks signatures with, when the approvals are
    /// open.
    pub(crate) fn verifier(&self) -> Option<Verifier> {
        self.inner.person.verifier()
    }

    /// Sets up the person's key from the password they chose, at the
    /// standard cost, and opens the approvals with it. Refused when a key
    /// is set up already.
    ///
    /// # Errors
    ///
    /// [`KeyRefusal::KeyExists`], [`KeyRefusal::TooShort`], or the file
    /// could not be written.
    pub fn set_up_approvals(&self, password: &[u8]) -> Result<PersonKey, KeyRefusal> {
        if self.approval_key()?.is_some() {
            return Err(KeyRefusal::KeyExists);
        }
        self.make_key(password, Cost::STANDARD)
    }

    /// Sets a new key from the password the person chose, after taking the
    /// signature off every approval made with the old one: see the [module
    /// documentation](self). Refusals of projects are kept. A folder or a
    /// link at the key file's name is moved aside.
    ///
    /// # Errors
    ///
    /// [`KeyRefusal::TooShort`] (nothing changed), or a store or the key
    /// file could not be written.
    pub fn reset_approvals(&self, password: &[u8]) -> Result<(PersonKey, ResetReport), KeyRefusal> {
        self.reset_approvals_at(password, Cost::STANDARD)
    }

    /// [`reset_approvals`](Self::reset_approvals), the new key made at
    /// `cost`: for tests, whose unoptimised builds take seconds at the
    /// standard cost. Nothing of the product calls it, so that every reset a
    /// person makes writes the standard cost (the review of the design).
    ///
    /// # Errors
    ///
    /// As [`reset_approvals`](Self::reset_approvals).
    #[doc(hidden)]
    pub fn reset_approvals_at(
        &self,
        password: &[u8],
        cost: Cost,
    ) -> Result<(PersonKey, ResetReport), KeyRefusal> {
        let state = self
            .state_dir()
            .ok_or_else(|| KeyRefusal::Failed("no state directory is known".to_owned()))?;
        // Checked before anything changes, so that a password too short
        // changes nothing.
        if String::from_utf8_lossy(password).chars().count() < MIN_PASSWORD_CHARS {
            return Err(KeyRefusal::TooShort);
        }
        let clock = Arc::clone(&self.inner.options.clock);
        // The stores first: a reset that stops half-way leaves approvals
        // unsigned under the old key, which ask again, never signed ones
        // that the new key would leave counting for nobody.
        let pins = PinStore::new(state.pins_file(), Arc::clone(&clock))
            .strip_signature()
            .map_err(|failure| KeyRefusal::Failed(failure.to_string()))?;
        let projects = TrustStore::new(state.trust_file(), clock)
            .strip_signatures()
            .map_err(|failure| KeyRefusal::Failed(failure.to_string()))?;
        move_aside(&state.root().join(KEY_FILE)).map_err(KeyRefusal::Failed)?;
        let key = self.make_key(password, cost)?;
        Ok((key, ResetReport { pins, projects }))
    }

    fn make_key(&self, password: &[u8], cost: Cost) -> Result<PersonKey, KeyRefusal> {
        let path = self
            .approval_key_file()
            .ok_or_else(|| KeyRefusal::Failed("no state directory is known".to_owned()))?;
        let now = self.inner.options.clock.now().to_timestamp();
        let (file, key) = KeyFile::make(password, fresh_salt()?, cost, now)?;
        file.write(&path, &*self.inner.options.clock)?;
        // A reset's new key replaces an open one: the old one checks
        // nothing that is left signed.
        self.inner.person.open(key.verifier());
        Ok(key)
    }
}

/// Moves what stands at the key file's name aside when it is not a file: a
/// folder or a link put there would keep a reset from writing the key.
fn move_aside(path: &Path) -> Result<(), String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "one look and one rename, at a reset the person asked for"
    )]
    let found = std::fs::symlink_metadata(path);
    match found {
        Ok(meta) if !meta.is_file() => {
            let aside = path.with_extension(format!("toml.aside-{}", std::process::id()));
            #[allow(
                clippy::disallowed_methods,
                reason = "one rename, at a reset the person asked for"
            )]
            let moved = std::fs::rename(path, &aside);
            moved.map_err(|failure| {
                format!(
                    "`{}` is not a file and could not be moved aside: {failure}",
                    path.display()
                )
            })
        }
        _ => Ok(()),
    }
}
