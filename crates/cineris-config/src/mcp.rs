//! MCP servers in the configuration (since step 30;
//! `docs/architecture/13-mcp.md`, "Configuration";
//! `docs/steps/30/answered.md`, points 3.1 to 3.4): what an entry must be
//! for its server to start, and a project's servers, read apart from the
//! user's.
//!
//! **An entry is judged whole** ([`server_problem`]), once its layers are
//! merged, since its keys may come from more than one: its name is the
//! dynamic-tool registry's rule; it is local (`command`) or remote (`url`),
//! never both or neither, with no key of the other kind; each variable's
//! and header's name can be one; and no value is taken from a provider's
//! key, a variable of the harness's own (`CINERIS_…`) or one
//! `shell.env_remove` keeps from commands. **A project's header** is taken
//! only from the variable named for its server and itself
//! ([`crate::import::derived_variable`]), never from one the project chose
//! or as text, so that a repository cannot send a token of the user's to a
//! host it picked. An entry that fails is refused with a warning naming
//! it: the configuration loads, and the server is not started.
//!
//! **A project's servers are kept apart** ([`ProjectMcpServer`]), as its
//! hooks are: never merged with the user's, whether the project is trusted
//! or not; trust and each entry's own approval decide whether one runs.

use toml::Value;

use crate::import::derived_variable;
use crate::key::KeyPath;
use crate::layer::Origin;
use crate::schema::{EnvText, McpServerSettings};

/// The most characters a server's name may have.
pub const MAX_SERVER_NAME: usize = crate::import::MAX_NAME;

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

/// Whether `variable` is one no server's value may come from: a provider's
/// key (of [`cineris_base::child_env::KNOWN_KEYS`], or one of `keys`), one
/// of the harness's own (`CINERIS_…`), or one `env_remove` names (a name
/// may end with `*`), each in any case on Windows.
#[must_use]
pub fn withheld_source(variable: &str, keys: &[String], env_remove: &[String]) -> bool {
    let windows = cfg!(windows);
    crate::import::harnesss_variable(variable)
        || keys
            .iter()
            .chain(env_remove)
            .any(|pattern| cineris_base::child_env::matches(variable, pattern, windows))
}

/// Why the server `name`, whose entry is `entry`, is not started as it is:
/// the first thing wrong with it, in words. `None` when nothing is.
/// `project` says whether the entry is a project's; `keys` are the
/// providers' key variables beside the known ones, and `env_remove` the
/// variables `shell.env_remove` keeps from commands.
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
    for (variable, value) in &entry.env.0 {
        if let Some(why) = bad_variable_name(variable) {
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
        if let EnvText::Env(source) = value
            && withheld_source(source, keys, env_remove)
        {
            return Some(format!(
                "its variable `{}` takes its value from `{}`, a provider's key, one of the \
                 harness's own or one `shell.env_remove` keeps from commands",
                shown(variable),
                shown(source)
            ));
        }
    }
    for (header, value) in &entry.headers.0 {
        if let Some(why) = bad_variable_name(header) {
            return Some(format!("its header `{}` {why}", shown(header)));
        }
        let derived = derived_variable(name, header);
        match value {
            EnvText::Env(source) if withheld_source(source, keys, env_remove) => {
                return Some(format!(
                    "its header `{}` takes its value from `{}`, a provider's key, one of the \
                     harness's own or one `shell.env_remove` keeps from commands",
                    shown(header),
                    shown(source)
                ));
            }
            EnvText::Env(source) if project && *source != derived => {
                return Some(format!(
                    "its header `{}` takes its value from `{}`: a project's server takes a \
                     header only from the variable named for the server and the header, \
                     `{derived}`",
                    shown(header),
                    shown(source)
                ));
            }
            EnvText::Literal(_) if project => {
                return Some(format!(
                    "its header `{}` is written as text: a project's server takes a header only \
                     from the variable named for the server and the header, `{derived}`",
                    shown(header)
                ));
            }
            _ => {}
        }
    }
    None
}

/// One of a project's MCP servers, read apart from the user's (since step
/// 30): an `[mcp.servers.<name>]` table of the project's own files, the
/// shared one's keys under the local one's. It is never merged with a
/// server of the user's, and it runs only once the project is trusted and
/// the entry, as it is now, is approved.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectMcpServer {
    /// Its name.
    pub name: String,
    /// The entry.
    pub settings: McpServerSettings,
    /// Where its table is: the file that defines it first, and the line.
    pub origin: Origin,
    /// Why it is not started as it is ([`server_problem`]), when it is not.
    pub problem: Option<String>,
    /// The table as the project's files give it, merged: what
    /// [`ProjectMcpServer::for_consent`] shows. Never `Debug`'s.
    table: toml::Table,
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
    /// it: whole, with only what the secret filter's formats know masked,
    /// and a literal of `env` or `headers` (13-mcp's rule for the trust
    /// question), so that no part of the command line the person approves
    /// is hidden.
    #[must_use]
    pub fn for_consent(&self) -> Value {
        crate::schema::redact_for_consent(
            &KeyPath::new(["mcp", "servers", self.name.as_str()]),
            &Value::Table(self.table.clone()),
        )
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
                "loader": {"command": "run", "env": {"LD_PRELOAD": "x"}}
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
    /// server and the header; the user's own from any of the user's.
    #[test]
    fn a_projects_header_is_taken_only_from_its_derived_variable() {
        let derived = "url = \"https://h\"\nheaders = { Authorization = { env = \
                       \"MY_TRACKER_AUTHORIZATION\" } }";
        assert_eq!(problem("my-tracker", derived, true), None);
        assert_eq!(problem("my-tracker", derived, false), None);
        for text in [
            "url = \"https://h\"\nheaders = { Authorization = { env = \"GITHUB_TOKEN\" } }",
            "url = \"https://h\"\nheaders = { Authorization = \"Bearer abc\" }",
        ] {
            let said = problem("my-tracker", text, true).unwrap_or_default();
            assert!(
                said.contains("`MY_TRACKER_AUTHORIZATION`"),
                "{text}: {said}"
            );
            assert_eq!(problem("my-tracker", text, false), None, "{text}");
        }
        // A derived name that is a provider's key is refused all the same.
        let said = problem(
            "anthropic",
            "url = \"https://h\"\nheaders = { api-key = { env = \"ANTHROPIC_API_KEY\" } }",
            true,
        )
        .unwrap_or_default();
        assert!(said.contains("provider"), "{said}");
    }
}
