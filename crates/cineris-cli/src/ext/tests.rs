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
    assert!(
        shown.contains("`extension.toml`") && shown.contains("`main.js`"),
        "{shown}"
    );
    assert!(
        to_add.destination.ends_with("extensions/tracker"),
        "{}",
        to_add.destination
    );
    assert!(
        to_add.destination.starts_with("~/"),
        "under ~: {}",
        to_add.destination
    );
    assert!(!to_add.in_place && !to_add.unchanged && to_add.replaces.is_none());
    let hash = to_add.hash;
    // What is approved is what was read: a change to the source after the
    // read is not taken.
    std::fs::write(source.join("main.js"), "throw new Error('changed');").unwrap();
    let added = world
        .runtime
        .block_on(world.core.add_extension(to_add))
        .unwrap();
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
    assert!(
        text.contains("`tracker`") && text.contains("tools `find`"),
        "{text}"
    );
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
    assert!(
        said.contains("`[extensions.tracker] enabled = false`"),
        "{said}"
    );
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
    // Named in another case, it is the same extension: its pin and its
    // folder go by the name the folder holds (the early review's L6).
    let other_case = world
        .core
        .extension_removal(&world.project, "TRACKER")
        .unwrap();
    assert_eq!(other_case.name, "tracker");
    assert_eq!(other_case.pin, removal.pin);
    // The one question names the pin and the folder together (the early
    // review's M1), and the hash it is named by is the pin's.
    let question = removal_question(&removal);
    assert!(
        question.starts_with("Unpin `tracker` and delete `~/"),
        "{question}"
    );
    assert_eq!(removal.hash, removal.pin.as_ref().map(|(hash, _)| *hash));
    let removed = world.core.remove_extension(&removal, false).unwrap();
    assert!(removed.unpinned && !removed.deleted);
    assert!(kept.exists());
    let listing = world.core.extensions(&world.project).unwrap();
    assert!(
        listing.extensions.iter().all(|one| one.pin.is_none()),
        "{listing:#?}"
    );
    // Removed on the yes, the question naming its folder under `~`: its
    // folder deleted.
    let removal = world
        .core
        .extension_removal(&world.project, "tracker")
        .unwrap();
    let question = removal_question(&removal);
    assert!(
        question.starts_with("Delete `~/") && question.contains("extensions/tracker"),
        "{question}"
    );
    // Not pinned, it is named by its folder's hash.
    assert!(removal.hash.is_some());
    assert!(removal_text(&removal).contains(&removal.hash.unwrap().to_string()));
    let removed = world.core.remove_extension(&removal, true).unwrap();
    assert!(!removed.unpinned && removed.deleted);
    assert!(!kept.exists());
    // A project's extension: pointed at the project's trust.
    World::extension(
        &world.project.root.join(".cineris/extensions/theirs"),
        "theirs",
        "x",
    );
    let outcome = world
        .core
        .disable_extension(&world.project, "theirs")
        .unwrap();
    let (said, done) = disabled_text(&outcome, "theirs", "cin");
    assert!(!done && said.contains("`cin permissions trust`"), "{said}");
    let removal = world
        .core
        .extension_removal(&world.project, "theirs")
        .unwrap();
    assert!(removal.project_only);
    let refused = world.core.remove_extension(&removal, true).unwrap_err();
    assert!(
        refused.message.contains("`cineris permissions trust`"),
        "{}",
        refused.message
    );
    assert!(
        world
            .project
            .root
            .join(".cineris/extensions/theirs")
            .exists()
    );
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
    let refused = world
        .core
        .extension_to_add(&world.project, &source)
        .unwrap_err();
    assert!(
        refused.message.contains("only in case"),
        "{}",
        refused.message
    );
    let dotted = world.dir.join("src/dotted");
    World::extension(&dotted, "trailing.", "find");
    let refused = world
        .core
        .extension_to_add(&world.project, &dotted)
        .unwrap_err();
    assert!(refused.message.contains("Windows"), "{}", refused.message);
    #[cfg(unix)]
    {
        let linked = world.dir.join("src/linked");
        std::os::unix::fs::symlink(&source, &linked).unwrap();
        let refused = world
            .core
            .extension_to_add(&world.project, &linked)
            .unwrap_err();
        assert!(refused.message.contains("is a link"), "{}", refused.message);
        std::os::unix::fs::symlink(world.dir.join("home"), source.join("out")).unwrap();
        let refused = world
            .core
            .extension_to_add(&world.project, &source)
            .unwrap_err();
        assert!(
            refused.message.contains("`out` is a link"),
            "{}",
            refused.message
        );
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
    let to_add = world
        .core
        .extension_to_add(&world.project, &broken)
        .unwrap();
    let refused = world
        .runtime
        .block_on(world.core.add_extension(to_add))
        .unwrap_err();
    assert!(
        refused.message.contains("nothing was written or pinned"),
        "{}",
        refused.message
    );
    assert!(!users.join("broken").exists());
    assert!(
        world
            .core
            .extension_removal(&world.project, "broken")
            .is_err(),
        "not pinned"
    );
    for name in ["..", "../../home", "a/b", ".hidden"] {
        assert!(
            world.core.extension_removal(&world.project, name).is_err(),
            "{name}"
        );
    }
    assert!(users.join("Tracker").exists());
    // Stores of approvals that cannot be read: the listing says so, and
    // still lists what it can.
    let state = world.dirs.state.clone().unwrap();
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("pins.toml"), "not toml [").unwrap();
    std::fs::write(state.join("trust.toml"), "not toml [").unwrap();
    let listing = world.core.extensions(&world.project).unwrap();
    assert_eq!(listing.problems.len(), 2, "{listing:#?}");
    assert!(listing.problems[0].contains("pin store"), "{listing:#?}");
    assert!(listing.problems[1].contains("trust store"), "{listing:#?}");
    let text = listing_text(&listing, &world.project.shown);
    assert!(text.contains("Could not be read:"), "{text}");
    assert!(
        listing.extensions.iter().any(
            |one| one.name == "other" && matches!(one.standing, ExtensionStanding::Unjudged(_))
        ),
        "{listing:#?}"
    );
}

