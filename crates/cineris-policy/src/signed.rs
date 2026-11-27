//! Approvals signed with a key only the person's password makes (the
//! signed approvals step; `docs/architecture/06-permissions-and-sandbox.md`,
//! "Signed approvals", and `docs/security.md`, section 3).
//!
//! Where nothing confines the model's commands, a program with the user's
//! rights can write every store that approves for the person. From this
//! step an approval counts only with a signature that checks against the
//! person's key, and that key is made, each time it is needed, from the
//! password the person types: Argon2id of the password and a salt, then
//! HKDF-SHA-256, gives an Ed25519 seed ([`PersonKey::derive`]). Nothing a
//! session trusts is stored: it checks with the key the typed password
//! makes ([`Verifier`]), so a key file replaced by another program opens
//! nothing.
//!
//! # What is signed
//!
//! A [`Statement`]: the bytes of one unit of a store as it is to be written
//! (a project's trust, the user's pins), with a domain label, the unit's
//! kind, and each field with its length before it, so that no two
//! statements of different content, or of different kinds, have the same
//! bytes. Each store builds its own statements from what it writes
//! ([`Statement::build`]), and checks a [`Signature`] against them with the
//! session's [`Verifier`].
//!
//! # What is not here
//!
//! - **Asking the person.** The front end asks for the password and makes
//!   the [`PersonKey`]; nothing here reads a terminal.
//! - **Keeping a key.** A [`PersonKey`] is made at a yes, signs, and is
//!   dropped. A session keeps only its [`Verifier`], which signs nothing.
//! - **The stores.** Each store says what of it is signed and how
//!   ([`crate::trust`], [`crate::approvals`]).

use std::fmt;

use serde::{Deserialize, Serialize};

mod key;

pub use key::{Cost, KEY_FILE, KeyError, KeyFile, MIN_PASSWORD_CHARS, PersonKey, Salt, fresh_salt};

/// The label every statement starts with: what these bytes are, and which
/// version of the form they are in.
const DOMAIN: &[u8] = b"cineris approval v1\0";

/// What a statement is of: which unit of which store. The kind is part of
/// the signed bytes, so a signature over one kind is never taken for
/// another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Unit {
    /// The user's own pins (`pins.toml`), all of them.
    UserPins,
    /// One project's trust and everything pinned with it (`trust.toml`).
    Project,
    /// One project's stored approvals (`approvals/<key>.toml`).
    StoredApprovals,
    /// One project's accepted entries of what was learned
    /// (`projects/<key>/knowledge.toml`).
    Knowledge,
    /// What an approved extension registered, beside its pin.
    Registration,
}

impl Unit {
    /// The unit's label in the signed bytes. Never changed once written:
    /// a change would make every signature of the unit fail.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::UserPins => "user pins",
            Self::Project => "project",
            Self::StoredApprovals => "stored approvals",
            Self::Knowledge => "knowledge",
            Self::Registration => "registration",
        }
    }
}

/// The bytes a signature is made over: see the [module
/// documentation](self). Made only by [`Statement::build`], so that every
/// statement has the domain label and its unit first.
#[derive(Clone, PartialEq, Eq)]
pub struct Statement {
    bytes: Vec<u8>,
}

impl Statement {
    /// A statement of `unit`, to which fields are added in a fixed order.
    #[must_use]
    pub fn build(unit: Unit) -> StatementBuilder {
        let mut fields = Fields::default();
        fields.bytes.extend_from_slice(DOMAIN);
        fields.text(unit.label());
        StatementBuilder { fields }
    }

    /// The bytes signed.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Statement({} bytes, {})",
            self.bytes.len(),
            cineris_base::ContentHash::of(&self.bytes)
        )
    }
}

/// Fields written with their lengths: what [`StatementBuilder`] and an
/// entry of it share.
#[derive(Debug, Default)]
pub struct Fields {
    bytes: Vec<u8>,
}

impl Fields {
    /// A field of bytes.
    pub fn bytes(&mut self, value: &[u8]) -> &mut Self {
        self.bytes
            .extend_from_slice(&(value.len() as u64).to_le_bytes());
        self.bytes.extend_from_slice(value);
        self
    }

