//! The manifest's path globs are matched against where a path **leads**,
//! however the script wrote it: `~`, a relative path, `..`, a link inside
//! the workspace leading out, a link outside leading in, and on Windows
//! another case.

use std::time::Duration;

use cineris_base::AbsPath;
use cineris_ext::capability::Capabilities;
use cineris_ext::host::{HostCall, HostError, Phase, serve};
use cineris_ext::manifest::Manifest;
use cineris_testkit::{Scratch, block_on};

use crate::{FakeHost, make_link, manifest};

struct Setup {
    scratch: Scratch,
    host: FakeHost,
    capabilities: Capabilities,
}

fn setup(name: &str) -> Setup {
    let scratch = Scratch::for_test(env!("CARGO_TARGET_TMPDIR"), "cineris-ext-it", name);
    scratch.file("ws/src/a.rs", "a");
    scratch.file("ws/out/old.txt", "o");
    scratch.file("ws/notes.md", "n");
    scratch.file("outside/secret.rs", "s");
    scratch.dir("home");
    let host = FakeHost::in_workspace(&scratch.join("ws"), Some(&scratch.join("home")));
    let parsed = Manifest::parse(&manifest("read = [\"src/**\"]\nwrite = [\"out/**\"]")).unwrap();
    let root = host.resolver.as_ref().unwrap().root().clone();
    let capabilities = Capabilities::compile(&parsed.capabilities, &root).unwrap();
    Setup {
        scratch,
        host,
        capabilities,
    }
}

impl Setup {
    fn read(&self, path: &str) -> Result<(), HostError> {
        block_on(serve(
            HostCall::Read {
                path: path.to_owned(),
                offset: 0,
                limit: None,
            },
            &self.capabilities,
            &self.host,
            Phase::Tool,
            Duration::from_secs(1),
        ))
        .map(|_| ())
    }

    fn write(&self, path: &str) -> Result<(), HostError> {
        block_on(serve(
            HostCall::Write {
                path: path.to_owned(),
                text: "x".to_owned(),
            },
            &self.capabilities,
            &self.host,
            Phase::Tool,
            Duration::from_secs(1),
        ))
        .map(|_| ())
    }

    fn absolute(&self, relative: &str) -> String {
        self.scratch.join(relative).to_string_lossy().into_owned()
    }
}

fn refused_by(result: Result<(), HostError>, capability: &str) -> bool {
    matches!(result, Err(HostError::Refused(refusal)) if refusal.capability == capability)
}

#[test]
fn a_read_is_judged_where_its_path_leads_however_it_is_written() {
    let setup = setup("a_read_is_judged_where_its_path_leads_however_it_is_written");
    for allowed in [
        "src/a.rs",
        "./src/a.rs",
        "src/../src/a.rs",
        "src/./a.rs",
        "src",
    ] {
        setup
            .read(allowed)
            .unwrap_or_else(|error| panic!("{allowed}: {error}"));
    }
    setup.read(&setup.absolute("ws/src/a.rs")).unwrap();
    for refused in [
        "notes.md",
        "src/../notes.md",
        "out/old.txt",
        "../outside/secret.rs",
        "~/x",
    ] {
        assert!(refused_by(setup.read(refused), "read"), "{refused}");
    }
    assert!(refused_by(
        setup.read(&setup.absolute("outside/secret.rs")),
        "read"
    ));
    // Nothing the manifest refused reached the host.
    assert!(
        setup.host.asked().iter().all(|asked| asked.contains("src")),
        "{:?}",
        setup.host.asked()
    );
}

#[test]
fn a_link_inside_the_workspace_that_leads_out_is_judged_outside() {
    let setup = setup("a_link_inside_the_workspace_that_leads_out_is_judged_outside");
    if !make_link(
        &setup.scratch.join("outside/secret.rs"),
        &setup.scratch.join("ws/src/escape.rs"),
        false,
    ) {
        return;
    }
    assert!(refused_by(setup.read("src/escape.rs"), "read"));
    if !make_link(
        &setup.scratch.join("outside"),
        &setup.scratch.join("ws/src/far"),
        true,
    ) {
        return;
    }
    assert!(refused_by(setup.read("src/far/secret.rs"), "read"));
}

