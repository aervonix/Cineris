//! Cineris run by a session's command: what the harness's own approving
//! subcommands and a nested headless run do there (the owner's answer to
//! the early review of step 28's third pull request, B, and the re-look's
//! conditions). One session's command runs them all, from one script, in
//! one turn of one headless run at `unrestricted`; each rule is read from
//! what its part of the script left, and judged by its own name.

use std::path::{Path, PathBuf};

use crate::{Machine, Run, shown};

/// The machine, the extension folder the approving subcommands name, and
/// the outer run whose one shell command ran the script.
struct Nested {
    machine: Machine,
    folder: PathBuf,
    run: Run,
}

/// The subcommands that approve, trust or answer for the person, each run
/// by the script under the session's command: since the review of step 28
/// (finding 1), `project`'s changes, `auth login` and `logout`, and `stats
/// reset` among them.
fn approving(folder: &Path) -> [Vec<String>; 23] {
    let folder = folder.to_string_lossy().into_owned();
    let words = |line: &[&str]| -> Vec<String> { line.iter().map(|w| (*w).to_owned()).collect() };
    [
        words(&[]),
        words(&["serve", "--stdio"]),
        words(&["attach"]),
        words(&["-p", "--trust-project", "hi"]),
        words(&["ext", "add", &folder, "--hash", "00"]),
        words(&["ext", "remove", "tracker", "--hash", "00"]),
        words(&["ext", "disable", "tracker"]),
        words(&["permissions", "trust"]),
        words(&["permissions", "untrust"]),
        words(&["permissions", "revoke", "a_1"]),
        words(&["permissions", "forget"]),
        words(&["setup", "skill-folders", "on"]),
        words(&["project", "add", "fact", "A fact of the review"]),
        words(&[
            "project",
            "add",
            "command",
            "A check of the review",
            "--purpose",
            "check",
            "--command",
            "touch review-ran.txt",
        ]),
        words(&["project", "accept", "k_1"]),
        words(&["project", "accept", "--shared"]),
        words(&["project", "edit", "k_1", "changed"]),
        words(&["project", "promote", "k_1"]),
        words(&["project", "disable", "k_1"]),
        words(&["project", "delete", "k_1"]),
        words(&["auth", "login", "anthropic"]),
        words(&["auth", "logout", "anthropic"]),
        words(&["stats", "reset"]),
    ]
}

/// The variables that move where Cineris's configuration and approvals are
/// read from, as the subcommands' own check compares them with the
/// session's (`cineris_sandbox::session::moved_stores`): each is moved for
/// one nested headless run of the script, which must be refused naming it
/// (the review of step 28, finding 19: only `CINERIS_CONFIG` was tried, and
/// an entry taken out of the check's list failed no test).
const STORES: &[&str] = if cfg!(windows) {
    &[
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "ProgramData",
        "CINERIS_CONFIG",
        "CINERIS_DATA_DIR",
    ]
} else {
    &[
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_DATA_HOME",
        "CINERIS_CONFIG",
        "CINERIS_DATA_DIR",
    ]
};

