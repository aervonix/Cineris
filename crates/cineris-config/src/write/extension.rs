//! `[extensions.<name>] enabled = false` written into the user's own
//! configuration file: what `cineris ext disable` does (since step 28's third
//! pull request; `docs/architecture/12-extensions.md`, "As built in step 28:
//! managing extensions").
//!
//! **The file is the user's, and is edited, never written anew.** Its text is
//! kept as it is, comments and order included, and one change is made to it:
//!
//! - where it has no entry for the extension, the table is added at its end,
//!   its header written by the TOML serialiser, so that a name that is not a
//!   bare key is quoted (`[extensions."acme.lint"]`) and no name can leave
//!   its table;
//! - where its entry says `enabled = true`, on a line of its own, that value
//!   becomes `false`;
//! - where its entry says `enabled = false`, nothing is written.
//!
//! Anything else (an entry with no `enabled`, one written inline, a file
//! that does not read) is refused with what to change by hand.
//!
//! **What is written is checked before and after.** The new text is read as
//! the loader reads a user's file and must give exactly the old settings
//! with that one value set; just before it is written the file is read again
//! and must be as it was read, so that a change made meanwhile is not lost;
//! and after it is written it is read back from the disk. The write is
//! atomic, through a link at the file's name to where it leads (a dotfiles
//! folder), keeping the file's mode; a file made new is owner-only.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "runs on the command line's main thread, for a subcommand, never on the core thread"
)]

use std::path::Path;

use toml::{Table, Value};

use super::WriteError;
use crate::document;
use crate::key::KeyPath;
use crate::layer::{LayerKind, Severity};
use crate::validate::{Checked, validate};

/// The largest file the loader reads.
const MAX_BYTES: u64 = 1024 * 1024;

/// What [`disable_extension`], and [`set_switch`](super::setting::set_switch),
/// did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disabled {
    /// The file says so now. `made`: the file was not there, and was made
    /// with the table alone.
    Written {
        /// The file was made.
        made: bool,
    },
    /// The file said so already: nothing was written.
    Already,
}

/// The table of the extension `name` as the serialiser writes it, a header
/// and `enabled = false`, so that the name is one key.
fn table_text(name: &str) -> Result<String, WriteError> {
    let mut entry = Table::new();
    entry.insert("enabled".to_owned(), Value::Boolean(false));
    let mut extensions = Table::new();
    extensions.insert(name.to_owned(), Value::Table(entry));
    let mut root = Table::new();
    root.insert("extensions".to_owned(), Value::Table(extensions));
    toml::to_string(&root).map_err(|error| WriteError::NotRead(error.to_string()))
}

/// The text of the user's file with `[extensions.<name>] enabled = false`,
/// from `old`, its text as it is (empty when there is no file); `None` when
/// it says so already. See the [module documentation](self).
///
/// # Errors
///
/// [`WriteError::NotRead`], with what to change by hand: the file does not
/// read, or its entry for the extension cannot be changed by this edit, or
/// the edit would not read back as meant.
pub fn disabled_text(old: &str, name: &str) -> Result<Option<String>, WriteError> {
    let parsed = document::parse(old).map_err(|failure| {
        WriteError::NotRead(match failure.line {
            Some(line) => format!("it does not read, at line {line}: {}", failure.message),
            None => format!("it does not read: {}", failure.message),
        })
    })?;
    if let Some(errors) = errors_of(&parsed) {
        return Err(WriteError::NotRead(format!(
            "it is not a valid configuration, so it is not changed: {errors}"
        )));
    }
    let mut meant = parsed.table.clone();
    let shown = cineris_base::in_code(name);
    let by_hand = |why: &str| {
        WriteError::NotRead(format!(
            "its entry for `{shown}` {why}: set `enabled = false` in it by hand"
        ))
    };
    let new = match parsed.table.get("extensions") {
        None => None,
        Some(Value::Table(extensions)) => match extensions.get(name) {
            None => None,
            Some(Value::Table(entry)) => match entry.get("enabled") {
                Some(Value::Boolean(false)) => return Ok(None),
                Some(Value::Boolean(true)) => {
                    let key = KeyPath::new(["extensions", name, "enabled"]);
                    let line = parsed
                        .lines
                        .get(&key)
                        .copied()
                        .ok_or_else(|| by_hand("is not on a line of its own"))?;
                    Some(
                        flipped(old, line, false)
                            .ok_or_else(|| by_hand("is not written `enabled = true`"))?,
                    )
                }
                _ => return Err(by_hand("has no `enabled` this edit can change")),
            },
            Some(_) => return Err(by_hand("is not a table")),
        },
        Some(_) => {
            return Err(WriteError::NotRead(
                "its `extensions` is not a table".to_owned(),
            ));
        }
    };
    let text = match new {
        Some(text) => text,
        None => {
            let mut text = old.to_owned();
            if !text.is_empty() {
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push('\n');
            }
            text.push_str(&table_text(name)?);
            text
        }
    };
    // What the edit must read back as: the old settings, and the one value.
    let extensions = meant
        .entry("extensions")
        .or_insert_with(|| Value::Table(Table::new()));
    if let Value::Table(extensions) = extensions {
        let entry = extensions
            .entry(name)
            .or_insert_with(|| Value::Table(Table::new()));
        if let Value::Table(entry) = entry {
            entry.insert("enabled".to_owned(), Value::Boolean(false));
        }
    }
    let back = document::parse(&text).map_err(|failure| {
        by_hand(&format!(
            "cannot take the table at the file's end ({})",
            failure.message
        ))
    })?;
    if errors_of(&back).is_some() || back.table != meant {
        return Err(by_hand("would not read back as only this change"));
    }
    Ok(Some(text))
}

