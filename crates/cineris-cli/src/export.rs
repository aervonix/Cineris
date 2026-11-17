//! `/export`'s files (since step 25; `docs/architecture/09-tui.md`,
//! "Exporting a session"): the writer the program gives the application,
//! which runs it on a worker. The file is created new and private to the
//! user (`cineris_base::private`), never over one that exists, and what a
//! write that failed part of the way wrote is removed.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_tui::export::{ExportError, ExportFile, Writer};

/// The most exports one session's default name takes in a day: the
/// `-2`, `-3` and so on after it stop here.
const SAME_DAY_MOST: u32 = 99;

/// The writer of `/export`'s files: a path the person gives is taken from
/// `folder`, the folder they started in, and so is the default name.
pub(crate) fn writer(folder: PathBuf) -> Writer {
    Arc::new(move |file: ExportFile| write(&folder, &file, &today()))
}

/// Today's date where the person is, as the default name has it.
fn today() -> String {
    let now = cineris_base::time::UtcTime::from_system(std::time::SystemTime::now());
    now.map_or_else(
        || "today".to_owned(),
        |now| cineris_base::zone::Zone::system().date_of(now).to_string(),
    )
}

/// Writes `file` from `folder`, with `date` in the default name.
fn write(folder: &Path, file: &ExportFile, date: &str) -> Result<String, ExportError> {
    if let Some(path) = &file.path {
        let path = Path::new(path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            folder.join(path)
        };
        return create(&path, &file.text);
    }
    let short: String = file.session.chars().take(8).collect();
    let short: String = short
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let ending = file.format.extension();
    let stem = format!("cineris-session-{date}-{short}");
    for n in 1..=SAME_DAY_MOST {
        let name = if n == 1 {
            format!("{stem}.{ending}")
        } else {
            format!("{stem}-{n}.{ending}")
        };
        match create(&folder.join(name), &file.text) {
            Err(ExportError::Exists(_)) => {}
            other => return other,
        }
    }
    Err(ExportError::Failed(format!(
        "every name from {stem}.{ending} to -{SAME_DAY_MOST} is taken in {}",
        folder.display()
    )))
}

/// Creates `path` new and private, and writes `text` to it whole, or
/// removes what it wrote.
fn create(path: &Path, text: &str) -> Result<String, ExportError> {
    let shown = path.display().to_string();
    let mut out = match cineris_base::private::create_new(path) {
        Ok(out) => out,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ExportError::Exists(shown));
        }
        Err(error) => return Err(ExportError::Failed(format!("{shown}: {error}"))),
    };
    let written = out.write_all(text.as_bytes()).and_then(|()| out.sync_all());
    if let Err(error) = written {
        drop(out);
        let _ = std::fs::remove_file(path);
        return Err(ExportError::Failed(format!("{shown}: {error}")));
    }
    Ok(shown)
}

#[cfg(test)]
mod tests {
    use cineris_tui::export::Format;

    use super::*;

    fn file(path: Option<&str>, text: &str) -> ExportFile {
        ExportFile {
            path: path.map(str::to_owned),
            session: "s_4f2a9c1e77".to_owned(),
            format: Format::Markdown,
            text: text.to_owned(),
        }
    }

    /// The default name is the date and the session's first eight
    /// characters, with `-2` and so on after it when that is taken; a name
    /// the person gives is taken from the folder they started in, and one
    /// that exists is named and left as it was. Each file is the user's
    /// alone.
    #[test]
    fn an_export_is_a_new_private_file_never_one_that_exists() {
        let folder = cineris_testkit::scratch::Scratch::outside("cineris-cli-export", "files");
        let at = folder.path().to_path_buf();
        let first = write(&at, &file(None, "one"), "2026-11-17").unwrap();
        assert!(
            first.ends_with("cineris-session-2026-11-17-s_4f2a9c.md"),
            "{first}"
        );
        let second = write(&at, &file(None, "two"), "2026-11-17").unwrap();
        assert!(
            second.ends_with("cineris-session-2026-11-17-s_4f2a9c-2.md"),
            "{second}"
        );
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "two");
        assert!(cineris_base::private::is_private(Path::new(&first)).unwrap());

        std::fs::write(at.join("notes.md"), "mine").unwrap();
        assert_eq!(
            write(&at, &file(Some("notes.md"), "export"), "2026-11-17"),
            Err(ExportError::Exists(
                at.join("notes.md").display().to_string()
            ))
        );
        assert_eq!(
            std::fs::read_to_string(at.join("notes.md")).unwrap(),
            "mine"
        );
        let named = write(&at, &file(Some("out.md"), "export"), "2026-11-17").unwrap();
        assert_eq!(std::fs::read_to_string(named).unwrap(), "export");
        // A folder that is not there: nothing is left behind.
        let failed = write(&at, &file(Some("missing/out.md"), "export"), "2026-11-17");
        assert!(matches!(failed, Err(ExportError::Failed(_))), "{failed:?}");
        assert!(!at.join("missing").exists());
    }
}
