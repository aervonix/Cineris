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
//! | `package.json` | `npm run <name>` for each script whose name says a build, a test run or a check, with what the script runs |
//! | `Makefile` | `make <target>` for the targets `build`, `test`, `check` and `lint` |
//! | `go.mod` | `go build ./...`, `go test ./...` and `go vet ./...` |
//! | `pyproject.toml` | `pytest`, `ruff check` and `mypy .`, each when the file has the tool's table |
//!
//! **A command is offered only as an entry the user writes would hold
//! it:** a line of plain words ([`normal_line`]) that the command
//! observer classes as a build, a test run or a check ([`classify`]). A
//! name taken from a file (a script's, a target's) that is not plain words
//! is not offered, so nothing a repository writes there reaches the
//! screen as a command; what a script runs is given beside it as text, for
//! the interface to make safe to show.

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

/// The most characters of what a script runs that are given.
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
    /// Offers `command` when it is plain words the observer classes, and
    /// not offered already.
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
            runs: runs.map(|runs| runs.chars().take(MAX_RUNS).collect()),
            source: source.to_owned(),
            purpose: purpose.to_wire(),
        });
    }
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
        let Value::String(runs) = runs else {
            continue;
        };
        if name.is_empty() || name.contains(char::is_whitespace) {
            continue;
        }
        // `npm test` is `npm run test`: one line for each.
        found.offer(name, &format!("npm run {name}"), Some(runs), "package.json");
    }
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

    #[test]
    fn what_a_script_runs_is_cut_and_the_scripts_are_bounded() {
        let long = "x".repeat(MAX_RUNS * 3);
        let mut scripts: Vec<String> = (0..40)
            .map(|n| format!("\"test:unit{n}\": \"a\""))
            .collect();
        scripts.push(format!("\"test\": \"{long}\""));
        let text = format!("{{\"scripts\": {{{}}}}}", scripts.join(","));
        let commands = found(&[("package.json", &text)]);
        assert_eq!(lines(&commands), ["npm run test"], "only known variants");
        assert_eq!(
            commands[0].runs.as_ref().map(|runs| runs.chars().count()),
            Some(MAX_RUNS)
        );
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
