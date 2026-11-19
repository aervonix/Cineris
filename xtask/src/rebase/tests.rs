//! The rebase's classes, its merge and its checks on the conflict shapes
//! the last wave met (`xtask/fixtures/rebase/`), then the command on small
//! repositories (`sys_`).
//!
//! The fixtures' texts are the repository's own: AGENTS.md's rows for the
//! open items' index (5996f119) and for a known flake (b37efe3e), added on
//! the same day by two pull requests of the tenth tooling step; the fields
//! `keep_sessions` and `allow_unrestricted` of `CoreOptions` in
//! `crates/cineris-core/src/core.rs`, each added to its struct and its
//! `Debug` by its own change; and two tests of `xtask/src/regen.rs` appended
//! to the same module. Each is cut to the part around the conflict, with
//! both sides' additions at the place where a rebase met them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::classify::{
    Class, Kind, Resolution, Stages, balanced, classify, dividers, keeps_both, resolve, unite,
};
use super::merge::{Hunk, Union, hunks, merge};
use super::{Hooks, How, Mode, Report, Request, State, describe, rebase, unmerged};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("rebase")
        .join(name);
    String::from_utf8(std::fs::read(&path).unwrap()).unwrap()
}

/// A case's three sides.
fn sides(case: &str) -> (String, String, String) {
    (
        fixture(&format!("{case}.base")),
        fixture(&format!("{case}.ours")),
        fixture(&format!("{case}.theirs")),
    )
}

/// The rustfmt check, passed in: these tests start no process.
fn any_layout(_: &str) -> Result<(), String> {
    Ok(())
}

