//! System tests (tier 3): the real `cineris` binary, run as a process.
//!
//! Test names here start with `sys_`. That prefix is what puts them in
//! nextest's process group and keeps them out of the `fast` profile.
//!
//! A test gets a [`Machine`]: a home directory and a workspace of its own,
//! under the system's temporary directory, and the scripted SSE server
//! standing in for Anthropic's endpoint. The user's configuration points
//! the built-in `anthropic` provider at the server, so the binary speaks
//! the real protocol over a real socket, and the server answers with the
//! recorded fixtures of `cineris-llm` or with a response written here.
//! Nothing of the developer's own machine is read: the binary is given an
//! environment whose home, configuration, state and data are the test's.
//!
//! | Module | What it shows |
//! |---|---|
//! | `names` | `--version`, help and usage errors under both names. |
//! | `runs` | Every exit code, each output format, and that standard output carries only the result. |
//! | `signals` | Ctrl-C, SIGTERM and Ctrl-Break, in a turn and in an extension's running command; the second Ctrl-C; the guard of a killed harness; a run with no console. |
//! | `commands` | The subcommands, and a project trusted through one. |
//! | `guards` | The guards of the harness's own approvals where the review of step 28 found a way past: a harness that is process 1 (Linux), and another harness's command at `serve --socket`. |
//! | `examples` | The three examples under `examples/`, added and run as their READMEs say. |
//! | `live` | The one run against a real model, ignored: `cargo xtask live` starts it. |
//! | `tools` | The real toolbox as a strict variant declares it. |
//! | `sandbox` | The hidden subcommand `__sandbox` as the sandbox launcher, and what it adds to starting a command (Linux). |
//! | `tui` | The interactive application under a pseudo-terminal: a turn with tools, an approval, interruptions, a steer and a queue. |
//! | `ts_client` | A client in TypeScript, typed only by the generated declarations, runs a turn over `serve --stdio` (Node 22.18 or later; `CINERIS_TS_CLIENT`). |

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    missing_docs,
    missing_debug_implementations,
    unreachable_pub,
    reason = "a system test starts processes and prepares files, and fails by panicking; its \
              helpers are shared between the modules of this one binary"
)]

/// Runs `rule`, one of the rules a folded test checks, each with a machine
/// of its own: a rule that fails names itself after the panic's message, so
/// that the test's failure says which rule broke. Folded by story, so that
/// one test's start serves its rules, and none waits on another's state.
pub fn part(name: &str, rule: fn()) {
    struct Named<'a>(&'a str);
    impl Drop for Named<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                eprintln!("the rule that failed: `{}`", self.0);
            }
        }
    }
    let _named = Named(name);
    rule();
}

/// One rule of a folded test that shares a world: its name, and the check.
pub type Rule<W> = (&'static str, fn(&W));

/// Judges every rule of a folded test that shares one world, `world`, each
/// by its name: a rule that fails is named, and does not keep the rules
/// after it from being judged; the test fails at the end, naming each rule
/// that failed.
pub fn judge<W>(world: &W, rules: &[Rule<W>]) {
    let failed: Vec<&str> = rules
        .iter()
        .filter(|(name, rule)| {
            let judged =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rule(world))).is_err();
            if judged {
                eprintln!("the rule that failed: `{name}`");
            }
            judged
        })
        .map(|(name, _)| *name)
        .collect();
    assert!(failed.is_empty(), "the rules that failed: {failed:?}");
}

/// Runs each rule named, a function of this binary, as a [`part`] of one
/// test, in order.
macro_rules! parts {
    ($($rule:path),+ $(,)?) => {
        $(crate::part(stringify!($rule), $rule);)+
    };
}

