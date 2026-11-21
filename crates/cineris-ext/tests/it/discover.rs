//! Extensions are the folders directly under the user's root and the
//! project's, each read whole and named by its own manifest; a root is
//! judged where it leads, a project's only inside the workspace, and one
//! that is not there costs a look and gives nothing.
//!
//! Each test's scratch folder holds `home/`, the home directory `~` stands
//! for, `home/proj/`, the workspace, `home/cfg/`, the configuration
//! directory (so the user's root is `home/cfg/extensions`), and `outside/`,
//! which is none of them.

use std::path::Path;

use cineris_base::{AbsPath, PathOptions, PathResolver};
use cineris_ext::discover::{self, Discovered, MAX_ENTRIES, MAX_PER_ROOT, Root};
use cineris_ext::folder::Snapshot;
use cineris_ext::source::Source;
use cineris_testkit::scratch;
use cineris_testkit::scratch::Scratch;

use crate::make_link;

/// The session's resolver: the workspace `home/proj`, and `~` for `home`.
fn resolver(scratch: &Scratch) -> PathResolver {
    let workspace = scratch.dir("home/proj");
    let home = AbsPath::from_os(&scratch.join("home")).unwrap();
    PathResolver::new(&workspace, PathOptions::new().home(home)).unwrap()
}

/// An extension in `folder` whose manifest gives `name`.
fn extension(scratch: &Scratch, folder: &str, name: &str) {
    scratch.file(
        format!("{folder}/extension.toml"),
        format!("api = 1\nname = \"{name}\"\nversion = \"0.1.0\"\nmain = \"main.js\"\n"),
    );
    scratch.file(format!("{folder}/main.js"), format!("// {folder}\n"));
}

/// The user's root, `home/cfg/extensions`.
fn user(scratch: &Scratch) -> Root {
    Root::user(&scratch.join("home/cfg"))
}

/// The project's root, `home/proj/.cineris/extensions`.
fn project(scratch: &Scratch) -> Root {
    Root::project(&scratch.join("home/proj"))
}

/// Removes a link to a folder: a file on Linux, a folder on Windows.
fn remove_link(link: &Path) {
    if std::fs::remove_file(link).is_err() {
        std::fs::remove_dir(link).unwrap();
    }
}

fn discover(scratch: &Scratch, roots: &[Root]) -> Discovered {
    discover::discover(roots, &resolver(scratch))
}

/// Each extension found, as its source, its name and how it is shown.
fn found(discovered: &Discovered) -> Vec<(Source, &str, &str)> {
    discovered
        .found
        .iter()
        .map(|found| {
            (
                found.source,
                found.manifest.name.as_str(),
                found.shown.as_str(),
            )
        })
        .collect()
}

/// What was left out, as its source, how it is shown and why.
fn left(discovered: &Discovered) -> Vec<(Source, &str, &str)> {
    discovered
        .left_out
        .iter()
        .map(|left| (left.source, left.shown.as_str(), left.reason.as_str()))
        .collect()
}

#[test]
fn extensions_are_the_folders_directly_under_each_root_by_name() {
    let scratch = scratch!("extensions_are_the_folders_directly_under_each_root_by_name");
    extension(&scratch, "home/cfg/extensions/zeta", "zeta");
    extension(&scratch, "home/cfg/extensions/alpha", "alpha");
    extension(&scratch, "home/proj/.cineris/extensions/mid", "mid");
    // Not extensions: a file beside the folders, a folder whose name
    // begins with `.`, and a manifest deeper than one folder.
    scratch.file("home/cfg/extensions/README.md", "my extensions");
    extension(&scratch, "home/cfg/extensions/.git", "git");
    extension(&scratch, "home/cfg/extensions/alpha/vendored", "vendored");

    let paths = resolver(&scratch);
    let discovered = discover::discover(&[user(&scratch), project(&scratch)], &paths);
    assert_eq!(
        found(&discovered),
        [
            (Source::User, "alpha", "~/cfg/extensions/alpha"),
            (Source::User, "zeta", "~/cfg/extensions/zeta"),
            (Source::Project, "mid", ".cineris/extensions/mid"),
        ]
    );
    assert_eq!(left(&discovered), []);
    assert_eq!(discovered.resolved, 2);
    let alpha = &discovered.found[0];
    assert_eq!(
        alpha.folder,
        paths
            .resolve("~/cfg/extensions/alpha")
            .unwrap()
            .to_fs_path()
    );
    // The snapshot is the folder read whole, nested files and all.
    assert_eq!(
        alpha.snapshot.hash(),
        Snapshot::read(&alpha.folder).unwrap().hash()
    );
    assert!(alpha.snapshot.file("vendored/main.js").is_some());
    assert_eq!(alpha.manifest.main, "main.js");
}

