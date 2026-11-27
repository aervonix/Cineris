//! The configuration's MCP servers as `cineris-mcp`'s definitions
//! (`docs/steps/30/answered.md`, points 3.1 and 5).
//!
//! - **The user's servers** (`[mcp.servers]` of the user's layers, merged)
//!   are [`Origin::User`].
//! - **A project's** (`[mcp.servers]` of its `.cineris/config.toml`, read
//!   apart, `cineris_config::Loaded::project_mcp_servers`) are
//!   [`Origin::ProjectConfig`]. One whose name the user's configuration has
//!   is refused: no project file can name, change or turn off one of the
//!   user's servers.
//! - **An entry that is not started as it is**
//!   (`cineris_config::mcp::server_problem`: a name the registry refuses, a
//!   server both local and remote, a variable no server may be given, a
//!   project's header not from its own variable) is refused too.
//!
//! Each refusal comes back with why, for a notice. Nothing here judges
//! trust or approval: which definitions start is the approvals' to say.
//! The project's `.mcp.json` is read by a later part of step 30.

use std::time::Duration;

use cineris_config::Loaded;
use cineris_config::schema::{EnvText, ExposeChoice, McpServerSettings};
use cineris_mcp::{EnvEntry, Expose, Launch, Origin, ServerDefinition, ValueSource};

/// The servers the configuration defines, and the entries it refuses.
#[derive(Debug, Clone, Default)]
pub struct Definitions {
    /// The user's servers, then the project's, each in name order.
    pub servers: Vec<ServerDefinition>,
    /// The entries refused, each with where it was and why.
    pub refused: Vec<Refused>,
}

/// An entry that is not a server's definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The server's name, as its entry gives it.
    pub name: String,
    /// Where it was defined.
    pub origin: Origin,
    /// Why it was refused, in words.
    pub why: String,
}

impl Refused {
    /// What the user is told of it.
    #[must_use]
    pub fn notice(&self) -> String {
        let whose = match self.origin {
            Origin::User => "Your MCP server",
            Origin::ProjectConfig | Origin::ProjectMcpJson => "The project's MCP server",
        };
        format!(
            "{whose} `{}` is not started: {}.",
            cineris_base::in_code(&self.name),
            self.why.trim_end_matches('.')
        )
    }
}

/// The definitions of `loaded`'s servers, the user's and the project's, and
/// the entries it refuses ([the module](self)). Every provider's key is
/// withheld: the built-in providers', each configured one's, and each a
/// provider's header reads (`crate::provider::key_variables`, the full
/// review of step 30's first part), from a local server's variables and a
/// remote one's headers alike.
#[must_use]
pub fn definitions(loaded: &Loaded) -> Definitions {
    let settings = &loaded.settings;
    let keys = crate::provider::key_variables(settings);
    let env_remove = &settings.shell.env_remove;
    let start_default = Duration::from_millis(settings.mcp.start_timeout_ms);
    let servers = &settings.mcp.servers;
    let mut made = Definitions::default();
    let mut refused: Vec<Refused> = servers
        .refused
        .iter()
        .map(|(name, why)| Refused {
            name: name.clone(),
            origin: Origin::User,
            why: why.clone(),
        })
        .collect();
    for (name, entry) in &servers.entries {
        match cineris_config::mcp::server_problem(name, entry, false, &keys, env_remove) {
            Some(why) => refused.push(Refused {
                name: name.clone(),
                origin: Origin::User,
                why,
            }),
            None => made
                .servers
                .push(definition(name, entry, Origin::User, start_default)),
        }
    }
    refused.sort_by(|one, other| one.name.cmp(&other.name));
    made.refused = refused;
    for server in &loaded.project_mcp_servers {
        let users = servers.entries.contains_key(&server.name)
            || servers.refused.contains_key(&server.name);
        let why = if users {
            Some(
                "your own configuration has a server of that name, which a project's cannot \
                 replace"
                    .to_owned(),
            )
        } else {
            server.problem.clone().or_else(|| {
                cineris_config::mcp::server_problem(
                    &server.name,
                    &server.settings,
                    true,
                    &keys,
                    env_remove,
                )
            })
        };
        match why {
            Some(why) => made.refused.push(Refused {
                name: server.name.clone(),
                origin: Origin::ProjectConfig,
                why,
            }),
            None => made.servers.push(definition(
                &server.name,
                &server.settings,
                Origin::ProjectConfig,
                start_default,
            )),
        }
    }
    made
}

