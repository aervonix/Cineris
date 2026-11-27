//! MCP servers in the configuration (since step 30;
//! `docs/architecture/13-mcp.md`, "Configuration";
//! `docs/steps/30/answered.md`, points 3.1 to 3.4): what an entry must be
//! for its server to start, and a project's servers, read apart from the
//! user's.
//!
//! **An entry is judged whole** ([`server_problem`]), once its layers are
//! merged, since its keys may come from more than one: its name is the
//! dynamic-tool registry's rule; it is local (`command`) or remote (`url`),
//! never both or neither, with no key of the other kind; neither its
//! `command` nor its `cwd` is under `/proc`; each variable's and header's
//! name can be one; a value is taken from the environment only by a
//! variable's exact name, never a pattern (`ANTHROPIC_API_KE*` would read
//! the key: the full review of step 30's first part), and never from a
//! provider's key, a variable of the harness's own (`CINERIS_…`) or one
//! `shell.env_remove` keeps from commands. **A project's header** is taken
//! only from the variable named for its server and itself in a namespace
//! only projects use ([`project_header_variable`]:
//! `CINERIS_MCP_<SERVER>_<HEADER>`), never from one the project chose, one
//! the user has for other tools (`GITHUB_TOKEN`), or as text, so that a
//! repository cannot send a token of the user's to a host it picked. An
//! entry that fails is refused with a warning naming it: the configuration
//! loads, and the server is not started. So is one with a key this version
//! does not know or a value of the wrong type: the configuration's check
//! says it of that server alone ([`SERVER_NOT_STARTED`]).
//!
//! **A project's servers are kept apart** ([`ProjectMcpServer`]), as its
//! hooks are: never merged with the user's, whether the project is trusted
//! or not; trust and each entry's own approval decide whether one runs.

use toml::Value;

use crate::import::derived_variable;
use crate::key::KeyPath;
use crate::layer::Origin;
use crate::schema::{EnvText, McpServerSettings, Settings};

/// The most characters a server's name may have.
pub const MAX_SERVER_NAME: usize = crate::import::MAX_NAME;

/// What the configuration's check says, after what is wrong, of a key of an
/// `[mcp.servers.<name>]` entry that it does not know or cannot read: that
/// entry's server is not started, and the rest of the configuration loads.
pub const SERVER_NOT_STARTED: &str = "this MCP server is not started";

/// The characters a pattern of variables is written with: never in a
/// variable a value is read from.
const PATTERN: [char; 5] = ['*', '?', '[', ']', '{'];

/// Whether `name` can be a server's: one to [`MAX_SERVER_NAME`] of the
/// ASCII letters, digits, `_`, `-` and `.`, the dynamic-tool registry's
/// rule for the part of a tool's name before its `/`.
#[must_use]
pub fn server_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SERVER_NAME
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
}

/// Why `name` cannot be a variable's or a header's name: it is empty, holds
/// `=` or a NUL, or begins with a digit. `None` when it can.
#[must_use]
pub fn bad_variable_name(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("is empty")
    } else if name.contains('=') {
        Some("holds `=`")
    } else if name.contains('\0') {
        Some("holds a NUL")
    } else if name.starts_with(|first: char| first.is_ascii_digit()) {
        Some("begins with a digit")
    } else {
        None
    }
}

/// Why `name` cannot be the name of a variable a server is given, or one a
/// value is read from ([`bad_variable_name`]), or why it is a pattern
/// (`*`, `?`, `[`, `]`, `{`) and not a name: a lookup that took a pattern
/// for a prefix read whatever matched it (the full review of step 30's
/// first part). `None` when it is a name.
#[must_use]
pub fn bad_source_name(name: &str) -> Option<&'static str> {
    bad_variable_name(name).or_else(|| {
        name.contains(PATTERN)
            .then_some("is a pattern, not a variable's name")
    })
}