#[test]
fn an_extension_whose_manifest_does_not_give_its_folders_name_is_left_out() {
    let scratch =
        scratch!("an_extension_whose_manifest_does_not_give_its_folders_name_is_left_out");
    extension(&scratch, "home/cfg/extensions/fine", "fine");
    extension(&scratch, "home/cfg/extensions/tracker", "trackr");
    // A name the file system would call the same on Windows is still
    // another name.
    extension(&scratch, "home/cfg/extensions/Upper", "upper");
    scratch.file("home/cfg/extensions/empty/main.js", "1");
    scratch.file(
        "home/cfg/extensions/newer/extension.toml",
        "api = 2\nname = \"newer\"\nversion = \"1\"\nmain = \"main.js\"\n",
    );
    scratch.file("home/cfg/extensions/newer/main.js", "1");
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert_eq!(
        found(&discovered),
        [(Source::User, "fine", "~/cfg/extensions/fine")]
    );
    assert_eq!(
        left(&discovered),
        [
            (
                Source::User,
                "~/cfg/extensions/Upper",
                "the extension folder `~/cfg/extensions/Upper` was left out: its manifest gives \
                 the name `upper`, and an extension's manifest (`extension.toml`) gives its \
                 folder's own name, `Upper`"
            ),
            (
                Source::User,
                "~/cfg/extensions/empty",
                "the extension folder `~/cfg/extensions/empty` was left out: the manifest of the \
                 extension is refused: the folder has no `extension.toml`"
            ),
            (
                Source::User,
                "~/cfg/extensions/newer",
                "the extension folder `~/cfg/extensions/newer` was left out: the manifest of the \
                 extension is refused: `api = 2` is not one this build of Cineris knows (it knows \
                 1): it needs another version of Cineris"
            ),
            (
                Source::User,
                "~/cfg/extensions/tracker",
                "the extension folder `~/cfg/extensions/tracker` was left out: its manifest gives \
                 the name `trackr`, and an extension's manifest (`extension.toml`) gives its \
                 folder's own name, `tracker`"
            ),
        ]
    );
}

#[test]
fn a_root_that_is_not_there_gives_nothing_and_is_not_resolved() {
    let scratch = scratch!("a_root_that_is_not_there_gives_nothing_and_is_not_resolved");
    let roots = [user(&scratch), project(&scratch)];
    // Neither the configuration directory nor `.cineris` is there.
    assert_eq!(discover(&scratch, &roots), Discovered::default());
    // Both are, with no `extensions` in them: still nothing resolved.
    scratch.file("home/cfg/config.toml", "");
    scratch.file("home/proj/.cineris/config.toml", "");
    assert_eq!(discover(&scratch, &roots), Discovered::default());
    // A root given with `~` when no home directory is known: nothing.
    let workspace = scratch.join("home/proj");
    let homeless = PathResolver::new(&workspace, PathOptions::new()).unwrap();
    scratch.dir("home/cfg/extensions");
    let tilde = Root::new(Source::User, "~/cfg/extensions");
    let discovered = discover::discover(&[tilde], &homeless);
    assert_eq!((discovered.found, discovered.left_out), (vec![], vec![]));
    // Roots that are there and empty: looked in, and nothing in them.
    scratch.dir("home/proj/.cineris/extensions");
    let discovered = discover(&scratch, &roots);
    assert_eq!(
        (found(&discovered), left(&discovered), discovered.resolved),
        (vec![], vec![], 2)
    );
}

#[test]
fn roots_written_with_a_tilde_a_relative_path_and_dots_lead_to_the_same_extensions() {
    let scratch =
        scratch!("roots_written_with_a_tilde_a_relative_path_and_dots_lead_to_the_same_extensions");
    extension(&scratch, "home/cfg/extensions/one", "one");
    extension(&scratch, "home/proj/.cineris/extensions/two", "two");
    extension(&scratch, "home/elsewhere/extensions/three", "three");
    scratch.dir("home/proj/sub");
    let paths = resolver(&scratch);
    let roots = [
        Root::new(Source::User, "~/cfg/./extensions"),
        // Into a folder that is there, and back out of it.
        Root::new(Source::Project, "sub/../.cineris/extensions"),
    ];
    let discovered = discover::discover(&roots, &paths);
    assert_eq!(
        found(&discovered),
        [
            (Source::User, "one", "~/cfg/extensions/one"),
            (Source::Project, "two", ".cineris/extensions/two"),
        ]
    );
    assert_eq!(left(&discovered), []);
    // The same places written other ways. A project's root given with `~`
    // that leads into the workspace is the workspace's; the workspace's
    // real location is the form the core may give.
    let roots = [
        Root::new(Source::User, "../cfg/extensions"),
        Root::new(Source::Project, "~/proj/.cineris/extensions"),
        Root::project(&paths.root().to_fs_path()),
        Root::new(Source::Project, "../proj/.cineris/extensions/"),
        // Out of the workspace by `..`.
        Root::new(Source::Project, "../elsewhere/extensions"),
    ];
    let discovered = discover::discover(&roots, &paths);
    assert_eq!(
        found(&discovered),
        [
            (Source::User, "one", "~/cfg/extensions/one"),
            (Source::Project, "two", ".cineris/extensions/two"),
        ]
    );
    let same = "it is the same folder as `.cineris/extensions`, which was looked in instead";
    // A root given absolute is shown as the user would read it: the
    // project's relative to the workspace (the full review of step 28).
    let projects = ".cineris/extensions".to_owned();
    assert_eq!(
        left(&discovered),
        [
            (
                Source::Project,
                projects.as_str(),
                format!(
                    "the project's folder of extensions `{projects}` was not looked in: {same}"
                )
                .as_str()
            ),
            (
                Source::Project,
                "../proj/.cineris/extensions/",
                format!(
                    "the project's folder of extensions `../proj/.cineris/extensions/` was not \
                     looked in: {same}"
                )
                .as_str()
            ),
            (
                Source::Project,
                "../elsewhere/extensions",
                "the project's folder of extensions `../elsewhere/extensions` was not looked in: \
                 it leads out of the workspace, and a project's extensions are read only from \
                 inside it"
            ),
        ]
    );
    // The workspace itself, given as its real location, is a root like any
    // other: its folders are looked at (`.cineris` is passed over).
    let discovered = discover::discover(
        &[Root::new(Source::Project, paths.root().to_fs_path())],
        &paths,
    );
    assert_eq!(found(&discovered), []);
    assert_eq!(
        left(&discovered),
        [(
            Source::Project,
            "./sub",
            "the extension folder `./sub` was left out: the manifest of the extension is \
             refused: the folder has no `extension.toml`"
        )]
    );
}

