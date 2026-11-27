//! The user's configuration file, written new at the end of the first run
//! (since step 25; `docs/architecture/08-configuration.md`, "First run").
//!
//! **It writes a file that does not exist, and nothing else.** It never
//! edits one: a file that is there, or anything at its name, is left as it
//! is and the write refused ([`WriteError::Exists`]).
//!
//! **What it writes goes through the TOML serialiser,** never text put
//! together, so that a name from another tool's file cannot escape its
//! table. Before the file is created the text is read as the loader reads
//! a user's file, strictly (an unknown key is an error), and compared with
//! what was meant; after it is written it is read back from the disk and
//! compared again, and a file that does not read back as meant is removed.
//!
//! The file is private to the user (`cineris_base::private`), as the
//! credentials file is: it can name the servers a person uses.
//!
//! **Since the full review of step 25's third pull request:**
//!
//! - **The model is written:** the chosen provider's model, the one the
//!   model catalog marks as its default, as `model.main`, so that the first
//!   prompt has a model with no other setting. *Since the review of the
//!   fixes:* the provider is written as the main role's
//!   (`model.roles.main.provider`) only beside a model whose id another
//!   provider has too, which needs it, and never without a model: written
//!   always, it held every later `model.main` (`--model`, a hand edit) to
//!   that provider's models.
//! - **`[mcp]` is checked as the loader does not check it:** the section is
//!   reserved in this version, so the strict read-back passes over it. A
//!   server's name must be settable ([`settable_name`]), two servers picked
//!   may not share a name, and at most [`MAX_SERVERS`] are written; the
//!   table read back must hold exactly those servers, each in 13-mcp's
//!   form ([`WriteError::NotRead`] otherwise). *Since the step 25 fixes (the
//!   review):* each header's value is taken from the variable the import
//!   gave it ([`HeaderVariables`]), and the table read back is refused when
//!   two headers take theirs from one variable, in any case, or one from a
//!   variable of the harness's ([`harnesss_variable`]: a provider's key's,
//!   or one that begins `CINERIS_`).
//! - **The file appears whole or not at all.** Its text goes to a private
//!   file of its own beside it, is flushed, and is given the file's name
//!   by a hard link, which refuses a name that is taken (a rename would
//!   replace it); a crash leaves that file behind, never part of the
//!   settings under the name the first run looks for.
//!
//! [`settable_name`]: crate::import::settable_name
//! [`MAX_SERVERS`]: crate::import::MAX_SERVERS
//! [`HeaderVariables`]: crate::import::HeaderVariables
//! [`harnesss_variable`]: crate::import::harnesss_variable

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "runs on the first run's worker, never on the core thread or the interface's"
)]

use std::io::Write as _;
use std::path::Path;

use toml::{Table, Value};

use crate::document;
use crate::import::{ImportedServer, Launch, MAX_SERVERS, harnesss_variable, settable_name};

pub mod extension;
pub mod setting;

use crate::layer::LayerKind;
use crate::schema::{CONFIG_VERSION, table_of};
use crate::validate::{Checked, validate};
pub use extension::{Disabled, disable_extension};
pub use setting::set_switch;

/// What the first run chose. `None` for a step skipped: its comment then
/// says how to do it later.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirstRunChoices {
    /// The provider whose key was stored or found, `None` when the step was
    /// skipped: written as the main role's provider beside a model whose id
    /// another provider has too ([`model_shared`](Self::model_shared)), and
    /// otherwise only said in the file's comment.
    pub provider: Option<String>,
    /// The provider's model, the one the catalog marks as its default:
    /// written as `model.main`. `None` when no provider was chosen, or the
    /// catalog marks none of its models.
    pub model: Option<String>,
    /// Whether another provider has a model of [`model`](Self::model)'s id
    /// too: the provider is then named beside it, as the main role's (since
    /// the review of the fixes of step 25's third pull request).
    pub model_shared: bool,
    /// What the imports step picked, `None` when it was skipped: then
    /// nothing was picked, and the file says so as it says a choice.
    pub imports: Option<Imports>,
    /// `permissions.mode`, `None` when the step was skipped: the default,
    /// `supervised`, holds.
    pub mode: Option<String>,
    /// `permissions.read_skill_folders`: written always, `false` unless the
    /// person said yes to reading their own skill folders without asking,
    /// a skip being no (since step 28's third pull request, the owner's
    /// answer).
    pub read_skill_folders: bool,
}