/// Whether `variable` tells a program's dynamic loader what to load or run
/// (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `LD_AUDIT` and every `LD_` variable,
/// `GLIBC_TUNABLES`, macOS's `DYLD_` ones), in any case. A server's
/// environment is set on the launcher's own process, before it confines
/// itself (the early review of step 30's live input): such a variable
/// would run code unconfined, so no server is given one.
#[must_use]
pub fn loads_code(variable: &str) -> bool {
    let name = variable.to_ascii_uppercase();
    name.starts_with("LD_") || name.starts_with("DYLD_") || name == "GLIBC_TUNABLES"
}

/// Whether `variable`, a variable's exact name, is one no server's value
/// may come from: a provider's key (of
/// [`cineris_base::child_env::KNOWN_KEYS`], the built-in providers' among
/// them, or one of `keys`), one of the harness's own (`CINERIS_…`), or one
/// `env_remove` names (a name there may end with `*`), each in any case on
/// Windows.
#[must_use]
pub fn withheld_source(variable: &str, keys: &[String], env_remove: &[String]) -> bool {
    crate::import::harnesss_variable(variable) || withheld_by_list(variable, keys, env_remove)
}

/// Whether `variable` is a provider's key or one `env_remove` names: what
/// [`withheld_source`] says but for the harness's own.
fn withheld_by_list(variable: &str, keys: &[String], env_remove: &[String]) -> bool {
    let windows = cfg!(windows);
    cineris_base::child_env::KNOWN_KEYS
        .iter()
        .copied()
        .chain(keys.iter().map(String::as_str))
        .chain(env_remove.iter().map(String::as_str))
        .any(|pattern| cineris_base::child_env::matches(variable, pattern, windows))
}

/// The variable a project's server takes its header `header` from: in a
/// namespace only Cineris projects use, `CINERIS_MCP_` and the server's
/// and the header's names in capitals, joined by `_`, each character that
/// cannot be in a variable's name made `_` (`tracker`'s `Authorization`
/// from `CINERIS_MCP_TRACKER_AUTHORIZATION`). Never a variable the user has
/// for other tools: a project's `github` and `token` made `GITHUB_TOKEN`
/// before (the full review of step 30's first part).
#[must_use]
pub fn project_header_variable(server: &str, header: &str) -> String {
    format!("CINERIS_MCP_{}", derived_variable(server, header))
}

/// Whether `path`, as an entry writes it, is under `/proc`, whose links a
/// process reads as its own (`/proc/self/cwd` is the folder the server runs
/// in, not the harness's), so that what the harness checked is not what the
/// server starts.
fn under_proc(path: &str) -> bool {
    let mut names: Vec<&str> = Vec::new();
    if !path.starts_with('/') {
        return false;
    }
    for name in path.split('/') {
        match name {
            "" | "." => {}
            ".." => {
                names.pop();
            }
            name => names.push(name),
        }
    }
    names.first() == Some(&"proc")
}

/// The key variables of the providers the configuration names beside the
/// built-in ones ([`cineris_base::child_env::KNOWN_KEYS`]): each provider's
/// `key_env`, and each variable a provider's header reads.
#[must_use]
pub fn provider_keys(settings: &Settings) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for provider in settings.providers.values() {
        let headers = provider.headers.0.values().filter_map(|value| match value {
            EnvText::Env(variable) => Some(variable.clone()),
            EnvText::Literal(_) => None,
        });
        for variable in provider.key_env.clone().into_iter().chain(headers) {
            if !keys.contains(&variable) {
                keys.push(variable);
            }
        }
    }
    keys
}

