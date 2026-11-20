//! The manifest is read strictly: what it allows is enforced, so anything
//! it does not understand refuses the extension.

use std::time::Duration;

use cineris_ext::event::Event;
use cineris_ext::manifest::{HostPattern, Manifest, Program};

use crate::manifest;

fn refused(text: &str) -> String {
    Manifest::parse(text).unwrap_err().to_string()
}

#[test]
fn a_full_manifest_is_read() {
    let parsed = Manifest::parse(&format!(
        "{}\n[limits]\nmemory = 48\ntime = 300\n",
        manifest(
            "exec = [\"gh\", \"/usr/bin/git\", \"C:\\\\Tools\\\\x.exe\"]\nexec_network = true\nread = [\"**\"]\nwrite = [\"out/**\"]\nnet = [\"api.github.com\", \"*.Example.com\", \"*\", \"127.0.0.1\", \"[0:0::1]\", \"::FFFF:10.0.0.5\"]\nhooks = [\"tool.before\", \"turn.end\"]"
        )
    ))
    .unwrap();
    assert_eq!(parsed.name, "probe");
    assert_eq!(
        parsed.capabilities.exec,
        [
            Program::Name("gh".to_owned()),
            Program::Path("/usr/bin/git".to_owned()),
            Program::Path("C:\\Tools\\x.exe".to_owned())
        ]
    );
    assert!(parsed.capabilities.exec_network);
    assert_eq!(
        parsed.capabilities.net,
        [
            HostPattern::Exact("api.github.com".to_owned()),
            HostPattern::Under(".example.com".to_owned()),
            HostPattern::Any,
            HostPattern::Exact("127.0.0.1".to_owned()),
            // An address in the one form a URL's host is read in.
            HostPattern::Exact("::1".to_owned()),
            HostPattern::Exact("10.0.0.5".to_owned())
        ]
    );
    assert_eq!(
        parsed.capabilities.hooks,
        [Event::ToolBefore, Event::TurnEnd]
    );
    assert_eq!(parsed.limits.memory_mib, 48);
    assert_eq!(parsed.limits.time, Duration::from_secs(300));
}

#[test]
fn nothing_is_allowed_unless_the_manifest_says_so() {
    let parsed = Manifest::parse(&manifest("")).unwrap();
    let declared = parsed.capabilities;
    assert!(declared.exec.is_empty() && declared.read.is_empty() && declared.write.is_empty());
    assert!(declared.net.is_empty() && declared.hooks.is_empty() && !declared.exec_network);
}

#[test]
fn an_unknown_key_anywhere_refuses_it() {
    for extra in [
        "expose = \"direct\"\n",
        "[capabilities]\nshell = true\n",
        "[limits]\ncpu = 1\n",
        "[permissions]\nlevel = \"unrestricted\"\n",
    ] {
        let text = format!("api = 1\nname = \"p\"\nversion = \"1\"\nmain = \"main.js\"\n{extra}");
        assert!(Manifest::parse(&text).is_err(), "{extra}");
    }
}

#[test]
fn a_newer_api_is_refused_for_its_version_whatever_else_it_holds() {
    let text = refused("api = 2\nname = \"p\"\nversion = \"1\"\nmain = \"main.js\"\nnew_key = 1\n");
    assert!(
        text.contains("api = 2") && text.contains("another version"),
        "{text}"
    );
}

#[test]
fn programs_are_bare_names_or_absolute_paths() {
    for bad in [
        "./gh",
        "bin/gh",
        "..",
        "../gh",
        "/usr/../bin/sh",
        "C:gh.exe",
        "\\\\server\\share\\x.exe",
        "//server/share/x",
        " gh",
        "",
    ] {
        let text = refused(&manifest(&format!("exec = [{bad:?}]")));
        assert!(text.contains("capabilities.exec"), "{bad}: {text}");
    }
}

#[test]
fn globs_are_relative_to_the_workspace() {
    for bad in [
        "/etc/**",
        "~/notes/**",
        "C:/x/**",
        "\\\\server\\x",
        "../**",
        "src/../../x",
        "./x",
        // A `\` splits folders for one reader and not another (the review
        // of step 28).
        "a\\b/**",
        "out\\**",
    ] {
        let text = refused(&manifest(&format!("read = [{bad:?}]")));
        assert!(text.contains("capabilities.read"), "{bad}: {text}");
        let text = refused(&manifest(&format!("write = [{bad:?}]")));
        assert!(text.contains("capabilities.write"), "{bad}: {text}");
    }
}

#[test]
fn hosts_are_names_addresses_or_wildcards_and_nothing_more() {
    for bad in [
        "https://x.com",
        "x.com:443",
        "x.com/path",
        "*x.com",
        "x.*.com",
        "",
        "-x.com",
    ] {
        let text = refused(&manifest(&format!("net = [{bad:?}]")));
        assert!(text.contains("capabilities.net"), "{bad}: {text}");
    }
    let parsed = Manifest::parse(&manifest("net = [\"*.example.com\"]")).unwrap();
    let under = &parsed.capabilities.net[0];
    assert!(under.matches("api.example.com"));
    assert!(!under.matches("example.com"));
    assert!(!under.matches("evilexample.com"));
    assert!(!under.names_literally("api.example.com"));
}

#[test]
fn hooks_name_the_seven_events_once_each() {
    let text = refused(&manifest("hooks = [\"tool.during\"]"));
    assert!(text.contains("tool.during"), "{text}");
    let text = refused(&manifest("hooks = [\"turn.end\", \"turn.end\"]"));
    assert!(text.contains("twice"), "{text}");
}

#[test]
fn limits_stay_within_the_hard_caps() {
    for bad in ["memory = 0", "memory = 65", "time = 0", "time = 601"] {
        let text = refused(&format!("{}\n[limits]\n{bad}\n", manifest("")));
        assert!(text.contains("limits."), "{bad}: {text}");
    }
}

#[test]
fn the_main_file_and_the_prompt_are_files_of_the_folder() {
    for bad in [
        "../main.js",
        "/main.js",
        "main.ts",
        "lib\\main.js",
        "./main.js",
    ] {
        let text = refused(&format!(
            "api = 1\nname = \"p\"\nversion = \"1\"\nmain = {bad:?}\n"
        ));
        assert!(text.contains("main"), "{bad}: {text}");
    }
    let snapshot = crate::snapshot(&[(
        "extension.toml",
        "api = 1\nname = \"p\"\nversion = \"1\"\nmain = \"main.js\"\nprompt = \"prompt.md\"\n",
    )]);
    let text = Manifest::of(&snapshot).unwrap_err().to_string();
    assert!(text.contains("main.js"), "{text}");
    let long = "x".repeat(4097);
    let snapshot = crate::snapshot(&[
        (
            "extension.toml",
            "api = 1\nname = \"p\"\nversion = \"1\"\nmain = \"main.js\"\nprompt = \"prompt.md\"\n",
        ),
        ("main.js", ""),
        ("prompt.md", &long),
    ]);
    let text = Manifest::of(&snapshot).unwrap_err().to_string();
    assert!(text.contains("4 KiB"), "{text}");
}

#[test]
fn names_follow_the_registrys_rules() {
    for bad in [
        "",
        ".hidden",
        "has space",
        "slash/name",
        "émoji",
        &"x".repeat(65),
    ] {
        let text = refused(&format!(
            "api = 1\nname = {bad:?}\nversion = \"1\"\nmain = \"main.js\"\n"
        ));
        assert!(text.contains("name"), "{bad}: {text}");
    }
}