impl Nested {
    /// Writes the script and runs the outer headless run, whose model asks
    /// for one shell command, the script: first a nested headless run with
    /// the key (its model's answers are the scripted server's second and
    /// third), then one with a variable that moves the stores, then each
    /// approving subcommand.
    fn run() -> Self {
        let machine = Machine::with_config(
            "sys_cineris_under_a_sessions_command",
            "[permissions]\nmode = \"unrestricted\"\n",
        );
        let folder = machine.home.join("src").join("tracker");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("extension.toml"),
            "api = 1\nname = \"tracker\"\nversion = \"1\"\ndescription = \"A test \
             extension\"\nmain = \"main.js\"\n",
        )
        .unwrap();
        std::fs::write(folder.join("main.js"), "// nothing\n").unwrap();
        let binary = Path::new(crate::CINERIS).display();
        let (script, command) = if cfg!(windows) {
            let mut text = format!(
                "@echo off\r\nset ANTHROPIC_API_KEY={}\r\n\"{binary}\" -p hi --output json > \
                 nested.json 2> nested.err\r\nsetlocal\r\nset CINERIS_CONFIG=evil.toml\r\n\
                 \"{binary}\" -p hi > moved.out 2> moved.err\r\n\"{binary}\" -p > bare.out 2> \
                 bare.err\r\nendlocal\r\n",
                crate::KEY
            );
            for name in STORES {
                let moved = machine.workspace.join("moved").join(name);
                text.push_str(&format!(
                    "setlocal\r\nset {name}={}\r\n\"{binary}\" -p hi > moved-{name}.out 2> \
                     moved-{name}.err\r\nendlocal\r\n",
                    moved.display()
                ));
            }
            for words in approving(&folder) {
                let words: Vec<String> = words.iter().map(|word| format!("\"{word}\"")).collect();
                text.push_str(&format!(
                    "\"{binary}\" {} >> said.txt 2>&1\r\n",
                    words.join(" ")
                ));
            }
            (("all.cmd", text), "cmd /c .\\all.cmd")
        } else {
            // The command's own process, started by a harness that closed
            // itself, is dumpable: its environment is its user's to read.
            let mut text = format!(
                "cat /proc/$$/environ > /dev/null 2>&1 && echo yes > dumpable.txt\n\
                 ANTHROPIC_API_KEY='{}' '{binary}' -p hi --output json > nested.json 2> \
                 nested.err\nCINERIS_CONFIG=evil.toml '{binary}' -p hi > moved.out 2> \
                 moved.err\nCINERIS_CONFIG=evil.toml '{binary}' -p > bare.out 2> bare.err\n",
                crate::KEY
            );
            for name in STORES {
                let moved = machine.workspace.join("moved").join(name);
                text.push_str(&format!(
                    "{name}='{}' '{binary}' -p hi > moved-{name}.out 2> moved-{name}.err\n",
                    moved.display()
                ));
            }
            for words in approving(&folder) {
                let words: Vec<String> = words.iter().map(|word| format!("'{word}'")).collect();
                text.push_str(&format!(
                    "'{binary}' {} >> said.txt 2>&1\n",
                    words.join(" ")
                ));
            }
            (("all.sh", text), "sh all.sh")
        };
        let ((name, text), command) = (script, command);
        std::fs::write(machine.workspace.join(name), text).unwrap();
        machine.reply_tool(
            "toolu_1",
            "shell",
            &serde_json::json!({"command": command, "timeout_ms": 60_000}),
        );
        // The nested run's: a write, which `read-only` refuses where its
        // configuration's `unrestricted` would let it, then its answer;
        // then the outer run's last.
        machine.reply_tool(
            "toolu_2",
            "write",
            &serde_json::json!({"path": "nested.txt", "content": "x\n"}),
        );
        machine.reply_text("Nested.");
        machine.reply_text("Done.");
        let run = machine.run(&["-p", "nest", "--output", "json"]);
        Self {
            machine,
            folder,
            run,
        }
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.machine.workspace.join(name)).unwrap_or_default()
    }
}

/// Cineris run by a session's command: each rule below, judged by its own
/// name, from one script run by one session's command.
#[test]
fn sys_cineris_run_by_a_sessions_command_is_held_or_refused() {
    let world = Nested::run();
    let rules: [crate::Rule<Nested>; 5] = [
        (
            "a_nested_headless_run_is_held_to_read_only_and_says_so",
            a_nested_headless_run_is_held_to_read_only_and_says_so,
        ),
        (
            "a_nested_headless_run_with_its_stores_moved_is_refused",
            a_nested_headless_run_with_its_stores_moved_is_refused,
        ),
        (
            "an_approving_subcommand_refuses_under_a_sessions_command",
            an_approving_subcommand_refuses_under_a_sessions_command,
        ),
        (
            "each_variable_that_moves_the_stores_refuses_a_nested_run",
            each_variable_that_moves_the_stores_refuses_a_nested_run,
        ),
        (
            "a_sessions_command_is_not_closed_as_its_harness_is",
            a_sessions_command_is_not_closed_as_its_harness_is,
        ),
    ];
    crate::judge(&world, &rules);
}

