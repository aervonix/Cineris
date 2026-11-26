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

/// A line built to nest or repeat without end is refused, and soon, on a
/// thread as small as the core's (the full review of the third part, M1).
#[test]
fn a_line_that_nests_or_repeats_without_end_is_refused_soon() {
    let lines = [
        (format!("{}cin", "nohup ".repeat(1000)), Posix),
        (
            format!("{}cin", "start ".repeat(5000)),
            ShellSyntax::PowerShell,
        ),
        (format!("find . {}cin", "-exec ".repeat(5000)), Posix),
        (format!("echo {}", "a=cin=b=cin=".repeat(2000)), Posix),
        (format!("echo {} cin", "x".repeat(60 * 1024)), Posix),
    ];
    let checked = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            lines
                .iter()
                .map(|(line, shell)| {
                    let started = std::time::Instant::now();
                    let refusal = refused(line, *shell, None);
                    (line.len(), refusal.is_some(), started.elapsed())
                })
                .collect::<Vec<_>>()
        })
        .unwrap()
        .join()
        .expect("no line overflows the stack");
    for (length, refused_too, took) in checked {
        assert!(refused_too, "a line of {length} bytes was not refused");
        assert!(
            took < std::time::Duration::from_secs(2),
            "{length} bytes took {took:?}"
        );
    }
}

/// A `sed` script runs a line only by its `e` command or an `s` command's
/// `e` flag: any other script's words are not read as lines.
#[test]
fn a_sed_script_runs_a_line_only_by_its_e() {
    for runs in [
        "1e cin ext add ./x",
        "s/^/cin ext add/e",
        "$e cin",
        "/x/ { e cin\n}",
        "s|a|b|ge",
    ] {
        assert!(sed_runs(runs), "{runs}");
    }
    for edits in [
        "s/cineris serve/cineris serve --stdio/",
        "1,20p",
        "s/e/f/g",
        "/^e/d",
        "y/abc/xyz/",
    ] {
        assert!(!sed_runs(edits), "{edits}");
    }
}

/// Look-alike letters are read as the Latin ones of Cineris's names, and a
/// version after a shell's or an interpreter's name is taken off.
#[test]
fn look_alikes_and_versions_are_read_as_the_names() {
    for written in ["ｃｉｎ", "cın", "CİN", "cinerıſ", "сіn", "ⅽⅰn"] {
        let name = name_of(written);
        assert!(name == "cin" || name == "cineris", "{written}: {name}");
    }
    assert_eq!(name_of("Cafe"), "cafe");
    for (written, kind) in [
        ("python3.12", "python"),
        ("lua5.4", "lua"),
        ("luajit", "lua"),
        ("bash5", "bash"),
        ("nodejs", "node"),
        ("sh", "sh"),
    ] {
        assert_eq!(kind_of(written), kind, "{written}");
    }
}

/// A program that runs what it reads as a line is told apart from one given
/// its own line, code or file.
#[test]
fn a_program_that_runs_what_it_reads_is_told_apart() {
    let words = |line: &str| -> Vec<String> { line.split(' ').map(str::to_owned).collect() };
    for reads in [
        "sh",
        "bash -s",
        "bash /dev/stdin",
        "python3",
        "python3 -",
        "at now",
        "crontab -",
        "make -f -",
        "cmd",
        "pwsh -Command -",
        "iex",
        "eval x",
        "xargs -0 sh -c",
        "sudo bash",
    ] {
        assert!(reads_a_line(&words(reads)), "{reads}");
    }
    for given in [
        "sh -c ls",
        "bash script.sh",
        "python3 -c pass",
        "python3 tool.py",
        "cmd /c dir",
        "pwsh -File a.ps1",
        "make install",
        "echo hi",
    ] {
        assert!(!reads_a_line(&words(given)), "{given}");
    }
}

/// The keys a terminal is told to type are joined as typed, past their
/// options.
#[test]
fn the_keys_a_terminal_is_told_to_type_are_joined() {
    let words = |line: &str| -> Vec<String> { line.split(' ').map(str::to_owned).collect() };
    assert_eq!(typed("tmux", &words("send-keys -t 0 -l c in")), "cin");
    assert_eq!(typed("xdotool", &words("type --delay 5 ci n")), "cin");
    assert_eq!(typed("wtype", &words("ci n")), "cin");
    assert_eq!(typed("tmux", &words("new -d")), "");
}
