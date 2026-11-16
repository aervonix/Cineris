//! `cargo xtask corpus`: the corpus of real code that performance
//! measurements read (`docs/testing-and-ci.md`, "A tree of real code to
//! measure").
//!
//! It is the `.rs` files of the crates `corpus/dependencies.toml` names and
//! what they need, at the versions `corpus/Cargo.lock` holds, with the
//! checksums that cargo verifies each download against. `fetch` has cargo
//! download them (`cargo fetch --locked`, in a manifest made for the purpose
//! outside the repository), copies the `.rs` files out of cargo's registry
//! into a folder named for the hash that `corpus/PIN` pins, checks that
//! folder against the pin, and only then puts it in its place. The folder is
//! kept outside the repository and outside `target`, so that a `cargo clean`
//! and a new worktree leave it be. What a measurement reads is therefore the
//! same bytes on a desk and on a hosted runner.
//!
//! The count, the hash and the places it is kept are
//! `cortex_testkit::corpus::shared`'s: this file includes it, so that the
//! command and the test that reads the corpus check it with the same code.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use crate::{Result, workspace};

#[path = "../../crates/cortex-testkit/src/corpus/shared.rs"]
mod shared;

use shared::{Measured, Pin};

const USAGE: &str = "\
usage: cargo xtask corpus fetch|check|path|pin [--update]

  fetch          make the pinned corpus of real code if it is not there whole,
                 and say where it is
  check          check the corpus in its place against the pin; fail if it is
                 not there or not whole
  path           print the corpus's folder, after the same check, and nothing
                 else (CORTEX_PERF_SOURCE=$(cargo xtask corpus path))
  pin [--update] make the corpus again from corpus/dependencies.toml and write
                 corpus/Cargo.lock and corpus/PIN: the way to change it.
                 --update takes the newest versions the list allows.

It is kept under $XDG_CACHE_HOME/cortex-harness/corpus (~/.cache/... when that
is not set) or %LOCALAPPDATA%\\cortex-harness\\corpus, in a folder named for its
hash; CORTEX_CORPUS_DIR names another place.";

/// The date given to every file of the corpus: the first of January 2021.
/// A file made by the fetch is not "modified a moment ago" to the index that
/// reads it, and two corpora of the same bytes are the same in this too.
const FILE_DATE: u64 = 1_609_459_200;

/// The folder of the repository's corpus list and lock.
fn source_folder() -> PathBuf {
    workspace::root().join("corpus")
}

/// `cargo xtask corpus …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let words: Vec<&str> = arguments.iter().map(String::as_str).collect();
    let root = shared::system_root()?;
    // `pin` mends a corpus/PIN that is not a pin, so only the others read it.
    let pinned = || pinned(shared::PIN_TEXT);
    match words.as_slice() {
        ["pin"] => repin(&root, false),
        ["pin", "--update"] => repin(&root, true),
        ["fetch"] => {
            let pin = pinned()?;
            let (folder, measured) = fetch(&root, &pin)?;
            summary(&folder, &measured);
            Ok(())
        }
        ["check"] => {
            let pin = pinned()?;
            let folder = shared::folder(&root, &pin);
            let timer = Instant::now();
            let measured = shared::verify(&folder, &pin).map_err(not_whole)?;
            summary(&folder, &measured);
            println!("checked in {:.2} s", timer.elapsed().as_secs_f64());
            Ok(())
        }
        ["path"] => {
            let pin = pinned()?;
            let folder = shared::folder(&root, &pin);
            shared::verify(&folder, &pin).map_err(not_whole)?;
            println!("{}", folder.display());
            Ok(())
        }
        _ => Err(USAGE.to_owned()),
    }
}

/// The pin `text` (corpus/PIN, as this build of `xtask` read it), or why it
/// is not one, and how to mend it.
fn pinned(text: &str) -> Result<Pin> {
    Pin::parse(text).map_err(|problem| {
        format!(
            "corpus/PIN is not a pin: {problem}. `cargo xtask corpus pin` writes it again from \
             corpus/Cargo.lock"
        )
    })
}

fn not_whole(problem: String) -> String {
    format!(
        "the corpus of real code is not usable: {problem}\nFetch it with `cargo xtask corpus fetch`."
    )
}

fn summary(folder: &Path, measured: &Measured) {
    println!(
        "corpus of real code: {} files, {:.1} MB, hash {:016x}\n  {}",
        measured.files,
        measured.bytes as f64 / 1e6,
        measured.hash,
        folder.display()
    );
}

