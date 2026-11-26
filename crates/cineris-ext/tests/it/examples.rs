//! The examples' scripts (`examples/`), run through the engine with a host
//! that only records what it is asked: each form `shell-gate`'s hook is to
//! refuse, and those it is to let through, as its README lists them; and
//! the line `release`'s command gives `git`, and the revision it refuses
//! before git is started. Their whole runs, added as their READMEs say and
//! used in a session, are `cineris-cli`'s system test `examples`; these are
//! the cases that would each cost that test a start.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cineris_base::path::{AbsPath, PathStyle};
use cineris_base::{CancelToken, Clock as _, FakeClock};
use cineris_ext::capability::Capabilities;
use cineris_ext::engine::{CallContext, CallError, ScriptRuntime, Source, Target};
use cineris_ext::folder::Snapshot;
use cineris_ext::host::ExecOutput;
use cineris_ext::manifest::Manifest;
use cineris_testkit::{FakeTimer, block_on};
use serde_json::{Value as Json, json};

use crate::FakeHost;

const TIME: Duration = Duration::from_secs(7);

/// The example in `folder` (from the repository's root), its manifest and
/// its script, as the engine is given them.
fn example(folder: &str) -> Arc<Snapshot> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(folder);
    let files: BTreeMap<String, Vec<u8>> = ["extension.toml", "main.js"]
        .into_iter()
        .map(|file| (file.to_owned(), std::fs::read(root.join(file)).unwrap()))
        .collect();
    Arc::new(Snapshot::from_files(files))
}

/// The example in `folder`, started, with its capabilities and a timer.
struct Started {
    runtime: ScriptRuntime,
    capabilities: Capabilities,
    timer: FakeTimer,
}

impl Started {
    fn of(folder: &str) -> Self {
        let snapshot = example(folder);
        let manifest = Manifest::of(&snapshot).unwrap();
        let workspace = AbsPath::parse("/work/app", PathStyle::Posix).unwrap();
        let capabilities = Capabilities::compile(&manifest.capabilities, &workspace).unwrap();
        let timer = FakeTimer::manual(&FakeClock::new(cineris_base::UtcTime::UNIX_EPOCH));
        let runtime = block_on(ScriptRuntime::start(
            &manifest.name,
            Source::Snapshot(snapshot),
            None,
            &timer,
        ))
        .unwrap();
        Self {
            runtime,
            capabilities,
            timer,
        }
    }

    /// Calls `target` with `input`, on `host`.
    fn call(&self, target: Target, input: Json, host: &FakeHost) -> Result<Json, CallError> {
        let cancel = CancelToken::new();
        let asked = matches!(target, Target::Hook(_)).then(|| self.timer.clock().monotonic());
        block_on(self.runtime.call(
            target,
            input,
            CallContext {
                host,
                capabilities: &self.capabilities,
                time: TIME,
                timer: &self.timer,
                clock: self.timer.clock(),
                cancel: &cancel,
                asked,
            },
        ))
    }
}

/// The forms `shell-gate` refuses (`examples/shell-gate/README.md`, "What
/// it refuses, in these forms"), each with what it shows.
const REFUSED: [&str; 37] = [
    // `rm`, recursive and forced: short options, alone or together.
    "rm -rf build",
    "rm -fr build",
    "rm -Rf build",
    "rm -r -f build",
    "rm -R -f build",
    "rm -rvf build",
    // Long ones, whole or shortened as GNU `rm` takes them.
    "rm --recursive --force build",
    "rm --rec --for build",
    "rm -r --f build",
    "rm --r -f build",
    // In any case: PowerShell's, where `rm` is `Remove-Item`.
    "rm -Recurse -Force build",
    "rm -r -Force build",
    // Anywhere in a command, and in each command of a line.
    "sudo rm -rf /",
    "find . -name '*.o' | xargs rm -rf",
    "env LC_ALL=C rm -rf build",
    "cd out && rm -rf .",
    "make; rm -rf build",
    "make || rm -rf build",
    "echo $(rm -rf .)",
    "(rm -rf build)",
    "/bin/rm -rf build",
    // A line a shell is given to read again.
    "sh -c 'rm -rf build'",
    "bash -c \"rm -rf build\"",
    "bash -lc 'cd x && rm -rf y'",
    "eval 'rm -rf build'",
    "sh -c \"bash -c 'rm -rf build'\"",
    // A forced push: `--force`, `-f` alone or among others, a `+` refspec,
    // `--mirror`.
    "git push --force origin main",
    "git push -f",
    "git push -uf origin main",
    "git fetch origin && git push origin +main",
    "git push --mirror origin",
    // git's own options before `push`, with their values.
    "git --git-dir .git push --force origin main",
    "git --work-tree . --namespace x push -f origin main",
    "git -C repo --attr-source HEAD push --force",
    // A forced refspec given as configuration.
    "git -c remote.origin.push=+refs/heads/main:refs/heads/main push origin",
    // And through a shell given the line.
    "bash -c 'git push --force'",
    // What only looks like one is refused too, as the README says.
    "echo rm -rf",
];