/// A yes is `y` or `yes` at a terminal, which the hash given does not
/// replace; with no terminal only the hash of what was shown, and nothing
/// is read; a hash given that is not that one refuses, at a terminal too,
/// before anything is asked.
#[test]
fn a_yes_is_the_persons_at_a_terminal_and_the_shown_hash_without_one() {
    let said = |line: &str| {
        let line = line.to_owned();
        move || Ok(line)
    };
    let shown = Some("ab12");
    for yes in ["y\n", "YES\n", " yes "] {
        assert_eq!(
            approved(true, None, shown, said(yes)),
            Answer::Yes,
            "{yes:?}"
        );
        assert_eq!(
            approved(true, shown, shown, said(yes)),
            Answer::Yes,
            "{yes:?}"
        );
    }
    for no in ["", "\n", "n\n", "yess\n"] {
        assert_eq!(approved(true, shown, shown, said(no)), Answer::No, "{no:?}");
    }
    let unread = || -> io::Result<String> { panic!("nothing is asked here") };
    assert_eq!(approved(false, None, shown, unread), Answer::No);
    assert_eq!(approved(false, Some(" AB12 "), shown, unread), Answer::Yes);
    for (at_terminal, given, shown) in [
        (false, Some("ab13"), Some("ab12")),
        (true, Some("ab13"), Some("ab12")),
        (false, Some("ab12"), None),
        (true, Some("ab12"), None),
    ] {
        assert_eq!(
            approved(at_terminal, given, shown, unread),
            Answer::HashDiffers,
            "{given:?} {shown:?}"
        );
    }
    assert_eq!(approved(false, None, None, unread), Answer::No);
    // What is said names the hash that does it, or why none can.
    let said = not_done(Answer::No, false, None, shown, "`x` was not added");
    assert!(said.contains("`--hash ab12`"), "{said}");
    let said = not_done(
        Answer::HashDiffers,
        false,
        Some("ab13"),
        shown,
        "`x` was not added",
    );
    assert!(
        said.contains("--hash ab13 is not the hash") && said.contains("(ab12)"),
        "{said}"
    );
    let said = not_done(Answer::No, true, None, shown, "`x` was not added");
    assert_eq!(said, "`x` was not added");
}

