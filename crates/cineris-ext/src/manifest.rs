//! An extension's manifest, `extension.toml` (`docs/architecture/12-extensions.md`,
//! "Tier 4"; `docs/steps/28/answered.md`, "The manifest and the
//! registration cache").
//!
//! It says what the extension is and what it may ask the host to do. Its
//! capabilities are values the harness enforces, so it is read strictly
//! (decision 0028): a key this build does not know, a value of the wrong
//! type, or an `api` it does not know refuses the extension with the
//! reason, rather than being passed over.

use std::time::Duration;

use serde::Deserialize;

use crate::event::Event;
use crate::folder::Snapshot;

/// The file name of a manifest, at the top of an extension's folder.
pub const FILE: &str = "extension.toml";

/// The one version of the manifest's format and of the host functions this
/// build knows.
pub const API: u32 = 1;

/// The memory a runtime may use unless the manifest says less, in MiB.
pub const DEFAULT_MEMORY_MIB: u64 = 32;
/// The most memory a manifest may ask for, in MiB.
pub const MAX_MEMORY_MIB: u64 = 64;
/// A tool call's time unless the manifest says otherwise.
pub const DEFAULT_TIME: Duration = Duration::from_secs(120);
/// The most time a manifest may give a tool call.
pub const MAX_TIME: Duration = Duration::from_secs(600);
/// The largest prompt file (section 7 of the system prompt).
pub const MAX_PROMPT_BYTES: usize = 4096;
/// The longest name, and the longest version.
const MAX_NAME: usize = 64;
/// The longest description.
const MAX_DESCRIPTION: usize = 1024;
/// The most entries a list of capabilities may have.
const MAX_ENTRIES: usize = 64;
/// The longest program or path glob a capability lists, in bytes: a real
/// program's path or a glob is well inside it, and it keeps what an
/// approval shows of each entry, sixty-four of them at most, bounded.
pub const MAX_ENTRY_BYTES: usize = 256;

/// Why a manifest was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the manifest of the extension is refused: {0}")]
pub struct ManifestError(pub String);

fn refused(reason: impl Into<String>) -> ManifestError {
    ManifestError(reason.into())
}

/// A program the extension may ask the host to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Program {
    /// A bare name, found in `PATH`'s absolute directories outside the
    /// workspace and the write roots.
    Name(String),
    /// An absolute path, taken as named.
    Path(String),
}

impl Program {
    /// The entry as the manifest wrote it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Name(text) | Self::Path(text) => text,
        }
    }
}

/// A host the extension may contact through the host's fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostPattern {
    /// `*`: any host. A private or loopback address is still refused unless
    /// it is named literally.
    Any,
    /// `*.example.com`: any host under the domain, not the domain itself.
    Under(String),
    /// One host, in lower case: a name, `localhost`, or an address.
    Exact(String),
}

impl HostPattern {
    /// Whether `host`, in lower case, is one this pattern names.
    #[must_use]
    pub fn matches(&self, host: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Under(domain) => host
                .strip_suffix(domain.as_str())
                .is_some_and(|rest| !rest.is_empty()),
            Self::Exact(name) => name == host,
        }
    }

    /// Whether the pattern names this host literally (not through a `*`).
    #[must_use]
    pub fn names_literally(&self, host: &str) -> bool {
        matches!(self, Self::Exact(name) if name == host)
    }
}

/// What the manifest lets the extension ask the host to do. Each list is
/// empty unless the manifest gives it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Declared {
    /// Programs for `cineris.exec`.
    pub exec: Vec<Program>,
    /// Whether those programs may reach the network.
    pub exec_network: bool,
    /// Workspace path globs `cineris.fs` may read.
    pub read: Vec<String>,
    /// Workspace path globs `cineris.fs` may write.
    pub write: Vec<String>,
    /// Hosts `cineris.http.fetch` may contact.
    pub net: Vec<HostPattern>,
    /// The events the script may hook.
    pub hooks: Vec<Event>,
}

/// The limits a manifest sets for its runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestLimits {
    /// The memory its runtime may use, in MiB.
    pub memory_mib: u64,
    /// The time one tool call may take.
    pub time: Duration,
}

impl Default for ManifestLimits {
    fn default() -> Self {
        Self {
            memory_mib: DEFAULT_MEMORY_MIB,
            time: DEFAULT_TIME,
        }
    }
}

