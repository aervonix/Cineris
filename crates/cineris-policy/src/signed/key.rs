//! The person's key: made from their password, never stored (the signed
//! approvals step).
//!
//! # The key file
//!
//! `approval-key.toml` in the state directory holds what the key is made
//! with, beside the password, and nothing a session trusts:
//!
//! ```toml
//! version = 1
//! kdf = "argon2id"
//! memory_kib = 65536
//! passes = 3
//! lanes = 1
//! salt = "9f2c…"        # 16 random bytes
//! public_key = "5e88…"  # what the password makes
//! made_at = "2026-11-27T10:15:02Z"
//! ```
//!
//! - **`public_key` is only for telling a mistyped password.** A session
//!   checks with the key the typed password makes ([`PersonKey::verifier`]),
//!   so another program that writes a key of its own here makes the
//!   person's password fail to open their approvals, a denial, and opens
//!   nothing of its own.
//! - **Read strictly,** as the stores are: a key it does not know, another
//!   version, a cost past the caps ([`Cost::new`]) or a file of more than
//!   4 KiB refuses the whole file, and with it every approval: nothing is
//!   checked against a key that cannot be made.
//! - **Written owner-only and atomically,** under its own lock, through no
//!   link.
//! - **The cost is the file's,** so that a later version can raise it at
//!   the next set-up or reset. A cost lowered in the file by another
//!   program makes another key from the same password, which opens nothing.
//!
//! # Making the key
//!
//! Argon2id of the password and the salt, at the file's cost, gives 32
//! bytes; HKDF-SHA-256 of them, with a label of its own, gives the Ed25519
//! seed. Both are wiped when dropped. The key pair is `ring`'s, which does
//! not wipe its own copy of the private scalar when dropped: a
//! [`PersonKey`] is made at a yes and dropped as soon as it has signed.

#![allow(
    clippy::disallowed_methods,
    reason = "blocking file calls; the key file is read at a yes and at a session's opening"
)]

use std::fmt;
use std::io;
use std::path::Path;

use cineris_base::{AtomicWriteError, AtomicWriteOptions, Clock, OpenError, write_atomic};
use cineris_protocol::ids::Timestamp;
use ring::signature::KeyPair as _;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::{Signature, Statement, Verifier, from_hex, to_hex};
use crate::store_lock;

/// The key file's name in the state directory.
pub const KEY_FILE: &str = "approval-key.toml";

/// The fewest characters (Unicode scalar values) a password may have when
/// it is chosen: 12, the maintainers' decision of 2026-11-27 after the
/// review of the design (the stores and the key file are enough to test a
/// guess offline, at about 13 million guesses a day on the reference
/// machine at the standard cost).
pub const MIN_PASSWORD_CHARS: usize = 12;

/// The version of the file's form.
const VERSION: u32 = 1;

/// The only key derivation this version knows.
const KDF: &str = "argon2id";

/// The largest file read.
const MAX_BYTES: u64 = 4 * 1024;

/// The label of the HKDF step: what the Argon2id output is turned into.
const SEED_LABEL: &[u8] = b"cineris approvals: the Ed25519 seed";

/// What making the key costs: Argon2id's memory in KiB, its passes over
/// that memory, and its lanes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cost {
    memory_kib: u32,
    passes: u32,
    lanes: u32,
}

impl Cost {
    /// What set-up and reset write: 64 MiB, 3 passes, 1 lane (about 160 ms
    /// on the second machine, measured before building). RFC 9106's second
    /// recommendation with one lane, since the key is made on one thread.
    pub const STANDARD: Self = Self {
        memory_kib: 64 * 1024,
        passes: 3,
        lanes: 1,
    };

    /// The most memory a file may ask for: 256 MiB, four times the
    /// standard.
    pub const MAX_MEMORY_KIB: u32 = 256 * 1024;

    /// The most passes a file may ask for.
    pub const MAX_PASSES: u32 = 12;

    /// The most lanes a file may ask for.
    pub const MAX_LANES: u32 = 4;

    /// The most work (memory times passes, in KiB) a file may ask for: four
    /// times the standard. Set-up and reset always write the standard, so
    /// this leaves room to raise it later and keeps a planted file from
    /// making each unlock and each yes take seconds (the OS layer's
    /// review, F4: at the old caps, about 14 s and 1 GiB).
    pub const MAX_WORK_KIB: u64 = 4 * 64 * 1024 * 3;

