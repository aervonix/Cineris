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
//!   gate. Nor can another program keep the person from a reset: a store it
//!   made unreadable, or whose lock it holds, is left as it is and said,
//!   and a folder or link at a lock's or the key file's name is moved aside
//!   ([`Core::reset_approvals`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use cineris_base::private::{LOCK_WAIT, hold_lock_within, lock_path};
use cineris_base::{Clock, Rng, SystemRng};
use cineris_policy::signed::{
    Cost, KEY_FILE, KeyError, KeyFile, MIN_PASSWORD_CHARS, PersonKey, Verifier, fresh_salt,
};
use cineris_policy::{Checked, PinStore, TrustStore};
use cineris_protocol::ids::Timestamp;
use cineris_protocol::methods::TrustResourceKind;

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

/// What a reset took the signature off, and what it left as it was.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResetReport {
    /// How many of the user's own pins: each asks again.
    pub pins: usize,
    /// How many projects' trust: each is asked about again.
    pub projects: usize,
    /// Each store the reset left as it was, a sentence that names its file
    /// (under `~`) and why: it could not be read (another program wrote what
    /// this version does not understand, or put a folder or a link at its
    /// name), could not be written, or its lock was held past the wait. What
    /// it holds counts for nothing under the new key, and what it pins keeps
    /// its gates: it is never moved aside, which would lift them (the review
    /// of the permission path).
    pub left: Vec<String>,
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
    /// What the entry of the project asked about pins, signed or not: each
    /// extension and hook command by its kind (`extension`, `hook`), its
    /// name and the hash it was approved at. Not what counts: what a
    /// listing can say was approved before, for the password to confirm.
    pub project_pins: Vec<(String, String, cineris_base::ContentHash)>,
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
        let mut project_pins = Vec::new();
        if let Some(project) = project {
            let entry = TrustStore::new(state.trust_file(), Arc::clone(&self.inner.options.clock))
                .project(&project.key, None)
                .map_err(|failure| KeyRefusal::Failed(failure.to_string()))?;
            for kind in [TrustResourceKind::Extension, TrustResourceKind::Hook] {
                for (name, hash) in entry.held(&kind) {
                    project_pins.push((kind.as_str().to_owned(), name, hash));
                }
            }
        }
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
            project_pins,
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
    /// documentation](self). Refusals of projects are kept.
    ///
    /// Another program cannot keep the person from it (the reviews of the
    /// permission path and of the system's layer: a forgotten password's only
    /// way out is the reset), in this order:
    ///
    /// 1. A folder or a link at a lock's name is moved aside, and the key
    ///    file's lock is taken, [`LOCK_WAIT`] at most: held past it, nothing
    ///    changes.
    /// 2. The new key file is written under a new name beside the key
    ///    file's, which shows that it can be: if it cannot, nothing changes.
    /// 3. The signatures are taken off the stores. A store that cannot be
    ///    read or written, or whose lock is held past [`LOCK_WAIT`], is left
    ///    as it is and said in the report ([`ResetReport::left`]): what it
    ///    holds counts for nothing under the new key, and what it pins keeps
    ///    its gates. It is never moved aside, which would leave no store, and
    ///    lift them.
    /// 4. Whatever stands at the key file's name (a file, read-only or not,
    ///    a folder, a link) is moved aside, to a new name of its own, and
    ///    the new key file is put in its place.
    ///
    /// # Errors
    ///
    /// [`KeyRefusal::TooShort`] (nothing changed); the key file's lock held
    /// past [`LOCK_WAIT`], or the new key file not written (nothing
    /// changed); or what stands at the key file's name could not be moved
    /// aside, or the new one put in its place (the stores' signatures taken
    /// off: everything asks again, and the old key opens nothing that
    /// counts).
    pub fn reset_approvals(&self, password: &[u8]) -> Result<(PersonKey, ResetReport), KeyRefusal> {
        self.reset_approvals_at(password, Self::reset_cost())
    }

    /// The cost every reset a person makes writes: the standard one (the
    /// review of the design). Apart so that a test sees it without making a
    /// key at it, which takes seconds in an unoptimised build.
    #[doc(hidden)]
    #[must_use]
    pub fn reset_cost() -> Cost {
        Cost::STANDARD
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
        self.reset_approvals_within(password, cost, LOCK_WAIT)
    }

    /// [`reset_approvals_at`](Self::reset_approvals_at), each lock waited
    /// for `wait` at most: for tests, which hold a lock and do not wait
    /// [`LOCK_WAIT`] for it.
    ///
    /// # Errors
    ///
    /// As [`reset_approvals`](Self::reset_approvals).
    #[doc(hidden)]
    pub fn reset_approvals_within(
        &self,
        password: &[u8],
        cost: Cost,
        wait: Duration,
    ) -> Result<(PersonKey, ResetReport), KeyRefusal> {
        let state = self
            .state_dir()
            .ok_or_else(|| KeyRefusal::Failed("no state directory is known".to_owned()))?;
        // Checked before anything changes, so that a password too short
        // changes nothing.
        if String::from_utf8_lossy(password).chars().count() < MIN_PASSWORD_CHARS {
            return Err(KeyRefusal::TooShort);
        }
        let home = self.inner.options.env.home();
        let shown = |text: String, file: &Path| {
            crate::ext::found::with_paths_shown(&text, &[file], home.as_deref())
        };
        let key_file = state.root().join(KEY_FILE);
        // What stands at a lock's name and is not a file (a folder, a link)
        // would keep the reset from taking it: moved aside. A lock file
        // holds nothing; one another process holds is waited for.
        for file in [state.pins_file(), state.trust_file(), key_file.clone()] {
            let lock = lock_path(&file);
            if !is_file(&lock) {
                set_aside(&lock).map_err(|failure| KeyRefusal::Failed(shown(failure, &lock)))?;
            }
        }
        // The key file's lock, held to the end: without it no new key can be
        // put in place, and a reset that cannot finish changes nothing.
        let _key_lock = hold_lock_within(&key_file, wait).map_err(|failure| {
            KeyRefusal::Failed(shown(
                format!(
                    "the key of your approvals could not be set, so nothing was changed: {failure}"
                ),
                &key_file,
            ))
        })?;
        let clock = Arc::clone(&self.inner.options.clock);
        // The new key, written under a new name first: a key that cannot be
        // written changes nothing (the review of the system's layer: a
        // read-only key file planted at its name stopped a reset after it
        // had taken every store's signatures off).
        let now = clock.now().to_timestamp();
        let (file, key) = KeyFile::make(password, fresh_salt()?, cost, now)?;
        let staged = stage(&file, &key_file, &*clock).map_err(|failure| {
            KeyRefusal::Failed(shown(
                format!(
                    "the new key of your approvals could not be written, so nothing was changed: \
                     {failure}"
                ),
                &key_file,
            ))
        })?;
        let mut report = ResetReport::default();
        // The stores first: a reset that stops half-way leaves approvals
        // unsigned under the old key, which ask again. A store left as it is
        // keeps what the old key signed, which counts for nothing under the
        // new one.
        let pins_file = state.pins_file();
        let pins = free_within(&pins_file, wait).and_then(|()| {
            PinStore::new(&pins_file, Arc::clone(&clock))
                .strip_signature()
                .map_err(|failure| failure.to_string())
        });
        match pins {
            Ok(count) => report.pins = count,
            Err(failure) => report.left.push(shown(
                format!(
                    "Your approvals of your own extensions were left as they were, and count for \
                     nothing under your new password: {failure}"
                ),
                &pins_file,
            )),
        }
        let trust_file = state.trust_file();
        let projects = free_within(&trust_file, wait).and_then(|()| {
            TrustStore::new(&trust_file, clock)
                .strip_signatures()
                .map_err(|failure| failure.to_string())
        });
        match projects {
            Ok(count) => report.projects = count,
            Err(failure) => report.left.push(shown(
                format!(
                    "The trust store was left as it was, and no project's trust in it counts \
                     under your new password: {failure}"
                ),
                &trust_file,
            )),
        }
        // Whatever stands at the key file's name, a file the reset cannot
        // replace included, goes aside, and the new key takes its place.
        let in_place = set_aside(&key_file).and_then(|_| {
            #[allow(
                clippy::disallowed_methods,
                reason = "one rename, at a reset the person asked for"
            )]
            let renamed = std::fs::rename(&staged, &key_file);
            renamed.map_err(|failure| format!("it could not be put in place: {failure}"))
        });
        if let Err(failure) = in_place {
            let _ = remove(&staged);
            return Err(KeyRefusal::Failed(shown(
                format!(
                    "the signatures were taken off your approvals, and the new key could not be \
                     set, so everything asks again and nothing counts: {failure}. Run the reset \
                     again"
                ),
                &key_file,
            )));
        }
        // A reset's new key replaces an open one: the old one checks
        // nothing that is left signed.
        self.inner.person.open(key.verifier());
        Ok((key, report))
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

