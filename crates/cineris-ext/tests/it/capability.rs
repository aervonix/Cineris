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
    // The fake host walks `src`, `src/a.rs`, `notes.md`, `out` and
    // `out/old.txt` through the gate it is handed; the manifest reads
    // `src/**` only.
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
    assert_eq!(listed["text"], "src\nsrc/a.rs");
}

#[test]
fn a_walk_enters_a_folder_a_glob_reaches_under_and_reads_only_what_it_names() {
    let setup = setup("a_walk_enters_a_folder_a_glob_reaches_under_and_reads_only_what_it_names");
    setup.scratch.file("ws/docs/x.rs", "r");
    setup.scratch.file("ws/docs/x.md", "m");
    let parsed = Manifest::parse(&manifest("read = [\"**/*.rs\"]")).unwrap();
    let root = setup.host.resolver.as_ref().unwrap().root().clone();
    let capabilities = Capabilities::compile(&parsed.capabilities, &root).unwrap();
    let at = |relative: &str| root.join(relative).unwrap();
    assert!(capabilities.reaches_under(&at("docs")));
    assert!(capabilities.reads(&at("docs/x.rs")));
    assert!(!capabilities.reads(&at("docs/x.md")));
    // A file is never let through for what a glob could reach under it.
    assert!(!capabilities.reads(&at("notes.md")));
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

/// A path outside the workspace is outside every glob, one whose braces
/// hold that very path among them included: the manifest refuses such a
/// glob when it is read, and the capabilities, given one past that check,
/// still refuse a read there (the review of step 28's close, finding 13:
/// the check of the path against the workspace was all that held, and no
/// test broke without it).
#[test]
fn a_glob_whose_braces_name_a_path_outside_reads_nothing_there() {
    let setup = setup("a_glob_whose_braces_name_a_path_outside_reads_nothing_there");
    let root = setup.host.resolver.as_ref().unwrap().root().clone();
    // The folder beside the workspace, in the spelling the resolver gives,
    // with `/` between its names on every system.
    let outside = root.parent().unwrap().join("outside").unwrap();
    let glob = format!("{{{}/**,src/**}}", outside.to_string().replace('\\', "/"));
    let refused = Manifest::parse(&manifest(&format!("read = [{glob:?}]"))).unwrap_err();
    assert!(
        refused.to_string().contains("capabilities.read"),
        "{refused}"
    );
    let declared = cineris_ext::manifest::Declared {
        read: vec![glob],
        ..cineris_ext::manifest::Declared::default()
    };
    let capabilities = Capabilities::compile(&declared, &root).unwrap();
    let read = |path: &str| {
        block_on(serve(
            HostCall::Read {
                path: path.to_owned(),
                offset: 0,
                limit: None,
            },
            &capabilities,
            &setup.host,
            Phase::Tool,
            Duration::from_secs(1),
        ))
        .map(|_| ())
    };
    assert!(refused_by(
        read(&setup.absolute("outside/secret.rs")),
        "read"
    ));
    assert!(refused_by(read("../outside/secret.rs"), "read"));
    assert!(
        !capabilities.reads(&outside.join("secret.rs").unwrap()),
        "a walk would read outside the workspace"
    );
    // Its other alternative still reads inside.
    read("src/a.rs").unwrap();
    assert!(
        setup
            .host
            .asked()
            .iter()
            .all(|asked| !asked.contains("secret")),
        "{:?}",
        setup.host.asked()
    );
}
