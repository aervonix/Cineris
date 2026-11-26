//! The subcommands, through the real binary: configuration, credentials,
//! models, and a project trusted through `permissions trust` (step 13a's
//! second carried-over point: the pin covers every rule of the project, so
//! the next start reads it as trusted).

use std::io::Write;
use std::process::Stdio;

use serde_json::Value;

use crate::{MODEL, Machine, OTHER_MODEL, Run, jsonl, shown, shown_hash};

/// The model a headless run's session started on, and its standard error.
fn model_of_a_run(machine: &Machine) -> (String, String) {
    machine.reply_text("Done.");
    let run = machine.run(&["-p", "go", "--output", "jsonl"]);
    run.expect_code(0);
    let lines: Vec<Value> = jsonl(&run.stdout);
    (lines[0]["model"].as_str().unwrap().to_owned(), run.stderr)
}

fn trust_line(machine: &Machine) -> String {
    let list = machine.run(&["permissions", "list"]);
    list.expect_code(0);
    list.stdout.lines().next().unwrap().to_owned()
}

#[test]
fn sys_a_project_trusted_with_permissions_trust_is_trusted_on_the_next_start() {
    let machine = Machine::new("sys_trust");
    // A dearer model, which waits for trust, and rules of all three
    // actions: the pin covers each of them.
    let rules = "[[permissions.rules]]\neffect = \"exec\"\nmatch = \"make test\"\naction = \
                 \"allow\"\n\n[[permissions.rules]]\neffect = \"exec\"\nmatch = \"make \
                 deploy\"\naction = \"deny\"\n\n[[permissions.rules]]\neffect = \"read\"\nmatch \
                 = \"secret/**\"\naction = \"ask\"\n";
    machine.project_config(&format!("[model]\nmain = \"{OTHER_MODEL}\"\n\n{rules}"));

    assert!(trust_line(&machine).ends_with(": not trusted"));
    let (model, stderr) = model_of_a_run(&machine);
    assert_eq!(model, MODEL, "an untrusted project's model waits");
    assert!(
        stderr.contains("model.main"),
        "what waits is said: {stderr}"
    );

    // --trust-project trusts it for one run, and pins nothing.
    machine.reply_text("Done.");
    let once = machine.run(&["-p", "go", "--trust-project", "--output", "jsonl"]);
    assert_eq!(jsonl(&once.expect_code(0).stdout)[0]["model"], OTHER_MODEL);
    assert!(trust_line(&machine).ends_with(": not trusted"));

    let trusted = machine.run(&["permissions", "trust"]);
    trusted.expect_code(0);
    for expected in [
        "allow exec make test",
        "deny exec make deploy",
        "ask read secret/**",
        "model.main",
    ] {
        assert!(
            trusted.stdout.contains(expected),
            "{expected}: {}",
            shown(&trusted)
        );
    }
    assert!(trust_line(&machine).ends_with(": trusted"));
    let (model, stderr) = model_of_a_run(&machine);
    assert_eq!(model, OTHER_MODEL, "trusted on the next start: {stderr}");
    assert!(!stderr.contains("rules have changed"), "{stderr}");

    // A rule the user has not seen sets the project's settings aside
    // again, until the change is approved.
    machine.project_config(&format!(
        "[model]\nmain = \"{OTHER_MODEL}\"\n\n{rules}\n[[permissions.rules]]\neffect = \
         \"exec\"\nmatch = \"curl *\"\naction = \"allow\"\n"
    ));
    assert!(trust_line(&machine).contains("rules have changed"));
    assert_eq!(model_of_a_run(&machine).0, MODEL);
    let approved = machine.run(&["permissions", "trust"]);
    assert!(
        approved.expect_code(0).stdout.starts_with("Approved"),
        "{}",
        shown(&approved)
    );
    assert_eq!(model_of_a_run(&machine).0, OTHER_MODEL);

    // Run headless from a folder below the root: the root's trust and
    // configuration apply, the workspace is the root, and commands start
    // in the folder, which the model is told (the follow-ups of step 23).
    machine.reply_text("Done.");
    let below = machine.workspace.join("src");
    let run = machine.run_in(&below, &["-p", "go", "--output", "jsonl"]);
    let start = &jsonl(&run.expect_code(0).stdout)[0];
    assert_eq!(
        start["model"],
        OTHER_MODEL,
        "the root's trust: {}",
        shown(&run)
    );
    assert!(
        std::path::Path::new(start["workspace"].as_str().unwrap())
            .ends_with(machine.workspace.file_name().unwrap()),
        "{start}"
    );
    let sent = String::from_utf8(machine.server.requests().last().unwrap().body.clone()).unwrap();
    assert!(sent.contains("Commands start in: src"), "{sent}");

    machine.run(&["permissions", "untrust"]).expect_code(0);
    assert!(trust_line(&machine).ends_with(": not trusted"));
    assert_eq!(model_of_a_run(&machine).0, MODEL);
}