#[test]
fn a_users_root_that_is_a_link_is_followed_wherever_it_leads() {
    let scratch = scratch!("a_users_root_that_is_a_link_is_followed_wherever_it_leads");
    extension(&scratch, "outside/dotfiles/extensions/mine", "mine");
    extension(&scratch, "home/dots/ext/kept", "kept");
    scratch.dir("home/cfg");
    scratch.dir("home/other");
    let links = [
        (
            scratch.join("outside/dotfiles/extensions"),
            scratch.join("home/cfg/extensions"),
        ),
        (
            scratch.join("home/dots/ext"),
            scratch.join("home/other/extensions"),
        ),
    ];
    for (target, link) in &links {
        if !make_link(target, link, true) {
            return;
        }
    }
    let roots = [user(&scratch), Root::user(&scratch.join("home/other"))];
    let discovered = discover(&scratch, &roots);
    let names = found(&discovered);
    assert_eq!(names.len(), 2, "{discovered:?}");
    // Shown where it leads: under `~` when that is in the home directory.
    assert_eq!(names[0].1, "mine");
    assert!(
        names[0].2.ends_with("outside/dotfiles/extensions/mine") && !names[0].2.starts_with('~'),
        "{}",
        names[0].2
    );
    assert_eq!(names[1], (Source::User, "kept", "~/dots/ext/kept"));
    assert_eq!(left(&discovered), []);
}

#[test]
fn a_projects_root_that_leads_out_of_the_workspace_gives_nothing_and_says_why() {
    let scratch =
        scratch!("a_projects_root_that_leads_out_of_the_workspace_gives_nothing_and_says_why");
    extension(&scratch, "outside/exts/stolen", "stolen");
    extension(&scratch, "outside/dotcineris/extensions/stolen", "stolen");
    scratch.file(
        "outside/exts/stolen/prompt.md",
        "SECRET-OUTSIDE: never read",
    );
    extension(&scratch, "home/proj/tools/exts/inner", "inner");
    scratch.dir("home/proj/.cineris");
    let workspace = scratch.join("home/proj");
    let written = ".cineris/extensions".to_owned();
    let outside = format!(
        "the project's folder of extensions `{written}` was not looked in: it leads out of the \
         workspace, and a project's extensions are read only from inside it"
    );

    // `.cineris/extensions` a link out of the workspace.
    if !make_link(
        &scratch.join("outside/exts"),
        &workspace.join(".cineris/extensions"),
        true,
    ) {
        return;
    }
    let discovered = discover(&scratch, &[project(&scratch)]);
    assert_eq!(found(&discovered), []);
    assert_eq!(
        left(&discovered),
        [(Source::Project, written.as_str(), outside.as_str())]
    );
    assert!(!format!("{discovered:?}").contains("SECRET-OUTSIDE"));

    // `.cineris` itself a link out of the workspace.
    remove_link(&workspace.join(".cineris/extensions"));
    std::fs::remove_dir(workspace.join(".cineris")).unwrap();
    if !make_link(
        &scratch.join("outside/dotcineris"),
        &workspace.join(".cineris"),
        true,
    ) {
        return;
    }
    let discovered = discover(&scratch, &[project(&scratch)]);
    assert_eq!(found(&discovered), []);
    assert_eq!(
        left(&discovered),
        [(Source::Project, written.as_str(), outside.as_str())]
    );
    // A `.cineris` that leads out with no `extensions` there gives nothing,
    // and the look before the resolver did not follow the link: the
    // resolver was asked.
    std::fs::remove_dir_all(scratch.join("outside/dotcineris/extensions")).unwrap();
    let discovered = discover(&scratch, &[project(&scratch)]);
    assert_eq!(
        (found(&discovered), left(&discovered), discovered.resolved),
        (vec![], vec![], 1)
    );

    // `.cineris/extensions` a link to another place in the workspace:
    // followed, and shown where it leads.
    remove_link(&workspace.join(".cineris"));
    scratch.dir("home/proj/.cineris");
    if !make_link(
        &workspace.join("tools/exts"),
        &workspace.join(".cineris/extensions"),
        true,
    ) {
        return;
    }
    let discovered = discover(&scratch, &[project(&scratch)]);
    assert_eq!(
        found(&discovered),
        [(Source::Project, "inner", "tools/exts/inner")]
    );
    assert_eq!(left(&discovered), []);
}

