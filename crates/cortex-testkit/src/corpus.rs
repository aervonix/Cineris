//! The corpus of real code that a `perf_` test measures: about 20,000 `.rs`
//! files of crates from crates.io, at the versions and under the hash that
//! the repository's `corpus/` pins, so that a number taken at a desk and a
//! number taken on a hosted runner are of the same bytes
//! (`docs/testing-and-ci.md`, "A tree of real code to measure").
//!
//! ```text
//! let real_code = cortex_testkit::corpus::real_code();   // a path, verified
//! ```
//!
//! [`real_code`] checks the corpus **every time it is asked for**: it reads
//! every file and compares the count, the size and the hash with the pin
//! (about 0.4 s of a warm disk). It fails with the command to run when the
//! corpus is not there or is not whole. **Nothing blocking asks for it**: a
//! `perf_` test is ignored in the blocking checks, and the corpus is fetched
//! by `cargo xtask corpus fetch`, which `try`'s `perf -p CRATE` runs first.
//!
//! The pin, the hash and the places the corpus is kept are in [`shared`],
//! which `cargo xtask corpus` includes as well.

use std::path::{Path, PathBuf};

#[path = "corpus/shared.rs"]
pub mod shared;

pub use shared::{Measured, Pin, ROOT_VARIABLE};

/// Where the pinned corpus is, if it is there and whole.
///
/// # Errors
///
/// Why it is not: not there, or what differs from the pin, with the command
/// that fetches it.
pub fn try_real_code() -> Result<PathBuf, String> {
    try_real_code_in(&shared::system_root()?, &Pin::pinned())
}

/// [`try_real_code`] for a corpus kept under `root`, against `pin`.
///
/// # Errors
///
/// As [`try_real_code`].
pub fn try_real_code_in(root: &Path, pin: &Pin) -> Result<PathBuf, String> {
    let folder = shared::folder(root, pin);
    shared::verify(&folder, pin)
        .map(|_| folder)
        .map_err(|problem| {
            format!(
                "the corpus of real code is not usable: {problem}\n\
                 Fetch it with `cargo xtask corpus fetch` (on the desk, `rx -- cargo xtask corpus \
                 fetch`); it is kept outside the repository, so one fetch serves every worktree."
            )
        })
}