/// The errors the loader would give `document` as a user's file, if any.
pub(super) fn errors_of(document: &document::Document) -> Option<String> {
    let mut diagnostics = Vec::new();
    let usable = validate(
        &Checked {
            layer: LayerKind::User,
            file: None,
            document,
            strict: false,
        },
        &mut diagnostics,
    );
    let errors: Vec<String> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| diagnostic.message.clone())
        .collect();
    if usable && errors.is_empty() {
        None
    } else {
        Some(errors.join("; "))
    }
}

/// `text` with the switch of line `line` (counted from 1), after its `=`,
/// made `value`: the other word made this one; `None` when that line holds
/// no such word.
pub(super) fn flipped(text: &str, line: u32, value: bool) -> Option<String> {
    let (from, to) = if value {
        ("false", "true")
    } else {
        ("true", "false")
    };
    let index = usize::try_from(line).ok()?.checked_sub(1)?;
    let mut out = String::with_capacity(text.len() + 1);
    let mut found = false;
    for (at, piece) in text.split_inclusive('\n').enumerate() {
        if at == index {
            let (key, value) = piece.split_once('=')?;
            let trimmed = value.trim_start();
            let rest = trimmed.strip_prefix(from)?;
            let space = &value[..value.len() - trimmed.len()];
            out.push_str(key);
            out.push('=');
            out.push_str(space);
            out.push_str(to);
            out.push_str(rest);
            found = true;
        } else {
            out.push_str(piece);
        }
    }
    found.then_some(out)
}

/// Writes `[extensions.<name>] enabled = false` into the user's file at
/// `path`. See the [module documentation](self).
///
/// # Errors
///
/// [`WriteError::NotRead`] when the file cannot take the change as this
/// edit makes it, or changed while it was written;
/// [`WriteError::Failed`] when the disk refused.
pub fn disable_extension(path: &Path, name: &str) -> Result<Disabled, WriteError> {
    write_edit(path, "`cineris ext disable`", |old| {
        disabled_text(old, name)
    })
}