fn strings(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn each_file_is_put_in_its_class() {
    for (path, class) in [
        ("docs/outstanding/README.md", Class::Index),
        ("docs/protocol/schema.json", Class::Generated("protocol")),
        (
            "docs/configuration/reference.md",
            Class::Generated("configuration"),
        ),
        (
            "docs/architecture/tui-look/screens.md",
            Class::Generated("screens"),
        ),
        (
            "crates/cineris-tui/snapshots/view.txt",
            Class::Generated("snapshots"),
        ),
        (
            "crates/cineris-llm/fixtures/anthropic/stream.sse",
            Class::Person,
        ),
        (
            "crates/cineris-core/src/core.rs",
            Class::Additions(Kind::Rust),
        ),
        ("docs/testing-and-ci.md", Class::Additions(Kind::Markdown)),
        ("AGENTS.md", Class::Additions(Kind::Markdown)),
        (
            "docs/outstanding/an-item.md",
            Class::Additions(Kind::Markdown),
        ),
        ("Cargo.lock", Class::Person),
        ("mutations/step-24.breaks", Class::Person),
        (".github/workflows/ci.yml", Class::Person),
    ] {
        assert_eq!(classify(path), class, "{path}");
    }
}

#[test]
fn rows_added_at_the_end_of_one_table_on_both_sides_are_both_kept() {
    for case in ["table-rows.md", "table-rows-crlf.md"] {
        let (base, ours, theirs) = sides(case);
        let merged = unite(Kind::Markdown, &base, &ours, &theirs, &any_layout).unwrap();
        assert_eq!(merged.text, fixture(&format!("{case}.merged")), "{case}");
        assert_eq!(merged.unions.len(), 1, "{case}");
        assert!(merged.unions[0].first[0].starts_with("| Write the open items' index"));
        assert!(merged.unions[0].second[0].starts_with("| Mark a test as a known flake"));
    }
    // The CRLF case keeps its line ends.
    let (base, ours, theirs) = sides("table-rows-crlf.md");
    let merged = unite(Kind::Markdown, &base, &ours, &theirs, &any_layout).unwrap();
    assert_eq!(
        merged.text.matches("\r\n").count(),
        merged.text.matches('\n').count()
    );
}

#[test]
fn fields_added_to_one_struct_and_its_debug_on_both_sides_are_both_kept() {
    let (base, ours, theirs) = sides("struct-fields.rs");
    let merged = unite(Kind::Rust, &base, &ours, &theirs, &any_layout).unwrap();
    assert_eq!(merged.text, fixture("struct-fields.rs.merged"));
    assert_eq!(merged.unions.len(), 2, "the struct and its Debug");
}

#[test]
fn tests_appended_to_one_module_on_both_sides_are_both_kept() {
    let (base, ours, theirs) = sides("tests-appended.rs");
    let merged = unite(Kind::Rust, &base, &ours, &theirs, &any_layout).unwrap();
    assert_eq!(merged.text, fixture("tests-appended.rs.merged"));
}

/// The shape the real rebase of this step met: main reworded a table's
/// last row and added rows after it, and the branch added a row after it
/// too. git stops there; main's rows and its wording are kept whole, and the
/// branch's row after them.
#[test]
fn a_row_added_after_one_the_other_side_reworded_is_kept_after_its_change() {
    let header = "| To | Run |\n|---|---|\n";
    let base = format!("{header}| a | b |\n| c | d |\n\nText.\n");
    let ours = format!("{header}| a | b |\n| c | D, reworded |\n| e | f |\n\nText.\n");
    let theirs = format!("{header}| a | b |\n| c | d |\n| g | h |\n\nText.\n");
    let merged = unite(Kind::Markdown, &base, &ours, &theirs, &any_layout).unwrap();
    assert_eq!(
        merged.text,
        format!("{header}| a | b |\n| c | D, reworded |\n| e | f |\n| g | h |\n\nText.\n")
    );
    assert_eq!(
        merged.unions,
        [Union {
            line: 4,
            first: strings(&["| c | D, reworded |", "| e | f |"]),
            second: strings(&["| g | h |"]),
        }]
    );
    // The same, sides the other way round: the addition is still after the
    // change, and the change is whole.
    let merged = unite(Kind::Markdown, &base, &theirs, &ours, &any_layout).unwrap();
    assert_eq!(
        merged.text,
        format!("{header}| a | b |\n| c | D, reworded |\n| e | f |\n| g | h |\n\nText.\n")
    );
    // An addition before a changed line goes before it.
    let before = format!("{header}| a | b |\n| x | y |\n| c | d |\n\nText.\n");
    let reworded = format!("{header}| a | b |\n| c | D |\n\nText.\n");
    let merged = unite(Kind::Markdown, &base, &reworded, &before, &any_layout).unwrap();
    assert_eq!(
        merged.text,
        format!("{header}| a | b |\n| x | y |\n| c | D |\n\nText.\n")
    );
    // Both sides rewording the row is still a person's.
    let other = format!("{header}| a | b |\n| c | d, otherwise |\n| g | h |\n\nText.\n");
    let error = unite(Kind::Markdown, &base, &ours, &other, &any_layout).unwrap_err();
    assert!(error.contains("both sides change line(s) 4..4"), "{error}");
}

#[test]
fn a_line_both_sides_changed_is_left_for_a_person() {
    let (base, ours, theirs) = sides("same-line.rs");
    let error = unite(Kind::Rust, &base, &ours, &theirs, &any_layout).unwrap_err();
    assert!(error.contains("both sides change line(s) 2..2"), "{error}");
}

#[test]
fn prose_added_at_one_place_on_both_sides_is_left_for_a_person() {
    let (base, ours, theirs) = sides("prose.md");
    let error = unite(Kind::Markdown, &base, &ours, &theirs, &any_layout).unwrap_err();
    assert!(error.contains("not a table's row"), "{error}");
}

#[test]
fn a_row_of_another_shape_than_its_table_is_left_for_a_person() {
    let (base, ours, theirs) = sides("row-of-another-shape.md");
    let error = unite(Kind::Markdown, &base, &ours, &theirs, &any_layout).unwrap_err();
    assert!(
        error.contains("a row of 2 cell dividers in a table whose header has 3"),
        "{error}"
    );
}

#[test]
fn rows_added_where_there_is_no_table_are_left_for_a_person() {
    let base = "# A page\n\nText.\n";
    let ours = "# A page\n\nText.\n| a | b |\n";
    let theirs = "# A page\n\nText.\n| c | d |\n";
    let error = unite(Kind::Markdown, base, ours, theirs, &any_layout).unwrap_err();
    assert!(error.contains("not in a table with a header"), "{error}");
}

#[test]
fn a_rust_union_that_rustfmt_would_lay_out_otherwise_is_left() {
    let (base, ours, theirs) = sides("struct-fields.rs");
    let refuse = |_: &str| Err("it would lay the result out otherwise".to_owned());
    let error = unite(Kind::Rust, &base, &ours, &theirs, &refuse).unwrap_err();
    assert!(error.starts_with("rustfmt: it would lay"), "{error}");
}

/// The shape a union made by hand got wrong in an earlier wave: both sides
/// appended a test, and the result kept one closing brace of two. Each
/// check refuses it alone.
#[test]
fn a_union_that_lost_a_brace_is_refused_by_each_check() {
    let (base, ours, theirs) = sides("tests-appended.rs");
    let whole = fixture("tests-appended.rs.merged");
    let keeps = |merged: &str| keeps_both(&base, &ours, &theirs, merged);
    // The first appended test's closing brace taken out: the second test's
    // `}` would stand in for it in a plain search.
    let at = whole
        .find("    }\n\n    #[test]\n    fn the_listing")
        .unwrap();
    let lost = format!("{}{}", &whole[..at], &whole[at + "    }\n".len()..]);
    let error = keeps(&lost).unwrap_err();
    assert!(
        error.contains("the line `    }` 2 time(s) where the two sides' changes come to 3"),
        "{error}"
    );
    let error = balanced(&lost).unwrap_err();
    assert!(error.contains("is never closed"), "{error}");
    assert_eq!(keeps(&whole), Ok(()));
    assert!(balanced(&whole).is_ok());
    // A line of theirs lost, or one doubled.
    let theirs_lost = whole.replace("        let text = listing();\n", "");
    assert!(keeps(&theirs_lost).unwrap_err().contains("lost or doubled"));
    let doubled = whole.replace(
        "        let text = listing();\n",
        "        let text = listing();\n        let text = listing();\n",
    );
    assert!(keeps(&doubled).unwrap_err().contains("lost or doubled"));
    // Lines out of their order: the base's, and a side's own.
    let swapped = whole.replace(
        "    use super::{WRITERS, listing, select};\n\n    #[test]\n",
        "    #[test]\n\n    use super::{WRITERS, listing, select};\n",
    );
    assert!(
        keeps(&swapped)
            .unwrap_err()
            .contains("both sides kept out of its order")
    );
    let theirs_first = whole.replace(
        "        let text = listing();\n",
        "",
    ).replace(
        "    fn every_place_regen_writes_is_in_the_repository() {\n",
        "    fn every_place_regen_writes_is_in_the_repository() {\n        let text = listing();\n",
    );
    assert!(
        keeps(&theirs_first)
            .unwrap_err()
            .contains("theirs added, or moved one")
    );
}

#[test]
fn brackets_are_counted_outside_comments_strings_and_characters() {
    for text in [
        "fn a() { let b = [1, (2)]; }\n",
        "fn a() { let s = \"}\"; }\n",
        "fn a() { let s = \"\\\"}\"; }\n",
        "fn a() { let s = r#\"}\"#; let t = br\"{\"; }\n",
        "fn a() { let c = '}'; let d = '\\''; let e = '\\u{7D}'; }\n",
        "fn a<'b>(x: &'b str) -> &'b str { 'outer: loop { break 'outer; } x }\n",
        "// }\nfn a() {}\n",
        "/* } /* { */ */ fn a() {}\n",
        "fn a() {\n    for r in [1] {}\n}\n",
    ] {
        assert_eq!(balanced(text), Ok(()), "{text}");
    }
    for (text, said) in [
        ("fn a() {\n", "line 1: `{` is never closed"),
        ("fn a() }\n", "line 1: `}` closes nothing"),
        (
            "fn a() {\n    (]\n}\n",
            "line 2: `]` closes the `(` of line 2",
        ),
        ("fn a() { \"\n", "a string is never closed"),
        ("fn a() { r#\"\" }\n", "a raw string is never closed"),
        ("/* \n", "a comment is never closed"),
    ] {
        let error = balanced(text).unwrap_err();
        assert!(error.contains(said), "{text:?}: {error}");
    }
}

#[test]
fn a_rows_cells_are_its_dividers_that_no_backslash_escapes() {
    assert_eq!(dividers("| a | b |"), 3);
    assert_eq!(dividers("| a \\| b | c |"), 3);
    assert_eq!(dividers("| a \\\\| b |"), 3);
    assert_eq!(dividers("no row"), 0);
}

#[test]
fn additions_are_pushed_down_past_the_lines_they_start_with() {
    // A `use` added at the top, and a test appended at the end: the end
    // matches back into the old test, and the addition reads as starting
    // inside it. Pushed down, it is the whole test, after the old one.
    let base = [
        "mod tests {",
        "    fn a() {",
        "        check();",
        "    }",
        "}",
    ];
    let ours = [
        "mod tests {",
        "    use x;",
        "    fn a() {",
        "        check();",
        "    }",
        "",
        "    fn b() {",
        "        check();",
        "    }",
        "}",
    ];
    let found = hunks(&base, &ours).unwrap();
    assert_eq!(
        found,
        [
            Hunk {
                a_start: 1,
                a_end: 1,
                b_start: 1,
                b_end: 2
            },
            Hunk {
                a_start: 4,
                a_end: 4,
                b_start: 5,
                b_end: 9
            }
        ]
    );
    // And so it unites with a test theirs appended there.
    let theirs = [
        "mod tests {",
        "    fn a() {",
        "        check();",
        "    }",
        "",
        "    fn c() {",
        "        check();",
        "    }",
        "}",
    ];
    let merged = merge(
        &format!("{}\n", base.join("\n")),
        &format!("{}\n", ours.join("\n")),
        &format!("{}\n", theirs.join("\n")),
    )
    .unwrap();
    assert_eq!(
        merged.unions,
        [Union {
            line: 6,
            first: strings(&["", "    fn b() {", "        check();", "    }"]),
            second: strings(&["", "    fn c() {", "        check();", "    }"]),
        }]
    );
    assert_eq!(balanced(&merged.text), Ok(()));
}

#[test]
fn changes_apart_merge_and_one_sides_change_is_taken_whole() {
    let base = "a\nb\nc\nd\ne\n";
    let ours = "A\nb\nc\nd\ne\n";
    let theirs = "a\nb\nc\nd\nE\n";
    let merged = merge(base, ours, theirs).unwrap();
    assert_eq!(merged.text, "A\nb\nc\nd\nE\n");
    assert!(merged.unions.is_empty());
    // The same change on both sides is taken once.
    let merged = merge(base, ours, ours).unwrap();
    assert_eq!(merged.text, ours);
    // An addition just before a line the other side changed goes before
    // the change, which is kept whole.
    let merged = merge(base, "a\nB\nc\nd\ne\n", "a\nx\nb\nc\nd\ne\n").unwrap();
    assert_eq!(merged.text, "a\nx\nB\nc\nd\ne\n");
    // Two changes of one line are not additions.
    let error = merge(base, "a\nB\nc\nd\ne\n", "a\nx\nc\nd\ne\n").unwrap_err();
    assert_eq!(error.line, 2);
    // Nor is a change beside a change and an addition of the other side.
    let error = merge(base, "a\nB\nc\nd\ne\n", "a\ny\nb\nC\nd\ne\n").unwrap_err();
    assert_eq!(error.line, 2);
    // A last line with no line end keeps that.
    let merged = merge("a\nb", "a\nb\nc", "z\na\nb").unwrap();
    assert_eq!(merged.text, "z\na\nb\nc");
}

#[test]
fn git_seeing_a_conflict_this_merge_does_not_is_left() {
    let error = unite(Kind::Rust, "a\n", "a\nb\n", "a\n", &any_layout).unwrap_err();
    assert!(error.contains("disagree"), "{error}");
}

#[test]
fn each_class_has_its_resolution() {
    let stages = |base: Option<&str>, ours: Option<&str>, theirs: Option<&str>| Stages {
        base: base.map(str::to_owned),
        ours: ours.map(str::to_owned),
        theirs: theirs.map(str::to_owned),
    };
    let all = stages(Some("1"), Some("2"), Some("3"));
    assert_eq!(
        resolve(
            "docs/protocol/schema.json",
            Class::Generated("protocol"),
            &all,
            &any_layout
        ),
        Resolution::Taken { then: "protocol" }
    );
    assert_eq!(
        resolve(
            "docs/outstanding/README.md",
            Class::Index,
            &all,
            &any_layout
        ),
        Resolution::Taken {
            then: "outstanding"
        }
    );
    let deleted = stages(Some("1"), None, Some("3"));
    assert!(matches!(
        resolve("docs/protocol/schema.json", Class::Generated("protocol"), &deleted, &any_layout),
        Resolution::Left(why) if why.contains("deleted it")
    ));
    assert!(matches!(
        resolve("Cargo.lock", Class::Person, &all, &any_layout),
        Resolution::Left(why) if why.contains("`Cargo.lock`")
    ));
    let added_twice = stages(None, Some("a\n"), Some("b\n"));
    assert!(matches!(
        resolve("src/a.rs", Class::Additions(Kind::Rust), &added_twice, &any_layout),
        Resolution::Left(why) if why.contains("added the whole file")
    ));
    let (base, ours, theirs) = sides("table-rows.md");
    let three = stages(Some(&base), Some(&ours), Some(&theirs));
    assert_eq!(
        resolve(
            "AGENTS.md",
            Class::Additions(Kind::Markdown),
            &three,
            &any_layout
        ),
        Resolution::United {
            text: fixture("table-rows.md.merged"),
            places: 1
        }
    );
}

#[test]
fn unmerged_entries_are_read_by_stage() {
    let listed = "100644 aaa 1\tsrc/a.rs\x00100644 bbb 2\tsrc/a.rs\x00100644 ccc 3\tsrc/a.rs\0\
                  100644 ddd 2\tdocs/a b.md\x00100644 eee 3\tdocs/a b.md\0";
    let files = unmerged(listed).unwrap();
    assert_eq!(
        files["src/a.rs"],
        [Some("aaa".into()), Some("bbb".into()), Some("ccc".into())]
    );
    assert_eq!(
        files["docs/a b.md"],
        [None, Some("ddd".into()), Some("eee".into())]
    );
    assert!(unmerged("100644 aaa 4\tx\0").is_err());
    assert!(unmerged("nonsense\0").is_err());
}

#[test]
fn a_request_is_read_and_what_does_not_go_together_is_refused() {
    assert_eq!(
        Request::parse(&[]).unwrap(),
        Request {
            mode: Mode::Start,
            onto: "origin/main".to_owned(),
            fetch: true,
            regen: true
        }
    );
    let request =
        Request::parse(&strings(&["--onto", "main", "--no-fetch", "--no-regen"])).unwrap();
    assert_eq!(
        (request.onto.as_str(), request.fetch, request.regen),
        ("main", false, false)
    );
    assert_eq!(
        Request::parse(&strings(&["--continue"])).unwrap().mode,
        Mode::Continue
    );
    for (words, said) in [
        (&["--onto"][..], "needs a ref"),
        (&["--onto", "--no-fetch"], "needs a ref"),
        (&["--continue", "--onto", "main"], "for starting a rebase"),
        (&["--abort", "--no-fetch"], "for starting a rebase"),
        (&["--abort", "--no-regen"], "takes nothing else"),
        (&["--force"], "unknown argument"),
    ] {
        let error = Request::parse(&strings(words)).unwrap_err();
        assert!(error.contains(said), "{words:?}: {error}");
    }
}

#[test]
fn the_state_kept_between_stops_reads_back() {
    let state = State {
        old: Some("abc".to_owned()),
        onto: Some("origin/main".to_owned()),
        generators: ["protocol".to_owned(), "outstanding".to_owned()].into(),
    };
    assert_eq!(State::parse(&state.text()), state);
    assert_eq!(State::parse(""), State::default());
}

#[test]
fn the_report_says_what_was_settled_how_and_what_is_left() {
    let report = Report {
        branch: "tooling-12-b".to_owned(),
        old: Some("0123456789abcdef".to_owned()),
        onto: "origin/main".to_owned(),
        stops: vec![super::Stop {
            commit: "abc1234 Add a row".to_owned(),
            files: vec![
                ("AGENTS.md".to_owned(), How::United(1)),
                (
                    "docs/protocol/schema.json".to_owned(),
                    How::Taken("protocol"),
                ),
                (
                    "docs/outstanding/README.md".to_owned(),
                    How::Taken("outstanding"),
                ),
                (
                    "Cargo.lock".to_owned(),
                    How::Left("settle it by hand".to_owned()),
                ),
            ],
        }],
        finished: false,
        ..Report::default()
    };
    let text = describe(&report);
    for said in [
        "Stopped at abc1234 Add a row:",
        "united    AGENTS.md: both sides' additions at 1 place(s)",
        "taken     docs/protocol/schema.json: the side rebased onto; `cargo xtask regen protocol`",
        "`cargo xtask outstanding` writes it at the end",
        "LEFT      Cargo.lock: settle it by hand",
        "`cargo xtask rebase --continue`",
        "The tip before the rebase was 0123456789ab.",
    ] {
        assert!(text.contains(said), "{said}\n{text}");
    }
    let done = describe(&Report {
        finished: true,
        stops: Vec::new(),
        after: vec!["docs/outstanding/README.md is up to date.".to_owned()],
        ..report
    });
    assert!(done.contains("with no conflict"), "{done}");
    assert!(done.contains("`git reset --keep 0123456789ab`"), "{done}");
    assert!(done.contains("is up to date"), "{done}");
}

// The command on small repositories.

use cineris_testkit::Scratch;
use cineris_testkit::scratch::git;

use crate::branch::Git;

/// git as the command runs it, on a test's repository: none of the
/// machine's configuration, so the identity is the repository's own.
fn isolated(dir: &Path) -> Git {
    let mut env: Vec<(&'static str, Option<std::ffi::OsString>)> = vec![
        ("GIT_CONFIG_NOSYSTEM", Some("1".into())),
        (
            "GIT_CONFIG_GLOBAL",
            Some(dir.join("no-such-git-config").into_os_string()),
        ),
    ];
    for variable in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
    ] {
        env.push((variable, None));
    }
    Git::with_env(dir, env)
}