    /// A field of text.
    pub fn text(&mut self, value: &str) -> &mut Self {
        self.bytes(value.as_bytes())
    }

    /// A field that is a number.
    pub fn number(&mut self, value: u64) -> &mut Self {
        self.bytes(&value.to_le_bytes())
    }

    /// A field that is a yes or a no.
    pub fn flag(&mut self, value: bool) -> &mut Self {
        self.bytes(&[u8::from(value)])
    }
}

/// A [`Statement`] being built: [`Statement::build`].
#[derive(Debug)]
pub struct StatementBuilder {
    fields: Fields,
}

impl StatementBuilder {
    /// Adds a field of bytes.
    #[must_use]
    pub fn bytes(mut self, value: &[u8]) -> Self {
        self.fields.bytes(value);
        self
    }

    /// Adds a field of text.
    #[must_use]
    pub fn text(mut self, value: &str) -> Self {
        self.fields.text(value);
        self
    }

    /// Adds a field that is a number.
    #[must_use]
    pub fn number(mut self, value: u64) -> Self {
        self.fields.number(value);
        self
    }

    /// Adds a field that is a yes or a no.
    #[must_use]
    pub fn flag(mut self, value: bool) -> Self {
        self.fields.flag(value);
        self
    }

    /// Adds a set of entries: how many there are, then each entry's fields,
    /// as one field each. The caller gives them in the set's own order
    /// (sorted), so that one set is always the same bytes.
    #[must_use]
    pub fn entries<T>(mut self, entries: &[T], write: impl Fn(&T, &mut Fields)) -> Self {
        self.fields.number(entries.len() as u64);
        for entry in entries {
            let mut fields = Fields::default();
            write(entry, &mut fields);
            self.fields.bytes(&fields.bytes);
        }
        self
    }

    /// The statement.
    #[must_use]
    pub fn finish(self) -> Statement {
        Statement {
            bytes: self.fields.bytes,
        }
    }
}

/// An Ed25519 signature over a [`Statement`]: 64 bytes, written as 128
/// lowercase hexadecimal digits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature([u8; Self::LEN]);

impl Signature {
    /// Its length in bytes.
    pub const LEN: usize = 64;

    /// A signature of these bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; Self::LEN]) -> Self {
        Self(bytes)
    }

    /// Its bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; Self::LEN] {
        &self.0
    }

    /// Reads the form [`Display`](fmt::Display) writes. `None` for
    /// anything else, upper case included.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        from_hex(text).map(Self)
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&to_hex(&self.0))
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({}…)", to_hex(&self.0[..4]))
    }
}

impl Serialize for Signature {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_hex(&text).ok_or_else(|| {
            serde::de::Error::custom("a signature is 128 lowercase hexadecimal digits")
        })
    }
}

/// The person's public key: what a session checks signatures with. It
/// signs nothing.
///
/// **It can be made only from a [`PersonKey`]** ([`PersonKey::verifier`]),
/// which only a typed password makes: there is no way to read one from a
/// file, from bytes or from text, so that no call can take what a session
/// checks with from something another program wrote (the review of the
/// design, finding 10). The key file's record of the key is kept apart, and
/// can only say whether a typed password is the chosen one
/// ([`KeyFile::opens`]).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Verifier([u8; Self::LEN]);

impl Verifier {
    /// Its length in bytes.
    pub const LEN: usize = 32;

    /// Whether `signature` is this key's, over exactly `statement`.
    #[must_use]
    pub fn verify(&self, statement: &Statement, signature: &Signature) -> bool {
        ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &self.0)
            .verify(&statement.bytes, &signature.0)
            .is_ok()
    }

    /// The first eight bytes, in hexadecimal: enough for a person to tell
    /// two keys apart.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        to_hex(&self.0[..8])
    }
}

impl fmt::Debug for Verifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Verifier({})", self.fingerprint())
    }
}

