use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{
    Format, Library, Request, check_features, doc_calls, doc_commands, libraries, lock,
    prepare_base, report, resolve, select,
};
use crate::combine::{git, remove_tree};
use crate::workspace::{Package, Target};

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn a_request_is_read() {
    assert_eq!(
        Request::parse(&[]).unwrap(),
        Request {
            against: None,
            packages: Vec::new(),
            format: Format::Text,
            features: Vec::new(),
        }
    );
    assert_eq!(
        Request::parse(&strings(&[
            "--against",
            "v1",
            "-p",
            "a",
            "--package",
            "b",
            "--format",
            "markdown"
        ]))
        .unwrap(),
        Request {
            against: Some("v1".to_owned()),
            packages: strings(&["a", "b"]),
            format: Format::Markdown,
            features: Vec::new(),
        }
    );
    let joined = Request::parse(&strings(&["--against=origin/main", "--package=c"])).unwrap();
    assert_eq!(joined.against.as_deref(), Some("origin/main"));
    assert_eq!(joined.packages, ["c"]);
    for (words, expected) in [
        (&["--format", "html"][..], "it is `text` or `markdown`"),
        (&["--against"], "needs a value"),
        (&["-p", "--against", "x"], "not a value"),
        (&["--nope"], "unknown argument"),
    ] {
        let error = Request::parse(&strings(words)).unwrap_err();
        assert!(error.contains(expected), "{words:?}: {error}");
    }
}

#[test]
fn features_are_read_with_their_crate() {
    let pair = |package: &str, feature: &str| (package.to_owned(), feature.to_owned());
    let request = Request::parse(&strings(&[
        "-p",
        "cineris-tui",
        "--features",
        "test-kit",
        "--features=cineris-policy/test-support,cineris-tui/test-kit",
    ]))
    .unwrap();
    assert_eq!(
        request.features,
        [
            pair("cineris-tui", "test-kit"),
            pair("cineris-policy", "test-support")
        ]
    );
    // A crate named twice is one crate.
    let twice = Request::parse(&strings(&["-p", "x", "-p", "x", "--features", "f"])).unwrap();
    assert_eq!(twice.features, [pair("x", "f")]);
    for (words, expected) in [
        (&["--features", "test-kit"][..], "which crate's?"),
        (&["-p", "a", "-p", "b", "--features", "x"], "which crate's?"),
        (&["--features", "a/"], "a feature is `CRATE/FEATURE`"),
        (&["--features", "/x"], "a feature is `CRATE/FEATURE`"),
        (&["--features"], "needs a value"),
        (&["-p", "a", "--features", ","], "names no feature"),
        (&["--features="], "names no feature"),
    ] {
        let error = Request::parse(&strings(words)).unwrap_err();
        assert!(error.contains(expected), "{words:?}: {error}");
    }
}

#[test]
fn a_feature_is_turned_on_where_its_crate_has_it_and_said_where_it_does_not() {
    let tui = Library {
        package: "cineris-tui".to_owned(),
        json: "cineris_tui".to_owned(),
        features: strings(&["default", "test-kit"]),
    };
    let policy_before = Library {
        package: "cineris-policy".to_owned(),
        json: "cineris_policy".to_owned(),
        features: Vec::new(),
    };
    let asked = [
        ("cineris-tui".to_owned(), "test-kit".to_owned()),
        ("cineris-policy".to_owned(), "test-support".to_owned()),
        ("cineris-core".to_owned(), "x".to_owned()),
    ];
    let core = Library {
        package: "cineris-core".to_owned(),
        json: "cineris_core".to_owned(),
        features: strings(&["x"]),
    };
    let (calls, missing) = doc_calls(&[&core, &tui, &policy_before], &asked);
    // The crate with a feature on has a call of its own, its feature by its
    // bare name; the others are documented together, first.
    assert_eq!(
        calls,
        [
            (strings(&["cineris-policy"]), Vec::new()),
            (strings(&["cineris-core"]), strings(&["--features", "x"])),
            (
                strings(&["cineris-tui"]),
                strings(&["--features", "test-kit"])
            ),
        ]
    );
    assert_eq!(missing, ["cineris-policy/test-support"]);
    // Nothing asked of these crates: one call, no flag at all.
    assert_eq!(
        doc_calls(&[&policy_before], &asked[..1]),
        (vec![(strings(&["cineris-policy"]), Vec::new())], Vec::new())
    );
}