/// The definition of the server `name` whose entry, judged sound, is
/// `entry`.
fn definition(
    name: &str,
    entry: &McpServerSettings,
    origin: Origin,
    start_default: Duration,
) -> ServerDefinition {
    let entries = |table: &std::collections::BTreeMap<String, EnvText>| {
        table
            .iter()
            .map(|(name, value)| EnvEntry {
                name: name.clone(),
                value: match value {
                    EnvText::Env(variable) => ValueSource::Env(variable.clone()),
                    EnvText::Literal(text) => ValueSource::Literal(text.clone()),
                },
            })
            .collect()
    };
    let launch = match (&entry.command, &entry.url) {
        (Some(program), _) => Launch::Local {
            program: program.0.clone(),
            args: entry.args.0.clone(),
            cwd: entry.cwd.clone(),
            env: entries(&entry.env.0),
            network: entry.network,
            // No server writes the workspace in this release.
            write: false,
        },
        (None, url) => Launch::Remote {
            url: url.as_ref().map(|url| url.0.clone()).unwrap_or_default(),
            headers: entries(&entry.headers.0),
        },
    };
    ServerDefinition {
        name: name.to_owned(),
        origin,
        launch,
        description: entry.description.clone(),
        enabled: entry.enabled,
        expose: match entry.expose {
            ExposeChoice::Search => Expose::Search,
            ExposeChoice::Direct => Expose::Direct,
        },
        tools: entry.tools.clone(),
        deny_tools: entry.deny_tools.clone(),
        start_timeout: entry
            .start_timeout_ms
            .map_or(start_default, Duration::from_millis),
        call_timeout: Duration::from_millis(entry.timeout_ms),
    }
}

#[cfg(test)]
mod tests {
    use cineris_config::{Dirs, Env, LoadOptions, Platform, ProjectTrust};

    use super::*;