#[test]
fn a_link_outside_that_leads_in_is_judged_where_it_leads() {
    let setup = setup("a_link_outside_that_leads_in_is_judged_where_it_leads");
    if !make_link(
        &setup.scratch.join("ws/src/a.rs"),
        &setup.scratch.join("outside/in.rs"),
        false,
    ) {
        return;
    }
    setup.read(&setup.absolute("outside/in.rs")).unwrap();
    if !make_link(
        &setup.scratch.join("ws/notes.md"),
        &setup.scratch.join("outside/notes.md"),
        false,
    ) {
        return;
    }
    assert!(refused_by(
        setup.read(&setup.absolute("outside/notes.md")),
        "read"
    ));
}

#[test]
fn a_write_is_judged_the_same_way() {
    let setup = setup("a_write_is_judged_the_same_way");
    setup.write("out/new.txt").unwrap();
    setup.write("./out/../out/new.txt").unwrap();
    for refused in [
        "src/a.rs",
        "out/../src/a.rs",
        "../outside/x",
        "~/x",
        "new.txt",
    ] {
        assert!(refused_by(setup.write(refused), "write"), "{refused}");
    }
    if make_link(
        &setup.scratch.join("ws/src"),
        &setup.scratch.join("ws/out/src"),
        true,
    ) {
        // `out/src/a.rs` is written where it leads: `src/a.rs`.
        assert!(refused_by(setup.write("out/src/a.rs"), "write"));
    }
}

#[test]
fn another_case_is_the_same_file_only_where_the_system_says_so() {
    let setup = setup("another_case_is_the_same_file_only_where_the_system_says_so");
    let result = setup.read("SRC/a.rs");
    if cfg!(windows) {
        result.unwrap();
    } else {
        assert!(refused_by(result, "read"));
    }
}

#[test]
fn each_entry_a_walk_reaches_is_checked_on_its_own() {
    let setup = setup("each_entry_a_walk_reaches_is_checked_on_its_own");
    // The host's listing holds `src/a.rs` and `notes.md`; the manifest
    // reads `src/**` only.
    let listed = block_on(serve(
        HostCall::List {
            path: "src".to_owned(),
            glob: None,
        },
        &setup.capabilities,
        &setup.host,
        Phase::Tool,
        Duration::from_secs(1),
    ))
    .unwrap();
    let shown: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(shown, ["src/a.rs"]);
}

#[test]
fn programs_networks_and_hooks_are_refused_unless_listed() {
    let parsed = Manifest::parse(&manifest(
        "exec = [\"gh\"]\nnet = [\"api.github.com\", \"*.example.com\"]\nhooks = [\"turn.end\"]",
    ))
    .unwrap();
    let workspace = AbsPath::parse("/work/app", cineris_base::PathStyle::Posix).unwrap();
    let capabilities = Capabilities::compile(&parsed.capabilities, &workspace).unwrap();
    assert!(capabilities.exec("gh").is_ok());
    for refused in ["git", "gh.exe", "/usr/bin/gh", "GH"] {
        assert_eq!(capabilities.exec(refused).unwrap_err().capability, "exec");
    }
    assert!(capabilities.host("api.github.com").is_ok());
    assert!(capabilities.host("x.example.com").is_ok());
    for refused in [
        "github.com",
        "example.com",
        "evil.com",
        "api.github.com.evil.com",
    ] {
        assert_eq!(capabilities.host(refused).unwrap_err().capability, "net");
    }
    assert!(capabilities.names_host_literally("api.github.com"));
    assert!(!capabilities.names_host_literally("x.example.com"));
    assert!(
        capabilities
            .hook(cineris_ext::event::Event::TurnEnd)
            .is_ok()
    );
    let refusal = capabilities
        .hook(cineris_ext::event::Event::PromptSubmit)
        .unwrap_err();
    assert!(
        refusal.to_string().contains("every prompt you send"),
        "{refusal}"
    );
}
