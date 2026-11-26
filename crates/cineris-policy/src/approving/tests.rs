use super::*;

use ShellSyntax::Posix;

/// Whether the fallback refuses `line` in `shell`.
fn refuses(line: &str, shell: ShellSyntax) -> bool {
    refused(line, shell, None).is_some()
}

/// Over the one table, the fallback refuses exactly the lines marked
/// refused, in each syntax given.
#[test]
fn the_fallback_refuses_exactly_the_tables_refused_lines() {
    let mut wrong = Vec::new();
    for (line, shells, refused_too) in CASES {
        for shell in *shells {
            let refusal = refused(line, *shell, None);
            if refusal.is_some() != *refused_too {
                wrong.push(format!("{line:?} in {shell:?}: {refusal:?}"));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} line(s):\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// Each reason says what is refused, why, and to run it in one's own
/// terminal; the application's says what it would answer (the early
/// review's L5); every one begins with [`REFUSED`].
#[test]
fn each_refusal_says_what_it_refuses_and_where_to_run_it() {
    let reason_of = |line: &str| reason(&refused(line, Posix, None).unwrap());
    let approves = reason_of("cin ext add x");
    assert!(approves.contains("`cineris ext add`"), "{approves}");
    let application = reason_of("cineris");
    assert!(
        application.contains("start its own application") && application.contains("trust question"),
        "{application}"
    );
    let widens = reason_of("cineris -p --trust-project hi");
    assert!(widens.contains("`cineris -p --trust-project`"), "{widens}");
    let moved = reason_of("env HOME=/tmp/x cineris -p hi");
    assert!(moved.contains("HOME"), "{moved}");
    let hidden = reason_of("cineris $A");
    assert!(hidden.contains("expands"), "{hidden}");
    for said in [approves, application, widens, moved, hidden] {
        assert!(said.starts_with(REFUSED), "{said}");
        assert!(said.contains("run it in your own terminal"), "{said}");
    }
}

/// The names the front end gives are Cineris's too, as the application and
/// with a subcommand (the early review's L6); without them, only `cineris`
/// and `cin` are, so that a test's binary run as a hook is not.
#[test]
fn the_front_ends_names_are_cinerises_too() {
    let own = OwnCommands::new(vec!["mycin.exe".to_owned()], None);
    for line in [
        "mycin",
        "mycin ext add x",
        "/opt/MYCIN.EXE permissions trust",
    ] {
        assert!(refused(line, Posix, Some(&own)).is_some(), "{line}");
        assert_eq!(refused(line, Posix, None), None, "{line}");
    }
    assert_eq!(refused("mycin --version", Posix, Some(&own)), None);
}

/// Where the front end gives its grammar, it decides, for each run of
/// Cineris found in the line, with what follows it and whether more that is
/// not known follows.
#[test]
fn the_front_ends_grammar_decides_each_run_found() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let kept = Arc::clone(&seen);
    let grammar: Grammar = Arc::new(move |invocation: &Invocation<'_>| {
        let (rest, open) = (invocation.words, invocation.open);
        kept.lock().unwrap().push((rest.to_vec(), open));
        if rest.first().is_some_and(|word| word == "bad") {
            Standing::Refused(Refusal::new("cineris bad", Kind::Approves))
        } else {
            Standing::Safe
        }
    });
    let own = OwnCommands::new(Vec::new(), Some(grammar));
    assert!(refused("sudo cineris bad x", Posix, Some(&own)).is_some());
    assert_eq!(refused("cineris ext add x", Posix, Some(&own)), None);
    assert!(refused("cineris bad $X", Posix, Some(&own)).is_some());
    let seen = seen.lock().unwrap();
    assert!(
        seen.contains(&(vec!["bad".to_owned(), "x".to_owned()], false)),
        "{seen:?}"
    );
    assert!(seen.contains(&(vec!["bad".to_owned()], true)), "{seen:?}");
}

/// A line read past the depth, or past the work one check does, is
/// refused: the bounds fail closed.
#[test]
fn a_line_read_too_deep_or_too_often_fails_closed() {
    let own = OwnCommands::new(Vec::new(), None);
    let reader = Reader::new(&own);
    assert_eq!(reader.read("cineris --version", Line::Shell, 1), None);
    assert!(
        reader
            .read("cineris --version", Line::Shell, DEPTH)
            .is_some()
    );
    for _ in 0..WORK {
        let _ = reader.read("cineris --version", Line::Shell, 1);
    }
    assert!(reader.read("cineris --version", Line::Shell, 1).is_some());
    // Nested lines are read to the depth.
    assert!(refuses("sh -c 'bash -c \"cin ext add x\"'", Posix));
}

/// A line that does not mention Cineris is passed over before it is
/// analysed: what makes the check cheap on every other line.
#[test]
fn a_line_that_never_names_cineris_is_passed_over() {
    let own = OwnCommands::new(Vec::new(), None);
    let reader = Reader::new(&own);
    assert!(!reader.mentioned("cargo test --workspace && git status"));
    assert!(reader.mentioned("c\\in'e'ris"));
    assert!(reader.mentioned("ls cin[e]ris"));
}

/// A refusal names the run it refuses, read past what started it: the
/// program and its arguments `Start-Process` gives, the rest of a `cmd /c`
/// line, a wrapper's command, `env -S`'s line, an internal subcommand.
#[test]
fn each_refusal_names_the_run_it_refuses() {
    for (line, shell, what, kind) in [
        (
            "Start-Process cineris -ArgumentList 'ext add ./x' -NoNewWindow -Wait",
            ShellSyntax::PowerShell,
            "cineris ext add",
            Kind::Approves,
        ),
        (
            "Start-Process -FilePath cin -ArgumentList 'permissions trust'",
            ShellSyntax::PowerShell,
            "cineris permissions trust",
            Kind::Approves,
        ),
        (
            "cmd /c cineris ext add x",
            ShellSyntax::PowerShell,
            "cineris ext add",
            Kind::Approves,
        ),
        (
            "sudo -u me cineris ext add ./x",
            Posix,
            "cineris ext add",
            Kind::Approves,
        ),
        (
            "env -S'cin permissions trust'",
            Posix,
            "cineris permissions trust",
            Kind::Approves,
        ),
        (
            "cineris __sandbox -- ls",
            Posix,
            "cineris __sandbox",
            Kind::Internal,
        ),
        (
            "cineris -p --allow 'exec:*' go",
            Posix,
            "cineris -p --allow",
            Kind::Widens,
        ),
    ] {
        let refusal = refused(line, shell, None).unwrap_or_else(|| panic!("{line:?}"));
        assert_eq!(
            (refusal.what.as_str(), refusal.kind),
            (what, kind),
            "{line:?}"
        );
    }
}

/// A grammar that panics is not trusted past it: the run is refused, and
/// the panic goes no further (the re-look at the early review, L5).
#[test]
fn a_grammar_that_panics_refuses() {
    let grammar: Grammar = Arc::new(|_: &Invocation<'_>| panic!("a grammar's own fault"));
    let own = OwnCommands::new(Vec::new(), Some(grammar));
    let refusal = refused("cineris ext list", Posix, Some(&own));
    assert!(
        matches!(
            refusal,
            Some(Refusal {
                kind: Kind::Hidden,
                ..
            })
        ),
        "{refusal:?}"
    );
}

/// Where the words may not be the program's own (a line read as text, a
/// pattern), the help or the version is not taken as asked (H2).
#[test]
fn the_help_is_safe_only_in_words_that_are_the_programs_own() {
    assert_eq!(
        fallback(&["--help".to_owned()], false, true),
        Standing::Safe
    );
    assert!(matches!(
        fallback(&["--help".to_owned()], false, false),
        Standing::Refused(Refusal {
            kind: Kind::Hidden,
            ..
        })
    ));
    assert!(matches!(
        fallback(
            &["ext".to_owned(), "add".to_owned(), "-h".to_owned()],
            false,
            false
        ),
        Standing::Refused(_)
    ));
}
