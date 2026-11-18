//! `/export`'s files (since step 25; `docs/architecture/09-tui.md`,
//! "Exporting a session"): the writer the program gives the application,
//! which runs it on a worker.
//!
//! **The text is written whole before the file has its name.** It goes to
//! a new file of a name of its own in the same folder (`.cineris-export-…
//! .tmp`), created private to the user (`cineris_base::private`), and is
//! flushed; only then is it given the file's name, by a hard link, which
//! never replaces what has the name and never follows a link there; the
//! name of its own is then removed. A program that ends in the middle, or
//! a write that fails, leaves no file cut short under the name asked for:
//! what fails is removed, and only an end of the process before that can
//! leave the name of its own behind. Where the file system has no hard
//! links (FAT, some network shares), the file is created new under its
//! name and written there, as before: never over one that exists, but a
//! process that ends in the middle can leave it cut short.
//!
//! **On Windows, a name that is not a file's is refused:** a device's
//! (`con.md`, `NUL.json`) and a stream of a file (`notes.txt:x.md`), which
//! would write to the device or hide the export inside the file.

use std::fs::File;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_base::Rng as _;
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
    write_with(folder, file, date, |out, text| out.write_all(text))
}

/// [`write`], with the bytes put in the file by `put`: a test stops it
/// part of the way to look at what is there.
fn write_with(
    folder: &Path,
    file: &ExportFile,
    date: &str,
    put: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
) -> Result<String, ExportError> {
    if let Some(path) = &file.path {
        let path = Path::new(path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            folder.join(path)
        };
        #[cfg(windows)]
        if let Some(why) = not_a_file(&path) {
            return Err(ExportError::Failed(why));
        }
        let beside = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(folder);
        let staged = Staged::write(beside, &path, &file.text, put)?;
        return staged.place(&path);
    }
    let short: String = file.session.chars().take(8).collect();
    let short: String = short
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let ending = file.format.extension();
    let stem = format!("cineris-session-{date}-{short}");
    let name = |n: u32| {
        if n == 1 {
            folder.join(format!("{stem}.{ending}"))
        } else {
            folder.join(format!("{stem}-{n}.{ending}"))
        }
    };
    let staged = Staged::write(folder, &name(1), &file.text, put)?;
    for n in 1..=SAME_DAY_MOST {
        match staged.place(&name(n)) {
            Err(ExportError::Exists(_)) => {}
            other => return other,
        }
    }
    Err(ExportError::Failed(format!(
        "every name from {stem}.{ending} to -{SAME_DAY_MOST} is taken in {}",
        folder.display()
    )))
}

/// The text, written whole to a private file of a name of its own,
/// waiting to be given the file's name. Dropped, the name of its own is
/// removed.
struct Staged<'a> {
    /// The name of its own.
    path: PathBuf,
    /// The text, for a file system with no hard links.
    text: &'a str,
}

impl<'a> Staged<'a> {
    /// Writes `text` with `put` to a new private file in `folder`, and
    /// flushes it; `target` is the file it is for, as the person reads it
    /// in an error.
    fn write(
        folder: &Path,
        target: &Path,
        text: &'a str,
        put: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
    ) -> Result<Self, ExportError> {
        let failed =
            |error: io::Error| ExportError::Failed(format!("{}: {error}", target.display()));
        let mut rng = cineris_base::SystemRng::new();
        let mut tries = 0;
        let (path, mut out) = loop {
            let path = folder.join(format!(".cineris-export-{:016x}.tmp", rng.next_u64()));
            match cineris_base::private::create_new(&path) {
                Ok(out) => break (path, out),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && tries < 16 => {
                    tries += 1;
                }
                Err(error) => return Err(failed(error)),
            }
        };
        let written = put(&mut out, text.as_bytes()).and_then(|()| out.sync_all());
        // Closed before a failed write is removed: Windows removes no file
        // that is open.
        drop(out);
        let staged = Self { path, text };
        written.map_err(failed)?;
        Ok(staged)
    }

    /// Gives the text `target`'s name, never over what has it; returns the
    /// path as the person reads it.
    fn place(&self, target: &Path) -> Result<String, ExportError> {
        let shown = target.display().to_string();
        match std::fs::hard_link(&self.path, target) {
            Ok(()) => Ok(shown),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(ExportError::Exists(shown))
            }
            // No hard links here: the file is created new under its name.
            Err(_) => create(target, self.text),
        }
    }
}