#[test]
fn an_extension_folder_reached_through_a_link_is_refused() {
    let scratch = scratch!("an_extension_folder_reached_through_a_link_is_refused");
    extension(&scratch, "outside/alias", "alias");
    scratch.file("outside/alias/prompt.md", "SECRET-OUTSIDE");
    extension(&scratch, "home/cfg/extensions/fine", "fine");
    let root = scratch.join("home/cfg/extensions");
    // A link to a folder outside, and one to a folder inside the root.
    for (target, link) in [
        (scratch.join("outside/alias"), root.join("alias")),
        (root.join("fine"), root.join("again")),
    ] {
        if !make_link(&target, &link, true) {
            return;
        }
    }
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert_eq!(
        found(&discovered),
        [(Source::User, "fine", "~/cfg/extensions/fine")]
    );
    let link = "it is a link, and an extension is read from a folder of its own in the folder of \
                extensions, never through a link";
    assert_eq!(
        left(&discovered),
        [
            (
                Source::User,
                "~/cfg/extensions/again",
                format!("the extension folder `~/cfg/extensions/again` was left out: {link}")
                    .as_str()
            ),
            (
                Source::User,
                "~/cfg/extensions/alias",
                format!("the extension folder `~/cfg/extensions/alias` was left out: {link}")
                    .as_str()
            ),
        ]
    );
    assert!(!format!("{discovered:?}").contains("SECRET-OUTSIDE"));
}

#[test]
fn one_name_from_the_users_root_and_the_projects_is_two_extensions() {
    let scratch = scratch!("one_name_from_the_users_root_and_the_projects_is_two_extensions");
    extension(&scratch, "home/cfg/extensions/tracker", "tracker");
    extension(&scratch, "home/proj/.cineris/extensions/tracker", "tracker");
    scratch.file(
        "home/proj/.cineris/extensions/tracker/main.js",
        "// the project's",
    );
    extension(&scratch, "home/more/extensions/tracker", "tracker");
    let discovered = discover(
        &scratch,
        &[
            user(&scratch),
            project(&scratch),
            Root::user(&scratch.join("home/more")),
        ],
    );
    assert_eq!(
        found(&discovered),
        [
            (Source::User, "tracker", "~/cfg/extensions/tracker"),
            (Source::Project, "tracker", ".cineris/extensions/tracker"),
        ]
    );
    assert_ne!(
        discovered.found[0].snapshot.hash(),
        discovered.found[1].snapshot.hash()
    );
    // A second of the user's of one name, from another root: the first
    // stands.
    assert_eq!(
        left(&discovered),
        [(
            Source::User,
            "~/more/extensions/tracker",
            "the extension folder `~/more/extensions/tracker` was left out: the user's extension \
             `tracker` was found first, in `~/cfg/extensions/tracker`"
        )]
    );
}

/// Why an extension whose name differs only in case from `others`' is
/// left out.
fn differs_in_case(shown: &str, name: &str, others: &str, more_than_one: bool) -> String {
    let (those, none) = if more_than_one {
        ("those of the user's extensions", "none of them is used")
    } else {
        ("that of the user's extension", "neither is used")
    };
    format!(
        "the extension folder `{shown}` was left out: its name `{name}` differs only in case from \
         {those} {others}: names that differ only in case are one name on Windows, where they \
         would share what an extension keeps under its name, so {none}"
    )
}

