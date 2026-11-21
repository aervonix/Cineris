//! Skills are found one level down in their roots, wherever a path or a
//! link leads, within their caps, one skill a name, and listed for the
//! system prompt as one line each.
//!
//! Each test's scratch folder holds `home/`, the home directory `~` stands
//! for, `home/proj/`, the workspace, and `outside/`, which is neither.
//!
//! The front matter's reader, `one_line` and the standard roots are tested
//! here too, as they were in the module.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "a test prepares real files, fails by panicking, and says when it skips"
)]

use cineris_base::{PathOptions, PathStyle};
use cineris_testkit::scratch::Scratch;

use super::*;
use crate::skills;

/// The session's resolver: the workspace `home/proj`, and `~` for `home`.
fn resolver(scratch: &Scratch) -> PathResolver {
    let workspace = scratch.dir("home/proj");
    let home = AbsPath::from_os(&scratch.join("home")).unwrap();
    PathResolver::new(&workspace, PathOptions::new().home(home)).unwrap()
}

/// A `SKILL.md` in `folder`, with this name and description.
fn skill(scratch: &Scratch, folder: &str, name: &str, description: &str) {
    scratch.file(
        format!("{folder}/SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n# {name}\n\nThe steps.\n"),
    );
}

/// Each skill found, as its name and the path it is shown at.
fn listed(found: &Found) -> Vec<(&str, &str)> {
    found
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill.path.as_str()))
        .collect()
}

/// What was left out, as its path and why.
fn left(found: &Found) -> Vec<(&str, &Reason)> {
    found
        .left_out
        .iter()
        .map(|left| (left.path.as_str(), &left.reason))
        .collect()
}

fn find(scratch: &Scratch, roots: &[Root]) -> Found {
    skills::find(&resolver(scratch), roots)
}

/// Makes a symbolic link to a file or, with `directory`, to a folder. On
/// Windows that needs a privilege an ordinary account lacks unless
/// Developer Mode is on; the CI runner has it, and there the link must be
/// made. Elsewhere `false` when it could not be, and the test says so and
/// stops.
fn make_link(target: &Path, link: &Path, directory: bool) -> bool {
    #[cfg(unix)]
    let made = {
        let _ = directory;
        std::os::unix::fs::symlink(target, link)
    };
    #[cfg(windows)]
    let made = if directory {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    match made {
        Ok(()) => true,
        Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
            println!("skipped: cannot make a link here ({error})");
            false
        }
        Err(error) => panic!("the CI runner cannot make a link: {error}"),
    }
}

fn read(text: &str) -> FrontMatter {
    FrontMatter::read(text).unwrap()
}

#[test]
fn plain_values_and_comments() {
    let front = read(
        "---\nname: pdf-tools # the folder's name\ndescription: Fill in forms\n  and merge \
         them.\n\n  Also split.\nlicense: MIT\n---\n# Body\n",
    );
    assert_eq!(front.get("name"), Some("pdf-tools"));
    assert_eq!(
        front.get("description"),
        Some("Fill in forms and merge them.\nAlso split.")
    );
    assert_eq!(front.get("license"), Some("MIT"));
    assert_eq!(front.get("missing"), None);
    // A `#` inside a word is not a comment.
    assert_eq!(
        read("---\nname: c#-tools\n---").get("name"),
        Some("c#-tools")
    );
}

#[test]
fn quoted_values_in_both_quotes_and_over_lines() {
    let front = read(
        "---\nname: \"pdf: tools\" # quoted\ndescription: 'It''s for\n  forms, \"all\" of \
         them.'\n---\n",
    );
    assert_eq!(front.get("name"), Some("pdf: tools"));
    assert_eq!(
        front.get("description"),
        Some("It's for forms, \"all\" of them.")
    );
    let escaped = read(
        "---\nname: \"a\\tb\\u00e9\\x41\\\"\\\\\"\ndescription: \"one \\\n  two\n\n  \
         three\"\n---",
    );
    assert_eq!(escaped.get("name"), Some("a\tbéA\"\\"));
    assert_eq!(escaped.get("description"), Some("one two\nthree"));
    // A quote never closed is taken as far as it goes.
    assert_eq!(read("---\nname: \"open\n---").get("name"), Some("open"));
}

#[test]
fn blocks_keep_or_fold_their_lines() {
    let front = read(
        "---\nname: x\ndescription: >-\n  Folded\n  into one.\n\n  Then more.\nnotes: |\n  \
         kept\n    indented\n  lines\n---",
    );
    assert_eq!(
        front.get("description"),
        Some("Folded into one.\nThen more.")
    );
    assert_eq!(front.get("notes"), Some("kept\n  indented\nlines"));
    // An indentation indicator is taken.
    assert_eq!(
        read("---\nd: |2\n    deeper\n  less\n---").get("d"),
        Some("  deeper\nless")
    );
    // A `>` that is not a block's header is plain text.
    assert_eq!(read("---\nd: >x\n---").get("d"), Some(">x"));
}