/// Why the server `name`, whose entry is `entry`, is not started as it is:
/// the first thing wrong with it, in words. `None` when nothing is.
/// `project` says whether the entry is a project's; `keys` are the
/// providers' key variables beside the known ones ([`provider_keys`]), and
/// `env_remove` the variables `shell.env_remove` keeps from commands.
#[must_use]
pub fn server_problem(
    name: &str,
    entry: &McpServerSettings,
    project: bool,
    keys: &[String],
    env_remove: &[String],
) -> Option<String> {
    if !server_name_ok(name) {
        return Some(format!(
            "its name may hold only one to {MAX_SERVER_NAME} of the ASCII letters, digits, `_`, \
             `-` and `.`"
        ));
    }
    let local = entry.command.is_some();
    match (local, entry.url.is_some()) {
        (true, true) => return Some("it has both `command` and `url`".to_owned()),
        (false, false) => return Some("it has neither `command` nor `url`".to_owned()),
        _ => {}
    }
    let wrong_kind = if local {
        (!entry.headers.0.is_empty()).then_some("headers")
    } else {
        [
            ("args", !entry.args.0.is_empty()),
            ("cwd", entry.cwd.is_some()),
            ("env", !entry.env.0.is_empty()),
            ("network", entry.network),
        ]
        .into_iter()
        .find_map(|(key, set)| set.then_some(key))
    };
    if let Some(key) = wrong_kind {
        let kind = if local {
            "a remote server's, and it is started as a command"
        } else {
            "a local server's, and it is reached at an address"
        };
        return Some(format!("`{key}` is {kind}"));
    }
    let shown = |text: &str| cineris_base::in_code(text).into_owned();
    for (key, path) in [
        (
            "command",
            entry.command.as_ref().map(|program| program.0.as_str()),
        ),
        ("cwd", entry.cwd.as_deref()),
    ] {
        if path.is_some_and(under_proc) {
            return Some(format!(
                "its `{key}` is under `/proc`, which the server would read as its own"
            ));
        }
    }
    let withheld = |source: &str| {
        format!(
            "takes its value from `{}`, a provider's key, one of the harness's own or one \
             `shell.env_remove` keeps from commands",
            shown(source)
        )
    };
    for (variable, value) in &entry.env.0 {
        if let Some(why) = bad_source_name(variable) {
            return Some(format!("its variable `{}` {why}", shown(variable)));
        }
        if variable.to_ascii_uppercase().starts_with("CINERIS_") {
            return Some(format!(
                "its variable `{}` is one of the harness's own, and no server is given one",
                shown(variable)
            ));
        }
        if loads_code(variable) {
            return Some(format!(
                "its variable `{}` tells the system's loader what to run, which it would do \
                 before the sandbox confines the server, and no server is given one",
                shown(variable)
            ));
        }
        if let EnvText::Env(source) = value {
            if let Some(why) = bad_source_name(source) {
                return Some(format!(
                    "its variable `{}` takes its value from `{}`, which {why}",
                    shown(variable),
                    shown(source)
                ));
            }
            if withheld_source(source, keys, env_remove) {
                return Some(format!(
                    "its variable `{}` {}",
                    shown(variable),
                    withheld(source)
                ));
            }
        }
    }
    for (header, value) in &entry.headers.0 {
        if let Some(why) = bad_variable_name(header) {
            return Some(format!("its header `{}` {why}", shown(header)));
        }
        let derived = project_header_variable(name, header);
        let only = || {
            format!(
                "a project's server takes a header only from the variable named for the server \
                 and the header, `{derived}`"
            )
        };
        match value {
            EnvText::Env(source) if project && *source != derived => {
                return Some(format!(
                    "its header `{}` takes its value from `{}`: {}",
                    shown(header),
                    shown(source),
                    only()
                ));
            }
            // The derived form is the harness's own namespace: it is the
            // one `CINERIS_` variable a project's header takes.
            EnvText::Env(source) if project && withheld_by_list(source, keys, env_remove) => {
                return Some(format!(
                    "its header `{}` {}",
                    shown(header),
                    withheld(source)
                ));
            }
            EnvText::Env(source) if !project => {
                if let Some(why) = bad_source_name(source) {
                    return Some(format!(
                        "its header `{}` takes its value from `{}`, which {why}",
                        shown(header),
                        shown(source)
                    ));
                }
                if withheld_source(source, keys, env_remove) {
                    return Some(format!(
                        "its header `{}` {}",
                        shown(header),
                        withheld(source)
                    ));
                }
            }
            EnvText::Literal(_) if project => {
                return Some(format!(
                    "its header `{}` is written as text: {}",
                    shown(header),
                    only()
                ));
            }
            _ => {}
        }
    }
    None
}