#[test]
fn each_call_of_cargo_doc_has_its_crates_and_its_features_and_no_other() {
    let lib = |package: &str, features: &[&str]| Library {
        package: package.to_owned(),
        json: package.replace('-', "_"),
        features: strings(features),
    };
    let tui = lib("cineris-tui", &["test-kit"]);
    let base = lib("cineris-base", &[]);
    let policy = lib("cineris-policy", &["test-support"]);
    let asked = [
        ("cineris-tui".to_owned(), "test-kit".to_owned()),
        ("cineris-policy".to_owned(), "test-support".to_owned()),
    ];
    let (commands, missing) = doc_commands(&[&base, &tui, &policy], &asked);
    assert!(missing.is_empty());
    let shown: Vec<(usize, String)> = commands
        .iter()
        .map(|(count, arguments)| (*count, arguments[7..].join(" ")))
        .collect();
    assert_eq!(
        shown,
        [
            (1, "-p cineris-base".to_owned()),
            (1, "-p cineris-tui --features test-kit".to_owned()),
            (1, "-p cineris-policy --features test-support".to_owned()),
        ]
    );
    assert!(
        commands
            .iter()
            .all(|(_, arguments)| arguments[..2] == ["doc", "--no-deps"])
    );
    // Nothing asked: one call of every crate, as before the step.
    let (commands, _) = doc_commands(&[&base, &tui], &[]);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].0, 2);
    assert_eq!(
        commands[0].1[7..].join(" "),
        "-p cineris-base -p cineris-tui"
    );
}

#[test]
fn a_feature_of_a_crate_not_compared_or_that_the_crate_lacks_is_refused() {
    let head: BTreeMap<String, Library> = [(
        "cineris-tui".to_owned(),
        Library {
            package: "cineris-tui".to_owned(),
            json: "cineris_tui".to_owned(),
            features: strings(&["test-kit"]),
        },
    )]
    .into_iter()
    .collect();
    let chosen = strings(&["cineris-tui"]);
    let feature = |package: &str, name: &str| vec![(package.to_owned(), name.to_owned())];
    assert_eq!(
        check_features(&feature("cineris-tui", "test-kit"), &chosen, &head),
        Ok(())
    );
    let error = check_features(&feature("cineris-tui", "nope"), &chosen, &head).unwrap_err();
    assert!(error.contains("has no such feature"), "{error}");
    let error = check_features(&feature("cineris-base", "x"), &chosen, &head).unwrap_err();
    assert!(
        error.contains("is not among the crates compared"),
        "{error}"
    );
}

fn package(name: &str, kinds: &[&str]) -> Package {
    Package {
        name: name.to_owned(),
        manifest: format!("crates/{name}/Cargo.toml"),
        publish_disabled: true,
        dependencies: Vec::new(),
        targets: vec![Target {
            name: name.to_owned(),
            kinds: strings(kinds),
            doctest: false,
        }],
        features: vec![("backends".to_owned(), Vec::new())],
    }
}

#[test]
fn a_library_is_a_package_with_a_lib_target_and_its_file_has_underscores() {
    let found = libraries(&[
        package("cineris-base", &["lib"]),
        package("cineris-macros", &["proc-macro"]),
        package("cineris-cli", &["bin"]),
    ]);
    let names: Vec<(&str, &str)> = found
        .values()
        .map(|library| (library.package.as_str(), library.json.as_str()))
        .collect();
    assert_eq!(
        names,
        [
            ("cineris-base", "cineris_base"),
            ("cineris-macros", "cineris_macros")
        ]
    );
    assert_eq!(found["cineris-base"].features, ["backends"]);
}

fn library(name: &str) -> (String, Library) {
    (
        name.to_owned(),
        Library {
            package: name.to_owned(),
            json: name.replace('-', "_"),
            features: Vec::new(),
        },
    )
}