#[test]
fn what_is_and_is_not_front_matter() {
    for none in [
        "",
        "name: x\n",
        "# Title\n---\nname: x\n---",
        "---\nname: x\n",
        " ---\nname: x\n---",
    ] {
        assert_eq!(FrontMatter::read(none), None, "{none:?}");
    }
    // A byte-order mark, `\r\n` and `...` as the close.
    let front = read("\u{feff}---\r\nname: x\r\ndescription: y \r\n...\r\nbody");
    assert_eq!(
        (front.get("name"), front.get("description")),
        (Some("x"), Some("y"))
    );
    // A key's first value stands; a nested map belongs to its key; a
    // `key:value` with no space is no field; a quoted key is one.
    let front = read(
        "---\nname: first\nmetadata:\n  name: nested\nname: second\nurl:x\n\"description\" : \
         d\n---",
    );
    assert_eq!(front.get("name"), Some("first"));
    assert_eq!(front.get("url"), None);
    assert_eq!(front.get("description"), Some("d"));
    assert_eq!(
        front.fields().map(|(key, _)| key).collect::<Vec<_>>(),
        ["name", "metadata", "description"]
    );
}

#[test]
fn the_standard_roots_in_the_order_a_name_is_taken() {
    let paths = |compat| {
        Root::standard(compat)
            .into_iter()
            .map(|root| root.path)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        paths(true),
        [
            ".cineris/skills",
            ".agents/skills",
            ".claude/skills",
            "~/.cineris/skills",
            "~/.agents/skills",
            "~/.claude/skills"
        ]
    );
    assert_eq!(paths(false), [".cineris/skills", "~/.cineris/skills"]);
}

#[test]
fn skills_are_found_one_level_down_in_every_root_and_listed_by_name() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "skills_are_found_one_level_down_in_every_root_and_listed_by_name",
    );
    skill(
        &scratch,
        "home/proj/.cineris/skills/zeta",
        "zeta",
        "Last by name",
    );
    skill(&scratch, "home/proj/.agents/skills/alpha", "alpha", "First");
    skill(&scratch, "home/proj/.claude/skills/mid", "mid", "Between");
    skill(
        &scratch,
        "home/.cineris/skills/beta",
        "beta",
        "The user's own",
    );
    scratch.file(
        "home/.claude/skills/plain/SKILL.md",
        "---\nname: plain\n---\nNo description.\n",
    );
    // Not skills: a `SKILL.md` deeper than one folder, one in the root
    // itself, a folder with none, and a file beside the folders.
    skill(
        &scratch,
        "home/proj/.cineris/skills/zeta/nested",
        "deep",
        "No",
    );
    skill(&scratch, "home/proj/.cineris/skills", "flat", "No");
    scratch.file("home/proj/.cineris/skills/empty/notes.md", "nothing");
    scratch.file("home/proj/.cineris/skills/README.md", "the folder's own");

    let resolver = resolver(&scratch);
    let found = skills::find(&resolver, &Root::standard(true));
    assert_eq!(
        listed(&found),
        [
            ("alpha", ".agents/skills/alpha/SKILL.md"),
            ("beta", "~/.cineris/skills/beta/SKILL.md"),
            ("mid", ".claude/skills/mid/SKILL.md"),
            ("plain", "~/.claude/skills/plain/SKILL.md"),
            ("zeta", ".cineris/skills/zeta/SKILL.md"),
        ]
    );
    assert_eq!(left(&found), []);
    assert_eq!(found.not_listed, 0);
    let alpha = &found.skills[0];
    assert_eq!(alpha.description, "First");
    assert_eq!(
        (alpha.owner, alpha.layout),
        (Owner::Project, Layout::Agents)
    );
    assert_eq!(
        &alpha.folder,
        resolver.resolve(".agents/skills/alpha").unwrap().abs()
    );
    assert_eq!(
        found.skills[3].description, "",
        "no description is an empty one"
    );
    assert_eq!(
        (found.skills[3].owner, found.skills[3].layout),
        (Owner::User, Layout::Claude)
    );

    // `compat.skills` off: only `.cineris/skills/`, the project's and the
    // user's.
    let own = skills::find(&resolver, &Root::standard(false));
    assert_eq!(
        listed(&own),
        [
            ("beta", "~/.cineris/skills/beta/SKILL.md"),
            ("zeta", ".cineris/skills/zeta/SKILL.md"),
        ]
    );
}