/// The pinned corpus of real code: the folder, verified against the pin.
///
/// # Panics
///
/// When the corpus is not there or is not whole; the message says why and
/// which command fetches it.
#[must_use]
pub fn real_code() -> PathBuf {
    try_real_code().unwrap_or_else(|problem| panic!("{problem}"))
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "tests that prepare and spoil folders of real files"
)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::shared::{self, Fnv, Pin};
    use crate::scratch::Scratch;

    fn corpus(name: &str) -> (Scratch, PathBuf) {
        let scratch = Scratch::outside("cortex-testkit-corpus", name);
        scratch.file("corpus/a-1.0.0/src/lib.rs", "pub fn a() {}\n");
        scratch.file("corpus/a-1.0.0/src/deep/mod.rs", "mod x;\r\nmod y;\r\n");
        scratch.file("corpus/b-2.0.0/src/lib.rs", "pub fn b() {}\n");
        let dir = scratch.join("corpus");
        (scratch, dir)
    }

    fn pin_of(dir: &Path) -> Pin {
        shared::measure(dir).unwrap().as_pin()
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a Path)]) -> impl Fn(&str) -> Option<OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_os_str().to_owned())
        }
    }

    #[test]
    fn the_repositorys_pin_is_a_pin() {
        let pin = Pin::pinned();
        assert!(pin.files > 0 && pin.bytes > 0, "{pin:?}");
        assert_eq!(Pin::parse(&pin.text()), Ok(pin));
    }

    #[test]
    fn a_pin_is_read_from_its_text_and_a_bad_one_is_refused() {
        let text = "# a comment\n\nhash = 00000000000000ff\nfiles = 3\nbytes = 40\n";
        assert_eq!(
            Pin::parse(text),
            Ok(Pin {
                hash: 255,
                files: 3,
                bytes: 40
            })
        );
        for (text, says) in [
            ("files = 3\nbytes = 4\n", "no `hash`"),
            ("hash = 00000000000000ff\nbytes = 4\n", "no `files`"),
            ("hash = 00000000000000ff\nfiles = 3\n", "no `bytes`"),
            (
                "hash = 00000000000000ff\nhash = 00000000000000ff\nfiles = 3\nbytes = 4\n",
                "given twice",
            ),
            ("hash = ff\nfiles = 3\nbytes = 4\n", "sixteen"),
            (
                "hash = 00000000000000zz\nfiles = 3\nbytes = 4\n",
                "hexadecimal",
            ),
            (
                "hash = 00000000000000ff\nfiles = x\nbytes = 4\n",
                "a number",
            ),
            (
                "hash = 00000000000000ff\nfiles = 3\nbytes = 4\nmore = 1\n",
                "not a key",
            ),
            ("hash 00000000000000ff\n", "key = value"),
        ] {
            let problem = Pin::parse(text).unwrap_err();
            assert!(problem.contains(says), "{text:?}: {problem}");
        }
    }

    #[test]
    fn the_hash_is_fnv_1a_of_the_paths_lengths_and_contents_in_path_order() {
        // FNV-1a's published test vectors.
        let mut hash = Fnv::new();
        assert_eq!(hash.0, 0xcbf2_9ce4_8422_2325);
        hash.write(b"a");
        assert_eq!(hash.0, 0xaf63_dc4c_8601_ec8c);
        let mut hash = Fnv::new();
        hash.write(b"foobar");
        assert_eq!(hash.0, 0x8594_4171_f739_67e8);

        // The corpus's own: path, a zero, the length, the content, file after
        // file in the order of the paths, whatever order they were made in.
        let (_scratch, dir) = corpus("the_hash_is_fnv");
        let mut expected = Fnv::new();
        for (path, content) in [
            ("a-1.0.0/src/deep/mod.rs", &b"mod x;\r\nmod y;\r\n"[..]),
            ("a-1.0.0/src/lib.rs", b"pub fn a() {}\n"),
            ("b-2.0.0/src/lib.rs", b"pub fn b() {}\n"),
        ] {
            expected.write(path.as_bytes());
            expected.write(&[0]);
            expected.write(&(content.len() as u64).to_le_bytes());
            expected.write(content);
        }
        let measured = shared::measure(&dir).unwrap();
        assert_eq!(measured.hash, expected.0);
        assert_eq!((measured.files, measured.bytes), (3, 16 + 14 + 14));
        assert_eq!(
            shared::list(&dir).unwrap(),
            [
                "a-1.0.0/src/deep/mod.rs",
                "a-1.0.0/src/lib.rs",
                "b-2.0.0/src/lib.rs"
            ]
        );
    }

    #[test]
    fn the_listing_is_in_the_byte_order_of_the_paths_whatever_the_folder_says() {
        let scratch = Scratch::outside("cortex-testkit-corpus", "order");
        // Made in the reverse of their order, a folder at a time, so that a
        // listing in the order the system gives them is not by chance the
        // sorted one.
        let mut expected = Vec::new();
        for n in (0..30).rev() {
            for name in ["Z", "b", "a"] {
                let relative = format!("d{n:02}/{name}.rs");
                scratch.file(&relative, name);
                expected.push(relative);
            }
        }
        expected.sort();
        assert_eq!(shared::list(scratch.path()).unwrap(), expected);
        assert_eq!(
            expected[0], "d00/Z.rs",
            "capitals come before small letters"
        );
    }

    #[test]
    fn the_corpus_that_is_the_pin_is_accepted() {
        let (_scratch, dir) = corpus("accepted");
        let pin = pin_of(&dir);
        assert_eq!(shared::verify(&dir, &pin).unwrap().as_pin(), pin);
    }

    #[test]
    fn a_changed_byte_is_refused() {
        let (scratch, dir) = corpus("changed_byte");
        let pin = pin_of(&dir);
        // The same length: only the hash can tell.
        scratch.file("corpus/a-1.0.0/src/lib.rs", "pub fn z() {}\n");
        let problem = shared::verify(&dir, &pin).unwrap_err();
        assert!(problem.contains("hashes to"), "{problem}");
        assert!(problem.contains("a file has changed"), "{problem}");
    }

    #[test]
    fn a_line_ending_that_changed_is_refused() {
        // What a checkout with `autocrlf` would do to a file.
        let (scratch, dir) = corpus("line_ending");
        let pin = pin_of(&dir);
        scratch.file("corpus/a-1.0.0/src/deep/mod.rs", "mod x;\nmod y;\n");
        let problem = shared::verify(&dir, &pin).unwrap_err();
        assert!(problem.contains("bytes"), "{problem}");
    }

    #[test]
    fn a_missing_file_is_refused() {
        let (_scratch, dir) = corpus("missing");
        let pin = pin_of(&dir);
        fs::remove_file(dir.join("b-2.0.0/src/lib.rs")).unwrap();
        let problem = shared::verify(&dir, &pin).unwrap_err();
        assert!(
            problem.contains("holds 2 files, and the pin says 3"),
            "{problem}"
        );
    }

    #[test]
    fn an_extra_file_is_refused_even_when_empty() {
        let (scratch, dir) = corpus("extra");
        let pin = pin_of(&dir);
        scratch.file("corpus/b-2.0.0/src/extra.rs", "");
        let problem = shared::verify(&dir, &pin).unwrap_err();
        assert!(
            problem.contains("holds 4 files, and the pin says 3"),
            "{problem}"
        );
    }

    #[test]
    fn a_file_under_another_name_is_refused() {
        let (_scratch, dir) = corpus("renamed");
        let pin = pin_of(&dir);
        fs::rename(
            dir.join("b-2.0.0/src/lib.rs"),
            dir.join("b-2.0.0/src/main.rs"),
        )
        .unwrap();
        let problem = shared::verify(&dir, &pin).unwrap_err();
        assert!(problem.contains("another name"), "{problem}");
    }

    #[test]
    fn a_folder_that_is_not_there_is_refused() {
        let (scratch, _dir) = corpus("not_there");
        let problem = shared::verify(&scratch.join("nothing"), &Pin::pinned()).unwrap_err();
        assert!(problem.contains("is not there"), "{problem}");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_not_followed() {
        let (_scratch, dir) = corpus("link");
        std::os::unix::fs::symlink(dir.join("a-1.0.0/src/lib.rs"), dir.join("c.rs")).unwrap();
        let problem = shared::measure(&dir).unwrap_err();
        assert!(
            problem.contains("neither a folder nor a regular file"),
            "{problem}"
        );
    }

    #[test]
    fn the_corpus_is_kept_where_the_system_keeps_caches() {
        let scratch = Scratch::outside("cortex-testkit-corpus", "paths");
        let (home, xdg, local, other) = (
            scratch.join("home"),
            scratch.join("xdg"),
            scratch.join("local"),
            scratch.join("other"),
        );
        let kept = |parent: &Path| parent.join("cortex-harness").join("corpus");
        // Linux: the cache folder, or `.cache` in the home folder.
        assert_eq!(
            shared::cache_root(&env(&[("HOME", &home), ("XDG_CACHE_HOME", &xdg)]), false),
            Ok(kept(&xdg))
        );
        assert_eq!(
            shared::cache_root(&env(&[("HOME", &home)]), false),
            Ok(kept(&home.join(".cache")))
        );
        // A cache folder that is not absolute is not used.
        assert_eq!(
            shared::cache_root(
                &|name| match name {
                    "HOME" => Some(home.clone().into_os_string()),
                    "XDG_CACHE_HOME" => Some(OsString::from("relative")),
                    _ => None,
                },
                false
            ),
            Ok(kept(&home.join(".cache")))
        );
        // Windows: the local application data folder, and not the home.
        assert_eq!(
            shared::cache_root(&env(&[("LOCALAPPDATA", &local), ("HOME", &home)]), true),
            Ok(kept(&local))
        );
        // The variable wins over both, whatever the system.
        for windows in [false, true] {
            assert_eq!(
                shared::cache_root(
                    &env(&[
                        ("CORTEX_CORPUS_DIR", &other),
                        ("HOME", &home),
                        ("LOCALAPPDATA", &local)
                    ]),
                    windows
                ),
                Ok(other.clone())
            );
        }
        // Nowhere to keep it is said, with the way out.
        assert!(
            shared::cache_root(&env(&[]), false)
                .unwrap_err()
                .contains("CORTEX_CORPUS_DIR")
        );
        assert!(
            shared::cache_root(&env(&[("HOME", &home)]), true)
                .unwrap_err()
                .contains("LOCALAPPDATA")
        );
        // An empty variable is not set, and a relative one is refused.
        assert!(
            shared::cache_root(
                &|name| (name == "CORTEX_CORPUS_DIR").then(OsString::new),
                true
            )
            .unwrap_err()
            .contains("LOCALAPPDATA")
        );
        assert!(
            shared::cache_root(
                &|name| (name == "CORTEX_CORPUS_DIR").then(|| OsString::from("relative/dir")),
                false
            )
            .unwrap_err()
            .contains("absolute")
        );
    }

    #[test]
    fn the_folder_of_a_corpus_is_named_by_its_hash() {
        let pin = Pin {
            hash: 0xab,
            files: 1,
            bytes: 1,
        };
        let root = std::env::temp_dir();
        assert_eq!(shared::folder(&root, &pin), root.join("00000000000000ab"));
    }

    #[test]
    fn a_corpus_that_is_not_fetched_or_not_whole_fails_with_the_command_that_fetches_it() {
        let (scratch, dir) = corpus("not_fetched");
        let pin = pin_of(&dir);
        // The folder of the pin, under a root of its own.
        let root = scratch.join("root");
        let problem = super::try_real_code_in(&root, &pin).unwrap_err();
        assert!(problem.contains("is not there"), "{problem}");
        assert!(problem.contains("cargo xtask corpus fetch"), "{problem}");
        let folder = shared::folder(&root, &pin);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("x.rs"), "fn x() {}\n").unwrap();
        let problem = super::try_real_code_in(&root, &pin).unwrap_err();
        assert!(problem.contains("cargo xtask corpus fetch"), "{problem}");
        // Whole: it is the folder, and it is the answer.
        fs::remove_dir_all(&folder).unwrap();
        fs::create_dir_all(folder.parent().unwrap()).unwrap();
        for relative in shared::list(&dir).unwrap() {
            let to = folder.join(&relative);
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::write(to, fs::read(dir.join(&relative)).unwrap()).unwrap();
        }
        assert_eq!(super::try_real_code_in(&root, &pin), Ok(folder));
    }
}