/// The harness closes its own process (`cineris_sandbox::harden`); a
/// command it starts is a new program, dumpable again from its `execve`:
/// on Linux the command reads its own environment, which `/proc` gives only
/// to a dumpable process's user. (Windows's commands are not given the
/// harness's access list: `cineris-sandbox`'s own tests open one.)
fn a_sessions_command_is_not_closed_as_its_harness_is(world: &Nested) {
    if cfg!(windows) {
        return;
    }
    assert_eq!(world.read("dumpable.txt"), "yes\n", "{}", shown(&world.run));
}

/// A headless run a session's command starts is held to `read-only`,
/// whatever its configuration says, and says so on standard error and in
/// its result (the maintainers' condition on B); it is given the key, which
/// a session's commands are not. Run from the user's own terminal, it is not
/// held: the same write is made.
fn a_nested_headless_run_is_held_to_read_only_and_says_so(world: &Nested) {
    world.run.expect_code(0);
    let said = world.read("nested.err");
    assert!(
        said.contains("held to `read-only`") && said.contains("a-nested-run-under-a-session"),
        "{said}\n{}",
        shown(&world.run)
    );
    let result: serde_json::Value = serde_json::from_str(&world.read("nested.json")).unwrap();
    assert_eq!(result["capped"]["level"], "read-only", "{result}");
    assert_eq!(result["files_changed"], serde_json::json!([]), "{result}");
    let written = world.machine.workspace.join("nested.txt");
    assert!(!written.exists(), "{result}");
    assert_eq!(
        result["capped"]["item"], "a-nested-run-under-a-session-is-capped-at-read-only",
        "{result}"
    );
    world.machine.reply_tool(
        "toolu_3",
        "write",
        &serde_json::json!({"path": "nested.txt", "content": "x\n"}),
    );
    world.machine.reply_text("Plain.");
    let own = world.machine.run(&["-p", "hi", "--output", "json"]);
    let record = own.expect_code(0).json();
    assert!(record.get("capped").is_none(), "{record}");
    assert!(written.exists(), "{record}");
}

/// A headless run a session's command starts with a variable that moves
/// where Cineris's configuration and approvals are read from is refused,
/// and reads nothing from there (the re-look at the early review, B).
fn a_nested_headless_run_with_its_stores_moved_is_refused(world: &Nested) {
    let said = world.read("moved.err");
    assert!(
        said.contains("CINERIS_CONFIG is set otherwise") && said.contains("was refused"),
        "{said}\n{}",
        shown(&world.run)
    );
    // The refusal is all it says: it ends before its core starts, which
    // would read the configuration the variable names (its standing is
    // asked beside its start, and waited for before the core: since step
    // 28's third pull request).
    assert_eq!(
        said.lines().filter(|line| !line.trim().is_empty()).count(),
        1,
        "{said}"
    );
    assert!(
        world.read("moved.out").is_empty(),
        "{}",
        world.read("moved.out")
    );
    // With no prompt as well, it says the refusal and not that it has no
    // prompt: what would end it early waits for its standing.
    let bare = world.read("bare.err");
    assert!(
        bare.contains("was refused") && !bare.contains("no prompt"),
        "{bare}"
    );
}

