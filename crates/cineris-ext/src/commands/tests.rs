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
/// and the project's is said not used, naming the user's.
#[test]
fn the_users_command_wins_over_a_projects_of_the_same_name() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "commands-the_users_command_wins_over_a_projects_of_the_same_name",
    );
    scratch.file(format!("{USER}/review.md"), "The user's review: {{args}}");
    scratch.file(format!("{PROJECT}/Review.md"), "The project's review.");
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
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].0, ".cineris/commands/Review.md");
    assert!(
        left[0].1.contains("a command of this name (`review`)"),
        "{left:?}"
    );
}

/// Bounds as the skills' have them: a file over its cap by its length,
/// names that differ only in case, a name the registry refuses, and the
/// entries of a root past its cap, each left out with its reason.
#[test]
fn a_folder_of_commands_is_read_within_its_bounds() {
    let scratch = Scratch::outside(
        "cineris-ext-unit",
        "commands-a_folder_of_commands_is_read_within_its_bounds",
    );
    scratch.file(
        format!("{USER}/big.md"),
        "x".repeat(usize::try_from(MAX_FILE_BYTES).unwrap() + 1),
    );
    scratch.file(format!("{USER}/Deploy.md"), "One.");
    scratch.file(format!("{USER}/deploy.md"), "Two.");
    scratch.file(format!("{USER}/two words.md"), "Bad name.");
    scratch.file(format!("{USER}/bytes.md"), [0xff_u8, 0xfe]);
    for index in 0..MAX_ENTRIES {
        scratch.file(format!("{PROJECT}/c{index:03}.md"), "Fine.");
    }
    let found = found(&scratch);
    assert!(found.get("big").is_none() && found.get("deploy").is_none());
    let lefts = left(&found);
    let reason = |path: &str| {
        lefts
            .iter()
            .filter(|(at, _)| at.ends_with(path))
            .map(|(_, why)| why.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(reason("big.md"), ["it is larger than 64 KiB"]);
    assert!(
        reason("deploy.md")[0].contains("`Deploy`, `deploy` have names that differ only in case")
    );
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

/// No link is followed inside a folder of commands, and a project's folder
/// that is a link out of the workspace is not read.
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
    // The project's whole folder a link out of the workspace.
    std::fs::remove_dir_all(scratch.join(PROJECT)).unwrap();
    assert!(linked("home/elsewhere", PROJECT));
    let found = self::found(&scratch);
    assert!(found.commands.is_empty());
    assert_eq!(left(&found)[0].1, Reason::OutsideWorkspace.to_string());
}