/// A repository whose `main` and `work` both changed a table, a Rust test
/// module and a generated file from one base, in one `fast-import`; `work`
/// checked out. `work_rust` is what `work` makes of the Rust file.
fn repository(scratch: &Scratch, work_rust: &str) -> PathBuf {
    let repo = scratch.git_init("repo");
    let config = repo.join(".git").join("config");
    let mut text = std::fs::read_to_string(&config).unwrap();
    text.push_str(
        "[user]\n\tname = Rebase Tester\n\temail = rebase@cineris.invalid\n\
         [commit]\n\tgpgsign = false\n[core]\n\tautocrlf = false\n",
    );
    std::fs::write(&config, text).unwrap();

    let (table, rows_main, rows_work) = sides("table-rows.md");
    let (rust, rust_main, _) = sides("tests-appended.rs");
    let files = |table: &str, rust: &str, schema: &str| {
        [
            ("docs/table.md", table.to_owned()),
            ("src/lib.rs", rust.to_owned()),
            ("docs/protocol/schema.json", schema.to_owned()),
        ]
    };
    let mut stream = String::new();
    let mut commit = |branch: &str, mark: u32, from: Option<u32>, files: &[(&str, String)]| {
        let at = 1_700_000_000 + u64::from(mark) * 60;
        let message = format!("Commit {mark}");
        stream.push_str(&format!(
            "commit refs/heads/{branch}\nmark :{mark}\n\
             author Rebase Tester <rebase@cineris.invalid> {at} +0000\n\
             committer Rebase Tester <rebase@cineris.invalid> {at} +0000\n\
             data {}\n{message}\n",
            message.len() + 1
        ));
        if let Some(from) = from {
            stream.push_str(&format!("from :{from}\n"));
        }
        for (path, text) in files {
            stream.push_str(&format!(
                "M 100644 inline {path}\ndata {}\n{text}\n",
                text.len()
            ));
        }
    };
    commit("main", 1, None, &files(&table, &rust, "{\"v\": 1}\n"));
    commit(
        "main",
        2,
        Some(1),
        &files(&rows_main, &rust_main, "{\"v\": 2}\n"),
    );
    commit(
        "work",
        3,
        Some(1),
        &files(&rows_work, work_rust, "{\"v\": 3}\n"),
    );
    import(&repo, &stream);
    git(&repo, &["checkout", "--quiet", "work"]);
    repo
}