#[test]
fn roots_written_with_a_tilde_a_relative_path_and_dots_lead_to_the_same_skills() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "roots_written_with_a_tilde_a_relative_path_and_dots_lead_to_the_same_skills",
    );
    skill(&scratch, "home/proj/.cineris/skills/one", "one", "1");
    skill(&scratch, "home/proj/.agents/skills/two", "two", "2");
    skill(&scratch, "home/proj/.claude/skills/three", "three", "3");
    skill(&scratch, "home/.claude/skills/four", "four", "4");
    skill(&scratch, "home/elsewhere/skills/five", "five", "5");
    scratch.dir("home/proj/sub");
    let workspace = scratch.join("home/proj");
    let roots = [
        Root::new(Owner::Project, Layout::Cineris, "./sub/../.cineris/skills"),
        // Out of the workspace and back into it.
        Root::new(Owner::Project, Layout::Agents, "../proj/.agents/skills/"),
        // Absolute, with a `.` step.
        Root::new(
            Owner::Project,
            Layout::Claude,
            format!("{}/./.claude/skills", workspace.display()),
        ),
        Root::new(Owner::User, Layout::Claude, "~/./.claude/../.claude/skills"),
        // The project's own folder written another way is looked at once.
        Root::new(Owner::User, Layout::Cineris, "~/proj/.cineris/skills"),
        // A project's root that leads out of the workspace by `..`.
        Root::new(Owner::Project, Layout::Cineris, "../elsewhere/skills"),
    ];
    let found = find(&scratch, &roots);
    assert_eq!(
        listed(&found),
        [
            ("four", "~/.claude/skills/four/SKILL.md"),
            ("one", ".cineris/skills/one/SKILL.md"),
            ("three", ".claude/skills/three/SKILL.md"),
            ("two", ".agents/skills/two/SKILL.md"),
        ]
    );
    assert_eq!(
        left(&found),
        [
            (
                "~/proj/.cineris/skills",
                &Reason::SameFolder(".cineris/skills".to_owned())
            ),
            ("../elsewhere/skills", &Reason::OutsideWorkspace),
        ]
    );
}

#[test]
fn a_link_in_a_root_that_leads_out_of_it_is_left_out() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_link_in_a_root_that_leads_out_of_it_is_left_out",
    );
    skill(&scratch, "outside/stolen", "stolen", "SECRET-OUTSIDE");
    skill(&scratch, "home/proj/.cineris/skills/fine", "fine", "Inside");
    skill(
        &scratch,
        "home/proj/.cineris/skills/store/real",
        "aliased",
        "Deeper",
    );
    skill(&scratch, "home/proj/.cineris/skills/upper/x", "x", "Unused");
    scratch.dir("home/proj/.cineris/skills/sneaky");
    scratch.dir("home/.claude/skills");
    let root = scratch.join("home/proj/.cineris/skills");
    let links = [
        // A folder that leads out of everything.
        (scratch.join("outside/stolen"), root.join("escape"), true),
        // A `SKILL.md` that does.
        (
            scratch.join("outside/stolen/SKILL.md"),
            root.join("sneaky/SKILL.md"),
            false,
        ),
        // Up out of the root, though still in the workspace.
        (scratch.join("home/proj/.cineris"), root.join("up"), true),
        // The root itself.
        (root.clone(), root.join("self"), true),
        // Inside the root: followed, and one level down by its name.
        (root.join("store/real"), root.join("alias"), true),
        // The user's folder linking into the project's: out of its root.
        (
            root.join("fine"),
            scratch.join("home/.claude/skills/borrowed"),
            true,
        ),
    ];
    for (target, link, directory) in &links {
        if !make_link(target, link, *directory) {
            return;
        }
    }
    let found = find(&scratch, &Root::standard(true));
    assert_eq!(
        listed(&found),
        [
            ("aliased", ".cineris/skills/alias/SKILL.md"),
            ("fine", ".cineris/skills/fine/SKILL.md"),
        ]
    );
    assert_eq!(
        left(&found),
        [
            (".cineris/skills/escape", &Reason::LinkOut),
            (".cineris/skills/self", &Reason::LinkOut),
            (".cineris/skills/sneaky/SKILL.md", &Reason::LinkOut),
            (".cineris/skills/up", &Reason::LinkOut),
            ("~/.claude/skills/borrowed", &Reason::LinkOut),
        ]
    );
    assert!(!format!("{found:?}").contains("SECRET-OUTSIDE"));
}

#[test]
fn a_root_that_is_a_link_is_followed_and_a_projects_only_inside_the_workspace() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_root_that_is_a_link_is_followed_and_a_projects_only_inside_the_workspace",
    );
    skill(
        &scratch,
        "home/dotfiles/skills/mine",
        "mine",
        "Kept in dotfiles",
    );
    skill(&scratch, "outside/shared/skills/far", "far", "Shared");
    skill(
        &scratch,
        "home/proj/docs/skills/documented",
        "documented",
        "In docs",
    );
    for parent in [
        "home/.claude",
        "home/.agents",
        "home/proj/.claude",
        "home/proj/.agents",
    ] {
        scratch.dir(parent);
    }
    let links = [
        (
            scratch.join("home/dotfiles/skills"),
            scratch.join("home/.claude/skills"),
        ),
        (
            scratch.join("outside/shared/skills"),
            scratch.join("home/.agents/skills"),
        ),
        (
            scratch.join("home/proj/docs/skills"),
            scratch.join("home/proj/.claude/skills"),
        ),
        (
            scratch.join("outside/shared/skills"),
            scratch.join("home/proj/.agents/skills"),
        ),
    ];
    for (target, link) in &links {
        if !make_link(target, link, true) {
            return;
        }
    }
    let found = find(&scratch, &Root::standard(true));
    let paths = listed(&found);
    assert_eq!(paths.len(), 3, "{found:?}");
    assert_eq!(paths[0], ("documented", "docs/skills/documented/SKILL.md"));
    // The user's root may lead anywhere: where it leads is shown, under
    // `~` when that is in the home directory.
    assert_eq!(paths[1].0, "far");
    assert!(
        paths[1].1.ends_with("outside/shared/skills/far/SKILL.md") && !paths[1].1.starts_with('~'),
        "{}",
        paths[1].1
    );
    assert_eq!(paths[2], ("mine", "~/dotfiles/skills/mine/SKILL.md"));
    assert_eq!(found.skills[1].owner, Owner::User);
    // The project's root that leads out of the workspace is not followed.
    assert_eq!(
        left(&found),
        [(".agents/skills", &Reason::OutsideWorkspace)]
    );
}

