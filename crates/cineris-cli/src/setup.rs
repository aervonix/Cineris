//! The first run's program (since step 25;
//! `docs/architecture/08-configuration.md`, "First run"): what the
//! application asks through `cineris_tui::app::Setup`, on its worker.
//!
//! - **The providers** are the core's, each with where its key would come
//!   from: an environment variable by its name, never its value.
//! - **A key** goes to the credential store `cineris auth login` uses.
//! - **What other tools left** is `cineris-config`'s discovery, which opens
//!   nothing; a user's MCP file is read only when the person asks to see
//!   inside it (`cineris_config::import`), its values dropped and its
//!   credentials masked before they leave the reader. What was read is kept
//!   here, so that the file written names exactly the servers shown.
//! - **The configuration file** is written new, through the TOML
//!   serialiser and read back strictly (`cineris_config::write`).
//!
//! It is given to the application only when the user's configuration file
//! does not exist ([`first_run_due`]).

#![allow(
    clippy::disallowed_methods,
    reason = "the first run's calls run on its worker, and the look for the file at start-up \
              comes before the terminal"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use cineris_base::Secret;
use cineris_config::discover::{FoundKind, Scope};
use cineris_config::import::{ImportedServer, Refusal};
use cineris_config::schema::CompatSettings;
use cineris_config::write::{FirstRunChoices, Imports, WriteError};
use cineris_config::{CredentialSource, Env, StoreKind};
use cineris_core::Core;
use cineris_tui::app::setup::{
    Chosen, Found, KeyStore, McpFile, McpServer, NotWritten, Provider, Providers, Setup,
};

/// Whether the first run is due: the user's configuration file, the one
/// `CINERIS_CONFIG` names or `config.toml` in the configuration directory,
/// does not exist. Its path, when it is.
#[must_use]
pub(crate) fn first_run_due(env: &Env) -> Option<PathBuf> {
    let file = cineris_config::Dirs::from_env(env).user_config?;
    // A file that is there, or anything else at its name (a link, even one
    // that leads nowhere), is the person's: the first run never writes
    // over it.
    match std::fs::symlink_metadata(&file) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(file),
        _ => None,
    }
}

/// The first run's program.
pub(crate) struct ProgramSetup {
    core: Core,
    env: Env,
    workspace: PathBuf,
    user_config: PathBuf,
    /// Each file discovery found, by its path as the person reads it.
    files: Mutex<BTreeMap<String, PathBuf>>,
    /// Each file's servers, as read, by its path as the person reads it.
    read: Mutex<BTreeMap<String, Vec<ImportedServer>>>,
}

impl ProgramSetup {
    pub(crate) fn new(core: Core, env: Env, workspace: PathBuf, user_config: PathBuf) -> Self {
        Self {
            core,
            env,
            workspace,
            user_config,
            files: Mutex::default(),
            read: Mutex::default(),
        }
    }
}

impl Setup for ProgramSetup {
    fn providers(&self) -> Result<Providers, String> {
        let providers = self
            .core
            .credentials()
            .into_iter()
            .map(|credential| {
                let (variable, stored) = match &credential.status {
                    Ok(status) => (
                        match &status.active {
                            Some(CredentialSource::Environment { variable }) => {
                                Some(variable.clone())
                            }
                            _ => None,
                        },
                        status.stored,
                    ),
                    Err(_) => (None, false),
                };
                let model = self.core.default_model(&credential.provider);
                Provider {
                    model_shared: model
                        .as_deref()
                        .is_some_and(|model| self.core.model_shared(model)),
                    model,
                    name: credential.provider,
                    variable,
                    stored,
                }
            })
            .collect();
        let (kind, described) = self.core.credential_store();
        Ok(Providers {
            providers,
            store: key_store(kind, &described),
            sandbox: self.core.sandbox_status()
                == cineris_protocol::session::SandboxStatus::Enforcing,
        })
    }

    fn store_key(&self, provider: &str, key: Secret) -> Result<(), String> {
        self.core
            .store_credential(provider, &key)
            .map_err(|error| error.message)
    }

    fn discover(&self) -> Found {
        let found = discovered(&self.env, &self.workspace);
        let mut files = self.files.lock().unwrap_or_else(PoisonError::into_inner);
        for (shown, path) in &found.paths {
            files.insert(shown.clone(), path.clone());
        }
        found.found
    }