/// An extension's manifest, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// Its name: the registry's rules, and the name of its folder.
    pub name: String,
    /// The author's version, shown.
    pub version: String,
    /// What it is for, shown and listed.
    pub description: String,
    /// The module the runtime starts with.
    pub main: String,
    /// The file of section 7 of the system prompt, if any.
    pub prompt: Option<String>,
    /// What it may ask the host to do.
    pub capabilities: Declared,
    /// Its runtime's limits.
    pub limits: ManifestLimits,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    api: u32,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    main: String,
    prompt: Option<String>,
    #[serde(default)]
    capabilities: RawCapabilities,
    #[serde(default)]
    limits: RawLimits,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawCapabilities {
    #[serde(default)]
    exec: Vec<String>,
    #[serde(default)]
    exec_network: bool,
    #[serde(default)]
    read: Vec<String>,
    #[serde(default)]
    write: Vec<String>,
    #[serde(default)]
    net: Vec<String>,
    #[serde(default)]
    hooks: Vec<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawLimits {
    memory: Option<u64>,
    time: Option<u64>,
}

impl Manifest {
    /// Reads a manifest's text.
    ///
    /// # Errors
    ///
    /// [`ManifestError`] names what is wrong: a key this build does not
    /// know, a value it does not take, or an `api` it does not know.
    pub fn parse(text: &str) -> Result<Self, ManifestError> {
        // The version is read first, so that a manifest of a newer format
        // is refused for that and not for a key it adds.
        if let Ok(table) = toml::from_str::<toml::Table>(text)
            && let Some(api) = table.get("api").and_then(toml::Value::as_integer)
            && api != i64::from(API)
        {
            return Err(refused(format!(
                "`api = {api}` is not one this build of Cineris knows (it knows {API}): it needs another version of Cineris"
            )));
        }
        let raw: Raw = toml::from_str(text).map_err(|error| refused(error.message().to_owned()))?;
        if raw.api != API {
            return Err(refused(format!(
                "`api = {}` is not one this build of Cineris knows (it knows {API})",
                raw.api
            )));
        }
        check_name(&raw.name)?;
        check_text("version", &raw.version, MAX_NAME, false)?;
        check_shown("version", &raw.version)?;
        check_text("description", &raw.description, MAX_DESCRIPTION, true)?;
        check_file("main", &raw.main, &["js", "mjs"])?;
        if let Some(prompt) = &raw.prompt {
            check_file("prompt", prompt, &["md", "txt"])?;
        }
        let capabilities = declared(raw.capabilities)?;
        let limits = limits(raw.limits)?;
        Ok(Self {
            name: raw.name,
            version: raw.version,
            description: raw.description,
            main: raw.main,
            prompt: raw.prompt,
            capabilities,
            limits,
        })
    }

    /// Reads the manifest of a folder's snapshot, and checks that the files
    /// it names are in it.
    ///
    /// # Errors
    ///
    /// When the manifest is missing, not UTF-8, refused, or names a file
    /// the folder does not have, or a prompt over its cap.
    pub fn of(snapshot: &Snapshot) -> Result<Self, ManifestError> {
        let bytes = snapshot
            .file(FILE)
            .ok_or_else(|| refused(format!("the folder has no `{FILE}`")))?;
        let text =
            std::str::from_utf8(bytes).map_err(|_| refused(format!("`{FILE}` is not UTF-8")))?;
        let manifest = Self::parse(text)?;
        if snapshot.file(&manifest.main).is_none() {
            return Err(refused(format!(
                "`main = \"{}\"` names a file the folder does not have",
                manifest.main
            )));
        }
        if let Some(prompt) = &manifest.prompt {
            let content = snapshot.file(prompt).ok_or_else(|| {
                refused(format!(
                    "`prompt = \"{prompt}\"` names a file the folder does not have"
                ))
            })?;
            if content.len() > MAX_PROMPT_BYTES {
                return Err(refused(format!("`{prompt}` is larger than 4 KiB")));
            }
            if std::str::from_utf8(content).is_err() {
                return Err(refused(format!("`{prompt}` is not UTF-8")));
            }
        }
        Ok(manifest)
    }
}

/// Whether `name` follows the registry's rules for an extension's name,
/// which is also a source of tools: 1 to 64 of ASCII letters, digits, `_`,
/// `-` and `.`, not starting with `.`. The registration cache asks it of
/// each entry's name.
pub(crate) fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.starts_with('.')
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
}