mod commands;
mod examples;
mod first_run;
mod guards;
#[cfg(target_os = "linux")]
mod hardened;
mod live;
mod names;
mod nested;
mod runs;
#[cfg(target_os = "linux")]
mod sandbox;
mod serve;
mod signals;
mod tools;
mod ts_client;
mod tui;
mod usage;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use cineris_config::{Dirs, Env, Platform};
use cineris_testkit::fixture::{self, Fixture};
use cineris_testkit::{Scratch, ServerReply, SseServer};
use serde_json::{Value, json};

/// The binary Cargo built for this test run.
pub const CINERIS: &str = env!("CARGO_BIN_EXE_cineris");

/// What a test that starts the binary as a process tree of its own gives
/// the tree, so that it can be sent a console event on Windows: the binary
/// itself under its hidden subcommand, which sends the event from inside
/// the binary's own console, as the harness does for its commands. Linux
/// needs none.
pub fn tree_guard() -> Option<cineris_sandbox::tree::Guard> {
    cfg!(windows).then(|| cineris_sandbox::tree::Guard::new(CINERIS, ["__tree-guard"]))
}

/// The model of the test's configuration: one of the built-in catalog.
pub const MODEL: &str = "claude-haiku-5-5";
/// Another model of the built-in catalog.
pub const OTHER_MODEL: &str = "claude-opus-5-5";
/// The key the binary is given. The scripted server takes any.
pub const KEY: &str = "sk-ant-test-0000";

/// The name of the test that turns this binary into a child process
/// (`cineris_testkit::children`).
pub const CHILD_ENTRY: &str = "sys_child_entry";

#[test]
fn sys_child_entry() {
    cineris_testkit::children::run_if_child();
}

/// The entry of this binary in a role (`cineris_testkit::role`): the shell
/// the interactive tests run the binary under. Passes at once otherwise.
#[test]
fn child_entry() {
    tui::child();
}

/// The variables that would let the developer's own set-up in.
pub const LEFT_OUT: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "DEEPSEEK_API_KEY",
    "VENICE_API_KEY",
];

/// A machine of the test's own, and a scripted model.
pub struct Machine {
    pub home: PathBuf,
    pub workspace: PathBuf,
    pub server: SseServer,
    /// The variables the binary is given beside the inherited ones.
    vars: Vec<(String, OsString)>,
    /// Whether the user's configuration leaves the shell to the binary's
    /// own pick ([`Machine::with_product_shell`]).
    product_shell: bool,
    _scratch: Scratch,
}

impl Machine {
    /// A machine whose user configuration names [`MODEL`] and points the
    /// `anthropic` provider at the scripted server, with `extra` after it.
    pub fn new(test: &str) -> Self {
        Self::with_config(test, "")
    }

    pub fn with_config(test: &str, extra: &str) -> Self {
        Self::made(test, extra, false)
    }

    /// As [`new`](Self::new), with the shell left to the binary's own pick
    /// on Windows too (PowerShell 7 on a hosted runner), where every other
    /// machine is given Windows PowerShell ([`user_config`](Self::user_config)):
    /// for the test that keeps the product's own pick covered end to end.
    pub fn with_product_shell(test: &str) -> Self {
        Self::made(test, "", true)
    }

