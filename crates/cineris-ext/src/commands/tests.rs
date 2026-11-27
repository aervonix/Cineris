#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "a test prepares real files, fails by panicking, and says when it skips"
)]

use cineris_base::PathOptions;
use cineris_testkit::scratch::Scratch;

use super::*;

/// A session's resolver: the workspace `home/proj`, the home `home`.
fn resolver(scratch: &Scratch) -> PathResolver {
    let workspace = scratch.dir("home/proj");
    let home = AbsPath::from_os(&scratch.join("home")).unwrap();
    PathResolver::new(&workspace, PathOptions::new().home(home)).unwrap()
}

/// The two roots: the user's under `home/.config/cineris`, the project's.
fn roots(scratch: &Scratch) -> Vec<Root> {
    vec![
        Root::project(),
        Root::user(&scratch.join("home/.config/cineris")),
    ]
}

const USER: &str = "home/.config/cineris/commands";
const PROJECT: &str = "home/proj/.cineris/commands";

fn found(scratch: &Scratch) -> Found {
    find(&resolver(scratch), &roots(scratch))
}

fn parsed(text: &str) -> Result<CommandFile, String> {
    parse("review", Owner::User, "~/x/review.md".to_owned(), text)
}

fn left(found: &Found) -> Vec<(String, String)> {
    found
        .left_out
        .iter()
        .map(|left| (left.path.clone(), left.reason.to_string()))
        .collect()
}

/// The front matter is read strictly: only its three keys, each once, every
/// line a field; and a body that puts in only what it names.
#[test]
fn a_commands_file_is_read_strictly() {
    let command = parsed(
        "---\ndescription: Review the staged changes\narguments: [focus, depth = \"shallow\"]\n\
         profile: review\n---\nReview, focusing on {{focus | \"correctness\"}} at {{depth}}.\n",
    )
    .unwrap();
    assert_eq!(command.description, "Review the staged changes");
    assert_eq!(
        command.arguments,
        vec![
            Argument {
                name: "focus".to_owned(),
                default: None
            },
            Argument {
                name: "depth".to_owned(),
                default: Some("shallow".to_owned())
            },
        ]
    );
    assert_eq!(command.profile.as_deref(), Some("review"));
    // No front matter: the whole file is the body.
    assert_eq!(
        parsed("Just do it: {{args}}")
            .unwrap()
            .expand("now")
            .unwrap(),
        "Just do it: now"
    );
    for (text, why) in [
        (
            "---\nmodel: x\n---\nhi",
            "the key `model`, which a command does not take",
        ),
        (
            "---\ndescription: a\ndescription: b\n---\nhi",
            "gives `description` twice",
        ),
        ("---\njunk\n---\nhi", "no `key: value` field: `junk`"),
        ("---\ndescription: a\nhi", "never closed"),
        (
            "---\narguments: [x]\n---\n{{y}}",
            "puts in `{{y}}`, which its front matter's",
        ),
        ("hi {{ not one }}", "which is no placeholder"),
        ("---\narguments: [args]\n---\n{{args}}", "name `args`"),
        ("---\narguments: [a, a]\n---\n{{a}}", "name `a` twice"),
        (
            "---\narguments: [a = x]\n---\n{{a}}",
            "not in double quotes",
        ),
        ("---\nprofile: two words\n---\nhi", "no profile's name"),
        ("---\ndescription: a\u{202e}b\n---\nhi", "acts unseen"),
        ("---\ndescription: a\n---\n  \n", "its body is empty"),
    ] {
        let error = parsed(text).unwrap_err();
        assert!(error.contains(why), "{text:?}: {error}");
    }
    // `\{{` is a `{{` of the text.
    assert_eq!(
        parsed("Keep \\{{ .Name }} as it is: {{args}}")
            .unwrap()
            .expand("x")
            .unwrap(),
        "Keep {{ .Name }} as it is: x"
    );
}