/// One pin of a unit, as it is signed: its kind, its name, the hash of
/// what was approved, and the hash of what it registered, if known.
pub(crate) struct SignedPin<'a> {
    pub(crate) kind: &'a str,
    pub(crate) name: &'a str,
    pub(crate) hash: [u8; 32],
    pub(crate) registration: Option<[u8; 32]>,
}

/// The statement of a project's entry: its key, revision, root, whether git
/// is trusted, and its pins in the order of kind and name. The store and
/// [`statement_of`] both make it here, so that they cannot differ.
pub(crate) fn project_statement(
    key: &str,
    revision: u64,
    root: &str,
    git_trusted: bool,
    pins: &mut [SignedPin<'_>],
) -> Statement {
    pins.sort_by(|a, b| (a.kind, a.name).cmp(&(b.kind, b.name)));
    Statement::build(Unit::Project)
        .text(key)
        .number(revision)
        .text(root)
        .flag(git_trusted)
        .entries(pins, write_pin)
        .finish()
}

/// The statement of the user's pins: the revision, and the pins in the
/// order of kind and name.
pub(crate) fn user_pins_statement(revision: u64, pins: &mut [SignedPin<'_>]) -> Statement {
    pins.sort_by(|a, b| (a.kind, a.name).cmp(&(b.kind, b.name)));
    Statement::build(Unit::UserPins)
        .number(revision)
        .entries(pins, write_pin)
        .finish()
}

fn write_pin(pin: &SignedPin<'_>, fields: &mut Fields) {
    fields
        .text(pin.kind)
        .text(pin.name)
        .bytes(&pin.hash)
        .bytes(pin.registration.as_ref().map_or(&[][..], |hash| &hash[..]));
}

/// The statement a proposal of the protocol is signed as: the bytes a front
/// end signs at the person's yes, made exactly as the store makes them.
///
/// # Errors
///
/// When a hash is not one (64 lowercase hexadecimal digits), or two pins
/// have one kind and name.
pub fn statement_of(
    proposal: &cineris_protocol::methods::ApprovalProposal,
) -> Result<Statement, String> {
    use cineris_protocol::methods::{ApprovalProposal, ProposedPin};
    // A function, not a closure: the pins it gives borrow from what it is
    // given, which a closure's signature cannot say.
    fn read(pins: &[ProposedPin]) -> Result<Vec<SignedPin<'_>>, String> {
        let mut read = Vec::with_capacity(pins.len());
        for pin in pins {
            let hash = from_hex::<32>(&pin.hash)
                .ok_or_else(|| format!("the hash of `{}` is not one", pin.name.escape_debug()))?;
            let registration = match &pin.registration {
                Some(text) => Some(from_hex::<32>(text).ok_or_else(|| {
                    format!(
                        "the registration of `{}` is not a hash",
                        pin.name.escape_debug()
                    )
                })?),
                None => None,
            };
            if read.iter().any(|other: &SignedPin<'_>| {
                other.kind == pin.kind.as_str() && other.name == pin.name
            }) {
                return Err(format!(
                    "the {} `{}` is pinned twice",
                    pin.kind,
                    pin.name.escape_debug()
                ));
            }
            read.push(SignedPin {
                kind: pin.kind.as_str(),
                name: &pin.name,
                hash,
                registration,
            });
        }
        Ok(read)
    }
    Ok(match proposal {
        ApprovalProposal::Project {
            key,
            revision,
            root,
            git_trusted,
            pins,
        } => project_statement(
            key.as_str(),
            *revision,
            root,
            *git_trusted,
            &mut read(pins)?,
        ),
        ApprovalProposal::UserPins { revision, pins } => {
            user_pins_statement(*revision, &mut read(pins)?)
        }
    })
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let digits = text.as_bytes();
    if digits.len() != N * 2 {
        return None;
    }
    let value = |digit: u8| match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    };
    let mut bytes = [0; N];
    for (byte, pair) in bytes.iter_mut().zip(digits.as_chunks::<2>().0) {
        *byte = (value(pair[0])? << 4) | value(pair[1])?;
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cheap() -> Cost {
        Cost::new(64, 1, 1).unwrap()
    }

    fn key(password: &str, salt: u8) -> (KeyFile, PersonKey) {
        KeyFile::make(
            password.as_bytes(),
            Salt::from_bytes([salt; Salt::LEN]),
            cheap(),
            cineris_protocol::ids::Timestamp::new("2026-11-27T10:00:00Z"),
        )
        .unwrap()
    }

    #[test]
    fn a_statements_fields_cannot_run_into_each_other() {
        let of = |a: &str, b: &str| Statement::build(Unit::Project).text(a).text(b).finish();
        assert_eq!(of("ab", "c"), of("ab", "c"));
        assert_ne!(of("ab", "c"), of("a", "bc"));
        assert_ne!(of("", "abc"), of("abc", ""));
        // The unit is signed: the same fields of another unit are other bytes.
        assert_ne!(
            Statement::build(Unit::Project).text("x").finish(),
            Statement::build(Unit::UserPins).text("x").finish()
        );
        // So are an entry's bounds: two entries are not one entry of both.
        let set = |entries: &[(&str, &str)]| {
            Statement::build(Unit::UserPins)
                .entries(entries, |(a, b), fields| {
                    fields.text(a).text(b);
                })
                .finish()
        };
        assert_ne!(set(&[("a", "b"), ("c", "d")]), set(&[("a", "b")]));
        assert_ne!(set(&[("a", "b")]), set(&[("a", "b"), ("", "")]));
        assert_ne!(set(&[("ab", ""), ("c", "")]), set(&[("a", "b"), ("c", "")]));
        assert!(
            set(&[]).bytes().starts_with(DOMAIN),
            "every statement starts with the domain"
        );
    }

    #[test]
    fn a_signature_checks_only_with_its_key_and_its_statement() {
        let (file, person) = key("correct horse", 1);
        let verifier = person.verifier();
        assert!(file.opens(&person));
        let statement = Statement::build(Unit::Project).text("k").number(1).finish();
        let signature = person.sign(&statement);
        assert!(verifier.verify(&statement, &signature));
        // Another statement.
        let other = Statement::build(Unit::Project).text("k").number(2).finish();
        assert!(!verifier.verify(&other, &signature));
        // Another key: another password, or the same one with another salt.
        let (_, stranger) = key("another password", 1);
        assert!(!stranger.verifier().verify(&statement, &signature));
        let (_, salted) = key("correct horse", 2);
        assert_ne!(salted.verifier(), verifier);
        assert!(!salted.verifier().verify(&statement, &signature));
        // A signature with one bit changed.
        let mut bytes = *signature.as_bytes();
        bytes[10] ^= 1;
        assert!(!verifier.verify(&statement, &Signature::from_bytes(bytes)));
        // A key that is not a point of the curve checks nothing.
        assert!(!Verifier([0xff; 32]).verify(&statement, &signature));
    }

    #[test]
    fn the_same_password_and_salt_make_the_same_key_again() {
        let (file, first) = key("correct horse", 7);
        let again = PersonKey::derive(b"correct horse", &file).unwrap();
        assert_eq!(again.verifier(), first.verifier());
        assert!(file.opens(&again));
        let wrong = PersonKey::derive(b"correct horsf", &file).unwrap();
        assert!(!file.opens(&wrong));
    }

    #[test]
    fn signatures_and_keys_are_written_as_hex_and_read_back_strictly() {
        let (_, person) = key("correct horse", 3);
        let signature = person.sign(&Statement::build(Unit::UserPins).finish());
        let text = signature.to_string();
        assert_eq!(text.len(), 128);
        assert_eq!(Signature::from_hex(&text), Some(signature));
        assert_eq!(Signature::from_hex(&text.to_uppercase()), None);
        assert_eq!(Signature::from_hex(&text[1..]), None);
        assert_eq!(Signature::from_hex(&format!("{text}0")), None);
        let verifier = person.verifier();
        // What is printed for a person names neither whole.
        assert!(!format!("{signature:?}").contains(&text[..16]));
        assert!(format!("{verifier:?}").contains(&verifier.fingerprint()));
    }
}