/// The variables `entry`'s server takes from the environment, in capitals:
/// what a project's derived header variable may not be.
#[must_use]
pub fn sources(entry: &McpServerSettings) -> Vec<String> {
    entry
        .env
        .0
        .values()
        .chain(entry.headers.0.values())
        .filter_map(|value| match value {
            EnvText::Env(variable) => Some(variable.to_ascii_uppercase()),
            EnvText::Literal(_) => None,
        })
        .collect()
}

/// The server's name when `key` is in an `[mcp.servers.<name>]` entry, at
/// a file's top level or in a profile: a key the configuration's check
/// cannot read there is that server's problem alone.
#[must_use]
pub fn server_of(key: &KeyPath) -> Option<&str> {
    let parts = key.parts();
    let parts = match parts {
        [first, _, rest @ ..] if first == "profiles" => rest,
        _ => parts,
    };
    match parts {
        [mcp, servers, name, ..] if mcp == "mcp" && servers == "servers" => Some(name.as_str()),
        _ => None,
    }
}

/// One of a project's MCP servers, read apart from the user's (since step
/// 30): an `[mcp.servers.<name>]` table of the project's own files, the
/// shared one's keys under the local one's. It is never merged with a
/// server of the user's, and it runs only once the project is trusted and
/// the entry, as it is now, is approved.
///
/// Its `Debug` form leaves out the table as written, which can hold a
/// literal of its `env` (the full review of step 30's first part).
#[derive(Clone, PartialEq)]
pub struct ProjectMcpServer {
    /// Its name.
    pub name: String,
    /// The entry, as far as it reads: an entry with a key of the wrong type
    /// has the defaults in its place, and its `problem` says so.
    pub settings: McpServerSettings,
    /// Where its table is: the file that defines it first, and the line.
    pub origin: Origin,
    /// Why it is not started as it is ([`server_problem`], or a key the
    /// configuration's check could not read), when it is not.
    pub problem: Option<String>,
    /// The table as the project's files give it, merged: what
    /// [`ProjectMcpServer::for_consent`] shows. Never `Debug`'s.
    table: toml::Table,
}

impl std::fmt::Debug for ProjectMcpServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectMcpServer")
            .field("name", &self.name)
            .field("settings", &self.settings)
            .field("origin", &self.origin)
            .field("problem", &self.problem)
            .finish_non_exhaustive()
    }
}

impl ProjectMcpServer {
    /// A project's server read from `table`.
    pub(crate) fn new(
        name: String,
        settings: McpServerSettings,
        origin: Origin,
        problem: Option<String>,
        table: toml::Table,
    ) -> Self {
        Self {
            name,
            settings,
            origin,
            problem,
            table,
        }
    }

    /// The entry as the question that asks the person to approve it shows
    /// it: whole, with only what the secret filter's formats know masked
    /// (13-mcp's rule for the trust question), a literal of its `env` and
    /// its `headers` included, so that no part of what the person approves
    /// is hidden (`NODE_OPTIONS = "--require ./.hidden/evil.js"`).
    #[must_use]
    pub fn for_consent(&self) -> Value {
        crate::schema::redact_for_consent(
            &KeyPath::new(["mcp", "servers", self.name.as_str()]),
            &Value::Table(self.table.clone()),
        )
    }