/// What `shell-gate` lets through: everything else, a push with a lease
/// included.
const LET_THROUGH: [&str; 16] = [
    "rm -r build",
    "rm -f build.log",
    "rm build",
    "rm -r -- -f",
    "rm --dir empty",
    "rm --interactive=never -r build",
    "git push origin main",
    "git push --force-with-lease origin main",
    "git push --force-with-lease --force-if-includes origin main",
    "git -c user.name=x push origin main",
    "git --git-dir .git push origin main",
    "git -c remote.origin.push=refs/heads/main push origin",
    "echo released",
    "sh -c 'echo hi'",
    "eval echo hi",
    "",
];

/// `shell-gate`'s hook refuses each form its README lists, with its reason,
/// and has no opinion on anything else: one runtime, asked about every
/// line in turn.
#[test]
fn shell_gate_refuses_the_forms_its_readme_lists_and_nothing_else() {
    let gate = Started::of("examples/shell-gate");
    assert_eq!(gate.runtime.registration().hooks.len(), 1);
    let host = FakeHost::new();
    let answer = |line: &str| {
        let input =
            json!({"event": "tool.before", "tool": "shell", "arguments": {"command": line}});
        gate.call(Target::Hook(0), input, &host).unwrap()
    };
    let mut wrong = Vec::new();
    for line in REFUSED {
        let answered = answer(line);
        let reason = answered["reason"].as_str().unwrap_or_default();
        let named = if line.contains("push") {
            reason.contains("A forced push replaces the remote's history")
        } else {
            reason.contains("deletes a whole tree without asking")
        };
        if answered["decision"] != "deny" || !named {
            wrong.push(format!("not refused as it should be: `{line}`: {answered}"));
        }
    }
    for line in LET_THROUGH {
        let answered = answer(line);
        if !answered.is_null() {
            wrong.push(format!("refused: `{line}`: {answered}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    // It decides from the text alone: it asked the host nothing.
    assert!(host.asked().is_empty(), "{:?}", host.asked());
}

/// `release`'s `/changelog:draft` gives git the line its README's rule
/// names, `--end-of-options` before the revision, and with no revision the
/// whole history; a revision that begins with `-` is refused before git is
/// started, saying it is not one.
#[test]
fn the_changelog_command_runs_its_line_and_refuses_an_option_for_a_revision() {
    let command = Started::of("examples/release/extensions/changelog");
    let mut host = FakeHost::new();
    host.exec_output = ExecOutput {
        code: Some(0),
        stdout: b"abc1234 Fix the outline of an empty file\n".to_vec(),
        ..ExecOutput::default()
    };
    let draft = |since: Option<&str>| {
        let arguments = since.map_or_else(|| json!({}), |since| json!({"since": since}));
        let input = json!({"args": since.unwrap_or_default(), "arguments": arguments});
        command.call(Target::Command("draft".to_owned()), input, &host)
    };
    let prompt = draft(Some("v0.1.0")).unwrap();
    assert!(
        prompt
            .as_str()
            .is_some_and(|prompt| prompt.contains("abc1234 Fix the outline of an empty file")),
        "{prompt}"
    );
    draft(None).unwrap();
    let execs: Vec<String> = host.asked();
    let line = |execs: &[String], at: usize| {
        execs[at]
            .split(" network=")
            .next()
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(execs.len(), 2, "{execs:?}");
    assert_eq!(
        line(&execs, 0),
        "exec git log --no-merges --max-count=200 --oneline --no-decorate --end-of-options \
         v0.1.0..HEAD"
    );
    assert_eq!(
        line(&execs, 1),
        "exec git log --no-merges --max-count=200 --oneline --no-decorate --end-of-options HEAD"
    );
    for option in ["--output=x", "-p", "--format=%B"] {
        let refused = draft(Some(option)).unwrap_err().to_string();
        assert!(
            refused.contains(&format!("`{option}` is not a revision")),
            "{option}: {refused}"
        );
    }
    assert_eq!(host.asked().len(), 2, "git was started for an option");
}