/// What `try` runs before a measurement that reads the corpus: the same as
/// `corpus fetch`.
pub(crate) fn ensure_for_try() -> Result<()> {
    let (folder, measured) = fetch(&shared::system_root()?, &pinned(shared::PIN_TEXT)?)?;
    summary(&folder, &measured);
    Ok(())
}

/// [`ensure`] with the corpus made as cargo makes it.
fn fetch(root: &Path, pin: &Pin) -> Result<(PathBuf, Measured)> {
    ensure(root, pin, &|work| make(work, true))
}

/// The corpus of `pin` under `root`: checked if it is there, made if it is
/// not, and made again if what is there is not the pin. `make` makes a
/// corpus in the folder it is given and says where it is and what it holds;
/// what it made is checked against the pin before anything is put in place,
/// and the folder it worked in is removed whatever came of it.
fn ensure(
    root: &Path,
    pin: &Pin,
    make: &dyn Fn(&Path) -> Result<(PathBuf, Measured)>,
) -> Result<(PathBuf, Measured)> {
    let folder = shared::folder(root, pin);
    if folder.exists() {
        let timer = Instant::now();
        match shared::verify(&folder, pin) {
            Ok(measured) => {
                println!(
                    "the corpus is there, and whole (checked in {:.2} s)",
                    timer.elapsed().as_secs_f64()
                );
                return Ok((folder, measured));
            }
            Err(problem) => {
                println!("{problem}\nmaking it again");
                remove(&folder)?;
            }
        }
    }
    let timer = Instant::now();
    let work = work_folder(root)?;
    let result = make(&work).and_then(|(made, measured)| {
        must_be_the_pin(&measured, pin)?;
        install(&made, &folder, pin)?;
        Ok(measured)
    });
    remove(&work).ok();
    println!("fetched in {:.1} s", timer.elapsed().as_secs_f64());
    result.map(|measured| (folder, measured))
}

/// Refuses a corpus that is not the pin, saying what to do about it.
fn must_be_the_pin(measured: &Measured, pin: &Pin) -> Result<()> {
    if measured.as_pin() == *pin {
        return Ok(());
    }
    Err(format!(
        "cargo gave a corpus of {} files, {} bytes and hash {:016x}, and the pin says \
         {} files, {} bytes and hash {}.\nThe pin is of the versions in corpus/Cargo.lock \
         as the repository holds them: if corpus/dependencies.toml or the lock changed \
         on purpose, run `cargo xtask corpus pin`; if not, a download or a copy went \
         wrong, and nothing was put in place.",
        measured.files,
        measured.bytes,
        measured.hash,
        pin.files,
        pin.bytes,
        pin.name()
    ))
}

/// `pin`: make the corpus from the list, write down the lock and the pin,
/// and keep the corpus where it is looked for.
fn repin(root: &Path, update: bool) -> Result<()> {
    let work = work_folder(root)?;
    let result = (|| {
        // With --update there is no lock to hold to: cargo chooses, from the
        // list. corpus/Cargo.lock is replaced only once the new corpus is
        // made, so a fetch that fails leaves it as it was.
        let (made, measured) = make_with(&work, false, !update)?;
        fs::copy(work.join("Cargo.lock"), source_folder().join("Cargo.lock"))
            .map_err(|error| format!("cannot write corpus/Cargo.lock: {error}"))?;
        let pin = measured.as_pin();
        fs::write(source_folder().join("PIN"), pin.text())
            .map_err(|error| format!("cannot write corpus/PIN: {error}"))?;
        let folder = shared::folder(root, &pin);
        if !folder.exists() {
            install(&made, &folder, &pin)?;
        }
        summary(&folder, &measured);
        println!(
            "wrote corpus/Cargo.lock and corpus/PIN: `git add corpus` and commit them, with \
             docs/testing-and-ci.md's figures if they moved"
        );
        Ok(())
    })();
    remove(&work).ok();
    result
}

/// A folder of this process's own, under the corpus's root, to make a corpus
/// in. One that a killed run left is `.work-<its process id>`: delete it.
fn work_folder(root: &Path) -> Result<PathBuf> {
    let work = root.join(format!(".work-{}", std::process::id()));
    remove(&work)?;
    fs::create_dir_all(&work)
        .map_err(|error| format!("cannot make {}: {error}", work.display()))?;
    Ok(work)
}