#[test]
fn a_link_outside_that_leads_into_a_root_is_followed() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_link_outside_that_leads_into_a_root_is_followed",
    );
    skill(
        &scratch,
        "home/proj/.cineris/skills/direct",
        "direct",
        "Plain",
    );
    skill(
        &scratch,
        "home/proj/.cineris/skills/store/back",
        "back",
        "By a hop",
    );
    skill(&scratch, "home/.claude/skills/users", "users", "The user's");
    scratch.dir("outside");
    let links = [
        // Paths outside that lead into the workspace and the home.
        (scratch.join("home/proj"), scratch.join("outside/to-proj")),
        (
            scratch.join("home/.claude/skills"),
            scratch.join("outside/to-skills"),
        ),
        // A folder of a root that leaves it and comes back.
        (
            scratch.join("home/proj/.cineris/skills/store/back"),
            scratch.join("outside/hop"),
        ),
        (
            scratch.join("outside/hop"),
            scratch.join("home/proj/.cineris/skills/hop"),
        ),
    ];
    for (target, link) in &links {
        if !make_link(target, link, true) {
            return;
        }
    }
    let outside = scratch.join("outside");
    let roots = [
        Root::new(
            Owner::Project,
            Layout::Cineris,
            format!("{}/to-proj/.cineris/skills", outside.display()),
        ),
        Root::new(
            Owner::User,
            Layout::Claude,
            format!("{}/to-skills", outside.display()),
        ),
    ];
    let found = find(&scratch, &roots);
    assert_eq!(
        listed(&found),
        [
            ("back", ".cineris/skills/hop/SKILL.md"),
            ("direct", ".cineris/skills/direct/SKILL.md"),
            ("users", "~/.claude/skills/users/SKILL.md"),
        ]
    );
    assert_eq!(left(&found), []);
}

#[test]
fn a_link_to_nothing_and_a_skill_file_that_is_no_file_are_left_out() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_link_to_nothing_and_a_skill_file_that_is_no_file_are_left_out",
    );
    scratch.dir("home/proj/.cineris/skills/folder-file/SKILL.md");
    scratch.dir("home/proj/.cineris/skills/file-gone");
    skill(&scratch, "home/proj/.cineris/skills/kept", "kept", "Fine");
    let root = scratch.join("home/proj/.cineris/skills");
    if !make_link(&scratch.join("outside/nothing"), &root.join("gone"), true)
        || !make_link(
            &scratch.join("outside/nothing.md"),
            &root.join("file-gone/SKILL.md"),
            false,
        )
    {
        return;
    }
    let found = find(&scratch, &Root::standard(false));
    assert_eq!(listed(&found), [("kept", ".cineris/skills/kept/SKILL.md")]);
    assert_eq!(
        left(&found),
        [
            (".cineris/skills/file-gone/SKILL.md", &Reason::Dangling),
            (".cineris/skills/folder-file/SKILL.md", &Reason::NotAFile),
            (".cineris/skills/gone", &Reason::Dangling),
        ]
    );
}

#[test]
fn a_skill_needs_front_matter_with_a_name_and_its_quirks_are_read() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_skill_needs_front_matter_with_a_name_and_its_quirks_are_read",
    );
    let at = |folder: &str, text: &str| {
        scratch.file(format!("home/proj/.cineris/skills/{folder}/SKILL.md"), text);
    };
    at("none", "# A skill\n\nNo front matter.\n");
    at("open", "---\nname: open\ndescription: never closed\n");
    at("nameless", "---\ndescription: Has no name\n---\n");
    at("unseen", "---\nname: \"\\u202e\\u200b\"\n---\n");
    at(
        "quoted",
        "\u{feff}---\r\nname: \"pdf: forms\"\r\ndescription: >\r\n  Fill in\r\n  and merge.\r\n---\r\n",
    );
    at(
        "single",
        "---\nname: 'it''s'\ndescription: \"Line one\\nline two\" # comment\nlicense: MIT\n---\n",
    );
    at(
        "folded",
        "---\nname: folded\ndescription:\n  Spread over\n  two lines\nmetadata:\n  author: someone\n---\n",
    );
    let found = find(&scratch, &Root::standard(false));
    let pairs: Vec<(&str, &str)> = found
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill.description.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("folded", "Spread over two lines"),
            ("it's", "Line one line two"),
            ("pdf: forms", "Fill in and merge."),
        ]
    );
    assert_eq!(
        left(&found),
        [
            (".cineris/skills/nameless/SKILL.md", &Reason::NoName),
            (".cineris/skills/none/SKILL.md", &Reason::NoFrontMatter),
            (".cineris/skills/open/SKILL.md", &Reason::NoFrontMatter),
            (".cineris/skills/unseen/SKILL.md", &Reason::NoName),
        ]
    );
}