    fn made(test: &str, extra: &str, product_shell: bool) -> Self {
        let _set_up = cineris_testkit::profile::phase("set-up");
        let scratch = Scratch::outside("cineris-cli-system", test);
        let home = scratch.path().join("home");
        let workspace = scratch.path().join("work").join("app");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&workspace).unwrap();
        let path = |parts: &[&str]| {
            let mut path = home.clone();
            path.extend(parts);
            path.into_os_string()
        };
        let vars = vec![
            ("HOME".to_owned(), home.clone().into_os_string()),
            ("USERPROFILE".to_owned(), home.clone().into_os_string()),
            ("XDG_CONFIG_HOME".to_owned(), path(&[".config"])),
            ("XDG_STATE_HOME".to_owned(), path(&[".local", "state"])),
            ("XDG_DATA_HOME".to_owned(), path(&[".local", "share"])),
            ("XDG_CACHE_HOME".to_owned(), path(&[".cache"])),
            ("APPDATA".to_owned(), path(&["AppData", "Roaming"])),
            ("LOCALAPPDATA".to_owned(), path(&["AppData", "Local"])),
            ("ANTHROPIC_API_KEY".to_owned(), OsString::from(KEY)),
        ];
        let machine = Self {
            home,
            workspace,
            server: SseServer::start(),
            vars,
            product_shell,
            _scratch: scratch,
        };
        machine.user_config(extra);
        machine
    }

    /// The variables the binary is given beside the inherited ones.
    pub fn vars(&self) -> &[(String, OsString)] {
        &self.vars
    }

    /// The binary's dialogs and first-run steps ready in a tenth of their
    /// times (`cineris_base::hook::READY_DIVISOR`): for a test that answers
    /// them and does not test the rule's timing, which otherwise waits its
    /// real seconds (0.3 s after a step is shown, a second after the key
    /// that answered the one before). The rule keeps its proportions, and
    /// its own tests are `cineris-tui`'s, on a fake clock; the tests that
    /// type through a dialog keep the real times (`docs/steps/ci-budgets-2/
    /// answered.md`, "The dialogs' times in the system tests").
    pub fn quick_dialogs(&mut self) {
        self.vars.push((
            cineris_base::hook::READY_DIVISOR.to_owned(),
            OsString::from("10"),
        ));
    }

    /// The binary's mark of its sessions' commands made to fail
    /// (`cineris_base::hook::MARK_FAILS`): what a harness does then, which
    /// a runner cannot otherwise bring about.
    pub fn fail_the_mark(&mut self) {
        self.vars.push((
            cineris_base::hook::MARK_FAILS.to_owned(),
            OsString::from("1"),
        ));
    }

    /// A subcommand held just before its core starts
    /// (`cineris_base::hook::HOLD_CORE`), in `folder`: the binary makes
    /// `held` there and waits until the test makes `go`.
    pub fn hold_the_core(&mut self, folder: &std::path::Path) {
        self.vars.push((
            cineris_base::hook::HOLD_CORE.to_owned(),
            folder.as_os_str().to_owned(),
        ));
    }

    /// Where the binary looks, worked out as it works it out.
    pub fn dirs(&self) -> Dirs {
        let mut env = Env::empty(Platform::CURRENT);
        for (name, value) in &self.vars {
            env.set(name, value.to_str().unwrap());
        }
        Dirs::from_env(&env)
    }

    /// Writes the user's configuration file: the machine's own settings as
    /// dotted keys at the top, so that `extra` may add to any table, and
    /// then `extra`.
    ///
    /// On Windows the shell is Windows PowerShell, unless `extra` says
    /// anything of the shell (a `shell.` key or a `[shell]` table) or the
    /// machine was made [`with_product_shell`](Self::with_product_shell):
    /// it starts in about two thirds of PowerShell 7's time on a hosted
    /// runner, and the binary asks no version of it on the side, where it
    /// starts PowerShell 7 once a machine to ask. These tests are not about
    /// which PowerShell runs a command (`cineris-tools`' tests run each
    /// shell).
    ///
    /// On Windows the terminal's brackets are trusted
    /// (`terminal.bracketed_paste = true`), unless `extra` says anything of
    /// the terminal: a pseudo-console that does not pass mode 2004's reply
    /// on would leave the binary weighing bursts of keys, and two prompts
    /// these tests send back to back, or a line and `\r\r`, can come in one
    /// read with a line break inside, a paste that sends nothing (the
    /// review of step 25's second pull request). Elsewhere the virtual
    /// terminal answers the query, which the binary trusts.
    pub fn user_config(&self, extra: &str) {
        let file = self.dirs().user_config.unwrap();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        // Left out when `extra` says anything of the shell: a `[shell]`
        // table after the dotted key would not parse (on Windows only).
        let shell = if cfg!(windows)
            && !self.product_shell
            && !extra.contains("shell.")
            && !extra.contains("[shell")
        {
            "shell.program = \"powershell\"\n"
        } else {
            ""
        };
        let paste = if cfg!(windows) && !extra.contains("terminal") {
            "terminal.bracketed_paste = true\n"
        } else {
            ""
        };
        fs::write(
            file,
            format!(
                "session.durability = \"never\"\ncredentials.store = \"file\"\nmodel.main = \
                 \"{MODEL}\"\nretry.attempts = 1\nproviders.anthropic.base_url = \"{}\"\n{shell}{paste}{extra}",
                self.server.base_url()
            ),
        )
        .unwrap();
    }

    /// Writes the user's configuration file as it is given.
    pub fn raw_user_config(&self, content: &str) {
        fs::write(self.dirs().user_config.unwrap(), content).unwrap();
    }

    /// Writes the project's configuration file.
    pub fn project_config(&self, content: &str) {
        let dir = self.workspace.join(".cineris");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.toml"), content).unwrap();
    }

    /// The binary, ready to run in the workspace with the machine's
    /// environment and nothing on standard input.
    pub fn command(&self, arguments: &[&str]) -> Command {
        self.command_as(Path::new(CINERIS), arguments)
    }

    pub fn command_as(&self, program: &Path, arguments: &[&str]) -> Command {
        // Marked here, where the binary is made ready to start: every test
        // starts what this returns.
        cineris_base::hook::started(program.as_os_str());
        let mut command = Command::new(program);
        command
            .args(arguments)
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (name, _) in std::env::vars_os() {
            let name = name.to_string_lossy();
            // The test kit's record of process starts goes on to the binary,
            // whose own starts are the test's.
            if (name.starts_with("CINERIS_") && name != cineris_base::hook::STARTS)
                || LEFT_OUT.contains(&name.as_ref())
            {
                command.env_remove(name.as_ref());
            }
        }
        for (name, value) in &self.vars {
            command.env(name, value);
        }
        command
    }

    /// Runs the binary to its end.
    pub fn run(&self, arguments: &[&str]) -> Run {
        let _running = cineris_testkit::profile::phase("the binary runs to its end");
        Run::of(self.command(arguments).output().expect("the binary runs"))
    }

    /// Runs the binary to its end in `dir`, a folder of the workspace.
    pub fn run_in(&self, dir: &Path, arguments: &[&str]) -> Run {
        let _running = cineris_testkit::profile::phase("the binary runs to its end");
        fs::create_dir_all(dir).unwrap();
        let mut command = self.command(arguments);
        command.current_dir(dir);
        Run::of(command.output().expect("the binary runs"))
    }

    /// The server's next answer: a recorded Anthropic fixture of
    /// `cineris-llm`.
    pub fn reply_fixture(&self, name: &str) {
        self.reply_fixture_of("anthropic-messages", name);
    }

    /// The server's next answer: a recorded fixture of `cineris-llm`, of
    /// `protocol`.
    pub fn reply_fixture_of(&self, protocol: &str, name: &str) {
        let directory = fixture::directory(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../cineris-llm"),
            protocol,
        )
        .join(name);
        let recorded = Fixture::load(&directory).unwrap();
        let mut reply = ServerReply::status(recorded.status);
        for (header, value) in recorded.headers.iter() {
            reply = reply.header(header, value);
        }
        self.server.push(reply.send(recorded.response));
    }

    /// The server's next answer: the model says `text` and ends its turn.
    pub fn reply_text(&self, text: &str) {
        self.server.push(ServerReply::sse().send(sse(&[
            message_start(),
            json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": text}}),
            json!({"type": "content_block_stop", "index": 0}),
            message_delta("end_turn"),
            json!({"type": "message_stop"}),
        ])));
    }

    /// The server's next answer: the model calls `tool` with `input`.
    pub fn reply_tool(&self, id: &str, tool: &str, input: &Value) {
        self.server.push(ServerReply::sse().send(sse(&[
            message_start(),
            json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "tool_use", "id": id, "name": tool, "input": {}}}),
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
            json!({"type": "content_block_stop", "index": 0}),
            message_delta("tool_use"),
            json!({"type": "message_stop"}),
        ])));
    }
}

