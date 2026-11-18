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

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "runs on the first run's worker, never on the core thread or the interface's"
)]

use std::io::Write as _;
use std::path::Path;

use toml::{Table, Value};

use crate::document;
use crate::import::{ImportedServer, Launch};
use crate::layer::LayerKind;
use crate::schema::CONFIG_VERSION;
use crate::validate::{Checked, validate};

/// What the first run chose. `None` for a step skipped: its comment then
/// says how to do it later.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FirstRunChoices {
    /// The provider whose key was stored or found, `None` when the step was
    /// skipped.
    pub provider: Option<String>,
    /// What the imports step picked, `None` when it was skipped: then
    /// nothing was picked, and the file says so as it says a choice.
    pub imports: Option<Imports>,
    /// `permissions.mode`, `None` when the step was skipped: the default,
    /// `supervised`, holds.
    pub mode: Option<String>,
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
    if choices.provider.is_none() {
        text.push_str(
            "# The provider was skipped: `cineris auth login <provider>` stores a key.\n",
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
    let mut file = match cineris_base::private::create_new(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(WriteError::Exists(shown));
        }
        Err(error) => return Err(failed(error)),
    };
    let written = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    drop(file);
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

/// The settings as a table.
fn table(choices: &FirstRunChoices) -> Result<Table, WriteError> {
    let mut root = Table::new();
    root.insert(
        "config_version".to_owned(),
        Value::Integer(i64::from(CONFIG_VERSION)),
    );
    if let Some(mode) = &choices.mode {
        let mut permissions = Table::new();
        permissions.insert("mode".to_owned(), Value::String(mode.clone()));
        root.insert("permissions".to_owned(), Value::Table(permissions));
    }
    let imports = choices.imports.clone().unwrap_or_default();
    let mut compat = Table::new();
    compat.insert(
        "instructions".to_owned(),
        Value::Boolean(imports.instructions),
    );
    compat.insert("skills".to_owned(), Value::Boolean(imports.skills));
    root.insert("compat".to_owned(), Value::Table(compat));
    if !imports.servers.is_empty() {
        let mut servers = Table::new();
        for server in &imports.servers {
            if server.refused.is_some() {
                return Err(WriteError::NotRead(
                    "a server that cannot be imported was picked".to_owned(),
                ));
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
        let env = server
            .env
            .iter()
            .map(|name| (name.clone(), variable(name.clone())))
            .collect();
        table.insert("env".to_owned(), Value::Table(env));
    }
    if !server.headers.is_empty() {
        let headers = server
            .headers
            .iter()
            .map(|name| (name.clone(), variable(server.header_variable(name))))
            .collect();
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
    if parsed.table != *meant {
        return Err(WriteError::NotRead(
            "it reads as other settings than were chosen".to_owned(),
        ));
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
        tracker.headers = vec!["Authorization".to_owned()];
        FirstRunChoices {
            provider: Some("anthropic".to_owned()),
            imports: Some(Imports {
                instructions: true,
                skills: false,
                servers: vec![docs, tracker],
            }),
            mode: Some("read-only".to_owned()),
        }
    }

    #[test]
    fn the_choices_are_written_as_settings_that_read_back_strictly() {
        let text = user_config_text(&chosen()).unwrap();
        let parsed: Table = text.parse().unwrap();
        assert_eq!(parsed["permissions"]["mode"].as_str(), Some("read-only"));
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
        assert!(parsed.get("permissions").is_none());
        assert!(parsed.get("mcp").is_none());
    }

    /// A name from another tool's file cannot leave its table: whatever it
    /// holds, it is one key, and the file reads back as one server.
    #[test]
    fn a_hostile_name_stays_one_server() {
        for name in [
            "a]\n[permissions]\nmode = \"unrestricted\"",
            "x\" = 1\n[y",
            "with space",
            "dotted.name",
            "",
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
            assert!(parsed.get("permissions").is_none(), "{name:?}: {text}");
            let servers = parsed["mcp"]["servers"].as_table().unwrap();
            assert_eq!(servers.len(), 1);
            assert!(servers.contains_key(name));
        }
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