    /// A cost, within the caps above and Argon2's own floor (8 KiB for each
    /// lane, one pass). The caps keep a planted file from making a session
    /// take minutes or gigabytes to fail.
    ///
    /// # Errors
    ///
    /// Says which part is out of bounds.
    pub fn new(memory_kib: u32, passes: u32, lanes: u32) -> Result<Self, String> {
        if !(1..=Self::MAX_LANES).contains(&lanes) {
            return Err(format!(
                "{lanes} lanes, where 1 to {} are read",
                Self::MAX_LANES
            ));
        }
        if !(1..=Self::MAX_PASSES).contains(&passes) {
            return Err(format!(
                "{passes} passes, where 1 to {} are read",
                Self::MAX_PASSES
            ));
        }
        let floor = 8 * lanes;
        if !(floor..=Self::MAX_MEMORY_KIB).contains(&memory_kib) {
            return Err(format!(
                "{memory_kib} KiB of memory, where {floor} to {} are read",
                Self::MAX_MEMORY_KIB
            ));
        }
        let work = u64::from(memory_kib) * u64::from(passes);
        if work > Self::MAX_WORK_KIB {
            return Err(format!(
                "{memory_kib} KiB over {passes} passes, more work than {} KiB",
                Self::MAX_WORK_KIB
            ));
        }
        Ok(Self {
            memory_kib,
            passes,
            lanes,
        })
    }

    /// Argon2's memory, in KiB.
    #[must_use]
    pub fn memory_kib(&self) -> u32 {
        self.memory_kib
    }

    /// Argon2's passes.
    #[must_use]
    pub fn passes(&self) -> u32 {
        self.passes
    }

    /// Argon2's lanes.
    #[must_use]
    pub fn lanes(&self) -> u32 {
        self.lanes
    }
}

/// The salt the key is made with: 16 random bytes, made at set-up and at
/// each reset ([`fresh_salt`]).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Salt([u8; Self::LEN]);

impl Salt {
    /// Its length in bytes.
    pub const LEN: usize = 16;

    /// A salt of these bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// Its bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }
}

impl fmt::Debug for Salt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Salt({})", to_hex(&self.0))
    }
}

/// A new salt from the system's random source.
///
/// # Errors
///
/// When the system gives no random bytes.
pub fn fresh_salt() -> Result<Salt, KeyError> {
    let mut bytes = [0; Salt::LEN];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| KeyError::Random)?;
    Ok(Salt(bytes))
}

/// Why the key could not be read, written or made.
#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    /// The key file could not be read.
    #[error(
        "the approvals key `{path}` could not be read, so no approval can be checked: {source}"
    )]
    Read {
        /// The file.
        path: String,
        /// Why.
        source: io::Error,
    },
    /// The key file is not one this build understands in full.
    #[error(
        "the approvals key `{path}` is not understood, so no approval can be checked: {problem}. \
         `cineris approvals reset` makes a new key, and drops every approval"
    )]
    NotUnderstood {
        /// The file.
        path: String,
        /// What is wrong with it.
        problem: String,
    },
    /// The key file could not be written.
    #[error(transparent)]
    Write(#[from] AtomicWriteError),
    /// A password chosen with too few characters.
    #[error("a password for your approvals has at least {MIN_PASSWORD_CHARS} characters")]
    PasswordTooShort,
    /// Argon2 refused what it was given.
    #[error("the approvals key could not be made: {0}")]
    Derive(String),
    /// The system gave no random bytes.
    #[error("the system gave no random bytes for a new approvals key")]
    Random,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnDisk {
    version: u32,
    kdf: String,
    memory_kib: u32,
    passes: u32,
    lanes: u32,
    salt: String,
    public_key: String,
    made_at: Timestamp,
}

/// What the person's key is made with: see the [module
/// documentation](self).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFile {
    cost: Cost,
    salt: Salt,
    /// The public key the chosen password made: only compared, never
    /// checked with (a [`Verifier`] cannot be made from it).
    public_key: [u8; Verifier::LEN],
    made_at: Timestamp,
}