/// Makes the commits of a `git fast-import` stream, in one git.
fn import(repo: &Path, stream: &str) {
    use std::io::Write as _;
    let mut child = std::process::Command::new("git")
        .args(["fast-import", "--quiet"])
        .current_dir(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", repo.join("no-such-git-config"))
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
}

/// The rebase onto `main`, with rustfmt as the command runs it and the end
/// recorded.
fn run(repo: &Path, mode: Mode) -> (crate::Result<Report>, Vec<BTreeSet<String>>) {
    let git = isolated(repo);
    let root = crate::workspace::root();
    let formatted = |text: &str| super::rustfmt_leaves(&root, text);
    let mut ended = Vec::new();
    let mut finish = |generators: &BTreeSet<String>| {
        ended.push(generators.clone());
        Ok(vec!["ended".to_owned()])
    };
    let request = Request {
        mode,
        onto: "main".to_owned(),
        fetch: false,
        regen: false,
    };
    let report = rebase(
        &git,
        repo,
        &request,
        &mut Hooks {
            formatted: &formatted,
            finish: &mut finish,
        },
    );
    (report, ended)
}

/// The whole command on Windows as on Linux, in as few gits as it can be
/// shown in: rows added to one table, tests appended to one module and a
/// generated file changed, each on both sides, all settled in one stop,
/// rustfmt run on the union, and the generator named at the end.
#[test]
fn sys_a_rebase_settles_rows_tests_and_a_generated_file_and_regenerates_at_the_end() {
    let scratch = Scratch::outside("xtask-rebase", "settles");
    let repo = repository(&scratch, &fixture("tests-appended.rs.theirs"));
    let (report, ended) = run(&repo, Mode::Start);
    let report = report.unwrap();
    assert!(report.finished, "{}", describe(&report));
    assert_eq!(report.stops.len(), 1, "{}", describe(&report));
    assert_eq!(
        report.stops[0].files,
        [
            (
                "docs/protocol/schema.json".to_owned(),
                How::Taken("protocol")
            ),
            ("docs/table.md".to_owned(), How::United(1)),
            ("src/lib.rs".to_owned(), How::United(1)),
        ]
    );
    assert_eq!(ended, [BTreeSet::from(["protocol".to_owned()])]);
    let read = |path: &str| std::fs::read_to_string(repo.join(path)).unwrap();
    assert_eq!(read("docs/table.md"), fixture("table-rows.md.merged"));
    assert_eq!(read("src/lib.rs"), fixture("tests-appended.rs.merged"));
    assert_eq!(
        read("docs/protocol/schema.json"),
        "{\"v\": 2}\n",
        "main's side"
    );
    // One commit on main's, the branch moved, nothing left behind.
    let read = git(&repo, &["rev-list", "--count", "main..work"]);
    assert_eq!(read, "1");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    assert!(!repo.join(".git").join("xtask-rebase").exists());
    let said = describe(&report);
    assert!(said.contains("every conflict settled"), "{said}");
}

/// A line both sides changed is left, with the rest of its stop settled;
/// the rebase waits; `--continue` after a person settles it ends it.
#[test]
#[cfg_attr(
    windows,
    ignore = "git's own stop and go on, the same on every system; on Windows it would cost \
              seconds of the system tier"
)]
fn sys_a_change_both_sides_made_is_left_and_continue_takes_it_up_after_a_person() {
    let scratch = Scratch::outside("xtask-rebase", "left");
    // `work` changes the first test's name, which main left alone, and
    // appends its test: the name merges cleanly; main's addition and
    // work's touch only at the end.
    let theirs = fixture("tests-appended.rs.theirs").replace("    use super::", "    use crate::");
    let repo = repository(&scratch, &theirs);
    // main also changed that `use` line: a change on both sides.
    git(&repo, &["checkout", "--quiet", "main"]);
    let main_rust = fixture("tests-appended.rs.ours").replace("    use super::", "    use self::");
    std::fs::write(repo.join("src/lib.rs"), main_rust).unwrap();
    git(
        &repo,
        &["commit", "--quiet", "-am", "Change the use on main"],
    );
    git(&repo, &["checkout", "--quiet", "work"]);

    let (report, ended) = run(&repo, Mode::Start);
    let report = report.unwrap();
    assert!(!report.finished);
    assert!(ended.is_empty(), "nothing is done at the end of a stop");
    let files = &report.stops[0].files;
    assert_eq!(
        files[0],
        (
            "docs/protocol/schema.json".to_owned(),
            How::Taken("protocol")
        )
    );
    assert_eq!(files[1], ("docs/table.md".to_owned(), How::United(1)));
    assert!(
        matches!(&files[2], (path, How::Left(why)) if path == "src/lib.rs" && why.contains("both sides change line(s) 3..3")),
        "{files:?}"
    );
    let said = describe(&report);
    assert!(said.contains("LEFT      src/lib.rs"), "{said}");
    // The file is as git left it, with its markers.
    let left = std::fs::read_to_string(repo.join("src/lib.rs")).unwrap();
    assert!(left.contains("<<<<<<<"), "{left}");
    // `--continue` with it still conflicted stops again.
    let (again, _) = run(&repo, Mode::Continue);
    let again = again.unwrap();
    assert!(!again.finished);
    assert_eq!(again.stops[0].files.len(), 1, "{:?}", again.stops);

    // A person settles it.
    std::fs::write(repo.join("src/lib.rs"), fixture("tests-appended.rs.merged")).unwrap();
    git(&repo, &["add", "src/lib.rs"]);
    let (done, ended) = run(&repo, Mode::Continue);
    let done = done.unwrap();
    assert!(done.finished, "{}", describe(&done));
    assert_eq!(
        ended,
        [BTreeSet::from(["protocol".to_owned()])],
        "the first stop's generator is kept"
    );
    assert_eq!(done.onto, "main");
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
}