/// Only a system that tells the two names apart in an ordinary folder can
/// hold both in one: on Windows the second would be written into the
/// first. Windows has the test below, in a folder made case-sensitive.
#[cfg(unix)]
#[test]
fn names_that_differ_only_in_case_in_one_source_are_all_refused() {
    let scratch = scratch!("names_that_differ_only_in_case_in_one_source_are_all_refused");
    for (folder, name) in [
        ("home/cfg/extensions/Lint", "Lint"),
        ("home/cfg/extensions/lint", "lint"),
        ("home/cfg/extensions/Tracker", "Tracker"),
        ("home/cfg/extensions/tracker", "tracker"),
        ("home/cfg/extensions/other", "other"),
        // Another root of the same source.
        ("home/more/extensions/TRACKER", "TRACKER"),
        // The project's is another source, and stands.
        ("home/proj/.cineris/extensions/tracker", "tracker"),
    ] {
        extension(&scratch, folder, name);
    }
    let discovered = discover(
        &scratch,
        &[
            user(&scratch),
            Root::user(&scratch.join("home/more")),
            project(&scratch),
        ],
    );
    assert_eq!(
        found(&discovered),
        [
            (Source::User, "other", "~/cfg/extensions/other"),
            (Source::Project, "tracker", ".cineris/extensions/tracker"),
        ]
    );
    // Every one of them refused, naming the others: none depends on which
    // was listed first.
    let reasons: Vec<&str> = discovered
        .left_out
        .iter()
        .map(|left| left.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        [
            differs_in_case(
                "~/cfg/extensions/Lint",
                "Lint",
                "`~/cfg/extensions/lint`",
                false
            ),
            differs_in_case(
                "~/cfg/extensions/Tracker",
                "Tracker",
                "`~/cfg/extensions/tracker` and `~/more/extensions/TRACKER`",
                true
            ),
            differs_in_case(
                "~/cfg/extensions/lint",
                "lint",
                "`~/cfg/extensions/Lint`",
                false
            ),
            differs_in_case(
                "~/cfg/extensions/tracker",
                "tracker",
                "`~/cfg/extensions/Tracker` and `~/more/extensions/TRACKER`",
                true
            ),
            differs_in_case(
                "~/more/extensions/TRACKER",
                "TRACKER",
                "`~/cfg/extensions/Tracker` and `~/cfg/extensions/tracker`",
                true
            ),
        ]
    );
    assert!(
        discovered
            .left_out
            .iter()
            .all(|left| left.source == Source::User)
    );
}

/// A folder made case-sensitive (`fsutil file setCaseSensitiveInfo`) is the
/// one place Windows holds two names that differ only in case: both are
/// refused there, as everywhere. `fsutil` is a process: hence the `sys_`
/// name.
#[cfg(windows)]
#[test]
fn sys_on_windows_names_that_differ_only_in_case_in_a_case_sensitive_folder_are_refused() {
    let scratch = scratch!(
        "sys_on_windows_names_that_differ_only_in_case_in_a_case_sensitive_folder_are_refused"
    );
    let root = scratch.dir("home/cfg/extensions");
    let made = std::process::Command::new("fsutil.exe")
        .args(["file", "setCaseSensitiveInfo"])
        .arg(&root)
        .arg("enable")
        .output()
        .expect("run fsutil");
    if !made.status.success() {
        let why = format!(
            "{}{}",
            String::from_utf8_lossy(&made.stdout),
            String::from_utf8_lossy(&made.stderr)
        );
        assert!(
            std::env::var_os("GITHUB_ACTIONS").is_none(),
            "the CI runner cannot make a folder case-sensitive: {why}"
        );
        println!("skipped: cannot make a folder case-sensitive here ({why})");
        return;
    }
    extension(&scratch, "home/cfg/extensions/Tracker", "Tracker");
    extension(&scratch, "home/cfg/extensions/tracker", "tracker");
    extension(&scratch, "home/cfg/extensions/other", "other");
    assert_eq!(
        scratch.listing("home/cfg/extensions"),
        ["Tracker", "other", "tracker"],
        "the folder holds both names"
    );
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert_eq!(
        found(&discovered),
        [(Source::User, "other", "~/cfg/extensions/other")]
    );
    let reasons: Vec<&str> = discovered
        .left_out
        .iter()
        .map(|left| left.reason.as_str())
        .collect();
    assert_eq!(
        reasons,
        [
            differs_in_case(
                "~/cfg/extensions/Tracker",
                "Tracker",
                "`~/cfg/extensions/tracker`",
                false
            ),
            differs_in_case(
                "~/cfg/extensions/tracker",
                "tracker",
                "`~/cfg/extensions/Tracker`",
                false
            ),
        ]
    );
}

#[test]
fn past_64_folders_a_root_the_rest_are_left_out_and_counted() {
    let scratch = scratch!("past_64_folders_a_root_the_rest_are_left_out_and_counted");
    // Made in an order that is not their names'.
    for index in (0..MAX_PER_ROOT + 2).rev() {
        let name = format!("e{index:02}");
        extension(
            &scratch,
            &format!("home/proj/.cineris/extensions/{name}"),
            &name,
        );
    }
    let discovered = discover(&scratch, &[project(&scratch)]);
    let names: Vec<String> = discovered
        .found
        .iter()
        .map(|found| found.manifest.name.clone())
        .collect();
    let first: Vec<String> = (0..MAX_PER_ROOT)
        .map(|index| format!("e{index:02}"))
        .collect();
    assert_eq!(names, first, "the first 64 by name are read");
    assert_eq!(
        left(&discovered),
        [(
            Source::Project,
            ".cineris/extensions",
            "the project's folder of extensions `.cineris/extensions` holds 2 more folders than \
             the 64 read from it, the first by name: they were left out"
        )]
    );
}

#[test]
fn a_root_of_more_than_1024_entries_is_looked_at_only_so_far() {
    let scratch = scratch!("a_root_of_more_than_1024_entries_is_looked_at_only_so_far");
    let root = scratch.dir("home/cfg/extensions");
    for index in 0..MAX_ENTRIES {
        std::fs::write(root.join(format!("f{index:04}.md")), "").unwrap();
    }
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert_eq!(left(&discovered), [], "1024 entries are all looked at");
    scratch.file("home/cfg/extensions/one-more.md", "");
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert_eq!(
        left(&discovered),
        [(
            Source::User,
            "~/cfg/extensions",
            "the user's folder of extensions `~/cfg/extensions` holds more than 1024 entries, \
             and the rest of it was not looked at"
        )]
    );
}

