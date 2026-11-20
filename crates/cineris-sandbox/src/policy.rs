//! The sandbox policy as the launcher applies it, and the bytes it travels
//! in (`docs/architecture/06-permissions-and-sandbox.md`, "The sandbox").
//!
//! The lease holds the policy in the protocol's form: path sets as text, the
//! protected paths, the network setting. The harness turns it into a
//! [`Policy`]: what a command started under it may read and write, whether
//! it may use the network, its private temporary directory, and the
//! Landlock version the harness counted on when it said the sandbox
//! enforces. The protected paths are not here: Landlock cannot keep a path
//! inside a writable tree from being changed, so they are checked after the
//! command instead ([`crate::protect`]).
//!
//! # The wire form
//!
//! The launcher reads the policy from its standard input, to the end, and
//! refuses what it does not understand in full: a policy is an enforced
//! value (`docs/decisions/0028-enforced-values-fail-closed.md`). The form is
//! a list of NUL-terminated fields, so that a path is carried byte for byte
//! whatever it holds:
//!
//! ```text
//! cineris-sandbox-policy\0 1\0
//! read\0 all\0              or one   read\0 <path>\0   for each root
//! write\0 <path>\0          for each root, possibly none
//! network\0 off|full\0      once
//! landlock\0 <version>\0    once
//! temp\0 <path>\0           at most once, and under a write root
//! input\0 <length>\0        at most once: the command's input follows `end`
//! end\0
//! <length> bytes            only with `input`: the command's standard input
//! ```
//!
//! Every path is absolute. A field this version does not know, a value it
//! does not know, a field given twice that may be given once, a missing
//! field, and anything after `end` refuse the whole policy. Nothing is
//! skipped: a skipped field could be the one that restricts.
//!
//! **The command's input** (step 28: an extension's `cineris.exec`, and a
//! hook command's event) travels on the same stream, after the policy: the
//! launcher's standard input is the only way into it. `input` gives its
//! length in decimal digits, with no sign and no leading zero, at most
//! [`MAX_INPUT`]; after `end` come exactly that many bytes, which are never
//! parsed. A length that is not written so, one past the limit, a second
//! `input`, or a number of bytes after `end` other than the length refuses
//! the whole policy, as anything after `end` does without `input`. Without
//! `input` the command's input is closed, as it always was.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The first field of every policy.
const MAGIC: &[u8] = b"cineris-sandbox-policy";
/// The version of the wire form this build writes and reads.
const VERSION: &[u8] = b"1";
/// The most bytes a policy may take. Far beyond any real one.
pub const MAX_BYTES: usize = 1 << 20;

/// The most bytes of input a command may be given after its policy: the
/// most an extension's `cineris.exec`, or a hook command's event, may carry.
pub const MAX_INPUT: usize = 1 << 20;

/// The most digits a length of input may be written with: those of
/// [`MAX_INPUT`].
const MAX_INPUT_DIGITS: usize = 7;

/// The newest Landlock version this build knows the rights of.
pub const NEWEST_LANDLOCK: u8 = 9;

/// What a command may read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// Everything: the default. A sandboxed command can read the user's
    /// files, and cannot send them anywhere while the network is off.
    All,
    /// The trees under these roots, and the write roots.
    Only(Vec<PathBuf>),
}

/// Whether a command may use the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Network {
    /// No network: no socket that reaches beyond the command's own
    /// processes (`docs/decisions/0032-what-the-linux-sandbox-needs.md`).
    Off,
    /// The whole network. Nothing about sockets is filtered.
    Full,
}

/// What one command may touch, as the launcher applies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    /// What may be read.
    pub read: Reach,
    /// The roots of the trees that may be written: the workspace, the
    /// session's temporary directory, and what `sandbox.write` adds.
    pub write: Vec<PathBuf>,
    /// Whether the network may be used.
    pub network: Network,
    /// The session's private temporary directory, which the command is
    /// given as `TMPDIR`. It is one of the write roots or under one.
    pub temp: Option<PathBuf>,
    /// The Landlock version the harness counted on when it said the
    /// sandbox enforces: the launcher asks the kernel for every right of
    /// this version, and refuses to start the command if one is missing.
    pub landlock: u8,
}