    fn read_servers(&self, path: &str) -> Result<Vec<McpServer>, String> {
        let real = self
            .files
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(path)
            .cloned()
            .ok_or_else(|| "it was not among the files found".to_owned())?;
        let servers =
            cineris_config::import::read_servers(&real).map_err(|error| error.to_string())?;
        let shown = servers.iter().map(shown_server).collect();
        self.read
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(path.to_owned(), servers);
        Ok(shown)
    }

    fn write_config(&self, chosen: &Chosen) -> Result<String, NotWritten> {
        let read = self.read.lock().unwrap_or_else(PoisonError::into_inner);
        let choices = choices(chosen, &read);
        let shown = self.env.display_path(&self.user_config);
        cineris_config::write::write_user_config(&self.user_config, &choices)
            .map_err(|error| not_written(error, &shown))?;
        Ok(shown)
    }
}

/// Why the file was not written, as the first run tells it: a file found
/// at its name apart from every other failure (since the review of the
/// fixes of step 25's third pull request: it was said to be nothing saved,
/// and the first run to run again, though the file found stops it).
fn not_written(error: WriteError, shown: &str) -> NotWritten {
    match error {
        WriteError::Exists(_) => NotWritten::Found(shown.to_owned()),
        other => NotWritten::Failed(other.to_string()),
    }
}

/// Where a key is kept, from the store's kind and its words.
pub(crate) fn key_store(kind: StoreKind, described: &str) -> KeyStore {
    match kind {
        StoreKind::CredentialManager => KeyStore::CredentialManager,
        StoreKind::File => match described.strip_prefix("the file ") {
            Some(path) => KeyStore::File(path.to_owned()),
            None => KeyStore::Unavailable(described.to_owned()),
        },
    }
}

/// What discovery found, and where each file is.
struct Discovered {
    found: Found,
    paths: Vec<(String, PathBuf)>,
}

/// What other tools left in `workspace` and the home directory: the MCP
/// files of the user's, and the project's own, which are never imported.
fn discovered(env: &Env, workspace: &Path) -> Discovered {
    let mut found = Found::default();
    let mut paths = Vec::new();
    for item in cineris_config::discover::discover(env, Some(workspace), &CompatSettings::default())
    {
        if item.kind != FoundKind::McpServers {
            continue;
        }
        match item.scope {
            Scope::User => {
                let shown = env.display_path(&item.path);
                paths.push((shown.clone(), item.path.clone()));
                found.user_mcp.push(McpFile {
                    path: shown,
                    from: item.from.to_owned(),
                });
            }
            Scope::Project => {
                let shown = item
                    .path
                    .strip_prefix(workspace)
                    .map_or_else(|_| item.path.clone(), Path::to_path_buf)
                    .to_string_lossy()
                    .replace('\\', "/");
                found.project_mcp.push(shown);
            }
        }
    }
    Discovered { found, paths }
}

/// A server as the first run shows it.
fn shown_server(server: &ImportedServer) -> McpServer {
    McpServer {
        name: server.name.clone(),
        shown: server.shown(),
        hidden: server.hidden(),
        env: server.env.clone(),
        headers: server
            .headers
            .iter()
            .map(|header| (header.clone(), server.header_variable(header)))
            .collect(),
        refused: server.refused.map(|refusal| {
            match refusal {
                Refusal::HoldsCredential => "holds a credential in its command",
                Refusal::Unseen => "holds a character that acts unseen",
                Refusal::Name => "has a name that cannot be a setting's",
                Refusal::NamedValue => {
                    "passes a value named as a credential that cannot be imported unseen \
                     (move it into a variable)"
                }
                Refusal::NotUnderstood => "is not a server this version reads",
            }
            .to_owned()
        }),
    }
}

/// What to write for `chosen`: each server picked as it was read, and only
/// one that can be imported.
fn choices(chosen: &Chosen, read: &BTreeMap<String, Vec<ImportedServer>>) -> FirstRunChoices {
    FirstRunChoices {
        provider: chosen.provider.clone(),
        model: chosen.model.clone(),
        model_shared: chosen.model_shared,
        imports: chosen.imports.as_ref().map(|imports| Imports {
            instructions: imports.instructions,
            skills: imports.skills,
            servers: imports
                .servers
                .iter()
                .filter_map(|(file, name)| {
                    read.get(file)?
                        .iter()
                        .find(|server| &server.name == name && server.refused.is_none())
                        .cloned()
                })
                .collect(),
        }),
        mode: chosen.mode.clone(),
    }
}

