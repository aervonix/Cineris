use cineris_core::approving::{CASES, ShellSyntax, refused as engine_refuses};

use super::*;
use crate::args::{SUBCOMMANDS, Topic};

fn words(line: &[&str]) -> Vec<String> {
    line.iter().map(|word| (*word).to_owned()).collect()
}

/// The command line `cineris WORDS`, each word quoted for a POSIX shell.
fn line_of(arguments: &[&str]) -> String {
    let mut line = String::from("cineris");
    for word in arguments {
        line.push_str(&format!(" '{}'", word.replace('\'', "'\\''")));
    }
    line
}

/// Whether the engine, given this grammar, refuses `cineris WORDS`.
fn engine_refused(arguments: &[&str]) -> bool {
    let own = own_commands();
    engine_refuses(&line_of(arguments), ShellSyntax::Posix, Some(&own)).is_some()
}

/// One example of each variant of [`Command`] and of its subcommands, and
/// whether it is refused: the engine refuses exactly when [`standing`]
/// does (the early review's M3).
#[test]
fn each_command_is_refused_by_the_engine_exactly_when_its_standing_says() {
    let examples: &[(&[&str], bool)] = &[
        (&["--help"], false),
        (&["ext", "--help"], false),
        (&["--version"], false),
        (&[], true),
        (&["fix the bug"], true),
        (&["-p", "hi"], false),
        (&["sessions", "list"], false),
        (&["config", "show"], false),
        (&["config", "check"], false),
        (&["config", "defaults"], false),
        (&["permissions", "list"], false),
        (&["permissions", "revoke", "a_1"], true),
        (&["permissions", "trust"], true),
        (&["permissions", "untrust"], true),
        (&["permissions", "forget"], true),
        (&["auth", "status"], false),
        (&["auth", "login", "anthropic"], true),
        (&["auth", "logout", "anthropic"], true),
        (&["models"], false),
        (&["usage"], false),
        (&["stats"], false),
        (&["stats", "reset"], true),
        (&["project", "list"], false),
        (&["project", "path"], false),
        (&["project", "accept", "p_1"], true),
        (&["project", "accept", "--shared"], true),
        (&["project", "disable", "p_1"], true),
        (&["project", "delete", "p_1"], true),
        (&["project", "promote", "p_1"], true),
        (&["project", "edit", "p_1", "new text"], true),
        (&["project", "add", "fact", "run everything"], true),
        (&["ext", "list"], false),
        (&["ext", "inspect", "tracker"], false),
        (&["ext", "disable", "tracker"], true),
        (&["ext", "remove", "tracker"], true),
        (&["ext", "add", "./tracker"], true),
        (&["setup", "skill-folders", "on"], true),
        (&["serve", "--stdio"], true),
        (&["attach"], true),
    ];
    for (arguments, refused_too) in examples {
        let command = parse(arguments.iter().map(OsString::from).collect())
            .unwrap_or_else(|error| panic!("{arguments:?}: {}", error.0));
        let standing = standing(&command);
        assert_eq!(
            standing != Standing::Safe,
            *refused_too,
            "{arguments:?}: {standing:?}"
        );
        assert_eq!(engine_refused(arguments), *refused_too, "{arguments:?}");
    }
    // Every variant has its example: a new one fails to build here until it
    // is given its choice in `standing`, and its example above.
    for (arguments, _) in examples {
        match parse(arguments.iter().map(OsString::from).collect()).unwrap() {
            Command::Help(_)
            | Command::Version
            | Command::Run(_)
            | Command::Sessions(SessionsCommand::List { .. })
            | Command::Config(
                ConfigCommand::Show { .. } | ConfigCommand::Check { .. } | ConfigCommand::Defaults,
            )
            | Command::Permissions(
                PermissionsCommand::List
                | PermissionsCommand::Revoke { .. }
                | PermissionsCommand::Trust { .. }
                | PermissionsCommand::Untrust
                | PermissionsCommand::Forget,
            )
            | Command::Auth(
                AuthCommand::Status | AuthCommand::Login { .. } | AuthCommand::Logout { .. },
            )
            | Command::Models
            | Command::Usage(_)
            | Command::Stats(StatsCommand::Show { .. } | StatsCommand::Reset)
            | Command::Project(
                ProjectCommand::List { .. }
                | ProjectCommand::Path
                | ProjectCommand::Change { .. }
                | ProjectCommand::Edit { .. }
                | ProjectCommand::Add { .. },
            )
            | Command::Ext(
                ExtCommand::List
                | ExtCommand::Inspect { .. }
                | ExtCommand::Disable { .. }
                | ExtCommand::Remove { .. }
                | ExtCommand::Add { .. },
            )
            | Command::Setup(SetupCommand::SkillFolders { .. })
            | Command::Serve(_)
            | Command::Attach(_) => {}
        }
    }
    // The internal subcommands, read before the parser, are refused too,
    // as what they are.
    for internal in [SANDBOX, GUARD] {
        assert!(
            matches!(
                grammar(&words(&[internal, "--", "ls"]), false),
                Standing::Refused(Refusal {
                    kind: Kind::Internal,
                    ..
                })
            ),
            "{internal}"
        );
        assert!(engine_refused(&[internal, "--", "ls"]), "{internal}");
    }
}