#[test]
fn every_refusal_names_the_folder_with_what_acts_unseen_written_as_an_escape() {
    let scratch =
        scratch!("every_refusal_names_the_folder_with_what_acts_unseen_written_as_an_escape");
    // A right-to-left override in a folder's name, which every system
    // allows, and an escape character in a manifest's name.
    extension(
        &scratch,
        "home/proj/.cineris/extensions/txt\u{202e}exe",
        "txt",
    );
    scratch.file(
        "home/proj/.cineris/extensions/bell/extension.toml",
        "api = 1\nname = \"be\\u001bll\"\nversion = \"1\"\nmain = \"main.js\"\n",
    );
    scratch.file("home/proj/.cineris/extensions/bell/main.js", "1");
    // A key the manifest does not know, which the TOML reader's refusal
    // quotes as it is: escaped by discovery, whose words these are not.
    scratch.file(
        "home/proj/.cineris/extensions/keyed/extension.toml",
        "api = 1\nname = \"keyed\"\nversion = \"1\"\nmain = \"main.js\"\n\"x\\u202Ey\" = 1\n",
    );
    scratch.file("home/proj/.cineris/extensions/keyed/main.js", "1");
    let discovered = discover(&scratch, &[project(&scratch)]);
    assert_eq!(found(&discovered), []);
    let reasons: Vec<&str> = discovered
        .left_out
        .iter()
        .map(|left| left.reason.as_str())
        .collect();
    assert_eq!(reasons.len(), 3, "{reasons:?}");
    assert!(
        reasons[0].starts_with(
            "the extension folder `.cineris/extensions/bell` was left out: the manifest of the \
             extension is refused: `name = \"be\\u{1b}ll\"`"
        ),
        "{}",
        reasons[0]
    );
    assert!(
        reasons[1].starts_with("the extension folder `.cineris/extensions/keyed` was left out")
            && reasons[1].contains("x\\u{202e}y"),
        "{}",
        reasons[1]
    );
    assert_eq!(
        reasons[2],
        "the extension folder `.cineris/extensions/txt\\u{202e}exe` was left out: its manifest \
         gives the name `txt`, and an extension's manifest (`extension.toml`) gives its folder's \
         own name, `txt\\u{202e}exe`"
    );
    assert_eq!(
        discovered.left_out[2].shown,
        ".cineris/extensions/txt\\u{202e}exe"
    );
    for left in &discovered.left_out {
        assert!(
            !left.reason.contains(['\u{202e}', '\u{1b}']),
            "{:?}",
            left.reason
        );
    }
}

#[cfg(unix)]
#[test]
fn a_folder_name_with_a_line_break_is_shown_as_an_escape() {
    let scratch = scratch!("a_folder_name_with_a_line_break_is_shown_as_an_escape");
    extension(&scratch, "home/cfg/extensions/two\nlines", "two");
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert_eq!(
        left(&discovered),
        [(
            Source::User,
            "~/cfg/extensions/two\\nlines",
            "the extension folder `~/cfg/extensions/two\\nlines` was left out: its manifest gives \
             the name `two`, and an extension's manifest (`extension.toml`) gives its folder's \
             own name, `two\\nlines`"
        )]
    );
}

#[test]
fn a_root_that_is_a_file_or_a_link_to_nothing_is_left_out_with_its_reason() {
    let scratch =
        scratch!("a_root_that_is_a_file_or_a_link_to_nothing_is_left_out_with_its_reason");
    scratch.file("home/cfg/extensions", "not a folder");
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert_eq!(
        left(&discovered),
        [(
            Source::User,
            "~/cfg/extensions",
            "the user's folder of extensions `~/cfg/extensions` was not looked in: it is not a \
             folder"
        )]
    );
    scratch.dir("home/proj/.cineris");
    let workspace = scratch.join("home/proj");
    if !make_link(
        &workspace.join("gone"),
        &workspace.join(".cineris/extensions"),
        true,
    ) {
        return;
    }
    let discovered = discover(&scratch, &[project(&scratch)]);
    let written = ".cineris/extensions".to_owned();
    assert_eq!(
        left(&discovered),
        [(
            Source::Project,
            written.as_str(),
            format!(
                "the project's folder of extensions `{written}` was not looked in: it is a link \
                 to something that is not there"
            )
            .as_str()
        )]
    );
}