/// Why a policy was refused. The message names the field or the value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the sandbox policy is not understood: {0}")]
pub struct PolicyError(String);

impl PolicyError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl Policy {
    /// The policy as the launcher reads it, for a command whose input is
    /// closed.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        self.encode_fields(None)
    }

    /// The policy as the launcher reads it, followed by `input`, which the
    /// command is given as its standard input.
    ///
    /// # Errors
    ///
    /// When `input` is longer than [`MAX_INPUT`]: the launcher would refuse
    /// it.
    pub fn encode_with_input(&self, input: &[u8]) -> Result<Vec<u8>, PolicyError> {
        if input.len() > MAX_INPUT {
            return Err(PolicyError::new(format!(
                "the command's input is {} bytes, and at most {MAX_INPUT} can be given",
                input.len()
            )));
        }
        let mut out = self.encode_fields(Some(input.len()));
        out.extend_from_slice(input);
        Ok(out)
    }

    fn encode_fields(&self, input: Option<usize>) -> Vec<u8> {
        let mut out = Vec::new();
        let mut field = |key: &[u8], value: &[u8]| {
            out.extend_from_slice(key);
            out.push(0);
            out.extend_from_slice(value);
            out.push(0);
        };
        field(MAGIC, VERSION);
        match &self.read {
            Reach::All => field(b"read", b"all"),
            Reach::Only(roots) => {
                for root in roots {
                    field(b"read", &path_bytes(root));
                }
            }
        }
        for root in &self.write {
            field(b"write", &path_bytes(root));
        }
        field(
            b"network",
            match self.network {
                Network::Off => b"off",
                Network::Full => b"full",
            },
        );
        field(b"landlock", self.landlock.to_string().as_bytes());
        if let Some(temp) = &self.temp {
            field(b"temp", &path_bytes(temp));
        }
        if let Some(length) = input {
            field(b"input", length.to_string().as_bytes());
        }
        out.extend_from_slice(b"end\0");
        out
    }

    /// Reads a policy, refusing it whole if any part of it is not
    /// understood, or if it carries a command's input
    /// ([`Policy::decode_with_input`] reads that).
    ///
    /// # Errors
    ///
    /// With the field or the value that was not understood.
    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyError> {
        match Self::decode_with_input(bytes)? {
            (policy, None) => Ok(policy),
            (_, Some(_)) => Err(PolicyError::new(
                "it carries a command's input, which only the launcher reads",
            )),
        }
    }

    /// Reads a policy and the command's input after it, if it has an
    /// `input` field, refusing the whole of it if any part is not
    /// understood. The input's bytes are taken as they are: nothing reads
    /// them but the command.
    ///
    /// # Errors
    ///
    /// With the field or the value that was not understood, or when the
    /// bytes after `end` are not exactly the input's length.
    pub fn decode_with_input(bytes: &[u8]) -> Result<(Self, Option<Vec<u8>>), PolicyError> {
        if bytes.len() > MAX_BYTES + MAX_INPUT {
            return Err(PolicyError::new("it is longer than any policy can be"));
        }
        let mut fields = Fields { rest: bytes };
        match (fields.next_part()?, fields.next_part()?) {
            (Some(MAGIC), Some(VERSION)) => {}
            (Some(MAGIC), Some(version)) => {
                return Err(PolicyError::new(format!(
                    "version `{}` of the wire form is not one this build reads",
                    String::from_utf8_lossy(version)
                )));
            }
            _ => return Err(PolicyError::new("it does not begin as a policy does")),
        }

        let mut read_all = false;
        let mut read_roots = Vec::new();
        let mut write = Vec::new();
        let mut network = None;
        let mut landlock = None;
        let mut temp = None;
        let mut input: Option<usize> = None;
        loop {
            let Some(key) = fields.next_part()? else {
                return Err(PolicyError::new("it ends without `end`"));
            };
            if key == b"end" {
                break;
            }
            let Some(value) = fields.next_part()? else {
                return Err(PolicyError::new(format!(
                    "the field `{}` has no value",
                    String::from_utf8_lossy(key)
                )));
            };
            match key {
                b"read" if value == b"all" => {
                    if read_all {
                        return Err(PolicyError::new("`read all` is given twice"));
                    }
                    read_all = true;
                }
                b"read" => read_roots.push(absolute("read", value)?),
                b"write" => write.push(absolute("write", value)?),
                b"network" => {
                    let value = match value {
                        b"off" => Network::Off,
                        b"full" => Network::Full,
                        other => {
                            return Err(PolicyError::new(format!(
                                "the network setting `{}` is not one this build knows",
                                String::from_utf8_lossy(other)
                            )));
                        }
                    };
                    once(&mut network, value, "network")?;
                }
                b"landlock" => {
                    let version = std::str::from_utf8(value)
                        .ok()
                        .and_then(|text| text.parse::<u8>().ok())
                        .filter(|version| (1..=NEWEST_LANDLOCK).contains(version))
                        .ok_or_else(|| {
                            PolicyError::new(format!(
                                "the Landlock version `{}` is not one this build knows",
                                String::from_utf8_lossy(value)
                            ))
                        })?;
                    once(&mut landlock, version, "landlock")?;
                }
                b"temp" => once(&mut temp, absolute("temp", value)?, "temp")?,
                b"input" => once(&mut input, input_length(value)?, "input")?,
                other => {
                    return Err(PolicyError::new(format!(
                        "the field `{}` is not one this build knows",
                        String::from_utf8_lossy(other)
                    )));
                }
            }
        }
        if bytes.len() - fields.rest.len() > MAX_BYTES {
            return Err(PolicyError::new("it is longer than any policy can be"));
        }
        let input = match input {
            None if !fields.rest.is_empty() => {
                return Err(PolicyError::new("there is more after `end`"));
            }
            None => None,
            Some(length) if fields.rest.len() != length => {
                return Err(PolicyError::new(format!(
                    "`input` says {length} bytes follow `end`, and {} do",
                    fields.rest.len()
                )));
            }
            Some(_) => Some(fields.rest.to_vec()),
        };
        let read = match (read_all, read_roots.is_empty()) {
            (true, true) => Reach::All,
            (false, _) => Reach::Only(read_roots),
            (true, false) => {
                return Err(PolicyError::new(
                    "`read all` and a read root are both given",
                ));
            }
        };
        let network = network.ok_or_else(|| PolicyError::new("`network` is missing"))?;
        let landlock = landlock.ok_or_else(|| PolicyError::new("`landlock` is missing"))?;
        if let Some(temp) = &temp
            && !write.iter().any(|root| temp.starts_with(root))
        {
            return Err(PolicyError::new(
                "the temporary directory is not under a write root",
            ));
        }
        Ok((
            Self {
                read,
                write,
                network,
                temp,
                landlock,
            },
            input,
        ))
    }

    /// Whether `path` is under one of the write roots. Both are compared as
    /// they are written: the harness gives resolved paths.
    #[must_use]
    pub fn may_write(&self, path: &Path) -> bool {
        self.write.iter().any(|root| path.starts_with(root))
    }
}