#[test]
fn the_crates_are_those_named_or_every_one_with_a_library_in_either_tree() {
    let head: BTreeMap<String, Library> = ["a", "b"].map(library).into();
    let base: BTreeMap<String, Library> = ["b", "c"].map(library).into();
    let packages = vec![package("a", &["lib"]), package("tool", &["bin"])];
    assert_eq!(
        select(&[], &head, &base, &packages).unwrap(),
        ["a", "b", "c"]
    );
    // Named: sorted, once each, from either tree.
    assert_eq!(
        select(&strings(&["c", "a", "c"]), &head, &base, &packages).unwrap(),
        ["a", "c"]
    );
    let error = select(&strings(&["tool"]), &head, &base, &packages).unwrap_err();
    assert!(error.contains("`tool` has no library"), "{error}");
    let error = select(&strings(&["zzz"]), &head, &base, &packages).unwrap_err();
    assert!(error.contains("no crate named `zzz`"), "{error}");
}

#[test]
fn a_second_run_is_refused_while_the_first_holds_the_lock() {
    let dir = std::env::temp_dir().join(format!("xtask-api-diff-lock-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let first = lock(&dir, &mut |_| {}).unwrap();
    // The second is tried again for a while, on no clock: nothing waits.
    let mut waits = 0;
    let error = lock(&dir, &mut |_| waits += 1).unwrap_err();
    assert!(waits > 0, "a held lock is tried again before it is refused");
    assert!(
        error.contains("another `cargo xtask api-diff` is running"),
        "{error}"
    );
    drop(first);
    assert!(lock(&dir, &mut |_| {}).is_ok());
    let _ = fs::remove_dir_all(&dir);
}

/// A repository in a directory of its own, with a commit of `files`.
struct Repo {
    root: PathBuf,
}

fn write(root: &Path, file: &str, text: &str) {
    let path = root.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn commit(root: &Path, message: &str) -> String {
    git(root, &["add", "-A"]).unwrap();
    git(root, &["commit", "--quiet", "-m", message]).unwrap();
    git(root, &["rev-parse", "HEAD"]).unwrap()
}

impl Repo {
    fn new(name: &str, branch: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("xtask-api-diff-{name}-{}", std::process::id()));
        remove_tree(&root).unwrap();
        fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet", "--initial-branch", branch]).unwrap();
        write(&root, ".gitignore", "target/\n");
        Self { root }
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = remove_tree(&self.root);
    }
}

#[test]
#[cfg_attr(
    windows,
    ignore = "starts git about ten times, seconds on a Windows runner: see the nightly"
)]
fn sys_main_is_the_default_ref_and_falls_back_to_origin_main() {
    let repo = Repo::new("refs", "main");
    write(&repo.root, "a.txt", "one\n");
    let first = commit(&repo.root, "first");
    assert_eq!(resolve(&repo.root, None).unwrap(), first);
    assert_eq!(resolve(&repo.root, Some("HEAD")).unwrap(), first);
    let error = resolve(&repo.root, Some("nothing")).unwrap_err();
    assert!(error.contains("`nothing` is not a ref"), "{error}");

    // A checkout of a branch from the remote has `origin/main` and no `main`.
    let other = Repo::new("origin", "work");
    write(&other.root, "a.txt", "one\n");
    let head = commit(&other.root, "first");
    git(
        &other.root,
        &["update-ref", "refs/remotes/origin/main", &head],
    )
    .unwrap();
    assert_eq!(resolve(&other.root, None).unwrap(), head);
    let bare = Repo::new("bare", "work");
    write(&bare.root, "a.txt", "one\n");
    commit(&bare.root, "first");
    let error = resolve(&bare.root, None).unwrap_err();
    assert!(error.contains("no `main` or `origin/main`"), "{error}");
}

fn state(root: &Path) -> String {
    format!(
        "{}\n{}\n{}",
        git(root, &["for-each-ref"]).unwrap(),
        git(root, &["worktree", "list"]).unwrap(),
        git(root, &["status", "--porcelain"]).unwrap()
    )
}