#[test]
fn sys_config_shows_and_checks_what_a_session_would_load() {
    let machine = Machine::new("sys_config");
    let show = machine.run(&["config", "show"]);
    assert!(
        show.expect_code(0)
            .stdout
            .contains(&format!("model.main = \"{MODEL}\"")),
        "{}",
        shown(&show)
    );
    let origin = machine.run(&["config", "show", "--origin"]);
    assert!(
        origin.expect_code(0).stdout.contains("# user: "),
        "{}",
        shown(&origin)
    );
    let layer = machine.run(&["config", "show", "--layer", "user"]);
    assert!(layer.expect_code(0).stdout.contains("retry.attempts = 1"));
    machine
        .run(&["config", "show", "--layer", "nowhere"])
        .expect_code(2);
    let defaults = machine.run(&["config", "defaults"]);
    assert!(
        defaults
            .expect_code(0)
            .stdout
            .contains("config_version = 1")
    );
    machine.run(&["config", "check"]).expect_code(0);

    // Run in a folder below the project's root, the project's own file is
    // read: the root is found by walking up (the follow-ups of step 23).
    machine.project_config("[retry]\nattempts = 2\n");
    let below = machine.run_in(
        &machine.workspace.join("src").join("deep"),
        &["config", "show", "--layer", "project"],
    );
    assert!(
        below.expect_code(0).stdout.contains("attempts = 2"),
        "{}",
        shown(&below)
    );

    // An unknown key is a warning to a run, and an error to `check`.
    machine.user_config("model.mian = \"x\"\n");
    let shown_anyway = machine.run(&["config", "show"]);
    assert!(shown_anyway.expect_code(0).stderr.contains("mian"));
    let checked = machine.run(&["config", "check"]);
    assert!(
        checked.expect_code(2).stderr.contains("mian"),
        "{}",
        shown(&checked)
    );
}