/// Sets a field that may be given once.
fn once<T>(slot: &mut Option<T>, value: T, name: &str) -> Result<(), PolicyError> {
    if slot.is_some() {
        return Err(PolicyError::new(format!("`{name}` is given twice")));
    }
    *slot = Some(value);
    Ok(())
}

/// The length of a command's input: decimal digits only, no sign and no
/// leading zero, at most [`MAX_INPUT`]. Read strictly: a length is what
/// decides where the policy ends and the input begins.
fn input_length(value: &[u8]) -> Result<usize, PolicyError> {
    let shown = || String::from_utf8_lossy(value).into_owned();
    let digits = !value.is_empty()
        && value.len() <= MAX_INPUT_DIGITS
        && value.iter().all(u8::is_ascii_digit)
        && (value.len() == 1 || value[0] != b'0');
    let length = digits
        .then(|| std::str::from_utf8(value).ok()?.parse::<usize>().ok())
        .flatten()
        .ok_or_else(|| {
            PolicyError::new(format!(
                "the input's length `{}` is not written as a length is",
                shown()
            ))
        })?;
    if length > MAX_INPUT {
        return Err(PolicyError::new(format!(
            "the input's length `{}` is more than {MAX_INPUT} bytes",
            shown()
        )));
    }
    Ok(length)
}