/// Removes a folder and what it holds; nothing if it is not there.
fn remove(folder: &Path) -> Result<()> {
    if !folder.exists() {
        return Ok(());
    }
    fs::remove_dir_all(folder)
        .map_err(|error| format!("cannot remove {}: {error}", folder.display()))
}

/// Puts the corpus made in `made` in its place, `folder`. A corpus that is
/// already there by then (two fetches at once) is checked and kept.
fn install(made: &Path, folder: &Path, pin: &Pin) -> Result<()> {
    if fs::rename(made, folder).is_ok() {
        return Ok(());
    }
    shared::verify(folder, pin).map(|_| ()).map_err(|problem| {
        format!(
            "cannot put the corpus in place at {}: {problem}",
            folder.display()
        )
    })
}

/// Makes the corpus in `work/corpus`: has cargo fetch the crates the list
/// names (with `--locked` unless this is a pin), and copies each crate's
/// `.rs` files out of the registry. Returns the folder and what is in it.
fn make(work: &Path, locked: bool) -> Result<(PathBuf, Measured)> {
    make_with(work, locked, true)
}

/// [`make`], with corpus/Cargo.lock given to cargo only when `with_lock`.
fn make_with(work: &Path, locked: bool, with_lock: bool) -> Result<(PathBuf, Measured)> {
    let source = source_folder();
    let list = fs::read_to_string(source.join("dependencies.toml"))
        .map_err(|error| format!("cannot read corpus/dependencies.toml: {error}"))?;
    fs::write(work.join("Cargo.toml"), manifest(&list))
        .and_then(|()| fs::write(work.join("lib.rs"), ""))
        .map_err(|error| format!("cannot write the manifest: {error}"))?;
    if let Some(lock) = with_lock
        .then(|| fs::read(source.join("Cargo.lock")).ok())
        .flatten()
    {
        fs::write(work.join("Cargo.lock"), lock)
            .map_err(|error| format!("cannot write the lock: {error}"))?;
    } else if locked {
        return Err("corpus/Cargo.lock is not there: `cargo xtask corpus pin` makes it".to_owned());
    }

    let cargo_home = cargo_home()?;
    let timer = Instant::now();
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.arg("fetch").current_dir(work);
    if locked {
        command.arg("--locked");
    }
    // Where this run's cargo keeps its registry, said in one place and
    // made absolute: the command runs in another folder.
    command.env("CARGO_HOME", &cargo_home);
    let output = command
        .output()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo fetch{}` failed ({}):\n{}",
            if locked { " --locked" } else { "" },
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    println!("cargo fetch: {:.1} s", timer.elapsed().as_secs_f64());

    let lock = fs::read_to_string(work.join("Cargo.lock"))
        .map_err(|error| format!("cargo wrote no Cargo.lock: {error}"))?;
    let packages = registry_packages(&lock)?;
    let sources = registry_folders(&cargo_home)?;
    let timer = Instant::now();
    let made = work.join("corpus");
    let mut copied = 0usize;
    for (name, version) in &packages {
        let wanted = format!("{name}-{version}");
        let from = sources
            .iter()
            .map(|folder| folder.join(&wanted))
            .find(|path| path.is_dir())
            .ok_or_else(|| {
                format!(
                    "cargo has not left {wanted} in {}/registry/src",
                    cargo_home.display()
                )
            })?;
        copied += copy_rust_files(&from, &made.join(&wanted))?;
    }
    println!(
        "copied {copied} files of {} crates: {:.1} s",
        packages.len(),
        timer.elapsed().as_secs_f64()
    );
    let timer = Instant::now();
    let measured = shared::measure(&made)?;
    println!("hashed in {:.2} s", timer.elapsed().as_secs_f64());
    Ok((made, measured))
}

/// The manifest cargo fetches from: a package with no code and no
/// workspace, whose dependencies are the list.
fn manifest(dependencies: &str) -> String {
    format!(
        "[package]\nname = \"cortex-corpus\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\
         publish = false\n\n[lib]\npath = \"lib.rs\"\n\n[workspace]\n\n[dependencies]\n{dependencies}"
    )
}

/// The registry crates of a lock file's text: name and version of each
/// `[[package]]` that has a registry as its source. A package from a
/// repository is refused: the corpus is crates.io's, at checksums.
fn registry_packages(lock: &str) -> Result<Vec<(String, String)>> {
    let mut packages = Vec::new();
    let mut block: [Option<&str>; 3] = [None; 3];
    // The package whose lines were just read is finished by the next
    // `[[package]]`, or by the end of the text.
    let mut finish = |block: &mut [Option<&str>; 3]| -> Result<()> {
        if let [Some(name), Some(version), Some(source)] = *block {
            if !(source.starts_with("registry+") || source.starts_with("sparse+")) {
                return Err(format!(
                    "{name} {version} comes from {source}: the corpus is registry crates only"
                ));
            }
            packages.push((name.to_owned(), version.to_owned()));
        }
        *block = [None; 3];
        Ok(())
    };
    for line in lock.lines() {
        if line == "[[package]]" {
            finish(&mut block)?;
        }
        for (slot, key) in ["name", "version", "source"].into_iter().enumerate() {
            if let Some(value) = value_of(line, key) {
                block[slot] = Some(value);
            }
        }
    }
    finish(&mut block)?;
    Ok(packages)
}

/// `value` of a line `key = "value"`.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.strip_prefix(key)?
        .strip_prefix(" = \"")?
        .strip_suffix('"')
}

/// Where cargo keeps its files: `$CARGO_HOME`, or `.cargo` in the home
/// folder; an absolute path either way.
fn cargo_home() -> Result<PathBuf> {
    let home = std::env::var_os("CARGO_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(|home| PathBuf::from(home).join(".cargo"))
        })
        .ok_or("neither CARGO_HOME nor HOME is set: cannot tell where cargo keeps its registry")?;
    if home.is_absolute() {
        Ok(home)
    } else {
        std::env::current_dir()
            .map(|here| here.join(home))
            .map_err(|error| format!("cannot tell the current folder: {error}"))
    }
}

/// The folders under `registry/src`, one for each registry cargo has used,
/// in name order.
fn registry_folders(cargo_home: &Path) -> Result<Vec<PathBuf>> {
    let src = cargo_home.join("registry").join("src");
    let mut folders: Vec<PathBuf> = fs::read_dir(&src)
        .map_err(|error| format!("cannot read {}: {error}", src.display()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    Ok(folders)
}

/// Copies every `.rs` file under `from` to the same place under `to`, byte
/// for byte, dated [`FILE_DATE`]. Anything else, and a link, is left. Returns
/// how many.
fn copy_rust_files(from: &Path, to: &Path) -> Result<usize> {
    let date = SystemTime::UNIX_EPOCH + Duration::from_secs(FILE_DATE);
    let mut copied = 0;
    let entries =
        fs::read_dir(from).map_err(|error| format!("cannot read {}: {error}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read {}: {error}", from.display()))?;
        let kind = entry
            .file_type()
            .map_err(|error| format!("cannot look at {}: {error}", entry.path().display()))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copied += copy_rust_files(&entry.path(), &target)?;
        } else if kind.is_file() && entry.path().extension().is_some_and(|ext| ext == "rs") {
            let content = fs::read(entry.path())
                .map_err(|error| format!("cannot read {}: {error}", entry.path().display()))?;
            fs::create_dir_all(to)
                .map_err(|error| format!("cannot make {}: {error}", to.display()))?;
            let mut file = fs::File::create(&target)
                .map_err(|error| format!("cannot make {}: {error}", target.display()))?;
            file.write_all(&content)
                .and_then(|()| file.set_modified(date))
                .map_err(|error| format!("cannot write {}: {error}", target.display()))?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    use cortex_testkit::Scratch;

    use super::shared::{Measured, Pin};
    use super::{
        FILE_DATE, copy_rust_files, ensure, install, manifest, registry_packages, shared, value_of,
    };

    #[test]
    fn a_pin_that_is_not_one_is_an_error_that_says_how_to_mend_it() {
        let error = super::pinned("hash = not hexadecimal").unwrap_err();
        assert!(error.starts_with("corpus/PIN is not a pin: "), "{error}");
        assert!(error.contains("`cargo xtask corpus pin`"), "{error}");
        assert_eq!(
            super::pinned(shared::PIN_TEXT).unwrap(),
            Pin::parse(shared::PIN_TEXT).unwrap()
        );
    }

    /// A small corpus, `scratch/source`, and its pin.
    fn small(scratch: &Scratch) -> (PathBuf, Pin) {
        scratch.file("source/a-1.0.0/src/lib.rs", "pub fn a() {}\r\n");
        scratch.file("source/b-2.0.0/src/lib.rs", "pub fn b() {}\n");
        let source = scratch.join("source");
        let pin = shared::measure(&source).unwrap().as_pin();
        (source, pin)
    }

    fn copy_tree(from: &Path, to: &Path) {
        for relative in shared::list(from).unwrap() {
            let target = to.join(&relative);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::write(target, fs::read(from.join(&relative)).unwrap()).unwrap();
        }
    }

    /// What the folder holds, with the folders `ensure` works in shown.
    fn kept(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn the_manifest_is_a_package_of_no_code_with_the_list_as_its_dependencies() {
        let text = manifest("serde = \"1\"\n");
        assert!(text.contains("[workspace]\n"), "{text}");
        assert!(text.contains("[lib]\npath = \"lib.rs\""), "{text}");
        assert!(text.ends_with("[dependencies]\nserde = \"1\"\n"), "{text}");
        // It is TOML that says what it means.
        assert!(text.contains("publish = false"), "{text}");
    }

    #[test]
    fn the_lock_gives_each_registry_crate_and_refuses_one_from_elsewhere() {
        let lock = "\
# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = \"cortex-corpus\"
version = \"0.0.0\"
dependencies = [
 \"serde\",
]

[[package]]
name = \"serde\"
version = \"1.0.1\"
source = \"registry+https://github.com/rust-lang/crates.io-index\"
checksum = \"abc\"
dependencies = [
 \"serde_core\",
]

[[package]]
name = \"serde_core\"
version = \"1.0.2\"
source = \"sparse+https://index.crates.io/\"
checksum = \"def\"
";
        assert_eq!(
            registry_packages(lock),
            Ok(vec![
                ("serde".to_owned(), "1.0.1".to_owned()),
                ("serde_core".to_owned(), "1.0.2".to_owned())
            ])
        );
        let git = format!(
            "{lock}\n[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nsource = \"git+https://example.invalid/x\"\n"
        );
        assert!(
            registry_packages(&git)
                .unwrap_err()
                .contains("registry crates only")
        );
        assert_eq!(value_of("name = \"a-b\"", "name"), Some("a-b"));
        assert_eq!(value_of(" \"a-b\",", "name"), None);
    }

    #[test]
    fn only_rust_files_are_copied_byte_for_byte_and_dated() {
        let scratch = cortex_testkit::Scratch::outside("xtask-corpus", "copy");
        let crlf = b"fn a() {}\r\nfn b() {}\r\n";
        scratch.file("from/c-1.0.0/src/lib.rs", crlf);
        scratch.file("from/c-1.0.0/src/deep/x.rs", [0u8, 255, 10, 13]);
        scratch.file("from/c-1.0.0/Cargo.toml", "[package]\n");
        scratch.file("from/c-1.0.0/README.md", "# c\n");
        scratch.file("from/c-1.0.0/src/data.rsx", "not rust\n");
        let copied =
            copy_rust_files(&scratch.join("from/c-1.0.0"), &scratch.join("to/c-1.0.0")).unwrap();
        assert_eq!(copied, 2);
        assert_eq!(
            shared::list(&scratch.join("to")).unwrap(),
            ["c-1.0.0/src/deep/x.rs", "c-1.0.0/src/lib.rs"]
        );
        assert_eq!(scratch.read("to/c-1.0.0/src/lib.rs"), crlf);
        assert_eq!(scratch.read("to/c-1.0.0/src/deep/x.rs"), [0u8, 255, 10, 13]);
        let modified = fs::metadata(scratch.join("to/c-1.0.0/src/lib.rs"))
            .unwrap()
            .modified()
            .unwrap();
        assert_eq!(
            modified,
            SystemTime::UNIX_EPOCH + Duration::from_secs(FILE_DATE)
        );
    }

    #[test]
    fn a_corpus_that_is_there_and_whole_is_not_made_again() {
        let scratch = Scratch::outside("xtask-corpus", "there");
        let (source, pin) = small(&scratch);
        let root = scratch.dir("root");
        copy_tree(&source, &shared::folder(&root, &pin));
        let made = Cell::new(0);
        let maker = |_: &Path| {
            made.set(made.get() + 1);
            Err("it must not be made".to_owned())
        };
        let (folder, measured) = ensure(&root, &pin, &maker).unwrap();
        assert_eq!(folder, shared::folder(&root, &pin));
        assert_eq!(measured.as_pin(), pin);
        assert_eq!(made.get(), 0);
    }

    /// A maker that copies `source` into `<work>/corpus`, `changed` by what
    /// the test says, counting its calls.
    fn maker<'a>(
        source: &'a Path,
        made: &'a Cell<u32>,
        spoil: &'a dyn Fn(&Path),
    ) -> impl Fn(&Path) -> Result<(PathBuf, Measured), String> + 'a {
        move |work| {
            made.set(made.get() + 1);
            let corpus = work.join("corpus");
            copy_tree(source, &corpus);
            spoil(&corpus);
            Ok((corpus.clone(), shared::measure(&corpus).unwrap()))
        }
    }

    #[test]
    fn a_missing_corpus_is_made_checked_and_put_in_place_and_nothing_is_left() {
        let scratch = Scratch::outside("xtask-corpus", "missing");
        let (source, pin) = small(&scratch);
        let root = scratch.dir("root");
        let made = Cell::new(0);
        let (folder, measured) = ensure(&root, &pin, &maker(&source, &made, &|_| {})).unwrap();
        assert_eq!(made.get(), 1);
        assert_eq!(measured.as_pin(), pin);
        assert_eq!(shared::verify(&folder, &pin).unwrap().as_pin(), pin);
        // The folder it worked in is gone: only the corpus is there.
        assert_eq!(kept(&root), [pin.name()]);
    }

    #[test]
    fn a_corpus_that_is_not_whole_is_made_again() {
        let scratch = Scratch::outside("xtask-corpus", "not_whole");
        let (source, pin) = small(&scratch);
        let root = scratch.dir("root");
        let folder = shared::folder(&root, &pin);
        copy_tree(&source, &folder);
        fs::write(folder.join("b-2.0.0/src/lib.rs"), "pub fn z() {}\n").unwrap();
        let made = Cell::new(0);
        ensure(&root, &pin, &maker(&source, &made, &|_| {})).unwrap();
        assert_eq!(made.get(), 1);
        assert_eq!(shared::verify(&folder, &pin).unwrap().as_pin(), pin);
        assert_eq!(kept(&root), [pin.name()]);
    }

    #[test]
    fn a_corpus_that_is_not_the_pin_is_refused_and_nothing_is_put_in_place() {
        let scratch = Scratch::outside("xtask-corpus", "not_the_pin");
        let (source, pin) = small(&scratch);
        let root = scratch.dir("root");
        let made = Cell::new(0);
        // A download that went wrong: one byte of a file is another.
        let spoil = |corpus: &Path| {
            fs::write(corpus.join("a-1.0.0/src/lib.rs"), "pub fn x() {}\r\n").unwrap()
        };
        let problem = ensure(&root, &pin, &maker(&source, &made, &spoil)).unwrap_err();
        assert!(problem.contains("cargo xtask corpus pin"), "{problem}");
        assert!(problem.contains("nothing was put in place"), "{problem}");
        assert_eq!(kept(&root), Vec::<String>::new());
    }

    #[test]
    fn a_corpus_another_fetch_put_in_place_meanwhile_is_kept_if_it_is_whole() {
        let scratch = Scratch::outside("xtask-corpus", "meanwhile");
        let (source, pin) = small(&scratch);
        let root = scratch.dir("root");
        let folder = shared::folder(&root, &pin);
        copy_tree(&source, &scratch.join("made"));
        // Another fetch got there first, whole: this one's is not needed.
        copy_tree(&source, &folder);
        install(&scratch.join("made"), &folder, &pin).unwrap();
        // Or it is there and is not the pin: that is not kept.
        fs::write(folder.join("b-2.0.0/src/lib.rs"), "pub fn z() {}\n").unwrap();
        let problem = install(&scratch.join("made"), &folder, &pin).unwrap_err();
        assert!(
            problem.contains("cannot put the corpus in place"),
            "{problem}"
        );
        // And with nothing there, it goes there.
        fs::remove_dir_all(&folder).unwrap();
        install(&scratch.join("made"), &folder, &pin).unwrap();
        assert_eq!(shared::verify(&folder, &pin).unwrap().as_pin(), pin);
        assert!(!scratch.join("made").exists());
    }
}