impl KeyFile {
    /// A new key file for `password`, and the key it makes: at set-up and
    /// at a reset. The caller gives the salt ([`fresh_salt`]) and the cost
    /// ([`Cost::STANDARD`]; tests give a smaller one).
    ///
    /// # Errors
    ///
    /// When the password has fewer than [`MIN_PASSWORD_CHARS`] characters,
    /// or Argon2 refuses.
    pub fn make(
        password: &[u8],
        salt: Salt,
        cost: Cost,
        made_at: Timestamp,
    ) -> Result<(Self, PersonKey), KeyError> {
        let chars = String::from_utf8_lossy(password).chars().count();
        if chars < MIN_PASSWORD_CHARS {
            return Err(KeyError::PasswordTooShort);
        }
        let key = PersonKey::make(password, &salt, cost)?;
        let file = Self {
            cost,
            salt,
            public_key: key.verifier().0,
            made_at,
        };
        Ok((file, key))
    }

    /// The cost the key is made at.
    #[must_use]
    pub fn cost(&self) -> Cost {
        self.cost
    }

    /// The first eight bytes of the key the chosen password made, in
    /// hexadecimal: for telling a person which key the file is of, when
    /// the password they typed does not open it.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        to_hex(&self.public_key[..8])
    }

    /// When the key was made.
    #[must_use]
    pub fn made_at(&self) -> &Timestamp {
        &self.made_at
    }

    /// Whether `key` is the one the file was written for: the typed
    /// password is the one chosen. Under the password anchor this only
    /// tells a mistyped password; what a session then checks with is
    /// `key`'s own [`PersonKey::verifier`].
    #[must_use]
    pub fn opens(&self, key: &PersonKey) -> bool {
        key.verifier().0 == self.public_key
    }

    /// Reads the key file at `path`. `None` when there is none: no key has
    /// been made.
    ///
    /// # Errors
    ///
    /// When it cannot be read, or is not understood in full.
    pub fn read(path: &Path) -> Result<Option<Self>, KeyError> {
        let read_error = |source| KeyError::Read {
            path: path.display().to_string(),
            source,
        };
        let not_understood = |problem: String| KeyError::NotUnderstood {
            path: path.display().to_string(),
            problem,
        };
        let (opened, found) = match cineris_base::open_regular(path) {
            Ok(opened) => opened,
            Err(OpenError::NotFound) => return Ok(None),
            Err(OpenError::Io(error)) => return Err(read_error(error)),
            Err(other) => return Err(read_error(io::Error::other(other.to_string()))),
        };
        if found.len() > MAX_BYTES {
            return Err(not_understood(format!(
                "it is {} bytes, more than a key file could be",
                found.len()
            )));
        }
        let mut text = String::new();
        io::Read::read_to_string(&mut io::Read::take(opened, MAX_BYTES), &mut text)
            .map_err(read_error)?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let disk: OnDisk =
            toml::from_str(text).map_err(|error| not_understood(error.message().to_owned()))?;
        if disk.version != VERSION {
            return Err(not_understood(format!(
                "its version is {}, and this version of Cineris reads version {VERSION}",
                disk.version
            )));
        }
        if disk.kdf != KDF {
            return Err(not_understood(format!(
                "its key is made with `{}`, which this version does not know",
                disk.kdf.escape_debug()
            )));
        }
        let cost = Cost::new(disk.memory_kib, disk.passes, disk.lanes)
            .map_err(|problem| not_understood(format!("it asks for {problem}")))?;
        let salt = from_hex::<{ Salt::LEN }>(&disk.salt)
            .map(Salt)
            .ok_or_else(|| not_understood("its salt is not 32 hexadecimal digits".to_owned()))?;
        let public_key = from_hex::<{ Verifier::LEN }>(&disk.public_key).ok_or_else(|| {
            not_understood("its public key is not 64 hexadecimal digits".to_owned())
        })?;
        // `made_at` is printed to the person when their password does not
        // open the key, so it must be a time and nothing else: a planted
        // file must not put lines of its own before them (the permission
        // path's review, finding 7).
        cineris_base::UtcTime::parse(disk.made_at.as_str()).map_err(|_| {
            not_understood("its `made_at` is not a time in UTC (`2026-11-27T10:15:02Z`)".to_owned())
        })?;
        Ok(Some(Self {
            cost,
            salt,
            public_key,
            made_at: disk.made_at,
        }))
    }

    /// Writes the key file at `path`, owner-only and atomically, under its
    /// lock, making the state directory owner-only if it is missing. A link
    /// at its name is not written through.
    ///
    /// # Errors
    ///
    /// When the lock cannot be taken or the file written.
    pub fn write(&self, path: &Path, clock: &dyn Clock) -> Result<(), KeyError> {
        let _lock = store_lock::hold(path).map_err(|source| AtomicWriteError::Io {
            path: path.display().to_string(),
            step: "lock",
            source,
        })?;
        let disk = OnDisk {
            version: VERSION,
            kdf: KDF.to_owned(),
            memory_kib: self.cost.memory_kib,
            passes: self.cost.passes,
            lanes: self.cost.lanes,
            salt: to_hex(&self.salt.0),
            public_key: to_hex(&self.public_key),
            made_at: self.made_at.clone(),
        };
        let text = toml::to_string(&disk).map_err(|error| KeyError::NotUnderstood {
            path: path.display().to_string(),
            problem: format!("it could not be written out: {error}"),
        })?;
        let options = AtomicWriteOptions {
            owner_only: true,
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        write_atomic(path, text.as_bytes(), &options, clock)?;
        Ok(())
    }
}