#[test]
#[cfg_attr(
    windows,
    ignore = "git's own abort, the same on every system; on Windows it would cost seconds of the \
              system tier"
)]
fn sys_abort_puts_the_branch_back_and_a_second_start_is_refused_while_one_is_stopped() {
    let scratch = Scratch::outside("xtask-rebase", "abort");
    let repo = repository(&scratch, "fn changed() {}\n");
    let before = git(&repo, &["rev-parse", "work"]);
    let (report, _) = run(&repo, Mode::Start);
    assert!(!report.unwrap().finished);
    let (again, _) = run(&repo, Mode::Start);
    assert!(again.unwrap_err().contains("a rebase is in progress"));
    let (aborted, _) = run(&repo, Mode::Abort);
    assert!(aborted.unwrap().finished);
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), before);
    assert_eq!(git(&repo, &["status", "--porcelain"]), "");
    let (none, _) = run(&repo, Mode::Abort);
    assert!(none.unwrap_err().contains("no rebase is in progress"));
    // Changes not committed are refused before anything starts.
    std::fs::write(repo.join("docs/table.md"), "changed\n").unwrap();
    let (dirty, _) = run(&repo, Mode::Start);
    assert!(dirty.unwrap_err().contains("not committed"));
}

#[test]
fn a_union_is_written_with_the_classes_own_check() {
    // A Rust union that does not balance is refused by `unite`, whatever the
    // merge made: the check runs on the text that would be written.
    let base = "fn a() {\n}\n";
    let ours = "fn a() {\n}\n(\n";
    let theirs = "fn a() {\n}\n[\n";
    let error = unite(Kind::Rust, base, ours, theirs, &any_layout).unwrap_err();
    assert!(error.contains("never closed"), "{error}");
    // The same lines in Markdown are refused as not rows.
    let error = unite(Kind::Markdown, base, ours, theirs, &any_layout).unwrap_err();
    assert!(error.contains("not a table's row"), "{error}");
}