    /// Each header the server sends, and the variable it takes its value
    /// from: for the trust question, which names each with the host.
    #[must_use]
    pub fn header_variables(&self) -> Vec<(String, String)> {
        self.settings
            .headers
            .0
            .iter()
            .filter_map(|(header, value)| match value {
                EnvText::Env(variable) => Some((header.clone(), variable.clone())),
                EnvText::Literal(_) => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Setting;

    fn entry(text: &str) -> McpServerSettings {
        let table: toml::Table = text.parse().unwrap();
        McpServerSettings::read(&Value::Table(table)).unwrap()
    }

    fn problem(name: &str, text: &str, project: bool) -> Option<String> {
        server_problem(
            name,
            &entry(text),
            project,
            &["GATEWAY_KEY".to_owned()],
            &["SECRET_*".to_owned()],
        )
    }

    #[test]
    fn a_servers_name_is_the_registrys_rule() {
        for name in ["docs", "my-server_2", "a.b", &"n".repeat(MAX_SERVER_NAME)] {
            assert!(server_name_ok(name), "{name}");
        }
        for name in [
            "",
            "my server",
            "a/b",
            "tool:x",
            "é",
            &"n".repeat(MAX_SERVER_NAME + 1),
        ] {
            assert!(!server_name_ok(name), "{name}");
            let said = problem(name, "command = \"x\"", false).unwrap();
            assert!(said.contains("its name"), "{said}");
            // The first run never writes a server the loader would refuse.
            assert!(!crate::import::settable_name(name), "{name}");
        }
    }

    /// The first run does not import a server the loader would not start:
    /// one named outside the rule, one whose variable's or header's name
    /// cannot be one, and one that takes a provider's key from the
    /// environment by its own name.
    #[test]
    fn the_first_run_imports_no_server_the_loader_would_refuse() {
        use crate::import::{Refusal, servers_in};
        let servers = servers_in(
            r#"{"mcpServers": {
                "fine": {"command": "run", "env": {"DOCS_TOKEN": "x"}},
                "my server": {"command": "run"},
                "digit": {"command": "run", "env": {"1ST": "x"}},
                "equals": {"url": "https://h.example.com/mcp", "headers": {"A=B": "x"}},
                "key": {"command": "run", "env": {"ANTHROPIC_API_KEY": "x"}},
                "loader": {"command": "run", "env": {"LD_PRELOAD": "x"}},
                "pattern": {"command": "run", "env": {"ANTHROPIC_API_KE*": "x"}}
            }}"#,
            false,
        )
        .unwrap();
        let refused: Vec<(&str, Option<Refusal>)> = servers
            .iter()
            .map(|server| (server.name.as_str(), server.refused))
            .collect();
        for (name, expected) in [
            ("fine", None),
            ("my server", Some(Refusal::Name)),
            ("digit", Some(Refusal::NotUnderstood)),
            ("equals", Some(Refusal::NotUnderstood)),
            ("key", Some(Refusal::NotUnderstood)),
            ("loader", Some(Refusal::NotUnderstood)),
            ("pattern", Some(Refusal::NotUnderstood)),
        ] {
            assert!(refused.contains(&(name, expected)), "{name}: {refused:?}");
        }
    }

    /// A server is local or remote, never both or neither, and has no key
    /// of the other kind.
    #[test]
    fn a_server_is_local_or_remote_and_has_only_its_kinds_keys() {
        assert_eq!(
            problem(
                "a",
                "command = \"x\"\nargs = [\"-v\"]\nnetwork = true",
                false
            ),
            None
        );
        assert_eq!(
            problem("a", "url = \"https://h.example.com/mcp\"", false),
            None
        );
        for (text, naming) in [
            ("command = \"x\"\nurl = \"https://h\"", "both"),
            ("enabled = false", "neither"),
            ("description = \"d\"", "neither"),
            ("url = \"https://h\"\nargs = [\"x\"]", "`args`"),
            ("url = \"https://h\"\ncwd = \"sub\"", "`cwd`"),
            ("url = \"https://h\"\nenv = { A = \"b\" }", "`env`"),
            ("url = \"https://h\"\nnetwork = true", "`network`"),
            (
                "command = \"x\"\nheaders = { Authorization = { env = \"A_AUTHORIZATION\" } }",
                "`headers`",
            ),
        ] {
            let said = problem("a", text, false).unwrap_or_default();
            assert!(said.contains(naming), "{text}: {said}");
        }
    }

    /// A variable's or a header's name that cannot be one, a value taken
    /// from a provider's key, the harness's own or one `shell.env_remove`
    /// names, and a variable of the harness's own, each refuse the server,
    /// naming it.
    #[test]
    fn a_name_or_a_source_no_server_may_have_refuses_it() {
        for (text, naming) in [
            ("env = { \"\" = \"v\" }", "is empty"),
            ("env = { \"A=B\" = \"v\" }", "holds `=`"),
            ("env = { \"1A\" = \"v\" }", "begins with a digit"),
            ("env = { CINERIS_LOG = \"debug\" }", "the harness's own"),
            ("env = { LD_PRELOAD = \"/x.so\" }", "`LD_PRELOAD`"),
            ("env = { ld_library_path = \"/x\" }", "`ld_library_path`"),
            ("env = { LD_AUDIT = { env = \"AUDIT\" } }", "`LD_AUDIT`"),
            ("env = { GLIBC_TUNABLES = \"x\" }", "`GLIBC_TUNABLES`"),
            (
                "env = { DYLD_INSERT_LIBRARIES = \"x\" }",
                "`DYLD_INSERT_LIBRARIES`",
            ),
            (
                "env = { T = { env = \"ANTHROPIC_API_KEY\" } }",
                "`ANTHROPIC_API_KEY`",
            ),
            (
                "env = { T = { env = \"cineris_config\" } }",
                "`cineris_config`",
            ),
            ("env = { T = { env = \"GATEWAY_KEY\" } }", "`GATEWAY_KEY`"),
            ("env = { T = { env = \"SECRET_ONE\" } }", "`SECRET_ONE`"),
        ] {
            let said = problem("a", &format!("command = \"x\"\n{text}"), false).unwrap_or_default();
            assert!(said.contains(naming), "{text}: {said}");
        }
        for (text, naming) in [
            ("headers = { \"\" = { env = \"V\" } }", "is empty"),
            (
                "headers = { \"1X\" = { env = \"V\" } }",
                "begins with a digit",
            ),
            (
                "headers = { Authorization = { env = \"OPENAI_API_KEY\" } }",
                "`OPENAI_API_KEY`",
            ),
        ] {
            let said =
                problem("a", &format!("url = \"https://h\"\n{text}"), false).unwrap_or_default();
            assert!(said.contains(naming), "{text}: {said}");
        }
        // A provider's key's name as the variable a server is given, from a
        // variable of its own, is the user's choice.
        assert_eq!(
            problem(
                "a",
                "command = \"x\"\nenv = { OPENAI_API_KEY = { env = \"MY_KEY\" } }",
                false
            ),
            None
        );
    }

    /// A project's header is taken only from the variable named for its
    /// server and the header in the projects' own namespace,
    /// `CINERIS_MCP_<SERVER>_<HEADER>`, the one `CINERIS_` variable it may
    /// take; the user's own header from any variable of the user's but the
    /// harness's own.
    #[test]
    fn a_projects_header_is_taken_only_from_its_derived_variable() {
        assert_eq!(
            project_header_variable("my-tracker", "Authorization"),
            "CINERIS_MCP_MY_TRACKER_AUTHORIZATION"
        );
        let derived = "url = \"https://h\"\nheaders = { Authorization = { env = \
                       \"CINERIS_MCP_MY_TRACKER_AUTHORIZATION\" } }";
        assert_eq!(problem("my-tracker", derived, true), None);
        // The user's own server never takes one of the harness's.
        let said = problem("my-tracker", derived, false).unwrap_or_default();
        assert!(said.contains("the harness's own"), "{said}");
        for text in [
            "url = \"https://h\"\nheaders = { Authorization = { env = \"GITHUB_TOKEN\" } }",
            "url = \"https://h\"\nheaders = { Authorization = { env = \
             \"MY_TRACKER_AUTHORIZATION\" } }",
            "url = \"https://h\"\nheaders = { Authorization = \"Bearer abc\" }",
        ] {
            let said = problem("my-tracker", text, true).unwrap_or_default();
            assert!(
                said.contains("`CINERIS_MCP_MY_TRACKER_AUTHORIZATION`"),
                "{text}: {said}"
            );
            assert_eq!(problem("my-tracker", text, false), None, "{text}");
        }
        // The full review's cases: a project's names chosen to make the
        // user's own token's name.
        for (server, header, source) in [
            ("github", "token", "GITHUB_TOKEN"),
            ("my_tracker", "Authorization", "MY_TRACKER_AUTHORIZATION"),
            ("anthropic", "api-key", "ANTHROPIC_API_KEY"),
        ] {
            let text =
                format!("url = \"https://h\"\nheaders = {{ {header} = {{ env = \"{source}\" }} }}");
            assert!(problem(server, &text, true).is_some(), "{server} {header}");
        }
        // The derived form is refused when `shell.env_remove` names it.
        let said = problem(
            "x",
            "url = \"https://h\"\nheaders = { A = { env = \"CINERIS_MCP_X_A\" } }",
            true,
        );
        assert_eq!(said, None);
        let removed = server_problem(
            "x",
            &entry("url = \"https://h\"\nheaders = { A = { env = \"CINERIS_MCP_X_A\" } }"),
            true,
            &[],
            &["CINERIS_*".to_owned()],
        );
        assert!(removed.is_some());
    }

    /// A value is read from the environment by a variable's exact name: a
    /// source written as a pattern is refused, whatever it would match (the
    /// full review: `ANTHROPIC_API_KE*` read the key), as is one a name
    /// cannot be; and so is a variable given a pattern's name.
    #[test]
    fn a_source_written_as_a_pattern_is_refused() {
        for source in [
            "ANTHROPIC_API_KE*",
            "*",
            "CINERIS*",
            "GATEWAY*",
            "A?",
            "[A]B",
            "1X",
            "A=B",
        ] {
            for project in [false, true] {
                let said = problem(
                    "a",
                    &format!("command = \"srv\"\nenv = {{ X = {{ env = \"{source}\" }} }}"),
                    project,
                );
                assert!(said.is_some(), "`{source}` taken, project {project}");
            }
            let said = problem(
                "a",
                &format!("url = \"https://h\"\nheaders = {{ A = {{ env = \"{source}\" }} }}"),
                false,
            );
            assert!(said.is_some(), "`{source}` taken as a header's source");
        }
        let said = problem("a", "command = \"srv\"\nenv = { \"X*\" = \"v\" }", false);
        assert!(said.is_some_and(|said| said.contains("pattern")));
        assert_eq!(bad_source_name("GITHUB_TOKEN"), None);
    }

    /// A built-in provider's key is a provider's key as the configuration
    /// loads, whether or not the user wrote its `key_env` (the full review:
    /// OpenRouter's, DeepSeek's and Venice's were taken), for a local
    /// server's variable and a remote one's header alike.
    #[test]
    fn a_built_in_providers_key_is_refused_as_a_source() {
        for key in ["OPENROUTER_API_KEY", "DEEPSEEK_API_KEY", "VENICE_API_KEY"] {
            let local = problem(
                "a",
                &format!("command = \"srv\"\nenv = {{ K = {{ env = \"{key}\" }} }}"),
                true,
            );
            assert!(local.is_some_and(|said| said.contains(key)), "{key}");
            let remote = problem(
                "a",
                &format!("url = \"https://h\"\nheaders = {{ A = {{ env = \"{key}\" }} }}"),
                false,
            );
            assert!(remote.is_some_and(|said| said.contains(key)), "{key}");
        }
    }

    /// A `command` or a `cwd` under `/proc` is refused: `/proc/self/cwd` is
    /// the harness's folder where it is checked, and the server's where it
    /// starts.
    #[test]
    fn a_command_or_folder_under_proc_is_refused() {
        for text in [
            "command = \"/proc/self/cwd/server\"",
            "command = \"//proc/1/root/bin/x\"",
            "command = \"/x/../proc/self/cwd/server\"",
            "command = \"srv\"\ncwd = \"/proc/self/cwd\"",
            "command = \"srv\"\ncwd = \"/./proc\"",
        ] {
            let said = problem("a", text, false).unwrap_or_default();
            assert!(said.contains("/proc"), "{text}: {said}");
        }
        assert_eq!(problem("a", "command = \"/opt/proc/server\"", false), None);
    }

    #[test]
    fn a_key_in_a_servers_entry_names_its_server() {
        for (key, server) in [
            ("mcp.servers.docs.type", Some("docs")),
            ("mcp.servers.docs", Some("docs")),
            ("profiles.work.mcp.servers.x.args", Some("x")),
            ("mcp.servers", None),
            ("mcp.start_timeout_ms", None),
            ("hooks.docs.command", None),
        ] {
            assert_eq!(server_of(&KeyPath::parse(key).unwrap()), server, "{key}");
        }
    }
}