/// What the imports step picked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imports {
    /// `compat.instructions`.
    pub instructions: bool,
    /// `compat.skills`.
    pub skills: bool,
    /// The servers picked, each written as an `[mcp.servers.<name>]`
    /// table.
    pub servers: Vec<ImportedServer>,
}

/// Why the file was not written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// Something is at its name already: nothing was written over it.
    #[error("{0} is there already: it was left as it is")]
    Exists(String),
    /// What would be written does not read back strictly as meant, so
    /// nothing was kept. A server that cannot be written is one.
    #[error("the settings would not read back as chosen: {0}")]
    NotRead(String),
    /// The disk refused, and what was written of it was removed.
    #[error("{path} could not be written: {error}")]
    Failed {
        /// The file.
        path: String,
        /// What the system said.
        error: String,
    },
}

/// The file's text for `choices`: a comment for each step, saying how to
/// do it again or later, then the settings.
///
/// # Errors
///
/// [`WriteError::NotRead`] when it would not read back strictly as meant:
/// a server picked that cannot be imported, say.
pub fn user_config_text(choices: &FirstRunChoices) -> Result<String, WriteError> {
    let table = table(choices)?;
    let body = toml::to_string(&table).map_err(|error| WriteError::NotRead(error.to_string()))?;
    let mut text = String::from(
        "# Cineris's settings, written by its first run. Each step can be\n\
         # done again: `cineris auth login <provider>` stores a key, `/mode` sets\n\
         # the mode, `/project` shows what is kept of a project, and this file is\n\
         # yours to edit.\n",
    );
    match (&choices.provider, &choices.model) {
        (None, _) => text.push_str(
            "# The provider was skipped: `cineris auth login <provider>` stores a key, and\n\
             # model.main names the model.\n",
        ),
        (Some(_), None) => {
            text.push_str("# No model of the provider chosen is known: model.main names one.\n")
        }
        (Some(_), Some(_)) => text.push_str(
            "# model.main is the model the catalog marks as the default of the provider\n\
             # chosen: `/model` chooses another for a session, and a change here keeps one.\n",
        ),
    }
    if choices.model.is_some() && choices.model_shared {
        text.push_str(
            "# Another provider has a model of that id too, so model.roles.main.provider\n\
             # names the one chosen: change or remove it with model.main.\n",
        );
    }
    if choices.imports.is_none() {
        text.push_str(
            "# The imports were skipped: nothing of other tools' is read. Set\n\
             # compat.instructions or compat.skills to true to read their files.\n",
        );
    }
    if choices.mode.is_none() {
        text.push_str("# The mode was skipped: supervised holds, and `/mode` changes it.\n");
    }
    if !choices.read_skill_folders {
        text.push_str(
            "# A read of your own skill folders is asked about: `cineris setup skill-folders`\n\
             # changes it.\n",
        );
    }
    text.push('\n');
    text.push_str(&body);
    reads_back(&text, &table)?;
    Ok(text)
}

/// Writes the user's configuration file at `path`, new: its folder is made
/// when it is not there. See the [module documentation](self).
///
/// # Errors
///
/// [`WriteError`]: something is at `path`, what would be written does not
/// read back as meant, or the disk refused.
pub fn write_user_config(path: &Path, choices: &FirstRunChoices) -> Result<(), WriteError> {
    let text = user_config_text(choices)?;
    let shown = path.display().to_string();
    let failed = |error: std::io::Error| WriteError::Failed {
        path: shown.clone(),
        error: error.to_string(),
    };
    if let Some(folder) = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
    {
        cineris_base::private::create_dir_all(folder).map_err(failed)?;
    }
    // Something at the name already: nothing is made beside it.
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(WriteError::Exists(shown));
    }
    // The text, whole and flushed, in a private file of its own beside the
    // file; then the file's name, which a hard link takes only when it is
    // free.
    let (new, mut file) = beside(path).map_err(failed)?;
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = written {
        let _ = std::fs::remove_file(&new);
        return Err(failed(error));
    }
    cineris_base::hook::point(BEFORE_THE_NAME);
    let named = std::fs::hard_link(&new, path);
    let _ = std::fs::remove_file(&new);
    let written = match named {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(WriteError::Exists(shown));
        }
        // A file system with no hard links: the text is written at the
        // name itself, made new.
        Err(_) => match cineris_base::private::create_new(path) {
            Ok(mut file) => file
                .write_all(text.as_bytes())
                .and_then(|()| file.sync_all()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(WriteError::Exists(shown));
            }
            Err(error) => return Err(failed(error)),
        },
    };
    let back = written.and_then(|()| std::fs::read_to_string(path));
    let kept = match back {
        Ok(back) if back == text => Ok(()),
        Ok(_) => Err(WriteError::NotRead(
            "the file on the disk is not what was written".to_owned(),
        )),
        Err(error) => Err(failed(error)),
    };
    if kept.is_err() {
        let _ = std::fs::remove_file(path);
    }
    kept
}