/// Writes into the user's file at `path` what `edit` makes of its text
/// (empty when there is no file; `None` when nothing is to change), as the
/// [module documentation](self) says: read again just before the write and
/// back from the disk after it, atomically, a file made new owner-only and
/// opened by a comment that names `made_by`.
pub(super) fn write_edit(
    path: &Path,
    made_by: &str,
    edit: impl Fn(&str) -> Result<Option<String>, WriteError>,
) -> Result<Disabled, WriteError> {
    let shown = path.display().to_string();
    let failed = |error: std::io::Error| WriteError::Failed {
        path: shown.clone(),
        error: error.to_string(),
    };
    let read = || -> Result<Option<String>, WriteError> {
        match std::fs::metadata(path) {
            Ok(found) if found.len() > MAX_BYTES => {
                return Err(WriteError::NotRead(
                    "it is larger than the 1 MiB the loader reads".to_owned(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(failed(error)),
        }
        let bytes = std::fs::read(path).map_err(failed)?;
        String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| WriteError::NotRead("it is not UTF-8".to_owned()))
    };
    let old = read()?;
    let made = old.is_none();
    let Some(text) = edit(old.as_deref().unwrap_or_default())? else {
        return Ok(Disabled::Already);
    };
    let text = if made {
        format!("# Cineris's settings, made by {made_by}: this file is yours to edit.\n\n{text}")
    } else {
        text
    };
    if made {
        if let Some(folder) = path
            .parent()
            .filter(|folder| !folder.as_os_str().is_empty())
        {
            cineris_base::private::create_dir_all(folder).map_err(failed)?;
        }
    } else if read()? != old {
        return Err(WriteError::NotRead(
            "it changed while it was being changed: nothing was written; run the command again"
                .to_owned(),
        ));
    }
    let options = cineris_base::AtomicWriteOptions {
        owner_only: made,
        ..cineris_base::AtomicWriteOptions::default()
    };
    cineris_base::write_atomic(
        path,
        text.as_bytes(),
        &options,
        &cineris_base::SystemClock::new(),
    )
    .map_err(|error| WriteError::Failed {
        path: shown.clone(),
        error: error.to_string(),
    })?;
    match std::fs::read_to_string(path) {
        Ok(back) if back == text => Ok(Disabled::Written { made }),
        Ok(_) => Err(WriteError::NotRead(
            "the file on the disk is not what was written".to_owned(),
        )),
        Err(error) => Err(failed(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled(text: &str, name: &str) -> Option<bool> {
        let table: Table = text.parse().unwrap();
        table
            .get("extensions")?
            .get(name)?
            .get("enabled")?
            .as_bool()
    }

    /// The table is added at the end, the rest of the text as it was; a
    /// name that is not a bare key is quoted, so that `acme.lint` is one key;
    /// an entry that says `true` is turned to `false` on its own line; one
    /// that says `false` already is left; anything else is refused with what
    /// to change by hand.
    #[test]
    fn the_users_file_is_edited_by_one_change_that_reads_back() {
        let old = "# mine\nmodel.main = \"m\" # kept\n\n[permissions]\nmode = \"supervised\"";
        let text = disabled_text(old, "tracker").unwrap().unwrap();
        assert!(text.starts_with(old), "{text}");
        assert!(
            text.ends_with("\n\n[extensions.tracker]\nenabled = false\n"),
            "{text}"
        );
        assert_eq!(enabled(&text, "tracker"), Some(false));
        let dotted = disabled_text(old, "acme.lint").unwrap().unwrap();
        assert!(dotted.contains("[extensions.\"acme.lint\"]\n"), "{dotted}");
        assert_eq!(enabled(&dotted, "acme.lint"), Some(false));
        let table: Table = dotted.parse().unwrap();
        assert!(
            table["extensions"].get("acme").is_none(),
            "one key, not a table"
        );
        // An empty file, and one with other extensions' entries.
        assert_eq!(
            disabled_text("", "x").unwrap().unwrap(),
            "[extensions.x]\nenabled = false\n"
        );
        let others = "[extensions.other]\nenabled = false\n";
        let text = disabled_text(others, "x").unwrap().unwrap();
        assert_eq!(enabled(&text, "other"), Some(false));
        assert_eq!(enabled(&text, "x"), Some(false));
        // On again by hand, off again by the command, on its own line.
        let on = "[extensions.x]\nenabled  =  true # back on\n\n[model]\nmain = \"m\"\n";
        let text = disabled_text(on, "x").unwrap().unwrap();
        assert_eq!(
            text,
            "[extensions.x]\nenabled  =  false # back on\n\n[model]\nmain = \"m\"\n"
        );
        assert_eq!(
            disabled_text("[extensions.x]\nenabled = false\n", "x").unwrap(),
            None
        );
        for refused in [
            "[extensions.x]\n",
            "extensions = { x = { enabled = true } }\n",
            "[extensions]\nx = 1\n",
            "extensions = 3\n",
            "[unclosed\n",
            "[permissions]\nmode = \"anything\"\n",
        ] {
            assert!(
                matches!(disabled_text(refused, "x"), Err(WriteError::NotRead(_))),
                "{refused:?}"
            );
        }
        // An inline table of others cannot take a table after it.
        assert!(matches!(
            disabled_text("extensions = { other = { enabled = true } }\n", "x"),
            Err(WriteError::NotRead(_))
        ));
    }

    /// The file is written in place through the disk and read back; one that
    /// is not there is made, owner-only; one that says so already is left.
    #[test]
    fn the_file_is_written_in_place_or_made() {
        let scratch = cineris_testkit::Scratch::outside("cineris-config-unit", "disable");
        let path = scratch.join("cineris/config.toml");
        assert_eq!(
            disable_extension(&path, "x").unwrap(),
            Disabled::Written { made: true }
        );
        let made = std::fs::read_to_string(&path).unwrap();
        assert!(
            made.starts_with("# Cineris's settings, made by `cineris ext disable`"),
            "{made}"
        );
        assert_eq!(enabled(&made, "x"), Some(false));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert_eq!(disable_extension(&path, "x").unwrap(), Disabled::Already);
        assert_eq!(
            disable_extension(&path, "y").unwrap(),
            Disabled::Written { made: false }
        );
        let both = std::fs::read_to_string(&path).unwrap();
        assert!(both.starts_with(&made), "{both}");
        assert_eq!(enabled(&both, "y"), Some(false));
    }
}