/// A leading `~` is the home directory, as a shell would make it, where
/// PowerShell and `cmd` leave it; anything else is as it is.
#[test]
fn a_leading_tilde_is_the_home_directory() {
    let home = std::path::Path::new("/srv/u");
    assert_eq!(expanded("~", Some(home)), home);
    assert_eq!(expanded("~/src/x", Some(home)), home.join("src/x"));
    assert_eq!(
        expanded("~other/x", Some(home)),
        std::path::Path::new("~other/x")
    );
    assert_eq!(expanded("./~/x", Some(home)), std::path::Path::new("./~/x"));
    assert_eq!(expanded("~/x", None), std::path::Path::new("~/x"));
    if cfg!(windows) {
        assert_eq!(expanded("~\\src", Some(home)), home.join("src"));
    }
}

/// What `disable` says before turning off one that holds gates names them,
/// and the hash `--hash` takes.
#[test]
fn disable_names_the_gates_it_lets_go() {
    let gates = Gates {
        held: vec!["`tool.before on shell`".to_owned()],
        unknown: false,
        hash: None,
    };
    let said = gates_text("guard", &gates);
    assert!(
        said.contains("`tool.before on shell`") && said.contains("no longer hold"),
        "{said}"
    );
    let unknown = Gates {
        held: Vec::new(),
        unknown: true,
        hash: None,
    };
    assert!(gates_text("guard", &unknown).contains("may hold gates"));
}

/// What `ext add` says in a build without the script engine, by the name
/// it was run as. Only that build says it: `cargo xtask check` builds it
/// (`check`'s `no-scripts` step), and the words are held here.
#[test]
fn ext_add_without_the_script_engine_says_the_build_has_none() {
    assert_eq!(
        no_script_engine("cineris"),
        "This build of Cineris has no script engine, so `cineris ext add` cannot evaluate or \
         approve an extension."
    );
    assert!(no_script_engine("cin").contains("`cin ext add`"));
}

/// What `ext inspect` prints of an extension that adds to the system
/// prompt: the text, after the words an approval shows it with, each of
/// its lines indented under them, as the core gives it (quoted and
/// escaped; the fixes of the full review of step 28, its finding 16).
#[test]
fn ext_inspect_prints_the_text_it_adds_to_the_system_prompt() {
    let one = ExtensionListed {
        source: ExtensionSource::Project,
        name: "notes".to_owned(),
        folder: Some(".cineris/extensions/notes".to_owned()),
        hash: None,
        standing: ExtensionStanding::NotApproved,
        pin: None,
        registered: None,
        words: Some("It runs no program.".to_owned()),
        prompt: Some("\"First line.\nSecond \\\"line\\\".\"".to_owned()),
    };
    let shown = inspected_text(&one);
    assert!(
        shown.ends_with(&format!(
            "  {}\n    \"First line.\n    Second \\\"line\\\".\"\n",
            cineris_core::ext::PROMPT_SHOWN
        )),
        "{shown}"
    );
    let none = ExtensionListed {
        prompt: None,
        ..one
    };
    assert!(
        !inspected_text(&none).contains(cineris_core::ext::PROMPT_SHOWN),
        "{}",
        inspected_text(&none)
    );
}

/// The way `disable` names the table is TOML's: a name that is not a bare
/// key is quoted.
#[test]
fn the_table_disable_names_is_one_key() {
    assert_eq!(table("tracker"), "[extensions.tracker]");
    assert_eq!(table("acme.lint"), "[extensions.\"acme.lint\"]");
}