/// The point of [`write_user_config`] where the file's text is whole and
/// flushed in a file of its own beside it, before it takes the file's
/// name: a test holds it (`cineris_base::hook`) to see that nothing is at
/// the name yet.
pub const BEFORE_THE_NAME: &str =
    "write_user_config: the text is whole beside the file, before it takes the file's name";

/// A new private file beside `path`, named for it, to write its text in.
fn beside(path: &Path) -> std::io::Result<(std::path::PathBuf, std::fs::File)> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    for attempt in 0..16_u32 {
        let new = path.with_file_name(format!(".{name}.{}-{attempt}.new", std::process::id()));
        match cineris_base::private::create_new(&new) {
            Ok(file) => return Ok((new, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no name beside the file was free",
    ))
}

/// The settings as a table.
fn table(choices: &FirstRunChoices) -> Result<Table, WriteError> {
    let mut root = Table::new();
    root.insert(
        "config_version".to_owned(),
        Value::Integer(i64::from(CONFIG_VERSION)),
    );
    // The model of the provider chosen (since the full review of step 25's
    // third pull request), and the provider as the main role's only beside
    // a model whose id another provider has too: named always, it held a
    // later `model.main` to that provider's models (the review of the
    // fixes).
    let mut model = Table::new();
    if let Some(main) = &choices.model {
        model.insert("main".to_owned(), Value::String(main.clone()));
        if choices.model_shared
            && let Some(provider) = &choices.provider
        {
            let mut main = Table::new();
            main.insert("provider".to_owned(), Value::String(provider.clone()));
            let mut roles = Table::new();
            roles.insert("main".to_owned(), Value::Table(main));
            model.insert("roles".to_owned(), Value::Table(roles));
        }
    }
    if !model.is_empty() {
        root.insert("model".to_owned(), Value::Table(model));
    }
    let mut permissions = Table::new();
    if let Some(mode) = &choices.mode {
        permissions.insert("mode".to_owned(), Value::String(mode.clone()));
    }
    permissions.insert(
        "read_skill_folders".to_owned(),
        Value::Boolean(choices.read_skill_folders),
    );
    root.insert("permissions".to_owned(), Value::Table(permissions));
    let imports = choices.imports.clone().unwrap_or_default();
    let mut compat = Table::new();
    compat.insert(
        "instructions".to_owned(),
        Value::Boolean(imports.instructions),
    );
    compat.insert("skills".to_owned(), Value::Boolean(imports.skills));
    root.insert("compat".to_owned(), Value::Table(compat));
    if !imports.servers.is_empty() {
        if imports.servers.len() > MAX_SERVERS {
            return Err(WriteError::NotRead(format!(
                "more than {MAX_SERVERS} servers were picked"
            )));
        }
        let mut servers = Table::new();
        for server in &imports.servers {
            if server.refused.is_some() {
                return Err(WriteError::NotRead(
                    "a server that cannot be imported was picked".to_owned(),
                ));
            }
            if !settable_name(&server.name) {
                return Err(WriteError::NotRead(format!(
                    "the server name {} cannot be a setting's",
                    cineris_base::visible(&server.name)
                )));
            }
            // A second of one name would take the first's place, and the
            // table would still read back as itself.
            if servers.contains_key(&server.name) {
                return Err(WriteError::NotRead(format!(
                    "two servers picked are named {}: one name is one server",
                    cineris_base::visible(&server.name)
                )));
            }
            servers.insert(server.name.clone(), Value::Table(server_table(server)));
        }
        let mut mcp = Table::new();
        mcp.insert("servers".to_owned(), Value::Table(servers));
        root.insert("mcp".to_owned(), Value::Table(mcp));
    }
    Ok(root)
}

/// One server as 13-mcp's form has it: its command and arguments or its
/// address, and each environment variable and header as a reference to a
/// variable. `network` and `expose` are never written, so the defaults
/// hold: no network, behind search.
fn server_table(server: &ImportedServer) -> Table {
    let variable = |name: String| {
        let mut reference = Table::new();
        reference.insert("env".to_owned(), Value::String(name));
        Value::Table(reference)
    };
    let mut table = Table::new();
    match &server.launch {
        Launch::Command { program, args } => {
            table.insert("command".to_owned(), Value::String(program.clone()));
            if !args.is_empty() {
                table.insert(
                    "args".to_owned(),
                    Value::Array(args.iter().cloned().map(Value::String).collect()),
                );
            }
        }
        Launch::Remote { url } => {
            table.insert("url".to_owned(), Value::String(url.clone()));
        }
    }
    if !server.env.is_empty() {
        let env = table_of(
            server
                .env
                .iter()
                .map(|name| (name.clone(), variable(name.clone()))),
        );
        table.insert("env".to_owned(), Value::Table(env));
    }
    if !server.headers.is_empty() {
        let headers = table_of(
            server
                .headers
                .iter()
                .map(|header| (header.name.clone(), variable(header.variable.clone()))),
        );
        table.insert("headers".to_owned(), Value::Table(headers));
    }
    table
}

/// Whether `text` reads, strictly and as a user's file, as `meant`.
fn reads_back(text: &str, meant: &Table) -> Result<(), WriteError> {
    let parsed =
        document::parse(text).map_err(|failure| WriteError::NotRead(failure.message.clone()))?;
    let mut diagnostics = Vec::new();
    let usable = validate(
        &Checked {
            layer: LayerKind::User,
            file: None,
            document: &parsed,
            strict: true,
        },
        &mut diagnostics,
    );
    if !usable
        || diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == crate::layer::Severity::Error)
    {
        let said: Vec<String> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect();
        return Err(WriteError::NotRead(said.join("; ")));
    }
    check_mcp(&parsed.table)?;
    if parsed.table != *meant {
        return Err(WriteError::NotRead(
            "it reads as other settings than were chosen".to_owned(),
        ));
    }
    Ok(())
}