#[test]
fn names_and_descriptions_are_one_line_with_nothing_unseen_and_cut_at_an_edge() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "names_and_descriptions_are_one_line_with_nothing_unseen_and_cut_at_an_edge",
    );
    // A bidirectional override and a zero-width joiner in the name; a line
    // break, an isolate, a byte-order mark and an escape in the
    // description.
    scratch.file(
        "home/proj/.cineris/skills/bidi/SKILL.md",
        "---\nname: \"ad\u{202e}min\u{200d}\"\ndescription: \"Run \u{2066}this\u{2069}\\nand \
         \u{feff}that\\e[31m\"\n---\n",
    );
    // A long description of two-byte characters, and a long name.
    let long = format!("{}x", "é".repeat(400));
    let name = "n".repeat(100);
    scratch.file(
        "home/proj/.cineris/skills/long/SKILL.md",
        format!("---\nname: {name}\ndescription: {long}\n---\n"),
    );
    // A folder whose own name holds an override: shown as an escape.
    skill(
        &scratch,
        "home/proj/.cineris/skills/x\u{202e}y",
        "named",
        "",
    );
    let found = find(&scratch, &Root::standard(false));
    let names: Vec<&str> = found
        .skills
        .iter()
        .map(|skill| skill.name.as_str())
        .collect();
    assert_eq!(names.len(), 3, "{names:?}");
    let bidi = &found.skills[0];
    assert_eq!(bidi.name, "admin");
    assert_eq!(bidi.description, "Run this and that[31m");
    let cut = &found.skills[2];
    assert!(cut.name.len() <= 64, "{}", cut.name);
    assert!(cut.name.starts_with(&"n".repeat(60)) && cut.name.ends_with('…'));
    assert!(cut.description.len() <= 300);
    assert!(cut.description.ends_with('…'));
    assert_eq!(
        cut.description.trim_end_matches('…'),
        "é".repeat(148),
        "cut between characters, never inside one"
    );
    let named = &found.skills[1];
    assert_eq!(named.name, "named");
    assert_eq!(named.path, ".cineris/skills/x\\u{202e}y/SKILL.md");
    for skill in &found.skills {
        for text in [&skill.name, &skill.description, &skill.path] {
            assert!(
                !text.chars().any(|c| c.is_control()
                    || (('\u{2000}'..='\u{2FFF}').contains(&c) && c != '…')),
                "{text:?}"
            );
        }
    }
}

#[test]
fn one_name_is_one_skill_the_projects_first_then_cineris_agents_claude() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "one_name_is_one_skill_the_projects_first_then_cineris_agents_claude",
    );
    skill(
        &scratch,
        "home/proj/.claude/skills/a",
        "shared",
        "project claude",
    );
    skill(
        &scratch,
        "home/proj/.agents/skills/b",
        "shared",
        "project agents",
    );
    skill(&scratch, "home/.cineris/skills/c", "shared", "user cineris");
    skill(&scratch, "home/.claude/skills/d", "both", "user claude");
    skill(&scratch, "home/.agents/skills/e", "both", "user agents");
    skill(
        &scratch,
        "home/proj/.claude/skills/f",
        "top",
        "project claude",
    );
    skill(
        &scratch,
        "home/proj/.cineris/skills/g",
        "top",
        "project cineris",
    );
    // The roots given in the opposite order: which is taken first is the
    // owner's and the layout's to say, not the order of the list.
    let reversed: Vec<Root> = Root::standard(true).into_iter().rev().collect();
    let found = find(&scratch, &reversed);
    let pairs: Vec<(&str, &str)> = found
        .skills
        .iter()
        .map(|skill| (skill.name.as_str(), skill.description.as_str()))
        .collect();
    assert_eq!(
        pairs,
        [
            ("both", "user agents"),
            ("shared", "project agents"),
            ("top", "project cineris"),
        ]
    );
    let duplicate = |name: &str, kept: &str| Reason::Duplicate {
        name: name.to_owned(),
        kept: kept.to_owned(),
    };
    assert_eq!(
        found.left_out,
        [
            LeftOut {
                path: "~/.claude/skills/d/SKILL.md".to_owned(),
                reason: duplicate("both", "~/.agents/skills/e/SKILL.md"),
            },
            LeftOut {
                path: ".claude/skills/a/SKILL.md".to_owned(),
                reason: duplicate("shared", ".agents/skills/b/SKILL.md"),
            },
            LeftOut {
                path: "~/.cineris/skills/c/SKILL.md".to_owned(),
                reason: duplicate("shared", ".agents/skills/b/SKILL.md"),
            },
            LeftOut {
                path: ".claude/skills/f/SKILL.md".to_owned(),
                reason: duplicate("top", ".cineris/skills/g/SKILL.md"),
            },
        ]
    );
}