/// The NUL-terminated parts of a policy, in order.
struct Fields<'a> {
    rest: &'a [u8],
}

impl<'a> Fields<'a> {
    /// The next part, or `None` at the end of the bytes.
    fn next_part(&mut self) -> Result<Option<&'a [u8]>, PolicyError> {
        if self.rest.is_empty() {
            return Ok(None);
        }
        let Some(end) = self.rest.iter().position(|byte| *byte == 0) else {
            return Err(PolicyError::new("its last field is not ended"));
        };
        let (part, rest) = self.rest.split_at(end);
        self.rest = &rest[1..];
        Ok(Some(part))
    }
}

/// A path field: absolute, and not empty.
fn absolute(field: &str, bytes: &[u8]) -> Result<PathBuf, PolicyError> {
    let path = PathBuf::from(os_string(bytes, field)?);
    if !path.is_absolute() {
        return Err(PolicyError::new(format!(
            "the {field} path `{}` is not absolute",
            path.display()
        )));
    }
    Ok(path)
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    // Only Linux starts the launcher. Elsewhere a policy is written for the
    // tests of its form, and a path that is not Unicode is written as the
    // nearest text.
    path.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
fn os_string(bytes: &[u8], _field: &str) -> Result<OsString, PolicyError> {
    use std::os::unix::ffi::OsStrExt;
    Ok(std::ffi::OsStr::from_bytes(bytes).to_owned())
}

#[cfg(not(unix))]
fn os_string(bytes: &[u8], field: &str) -> Result<OsString, PolicyError> {
    String::from_utf8(bytes.to_vec())
        .map(OsString::from)
        .map_err(|_| PolicyError::new(format!("the {field} path is not text")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        if cfg!(windows) {
            PathBuf::from(format!(r"C:\{name}"))
        } else {
            PathBuf::from(format!("/{name}"))
        }
    }

    fn sample() -> Policy {
        Policy {
            read: Reach::All,
            write: vec![root("work"), root("tmp")],
            network: Network::Off,
            temp: Some(root("tmp").join("cineris-s1")),
            landlock: 6,
        }
    }

    #[test]
    fn a_policy_reads_back_as_it_was_written() {
        let policy = sample();
        assert_eq!(Policy::decode(&policy.encode()).unwrap(), policy);
        let narrow = Policy {
            read: Reach::Only(vec![root("usr"), root("work")]),
            write: Vec::new(),
            network: Network::Full,
            temp: None,
            landlock: 3,
        };
        assert_eq!(Policy::decode(&narrow.encode()).unwrap(), narrow);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_is_carried_byte_for_byte() {
        use std::os::unix::ffi::OsStrExt;
        let odd = PathBuf::from(std::ffi::OsStr::from_bytes(b"/w/line\nbreak/\xff\xfe"));
        let policy = Policy {
            write: vec![odd.clone()],
            temp: Some(odd.join("t")),
            ..sample()
        };
        let back = Policy::decode(&policy.encode()).unwrap();
        assert_eq!(back.write, vec![odd]);
    }

    /// Rewrites a policy's bytes and expects the decoder to refuse them,
    /// naming `naming`.
    fn refused(change: impl Fn(&str) -> String, naming: &str) {
        let text = String::from_utf8(sample().encode()).unwrap();
        let changed = change(&text);
        assert_ne!(changed, text, "the change did nothing");
        let error = Policy::decode(changed.as_bytes()).unwrap_err();
        assert!(error.to_string().contains(naming), "{error}");
    }

    #[test]
    fn an_unknown_field_or_value_refuses_the_whole_policy() {
        refused(|text| text.replace("end\0", "share\0/x\0end\0"), "`share`");
        refused(|text| text.replace("off", "loopback"), "`loopback`");
        refused(
            |text| text.replace("landlock\x006", "landlock\x0042"),
            "`42`",
        );
        refused(|text| text.replace("landlock\x006", "landlock\x000"), "`0`");
        refused(
            |text| text.replace("read\0all", "read\0most"),
            "not absolute",
        );
        refused(|text| text.replacen("\x001\0", "\x002\0", 1), "version `2`");
    }

    #[test]
    fn a_field_given_twice_or_missing_refuses_it() {
        refused(
            |text| text.replace("end\0", "network\0full\0end\0"),
            "`network` is given twice",
        );
        refused(
            |text| text.replace("end\0", "read\0all\0end\0"),
            "given twice",
        );
        refused(
            |text| text.replace("network\0off\0", ""),
            "`network` is missing",
        );
        refused(
            |text| text.replace("landlock\x006\0", ""),
            "`landlock` is missing",
        );
        refused(
            |text| text.replace("end\0", &format!("read\0{}\0end\0", root("usr").display())),
            "both given",
        );
    }

    #[test]
    fn a_policy_cut_short_or_with_more_after_its_end_is_refused() {
        let bytes = sample().encode();
        for cut in 0..bytes.len() {
            assert!(
                Policy::decode(&bytes[..cut]).is_err(),
                "a policy cut at {cut} was accepted"
            );
        }
        let mut longer = bytes.clone();
        longer.extend_from_slice(b"write\0/\0");
        assert!(Policy::decode(&longer).is_err());
        assert!(Policy::decode(b"").is_err());
        assert!(Policy::decode(&vec![b'x'; MAX_BYTES + 1]).is_err());
    }

    /// A command's input follows the policy and comes back byte for byte,
    /// NULs and all; [`Policy::decode`], which no launcher uses, refuses a
    /// policy that carries one.
    #[test]
    fn a_commands_input_follows_the_policy_and_reads_back_whole() {
        let policy = sample();
        for input in [&b""[..], b"x", b"end\0input\x003\0abc", &[0u8; 300]] {
            let bytes = policy.encode_with_input(input).unwrap();
            let (back, carried) = Policy::decode_with_input(&bytes).unwrap();
            assert_eq!(back, policy);
            assert_eq!(carried.as_deref(), Some(input));
            assert!(Policy::decode(&bytes).is_err());
        }
        // Without the field, there is no input.
        assert_eq!(
            Policy::decode_with_input(&policy.encode()).unwrap(),
            (policy.clone(), None)
        );
        // The most there may be, and one byte more.
        let most = vec![7u8; MAX_INPUT];
        let bytes = policy.encode_with_input(&most).unwrap();
        assert_eq!(Policy::decode_with_input(&bytes).unwrap().1, Some(most));
        assert!(policy.encode_with_input(&vec![7u8; MAX_INPUT + 1]).is_err());
        // Written by hand past the limit, with the bytes to match: refused.
        let mut past = policy.encode();
        past.truncate(past.len() - b"end\0".len());
        past.extend_from_slice(format!("input\0{}\0end\0", MAX_INPUT + 1).as_bytes());
        past.extend_from_slice(&vec![7u8; MAX_INPUT + 1]);
        let error = Policy::decode_with_input(&past).unwrap_err();
        assert!(error.to_string().contains("more than"), "{error}");
    }

    /// The bytes after `end` must be exactly the length `input` says: one
    /// fewer, one more, or any without the field refuses the whole policy;
    /// so do a second `input` and a length not written as one.
    #[test]
    fn an_input_whose_length_does_not_match_or_is_given_twice_is_refused() {
        let policy = sample();
        let with = policy.encode_with_input(b"abc").unwrap();
        let short = &with[..with.len() - 1];
        let error = Policy::decode_with_input(short).unwrap_err();
        assert!(error.to_string().contains("3 bytes"), "{error}");
        let mut long = with.clone();
        long.push(b'd');
        assert!(Policy::decode_with_input(&long).is_err());
        let mut after = policy.encode();
        after.extend_from_slice(b"abc");
        let error = Policy::decode_with_input(&after).unwrap_err();
        assert!(error.to_string().contains("more after `end`"), "{error}");

        let text = String::from_utf8(with).unwrap();
        let decode = |text: String| Policy::decode_with_input(text.as_bytes());
        let twice = text.replace("input\x003\0", "input\x003\0input\x003\0");
        let error = decode(twice).unwrap_err();
        assert!(
            error.to_string().contains("`input` is given twice"),
            "{error}"
        );
        for written in [
            "03", "+3", "-3", " 3", "3 ", "", "0x3", "1048577", "99999999",
        ] {
            let changed = text.replace("input\x003\0", &format!("input\0{written}\0"));
            assert!(decode(changed).is_err(), "the length `{written}` was taken");
        }
    }

    /// Whatever is done to the length, to a repeat of the field or to the
    /// bytes after `end`, the input read back is exactly the input written,
    /// or the whole policy is refused: never a policy with other bytes for
    /// the command, and never a policy that reads on into the input.
    #[test]
    fn a_damaged_input_field_is_refused_never_misread() {
        use cineris_base::Rng as _;
        use cineris_testkit::cases::{self, ensure};

        #[derive(Debug)]
        struct Case {
            input: Vec<u8>,
            length: String,
            repeat: bool,
            extra: Vec<u8>,
            cut: usize,
        }
        cases::check(
            "a_damaged_input_field_is_refused_never_misread",
            300,
            |rng| {
                let input: Vec<u8> = (0..cases::small(rng, 64))
                    .map(|_| u8::try_from(rng.below(256)).unwrap_or(0))
                    .collect();
                let true_length = input.len().to_string();
                let length = match rng.below(6) {
                    0 => (input.len() + 1 + cases::small(rng, 8)).to_string(),
                    1 => input.len().saturating_sub(1).to_string(),
                    2 => format!("0{true_length}"),
                    3 => (*cases::pick(rng, &["", "x", "-1", "1e3", "\u{0663}"])).to_owned(),
                    _ => true_length,
                };
                let extra = (0..usize::from(rng.below(4) == 0) * cases::small(rng, 4))
                    .map(|_| b'!')
                    .collect();
                let cut = usize::from(rng.below(4) == 0) * cases::small(rng, 4);
                Case {
                    input,
                    length,
                    repeat: rng.below(6) == 0,
                    extra,
                    cut,
                }
            },
            |case| {
                let policy = sample();
                let mut bytes = policy.encode();
                bytes.truncate(bytes.len() - b"end\0".len());
                let field = format!("input\0{}\0", case.length);
                bytes.extend_from_slice(field.as_bytes());
                if case.repeat {
                    bytes.extend_from_slice(field.as_bytes());
                }
                bytes.extend_from_slice(b"end\0");
                bytes.extend_from_slice(&case.input);
                bytes.extend_from_slice(&case.extra);
                bytes.truncate(bytes.len().saturating_sub(case.cut));
                // What follows `end` once the bytes are cut; `None` when
                // the cut reaches into the policy itself.
                let mut after = case.input.clone();
                after.extend_from_slice(&case.extra);
                let after = after
                    .len()
                    .checked_sub(case.cut)
                    .map(|kept| after[..kept].to_vec());
                // A length written as a length is: its number written back
                // is the same text.
                let declared = case
                    .length
                    .parse::<usize>()
                    .ok()
                    .filter(|length| length.to_string() == case.length && *length <= MAX_INPUT);
                let sound =
                    !case.repeat && declared.is_some() && after.as_ref().map(Vec::len) == declared;
                match Policy::decode_with_input(&bytes) {
                    Ok((back, input)) => {
                        ensure!(sound, "a damaged policy was read: {input:?}");
                        ensure!(back == policy, "the policy came back as {back:?}");
                        ensure!(input == after, "the input came back as {input:?}");
                    }
                    Err(error) => ensure!(!sound, "a sound policy was refused: {error}"),
                }
                Ok(())
            },
        );
    }

    #[test]
    fn the_temporary_directory_must_be_writable() {
        let policy = Policy {
            temp: Some(root("elsewhere")),
            ..sample()
        };
        let error = Policy::decode(&policy.encode()).unwrap_err();
        assert!(error.to_string().contains("temporary"), "{error}");
    }

    #[test]
    fn a_write_root_reaches_only_paths_under_it() {
        let policy = sample();
        assert!(policy.may_write(&root("work").join("src")));
        assert!(policy.may_write(&root("work")));
        // A root that shares a prefix of its text is another tree.
        assert!(!policy.may_write(&root("work2")));
        assert!(!policy.may_write(&root("home")));
    }
}
