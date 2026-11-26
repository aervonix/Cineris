//! `cineris ext`: the words it prints and asks, and one world in which
//! each subcommand runs through a core in this process, as the binary runs
//! it, on a home of the test's own.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cineris_config::{Dirs, Env, Platform};
use cineris_core::{Core, CoreOptions, ExtensionStanding, Project};

use super::*;

/// A home of the test's own, a workspace in it, and a core that reads
/// nothing of the machine's.
struct World {
    _scratch: cineris_testkit::Scratch,
    dir: PathBuf,
    dirs: Dirs,
    core: Core,
    project: Project,
    runtime: tokio::runtime::Runtime,
}

impl World {
    fn new(test: &str) -> Self {
        let scratch = cineris_testkit::Scratch::outside("cineris-cli-unit", test);
        let dir = scratch.path().to_path_buf();
        let home = dir.join("home");
        let workspace = home.join("work");
        std::fs::create_dir_all(&workspace).unwrap();
        let variable = match Platform::CURRENT {
            Platform::Unix => "HOME",
            Platform::Windows => "USERPROFILE",
        };
        let env = Env::empty(Platform::CURRENT).with(variable, home.to_str().unwrap());
        let mut dirs = Dirs::from_env(&env);
        dirs.managed_config = Some(dir.join("managed.toml"));
        let clock = cineris_base::FakeClock::starting_at("2026-11-25T10:00:00Z").unwrap();
        let timer = Arc::new(cineris_testkit::timer::FakeTimer::manual(&clock));
        let mut options = CoreOptions::new(env, timer);
        options.dirs = Some(dirs.clone());
        options.clock = Arc::new(clock);
        let core = Core::start(options).unwrap();
        let project = core.project(&workspace).unwrap();
        Self {
            _scratch: scratch,
            dir,
            dirs,
            core,
            project,
            runtime: tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap(),
        }
    }

    /// An extension's folder at `at`, registering one tool.
    fn extension(at: &Path, name: &str, tool: &str) {
        std::fs::create_dir_all(at).unwrap();
        std::fs::write(
            at.join("extension.toml"),
            format!(
                "api = 1\nname = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"Finds \
                 things.\"\nmain = \"main.js\"\n\n[capabilities]\nnet = [\"*\"]\n"
            ),
        )
        .unwrap();
        std::fs::write(
            at.join("main.js"),
            format!(
                "cineris.tool({{ name: \"{tool}\", description: \"Finds.\", input: {{ type: \
                 \"object\" }}, async run() {{ return {{}}; }} }});\n"
            ),
        )
        .unwrap();
    }

    fn users(&self) -> PathBuf {
        self.dirs.config.clone().unwrap().join("extensions")
    }

    fn standing(&self, name: &str) -> ExtensionStanding {
        let listing = self.core.extensions(&self.project).unwrap();
        listing
            .extensions
            .iter()
            .find(|one| one.name == name)
            .unwrap_or_else(|| panic!("{name}: {listing:#?}"))
            .standing
            .clone()
    }
}