impl Drop for Staged<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Creates `path` new and private, and writes `text` to it whole, or
/// removes what it wrote: where the file system has no hard links.
fn create(path: &Path, text: &str) -> Result<String, ExportError> {
    let shown = path.display().to_string();
    let mut out = match cineris_base::private::create_new(path) {
        Ok(out) => out,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
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

/// Why `path` names no ordinary file on Windows, if it does not: its file
/// name is a device's (`CON`, `PRN`, `AUX`, `NUL`, `COM1` to `COM9`,
/// `LPT1` to `LPT9`, in any case and with any ending, spaces before the
/// ending ignored as Windows ignores them), or a name on the way holds a
/// `:` after the drive, which names a stream of a file.
#[cfg(windows)]
fn not_a_file(path: &Path) -> Option<String> {
    use std::path::Component;
    let shown = path.display();
    let stream = path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().contains(':'))
    });
    if stream {
        return Some(format!(
            "{shown} names a stream inside a file, not a file of its own"
        ));
    }
    let name = path.file_name()?.to_string_lossy().to_ascii_uppercase();
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix)
            .is_some_and(|digit| matches!(digit.as_bytes(), [b'1'..=b'9']))
    };
    let device =
        matches!(stem, "CON" | "PRN" | "AUX" | "NUL") || numbered("COM") || numbered("LPT");
    device.then(|| format!("{shown} is the name of a device on Windows, not of a file"))
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

    /// What is in `folder`, by name.
    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The default name is the date and the session's first eight
    /// characters, with `-2` and so on after it when that is taken; a name
    /// the person gives is taken from the folder they started in, and one
    /// that exists is named and left as it was. Each file is the user's
    /// alone, and nothing else is left in the folder.
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
        assert_eq!(std::fs::read_to_string(&named).unwrap(), "export");
        assert!(cineris_base::private::is_private(Path::new(&named)).unwrap());
        // A folder that is not there: nothing is left behind.
        let failed = write(&at, &file(Some("missing/out.md"), "export"), "2026-11-17");
        assert!(matches!(failed, Err(ExportError::Failed(_))), "{failed:?}");
        assert!(!at.join("missing").exists());
        assert_eq!(
            names(&at),
            [
                "cineris-session-2026-11-17-s_4f2a9c-2.md",
                "cineris-session-2026-11-17-s_4f2a9c.md",
                "notes.md",
                "out.md"
            ],
            "no name of its own is left"
        );
    }

    /// The text is written whole under a name of its own before the file
    /// has its name: while it is being written nothing has the name, so a
    /// program that ends then leaves no file cut short there; a write that
    /// fails part of the way leaves nothing at all (the full review of step
    /// 25's second pull request).
    #[test]
    fn nothing_has_the_name_until_the_text_is_written_whole() {
        let folder = cineris_testkit::scratch::Scratch::outside("cineris-cli-export", "staged");
        let at = folder.path().to_path_buf();
        let target = at.join("out.md");
        let written = write_with(
            &at,
            &file(Some("out.md"), "the export"),
            "2026-11-17",
            |out, text| {
                out.write_all(&text[..4])?;
                assert!(!target.exists(), "the name is taken part of the way");
                out.write_all(&text[4..])
            },
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&written).unwrap(), "the export");
        assert_eq!(names(&at), ["out.md"]);

        let failed = write_with(&at, &file(None, "the export"), "2026-11-17", |out, text| {
            out.write_all(&text[..4])?;
            Err(io::Error::other("the disk is full"))
        });
        assert!(
            matches!(&failed, Err(ExportError::Failed(why)) if why.contains("the disk is full")),
            "{failed:?}"
        );
        assert_eq!(
            names(&at),
            ["out.md"],
            "nothing of the failed write is left"
        );
    }

    /// On Windows a device's name and a stream of a file are refused, and
    /// nothing is written: `con.md` would write to the console, and
    /// `notes.txt:x.md` would hide the export inside `notes.txt`. A name
    /// that only begins as a device's is a file's.
    #[cfg(windows)]
    #[test]
    fn a_device_or_a_stream_is_refused_on_windows() {
        let folder = cineris_testkit::scratch::Scratch::outside("cineris-cli-export", "names");
        let at = folder.path().to_path_buf();
        std::fs::write(at.join("notes.txt"), "mine").unwrap();
        for name in [
            "con.md",
            "NUL.json",
            "Aux .md",
            "com1.md",
            "lpt9.backup.md",
            "notes.txt:x.md",
            "sub:dir\\out.md",
        ] {
            let refused = write(&at, &file(Some(name), "export"), "2026-11-17");
            assert!(
                matches!(&refused, Err(ExportError::Failed(why)) if why.contains("device") || why.contains("stream")),
                "{name}: {refused:?}"
            );
        }
        assert_eq!(
            std::fs::read_to_string(at.join("notes.txt")).unwrap(),
            "mine"
        );
        assert_eq!(names(&at), ["notes.txt"]);
        for name in ["console.md", "com10.md", "lpt.md", "nul-and-void.json"] {
            write(&at, &file(Some(name), "export"), "2026-11-17").unwrap();
        }
    }
}