#[test]
fn sys_a_key_stored_with_auth_login_is_used_and_never_shown() {
    let machine = Machine::new("sys_auth");
    let status = machine.run(&["auth", "status"]);
    assert!(
        status
            .expect_code(0)
            .stdout
            .contains("anthropic: key from the environment variable ANTHROPIC_API_KEY"),
        "{}",
        shown(&status)
    );
    // Nothing on standard input is no key.
    machine.run(&["auth", "login", "anthropic"]).expect_code(2);
    // A provider nobody knows is refused, not stored under a typo.
    let typo = machine.run(&["auth", "logout", "anthropc"]);
    assert!(
        typo.expect_code(2).stderr.contains("anthropic"),
        "{}",
        shown(&typo)
    );

    let stored = "sk-ant-stored-1111";
    let mut login = machine
        .command(&["auth", "login", "anthropic"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    login
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{stored}\n").as_bytes())
        .unwrap();
    let login = Run::of(login.wait_with_output().unwrap());
    login.expect_code(0);

    let without_variable = |arguments: &[&str]| {
        Run::of(
            machine
                .command(arguments)
                .env_remove("ANTHROPIC_API_KEY")
                .output()
                .unwrap(),
        )
    };
    let status = without_variable(&["auth", "status"]);
    assert!(
        status
            .expect_code(0)
            .stdout
            .contains("anthropic: key from the")
    );
    machine.reply_text("Done.");
    let run = without_variable(&["-p", "go"]);
    run.expect_code(0);
    assert_eq!(
        machine.server.requests()[0].header("x-api-key"),
        Some(stored)
    );
    for output in [&login, &status, &run] {
        assert!(!output.stdout.contains(stored) && !output.stderr.contains(stored));
    }

    let logout = machine.run(&["auth", "logout", "anthropic"]);
    assert!(logout.expect_code(0).stdout.starts_with("Removed"));
    let status = without_variable(&["auth", "status"]);
    assert!(status.expect_code(0).stdout.contains("anthropic: no key"));
}

#[test]
fn sys_models_lists_the_catalog() {
    let machine = Machine::new("sys_models");
    let models = machine.run(&["models"]);
    let out = &models.expect_code(0).stdout;
    assert!(out.contains(MODEL) && out.contains(OTHER_MODEL), "{out}");
}

/// `cineris project`, through the real binary: an entry is written, listed
/// with its command in full, the repository's entries are accepted with
/// one command, an entry is turned off and deleted, and a mistake is said.
#[test]
fn sys_project_writes_lists_accepts_and_turns_off_entries() {
    let machine = Machine::new("sys_project");
    std::fs::create_dir_all(machine.workspace.join(".cineris")).unwrap();
    std::fs::write(
        machine.workspace.join(".cineris").join("profile.toml"),
        "version = 1\n\n[[entry]]\nkind = \"convention\"\ntext = \"Errors are returned.\"\n",
    )
    .unwrap();

    let line = "cargo nextest run --workspace --no-fail-fast --features a,b";
    let added = machine.run(&[
        "project",
        "add",
        "command",
        "Runs the tests.",
        "--purpose",
        "test",
        "--command",
        line,
        "--cwd",
        "crates/app",
    ]);
    // A command the user writes is accepted at once, and still asks until
    // the project is trusted: that is said.
    let out = &added.expect_code(0).stdout;
    assert!(
        out.starts_with("Added k_1\n") && out.contains("an accepted command still asks"),
        "{}",
        shown(&added)
    );
    let added = machine.run(&["project", "add", "fact", "The docs are in docs/."]);
    assert_eq!(
        added.expect_code(0).stdout,
        "Added k_2\n",
        "{}",
        shown(&added)
    );

    // The listing: the repository's entry waits, the user's are in use,
    // and the command's line and directory are whole.
    let list = machine.run(&["project"]);
    let out = &list.expect_code(0).stdout;
    assert!(out.contains("Waiting for your review:\n  s_"), "{out}");
    assert!(
        out.contains("[proposed, shared]  convention: Errors are returned."),
        "{out}"
    );
    assert!(
        out.contains(&format!(
            "  k_1  [active, accepted]  test: `{line}` (in `crates/app`)  Runs the tests.\n      written by you\n"
        )),
        "{out}"
    );
    assert!(
        out.contains("  k_2  [active]  fact: The docs are in docs/."),
        "{out}"
    );

    // One command accepts every entry of the repository's file.
    let accepted = machine.run(&["project", "accept", "--shared"]);
    assert!(
        accepted.expect_code(0).stdout.starts_with("Accepted s_"),
        "{}",
        shown(&accepted)
    );
    let again = machine.run(&["project", "accept", "--shared"]);
    assert_eq!(
        again.expect_code(0).stdout,
        "No entry of the repository's file waits for you.\n"
    );

    // `--json` prints the listing's own choice of entries.
    let commands = machine.run(&["project", "list", "--kind", "command", "--json"]);
    let listed: Value = serde_json::from_str(commands.expect_code(0).stdout.trim()).unwrap();
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1, "{listed}");
    assert_eq!(listed["entries"][0]["id"], "k_1");

    // What is turned off is listed only with --all.
    let off = machine.run(&["project", "disable", "k_2"]);
    assert_eq!(
        off.expect_code(0).stdout,
        "Turned off k_2\n",
        "{}",
        shown(&off)
    );
    let all = machine.run(&["project", "list", "--all", "--json"]);
    let listed: Value = serde_json::from_str(all.expect_code(0).stdout.trim()).unwrap();
    let statuses: Vec<(&str, &str)> = listed["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["kind"].as_str().unwrap(),
                entry["status"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        [
            ("command", "active"),
            ("fact", "disabled"),
            ("convention", "active")
        ]
    );
    let deleted = machine.run(&["project", "delete", "k_2"]);
    assert_eq!(deleted.expect_code(0).stdout, "Deleted k_2\n");

    // A mistake is said, and is a usage error.
    let wrong = machine.run(&["project", "accept", "k_7"]);
    assert!(
        wrong
            .expect_code(2)
            .stderr
            .contains("no entry of this project has the id 'k_7'"),
        "{}",
        shown(&wrong)
    );
    let paths = machine.run(&["project", "path"]);
    let out = &paths.expect_code(0).stdout;
    assert!(
        out.contains("knowledge.toml") && out.contains("profile.toml"),
        "{out}"
    );

    // A repository's file that cannot be read is said, not taken for one
    // with nothing in it.
    std::fs::write(
        machine.workspace.join(".cineris").join("profile.toml"),
        "version = 99\n",
    )
    .unwrap();
    let unread = machine.run(&["project", "accept", "--shared"]);
    assert!(
        unread.expect_code(2).stderr.contains("profile.toml"),
        "{}",
        shown(&unread)
    );
}

/// `permissions trust` shows a project's extensions as the trust question
/// does, and with no terminal to ask at approves them only with
/// `--with-extensions`: without it, it trusts the rules and says the
/// extensions were left unapproved (step 28).
#[test]
fn sys_permissions_trust_approves_a_projects_extensions_only_with_the_flag_where_none_can_ask() {
    let machine = Machine::new("sys_trust_extensions");
    let folder = machine.workspace.join(".cineris/extensions/probe");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"probe\"\nversion = \"1\"\ndescription = \"A test extension\"\nmain = \
         \"main.js\"\n[capabilities]\nexec = [\"gh\"]\n",
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), "// nothing yet\n").unwrap();
    let trust_file = machine.dirs().state.unwrap().join("trust.toml");
    let pinned = || std::fs::read_to_string(&trust_file).unwrap_or_default();

    let plain = machine.run(&["permissions", "trust"]);
    plain.expect_code(0);
    for shown_too in [
        "Its extensions",
        "`probe` in `.cineris/extensions/probe`.",
        "It may run `gh`",
    ] {
        assert!(
            plain.stdout.contains(shown_too),
            "{shown_too}: {}",
            shown(&plain)
        );
    }
    // What it says the programs can do is what a session on this machine
    // would let them do (the fresh review of step 28: the command's core
    // had no launcher, and said they were not confined where sessions
    // confine them).
    let enforcing = matches!(
        cineris_sandbox::support::detect().assess(),
        cineris_sandbox::support::Assessment::Enforcing { .. }
    );
    assert_eq!(
        plain.stdout.contains(cineris_core::ext::NOT_CONFINED),
        !enforcing,
        "{}",
        shown(&plain)
    );
    assert!(
        plain.stderr.contains("left unapproved")
            && plain.stderr.contains("`--extension NAME=HASH`"),
        "{}",
        shown(&plain)
    );
    assert!(!pinned().contains("kind = \"extension\""), "{}", pinned());

    // Named with another hash, nothing is trusted or pinned; named with the
    // hash shown, it is approved (the owner's answer to the early review
    // of step 28's third pull request).
    let hash = shown_hash(&plain.stdout, "`probe` in");
    let wrong = machine.run(&["permissions", "trust", "--extension", "probe=0000"]);
    wrong.expect_code(2);
    assert!(
        wrong.stderr.contains("not the hash it is shown with"),
        "{}",
        shown(&wrong)
    );
    assert!(!pinned().contains("kind = \"extension\""), "{}", pinned());
    let named = format!("probe={hash}");
    let flagged = machine.run(&["permissions", "trust", "--extension", &named]);
    flagged.expect_code(0);
    assert!(
        flagged.stdout.contains("Approved its extensions: `probe`."),
        "{}",
        shown(&flagged)
    );
    let file = pinned();
    assert!(
        file.contains("kind = \"extension\"") && file.contains("name = \"probe\""),
        "{file}"
    );
    // Approved as it is, it is not offered again.
    let again = machine.run(&["permissions", "trust"]);
    again.expect_code(0);
    assert!(
        !again.stdout.contains("Its extensions"),
        "{}",
        shown(&again)
    );
}