/// One story, as a person meets it: an extension added from a folder of
/// their own is shown in words and file by file, written to their folder
/// of extensions and pinned as shown, its registration kept; listed as
/// approved with what it registered; turned off in their configuration;
/// added again in place once changed, the question naming it; and removed,
/// its pin taken away and its folder deleted. A project's extension is
/// pointed at `permissions trust` by `disable` and `remove`.
#[test]
fn an_extension_is_added_listed_turned_off_and_removed() {
    let world = World::new("ext_story");
    let source = world.dir.join("src/tracker");
    World::extension(&source, "tracker", "find");
    let to_add = world
        .core
        .extension_to_add(&world.project, &source)
        .unwrap();
    let shown = to_add_text(&to_add);
    assert!(shown.contains(cineris_core::ext::ANY_HOST), "{shown}");
    assert!(shown.contains("`extension.toml`") && shown.contains("`main.js`"), "{shown}");
    assert!(to_add.destination.ends_with("extensions/tracker"), "{}", to_add.destination);
    assert!(to_add.destination.starts_with("~/"), "under ~: {}", to_add.destination);
    assert!(!to_add.in_place && !to_add.unchanged && to_add.replaces.is_none());
    let hash = to_add.hash;
    // What is approved is what was read: a change to the source after the
    // read is not taken.
    std::fs::write(source.join("main.js"), "throw new Error('changed');").unwrap();
    let added = world.runtime.block_on(world.core.add_extension(to_add)).unwrap();
    assert!(added.written);
    assert_eq!(added.registered.tools, ["find"]);
    assert_eq!(added.not_cached, None);
    let written = world
        .core
        .extension_to_add(&world.project, &world.users().join("tracker"))
        .unwrap();
    assert_eq!(written.hash, hash, "the snapshot shown is the one written");
    assert!(written.in_place && written.unchanged && written.pinned);
    assert_eq!(world.standing("tracker"), ExtensionStanding::Approved);
    let listing = world.core.extensions(&world.project).unwrap();
    let text = listing_text(&listing, &world.project.shown);
    assert!(text.contains("`tracker`") && text.contains("tools `find`"), "{text}");
    let inspected = inspected_text(
        listing
            .extensions
            .iter()
            .find(|one| one.name == "tracker")
            .unwrap(),
    );
    assert!(inspected.contains(&hash.to_string()), "{inspected}");
    // Turned off, in a file made for it.
    let outcome = world
        .core
        .disable_extension(&world.project, "tracker")
        .unwrap();
    let (said, done) = disabled_text(&outcome, "tracker", "cineris");
    assert!(done, "{said}");
    assert!(said.contains("`[extensions.tracker] enabled = false`"), "{said}");
    assert!(said.contains("does not apply it"), "{said}");
    assert_eq!(world.standing("tracker"), ExtensionStanding::TurnedOff);
    // Changed in place, and added again there: the question names it.
    let kept = world.users().join("tracker");
    World::extension(&kept, "tracker", "find_more");
    let again = world.core.extension_to_add(&world.project, &kept).unwrap();
    assert!(again.in_place && again.pinned_before.is_some());
    assert!(add_question(&again).contains(&again.destination));
    // Removed with no yes: unpinned, its folder left as it is.
    let removal = world
        .core
        .extension_removal(&world.project, "tracker")
        .unwrap();
    assert!(removal.pin.is_some());
    let removed = world.core.remove_extension(&removal, false).unwrap();
    assert!(removed.unpinned && !removed.deleted);
    assert!(kept.exists());
    let listing = world.core.extensions(&world.project).unwrap();
    assert!(listing.extensions.iter().all(|one| one.pin.is_none()), "{listing:#?}");
    // Removed on the yes, the question naming its folder under `~`: its
    // folder deleted.
    let removal = world
        .core
        .extension_removal(&world.project, "tracker")
        .unwrap();
    let question = removal_question(&removal.folder.as_ref().unwrap().0, false);
    assert!(question.contains("~/") && question.contains("extensions/tracker"), "{question}");
    let removed = world.core.remove_extension(&removal, true).unwrap();
    assert!(!removed.unpinned && removed.deleted);
    assert!(!kept.exists());
    // A project's extension: pointed at the project's trust.
    World::extension(
        &world.project.root.join(".cineris/extensions/theirs"),
        "theirs",
        "x",
    );
    let outcome = world.core.disable_extension(&world.project, "theirs").unwrap();
    let (said, done) = disabled_text(&outcome, "theirs", "cin");
    assert!(!done && said.contains("`cin permissions trust`"), "{said}");
    let removal = world
        .core
        .extension_removal(&world.project, "theirs")
        .unwrap();
    assert!(removal.project_only);
    let refused = world.core.remove_extension(&removal, true).unwrap_err();
    assert!(refused.message.contains("`cineris permissions trust`"), "{}", refused.message);
    assert!(world.project.root.join(".cineris/extensions/theirs").exists());
}