pub fn message_start() -> Value {
    json!({"type": "message_start", "message": {"model": MODEL, "id": "msg_test",
        "type": "message", "role": "assistant", "content": [], "stop_reason": null,
        "usage": {"input_tokens": 20, "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0, "output_tokens": 1}}})
}

pub fn message_delta(stop: &str) -> Value {
    json!({"type": "message_delta", "delta": {"stop_reason": stop},
        "usage": {"output_tokens": 12}})
}

/// Events as an Anthropic stream writes them.
pub fn sse(events: &[Value]) -> Vec<u8> {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!(
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        ));
    }
    body.into_bytes()
}

/// What a finished run left.
#[derive(Debug)]
pub struct Run {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn of(output: Output) -> Self {
        Self {
            code: output.status.code(),
            stdout: String::from_utf8(output.stdout).expect("standard output is UTF-8"),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }
    }

    /// Asserts the exit code, showing both streams when it is not.
    pub fn expect_code(&self, code: i32) -> &Self {
        assert_eq!(
            self.code,
            Some(code),
            "stdout:\n{}\nstderr:\n{}",
            self.stdout,
            self.stderr
        );
        self
    }

    /// Standard output as the one JSON object of `--output json`.
    pub fn json(&self) -> Value {
        assert_eq!(
            self.stdout.matches('\n').count(),
            1,
            "json is one line: {}",
            self.stdout
        );
        let value: Value = serde_json::from_str(&self.stdout).expect("standard output is JSON");
        schema_check("HeadlessRecord", &value);
        value
    }
}

/// The generated schema of the protocol, as checked in.
fn schema() -> &'static Value {
    static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        let file = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/protocol/schema/cineris-protocol.schema.json");
        serde_json::from_str(&fs::read_to_string(file).unwrap()).unwrap()
    })
}