/// The subcommands that approve, run by a session's command that the
/// permission engine cannot see them in (a script file), refuse, and write
/// nothing; run from the user's own terminal, as the test runs them, they
/// do not refuse (the owner's answer to the early review of step 28's
/// third pull request, B). On Linux the line of parents reaches the
/// session's process; on Windows the command is in the sessions' job.
#[test]
fn sys_an_approving_subcommand_refuses_under_a_sessions_command() {
    let machine = Machine::new("sys_approving_under_a_session");
    let folder = machine.home.join("src").join("tracker");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"tracker\"\nversion = \"1\"\ndescription = \"A test extension\"\nmain \
         = \"main.js\"\n",
    )
    .unwrap();
    std::fs::write(folder.join("main.js"), "// nothing\n").unwrap();
    let binary = std::path::Path::new(crate::CINERIS);
    let said = machine.workspace.join("said.txt");
    // Each subcommand, one after another, its standard error kept.
    let subcommands: [&[&str]; 12] = [
        &[],
        &["serve", "--stdio"],
        &["attach"],
        &["-p", "--trust-project", "hi"],
        &["ext", "add", &folder.to_string_lossy(), "--hash", "00"],
        &["ext", "remove", "tracker", "--hash", "00"],
        &["ext", "disable", "tracker"],
        &["permissions", "trust"],
        &["permissions", "untrust"],
        &["permissions", "revoke", "a_1"],
        &["permissions", "forget"],
        &["setup", "skill-folders", "on"],
    ];
    let (script, line) = if cfg!(windows) {
        let mut text = String::from("@echo off\r\n");
        for words in subcommands {
            text.push_str(&format!(
                "\"{}\" {} >> said.txt 2>&1\r\n",
                binary.display(),
                words
                    .iter()
                    .map(|word| format!("\"{word}\""))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        ("approve.cmd", (text, "cmd /c .\\approve.cmd"))
    } else {
        let mut text = String::new();
        for words in subcommands {
            text.push_str(&format!(
                "'{}' {} >> said.txt 2>&1\n",
                binary.display(),
                words
                    .iter()
                    .map(|word| format!("'{word}'"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        ("approve.sh", (text, "sh approve.sh"))
    };
    let (text, command) = line;
    std::fs::write(machine.workspace.join(script), text).unwrap();
    machine.reply_tool(
        "toolu_1",
        "shell",
        &serde_json::json!({"command": command, "timeout_ms": 60_000}),
    );
    machine.reply_text("Done.");
    let run = machine.run(&[
        "-p",
        "approve it",
        "--permission-mode",
        "unrestricted",
        "--output",
        "json",
    ]);
    run.expect_code(0);
    let said = std::fs::read_to_string(&said).unwrap_or_default();
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
    ] {
        assert!(said.contains(what), "{what}: {said}\n{}", shown(&run));
    }
    assert!(said.contains("run it in your own terminal"), "{said}");
    let state = machine.dirs().state.unwrap();
    assert!(!state.join("pins.toml").exists(), "{said}");
    assert!(!state.join("trust.toml").exists(), "{said}");
    let config = machine.dirs().config.unwrap();
    assert!(
        !config.join("extensions").join("tracker").exists(),
        "{said}"
    );

    // From a shell the test starts, under the test runner's own jobs on
    // Windows and its own line of parents on Linux, none a session's, it is
    // not refused either.
    #[cfg(windows)]
    let shell = {
        use std::os::windows::process::CommandExt as _;
        // `cmd` reads its own line: given as it is, not quoted again.
        std::process::Command::new("cmd")
            .arg("/c")
            .raw_arg(format!(
                "\"\"{}\" ext add \"{}\" --hash 00\"",
                binary.display(),
                folder.display()
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
                    folder.display()
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
    // From the test's own process, which no session started, it is not
    // refused: the hash given is checked, and is not the one shown.
    let own = machine.run(&["ext", "add", &folder.to_string_lossy(), "--hash", "00"]);
    own.expect_code(2);
    assert!(!own.stderr.contains("was refused"), "{}", shown(&own));
    assert!(
        own.stderr.contains("is not the hash of what is shown"),
        "{}",
        shown(&own)
    );
}

/// With no terminal to ask at, `ext add`, `remove` and `disable` act only
/// on the hash of what they show (the owner's answer to the early review
/// of step 28's third pull request): `add` shows what the script
/// registers, evaluated before its question, and approves only with the
/// snapshot's hash; `disable` of one that holds a gate asks first, and
/// turns it off only with its pinned version's hash; `remove` unpins and
/// deletes on that hash, and does neither without it.
#[test]
fn sys_ext_add_disable_and_remove_act_only_on_the_hash_shown() {
    let machine = Machine::new("sys_ext_by_hash");
    let folder = machine.home.join("src").join("guard");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        "api = 1\nname = \"guard\"\nversion = \"1\"\ndescription = \"Guards.\"\nmain = \
         \"main.js\"\n[capabilities]\nhooks = [\"tool.before\"]\n",
    )
    .unwrap();
    std::fs::write(
        folder.join("main.js"),
        "cineris.tool({ name: 'look', description: 'Looks around.', input: { type: 'object' }, \
         async run() { return {}; } });\n\
         cineris.on('tool.before', { tools: ['shell'] }, async () => ({ decision: 'allow' }));\n",
    )
    .unwrap();
    let path = folder.to_string_lossy().into_owned();
    let pins = machine.dirs().state.unwrap().join("pins.toml");
    let pinned = || std::fs::read_to_string(&pins).unwrap_or_default();
    let kept = machine
        .dirs()
        .config
        .unwrap()
        .join("extensions")
        .join("guard");
    let hash_of = |text: &str, before: &str| -> String {
        let at = text
            .find(before)
            .unwrap_or_else(|| panic!("{before}: {text}"));
        text[at + before.len()..]
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect()
    };

    // Shown, evaluated, and not approved without the hash.
    let plain = machine.run(&["ext", "add", &path]);
    plain.expect_code(2);
    assert!(
        plain.stdout.contains("Its tools:") && plain.stdout.contains("\"Looks around.\""),
        "{}",
        shown(&plain)
    );
    let hash = hash_of(&plain.stdout, "(its hash is ");
    assert!(
        plain.stderr.contains(&format!("`--hash {hash}`")),
        "{}",
        shown(&plain)
    );
    assert!(
        !kept.exists() && !pinned().contains("guard"),
        "{}",
        pinned()
    );
    let added = machine.run(&["ext", "add", &path, "--hash", &hash]);
    added.expect_code(0);
    assert!(
        kept.exists() && pinned().contains("guard"),
        "{}",
        shown(&added)
    );

    // Its gate is named, and it is not turned off without the hash.
    let config = machine.dirs().user_config.unwrap();
    let before = std::fs::read_to_string(&config).unwrap_or_default();
    let asked = machine.run(&["ext", "disable", "guard"]);
    asked.expect_code(2);
    assert!(
        asked.stdout.contains("holds gates") && asked.stdout.contains("tool.before on shell"),
        "{}",
        shown(&asked)
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap_or_default(), before);
    let wrong = machine.run(&["ext", "disable", "guard", "--hash", "00"]);
    wrong.expect_code(2);
    assert_eq!(std::fs::read_to_string(&config).unwrap_or_default(), before);
    let off = machine.run(&["ext", "disable", "guard", "--hash", &hash]);
    off.expect_code(0);
    assert!(
        std::fs::read_to_string(&config)
            .unwrap()
            .contains("enabled = false"),
        "{}",
        shown(&off)
    );

    // Not removed at all without the hash; unpinned and deleted with it.
    let kept_pin = machine.run(&["ext", "remove", "guard"]);
    kept_pin.expect_code(2);
    assert!(
        kept.exists() && pinned().contains("guard"),
        "{}",
        shown(&kept_pin)
    );
    let removed = machine.run(&["ext", "remove", "guard", "--hash", &hash]);
    removed.expect_code(0);
    assert!(!kept.exists(), "{}", shown(&removed));
    assert!(!pinned().contains("guard"), "{}", pinned());
}

/// A headless run a session's command starts is held to `read-only`,
/// whatever its configuration says, and says so on standard error and in
/// its result (the maintainers' condition on B). The nested run's own
/// model calls are the scripted server's second and third answers; it is
/// given the key, which a session's commands are not.
#[test]
fn sys_a_nested_headless_run_is_held_to_read_only_and_says_so() {
    let machine = Machine::with_config(
        "sys_nested_read_only",
        "[permissions]\nmode = \"unrestricted\"\n",
    );
    let binary = std::path::Path::new(crate::CINERIS);
    let (script, command) = if cfg!(windows) {
        (
            "nested.cmd",
            format!(
                "@echo off\r\nset ANTHROPIC_API_KEY={}\r\n\"{}\" -p hi --output json > \
                 nested.json 2> nested.err\r\n",
                crate::KEY,
                binary.display()
            ),
        )
    } else {
        (
            "nested.sh",
            format!(
                "ANTHROPIC_API_KEY='{}' '{}' -p hi --output json > nested.json 2> nested.err\n",
                crate::KEY,
                binary.display()
            ),
        )
    };
    std::fs::write(machine.workspace.join(script), command).unwrap();
    let line = if cfg!(windows) {
        "cmd /c .\\nested.cmd"
    } else {
        "sh nested.sh"
    };
    machine.reply_tool(
        "toolu_1",
        "shell",
        &serde_json::json!({"command": line, "timeout_ms": 60_000}),
    );
    // The nested run's: a write, which `read-only` refuses where its
    // configuration's `unrestricted` would let it, then its answer; then
    // the outer run's last.
    machine.reply_tool(
        "toolu_2",
        "write",
        &serde_json::json!({"path": "nested.txt", "content": "x\n"}),
    );
    machine.reply_text("Nested.");
    machine.reply_text("Done.");
    let run = machine.run(&["-p", "nest", "--output", "json"]);
    run.expect_code(0);
    let said = std::fs::read_to_string(machine.workspace.join("nested.err")).unwrap_or_default();
    assert!(
        said.contains("held to `read-only`") && said.contains("a-nested-run-under-a-session"),
        "{said}\n{}",
        shown(&run)
    );
    let result: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(machine.workspace.join("nested.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["capped"]["level"], "read-only", "{result}");
    assert_eq!(result["files_changed"], serde_json::json!([]), "{result}");
    assert!(!machine.workspace.join("nested.txt").exists(), "{result}");
    assert_eq!(
        result["capped"]["item"], "a-nested-run-under-a-session-is-capped-at-read-only",
        "{result}"
    );
    // Run from the user's own terminal, it is not held: the same write is
    // made.
    machine.reply_tool(
        "toolu_3",
        "write",
        &serde_json::json!({"path": "nested.txt", "content": "x\n"}),
    );
    machine.reply_text("Plain.");
    let own = machine.run(&["-p", "hi", "--output", "json"]);
    let record = own.expect_code(0).json();
    assert!(record.get("capped").is_none(), "{record}");
    assert!(machine.workspace.join("nested.txt").exists(), "{record}");
}

/// A harness whose mark of its sessions' commands failed (on Windows, a job
/// it could not make or join; here the test hooks' switch) does not run at
/// full width: each part is a front end, with a machine of its own whose
/// configuration says `unrestricted` (the coordinator's condition on the
/// full review's fixes of the third part).
#[test]
fn sys_a_harness_whose_mark_failed_holds_its_sessions_at_supervised_and_says_so() {
    parts![
        a_headless_run_whose_mark_failed_is_held_at_supervised_and_says_so,
        crate::serve::a_server_whose_mark_failed_refuses_a_raise_past_supervised,
        crate::tui::the_application_whose_mark_failed_says_so,
    ];
}

/// A headless run whose mark failed is held at `supervised`: a write past
/// the workspace, which its configuration's `unrestricted` would make, is
/// refused where nobody can be asked; and it says so on standard error and
/// in its result's `capped`, with no open item.
fn a_headless_run_whose_mark_failed_is_held_at_supervised_and_says_so() {
    let mut machine = Machine::with_config(
        "sys_mark_failed_headless",
        "[permissions]\nmode = \"unrestricted\"\n",
    );
    machine.fail_the_mark();
    let outside = machine.home.join("outside.txt");
    machine.reply_tool(
        "toolu_1",
        "write",
        &serde_json::json!({"path": outside.to_str().unwrap(), "content": "x\n"}),
    );
    machine.reply_text("Done.");
    let run = machine.run(&["-p", "write it", "--output", "json"]);
    let result = run.expect_code(0).json();
    assert!(!outside.exists(), "{result}");
    assert_eq!(result["capped"]["level"], "supervised", "{result}");
    assert!(result["capped"].get("item").is_none(), "{result}");
    let reason = result["capped"]["reason"].as_str().unwrap_or_default();
    assert!(reason.contains("could not make a job"), "{result}");
    assert!(
        run.stderr
            .contains("holds its sessions at `supervised` at most"),
        "{}",
        shown(&run)
    );
}

/// A headless run a session's command starts with a variable that moves
/// where Cineris's configuration and approvals are read from is refused,
/// and reads nothing from there (the re-look at the early review, B).
#[test]
fn sys_a_nested_headless_run_with_its_stores_moved_is_refused() {
    let machine = Machine::new("sys_nested_moved");
    let binary = std::path::Path::new(crate::CINERIS);
    let (script, line) = if cfg!(windows) {
        (
            "moved.cmd",
            format!(
                "@echo off\r\nset CINERIS_CONFIG=evil.toml\r\n\"{}\" -p hi > moved.out 2> moved.err\r\n",
                binary.display()
            ),
        )
    } else {
        (
            "moved.sh",
            format!(
                "CINERIS_CONFIG=evil.toml '{}' -p hi > moved.out 2> moved.err\n",
                binary.display()
            ),
        )
    };
    std::fs::write(machine.workspace.join(script), line).unwrap();
    let command = if cfg!(windows) {
        "cmd /c .\\moved.cmd"
    } else {
        "sh moved.sh"
    };
    machine.reply_tool(
        "toolu_1",
        "shell",
        &serde_json::json!({"command": command, "timeout_ms": 60_000}),
    );
    machine.reply_text("Done.");
    machine
        .run(&[
            "-p",
            "nest",
            "--permission-mode",
            "unrestricted",
            "--output",
            "json",
        ])
        .expect_code(0);
    let said = std::fs::read_to_string(machine.workspace.join("moved.err")).unwrap_or_default();
    assert!(
        said.contains("CINERIS_CONFIG is set otherwise") && said.contains("was refused"),
        "{said}"
    );
}