/// A key written indented is not passed over (the full review of step 28,
/// its finding 12): an indented mapping, valid YAML, and a key indented by
/// a tab after a field, which was folded into that field's value, are each
/// refused, as the same `profile:` at the first column is read and the
/// command refused for naming one. A value that wraps onto a line of its
/// own still reads, and a quoted value or a block may hold `key: ` text.
#[test]
fn a_key_written_indented_is_not_passed_over() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "commands-a_key_written_indented_is_not_passed_over",
    );
    // A mapping indented two spaces: valid YAML.
    scratch.file(
        format!("{PROJECT}/deploy.md"),
        "---\n  description: Deploy to staging\n  profile: careful\n---\nDeploy the app.\n",
    );
    // A key indented by a tab after a field: folded into that field.
    scratch.file(
        format!("{PROJECT}/ship.md"),
        "---\ndescription: Ship it\n\tprofile: careful\n---\nShip it.\n",
    );
    // At the first column it is read, so the command is judged and not sent.
    scratch.file(
        format!("{PROJECT}/plain.md"),
        "---\nprofile: careful\n---\nDo it.\n",
    );
    let found = found(&scratch);
    assert_eq!(
        found.get("plain").and_then(|c| c.profile.as_deref()),
        Some("careful")
    );
    for name in ["deploy", "ship"] {
        assert!(found.get(name).is_none(), "{name}: {found:#?}");
    }
    let left = left(&found);
    for (path, line) in [
        (
            ".cineris/commands/deploy.md",
            "`  description: Deploy to staging`",
        ),
        (".cineris/commands/ship.md", "`\\tprofile: careful`"),
    ] {
        let (_, why) = left
            .iter()
            .find(|(left, _)| left == path)
            .unwrap_or_else(|| panic!("{path}: {left:?}"));
        assert!(
            why.contains("no `key: value` field") && why.contains(line),
            "{path}: {why}"
        );
    }
    // A value over two lines reads; quotes and a block hold text.
    let wrapped = parsed(
        "---\ndescription: Review the\n  staged changes # mine\n\n  # an indented comment\n---\nhi",
    )
    .unwrap();
    assert_eq!(wrapped.description, "Review the staged changes");
    for text in [
        "---\ndescription: \"Ship: it\n  profile: careful\"\n---\nhi",
        "---\ndescription: |\n  profile: careful\n---\nhi",
        "---\n  # an indented comment first\ndescription: d\n---\nhi",
    ] {
        let command = parsed(text).unwrap_or_else(|why| panic!("{text:?}: {why}"));
        assert!(command.profile.is_none(), "{text:?}");
    }
    for text in [
        "---\ndescription:\n  profile: careful\n---\nhi",
        "---\ndescription: a\n  - profile: careful\n---\nhi",
    ] {
        let error = parsed(text).unwrap_err();
        assert!(error.contains("no `key: value` field"), "{text:?}: {error}");
    }
}