/// The person's signing key, made from their password at a yes
/// ([`PersonKey::derive`]) and dropped as soon as it has signed. Not
/// `Clone`; its `Debug` names only its public key.
pub struct PersonKey {
    pair: ring::signature::Ed25519KeyPair,
}

impl PersonKey {
    /// The key `password` makes with `file`'s salt and cost. Whether it is
    /// the key the file was written for is [`KeyFile::opens`]; under the
    /// password anchor a session checks with this key's own
    /// [`verifier`](Self::verifier) whatever the file says.
    ///
    /// # Errors
    ///
    /// When Argon2 refuses.
    pub fn derive(password: &[u8], file: &KeyFile) -> Result<Self, KeyError> {
        Self::make(password, &file.salt, file.cost)
    }

    fn make(password: &[u8], salt: &Salt, cost: Cost) -> Result<Self, KeyError> {
        let params = argon2::Params::new(cost.memory_kib, cost.passes, cost.lanes, Some(32))
            .map_err(|error| KeyError::Derive(error.to_string()))?;
        let argon =
            argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
        let mut stretched = Zeroizing::new([0_u8; 32]);
        argon
            .hash_password_into(password, &salt.0, &mut *stretched)
            .map_err(|error| KeyError::Derive(error.to_string()))?;
        let mut seed = Zeroizing::new([0_u8; 32]);
        ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, SEED_LABEL)
            .extract(&*stretched)
            .expand(&[b"v1"], Seed)
            .and_then(|okm| okm.fill(&mut *seed))
            .map_err(|_| KeyError::Derive("HKDF refused".to_owned()))?;
        let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(&*seed)
            .map_err(|error| KeyError::Derive(error.to_string()))?;
        Ok(Self { pair })
    }

    /// The key's public half: what a session checks this key's signatures
    /// with.
    #[must_use]
    pub fn verifier(&self) -> Verifier {
        let mut bytes = [0; Verifier::LEN];
        bytes.copy_from_slice(self.pair.public_key().as_ref());
        Verifier(bytes)
    }

    /// Signs `statement`.
    #[must_use]
    pub fn sign(&self, statement: &Statement) -> Signature {
        let mut bytes = [0; Signature::LEN];
        bytes.copy_from_slice(self.pair.sign(statement.bytes()).as_ref());
        Signature(bytes)
    }
}

impl fmt::Debug for PersonKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersonKey")
            .field("verifier", &self.verifier())
            .finish_non_exhaustive()
    }
}

/// HKDF's output length: one Ed25519 seed.
struct Seed;