/// Whether the lock of the store at `file` can be had within `wait`: taken,
/// and let go at once, for the store's own change to take again. A lock
/// another program holds past `wait` is said, with its file, and the store
/// is left as it is (the review of the permission path: a lock held by a
/// process left running kept the reset waiting). One taken in between is
/// waited for [`LOCK_WAIT`] at most by the store's change, then said.
fn free_within(file: &Path, wait: Duration) -> Result<(), String> {
    hold_lock_within(file, wait)
        .map(drop)
        .map_err(|failure| failure.to_string())
}

/// Whether `path` is a regular file, or nothing: a link is not followed.
fn is_file(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "one look, at a reset the person asked for"
    )]
    let found = std::fs::symlink_metadata(path);
    found.map_or(true, |found| found.is_file())
}

/// `path` with `.<what>-` and sixteen random hexadecimal digits added: a name
/// beside it that no other program can have made first.
fn beside(path: &Path, what: &str, rng: &mut SystemRng) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{what}-{:016x}", rng.next_u64()));
    PathBuf::from(name)
}

/// Moves whatever stands at `path` aside (a file, read-only or not, a
/// folder, a link, which is not followed) to a new name beside it, and gives
/// that name; `None` when nothing stands there. Only the key file's name and
/// the locks' are moved aside, never a store's: one that cannot be read is
/// left as it is.
fn set_aside(path: &Path) -> Result<Option<PathBuf>, String> {
    #[allow(
        clippy::disallowed_methods,
        reason = "a look and a rename, at a reset the person asked for"
    )]
    let moved = |to: &Path| std::fs::rename(path, to);
    #[allow(
        clippy::disallowed_methods,
        reason = "a look, at a reset the person asked for"
    )]
    let there = |path: &Path| std::fs::symlink_metadata(path);
    if there(path).is_err() {
        return Ok(None);
    }
    let mut rng = SystemRng::new();
    let mut last = None;
    for _ in 0..16 {
        let aside = beside(path, "aside", &mut rng);
        // A random name is free unless something chose the same 64 bits.
        if there(&aside).is_ok() {
            continue;
        }
        match moved(&aside) {
            Ok(()) => return Ok(Some(aside)),
            Err(failure) => last = Some(failure),
        }
    }
    Err(match last {
        Some(failure) => format!("it could not be moved aside: {failure}"),
        None => "no name beside it was free to move it aside to".to_owned(),
    })
}