/// What `ext add` refuses before anything is shown: a folder that is a link,
/// or holds one; a name that differs only in case from a folder of the
/// user's; a name no session would find. And `remove` refuses a name that
/// is not one, so that nothing outside the folder of extensions is named.
#[test]
fn what_cannot_be_added_or_removed_is_refused_first() {
    let world = World::new("ext_refused");
    let users = world.users();
    World::extension(&users.join("Tracker"), "Tracker", "find");
    let source = world.dir.join("src/tracker");
    World::extension(&source, "tracker", "find");
    let refused = world.core.extension_to_add(&world.project, &source).unwrap_err();
    assert!(refused.message.contains("only in case"), "{}", refused.message);
    let dotted = world.dir.join("src/dotted");
    World::extension(&dotted, "trailing.", "find");
    let refused = world.core.extension_to_add(&world.project, &dotted).unwrap_err();
    assert!(refused.message.contains("Windows"), "{}", refused.message);
    #[cfg(unix)]
    {
        let linked = world.dir.join("src/linked");
        std::os::unix::fs::symlink(&source, &linked).unwrap();
        let refused = world.core.extension_to_add(&world.project, &linked).unwrap_err();
        assert!(refused.message.contains("is a link"), "{}", refused.message);
        std::os::unix::fs::symlink(world.dir.join("home"), source.join("out")).unwrap();
        let refused = world.core.extension_to_add(&world.project, &source).unwrap_err();
        assert!(refused.message.contains("`out` is a link"), "{}", refused.message);
    }
    // A folder of another content at the name: the question names it.
    World::extension(&users.join("other"), "other", "kept");
    let other = world.dir.join("src/other");
    World::extension(&other, "other", "new");
    let to_add = world.core.extension_to_add(&world.project, &other).unwrap();
    let question = add_question(&to_add);
    assert!(
        question.contains(&format!("replace `{}`", to_add.destination))
            && question.contains("holds something else"),
        "{question}"
    );
    // A script that cannot be evaluated: nothing is written or pinned.
    let broken = world.dir.join("src/broken");
    World::extension(&broken, "broken", "x");
    std::fs::write(broken.join("main.js"), "throw new Error('broken');").unwrap();
    let to_add = world.core.extension_to_add(&world.project, &broken).unwrap();
    let refused = world
        .runtime
        .block_on(world.core.add_extension(to_add))
        .unwrap_err();
    assert!(refused.message.contains("nothing was written or pinned"), "{}", refused.message);
    assert!(!users.join("broken").exists());
    assert!(world.core.extension_removal(&world.project, "broken").is_err(), "not pinned");
    for name in ["..", "../../home", "a/b", ".hidden"] {
        assert!(
            world.core.extension_removal(&world.project, name).is_err(),
            "{name}"
        );
    }
    assert!(users.join("Tracker").exists());
}

/// A yes is `y` or `yes` at a terminal, `--yes` ignored there; with no
/// terminal only `--yes`, and nothing is read.
#[test]
fn a_yes_is_the_persons_at_a_terminal_and_the_flags_without_one() {
    let said = |line: &str| {
        let line = line.to_owned();
        move || Ok(line)
    };
    for yes in ["y\n", "YES\n", " yes "] {
        assert!(approved(true, false, said(yes)), "{yes:?}");
    }
    for no in ["", "\n", "n\n", "yess\n"] {
        assert!(!approved(true, true, said(no)), "{no:?}");
    }
    let unread = || -> io::Result<String> { panic!("nothing is asked without a terminal") };
    assert!(!approved(false, false, unread));
    assert!(approved(false, true, unread));
}

/// The way `disable` names the table is TOML's: a name that is not a bare
/// key is quoted.
#[test]
fn the_table_disable_names_is_one_key() {
    assert_eq!(table("tracker"), "[extensions.tracker]");
    assert_eq!(table("acme.lint"), "[extensions.\"acme.lint\"]");
}