#[test]
fn past_a_hundred_skills_the_rest_are_counted_and_not_listed() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "past_a_hundred_skills_the_rest_are_counted_and_not_listed",
    );
    for index in 0..103 {
        skill(
            &scratch,
            &format!("home/proj/.cineris/skills/s{index:03}"),
            &format!("s{index:03}"),
            "A skill",
        );
    }
    let found = find(&scratch, &Root::standard(false));
    assert_eq!(found.skills.len(), 100);
    assert_eq!(found.not_listed, 3);
    assert_eq!(found.skills[99].name, "s099");
    let index = skills::render(&found, INDEX_CAP);
    assert_eq!((index.listed, index.not_listed), (100, 3));
    assert!(
        index
            .text
            .ends_with("\n- s099: A skill (.cineris/skills/s099/SKILL.md)\n(3 more skills are not listed, to keep this within its size.)"),
        "{}",
        index.text
    );
}

#[test]
fn a_skill_file_larger_than_64_kib_is_left_out() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_skill_file_larger_than_64_kib_is_left_out",
    );
    let at_most = |extra: usize| {
        let head = "---\nname: big\n---\n";
        let cap = 64 * 1024;
        format!("{head}{}", "x".repeat(cap - head.len() + extra))
    };
    scratch.file("home/proj/.cineris/skills/fits/SKILL.md", at_most(0));
    scratch.file(
        "home/proj/.cineris/skills/over/SKILL.md",
        at_most(1).replace("name: big", "name: bog"),
    );
    let found = find(&scratch, &Root::standard(false));
    assert_eq!(listed(&found), [("big", ".cineris/skills/fits/SKILL.md")]);
    assert_eq!(
        left(&found),
        [(".cineris/skills/over/SKILL.md", &Reason::TooLarge)]
    );
}

#[test]
fn a_root_with_more_than_a_thousand_entries_is_looked_at_only_so_far() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_root_with_more_than_a_thousand_entries_is_looked_at_only_so_far",
    );
    let root = scratch.dir("home/proj/.cineris/skills");
    for index in 0..1_000 {
        std::fs::write(root.join(format!("f{index:04}.md")), "").unwrap();
    }
    let found = find(&scratch, &Root::standard(false));
    assert_eq!(left(&found), [], "a thousand entries are all looked at");
    scratch.file("home/proj/.cineris/skills/one-more.md", "");
    let found = find(&scratch, &Root::standard(false));
    assert_eq!(left(&found), [(".cineris/skills", &Reason::TooManyEntries)]);
}

/// A skill made in memory, for the rendering.
fn made(name: &str, description: &str, path: &str) -> Skill {
    Skill {
        name: name.to_owned(),
        description: description.to_owned(),
        path: path.to_owned(),
        owner: Owner::Project,
        layout: Layout::Cineris,
        folder: AbsPath::parse("/w/.cineris/skills/x", PathStyle::Posix).unwrap(),
    }
}

#[test]
fn the_index_is_a_heading_and_a_line_a_skill() {
    let found = Found {
        skills: vec![
            made(
                "pdf",
                "Fill in and merge PDF forms.",
                ".cineris/skills/pdf/SKILL.md",
            ),
            made("review", "", "~/.claude/skills/review/SKILL.md"),
        ],
        not_listed: 0,
        left_out: Vec::new(),
    };
    let index = skills::render(&found, INDEX_CAP);
    assert_eq!(
        index.text,
        format!(
            "{HEADING}\n- pdf: Fill in and merge PDF forms. (.cineris/skills/pdf/SKILL.md)\n- \
             review (~/.claude/skills/review/SKILL.md)"
        )
    );
    assert_eq!((index.listed, index.not_listed), (2, 0));
    assert!(HEADING.starts_with("Skills: "));
    // The same skills, the same bytes; none, no section.
    assert_eq!(skills::render(&found, INDEX_CAP), index);
    assert_eq!(skills::render(&Found::default(), INDEX_CAP).text, "");
}

#[test]
fn the_index_keeps_within_its_cap_and_counts_what_it_leaves_out() {
    let description = "d".repeat(MAX_DESCRIPTION_BYTES);
    let found = Found {
        skills: (0..MAX_SKILLS)
            .map(|index| {
                made(
                    &format!("skill-{index:03}"),
                    &description,
                    &format!(".cineris/skills/skill-{index:03}/SKILL.md"),
                )
            })
            .collect(),
        not_listed: 7,
        left_out: Vec::new(),
    };
    assert_eq!(INDEX_CAP, 8 * 1024);
    let index = skills::render(&found, INDEX_CAP);
    assert!(index.text.len() <= 8 * 1024, "{}", index.text.len());
    assert!(index.listed > 0 && index.listed < MAX_SKILLS);
    assert_eq!(index.not_listed, 7 + MAX_SKILLS - index.listed);
    assert!(index.text.ends_with(&format!(
        "\n({} more skills are not listed, to keep this within its size.)",
        index.not_listed
    )));
    assert_eq!(index.text.lines().count(), 1 + index.listed + 1);
    // One left out is said in the singular.
    let one = Found {
        skills: vec![made("a", "b", "c/SKILL.md")],
        not_listed: 1,
        left_out: Vec::new(),
    };
    assert!(
        skills::render(&one, INDEX_CAP)
            .text
            .ends_with("\n(1 more skill is not listed, to keep this within its size.)")
    );
    // A cap too small for one line gives nothing, and counts them all.
    let tiny = skills::render(&found, HEADING.len() + 10);
    assert_eq!((tiny.text.as_str(), tiny.listed), ("", 0));
    assert_eq!(tiny.not_listed, MAX_SKILLS + 7);
}