/// The `[mcp]` table as it reads back, checked as the loader does not check
/// it (the section is reserved in this version): only servers, at most
/// [`MAX_SERVERS`] of them, each with a settable name and in 13-mcp's form,
/// a command (with text arguments) or an address, and its environment and
/// headers each a reference to a variable.
fn check_mcp(table: &Table) -> Result<(), WriteError> {
    let bad = |why: &str| Err(WriteError::NotRead(format!("its [mcp] table {why}")));
    let Some(mcp) = table.get("mcp") else {
        return Ok(());
    };
    let Some(mcp) = mcp.as_table() else {
        return bad("is not a table");
    };
    if mcp.keys().any(|key| key != "servers") {
        return bad("holds more than servers");
    }
    let Some(servers) = mcp.get("servers").and_then(Value::as_table) else {
        return bad("has no table of servers");
    };
    if servers.len() > MAX_SERVERS {
        return bad("holds more servers than are written");
    }
    let reference = |value: &Value| {
        value.as_table().is_some_and(|reference| {
            reference.len() == 1 && reference.get("env").is_some_and(Value::is_str)
        })
    };
    // The variables headers are taken from, in capitals: one header's each,
    // none of the harness's, and none a server takes from the environment
    // by its own name (since the step 25 fixes, the review and its review).
    let mut header_variables = std::collections::BTreeSet::new();
    let environment: std::collections::BTreeSet<String> = servers
        .values()
        .filter_map(|server| server.get("env").and_then(Value::as_table))
        .flat_map(|env| env.keys())
        .map(|name| name.to_ascii_uppercase())
        .collect();
    for (name, server) in servers {
        if !settable_name(name) {
            return bad("names a server with a name that cannot be a setting's");
        }
        let Some(server) = server.as_table() else {
            return bad("holds a server that is not a table");
        };
        for (key, value) in server {
            let written = match key.as_str() {
                "command" | "url" => value.is_str(),
                "args" => value
                    .as_array()
                    .is_some_and(|args| args.iter().all(Value::is_str)),
                "env" | "headers" => value
                    .as_table()
                    .is_some_and(|names| names.values().all(reference)),
                _ => false,
            };
            if !written {
                return bad(&format!(
                    "gives a server {} as the first run never writes it",
                    cineris_base::visible(key)
                ));
            }
        }
        if server.contains_key("command") == server.contains_key("url") {
            return bad("holds a server with neither or both of a command and an address");
        }
        let variables = server
            .get("headers")
            .and_then(Value::as_table)
            .into_iter()
            .flat_map(|headers| headers.values())
            .filter_map(|reference| reference.get("env").and_then(Value::as_str));
        for variable in variables {
            if harnesss_variable(variable) {
                return bad(&format!(
                    "takes a header from {}, a variable of the harness's own",
                    cineris_base::visible(variable)
                ));
            }
            if environment.contains(&variable.to_ascii_uppercase()) {
                return bad(&format!(
                    "takes a header from {}, which a server takes from the environment",
                    cineris_base::visible(variable)
                ));
            }
            if !header_variables.insert(variable.to_ascii_uppercase()) {
                return bad(&format!(
                    "takes two headers from one variable, {}",
                    cineris_base::visible(variable)
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::Refusal;

    fn server(name: &str, launch: Launch) -> ImportedServer {
        ImportedServer {
            name: name.to_owned(),
            launch,
            env: Vec::new(),
            headers: Vec::new(),
            refused: None,
        }
    }

    fn chosen() -> FirstRunChoices {
        let mut docs = server(
            "docs",
            Launch::Command {
                program: "npx".to_owned(),
                args: vec!["-y".to_owned(), "@example/docs-mcp".to_owned()],
            },
        );
        docs.env = vec!["DOCS_TOKEN".to_owned()];
        let mut tracker = server(
            "tracker",
            Launch::Remote {
                url: "https://tracker.example.com/mcp".to_owned(),
            },
        );
        tracker.headers = vec![crate::import::Header {
            name: "Authorization".to_owned(),
            variable: "TRACKER_AUTHORIZATION".to_owned(),
        }];
        FirstRunChoices {
            provider: Some("anthropic".to_owned()),
            model: Some("claude-sonnet-5-5".to_owned()),
            model_shared: false,
            imports: Some(Imports {
                instructions: true,
                skills: false,
                servers: vec![docs, tracker],
            }),
            mode: Some("read-only".to_owned()),
            read_skill_folders: true,
        }
    }

    #[test]
    fn the_choices_are_written_as_settings_that_read_back_strictly() {
        let text = user_config_text(&chosen()).unwrap();
        let parsed: Table = text.parse().unwrap();
        assert_eq!(parsed["model"]["main"].as_str(), Some("claude-sonnet-5-5"));
        assert!(
            parsed["model"].get("roles").is_none(),
            "a model no other provider has is named alone: {text}"
        );
        assert!(text.contains("`/model` chooses another"), "{text}");
        assert!(!text.contains("model.roles.main.provider"), "{text}");
        assert_eq!(parsed["permissions"]["mode"].as_str(), Some("read-only"));
        assert_eq!(
            parsed["permissions"]["read_skill_folders"].as_bool(),
            Some(true),
            "a yes is written"
        );
        assert_eq!(parsed["compat"]["instructions"].as_bool(), Some(true));
        assert_eq!(parsed["compat"]["skills"].as_bool(), Some(false));
        let docs = &parsed["mcp"]["servers"]["docs"];
        assert_eq!(docs["command"].as_str(), Some("npx"));
        assert_eq!(
            docs["env"]["DOCS_TOKEN"]["env"].as_str(),
            Some("DOCS_TOKEN")
        );
        let tracker = &parsed["mcp"]["servers"]["tracker"];
        assert_eq!(
            tracker["headers"]["Authorization"]["env"].as_str(),
            Some("TRACKER_AUTHORIZATION")
        );
        for never in ["network", "expose"] {
            assert!(docs.get(never).is_none() && tracker.get(never).is_none());
        }
        assert!(!text.contains("was skipped"), "{text}");
    }

    #[test]
    fn a_skipped_step_is_a_comment_and_nothing_imported() {
        let text = user_config_text(&FirstRunChoices::default()).unwrap();
        for said in [
            "The provider was skipped: `cineris auth login <provider>`",
            "The imports were skipped",
            "The mode was skipped",
        ] {
            assert!(text.contains(said), "{said}: {text}");
        }
        let parsed: Table = text.parse().unwrap();
        assert_eq!(parsed["compat"]["instructions"].as_bool(), Some(false));
        assert_eq!(parsed["compat"]["skills"].as_bool(), Some(false));
        // A skipped mode writes none; the skill folders are written off,
        // a skip being no.
        assert!(parsed["permissions"].get("mode").is_none());
        assert_eq!(
            parsed["permissions"]["read_skill_folders"].as_bool(),
            Some(false)
        );
        assert!(text.contains("`cineris setup skill-folders`"), "{text}");
        assert!(parsed.get("mcp").is_none());
        assert!(parsed.get("model").is_none());
    }

    /// A provider the catalog marks no model of writes no model, nor the
    /// provider without one, and the file says model.main names one (the
    /// review of the fixes of the third pull request: the provider alone
    /// held a model given on the command line to its models).
    #[test]
    fn a_provider_with_no_model_known_writes_no_model_and_says_so() {
        for model_shared in [false, true] {
            let choices = FirstRunChoices {
                provider: Some("internal".to_owned()),
                model_shared,
                ..FirstRunChoices::default()
            };
            let text = user_config_text(&choices).unwrap();
            let parsed: Table = text.parse().unwrap();
            assert!(parsed.get("model").is_none(), "{text}");
            assert!(text.contains("model.main names one"), "{text}");
        }
    }

    /// The provider is named beside the model, as the main role's, only
    /// when another provider has a model of that id: then the model would
    /// not be found without it (the review of the fixes of the third pull
    /// request).
    #[test]
    fn the_provider_is_named_only_beside_a_model_of_an_id_another_has() {
        let mut choices = chosen();
        choices.model_shared = true;
        let text = user_config_text(&choices).unwrap();
        let parsed: Table = text.parse().unwrap();
        assert_eq!(parsed["model"]["main"].as_str(), Some("claude-sonnet-5-5"));
        assert_eq!(
            parsed["model"]["roles"]["main"]["provider"].as_str(),
            Some("anthropic")
        );
        assert!(text.contains("model.roles.main.provider"), "{text}");
        choices.model_shared = false;
        let parsed: Table = user_config_text(&choices).unwrap().parse().unwrap();
        assert!(parsed["model"].get("roles").is_none());
    }

    /// A name from another tool's file cannot leave its table: whatever it
    /// holds, it is one key, and the file reads back as one server.
    #[test]
    fn a_hostile_name_stays_one_server() {
        for name in [
            "a] [permissions] mode = \"unrestricted\"",
            "x\" = 1 [y",
            "with space",
            "dotted.name",
            "# a comment",
        ] {
            let choices = FirstRunChoices {
                imports: Some(Imports {
                    servers: vec![server(
                        name,
                        Launch::Command {
                            program: "run\"\n[permissions]".to_owned(),
                            args: Vec::new(),
                        },
                    )],
                    ..Imports::default()
                }),
                ..FirstRunChoices::default()
            };
            let text = user_config_text(&choices).unwrap();
            let parsed: Table = text.parse().unwrap();
            // Only the skill folders' switch is in `[permissions]`.
            assert_eq!(
                parsed["permissions"].as_table().map(toml::Table::len),
                Some(1),
                "{name:?}: {text}"
            );
            assert!(
                parsed["permissions"].get("mode").is_none(),
                "{name:?}: {text}"
            );
            let servers = parsed["mcp"]["servers"].as_table().unwrap();
            assert_eq!(servers.len(), 1);
            assert!(servers.contains_key(name));
        }
    }

    /// What the `[mcp]` table may hold is checked as the loader does not
    /// check it: a name that cannot be a setting's, two servers of one name
    /// and too many servers are refused before anything is written; a table
    /// read back with anything 13-mcp's form does not have is refused (the
    /// full review of the third pull request).
    #[test]
    fn the_mcp_table_is_checked_as_the_loader_does_not() {
        let named = |names: &[&str]| FirstRunChoices {
            imports: Some(Imports {
                servers: names
                    .iter()
                    .map(|name| {
                        server(
                            name,
                            Launch::Command {
                                program: "run".to_owned(),
                                args: Vec::new(),
                            },
                        )
                    })
                    .collect(),
                ..Imports::default()
            }),
            ..FirstRunChoices::default()
        };
        let long = "n".repeat(crate::import::MAX_NAME + 1);
        for names in [
            vec![""],
            vec!["line\nbreak"],
            vec![long.as_str()],
            vec!["docs", "docs"],
        ] {
            // Each guard of the table on its own, before anything is
            // written: the read-back refuses most of them a second time.
            assert!(
                matches!(table(&named(&names)), Err(WriteError::NotRead(_))),
                "{names:?}"
            );
            assert!(user_config_text(&named(&names)).is_err(), "{names:?}");
        }
        let many: Vec<String> = (0..=MAX_SERVERS).map(|n| format!("s{n}")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(matches!(table(&named(&many)), Err(WriteError::NotRead(_))));
        assert!(user_config_text(&named(&many[..MAX_SERVERS])).is_ok());
        // Read back: a name that cannot be set, and too many servers.
        let bad_name: Table = "[mcp.servers.\"\"]\ncommand = \"run\"\n".parse().unwrap();
        assert!(check_mcp(&bad_name).is_err());
        let too_many: Table = (0..=MAX_SERVERS)
            .map(|n| format!("[mcp.servers.s{n}]\ncommand = \"run\"\n"))
            .collect::<String>()
            .parse()
            .unwrap();
        assert!(check_mcp(&too_many).is_err());
        // Read back: anything but 13-mcp's form is refused.
        for text in [
            "[mcp.servers.a]\ncommand = \"run\"\nnetwork = true\n",
            "[mcp.servers.a]\ncommand = \"run\"\nurl = \"https://x.example.com\"\n",
            "[mcp.servers.a]\nargs = [\"x\"]\n",
            "[mcp.servers.a]\ncommand = \"run\"\nargs = [1]\n",
            "[mcp.servers.a]\ncommand = \"run\"\nenv = { A = \"value\" }\n",
            "[mcp]\nexpose = \"direct\"\n",
        ] {
            let table: Table = text.parse().unwrap();
            assert!(check_mcp(&table).is_err(), "{text}");
        }
        let fine: Table =
            "[mcp.servers.a]\ncommand = \"run\"\nargs = [\"x\"]\nenv = { A = { env = \"A\" } }\n"
                .parse()
                .unwrap();
        assert!(check_mcp(&fine).is_ok());
    }

    /// Read back, two headers that take their values from one variable, in
    /// any case, and one that takes it from a variable of the harness's (a
    /// provider's key's, or one that begins `CINERIS_`), are refused: the
    /// token set for one host would be sent to the other, or the person's
    /// key to the server (the step 25 fixes, the review). Two servers that
    /// take one variable of the environment by its own name, as the other
    /// client's file had it, are written.
    #[test]
    fn two_headers_of_one_variable_or_one_of_the_harnesss_are_refused() {
        let remote = |name: &str, headers: &str| {
            format!(
                "[mcp.servers.{name}]\nurl = \"https://{name}.example.com/mcp\"\nheaders = \
                 {{ {headers} }}\n"
            )
        };
        for text in [
            format!(
                "{}{}",
                remote("one", "Authorization = { env = \"TRACKER\" }"),
                remote("two", "Authorization = { env = \"TRACKER\" }")
            ),
            format!(
                "{}{}",
                remote("one", "Authorization = { env = \"TRACKER\" }"),
                remote("two", "Authorization = { env = \"tracker\" }")
            ),
            remote(
                "one",
                "A = { env = \"ONE_TOKEN\" }, B = { env = \"ONE_TOKEN\" }",
            ),
            remote("anthropic", "api-key = { env = \"ANTHROPIC_API_KEY\" }"),
            remote("openai", "api-key = { env = \"openai_api_key\" }"),
            remote("x", "A = { env = \"CINERIS_CONFIG\" }"),
            format!(
                "{}[mcp.servers.tracker-cli]\ncommand = \"tracker\"\nenv = {{ TRACKER_TOKEN = {{ env = \
                 \"TRACKER_TOKEN\" }} }}\n",
                remote("tracker", "Token = { env = \"tracker_token\" }")
            ),
        ] {
            let table: Table = text.parse().unwrap();
            assert!(check_mcp(&table).is_err(), "{text}");
        }
        let shared_env: Table =
            "[mcp.servers.a]\ncommand = \"run\"\nenv = { GITHUB_TOKEN = { env = \
                                 \"GITHUB_TOKEN\" } }\n[mcp.servers.b]\ncommand = \"run\"\nenv = { \
                                 GITHUB_TOKEN = { env = \"GITHUB_TOKEN\" } }\n"
                .parse()
                .unwrap();
        assert!(check_mcp(&shared_env).is_ok());
        let apart = format!(
            "{}{}",
            remote("one", "Authorization = { env = \"ONE_AUTHORIZATION\" }"),
            remote("two", "Authorization = { env = \"MCP_ANTHROPIC_API_KEY\" }")
        );
        assert!(check_mcp(&apart.parse().unwrap()).is_ok());
    }

    /// The file appears whole or not at all: while its text is written,
    /// nothing is at its name, and the file it was written in is gone once
    /// it has the name (the full review of the third pull request).
    #[test]
    fn the_file_takes_its_name_only_once_it_is_whole() {
        let scratch = cineris_testkit::Scratch::outside("cineris-config-unit", "write_whole");
        let path = scratch.join("cineris/config.toml");
        let hold = cineris_base::hook::Hold::new(BEFORE_THE_NAME);
        let writing = {
            let path = path.clone();
            std::thread::spawn(move || write_user_config(&path, &chosen()))
        };
        assert!(hold.wait_reached(std::time::Duration::from_secs(10)));
        assert!(
            std::fs::symlink_metadata(&path).is_err(),
            "nothing is at the name while the text is written"
        );
        let beside: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        assert_eq!(beside.len(), 1, "{beside:?}");
        assert_eq!(
            std::fs::read_to_string(&beside[0]).unwrap(),
            user_config_text(&chosen()).unwrap(),
            "the text is whole before it takes the name"
        );
        hold.release();
        writing.join().unwrap().unwrap();
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        assert_eq!(left, std::slice::from_ref(&path), "only the file is left");
    }

    /// A file that appears at the name while the text is being written is
    /// left as it is: the hard link refuses a name that is taken, where a
    /// rename would replace it (the full review of the third pull request).
    #[test]
    fn a_file_that_appears_meanwhile_is_left_as_it_is() {
        let scratch = cineris_testkit::Scratch::outside("cineris-config-unit", "write_meanwhile");
        let path = scratch.join("cineris/config.toml");
        let hold = cineris_base::hook::Hold::new(BEFORE_THE_NAME);
        let writing = {
            let path = path.clone();
            std::thread::spawn(move || write_user_config(&path, &chosen()))
        };
        assert!(hold.wait_reached(std::time::Duration::from_secs(10)));
        std::fs::write(&path, "# the person's own\n").unwrap();
        hold.release();
        let written = writing.join().unwrap();
        assert!(matches!(written, Err(WriteError::Exists(_))), "{written:?}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# the person's own\n"
        );
        let left = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(left, 1, "nothing of the first run's is left beside it");
    }

    #[test]
    fn a_server_that_cannot_be_imported_is_never_written() {
        let mut choices = chosen();
        if let Some(imports) = choices.imports.as_mut() {
            imports.servers[0].refused = Some(Refusal::HoldsCredential);
        }
        assert!(matches!(
            user_config_text(&choices),
            Err(WriteError::NotRead(_))
        ));
    }

    #[test]
    fn the_file_is_made_new_and_never_over_one_that_is_there() {
        let scratch = cineris_testkit::Scratch::outside("cineris-config-unit", "write");
        let path = scratch.join("new/folder/config.toml");
        write_user_config(&path, &chosen()).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, user_config_text(&chosen()).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let error = write_user_config(&path, &FirstRunChoices::default()).unwrap_err();
        assert!(matches!(error, WriteError::Exists(_)), "{error}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            written,
            "untouched"
        );
    }
}