/// The registry's rules for a name ([`is_name`]).
fn check_name(name: &str) -> Result<(), ManifestError> {
    if is_name(name) {
        Ok(())
    } else {
        Err(refused(format!(
            "`name = \"{name}\"`: a name is 1 to {MAX_NAME} of ASCII letters, digits, `_`, `-` and `.`, and does not start with `.`"
        )))
    }
}

/// What an approval shows as code, a program, a path glob or the version:
/// at most [`MAX_ENTRY_BYTES`], and with no backtick, which would end the
/// code span it is shown in, so that what follows it would read as the
/// harness's own words (the early review of step 28's second part).
fn check_shown(key: &str, text: &str) -> Result<(), ManifestError> {
    if text.len() > MAX_ENTRY_BYTES {
        return Err(refused(format!(
            "`{key}` has an entry longer than {MAX_ENTRY_BYTES} bytes"
        )));
    }
    if text.contains('`') {
        return Err(refused(format!(
            "`{key}` has an entry that holds a backtick, which would end the code an approval shows it as"
        )));
    }
    Ok(())
}

fn check_text(key: &str, text: &str, most: usize, may_be_empty: bool) -> Result<(), ManifestError> {
    if !may_be_empty && text.trim().is_empty() {
        return Err(refused(format!("`{key}` is empty")));
    }
    if text.len() > most {
        return Err(refused(format!("`{key}` is longer than {most} bytes")));
    }
    if text
        .chars()
        .any(|character| character.is_control() && character != '\n')
    {
        return Err(refused(format!("`{key}` holds a control character")));
    }
    Ok(())
}

/// A path relative to the folder, with `/` between names, no `.` or `..`
/// name, and one of `extensions`.
fn check_file(key: &str, path: &str, extensions: &[&str]) -> Result<(), ManifestError> {
    let names: Vec<&str> = path.split('/').collect();
    let fine = !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && names.iter().all(|name| {
            !name.is_empty() && *name != "." && *name != ".." && !name.chars().any(char::is_control)
        })
        && extensions.iter().any(|extension| {
            path.rsplit_once('.')
                .is_some_and(|(_, found)| found == *extension)
        });
    if fine {
        Ok(())
    } else {
        Err(refused(format!(
            "`{key} = \"{path}\"`: a file in the extension's folder, written with `/`, with no `.` or `..`, ending in .{}",
            extensions.join(" or .")
        )))
    }
}

fn declared(raw: RawCapabilities) -> Result<Declared, ManifestError> {
    for (key, list) in [
        ("exec", &raw.exec),
        ("read", &raw.read),
        ("write", &raw.write),
        ("net", &raw.net),
        ("hooks", &raw.hooks),
    ] {
        if list.len() > MAX_ENTRIES {
            return Err(refused(format!(
                "`capabilities.{key}` has more than {MAX_ENTRIES} entries"
            )));
        }
    }
    for entry in &raw.exec {
        check_shown("capabilities.exec", entry)?;
    }
    let exec = raw
        .exec
        .into_iter()
        .map(program)
        .collect::<Result<Vec<_>, _>>()?;
    for (key, globs) in [("read", &raw.read), ("write", &raw.write)] {
        for glob in globs {
            check_shown(&format!("capabilities.{key}"), glob)?;
            check_glob(key, glob)?;
        }
    }
    let net = raw
        .net
        .iter()
        .map(|pattern| host_pattern(pattern))
        .collect::<Result<Vec<_>, _>>()?;
    let mut hooks = Vec::new();
    for name in &raw.hooks {
        let event = Event::parse(name).ok_or_else(|| {
            refused(format!(
                "`capabilities.hooks` names `{name}`, which is not an event: {}",
                Event::ALL.map(Event::as_str).join(", ")
            ))
        })?;
        if hooks.contains(&event) {
            return Err(refused(format!(
                "`capabilities.hooks` names `{name}` twice"
            )));
        }
        hooks.push(event);
    }
    Ok(Declared {
        exec,
        exec_network: raw.exec_network,
        read: raw.read,
        write: raw.write,
        net,
        hooks,
    })
}