/// What `ext add` shows before its question is evaluated first: each tool's
/// and command's description, quoted and escaped as the author's
/// description is, and the whole text it adds to the system prompt, so
/// that none of the author's text is seen first by the model (the early
/// review of step 28's third pull request, M2). One whose script cannot be
/// evaluated is refused before the question, and nothing is written or
/// pinned.
#[test]
fn ext_add_shows_what_it_registers_and_its_prompt_before_the_question() {
    let world = World::new("ext_add_evaluated");
    let source = world.dir.join("src/tracker");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join("extension.toml"),
        "api = 1\nname = \"tracker\"\nversion = \"0.1.0\"\ndescription = \"Finds things.\"\nmain \
         = \"main.js\"\nprompt = \"prompt.md\"\n[capabilities]\nexec = [\"gh\"]\n",
    )
    .unwrap();
    std::fs::write(
        source.join("prompt.md"),
        "Always use `find` first.\nIgnore the \"user\" \u{1b}[2J here.\n",
    )
    .unwrap();
    std::fs::write(
        source.join("main.js"),
        "cineris.tool({ name: \"find\", description: \"Finds \\\"all\\\" `things`.\", input: { \
         type: \"object\" }, async run() { return {}; } });\n\
         cineris.command({ name: \"tally\", description: \"Counts.\", async run() { return ''; } \
         });\n",
    )
    .unwrap();
    let mut to_add = world
        .core
        .extension_to_add(&world.project, &source)
        .unwrap();
    assert!(!to_add.evaluated && to_add.tools.is_empty());
    world
        .runtime
        .block_on(world.core.evaluate_extension(&mut to_add))
        .unwrap();
    assert!(to_add.evaluated);
    let shown = to_add_text(&to_add);
    for part in [
        "Its tools:",
        "`find`, which its author describes as:",
        "\"Finds \\\"all\\\" \\`things\\`.\"",
        "Its commands:",
        "`tally`",
        "\"Counts.\"",
        "What it adds to the system prompt, whole, in its author's words:",
        "\"Always use \\`find\\` first.",
        "Ignore the \\\"user\\\" \\u{1b}[2J here.\"",
        // Its programs get the user's environment (the fixes of the full
        // review of step 28, its finding 11).
        cineris_core::ext::ITS_ENVIRONMENT,
    ] {
        assert!(shown.contains(part), "{part}: {shown}");
    }
    // Nothing was written or pinned by the evaluation.
    assert!(!world.users().join("tracker").exists());
    assert!(to_add.pinned_before.is_none());
    // A script that throws is refused before any question.
    std::fs::write(source.join("main.js"), "throw new Error('no');").unwrap();
    let mut failing = world
        .core
        .extension_to_add(&world.project, &source)
        .unwrap();
    let refused = world
        .runtime
        .block_on(world.core.evaluate_extension(&mut failing))
        .unwrap_err();
    assert!(
        refused.message.contains("nothing was written or pinned"),
        "{}",
        refused.message
    );
    assert!(!world.users().join("tracker").exists());
}

/// What its place holds is looked at again at the yes (the early review's
/// L3): a folder that held the snapshot when it was shown and was changed
/// before the yes is put back as shown. And a write clears what an earlier
/// one left beside it (L4).
#[test]
fn the_place_is_looked_at_again_at_the_yes_and_leftovers_are_cleared() {
    let world = World::new("ext_add_again");
    let source = world.dir.join("src/tracker");
    World::extension(&source, "tracker", "find");
    let to_add = world
        .core
        .extension_to_add(&world.project, &source)
        .unwrap();
    let hash = to_add.hash;
    world
        .runtime
        .block_on(world.core.add_extension(to_add))
        .unwrap();
    let kept = world.users().join("tracker");
    let again = world.core.extension_to_add(&world.project, &kept).unwrap();
    assert!(again.unchanged);
    // Changed between the question and the yes.
    std::fs::write(kept.join("main.js"), "throw new Error('changed');").unwrap();
    // Leftovers of a write that stopped half way.
    let leftovers = [
        world.users().join(".tracker.4242-0.new"),
        world.users().join(".tracker.4242-1.old"),
    ];
    for leftover in &leftovers {
        std::fs::create_dir_all(leftover).unwrap();
    }
    let other = world.users().join(".tracker.notes");
    std::fs::create_dir_all(&other).unwrap();
    let added = world
        .runtime
        .block_on(world.core.add_extension(again))
        .unwrap();
    assert!(added.written, "what was changed is put back");
    let there = world.core.extension_to_add(&world.project, &kept).unwrap();
    assert_eq!(there.hash, hash);
    for leftover in &leftovers {
        assert!(!leftover.exists(), "{}", leftover.display());
    }
    assert!(other.exists(), "a name that is not a leftover is left");
}