    /// Loads a configuration of `user`'s file and `project`'s
    /// `.cineris/config.toml`.
    #[allow(
        clippy::disallowed_methods,
        reason = "a test writes its configuration's files"
    )]
    fn loaded(test: &str, user: &str, project: &str, trust: ProjectTrust) -> Loaded {
        let scratch = cineris_testkit::Scratch::outside("cineris-core-mcp-definitions", test);
        let home = scratch.dir("home");
        let workspace = scratch.dir("work");
        let variable = match Platform::CURRENT {
            Platform::Unix => "HOME",
            Platform::Windows => "USERPROFILE",
        };
        let env = Env::empty(Platform::CURRENT).with(variable, home.to_str().unwrap());
        let mut dirs = Dirs::from_env(&env);
        dirs.managed_config = Some(scratch.join("managed.toml"));
        let user_file = dirs.user_config.clone().unwrap();
        std::fs::create_dir_all(user_file.parent().unwrap()).unwrap();
        std::fs::write(&user_file, user).unwrap();
        scratch.file("work/.cineris/config.toml", project);
        let options = LoadOptions {
            dirs,
            workspace: Some(workspace),
            trust,
            ..LoadOptions::new(env)
        };
        cineris_config::load(&options).unwrap()
    }

    /// The configuration withholds a provider's key from a server as it
    /// loads by `cineris_base::child_env::KNOWN_KEYS`, as it cannot see the
    /// provider layer's table: every built-in provider's key variable is on
    /// that list (the full review of step 30's first part).
    #[test]
    fn every_built_in_providers_key_is_a_known_key() {
        for name in crate::provider::builtin_names() {
            if let Some((_, Some(variable))) = crate::provider::builtin(name) {
                assert!(
                    cineris_base::child_env::KNOWN_KEYS.contains(&variable),
                    "{name}'s `{variable}`"
                );
            }
        }
    }

    /// The user's servers and a project's become definitions, each with
    /// its origin and every key of its entry; a project's whose name is the
    /// user's, and an entry that is not started as it is, are refused with
    /// why.
    #[test]
    fn the_users_and_a_projects_servers_become_definitions() {
        let user = "[mcp]\nstart_timeout_ms = 20000\n\
                    [mcp.servers.docs]\ncommand = \"docs-mcp\"\nargs = [\"--root\", \".\"]\n\
                    cwd = \"sub\"\nenv = { DOCS_TOKEN = { env = \"DOCS_TOKEN\" }, MODE = \"fast\" }\n\
                    network = true\ndescription = \"Search the docs\"\n\
                    [mcp.servers.tracker]\nurl = \"https://tracker.example.com/mcp\"\n\
                    headers = { Authorization = { env = \"TRACKER_AUTH\" } }\n\
                    tools = [\"create_issue\"]\ndeny_tools = [\"delete_issue\"]\n\
                    expose = \"direct\"\ntimeout_ms = 30000\nenabled = false\n\
                    start_timeout_ms = 5000\n\
                    [mcp.servers.broken]\nenabled = true\n\
                    [mcp.servers.typed]\ncommand = \"t\"\ntype = \"stdio\"\n\
                    [mcp.servers.venice]\ncommand = \"v\"\n\
                    env = { K = { env = \"VENICE_API_KEY\" } }\n\
                    [mcp.servers.teamed]\ncommand = \"t\"\n\
                    env = { K = { env = \"TEAM_TOKEN\" } }\n\
                    [providers.openrouter]\nheaders = { X-Team = { env = \"TEAM_TOKEN\" } }\n";
        let project = "[mcp.servers.docs]\ncommand = \"not-yours\"\n\
                       [mcp.servers.lint]\ncommand = \"lint-mcp\"\n\
                       [mcp.servers.remote]\nurl = \"https://r.example.com/mcp\"\n\
                       headers = { Authorization = { env = \"HUB_TOKEN\" } }\n";
        for (test, trust) in [
            ("untrusted", ProjectTrust::Untrusted),
            ("trusted", ProjectTrust::Trusted),
        ] {
            let made = definitions(&loaded(test, user, project, trust));
            let names: Vec<(&str, Origin)> = made
                .servers
                .iter()
                .map(|server| (server.name.as_str(), server.origin))
                .collect();
            assert_eq!(
                names,
                [
                    ("docs", Origin::User),
                    ("tracker", Origin::User),
                    ("lint", Origin::ProjectConfig)
                ]
            );
            let docs = &made.servers[0];
            assert_eq!(
                docs.launch,
                Launch::Local {
                    program: "docs-mcp".to_owned(),
                    args: vec!["--root".to_owned(), ".".to_owned()],
                    cwd: Some("sub".to_owned()),
                    env: vec![
                        EnvEntry {
                            name: "DOCS_TOKEN".to_owned(),
                            value: ValueSource::Env("DOCS_TOKEN".to_owned()),
                        },
                        EnvEntry {
                            name: "MODE".to_owned(),
                            value: ValueSource::Literal(cineris_base::Secret::new("fast")),
                        },
                    ],
                    network: true,
                    write: false,
                }
            );
            assert_eq!(docs.description.as_deref(), Some("Search the docs"));
            assert_eq!(docs.start_timeout, Duration::from_secs(20));
            assert_eq!(docs.call_timeout, Duration::from_secs(60));
            assert!(docs.enabled && docs.expose == Expose::Search);
            let tracker = &made.servers[1];
            assert_eq!(
                tracker.launch,
                Launch::Remote {
                    url: "https://tracker.example.com/mcp".to_owned(),
                    headers: vec![EnvEntry {
                        name: "Authorization".to_owned(),
                        value: ValueSource::Env("TRACKER_AUTH".to_owned()),
                    }],
                }
            );
            assert_eq!(tracker.tools, Some(vec!["create_issue".to_owned()]));
            assert_eq!(tracker.deny_tools, ["delete_issue"]);
            assert_eq!(tracker.expose, Expose::Direct);
            assert!(!tracker.enabled);
            assert_eq!(tracker.start_timeout, Duration::from_secs(5));
            assert_eq!(tracker.call_timeout, Duration::from_secs(30));

            let refused: Vec<(&str, Origin)> = made
                .refused
                .iter()
                .map(|one| (one.name.as_str(), one.origin))
                .collect();
            assert_eq!(
                refused,
                [
                    ("broken", Origin::User),
                    ("teamed", Origin::User),
                    ("typed", Origin::User),
                    ("venice", Origin::User),
                    ("docs", Origin::ProjectConfig),
                    ("remote", Origin::ProjectConfig)
                ]
            );
            let why = |at: usize| made.refused[at].why.as_str();
            assert!(why(0).contains("neither"), "{:?}", made.refused);
            // Another client's key refuses that server alone (the full
            // review), and a built-in provider's key is withheld though the
            // user wrote no `key_env` for it.
            // A variable a provider's header reads is a provider's key.
            assert!(why(1).contains("TEAM_TOKEN"), "{}", why(1));
            assert!(why(2).contains("unknown key"), "{}", why(2));
            assert!(why(3).contains("VENICE_API_KEY"), "{}", why(3));
            assert!(why(4).contains("your own configuration"));
            assert!(
                why(5).contains("CINERIS_MCP_REMOTE_AUTHORIZATION"),
                "{}",
                why(5)
            );
            let notice = made.refused[4].notice();
            assert!(
                notice.starts_with("The project's MCP server `docs` is not started"),
                "{notice}"
            );
        }
    }
}