impl ring::hkdf::KeyType for Seed {
    fn len(&self) -> usize {
        32
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cineris_base::FakeClock;

    use super::*;
    use crate::store_lock::tests::Scratch;

    fn cheap() -> Cost {
        Cost::new(64, 1, 1).unwrap()
    }

    fn at() -> Timestamp {
        Timestamp::new("2026-11-27T10:00:00Z")
    }

    #[test]
    fn a_password_shorter_than_twelve_characters_is_refused() {
        let salt = Salt::from_bytes([1; Salt::LEN]);
        assert!(matches!(
            KeyFile::make(b"12345678901", salt, cheap(), at()),
            Err(KeyError::PasswordTooShort)
        ));
        assert!(KeyFile::make(b"123456789012", salt, cheap(), at()).is_ok());
        // Characters, not bytes: eleven letters of two bytes each are eleven.
        assert!(matches!(
            KeyFile::make("ééééééééééé".as_bytes(), salt, cheap(), at()),
            Err(KeyError::PasswordTooShort)
        ));
        assert!(KeyFile::make("éééééééééééé".as_bytes(), salt, cheap(), at()).is_ok());
    }

    #[test]
    fn a_cost_past_its_caps_is_refused() {
        assert!(Cost::new(Cost::MAX_MEMORY_KIB, 3, Cost::MAX_LANES).is_ok());
        assert!(Cost::new(64 * 1024, Cost::MAX_PASSES, 1).is_ok());
        assert!(
            Cost::new(Cost::MAX_MEMORY_KIB, Cost::MAX_PASSES, 1).is_err(),
            "the most memory and the most passes together are more work than the cap"
        );
        assert!(Cost::new(Cost::STANDARD.memory_kib(), Cost::STANDARD.passes(), 1).is_ok());
        assert!(Cost::new(Cost::MAX_MEMORY_KIB + 1, 1, 1).is_err());
        assert!(Cost::new(64, Cost::MAX_PASSES + 1, 1).is_err());
        assert!(Cost::new(64, 1, Cost::MAX_LANES + 1).is_err());
        assert!(Cost::new(64, 0, 1).is_err());
        assert!(Cost::new(64, 1, 0).is_err());
        assert!(Cost::new(15, 1, 2).is_err(), "8 KiB for each lane");
        assert!(Cost::new(16, 1, 2).is_ok());
    }

    #[test]
    fn a_key_file_is_written_and_read_back_strictly() {
        let scratch = Scratch::new();
        let path = scratch.0.join(KEY_FILE);
        assert!(KeyFile::read(&path).unwrap().is_none(), "none made yet");
        let (file, key) =
            KeyFile::make(b"correct horse", fresh_salt().unwrap(), cheap(), at()).unwrap();
        let clock = Arc::new(FakeClock::starting_at("2026-11-27T10:00:00Z").unwrap());
        file.write(&path, &*clock).unwrap();
        let read = KeyFile::read(&path).unwrap().unwrap();
        assert_eq!(read, file);
        assert!(read.opens(&PersonKey::derive(b"correct horse", &read).unwrap()));
        assert!(read.opens(&key));
        assert_eq!(read.fingerprint(), key.verifier().fingerprint());

        let text = std::fs::read_to_string(&path).unwrap();
        let refused = |text: String| {
            std::fs::write(&path, text).unwrap();
            assert!(
                matches!(KeyFile::read(&path), Err(KeyError::NotUnderstood { .. })),
                "{}",
                std::fs::read_to_string(&path).unwrap()
            );
        };
        refused(text.replace("version = 1", "version = 2"));
        refused(text.replace("argon2id", "pbkdf2"));
        refused(text.replace("memory_kib = 64", "memory_kib = 9999999"));
        refused(format!("{text}extra = 1\n"));
        refused(text.replace("salt = \"", "salt = \"0"));
        refused(text.replace("public_key = \"", "public_key = \"0"));
        // A `made_at` that is not a time, lines of its own included.
        refused(text.replace(
            "made_at = \"2026-11-27T10:00:00Z\"",
            "made_at = \"now\\ntype your password again below\"",
        ));
        refused("x".repeat(5000));
    }

    #[test]
    fn a_lowered_cost_makes_another_key() {
        let (file, key) = KeyFile::make(
            b"correct horse",
            Salt::from_bytes([3; 16]),
            Cost::new(128, 2, 1).unwrap(),
            at(),
        )
        .unwrap();
        let mut lowered = file.clone();
        lowered.cost = cheap();
        let again = PersonKey::derive(b"correct horse", &lowered).unwrap();
        assert_ne!(again.verifier(), key.verifier());
        assert!(!file.opens(&again));
    }

    #[test]
    fn a_key_prints_only_its_public_half() {
        let (_, key) =
            KeyFile::make(b"correct horse", Salt::from_bytes([4; 16]), cheap(), at()).unwrap();
        let printed = format!("{key:?}");
        assert!(printed.contains(&key.verifier().fingerprint()), "{printed}");
        assert!(!printed.contains("correct"), "{printed}");
    }
}