/// A project's body, and its arguments' defaults, hold nothing that acts
/// unseen, the first such character named as an escape; a line break, a
/// `\r\n` one too, a tab and an emoji's selector are seen. The user's own
/// are sent as they wrote them.
#[test]
fn a_projects_command_holds_nothing_that_acts_unseen() {
    let project = |text: &str| {
        parse(
            "review",
            Owner::Project,
            ".cineris/commands/review.md".to_owned(),
            text,
        )
    };
    for (text, unseen) in [
        ("Review this\u{202e} file", "\\u{202e}"),
        ("Do as told.\u{e0049}\u{e0047}", "\\u{e0049}"),
        ("Red \u{1b}[31m", "\\u{1b}"),
        ("Over\rwritten", "\\r"),
        ("Fine\r\nthen \u{200b}{{args}}", "\\u{200b}"),
        ("Put {{args | \"a\u{2066}\"}}", "\\u{2066}"),
    ] {
        let error = project(text).unwrap_err();
        assert!(
            error.starts_with(&format!(
                "its body holds `{unseen}`, a character that acts unseen (a control or a format \
                 character): a project's command is sent as your own prompt"
            )),
            "{text:?}: {error}"
        );
        assert!(parsed(text).is_ok(), "the user's own: {text:?}");
    }
    let defaulted = "---\narguments: [focus = \"a\u{feff}b\"]\n---\nOn {{focus}}.";
    let error = project(defaulted).unwrap_err();
    assert!(
        error.starts_with(
            "its `arguments` give `focus` a default that holds `\\u{feff}`, a character that acts \
             unseen"
        ),
        "{error}"
    );
    assert_eq!(
        parsed(defaulted).unwrap().expand("").unwrap(),
        "On a\u{feff}b."
    );
    // What is seen: line breaks of both kinds, a tab, an emoji's selector.
    let seen = project("Lint:\r\n\t{{args}}\nand ship \u{2764}\u{fe0f}\r\n").unwrap();
    assert_eq!(
        seen.expand("all").unwrap(),
        "Lint:\r\n\tall\nand ship \u{2764}\u{fe0f}\r\n"
    );
}

/// What is typed after the name is given to the arguments: by position, by
/// `name=value`, in quotes; a default from the placeholder over the front
/// matter's; and what does not fit is refused with a reason.
#[test]
fn arguments_are_put_in_and_what_does_not_fit_is_refused() {
    let command = parsed(
        "---\narguments: [focus, depth = \"shallow\"]\n---\n\
         focus={{focus | \"correctness\"}} depth={{depth}} all=[{{args}}]",
    )
    .unwrap();
    let expand = |typed: &str| command.expand(typed);
    assert_eq!(
        expand("").unwrap(),
        "focus=correctness depth=shallow all=[]"
    );
    assert_eq!(
        expand("security deep").unwrap(),
        "focus=security depth=deep all=[security deep]"
    );
    assert_eq!(
        expand("depth=deep").unwrap(),
        "focus=correctness depth=deep all=[depth=deep]"
    );
    assert_eq!(
        expand("\"two words\" focus2=x").unwrap(),
        "focus=two words depth=focus2=x all=[\"two words\" focus2=x]"
    );
    // What is put in is not read again.
    assert_eq!(
        expand("{{depth}}").unwrap(),
        "focus={{depth}} depth=shallow all=[{{depth}}]"
    );
    // Extra words go to `{{args}}` alone.
    assert!(expand("a b c").is_ok());
    for (typed, why) in [
        ("\"open", "never closed"),
        ("focus=a focus=b", "given twice"),
    ] {
        assert!(expand(typed).unwrap_err().0.contains(why), "{typed}");
    }
    // A body without `{{args}}` takes no more words than it has arguments.
    let one = parsed("---\narguments: [focus]\n---\nOn {{focus}}.").unwrap();
    let error = one.expand("a b").unwrap_err().0;
    assert!(
        error.contains("takes one argument, `focus`, and was given 1 more: `b`"),
        "{error}"
    );
    // An argument put in with no default anywhere is needed.
    let error = one.expand("").unwrap_err().0;
    assert!(error.contains("needs the argument `focus`"), "{error}");
    // A body of `{{args}}` alone needs something typed.
    let error = parsed("{{args}}").unwrap().expand("  ").unwrap_err().0;
    assert!(error.contains("gave nothing to send"), "{error}");
    // Bounded: what is typed, and what it expands to.
    assert!(
        one.expand(&"x".repeat(MAX_TYPED_BYTES + 1))
            .unwrap_err()
            .0
            .contains("was typed")
    );
    let many = parsed(&"{{args}}".repeat(64)).unwrap();
    let error = many.expand(&"y".repeat(8 * 1024)).unwrap_err().0;
    assert!(error.contains("would send more than 256 KiB"), "{error}");
    // A script command's arguments: by name or position, the rest kept.
    let assigned = assign(&["env"], "staging --fast env2=x").unwrap();
    assert_eq!(assigned.get("env"), Some("staging"));
    assert_eq!(assigned.rest, ["--fast", "env2=x"]);
}