/// Each flag of a run after `-p`, with an example, stands by its own
/// choice: the engine refuses it exactly when the choice does. A flag
/// added without an example here fails.
#[test]
fn each_flag_after_p_stands_by_its_own_choice() {
    let examples: &[(&str, &[&str])] = &[
        ("-p, --print", &[]),
        ("--output", &["--output", "json"]),
        (
            "--include-deltas",
            &["--output", "jsonl", "--include-deltas"],
        ),
        ("--continue", &["--continue"]),
        ("--resume", &["--resume", "s_1"]),
        ("--fork", &["--fork", "s_1"]),
        ("--no-session", &["--no-session"]),
        ("--fullscreen", &["--fullscreen"]),
        ("--model", &["--model", "m"]),
        ("--effort", &["--effort", "high"]),
        ("--speed", &["--speed", "fast"]),
        ("--permission-mode", &["--permission-mode", "unrestricted"]),
        ("--allow", &["--allow", "exec:*"]),
        ("--deny", &["--deny", "write **"]),
        ("--trust-project", &["--trust-project"]),
        ("--fail-on-denial", &["--fail-on-denial"]),
        ("--max-requests", &["--max-requests", "3"]),
        ("--max-cost", &["--max-cost", "0.10"]),
        ("--max-time", &["--max-time", "90s"]),
        ("--system-append", &["--system-append", "be brief"]),
        ("--profile", &["--profile", "yolo"]),
        ("-c", &["-c", "permissions.mode=unrestricted"]),
        ("--log-level", &["--log-level", "debug"]),
        ("-q, --quiet", &["-q"]),
        ("-v, --verbose", &["-v"]),
        ("-h, --help", &["--help"]),
        ("-V, --version", &["--version"]),
    ];
    // The maintainers' Option A, written out apart from the table's own
    // choices: what a nested headless run may not carry.
    let refused_flags = [
        "--continue",
        "--resume",
        "--fork",
        "--permission-mode",
        "--allow",
        "--trust-project",
        "--profile",
        "-c",
    ];
    for flag in FLAGS {
        let (_, example) = examples
            .iter()
            .find(|(names, _)| *names == flag.names)
            .unwrap_or_else(|| panic!("{} has no example here", flag.names));
        assert_eq!(
            flag.nested != Nested::Safe,
            refused_flags.contains(&flag.names),
            "{}: its choice is not the maintainers'",
            flag.names
        );
        let mut arguments = vec!["-p"];
        arguments.extend_from_slice(example);
        arguments.push("hi");
        let command = parse(arguments.iter().map(OsString::from).collect()).unwrap();
        if let Command::Run(run) = &command {
            let name = flag.names.split(", ").next().unwrap();
            assert!(
                flags_of(run).iter().any(|(given, _)| *given == name),
                "{name} is not told among the run's flags"
            );
        }
        let refused_too = match flag.nested {
            Nested::Safe => false,
            Nested::Refused => true,
            // The example's value widens.
            Nested::Narrowing(_) => true,
        };
        assert_eq!(
            standing(&command) != Standing::Safe,
            refused_too,
            "{}",
            flag.names
        );
        assert_eq!(engine_refused(&arguments), refused_too, "{}", flag.names);
        if let Nested::Narrowing(values) = flag.nested {
            for value in values {
                let narrowed = ["-p", "--permission-mode", value, "hi"];
                assert!(!engine_refused(&narrowed), "{value}");
                let attached = format!("--permission-mode={value}");
                assert!(!engine_refused(&["-p", &attached, "hi"]), "{attached}");
            }
        }
    }
}

/// Where more words follow that only the shell knows, a run is safe only
/// when its subcommand and action are written out and only read.
#[test]
fn what_only_the_shell_knows_is_safe_only_after_a_reading_action() {
    for (written, safe) in [
        (&["ext", "list"][..], true),
        (&["ext", "inspect"], true),
        (&["sessions", "list"], true),
        (&["models"], true),
        (&["--version"], true),
        (&["ext"], false),
        (&["ext", "add"], false),
        (&["permissions"], false),
        (&["stats"], false),
        (&["-p"], false),
        (&[], false),
    ] {
        assert_eq!(
            grammar(&words(written), true) == Standing::Safe,
            safe,
            "{written:?}"
        );
    }
}

/// The one table the fallback is tested over, read with this grammar: the
/// same lines refused, the same allowed (the maintainers' condition).
#[test]
fn the_engines_table_reads_the_same_with_this_grammar() {
    let own = own_commands();
    for (line, shells, refused_too) in CASES {
        for shell in *shells {
            let refusal = engine_refuses(line, *shell, Some(&own));
            assert_eq!(
                refusal.is_some(),
                *refused_too,
                "{line:?} in {shell:?}: {refusal:?}"
            );
        }
    }
}

/// Every subcommand the help names is in the table [`parse`] reads.
#[test]
fn every_subcommand_is_in_the_table_the_parser_reads() {
    for topic in [
        Topic::Sessions,
        Topic::Config,
        Topic::Permissions,
        Topic::Auth,
        Topic::Models,
        Topic::Usage,
        Topic::Stats,
        Topic::Project,
        Topic::Ext,
        Topic::Setup,
        Topic::Serve,
        Topic::Attach,
    ] {
        assert!(
            SUBCOMMANDS.iter().any(|(_, known)| *known == topic),
            "{topic:?}"
        );
        // Exhaustive: a new topic is given a word here.
        match topic {
            Topic::Main
            | Topic::Sessions
            | Topic::Config
            | Topic::Permissions
            | Topic::Auth
            | Topic::Models
            | Topic::Usage
            | Topic::Stats
            | Topic::Project
            | Topic::Ext
            | Topic::Setup
            | Topic::Serve
            | Topic::Attach => {}
        }
    }
}