/// Writes `file` under a new name beside `key_file`, owner-only, and gives
/// that name: what a reset puts in the key file's place once the stores'
/// signatures are off. The lock file its write made beside it is removed.
fn stage(file: &KeyFile, key_file: &Path, clock: &dyn Clock) -> Result<PathBuf, String> {
    let mut rng = SystemRng::new();
    for _ in 0..16 {
        let staged = beside(key_file, "new", &mut rng);
        if exists(&staged) {
            continue;
        }
        let written = file.write(&staged, clock);
        let _ = remove(&lock_path(&staged));
        return match written {
            Ok(()) => Ok(staged),
            Err(failure) => {
                let _ = remove(&staged);
                Err(failure.to_string())
            }
        };
    }
    Err("no name beside the key file was free to write the new key under".to_owned())
}

/// Whether anything stands at `path`, a link included.
fn exists(path: &Path) -> bool {
    #[allow(
        clippy::disallowed_methods,
        reason = "one look, at a reset the person asked for"
    )]
    let found = std::fs::symlink_metadata(path);
    found.is_ok()
}

/// Removes the file at `path`, best effort: a reset's own leftovers.
fn remove(path: &Path) -> std::io::Result<()> {
    #[allow(
        clippy::disallowed_methods,
        reason = "one removal of the reset's own file"
    )]
    let removed = std::fs::remove_file(path);
    removed
}
