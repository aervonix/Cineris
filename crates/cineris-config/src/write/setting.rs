//! One switch of the user's own configuration file set, the rest of the
//! file kept as it is: what `cineris setup skill-folders` does with
//! `permissions.read_skill_folders` (since step 28's third pull request, the
//! owner's answer: the first run asks it, and `cineris setup` changes it
//! later).
//!
//! **The file is edited, never written anew,** as
//! [`disable_extension`](super::disable_extension) edits it:
//!
//! - where the key is on a line of its own, its `true` or `false` is turned
//!   to the other;
//! - where the table is there and the key is not, the key is put on the
//!   line after the table's own header (`[permissions]`);
//! - where neither is, the table is added at the file's end, as the TOML
//!   serialiser writes it;
//! - where it says so already, nothing is written.
//!
//! Anything else (the table written inline or with dotted keys, a value
//! that is not a switch, a file that does not read or is not valid) is
//! refused with what to change by hand. The new text must read back as the
//! old settings with that one value; the file is read again just before the
//! write, and back from the disk after it.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "runs on the command line's main thread, for a subcommand, never on the core thread"
)]

use std::path::Path;

use toml::{Table, Value};

use super::WriteError;
use super::extension::{Disabled, write_edit};
use crate::document;
use crate::key::KeyPath;

/// The text of the user's file `old` (empty when there is none) with the
/// switch `table.key` set to `value`; `None` when it says so already. See
/// the [module documentation](self).
///
/// # Errors
///
/// [`WriteError::NotRead`], with what to change by hand.
pub fn switch_text(
    old: &str,
    table: &str,
    key: &str,
    value: bool,
) -> Result<Option<String>, WriteError> {
    let parsed = document::parse(old).map_err(|failure| {
        WriteError::NotRead(match failure.line {
            Some(line) => format!("it does not read, at line {line}: {}", failure.message),
            None => format!("it does not read: {}", failure.message),
        })
    })?;
    if let Some(errors) = super::extension::errors_of(&parsed) {
        return Err(WriteError::NotRead(format!(
            "it is not a valid configuration, so it is not changed: {errors}"
        )));
    }
    let by_hand = |why: &str| {
        WriteError::NotRead(format!(
            "its `{table}` {why}: set `{key} = {value}` in it by hand"
        ))
    };
    let lines: Vec<&str> = old.split_inclusive('\n').collect();
    let text = match parsed.table.get(table) {
        None => {
            let mut text = old.to_owned();
            if !text.is_empty() {
                if !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push('\n');
            }
            let mut inner = Table::new();
            inner.insert(key.to_owned(), Value::Boolean(value));
            let mut root = Table::new();
            root.insert(table.to_owned(), Value::Table(inner));
            text.push_str(
                &toml::to_string(&root).map_err(|error| WriteError::NotRead(error.to_string()))?,
            );
            text
        }
        Some(Value::Table(inner)) => match inner.get(key) {
            Some(Value::Boolean(now)) if *now == value => return Ok(None),
            Some(Value::Boolean(_)) => {
                let line = parsed
                    .lines
                    .get(&KeyPath::new([table, key]))
                    .copied()
                    .ok_or_else(|| by_hand("is not written on lines of its own"))?;
                super::extension::flipped(old, line, value)
                    .ok_or_else(|| by_hand("does not hold the key on a line of its own"))?
            }
            Some(_) => return Err(by_hand("holds the key as something other than a switch")),
            None => {
                let header = parsed
                    .lines
                    .get(&KeyPath::new([table]))
                    .and_then(|line| usize::try_from(*line).ok())
                    .filter(|line| {
                        lines.get(line.wrapping_sub(1)).is_some_and(|text| {
                            let text = text.trim();
                            text == format!("[{table}]")
                                || text.starts_with(&format!("[{table}] #"))
                        })
                    })
                    .ok_or_else(|| by_hand("is not written under a header of its own"))?;
                let mut text = String::with_capacity(old.len() + key.len() + 8);
                for (at, piece) in lines.iter().enumerate() {
                    text.push_str(piece);
                    if at + 1 == header {
                        if !piece.ends_with('\n') {
                            text.push('\n');
                        }
                        text.push_str(&format!("{key} = {value}\n"));
                    }
                }
                text
            }
        },
        Some(_) => return Err(by_hand("is not a table")),
    };
    let mut meant = parsed.table.clone();
    if let Value::Table(inner) = meant
        .entry(table)
        .or_insert_with(|| Value::Table(Table::new()))
    {
        inner.insert(key.to_owned(), Value::Boolean(value));
    }
    let back = document::parse(&text)
        .map_err(|failure| by_hand(&format!("cannot take the change ({})", failure.message)))?;
    if super::extension::errors_of(&back).is_some() || back.table != meant {
        return Err(by_hand("would not read back as only this change"));
    }
    Ok(Some(text))
}

/// Sets the switch `table.key` to `value` in the user's file at `path`. See
/// the [module documentation](self).
///
/// # Errors
///
/// [`WriteError::NotRead`] when the file cannot take the change as this
/// edit makes it, or changed while it was written;
/// [`WriteError::Failed`] when the disk refused.
pub fn set_switch(
    path: &Path,
    table: &str,
    key: &str,
    value: bool,
) -> Result<Disabled, WriteError> {
    write_edit(path, "`cineris setup`", |old| {
        switch_text(old, table, key, value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switch(text: &str) -> Option<bool> {
        let table: Table = text.parse().unwrap();
        table
            .get("permissions")?
            .get("read_skill_folders")?
            .as_bool()
    }

    /// The switch is put under the table's own header, turned on its own
    /// line, added with its table at the end, or left where it says so; the
    /// rest of the file as it was, and anything else refused.
    #[test]
    fn a_switch_is_set_by_one_change_that_reads_back() {
        let set = |old: &str, value| switch_text(old, "permissions", "read_skill_folders", value);
        let under = "# mine\n[permissions]\nmode = \"supervised\"\n\n[model]\nmain = \"m\"\n";
        let text = set(under, true).unwrap().unwrap();
        assert_eq!(
            text,
            "# mine\n[permissions]\nread_skill_folders = true\nmode = \"supervised\"\n\n[model]\nmain = \"m\"\n"
        );
        let off = set(&text, false).unwrap().unwrap();
        assert_eq!(switch(&off), Some(false));
        assert_eq!(off.replace("false", "true"), text, "one word changed");
        assert_eq!(set(&off, false).unwrap(), None);
        let none = set("[model]\nmain = \"m\"", true).unwrap().unwrap();
        assert!(
            none.ends_with("\n\n[permissions]\nread_skill_folders = true\n"),
            "{none}"
        );
        assert_eq!(switch(&none), Some(true));
        for refused in [
            "permissions = { mode = \"supervised\" }\n",
            "permissions.mode = \"supervised\"\n",
            "[permissions]\nread_skill_folders = 1\n",
            "[unclosed\n",
        ] {
            assert!(
                matches!(set(refused, true), Err(WriteError::NotRead(_))),
                "{refused:?}"
            );
        }
    }
}