#[cfg(windows)]
#[test]
fn on_windows_a_root_in_another_case_and_with_backslashes_is_the_same_folder() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "on_windows_a_root_in_another_case_and_with_backslashes_is_the_same_folder",
    );
    skill(&scratch, "home/proj/.cineris/skills/fine", "fine", "Found");
    skill(&scratch, "home/.claude/skills/mine", "mine", "Found too");
    let roots = [
        Root::new(Owner::Project, Layout::Cineris, r".CINERIS\Skills"),
        Root::new(Owner::User, Layout::Claude, r"~\.Claude\SKILLS\"),
        // The same folder again, in yet another case: looked at once.
        Root::new(Owner::Project, Layout::Agents, r".\SUB\..\.cineris\SKILLS"),
    ];
    scratch.dir("home/proj/sub");
    let found = find(&scratch, &roots);
    // Shown as the file system spells it.
    assert_eq!(
        listed(&found),
        [
            ("fine", ".cineris/skills/fine/SKILL.md"),
            ("mine", "~/.claude/skills/mine/SKILL.md"),
        ]
    );
    assert_eq!(
        left(&found),
        [(
            r".\SUB\..\.cineris\SKILLS",
            &Reason::SameFolder(".cineris/skills".to_owned())
        )]
    );
}

/// A junction is the folder link any Windows account can make, so the one
/// a repository is likeliest to plant. It is made with `mklink`, which is a
/// process: hence the `sys_` name.
#[cfg(windows)]
#[test]
fn sys_on_windows_a_junction_out_of_a_root_is_left_out_and_one_inside_followed() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "sys_on_windows_a_junction_out_of_a_root_is_left_out_and_one_inside_followed",
    );
    skill(&scratch, "outside/stolen", "stolen", "SECRET-OUTSIDE");
    skill(
        &scratch,
        "home/proj/.cineris/skills/store/real",
        "aliased",
        "Inside",
    );
    let root = scratch
        .join("home")
        .join("proj")
        .join(".cineris")
        .join("skills");
    for (link, target) in [
        (root.join("escape"), scratch.join("outside").join("stolen")),
        (root.join("alias"), root.join("store").join("real")),
    ] {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .expect("run mklink");
        assert!(
            made.status.success(),
            "mklink failed: {}",
            String::from_utf8_lossy(&made.stderr)
        );
    }
    let found = find(&scratch, &Root::standard(false));
    assert_eq!(
        listed(&found),
        [("aliased", ".cineris/skills/alias/SKILL.md")]
    );
    assert_eq!(left(&found), [(".cineris/skills/escape", &Reason::LinkOut)]);
    assert!(!format!("{found:?}").contains("SECRET-OUTSIDE"));
}

/// A root that is not there costs a look at each name on the way to it,
/// each named to the caller first, and nothing is resolved: six roots of
/// which none is there are six looks, and a seventh where the user has a
/// `.claude` with no `skills` in it.
#[test]
fn a_root_that_is_not_there_costs_a_look_a_name_and_no_resolve() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_root_that_is_not_there_costs_a_look_a_name_and_no_resolve",
    );
    scratch.dir("home/.claude/projects");
    let resolver = resolver(&scratch);
    let looks = cineris_testkit::Hold::new(A_NAME_IS_LOOKED_AT).skipping(usize::MAX);
    let resolved = cineris_testkit::Hold::new(A_ROOT_IS_RESOLVED).skipping(usize::MAX);
    let mut seen: Vec<PathBuf> = Vec::new();
    let found = find_seeing(&resolver, &Root::standard(true), &mut |place| {
        // Each a name on the way, looked at for its kind alone.
        let Seen::Way(place) = place else {
            panic!("{place:?} is no name on the way");
        };
        seen.push(place.to_path_buf());
    });
    assert_eq!(found, Found::default());
    let workspace = resolver.root().to_fs_path();
    let home = scratch.join("home");
    assert_eq!(
        seen,
        [
            workspace.join(".cineris"),
            workspace.join(".agents"),
            workspace.join(".claude"),
            home.join(".cineris"),
            home.join(".agents"),
            home.join(".claude"),
            home.join(".claude").join("skills"),
        ]
    );
    assert_eq!(looks.arrivals(), 7);
    assert_eq!(resolved.arrivals(), 0);
}