#[test]
#[cfg_attr(
    windows,
    ignore = "starts git about ten times, seconds on a Windows runner: see the nightly"
)]
fn sys_the_other_tree_moves_between_commits_and_the_repository_is_left_alone() {
    let repo = Repo::new("base", "main");
    write(&repo.root, "a.txt", "one\n");
    let first = commit(&repo.root, "first");
    write(&repo.root, "a.txt", "two\n");
    write(&repo.root, "b.txt", "new\n");
    let second = commit(&repo.root, "second");
    // Uncommitted work is in the session's tree and not in the other one.
    write(&repo.root, "a.txt", "three\n");
    let before = state(&repo.root);

    let base = repo.root.join("target").join("api-diff").join("base");
    prepare_base(&repo.root, &base, &first).unwrap();
    assert_eq!(fs::read_to_string(base.join("a.txt")).unwrap(), "one\n");
    assert!(!base.join("b.txt").exists());
    // The next run moves the same clone.
    prepare_base(&repo.root, &base, &second).unwrap();
    assert_eq!(fs::read_to_string(base.join("a.txt")).unwrap(), "two\n");
    assert!(base.join("b.txt").exists());
    // A clone that no longer works is made again.
    fs::remove_file(base.join(".git").join("HEAD")).unwrap();
    prepare_base(&repo.root, &base, &first).unwrap();
    assert_eq!(fs::read_to_string(base.join("a.txt")).unwrap(), "one\n");

    assert_eq!(state(&repo.root), before, "the repository was written to");
    assert_eq!(
        fs::read_to_string(repo.root.join("a.txt")).unwrap(),
        "three\n"
    );
}

const MANIFEST: &str = "[package]\nname = \"NAME\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";

fn lock_file(names: &[&str]) -> String {
    let mut text = "# This file is automatically @generated by Cargo.\n\
                    # It is not intended for manual editing.\nversion = 4\n"
        .to_owned();
    for name in names {
        text.push_str(&format!(
            "\n[[package]]\nname = \"{name}\"\nversion = \"0.1.0\"\n"
        ));
    }
    text
}

/// `cargo xtask api-diff` against rustdoc, the real one: a workspace of one
/// crate at a commit, and in the working tree a changed signature, an added
/// field, a removed function, a new default method, a derive, and a crate
/// that was not there. It documents each tree, which takes a few seconds, so
/// it is ignored in the blocking checks; the nightly workflow runs it with
/// the other tests of this name (`sys_the_real`), and it is what tells that a
/// toolchain bump has moved rustdoc's JSON (`SUPPORTED_FORMATS`).
#[test]
#[ignore = "documents two crates with the real rustdoc: the nightly runs it"]
fn sys_the_real_rustdoc_reports_what_was_added_removed_and_changed() {
    let repo = Repo::new("real", "main");
    let root = &repo.root;
    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"small\"]\nresolver = \"3\"\n",
    );
    write(root, "Cargo.lock", &lock_file(&["small"]));
    write(root, "small/Cargo.toml", &MANIFEST.replace("NAME", "small"));
    write(
        root,
        "small/src/lib.rs",
        "pub struct Config { pub name: String }\n\
         pub fn load(path: &str) -> Config { Config { name: path.to_owned() } }\n\
         pub fn gone() {}\n\
         pub trait Source { fn read(&self) -> u8; }\n",
    );
    commit(root, "the crate");

    write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"small\", \"fresh\"]\nresolver = \"3\"\n",
    );
    write(root, "Cargo.lock", &lock_file(&["fresh", "small"]));
    write(root, "fresh/Cargo.toml", &MANIFEST.replace("NAME", "fresh"));
    write(root, "fresh/src/lib.rs", "pub fn hello() {}\n");
    write(
        root,
        "small/src/lib.rs",
        "#[derive(Clone)]\npub struct Config { pub name: String, pub retries: u32 }\n\
         pub fn load(path: &str, strict: bool) -> Config { Config { name: path.to_owned(), retries: strict.into() } }\n\
         pub trait Source { fn read(&self) -> u8; fn name(&self) -> String { String::new() } }\n",
    );

    let request = Request {
        against: Some("HEAD".to_owned()),
        packages: Vec::new(),
        format: Format::Text,
        features: Vec::new(),
    };
    let text = report(root, &request).unwrap();
    let body: Vec<&str> = text.lines().skip(1).collect();
    assert_eq!(
        body,
        [
            "fresh: 1 added, 0 removed, 0 changed (a crate the ref does not have)",
            "  + fresh::hello  fn hello()",
            "small: 3 added, 1 removed, 1 changed",
            "  + impl Clone for small::Config  impl Clone for Config",
            "  + small::Config::retries  retries: u32",
            "  + small::Source::name  fn name(&self) -> String { .. }",
            "  - small::gone  fn gone()",
            "  ~ small::load",
            "      was  fn load(path: &str) -> Config",
            "      now  fn load(path: &str, strict: bool) -> Config",
        ],
        "{text}"
    );

    // The second run builds nothing again: the same report.
    assert_eq!(report(root, &request).unwrap(), text);
}