/// The user's command of a name is used over a project's (names folded),
/// and the project's is said not used, naming the user's; a file of the
/// user's that cannot be used still holds its name.
#[test]
fn the_users_command_wins_over_a_projects_of_the_same_name() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "commands-the_users_command_wins_over_a_projects_of_the_same_name",
    );
    scratch.file(format!("{USER}/review.md"), "The user's review: {{args}}");
    scratch.file(format!("{USER}/deploy.md"), [0xff_u8, 0xfe]);
    scratch.file(format!("{PROJECT}/Review.md"), "The project's review.");
    scratch.file(format!("{PROJECT}/deploy.md"), "The project's deploy.");
    scratch.file(format!("{PROJECT}/lint.md"), "Lint.");
    scratch.file(format!("{PROJECT}/notes.txt"), "Not a command.");
    let found = found(&scratch);
    let names: Vec<(&str, Owner)> = found
        .commands
        .iter()
        .map(|command| (command.name.as_str(), command.owner))
        .collect();
    assert_eq!(names, [("lint", Owner::Project), ("review", Owner::User)]);
    assert_eq!(
        found.get("review").unwrap().expand("now").unwrap(),
        "The user's review: now"
    );
    assert_eq!(
        found.get("review").unwrap().path,
        "~/.config/cineris/commands/review.md"
    );
    assert_eq!(found.get("lint").unwrap().path, ".cineris/commands/lint.md");
    let left = left(&found);
    assert_eq!(left.len(), 3, "{left:?}");
    assert_eq!(
        left[0],
        (
            "~/.config/cineris/commands/deploy.md".to_owned(),
            Reason::NotText.to_string()
        )
    );
    assert_eq!(left[1].0, ".cineris/commands/Review.md");
    assert!(
        left[1].1.contains("a command of this name (`review`)"),
        "{left:?}"
    );
    // The user's `deploy.md` is not used, and still holds its name.
    assert_eq!(
        left[2],
        (
            ".cineris/commands/deploy.md".to_owned(),
            Reason::TakenByUser("deploy".to_owned()).to_string()
        )
    );
}

/// When a folder of the user's could not be read whole, no project's
/// command is used, each saying why: its name could not be checked against
/// the user's. A folder of the user's that is not there has no names.
#[test]
fn no_projects_command_is_used_when_the_users_folder_was_not_read_whole() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "commands-no_projects_command_is_used_when_the_users_folder_was_not_read_whole",
    );
    scratch.file(format!("{PROJECT}/lint.md"), "Lint.");
    // No folder of the user's: the project's is used.
    let found = found(&scratch);
    assert!(found.get("lint").is_some() && found.left_out.is_empty());
    // The project's left out, and the user's folder said, with `why`.
    let refused = |found: &Found, why: &str| {
        assert!(found.get("lint").is_none(), "{why}");
        let left = left(found);
        assert!(
            left.contains(&(
                ".cineris/commands/lint.md".to_owned(),
                Reason::UsersUnread.to_string()
            )),
            "{why}: {left:?}"
        );
        assert!(
            left.iter().any(|(_, reason)| reason.contains(why)),
            "{left:?}"
        );
    };
    // Not a folder.
    scratch.file(USER, "A file.");
    refused(&self::found(&scratch), "it is not a folder");
    // Not readable: the configuration directory is a file.
    std::fs::remove_dir_all(scratch.join("home/.config/cineris")).unwrap();
    scratch.file("home/.config/cineris", "A file.");
    refused(&self::found(&scratch), "it could not be read");
    // More entries than are looked at: the user's are used, and no
    // project's, whose name might be past them.
    std::fs::remove_file(scratch.join("home/.config/cineris")).unwrap();
    for index in 0..=MAX_ENTRIES {
        scratch.file(format!("{USER}/u{index:03}.md"), "Mine.");
    }
    let found = self::found(&scratch);
    refused(&found, "more than 100 entries");
    assert_eq!(found.commands.len(), MAX_ENTRIES);
    // A path that is not Unicode, which as text would name another place.
    #[cfg(unix)]
    let odd = <std::ffi::OsStr as std::os::unix::ffi::OsStrExt>::from_bytes(b"cfg\xff");
    #[cfg(windows)]
    let odd = <std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(&[
        u16::from(b'c'),
        0xD800,
    ]);
    let config = scratch.join("home").join(odd);
    assert!(!Root::user(&scratch.join("home/.config/cineris")).lossy);
    let user = Root::user(&config);
    assert!(user.lossy);
    let found = find(&resolver(&scratch), &[Root::project(), user]);
    refused(
        &found,
        "it could not be read: its path is not valid Unicode",
    );
}