#[cfg(test)]
mod tests {
    use cineris_config::Platform;
    use cineris_tui::app::setup::Imported;

    use super::*;

    fn env_at(home: &Path) -> Env {
        let mut env = Env::empty(Platform::CURRENT);
        let home = home.to_str().unwrap();
        for name in ["HOME", "USERPROFILE"] {
            env.set(name, home);
        }
        env.set("XDG_CONFIG_HOME", format!("{home}/.config"));
        env.set("APPDATA", format!("{home}/AppData/Roaming"));
        env
    }

    #[test]
    fn the_first_run_is_due_only_where_no_file_is() {
        let scratch = cineris_testkit::Scratch::outside("cineris-cli-unit", "first_run_due");
        let env = env_at(&scratch.dir("home"));
        let due = first_run_due(&env).expect("no file yet");
        assert!(due.ends_with("config.toml"), "{}", due.display());
        std::fs::create_dir_all(due.parent().unwrap()).unwrap();
        std::fs::write(&due, "").unwrap();
        assert_eq!(first_run_due(&env), None);
        let mut named = env.clone();
        named.set(
            "CINERIS_CONFIG",
            scratch.join("elsewhere.toml").to_str().unwrap(),
        );
        assert_eq!(first_run_due(&named), Some(scratch.join("elsewhere.toml")));
    }

    #[test]
    fn discovery_lists_the_users_files_and_the_projects_apart() {
        let scratch = cineris_testkit::Scratch::outside("cineris-cli-unit", "first_run_found");
        let home = scratch.dir("home");
        let workspace = scratch.dir("work");
        scratch.file("home/.claude.json", "{}");
        scratch.file("work/.mcp.json", "{}");
        let found = discovered(&env_at(&home), &workspace);
        assert_eq!(
            found.found.user_mcp,
            [McpFile {
                path: "~/.claude.json".to_owned(),
                from: "Claude Code".to_owned(),
            }]
        );
        assert_eq!(found.found.project_mcp, [".mcp.json"]);
        assert_eq!(found.paths.len(), 1, "a project's file is never read");
    }

