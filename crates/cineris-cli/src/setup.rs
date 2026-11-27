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
use cineris_config::import::{HeaderVariables, ImportedServer, Refusal};
use cineris_config::schema::CompatSettings;
use cineris_config::write::{FirstRunChoices, Imports, WriteError};
use cineris_config::{CredentialSource, Env, StoreKind};
use cineris_core::Core;
use cineris_tui::app::setup::{
    Chosen, Found, KeyStore, McpFile, McpHeader, McpServer, NotWritten, Provider, Providers, Setup,
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

/// `cineris setup …`: a choice the first run asks, made again in the user's
/// own configuration (since step 28's third pull request, the owner's
/// answer: the first run asks whether the user's own skill folders are read
/// without asking, and `cineris setup` changes it later).
pub(crate) fn command(command: crate::args::SetupCommand, name: &'static str) -> u8 {
    use std::io::IsTerminal as _;
    let crate::args::SetupCommand::SkillFolders { value } = command;
    if let Err(code) = crate::not_under_a_session(name, "setup skill-folders") {
        return code;
    }
    let env = Env::from_process();
    let Some(file) = cineris_config::Dirs::from_env(&env).user_config else {
        eprintln!(
            "{name}: your configuration file could not be worked out: the home directory is not known"
        );
        return crate::exit::USAGE;
    };
    let value = match value {
        Some(value) => value,
        None if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() => {
            let mut asked = format!("{}\n", cineris_tui::app::setup::SKILL_FOLDERS_QUESTION);
            for (answer, means) in cineris_tui::app::setup::SKILL_FOLDERS_ANSWERS {
                asked.push_str(&format!("  {answer}: {means}\n"));
            }
            asked.push_str("Read them without asking? [y/N] ");
            crate::print_out(&asked);
            let mut line = String::new();
            let _ = std::io::stdin().read_line(&mut line);
            skill_folders_answer(&line)
        }
        None => {
            eprintln!(
                "{name}: there is no terminal to ask at: `{name} setup skill-folders on` or `off`"
            );
            return crate::exit::USAGE;
        }
    };
    match set_skill_folders(&file, value, &env) {
        Ok(said) => {
            crate::print_out(&crate::visible(&said));
            crate::exit::COMPLETED
        }
        Err(said) => {
            eprintln!("{name}: {}", crate::visible(&said));
            crate::exit::USAGE
        }
    }
}

/// Whether an answer to the skill folders' question is a yes: `y` or `yes`,
/// and anything else, an empty line included, no.
fn skill_folders_answer(line: &str) -> bool {
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Sets `permissions.read_skill_folders` to `value` in the user's file at
/// `file`, and says what it did.
fn set_skill_folders(file: &Path, value: bool, env: &Env) -> Result<String, String> {
    let shown = env.display_path(file);
    let done = cineris_config::write::set_switch(file, "permissions", "read_skill_folders", value)
        .map_err(|error| format!("{shown} was not changed: {error}"))?;
    let what = if value {
        "Your own skill folders are read without asking, and what is read, secrets included, \
         goes to the model's provider"
    } else {
        "A read of your own skill folders is asked about"
    };
    let mut said = match done {
        cineris_config::write::Disabled::Written { .. } => format!(
            "{what}: `permissions.read_skill_folders = {value}` in {shown}. It takes effect in \
             the next session; sessions running now keep what they had.\n"
        ),
        cineris_config::write::Disabled::Already => {
            format!("{what} already ({shown}): nothing was written.\n")
        }
    };
    if matches!(
        done,
        cineris_config::write::Disabled::Written { made: true }
    ) {
        said.push_str(&format!(
            "{shown} was not there, and was made with this alone, so the first run, which starts \
             only while that file is not there, does not start.\n"
        ));
    }
    Ok(said)
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
        let mut servers =
            cineris_config::import::read_servers(&real).map_err(|error| error.to_string())?;
        let mut read = self.read.lock().unwrap_or_else(PoisonError::into_inner);
        name_headers(
            &mut servers,
            path,
            &read,
            &self.core.provider_key_variables(),
        );
        let shown = servers
            .iter()
            .map(|server| shown_server(server, &self.env))
            .collect();
        read.insert(path.to_owned(), servers);
        Ok(shown)
    }

    fn write_config(&self, chosen: &Chosen) -> Result<String, NotWritten> {
        let read = self.read.lock().unwrap_or_else(PoisonError::into_inner);
        let choices = choices(chosen, &read, &self.core.provider_key_variables());
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

/// Gives each header of `servers`, the servers of the file at `path`, a
/// variable of its own among every file the first run has read (`read`,
/// whose rows were drawn, this file's own earlier reading left out) and
/// none that is a provider's key's (`keys`, the configuration's) or one of
/// the harness's (since the step 25 fixes, the review: two servers of
/// another client could be given one variable, and one could be given
/// `ANTHROPIC_API_KEY`). What the row shows is what is written.
fn name_headers(
    servers: &mut [ImportedServer],
    path: &str,
    read: &BTreeMap<String, Vec<ImportedServer>>,
    keys: &[String],
) {
    let mut variables = HeaderVariables::new(keys);
    for (file, others) in read {
        if file != path {
            variables.take(others);
        }
    }
    variables.name(servers);
}

/// A server as the first run shows it, each header saying whether its
/// variable is set already in `env`, the environment the first run started
/// in.
fn shown_server(server: &ImportedServer, env: &Env) -> McpServer {
    McpServer {
        name: server.name.clone(),
        shown: server.shown(),
        hidden: server.hidden(),
        env: server.env.clone(),
        headers: server
            .headers
            .iter()
            .map(|header| McpHeader {
                name: header.name.clone(),
                variable: header.variable.clone(),
                set: env.get(&header.variable).is_some(),
            })
            .collect(),
        host: server.host(),
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
/// one that can be imported, its headers' variables settled among every
/// server picked, from whichever file, and none of `keys`, the providers'
/// (`HeaderVariables::settle`: a header keeps the variable its row showed
/// unless another picked server takes that name from its environment).
fn choices(
    chosen: &Chosen,
    read: &BTreeMap<String, Vec<ImportedServer>>,
    keys: &[String],
) -> FirstRunChoices {
    FirstRunChoices {
        provider: chosen.provider.clone(),
        model: chosen.model.clone(),
        model_shared: chosen.model_shared,
        imports: chosen.imports.as_ref().map(|imports| {
            let mut servers: Vec<ImportedServer> = imports
                .servers
                .iter()
                .filter_map(|(file, name)| {
                    read.get(file)?
                        .iter()
                        .find(|server| &server.name == name && server.refused.is_none())
                        .cloned()
                })
                .collect();
            HeaderVariables::new(keys).settle(&mut servers);
            Imports {
                instructions: imports.instructions,
                skills: imports.skills,
                servers,
            }
        }),
        mode: chosen.mode.clone(),
        read_skill_folders: chosen.read_skill_folders,
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

    /// `cineris setup skill-folders` changes what the first run wrote, in
    /// place: on, then off, then off again writes nothing; and only `y` or
    /// `yes` is a yes to its question.
    #[test]
    fn setup_changes_the_skill_folders_switch_in_place() {
        let scratch = cineris_testkit::Scratch::outside("cineris-cli-unit", "setup_skills");
        let env = env_at(scratch.path());
        let file = scratch.join("config.toml");
        cineris_config::write::write_user_config(&file, &FirstRunChoices::default()).unwrap();
        let first = std::fs::read_to_string(&file).unwrap();
        let switch = |value: bool| {
            std::fs::read_to_string(&file)
                .unwrap()
                .contains(&format!("read_skill_folders = {value}"))
        };
        assert!(switch(false), "the first run's default");
        let said = set_skill_folders(&file, true, &env).unwrap();
        assert!(
            said.contains("read without asking") && said.contains("next session"),
            "{said}"
        );
        assert!(switch(true) && !switch(false));
        set_skill_folders(&file, false, &env).unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            first,
            "one word, back"
        );
        let said = set_skill_folders(&file, false, &env).unwrap();
        assert!(said.contains("nothing was written"), "{said}");
        for yes in ["y\n", "Yes\n"] {
            assert!(skill_folders_answer(yes));
        }
        for no in ["", "\n", "n\n", "ok\n"] {
            assert!(!skill_folders_answer(no));
        }
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
        let shown: Vec<McpServer> = servers
            .iter()
            .map(|server| shown_server(server, &Env::empty(cineris_config::Platform::CURRENT)))
            .collect();
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
            read_skill_folders: true,
        };
        let written = choices(&chosen, &read, &[]);
        assert!(written.read_skill_folders, "a yes is written");
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
    /// the first run says it is found, and reads it at once (the review of
    /// the fixes of the third pull request; read at once since S-L4).
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
        let shown = shown_server(&servers[0], &Env::empty(Platform::CURRENT));
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

    /// A header's variable is one of its own among every file the first run
    /// read, and none of a provider's key, the configuration's own
    /// providers' included; one already set is drawn as set, with the host
    /// its value would go to, and written as the row showed it; reading a
    /// file again gives it the same names (the step 25 fixes, the review).
    #[test]
    fn a_headers_variable_is_its_own_and_one_already_set_is_flagged() {
        let scratch = cineris_testkit::Scratch::outside("cineris-cli-unit", "first_run_headers");
        let home = scratch.dir("home");
        let mut env = env_at(&home);
        env.set("GITHUB_TOKEN", "ghp_the_persons_own");
        let mut dirs = cineris_config::Dirs::from_env(&env);
        dirs.managed_config = Some(scratch.join("managed/config.toml"));
        // The user's file is not there yet (the first run is due), so a
        // provider of the user's own comes from the managed file here.
        let managed = dirs.managed_config.clone().unwrap();
        std::fs::create_dir_all(managed.parent().unwrap()).unwrap();
        std::fs::write(
            &managed,
            "[providers.corp]\nprotocol = \"anthropic-messages\"\nbase_url = \
             \"https://gw.example.com\"\nkey_env = \"CORP_GATEWAY_KEY\"\n",
        )
        .unwrap();
        scratch.file(
            "home/.claude.json",
            r#"{"mcpServers": {
                "github": {"url": "https://mcp.example.org/gh", "headers": {"Token": "x"}},
                "anthropic": {"url": "https://a.example.net/mcp", "headers": {"api-key": "x"}},
                "corp": {"url": "https://c.example.net/mcp", "headers": {"gateway-key": "x"}},
                "tracker": {"url": "https://tracker.example.net/mcp", "headers": {"Token": "x"}},
                "my-tracker": {"url": "https://tracker.example.com/mcp",
                               "headers": {"Authorization": "Bearer one"}}
            }}"#,
        );
        scratch.file(
            "home/.cursor/mcp.json",
            r#"{"mcpServers": {
                "tracker-cli": {"command": "tracker", "env": {"TRACKER_TOKEN": "x"}},
                "my_tracker": {"url": "https://other.example.net/mcp",
                               "headers": {"Authorization": "Bearer two"}}
            }}"#,
        );
        let mut options = cineris_core::CoreOptions::new(
            env.clone(),
            std::sync::Arc::new(cineris_core::TokioTimer),
        );
        options.dirs = Some(dirs.clone());
        let core = Core::start(options).expect("the core starts");
        let user = dirs.user_config.clone().unwrap();
        let setup = ProgramSetup::new(core, env.clone(), scratch.dir("work"), user.clone());
        let found = setup.discover();
        assert_eq!(found.user_mcp.len(), 2, "{found:?}");
        let header = |servers: &[McpServer], name: &str| -> (McpHeader, Option<String>) {
            let server = servers.iter().find(|server| server.name == name).unwrap();
            (server.headers[0].clone(), server.host.clone())
        };
        // The paths as the person reads them, which on Windows may hold `\`.
        let file = |ending: &str| {
            found
                .user_mcp
                .iter()
                .find(|file| file.path.ends_with(ending))
                .map(|file| file.path.clone())
                .unwrap()
        };
        let (claude_file, cursor_file) = (file(".claude.json"), file("mcp.json"));
        let claude = setup.read_servers(&claude_file).unwrap();
        let (github, host) = header(&claude, "github");
        assert_eq!(github.variable, "GITHUB_TOKEN");
        assert!(github.set, "the person's own GITHUB_TOKEN is set");
        assert_eq!(host.as_deref(), Some("mcp.example.org"));
        let (anthropic, _) = header(&claude, "anthropic");
        assert_eq!(anthropic.variable, "MCP_ANTHROPIC_API_KEY");
        assert!(!anthropic.set);
        let (corp, _) = header(&claude, "corp");
        assert_eq!(
            corp.variable, "MCP_CORP_GATEWAY_KEY",
            "the configuration's own provider's key"
        );
        let (first, _) = header(&claude, "my-tracker");
        assert_eq!(first.variable, "MY_TRACKER_AUTHORIZATION");
        let cursor = setup.read_servers(&cursor_file).unwrap();
        let (second, host) = header(&cursor, "my_tracker");
        assert_eq!(
            second.variable, "MY_TRACKER_AUTHORIZATION_2",
            "a server of another file read after it"
        );
        assert_eq!(host.as_deref(), Some("other.example.net"));
        // Read again, a file keeps its names: its own earlier reading does
        // not take them from it.
        let again = setup.read_servers(&claude_file).unwrap();
        assert_eq!(header(&again, "my-tracker").0, first);
        // What is written is what the rows showed.
        let chosen = Chosen {
            imports: Some(Imported {
                instructions: false,
                skills: false,
                servers: vec![
                    (claude_file.clone(), "my-tracker".to_owned()),
                    (cursor_file.clone(), "my_tracker".to_owned()),
                    (claude_file.clone(), "github".to_owned()),
                    (claude_file.clone(), "anthropic".to_owned()),
                    (claude_file.clone(), "tracker".to_owned()),
                    (cursor_file.clone(), "tracker-cli".to_owned()),
                ],
            }),
            ..Chosen::default()
        };
        setup.write_config(&chosen).expect("written");
        let text = std::fs::read_to_string(&user).unwrap();
        for variable in [
            "MY_TRACKER_AUTHORIZATION",
            "MY_TRACKER_AUTHORIZATION_2",
            "GITHUB_TOKEN",
            "MCP_ANTHROPIC_API_KEY",
        ] {
            assert!(
                text.contains(&format!("\"{variable}\"")),
                "{variable}:\n{text}"
            );
        }
        assert!(!text.contains("\"ANTHROPIC_API_KEY\""), "{text}");
        // A file read later that takes from its environment the name an
        // earlier file's header was given: the header is given another at
        // write time, so the token set for the local tool is never sent to
        // the remote server (the review of the step 25 fixes).
        assert_eq!(header(&claude, "tracker").0.variable, "TRACKER_TOKEN");
        assert!(
            text.contains("Token = { env = \"TRACKER_TOKEN_2\" }")
                || text.contains("env = \"TRACKER_TOKEN_2\""),
            "{text}"
        );
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