/// Fails unless `value` fits the definition `name` of the generated schema.
pub fn schema_check(name: &str, value: &Value) {
    if let Err(problem) = cineris_testkit::schema::check(schema(), name, value) {
        panic!("not a valid {name}: {problem}\n  in {value}");
    }
}

/// Every line of `--output jsonl`, each checked against the generated
/// schema: the first is a `session` record, the last a `result` record, and
/// every one between is an event as `session/event` carries it.
pub fn jsonl(stdout: &str) -> Vec<Value> {
    assert!(stdout.ends_with('\n'), "every line ends: {stdout}");
    let lines: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|error| panic!("{error}: {line}")))
        .collect();
    assert!(lines.len() >= 2, "{stdout}");
    let last = lines.len() - 1;
    for (index, line) in lines.iter().enumerate() {
        if index == 0 || index == last {
            schema_check("HeadlessRecord", line);
        } else {
            schema_check("SessionEvent", line);
        }
    }
    assert_eq!(lines[0]["type"], "session");
    assert_eq!(lines[last]["type"], "result");
    lines
}

/// The hash `permissions trust` shows with the first entry after `after`
/// (`Its hash is H.`): what `--extension NAME=HASH` names it by.
pub fn shown_hash(stdout: &str, after: &str) -> String {
    let from = stdout
        .find(after)
        .unwrap_or_else(|| panic!("{after}: {stdout}"));
    let rest = &stdout[from..];
    let at = rest
        .find("Its hash is ")
        .unwrap_or_else(|| panic!("no hash: {stdout}"));
    rest[at + "Its hash is ".len()..]
        .split('.')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Prints a run's streams when a test that started it fails later.
pub fn shown(run: &Run) -> String {
    format!("stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr)
}