/// A program entry: a bare name, or an absolute path. A relative path is
/// refused: it would name whichever file is there.
fn program(entry: String) -> Result<Program, ManifestError> {
    let bad = entry.trim().is_empty()
        || entry != entry.trim()
        || entry.chars().any(char::is_control)
        || entry == "."
        || entry == "..";
    if bad {
        return Err(refused(format!(
            "`capabilities.exec` has `{entry}`, which is not a program's name"
        )));
    }
    if !entry.contains('/') && !entry.contains('\\') {
        if entry.contains(':') {
            return Err(refused(format!(
                "`capabilities.exec` has `{entry}`: a drive with no root is a relative path"
            )));
        }
        return Ok(Program::Name(entry));
    }
    let unix_absolute = entry.starts_with('/');
    let bytes = entry.as_bytes();
    let windows_absolute = bytes.len() > 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let has_dots = entry
        .split(['/', '\\'])
        .any(|name| name == "." || name == "..");
    if (unix_absolute || windows_absolute)
        && !has_dots
        && !entry.starts_with("//")
        && !entry.starts_with("\\\\")
    {
        Ok(Program::Path(entry))
    } else {
        Err(refused(format!(
            "`capabilities.exec` has `{entry}`: a program is a bare name or an absolute path, with no `.` or `..`"
        )))
    }
}

/// A workspace path glob: relative to the workspace, so no root, no `~`,
/// no drive, and no `.` or `..` name; its folders split by `/` alone, on
/// every system. A `\` is refused: a program's write roots and the
/// manifest's own check would read it differently on Linux, one as a
/// separator and the other as a name's character (the review of step 28).
/// Whether it compiles is the capability check's to say, against the
/// workspace.
fn check_glob(key: &str, glob: &str) -> Result<(), ManifestError> {
    let first = glob.chars().next();
    let names_dots = glob.split('/').any(|name| name == "." || name == "..");
    let bad = glob.trim().is_empty()
        || matches!(first, Some('/' | '~'))
        || glob.contains('\\')
        || glob.as_bytes().get(1) == Some(&b':')
        || names_dots
        || glob.chars().any(char::is_control);
    if bad {
        Err(refused(format!(
            "`capabilities.{key}` has `{glob}`: a glob of paths in the workspace, written relative to it with `/` between its folders, with no `.` or `..`"
        )))
    } else {
        Ok(())
    }
}

/// A host pattern: `*`, `*.domain`, or one host (a name or an address).
fn host_pattern(pattern: &str) -> Result<HostPattern, ManifestError> {
    let lower = pattern.to_ascii_lowercase();
    if lower == "*" {
        return Ok(HostPattern::Any);
    }
    let (under, host) = match lower.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, lower.as_str()),
    };
    let address = host.parse::<std::net::IpAddr>().is_ok()
        || host
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
            .is_some_and(|inside| inside.parse::<std::net::Ipv6Addr>().is_ok());
    let name = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        });
    if under && name {
        Ok(HostPattern::Under(format!(".{host}")))
    } else if !under && (name || address) {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        // An address in the one form a URL's host is read in
        // (`host::url_host`): IPv6 at its shortest, and an IPv4-mapped one
        // as its IPv4 address.
        let host = host.parse::<std::net::IpAddr>().map_or_else(
            |_| host.to_owned(),
            |address| address.to_canonical().to_string(),
        );
        Ok(HostPattern::Exact(host))
    } else {
        Err(refused(format!(
            "`capabilities.net` has `{pattern}`: a host is `*`, `*.example.com`, a host's name or an address, with no scheme, port or path"
        )))
    }
}

fn limits(raw: RawLimits) -> Result<ManifestLimits, ManifestError> {
    let mut limits = ManifestLimits::default();
    if let Some(memory) = raw.memory {
        if memory == 0 || memory > MAX_MEMORY_MIB {
            return Err(refused(format!(
                "`limits.memory = {memory}`: from 1 to {MAX_MEMORY_MIB} (MiB)"
            )));
        }
        limits.memory_mib = memory;
    }
    if let Some(time) = raw.time {
        if time == 0 || time > MAX_TIME.as_secs() {
            return Err(refused(format!(
                "`limits.time = {time}`: from 1 to {} (seconds a tool call)",
                MAX_TIME.as_secs()
            )));
        }
        limits.time = Duration::from_secs(time);
    }
    Ok(limits)
}