    #[test]
    fn only_a_server_shown_and_importable_is_written() {
        let servers = cineris_config::import::servers_in(
            r#"{"mcpServers": {
                "docs": {"command": "docs-mcp", "env": {"TOKEN": "tk_live_0123456789abcdef"}},
                "github": {"command": "docker", "args": ["-e", "GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789"]}
            }}"#,
            false,
        )
        .unwrap();
        let shown: Vec<McpServer> = servers.iter().map(shown_server).collect();
        assert_eq!(
            shown[1].refused.as_deref(),
            Some("holds a credential in its command")
        );
        assert!(!format!("{shown:?}").contains("ghp_abc"));
        let mut read = BTreeMap::new();
        read.insert("~/.claude.json".to_owned(), servers);
        let chosen = Chosen {
            provider: Some("anthropic".to_owned()),
            model: Some("claude-sonnet-5-5".to_owned()),
            model_shared: true,
            imports: Some(Imported {
                instructions: true,
                skills: false,
                servers: vec![
                    ("~/.claude.json".to_owned(), "docs".to_owned()),
                    ("~/.claude.json".to_owned(), "github".to_owned()),
                    ("~/other.json".to_owned(), "docs".to_owned()),
                ],
            }),
            mode: Some("read-only".to_owned()),
        };
        let written = choices(&chosen, &read);
        assert_eq!(written.provider.as_deref(), Some("anthropic"));
        assert_eq!(
            written.model.as_deref(),
            Some("claude-sonnet-5-5"),
            "the model chosen with the provider is written"
        );
        assert!(
            written.model_shared,
            "and whether the provider is named beside it"
        );
        let imports = written.imports.unwrap();
        assert!(imports.instructions);
        let names: Vec<&str> = imports
            .servers
            .iter()
            .map(|server| server.name.as_str())
            .collect();
        assert_eq!(names, ["docs"]);
    }

    /// A file found at the name is told apart from a write that failed:
    /// the first run says it is in force from the next start (the review
    /// of the fixes of the third pull request).
    #[test]
    fn a_file_found_at_the_name_is_not_a_failure() {
        let shown = "~/.config/cineris/config.toml";
        assert_eq!(
            not_written(WriteError::Exists("config.toml".to_owned()), shown),
            NotWritten::Found(shown.to_owned())
        );
        assert!(matches!(
            not_written(WriteError::NotRead("no".to_owned()), shown),
            NotWritten::Failed(_)
        ));
        let scratch = cineris_testkit::Scratch::outside("cineris-cli-unit", "first_run_found_file");
        let path = scratch.file("config.toml", "# the person's own\n");
        let error = cineris_config::write::write_user_config(&path, &FirstRunChoices::default())
            .unwrap_err();
        assert_eq!(
            not_written(error, shown),
            NotWritten::Found(shown.to_owned())
        );
    }

    /// A server whose value under a credential's name cannot be imported
    /// unseen is refused with a row saying to move it into a variable (the
    /// review of the fixes of the third pull request; the words since the
    /// short read of B, under which a token can refuse too, after a name
    /// with a shell's character or in an argument of `cmd`).
    #[test]
    fn a_loose_value_under_a_credentials_name_says_to_move_it() {
        let servers = cineris_config::import::servers_in(
            r#"{"mcpServers": {"db": {"command": "db-mcp", "args": ["--password", "a b c"]}}}"#,
            false,
        )
        .unwrap();
        let shown = shown_server(&servers[0]);
        assert_eq!(
            shown.refused.as_deref(),
            Some(
                "passes a value named as a credential that cannot be imported unseen \
                 (move it into a variable)"
            )
        );
        assert!(!shown.shown.contains("a b c"), "{}", shown.shown);
    }

    /// The program says a provider's model is shared when another provider
    /// has a model of its id in the core's catalog, the user's entries
    /// included, and only then (the short review of the fixes, Low 6:
    /// nothing caught the program dropping it).
    #[test]
    fn a_providers_model_is_shared_when_another_provider_has_its_id() {
        let scratch = cineris_testkit::Scratch::outside("cineris-cli-unit", "first_run_shared");
        let env = env_at(&scratch.dir("home"));
        let mut dirs = cineris_config::Dirs::from_env(&env);
        dirs.managed_config = Some(scratch.join("managed/config.toml"));
        let user = dirs.user_config.clone().unwrap();
        std::fs::create_dir_all(user.parent().unwrap()).unwrap();
        let anthropic = |config: &str| {
            std::fs::write(&user, config).unwrap();
            let mut options = cineris_core::CoreOptions::new(
                env.clone(),
                std::sync::Arc::new(cineris_core::TokioTimer),
            );
            options.dirs = Some(dirs.clone());
            let core = Core::start(options).expect("the core starts");
            ProgramSetup::new(core, env.clone(), scratch.dir("work"), user.clone())
                .providers()
                .unwrap()
                .providers
                .into_iter()
                .find(|provider| provider.name == "anthropic")
                .unwrap()
        };
        let keys = "[credentials]\nstore = \"file\"\n";
        let alone = anthropic(keys);
        assert_eq!(alone.model.as_deref(), Some("claude-sonnet-5-5"));
        assert!(!alone.model_shared);
        let twin = anthropic(&format!(
            "{keys}\n[models.twin]\nprovider = \"local\"\nid = \"claude-sonnet-5-5\"\nname = \
             \"Twin\"\nprotocol = \"anthropic-messages\"\ncontext_window = 100000\nmax_output = \
             8000\n\n[providers.local]\nprotocol = \"anthropic-messages\"\nbase_url = \
             \"http://127.0.0.1:9\"\n"
        ));
        assert_eq!(twin.model.as_deref(), Some("claude-sonnet-5-5"));
        assert!(twin.model_shared, "another provider has a model of its id");
    }

    #[test]
    fn a_key_is_kept_where_the_store_says() {
        assert_eq!(
            key_store(
                StoreKind::File,
                "the file ~/.config/cineris/credentials.toml"
            ),
            KeyStore::File("~/.config/cineris/credentials.toml".to_owned())
        );
        assert_eq!(
            key_store(StoreKind::CredentialManager, "Windows Credential Manager"),
            KeyStore::CredentialManager
        );
        assert!(matches!(
            key_store(StoreKind::File, "no credential store"),
            KeyStore::Unavailable(_)
        ));
    }
}