#[cfg(windows)]
#[test]
fn on_windows_a_root_in_another_case_and_with_backslashes_is_the_same_place() {
    let scratch =
        scratch!("on_windows_a_root_in_another_case_and_with_backslashes_is_the_same_place");
    extension(&scratch, "home/proj/.cineris/extensions/fine", "fine");
    extension(&scratch, "home/cfg/extensions/mine", "mine");
    let paths = resolver(&scratch);
    let upper = scratch.join("home/proj").to_string_lossy().to_uppercase();
    let roots = [
        Root::new(Source::Project, r".CINERIS\Extensions"),
        Root::new(Source::User, r"~\CFG\EXTENSIONS\"),
        // The same folder again, in yet another case and absolute, and as
        // the workspace's real location gives it (`\\?\C:\…`): looked in
        // once.
        Root::new(Source::Project, format!(r"{upper}\.Cineris\EXTENSIONS")),
        Root::project(&paths.root().to_fs_path()),
    ];
    let discovered = discover::discover(&roots, &paths);
    // Shown as the file system spells it.
    assert_eq!(
        found(&discovered),
        [
            (Source::Project, "fine", ".cineris/extensions/fine"),
            (Source::User, "mine", "~/cfg/extensions/mine"),
        ]
    );
    let reasons: Vec<&str> = discovered
        .left_out
        .iter()
        .map(|left| left.reason.as_str())
        .collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    for reason in reasons {
        assert!(
            reason.ends_with(
                "was not looked in: it is the same folder as `.cineris/extensions`, which was \
                 looked in instead"
            ),
            "{reason}"
        );
    }
}

/// Every folder's name is kept, read or not and an extension or not, with
/// where each root really is; and `names` gives them reading no folder.
#[test]
fn every_folders_name_is_kept_and_names_reads_no_folder() {
    let scratch = scratch!("every_folders_name_is_kept_and_names_reads_no_folder");
    extension(&scratch, "home/cfg/extensions/fine", "fine");
    // Not an extension: a manifest of another name.
    extension(&scratch, "home/cfg/extensions/Other", "else");
    scratch.file("home/cfg/extensions/notes.txt", "a file beside the folders");
    scratch.dir("home/cfg/extensions/.git");
    let wanted = [
        (Source::User, "Other".to_owned()),
        (Source::User, "fine".to_owned()),
    ];
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert_eq!(found(&discovered).len(), 1);
    assert_eq!(discovered.names, wanted);
    assert!(discovered.cut.is_empty());
    let real = std::fs::canonicalize(scratch.join("home/cfg/extensions")).unwrap();
    assert_eq!(discovered.roots.len(), 1);
    assert!(cineris_base::same_location(&discovered.roots[0].1, &real));

    let read = cineris_testkit::Hold::new(discover::AN_EXTENSION_IS_READ).skipping(usize::MAX);
    let named = discover::names(&[user(&scratch)], &resolver(&scratch));
    assert_eq!(read.arrivals(), 0, "no folder is read");
    assert!(named.found.is_empty());
    assert_eq!(named.names, wanted);
}

/// At most 16 MiB are read from one root: past them the rest of its folders
/// are left out, the first by name read, and counted (the full review of
/// step 28).
#[test]
fn past_16_mib_a_root_the_rest_are_left_out_and_counted() {
    let scratch = scratch!("past_16_mib_a_root_the_rest_are_left_out_and_counted");
    let mebibyte = vec![b'x'; 1024 * 1024 - 64];
    // Each folder a little under its own cap of 4 MiB: four of them are a
    // little under 16 MiB, so a fifth is read, and the sixth is not.
    for name in ["a", "b", "c", "d", "e", "f"] {
        extension(&scratch, &format!("home/cfg/extensions/{name}"), name);
        for file in 0..4 {
            scratch.file(
                format!("home/cfg/extensions/{name}/data{file}.txt"),
                &mebibyte,
            );
        }
    }
    let discovered = discover(&scratch, &[user(&scratch)]);
    let names: Vec<&str> = found(&discovered)
        .into_iter()
        .map(|(_, name, _)| name)
        .collect();
    assert_eq!(names, ["a", "b", "c", "d", "e"]);
    assert_eq!(
        left(&discovered),
        [(
            Source::User,
            "~/cfg/extensions",
            "the user's folder of extensions `~/cfg/extensions` holds 1 more folder past the 16 \
             MiB read from it, the first by name: it was left out"
        )]
    );
}

/// A folder swapped for a link while it is read is left out: it is looked
/// at again after the read, and what is pinned is a folder of its own.
#[test]
fn a_folder_swapped_for_a_link_while_it_is_read_is_left_out() {
    let scratch = scratch!("a_folder_swapped_for_a_link_while_it_is_read_is_left_out");
    extension(&scratch, "home/cfg/extensions/probe", "probe");
    let read = cineris_testkit::Hold::new(discover::AN_EXTENSION_WAS_READ);
    let home = scratch.join("home");
    let discovering = {
        let home = home.clone();
        std::thread::spawn(move || {
            let workspace = home.join("proj");
            std::fs::create_dir_all(&workspace).unwrap();
            let paths = PathResolver::new(
                &workspace,
                PathOptions::new().home(AbsPath::from_os(&home).unwrap()),
            )
            .unwrap();
            discover::discover(&[Root::user(&home.join("cfg"))], &paths)
        })
    };
    read.reach();
    let folder = home.join("cfg/extensions/probe");
    let was = home.join("cfg/extensions/probe-was");
    std::fs::rename(&folder, &was).unwrap();
    let linked = make_link(&was, &folder, true);
    read.release();
    let discovered = discovering.join().unwrap();
    if !linked {
        return;
    }
    assert!(found(&discovered).is_empty(), "{:?}", found(&discovered));
    assert!(
        left(&discovered).iter().any(|(_, _, reason)| reason
            .contains("it changed while it was read, and is no longer a folder of its own")),
        "{:?}",
        left(&discovered)
    );
}

/// Every folder read counts toward the 16 MiB, one left out after its
/// read too (the fresh review of step 28: folders of 4 MiB with no
/// manifest were all read before anything was trusted).
#[test]
fn past_16_mib_a_root_the_rest_are_left_out_unread_though_none_had_a_manifest() {
    let scratch =
        scratch!("past_16_mib_a_root_the_rest_are_left_out_unread_though_none_had_a_manifest");
    let mebibyte = vec![b'x'; 1024 * 1024 - 64];
    for name in ["a", "b", "c", "d", "e", "f", "g"] {
        for file in 0..4 {
            scratch.file(
                format!("home/cfg/extensions/{name}/data{file}.txt"),
                &mebibyte,
            );
        }
    }
    let reads = cineris_testkit::Hold::new(discover::AN_EXTENSION_IS_READ).skipping(usize::MAX);
    let discovered = discover(&scratch, &[user(&scratch)]);
    assert!(found(&discovered).is_empty());
    assert!(reads.arrivals() <= 5, "{} folders read", reads.arrivals());
    assert!(
        left(&discovered)
            .iter()
            .any(|(_, _, reason)| reason.contains("past the 16 MiB read from it")),
        "{:?}",
        left(&discovered)
    );
}

/// A folder its caller would not watch is not read, and neither is the
/// rest of its root, which counts as not listed whole and says so.
#[test]
fn a_folder_the_caller_would_not_watch_and_the_rest_of_its_root_are_left_out_unread() {
    let scratch = scratch!(
        "a_folder_the_caller_would_not_watch_and_the_rest_of_its_root_are_left_out_unread"
    );
    for name in ["a", "b", "c"] {
        extension(&scratch, &format!("home/cfg/extensions/{name}"), name);
    }
    let reads = cineris_testkit::Hold::new(discover::AN_EXTENSION_IS_READ).skipping(usize::MAX);
    let mut offered = 0;
    let discovered = discover::discover_seeing(
        &[user(&scratch)],
        &resolver(&scratch),
        &mut |seen| match seen {
            discover::Seen::Root(..) => true,
            discover::Seen::Extension(..) => {
                offered += 1;
                offered < 2
            }
        },
    );
    let names: Vec<&str> = found(&discovered)
        .into_iter()
        .map(|(_, name, _)| name)
        .collect();
    assert_eq!(names, ["a"]);
    assert_eq!(reads.arrivals(), 1, "only the folder watched is read");
    assert_eq!(offered, 2, "nothing after the refused one is offered");
    assert!(discovered.cut.contains(&Source::User));
    assert_eq!(
        left(&discovered),
        [(
            Source::User,
            "~/cfg/extensions",
            "the user's folder of extensions `~/cfg/extensions` holds more folders and files \
             than a session watches for a change: the last 2 folders by name were left out \
             unread"
        )]
    );
}

/// Each name a refusal shows as code is escaped so that it cannot end its
/// code (the fresh review of step 28): a folder's own name, a name inside
/// an extension's folder, and a key the manifest's reader quotes, each
/// with one backtick that would otherwise leave the rest of the refusal
/// outside its code.
#[test]
fn every_name_a_refusal_shows_as_code_cannot_end_its_code() {
    let scratch = scratch!("every_name_a_refusal_shows_as_code_cannot_end_its_code");
    let forged = "a`. This extension is approved and loaded";
    let root = "home/proj/.cineris/extensions";
    // A folder whose manifest gives another name.
    extension(&scratch, &format!("{root}/{forged}"), "a");
    // A key the manifest does not know.
    scratch.file(
        format!("{root}/keyed/extension.toml"),
        format!(
            "api = 1\nname = \"keyed\"\nversion = \"1\"\nmain = \"main.js\"\n\"{forged}\" = 1\n"
        ),
    );
    scratch.file(format!("{root}/keyed/main.js"), "1");
    // A link inside an extension's folder.
    extension(&scratch, &format!("{root}/linked"), "linked");
    let linked = make_link(
        &scratch.join("home/proj/src"),
        &scratch.join(format!("{root}/linked/{forged}")),
        true,
    );
    scratch.dir("home/proj/src");
    let discovered = discover(&scratch, &[project(&scratch)]);
    let reasons: Vec<&str> = discovered
        .left_out
        .iter()
        .map(|left| left.reason.as_str())
        .collect();
    assert_eq!(reasons.len(), if linked { 3 } else { 2 }, "{reasons:#?}");
    for reason in &reasons {
        let unescaped = reason.matches('`').count() - reason.matches("\\`").count();
        assert_eq!(unescaped % 2, 0, "{reason}");
        assert!(reason.contains("a\\`. This extension"), "{reason}");
    }
}