/// Every place a root's skills are found through is named to the caller
/// before it is looked at or read, where it really is: the names on the
/// way, the root, each entry, the folder a link in it leads to, and each
/// `SKILL.md`; and nothing beyond a link that leads out of the root.
#[test]
fn each_place_is_named_before_it_is_looked_at_or_read() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "each_place_is_named_before_it_is_looked_at_or_read",
    );
    skill(&scratch, "home/proj/.cineris/skills/pdf", "pdf", "Forms.");
    skill(&scratch, "outside/stolen", "stolen", "Not ours.");
    scratch.dir("home/proj/.cineris/skills/empty");
    let root = scratch.join("home/proj/.cineris/skills");
    let linked = make_link(&root.join("pdf"), &root.join("alias"), true)
        && make_link(&scratch.join("outside/stolen"), &root.join("escape"), true);
    let resolver = resolver(&scratch);
    let workspace = resolver.root().to_fs_path();
    let real = workspace.join(".cineris").join("skills");
    let reads = cineris_testkit::Hold::new(A_SKILL_IS_READ).skipping(usize::MAX);
    let mut seen: Vec<PathBuf> = Vec::new();
    // How many `SKILL.md` files had been read as each was named.
    let mut read_then: Vec<usize> = Vec::new();
    let found = find_seeing(&resolver, &Root::standard(false), &mut |place| {
        let (Seen::Way(place) | Seen::Place(place)) = place;
        seen.push(place.to_path_buf());
        if place.ends_with(FILE_NAME) {
            read_then.push(reads.arrivals());
        }
    });
    assert!(!found.skills.is_empty(), "{found:?}");
    assert!(!read_then.is_empty());
    for (named, read) in read_then.iter().enumerate() {
        assert_eq!(*read, named, "a SKILL.md is named before it is read");
    }
    let at = |place: &Path| {
        seen.iter()
            .position(|named| named == place)
            .unwrap_or_else(|| panic!("{} was not named: {seen:#?}", place.display()))
    };
    assert!(at(&workspace.join(".cineris")) < at(&real));
    assert!(at(&real) < at(&real.join("pdf")));
    assert!(at(&real.join("pdf")) < at(&real.join("pdf").join(FILE_NAME)));
    let _ = at(&real.join("empty"));
    if linked {
        // The link itself, and where it leads, which is inside the root.
        let _ = at(&real.join("alias"));
        let _ = at(&real.join("escape"));
        assert!(
            !seen
                .iter()
                .any(|place| place.starts_with(scratch.join("outside"))),
            "{seen:#?}"
        );
    }
}

/// A root reached through a link where nothing is yet (the review of step
/// 28's sections): where it would be is named, so that a root made there
/// later is seen, whether the link leads to a folder with no `skills` in
/// it, or to nothing.
#[test]
fn a_root_through_a_link_is_named_where_it_would_be() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_root_through_a_link_is_named_where_it_would_be",
    );
    let dotfiles = scratch.dir("home/dotfiles/claude");
    let home = scratch.join("home");
    if !make_link(&dotfiles, &home.join(".claude"), true)
        || !make_link(
            &scratch.join("home/dotfiles/agents"),
            &home.join(".agents"),
            true,
        )
    {
        return;
    }
    let resolver = resolver(&scratch);
    let mut ways: Vec<PathBuf> = Vec::new();
    let found = find_seeing(&resolver, &Root::standard(true), &mut |place| {
        if let Seen::Way(place) = place {
            ways.push(place.to_path_buf());
        }
    });
    assert!(found.skills.is_empty(), "{found:?}");
    let real = |path: PathBuf| {
        let parent = std::fs::canonicalize(path.parent().unwrap()).unwrap();
        parent.join(path.file_name().unwrap())
    };
    let would_be = real(dotfiles.join("skills"));
    assert!(
        ways.iter()
            .any(|way| cineris_base::same_location(way, &would_be)),
        "{would_be:?} is not named: {ways:#?}"
    );
    // The link to nothing: where it leads.
    let leads = scratch.join("home/dotfiles/agents");
    assert!(
        ways.iter().any(|way| way == &leads),
        "{leads:?} is not named: {ways:#?}"
    );
    assert_eq!(
        found.left_out,
        [LeftOut {
            path: "~/.agents/skills".to_owned(),
            reason: Reason::Dangling
        }]
    );
}

/// A root reads at most 8 MiB of `SKILL.md` files, every byte read
/// counting: the skills past it are left out, unread, with the reason.
#[test]
fn a_root_reads_at_most_its_bytes_of_skills() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "a_root_reads_at_most_its_bytes_of_skills",
    );
    // 130 skills of a little under 64 KiB each: about 8.1 MiB.
    let padding = "x".repeat(64 * 1024 - 200);
    for n in 0..130 {
        scratch.file(
            format!("home/proj/.cineris/skills/s{n:03}/SKILL.md"),
            format!("---\nname: s{n:03}\ndescription: One.\n---\n{padding}\n"),
        );
    }
    let found = find(&scratch, &Root::standard(false));
    let past: Vec<&LeftOut> = found
        .left_out
        .iter()
        .filter(|left| left.reason == Reason::PastTheRootsBytes)
        .collect();
    assert!(!past.is_empty(), "{:?}", found.left_out);
    let read = found.skills.len() + found.not_listed;
    assert_eq!(read + past.len(), 130);
    let size = (padding.len() + 40) as u64;
    assert!((read as u64) * size <= MAX_ROOT_BYTES, "{read}");
    assert!(((read + 1) as u64) * size > MAX_ROOT_BYTES - size, "{read}");
    // By name: the first ones were read, the last ones left out.
    assert!(
        past.iter()
            .all(|left| left.path.as_str() > ".cineris/skills/s099/SKILL.md"),
        "{past:?}"
    );
}