/// Each variable that moves where Cineris's configuration and approvals
/// are read from, set otherwise than for the session for one nested
/// headless run, refuses it, naming the variable, before it reads anything
/// (the review of step 28, finding 19).
fn each_variable_that_moves_the_stores_refuses_a_nested_run(world: &Nested) {
    for name in STORES {
        let said = world.read(&format!("moved-{name}.err"));
        assert!(
            said.contains(&format!("{name} is set otherwise")) && said.contains("was refused"),
            "{name}: {said}\n{}",
            shown(&world.run)
        );
        let out = world.read(&format!("moved-{name}.out"));
        assert!(out.is_empty(), "{name}: {out}");
    }
}

/// The subcommands that approve, run by a session's command that the
/// permission engine cannot see them in (a script file), refuse, and write
/// nothing; run from a shell the test starts, under the test runner's own
/// jobs on Windows and its own line of parents on Linux, none a session's,
/// they do not refuse. On Linux the line of parents reaches the session's
/// process; on Windows the command is in the sessions' job.
fn an_approving_subcommand_refuses_under_a_sessions_command(world: &Nested) {
    let said = world.read("said.txt");
    for what in [
        "`cineris` was refused",
        "`cineris serve` was refused",
        "`cineris attach` was refused",
        "`cineris -p --trust-project` was refused under a session's command",
        "`cineris ext add` was refused",
        "`cineris ext remove` was refused",
        "`cineris ext disable` was refused",
        "`cineris permissions trust` was refused",
        "`cineris permissions untrust` was refused",
        "`cineris permissions revoke` was refused",
        "`cineris permissions forget` was refused",
        "`cineris setup skill-folders` was refused",
        "`cineris project add` was refused",
        "`cineris project accept` was refused",
        "`cineris project edit` was refused",
        "`cineris project promote` was refused",
        "`cineris project disable` was refused",
        "`cineris project delete` was refused",
        "`cineris auth login` was refused",
        "`cineris auth logout` was refused",
        "`cineris stats reset` was refused",
    ] {
        assert!(said.contains(what), "{what}: {said}\n{}", shown(&world.run));
    }
    assert!(said.contains("run it in your own terminal"), "{said}");
    let machine = &world.machine;
    let state = machine.dirs().state.unwrap();
    assert!(!state.join("pins.toml").exists(), "{said}");
    assert!(!state.join("trust.toml").exists(), "{said}");
    let config = machine.dirs().config.unwrap();
    assert!(
        !config.join("extensions").join("tracker").exists(),
        "{said}"
    );
    // `project add` wrote no entry: the person's own listing has none.
    let entries = machine.run(&["project", "list", "--all", "--json"]);
    entries.expect_code(0);
    let listed: serde_json::Value = serde_json::from_str(&entries.stdout).unwrap();
    assert_eq!(listed["entries"], serde_json::json!([]), "{listed}\n{said}");
    let binary = Path::new(crate::CINERIS);
    #[cfg(windows)]
    let shell = {
        use std::os::windows::process::CommandExt as _;
        // `cmd` reads its own line: given as it is, not quoted again.
        std::process::Command::new("cmd")
            .arg("/c")
            .raw_arg(format!(
                "\"\"{}\" ext add \"{}\" --hash 00\"",
                binary.display(),
                world.folder.display()
            ))
            .envs(
                machine
                    .vars()
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_os_str())),
            )
            .current_dir(&machine.workspace)
            .output()
            .unwrap()
    };
    #[cfg(not(windows))]
    let shell = {
        std::process::Command::new("sh")
            .args([
                "-c",
                &format!(
                    "'{}' ext add '{}' --hash 00",
                    binary.display(),
                    world.folder.display()
                ),
            ])
            .envs(
                machine
                    .vars()
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.as_os_str())),
            )
            .current_dir(&machine.workspace)
            .output()
            .unwrap()
    };
    let said_there = String::from_utf8_lossy(&shell.stderr);
    assert!(!said_there.contains("was refused"), "{said_there}");
    assert!(
        said_there.contains("is not the hash of what is shown"),
        "{said_there}"
    );
}