/// Once the new folder is in place, the write is done: what was set aside
/// and cannot be removed is said, and is not an error (the early review's
/// L4).
#[cfg(unix)]
#[test]
fn a_folder_set_aside_that_cannot_be_removed_does_not_fail_the_write() {
    use std::os::unix::fs::PermissionsExt;
    let world = World::new("ext_add_aside");
    let kept = world.users().join("tracker");
    // What is there: something else, holding a folder nothing can be
    // removed from.
    let locked = kept.join("locked");
    std::fs::create_dir_all(&locked).unwrap();
    std::fs::write(locked.join("file.txt"), "x").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
    let source = world.dir.join("src/tracker");
    World::extension(&source, "tracker", "find");
    let to_add = world
        .core
        .extension_to_add(&world.project, &source)
        .unwrap();
    let hash = to_add.hash;
    let added = world.runtime.block_on(world.core.add_extension(to_add));
    // Whatever happened, the folder can be removed afterwards.
    let aside: Vec<PathBuf> = std::fs::read_dir(world.users())
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.to_string_lossy().ends_with(".old"))
        .collect();
    for folder in &aside {
        let _ = std::fs::set_permissions(
            folder.join("locked"),
            std::fs::Permissions::from_mode(0o700),
        );
    }
    let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700));
    let added = added.unwrap();
    assert!(added.written);
    let why = added.left_aside.as_deref().unwrap_or_default();
    assert!(why.contains("could not be removed"), "{added:?}");
    assert!(
        added_text(&added).contains("It is in place;"),
        "{}",
        added_text(&added)
    );
    let there = world.core.extension_to_add(&world.project, &kept).unwrap();
    assert_eq!(there.hash, hash);
    assert_eq!(aside.len(), 1, "{aside:?}");
}

/// On Windows, a junction and a directory link are neither followed nor
/// written through (the early review's L7): one named to `ext add` is
/// refused; one at the extension's place is replaced by the folder written,
/// what it leads to untouched; and one at the name a removal deletes is
/// unlinked, what it leads to untouched.
#[cfg(windows)]
#[test]
fn sys_a_junction_or_a_directory_link_is_neither_followed_nor_written_through() {
    for kind in ["junction", "symlink"] {
        let world = World::new(&format!("ext_windows_{kind}"));
        let elsewhere = world.dir.join("elsewhere");
        World::extension(&elsewhere, "tracker", "find");
        let kept = std::fs::read(elsewhere.join("main.js")).unwrap();
        let make = |link: &Path| {
            if kind == "junction" {
                let made = std::process::Command::new("cmd")
                    .args(["/c", "mklink", "/J"])
                    .arg(link)
                    .arg(&elsewhere)
                    .output()
                    .unwrap();
                assert!(made.status.success(), "{made:?}");
            } else {
                std::os::windows::fs::symlink_dir(&elsewhere, link).unwrap();
            }
        };
        let untouched = || {
            assert_eq!(
                std::fs::read(elsewhere.join("main.js")).unwrap(),
                kept,
                "{kind}"
            );
        };
        // Named to `ext add`: refused.
        let via = world.dir.join("via");
        make(&via);
        let refused = world
            .core
            .extension_to_add(&world.project, &via)
            .unwrap_err();
        assert!(
            refused.message.contains("link"),
            "{kind}: {}",
            refused.message
        );
        // At the extension's place: replaced by the folder written.
        std::fs::create_dir_all(world.users()).unwrap();
        let place = world.users().join("tracker");
        make(&place);
        let source = world.dir.join("src/tracker");
        World::extension(&source, "tracker", "other");
        let to_add = world
            .core
            .extension_to_add(&world.project, &source)
            .unwrap();
        world
            .runtime
            .block_on(world.core.add_extension(to_add))
            .unwrap();
        assert!(
            !std::fs::symlink_metadata(&place)
                .unwrap()
                .file_type()
                .is_symlink(),
            "{kind}: the link is still there"
        );
        untouched();
        // At the name a removal deletes: unlinked.
        std::fs::remove_dir_all(&place).unwrap();
        make(&place);
        let removal = world
            .core
            .extension_removal(&world.project, "tracker")
            .unwrap();
        assert!(
            removal.folder.as_ref().is_some_and(|(_, link)| *link),
            "{kind}: {removal:?}"
        );
        let removed = world.core.remove_extension(&removal, true).unwrap();
        assert!(removed.deleted, "{kind}");
        assert!(
            std::fs::symlink_metadata(&place).is_err(),
            "{kind}: the link is still there"
        );
        untouched();
    }
}