/// Names that differ only in case clash as a folder's listing gives them,
/// whatever the disk can hold: on a disk that keeps one name for both
/// (Windows'), the rule is tested here. Each is given with every name of
/// it, in the listing's order; a name that stands alone is not.
#[test]
fn names_that_differ_only_in_case_clash() {
    let listed = [
        "Deploy",
        "LINT",
        "Lint",
        "deploy",
        "lint",
        "review",
        "two words",
    ]
    .map(str::to_owned);
    assert_eq!(
        case_clashes(&listed).into_iter().collect::<Vec<_>>(),
        [
            ("deploy".to_owned(), vec!["Deploy", "deploy"]),
            ("lint".to_owned(), vec!["LINT", "Lint", "lint"]),
        ]
    );
    assert!(case_clashes(&["deploy".to_owned(), "review".to_owned()]).is_empty());
}

/// Bounds as the skills' have them: a file over its cap by its length,
/// names that differ only in case (where the disk holds both), a name the
/// registry refuses, and the entries of a root past its cap, each left out
/// with its reason.
#[test]
fn a_folder_of_commands_is_read_within_its_bounds() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "commands-a_folder_of_commands_is_read_within_its_bounds",
    );
    scratch.file(
        format!("{USER}/big.md"),
        // 64 KiB and one byte, written out: the cap is 64 KiB, whatever the
        // constant says.
        "x".repeat(64 * 1024 + 1),
    );
    scratch.file(format!("{USER}/Deploy.md"), "One.");
    scratch.file(format!("{USER}/deploy.md"), "Two.");
    // Windows' disk keeps one name for both: the second write went to the
    // first's file.
    let deploys: Vec<String> = scratch
        .listing(USER)
        .into_iter()
        .filter(|name| name.eq_ignore_ascii_case("deploy.md"))
        .collect();
    scratch.file(format!("{USER}/two words.md"), "Bad name.");
    scratch.file(format!("{USER}/bytes.md"), [0xff_u8, 0xfe]);
    for index in 0..MAX_ENTRIES {
        scratch.file(format!("{PROJECT}/c{index:03}.md"), "Fine.");
    }
    let found = found(&scratch);
    assert!(found.get("big").is_none());
    let lefts = left(&found);
    let reason = |path: &str| {
        lefts
            .iter()
            .filter(|(at, _)| at.ends_with(path))
            .map(|(_, why)| why.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(reason("big.md"), ["it is larger than 64 KiB"]);
    if deploys.len() == 2 {
        assert!(found.get("Deploy").is_none() && found.get("deploy").is_none());
        assert!(
            reason("deploy.md")[0]
                .contains("`Deploy`, `deploy` have names that differ only in case")
        );
    } else {
        // Only the clash is skipped: `names_that_differ_only_in_case_clash`
        // tests it on names, and the one file there is read.
        assert!(
            cfg!(windows) || std::env::var_os("GITHUB_ACTIONS").is_none(),
            "the CI runner's disk keeps one name for `Deploy.md` and `deploy.md`: {deploys:?}"
        );
        println!(
            "skipped: this disk keeps one name for `Deploy.md` and `deploy.md` ({deploys:?}), so \
             their clash is not looked for in a folder"
        );
        assert_eq!(deploys.len(), 1, "{deploys:?}");
        let name = command_name(&deploys[0]).unwrap();
        assert_eq!(found.get(name).unwrap().expand("").unwrap(), "Two.");
        assert!(reason(&deploys[0]).is_empty(), "{lefts:?}");
    }
    assert!(reason("two words.md")[0].contains("cannot be a name"));
    assert_eq!(reason("bytes.md"), ["it is not UTF-8 text"]);
    // The project's root: 100 entries looked at, the rest not, and said.
    assert_eq!(
        found
            .commands
            .iter()
            .filter(|command| command.owner == Owner::Project)
            .count(),
        MAX_ENTRIES
    );
    scratch.file(format!("{PROJECT}/zzz.md"), "One too many.");
    let found = self::found(&scratch);
    assert!(
        left(&found)
            .iter()
            .any(|(at, why)| at == ".cineris/commands" && why.contains("more than 100 entries"))
    );
}

/// No link is followed inside a folder of commands, a user's link still
/// holds its name, and a project's folder that is a link out of the
/// workspace is not read.
#[test]
fn no_link_is_followed_in_a_folder_of_commands() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "commands-no_link_is_followed_in_a_folder_of_commands",
    );
    scratch.file("home/elsewhere/secret.md", "Text from outside.");
    scratch.dir(PROJECT);
    let linked = |target: &str, link: &str| {
        let made = {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(scratch.join(target), scratch.join(link))
            }
            #[cfg(windows)]
            {
                if target.ends_with(".md") {
                    std::os::windows::fs::symlink_file(scratch.join(target), scratch.join(link))
                } else {
                    std::os::windows::fs::symlink_dir(scratch.join(target), scratch.join(link))
                }
            }
        };
        match made {
            Ok(()) => true,
            Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
                println!("skipped: cannot make a link here ({error})");
                false
            }
            Err(error) => panic!("the CI runner cannot make a link: {error}"),
        }
    };
    if !linked("home/elsewhere/secret.md", &format!("{PROJECT}/secret.md")) {
        return;
    }
    let found = found(&scratch);
    assert!(found.get("secret").is_none());
    assert_eq!(
        left(&found),
        [(
            ".cineris/commands/secret.md".to_owned(),
            Reason::Link.to_string()
        )]
    );
    // A link of the user's is not followed either, and still holds its
    // name from a project's command.
    scratch.file(format!("{PROJECT}/deploy.md"), "The project's deploy.");
    scratch.dir(USER);
    assert!(linked(
        "home/elsewhere/secret.md",
        &format!("{USER}/deploy.md")
    ));
    let found = self::found(&scratch);
    assert!(found.get("deploy").is_none());
    assert_eq!(
        left(&found),
        [
            (
                "~/.config/cineris/commands/deploy.md".to_owned(),
                Reason::Link.to_string()
            ),
            (
                ".cineris/commands/secret.md".to_owned(),
                Reason::Link.to_string()
            ),
            (
                ".cineris/commands/deploy.md".to_owned(),
                Reason::TakenByUser("deploy".to_owned()).to_string()
            ),
        ]
    );
    std::fs::remove_dir_all(scratch.join(USER)).unwrap();
    // The project's whole folder a link out of the workspace.
    std::fs::remove_dir_all(scratch.join(PROJECT)).unwrap();
    assert!(linked("home/elsewhere", PROJECT));
    let found = self::found(&scratch);
    assert!(found.commands.is_empty());
    assert_eq!(left(&found)[0].1, Reason::OutsideWorkspace.to_string());
}
