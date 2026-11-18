//! The rename's table, case by case; the kept lines; the check; a second
//! pass that changes nothing; the lock's order; and, with git, a branch
//! made again and rebased onto a renamed main.

use std::path::Path;

use crate::testkit::Scratch;

use super::branch::{self, CommitText, Options, parse_tree, quote_path};
use super::{
    Formatter, check_moves, is_alias, rename_path, rename_text, sort_lock, transform, violations,
};

/// `text` as a file at `path` (an ordinary one) would have it.
fn renamed(text: &str) -> String {
    rename_text("docs/some-page.md", text).into_owned()
}

#[test]
fn every_case_form_of_the_brand_is_renamed() {
    for (old, new) in [
        ("Cortex-Harness", "Cineris"),
        ("cortex-harness", "cineris"),
        ("CORTEX-HARNESS", "CINERIS"),
        ("Cortex", "Cineris"),
        ("cortex", "cineris"),
        ("CORTEX", "CINERIS"),
        // Compound forms: crates, identifiers, variables, folders, keys.
        ("cortex-tui", "cineris-tui"),
        (
            "crates/cortex-core/src/lib.rs",
            "crates/cineris-core/src/lib.rs",
        ),
        ("cortex_base::log", "cineris_base::log"),
        ("CortexError", "CinerisError"),
        ("CORTEX_PERMISSIONS_MODE", "CINERIS_PERMISSIONS_MODE"),
        ("CORTEX_DECIDE_RULES", "CINERIS_DECIDE_RULES"),
        (
            "~/work/app/.cortex/config.toml",
            "~/work/app/.cineris/config.toml",
        ),
        (
            "~/.config/cortex-harness/config.toml",
            "~/.config/cineris/config.toml",
        ),
        (
            r"C:\PEOPLE\DEV\APPDATA\ROAMING\CORTEX-HARNESS\CREDENTIALS.TOML",
            r"C:\PEOPLE\DEV\APPDATA\ROAMING\CINERIS\CREDENTIALS.TOML",
        ),
        ("$XDG_STATE_HOME/cortex-harness", "$XDG_STATE_HOME/cineris"),
        ("cortex.ownerTermsRequired", "cineris.ownerTermsRequired"),
        (
            "Cortex-Harness <dev@cortex-harness.invalid>",
            "Cineris <dev@cineris.invalid>",
        ),
        (
            "Cortex Test <test@cortex-harness.invalid>",
            "Cineris Test <test@cineris.invalid>",
        ),
        (
            "github.com/aervonix/Cortex-Harness",
            "github.com/aervonix/Cineris",
        ),
        (
            "`cortex attach`, `cortex serve`",
            "`cineris attach`, `cineris serve`",
        ),
        ("\x1b[1mCortex-Harness\x1b[0m", "\x1b[1mCineris\x1b[0m"),
        ("cortex-protocol.d.ts", "cineris-protocol.d.ts"),
        ("cortex_fuzz::sse_parser", "cineris_fuzz::sse_parser"),
        (
            "~/.local/state/cortex-live/spend.jsonl",
            "~/.local/state/cineris-live/spend.jsonl",
        ),
    ] {
        assert_eq!(renamed(old), new, "{old}");
    }
}

#[test]
fn the_developers_folders_leave_the_products_folder() {
    for (old, new) in [
        (
            "~/.config/cortex-harness/owner-terms",
            "~/.config/cineris-dev/owner-terms",
        ),
        (
            "/h/.config/cortex-harness/owner-terms\")",
            "/h/.config/cineris-dev/owner-terms\")",
        ),
        (
            "~/.cache/cortex-harness/corpus",
            "~/.cache/cineris-dev/corpus",
        ),
        (
            "$XDG_CACHE_HOME/cortex-harness/corpus/",
            "$XDG_CACHE_HOME/cineris-dev/corpus/",
        ),
        (
            r"%LOCALAPPDATA%\cortex-harness\corpus",
            r"%LOCALAPPDATA%\cineris-dev\corpus",
        ),
        (
            r#""%LOCALAPPDATA%\\cortex-harness\\corpus""#,
            r#""%LOCALAPPDATA%\\cineris-dev\\corpus""#,
        ),
        (
            r#"base.join("cortex-harness").join("corpus")"#,
            r#"base.join("cineris-dev").join("corpus")"#,
        ),
        // As rustfmt lays a chain out.
        (
            ".join(\".config\")\n        .join(\"cortex-harness\")\n        .join(\"owner-terms\"),",
            ".join(\".config\")\n        .join(\"cineris-dev\")\n        .join(\"owner-terms\"),",
        ),
        // The product's own folders stay the product's.
        (
            "~/.config/cortex-harness/credentials.toml",
            "~/.config/cineris/credentials.toml",
        ),
        (
            r#".join("cortex-harness").join("trust.toml")"#,
            r#".join("cineris").join("trust.toml")"#,
        ),
        (
            "~/.config/cortex-harness/owner-terms-old",
            "~/.config/cineris/owner-terms-old",
        ),
        ("cortex-harness/corpora", "cineris/corpora"),
        (
            "/users/dev/.config/cortex-harness-notes/a",
            "/users/dev/.config/cineris-notes/a",
        ),
    ] {
        assert_eq!(renamed(old), new, "{old}");
    }
}

#[test]
fn a_spelling_the_table_does_not_know_is_left_for_the_check() {
    for odd in ["CorTex", "cortex-Harness", "Cortex-harness"] {
        let text = format!("the {odd} here");
        let out = renamed(&text);
        assert!(out.to_lowercase().contains("cortex"), "{odd}: {out}");
        assert_eq!(violations("docs/a.md", out.as_bytes()).len(), 1, "{odd}");
    }
}

#[test]
fn the_alias_is_renamed_where_it_is_the_alias() {
    for (old, new) in [
        ("the alias `cx`.", "the alias `cin`."),
        (
            "const ALIAS: &str = \"cx\";",
            "const ALIAS: &str = \"cin\";",
        ),
        ("name_for(\"./cx\")", "name_for(\"./cin\")"),
        ("name_for(\"bin/cx\")", "name_for(\"bin/cin\")"),
        ("name_for(\"cx.exe\")", "name_for(\"cin.exe\")"),
        ("name_for(\"CX.EXE\")", "name_for(\"CIN.EXE\")"),
        (
            r#"name_for(r"C:\tools\CX.EXE")"#,
            r#"name_for(r"C:\tools\CIN.EXE")"#,
        ),
        ("`cortex.exe` and `cx.exe`", "`cineris.exe` and `cin.exe`"),
        ("creates cx.exe as a link", "creates cin.exe as a link"),
        ("a shim (`cx.cmd`)", "a shim (`cin.cmd`)"),
        (
            "| `cx --version` | 0.36 ms |",
            "| `cin --version` | 0.36 ms |",
        ),
        (
            "headless `cx -p`, to its request",
            "headless `cin -p`, to its request",
        ),
        ("target/debug/cx --version", "target/debug/cin --version"),
        (
            ".replace(\"`cortex\", \"`cx\")",
            ".replace(\"`cineris\", \"`cin\")",
        ),
        (
            "also installed under the alias cx: argument",
            "also installed under the alias cin: argument",
        ),
        (
            "[(\"cortex\", binary), (\"cx\", alias)]",
            "[(\"cineris\", binary), (\"cin\", alias)]",
        ),
        // Paths, names made from it, and what a command prints.
        (
            "Some(&path(\"/config/cx\"))",
            "Some(&path(\"/config/cin\"))",
        ),
        ("\"/config/cx/AGENTS.md\"", "\"/config/cin/AGENTS.md\""),
        (
            r#""D:\\Config\\cx\\AGENTS.md""#,
            r#""D:\\Config\\cin\\AGENTS.md""#,
        ),
        (
            "std::fs::write(first_thread, \"cx-tree-guard\")",
            "std::fs::write(first_thread, \"cin-tree-guard\")",
        ),
        (
            "const MARK: &str = \"cx-child=\";",
            "const MARK: &str = \"cin-child=\";",
        ),
        ("--test-threads=1 cx-demo=x", "--test-threads=1 cin-demo=x"),
        (
            "format!(\"/tmp/cx-{name}-{}\", id)",
            "format!(\"/tmp/cin-{name}-{}\", id)",
        ),
        (
            "format!(\"cx{}\", EXE_SUFFIX)",
            "format!(\"cin{}\", EXE_SUFFIX)",
        ),
        (
            ".starts_with(\"cx: unexpected argument\")",
            ".starts_with(\"cin: unexpected argument\")",
        ),
        (
            "text.replace(\"cortex \", \"cx \")",
            "text.replace(\"cineris \", \"cin \")",
        ),
        (
            ".replace(\"'cortex \", \"'cx \")",
            ".replace(\"'cineris \", \"'cin \")",
        ),
        (
            "all.contains(\"`cx project accept ID…`\")",
            "all.contains(\"`cin project accept ID…`\")",
        ),
        (
            "as_cx.contains(\"Usage: cx \")",
            "as_cx.contains(\"Usage: cin \")",
        ),
        (
            "-E (binary_id(=cx) and test(=a))",
            "-E (binary_id(=cin) and test(=a))",
        ),
        (
            "[\"cx::it a\", \"cx b::c\"]",
            "[\"cx::it a\", \"cin b::c\"]",
        ),
    ] {
        assert_eq!(renamed(old), new, "{old}");
    }
}

#[test]
fn cx_that_is_not_the_alias_stays() {
    for text in [
        "fn plan(&self, args: &Json, cx: &PlanCx)",
        "Must stop when `cx.cancel` fires.",
        "let as_cx = help(topic, name);",
        "binary: \"cx::it\".to_owned(),",
        "cx.paths.root",
        "the tcx and cxx",
        "src/cx.rs and docs/cx.md",
        "let cx = Cx::new();",
        "fn verify<'a>(&'a self, cx: VerifyCx<'a>)",
        "execute(tool.as_ref(), plan, &cx, &mut out)",
        "poll_fn(|cx| waits.poll(cx))",
        "self.verifier.verify(cx).await",
        "(binary_id(=cx::it) and test(=a))",
        "the --cx-flag and x-cx-y",
        "if !cx.asked && cx.round > 1 {",
        "| `cx::it trust::sys_flaky` |",
        "a CX of two",
    ] {
        assert_eq!(renamed(text), text);
        assert!(!(0..text.len()).any(|at| is_alias(text, at)), "{text}");
    }
}

#[test]
fn kept_files_and_lines_keep_their_text() {
    let decision = "# 0013: the command `cortex`, its alias `cx`\nSnowflake's `cortex`.\n";
    assert_eq!(
        rename_text("docs/decisions/0013-naming.md", decision),
        decision
    );
    assert_eq!(
        rename_path("docs/decisions/0013-naming.md"),
        "docs/decisions/0013-naming.md"
    );
    for whole in [
        "xtask/src/rename.rs",
        "xtask/src/rename/branch.rs",
        "xtask/src/rename/tests.rs",
        "docs/decisions/0038-the-project-is-named-cineris.md",
        "docs/steps/rename-cineris/answered.md",
    ] {
        assert_eq!(rename_text(whole, decision), decision, "{whole}");
        assert!(violations(whole, decision.as_bytes()).is_empty(), "{whole}");
    }

    // The build plan's row about the clash keeps its words; the rest of the
    // page is renamed.
    let plan = "| **The name clash step 2 found.** Snowflake's coding-agent CLI \
                installs a command named `cortex`; two vendor tools install `cx`. | \
                **The names stay `cortex` and `cx`.** |\n\
                | The command and its alias | `cortex` and `cx` (steps 2, 13, 34) |\n";
    let out = rename_text("docs/build-plan.md", plan);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], plan.lines().next().unwrap());
    assert_eq!(
        lines[1],
        "| The command and its alias | `cineris` and `cin` (steps 2, 13, 34) |"
    );
    let found = violations("docs/build-plan.md", out.as_bytes());
    assert!(found.is_empty(), "{found:?}");
    // The same row in another file is not kept.
    assert_eq!(violations("docs/other.md", plan.as_bytes()).len(), 2);

    let index = "| [0013](0013-naming.md) | The command `cortex`, its alias `cx` | Superseded |\n\
                 | [0030](0030-process-tree-control-in-cortex-sandbox.md) | In `cortex-sandbox` | Accepted |\n";
    let out = rename_text("docs/decisions/README.md", index);
    assert_eq!(
        out,
        "| [0013](0013-naming.md) | The command `cortex`, its alias `cx` | Superseded |\n\
         | [0030](0030-process-tree-control-in-cineris-sandbox.md) | In `cineris-sandbox` | Accepted |\n"
    );

    // `cx` that is the tools' context, in the comments that say so.
    let contract =
        "    /// in `cx`, and nothing else. That is a handful\n    /// the alias `cx` here\n";
    let out = rename_text("crates/cineris-tools/src/contract.rs", contract);
    assert_eq!(
        out,
        "    /// in `cx`, and nothing else. That is a handful\n    /// the alias `cin` here\n"
    );
    // A kept line is kept wherever the path is renamed from.
    assert_eq!(
        rename_path("crates/cortex-tools/src/contract.rs"),
        "crates/cineris-tools/src/contract.rs"
    );
}

#[test]
fn the_check_finds_a_planted_old_name_and_says_where() {
    let page = "# Cineris\n\nRun `cineris`.\nSet CORTEX_LOG to see it.\nOr `cx`.\n";
    let found = violations("docs/page.md", page.as_bytes());
    let lines: Vec<Option<usize>> = found.iter().map(|violation| violation.line).collect();
    assert_eq!(lines, [Some(4), Some(5)]);
    assert!(
        found[0].message.contains("an old name"),
        "{}",
        found[0].message
    );
    assert!(
        found[1].message.contains("the old alias"),
        "{}",
        found[1].message
    );

    // A file that is not text is not renamed, and the check says so when
    // it holds the old name.
    let found = violations("assets/data.bin", b"\0\x01Cortex\0");
    assert_eq!(found.len(), 1);
    assert!(
        found[0].message.contains("not UTF-8 text"),
        "{}",
        found[0].message
    );

    let found = violations("crates/cortex-x/src/lib.rs", b"fn x() {}\n");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, None);

    assert!(
        violations(
            "docs/page.md",
            "# Cineris\nRun `cineris` or `cin`.\n".as_bytes()
        )
        .is_empty()
    );
    // An image's bytes are not text.
    assert!(violations("assets/logo.png", b"\x89PNG\0\0\0CX.EXE").is_empty());
}

#[test]
fn a_second_pass_changes_nothing() {
    let sample = "Cortex-Harness, `cortex`, `cx`, cortex_base, CORTEX_LOG, .cortex/,\n\
                  ~/.config/cortex-harness/owner-terms, \"cx.exe\", ~/.cache/cortex-harness/corpus\n";
    let once = renamed(sample);
    assert_ne!(once, sample);
    assert_eq!(renamed(&once), once);
    assert_eq!(
        rename_path(&rename_path("crates/cortex-a/x.rs")),
        "crates/cineris-a/x.rs"
    );
}

/// Every file of this repository, renamed twice: the second pass changes
/// neither a text nor a path, and the first leaves nothing for the check.
#[test]
fn the_whole_tree_renamed_once_is_renamed() {
    let root = crate::workspace::root();
    let files = crate::workspace::tracked_files(&root).unwrap();
    assert!(files.len() > 100);
    let none = Formatter::none();
    // On every core: the searches run unoptimised in a test's build.
    let problems: Vec<String> = super::in_parallel(&files, |file| {
        let bytes = std::fs::read(root.join(file)).unwrap();
        let once = transform(file, &bytes, &none);
        let Some(text) = once.contents else {
            // Unchanged: its second pass is its first.
            return violations(&once.path, &bytes)
                .first()
                .map(ToString::to_string);
        };
        let twice = transform(&once.path, &text, &none);
        if twice.path != once.path || twice.contents.is_some() {
            return Some(format!("{file} changes on the second pass"));
        }
        violations(&once.path, &text)
            .first()
            .map(ToString::to_string)
    })
    .into_iter()
    .flatten()
    .collect();
    assert!(problems.is_empty(), "{problems:#?}");
}

#[test]
fn a_lock_is_put_in_cargos_order() {
    let lock = "# head\nversion = 4\n\n\
                [[package]]\nname = \"b-lib\"\nversion = \"1.0.0\"\n\n\
                [[package]]\nname = \"cineris-a\"\nversion = \"0.0.0\"\ndependencies = [\n \"z-lib 0.9.0\",\n \"z-lib 0.10.0\",\n \"cineris-b\",\n \"b-lib\",\n]\n\n\
                [[package]]\nname = \"cineris-b\"\nversion = \"0.0.0\"\n\n\
                [[package]]\nname = \"z-lib\"\nversion = \"0.9.0\"\n\n\
                [[package]]\nname = \"z-lib\"\nversion = \"0.10.0\"\n";
    let shuffled = lock.replace("cineris-", "zz-");
    let sorted = sort_lock(&shuffled);
    assert!(sorted.starts_with("# head\nversion = 4\n\n[[package]]\nname = \"b-lib\""));
    // Versions of one name keep their order (cargo's, by version).
    assert!(
        sorted.find("version = \"0.9.0\"").unwrap() < sorted.find("version = \"0.10.0\"").unwrap()
    );
    assert!(sorted.contains(
        "dependencies = [\n \"b-lib\",\n \"z-lib 0.9.0\",\n \"z-lib 0.10.0\",\n \"zz-b\",\n]\n"
    ));
    assert!(
        sorted.ends_with("name = \"zz-b\"\nversion = \"0.0.0\"\n"),
        "{sorted}"
    );
    assert_eq!(sort_lock(&sorted), sorted);

    // The repository's own locks are in that order already.
    let root = crate::workspace::root();
    for file in ["Cargo.lock", "fuzz/Cargo.lock", "corpus/Cargo.lock"] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        assert_eq!(sort_lock(&text), text, "{file}");
    }
    // A text of another shape is left alone.
    assert_eq!(sort_lock("no packages\n"), "no packages\n");
}

#[test]
fn two_paths_that_would_become_one_are_refused() {
    assert!(check_moves([("a/cortex-x/f", "a/cineris-x/f"), ("b", "b")]).is_ok());
    let error = check_moves([
        ("a/cortex-x/f", "a/cineris-x/f"),
        ("a/cineris-x/f", "a/cineris-x/f"),
    ])
    .unwrap_err();
    assert!(error.contains("would both become a/cineris-x/f"), "{error}");
}

#[test]
fn a_file_that_is_not_text_keeps_its_bytes_and_moves() {
    let none = Formatter::none();
    let image = b"\x89PNG\r\n\x1a\n\0\0cortex";
    let out = transform(
        "assets/cortex-harness-logo-dark-background.png",
        image,
        &none,
    );
    assert_eq!(out.path, "assets/cineris-logo-dark-background.png");
    assert!(out.contents.is_none());
}

/// Commits from before the rename keep the old identity, and new ones carry
/// the new: the owner guard, which reads every commit a push sends, lets
/// both through. It looks for the owner's terms, not for an identity.
#[test]
fn both_identities_pass_the_owner_guard() {
    let terms = crate::owner_guard::Terms::parse("Quentin Example\nq.example@example.invalid\n");
    let log: String = [
        [
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "Cortex-Harness",
            "dev@cortex-harness.invalid",
        ],
        [
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "Cineris",
            "dev@cineris.invalid",
        ],
    ]
    .iter()
    .map(|[hash, name, email]| {
        format!("{hash}\u{1f}{name}\u{1f}{email}\u{1f}{name}\u{1f}{email}\u{1f}A change\n\u{1e}\n")
    })
    .collect();
    assert_eq!(crate::owner_guard::scan_commits(&terms, &log), []);
}

#[test]
fn the_branch_command_reads_its_range_and_names_its_branch() {
    let parsed = Options::parse(&["main..worktree-step-25-wip"]).unwrap();
    assert_eq!(
        parsed,
        Options {
            base: "main".to_owned(),
            tip: "worktree-step-25-wip".to_owned(),
            to: "worktree-step-25-wip-renamed".to_owned(),
            force: false,
        }
    );
    let named =
        Options::parse(&["abc123..origin/worktree-step-25", "--to", "s25", "--force"]).unwrap();
    assert_eq!((named.to.as_str(), named.force), ("s25", true));
    assert_eq!(
        Options::parse(&["a..origin/step"]).unwrap().to,
        "step-renamed"
    );
    assert!(
        Options::parse(&["main..HEAD~2"]).is_err(),
        "a tip that is not a name needs --to"
    );
    assert!(Options::parse(&["main..deadbeef"]).is_err());
    assert!(Options::parse(&["main"]).is_err());
    assert!(Options::parse(&["main...tip"]).is_err());
    assert!(Options::parse(&["..tip"]).is_err());
    assert!(Options::parse(&["a..b", "--to"]).is_err());
    assert!(Options::parse(&["a..b", "--other"]).is_err());
}

#[test]
fn the_pieces_of_a_branch_rewrite_read_and_write_gits_formats() {
    assert_eq!(quote_path("crates/a b/c.rs"), "crates/a b/c.rs");
    assert_eq!(quote_path("\"odd\\name\nx"), "\"\\\"odd\\\\name\\nx\"");

    let listing = b"100644 blob aaaa\tcrates/cortex-a/src/lib.rs\x00100755 blob bbbb\tx\x00120000 blob cccc\tlink\0";
    let entries = parse_tree(listing).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].new_path, "crates/cineris-a/src/lib.rs");
    assert_eq!(
        (entries[1].mode.as_str(), entries[1].id.as_str()),
        ("100755", "bbbb")
    );
    assert!(parse_tree(b"100644 blob\tx\0").is_err());

    let raw =
        b"tree t\nparent p\nauthor Cortex-Harness <dev@cortex-harness.invalid> 1700000000 +0100\n\
                committer Cortex Test <test@cortex-harness.invalid> 1700000060 +0000\n\n\
                Test cortex-tui's `cx` alias\n";
    let text = CommitText::parse(raw).unwrap();
    assert_eq!(text.encoding, None);
    let out = text.renamed();
    assert_eq!(out.author, "Cineris <dev@cineris.invalid> 1700000000 +0100");
    assert_eq!(
        out.committer,
        "Cineris Test <test@cineris.invalid> 1700000060 +0000"
    );
    assert_eq!(out.message, "Test cineris-tui's `cin` alias\n");
    assert!(CommitText::parse(b"tree t\n\nno people\n").is_err());
}

/// A Rust file that `rustfmt` keeps as it is gets formatted again after the
/// rename (a longer name pushes a call over the width; imports are sorted
/// by name); one it would change, a sample of odd code, is left as the
/// names leave it.
#[test]
fn sys_a_rust_file_is_formatted_again_only_when_it_was_formatted() {
    let formatter = Formatter::at(&crate::workspace::root()).unwrap();
    let formatted = "use clap_like::y;\nuse cortex_base::x;\n\nfn f() {\n    \
                     let some_long_variable_name = \
                     cortex_protocol::some_function_name(argument_one, argument_two_x);\n}\n";
    let out = transform(
        "crates/cortex-a/src/lib.rs",
        formatted.as_bytes(),
        &formatter,
    );
    let out = String::from_utf8(out.contents.unwrap()).unwrap();
    assert_eq!(
        out,
        "use cineris_base::x;\nuse clap_like::y;\n\nfn f() {\n    \
         let some_long_variable_name =\n        \
         cineris_protocol::some_function_name(argument_one, argument_two_x);\n}\n"
    );
    let odd = "fn  f( ) { cortex::x() }\n";
    let out = transform(
        "crates/cortex-index/fixtures/odd.rs",
        odd.as_bytes(),
        &formatter,
    );
    assert_eq!(out.contents.unwrap(), b"fn  f( ) { cineris::x() }\n");
}

/// git as these tests run it: none of the machine's configuration, and
/// commits made as the identity the repository had before the rename.
fn git(dir: &Path, arguments: &[&str]) -> String {
    let mut command = std::process::Command::new("git");
    for variable in crate::owner_guard::GIT_LOCATION {
        command.env_remove(variable);
    }
    let output = command
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.autocrlf=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(arguments)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", dir.join("no-such-git-config"))
        .env("GIT_AUTHOR_NAME", "Cortex-Harness")
        .env("GIT_AUTHOR_EMAIL", "dev@cortex-harness.invalid")
        .env("GIT_AUTHOR_DATE", "1700000000 +0100")
        .env("GIT_COMMITTER_NAME", "Cortex Test")
        .env("GIT_COMMITTER_EMAIL", "test@cortex-harness.invalid")
        .env("GIT_COMMITTER_DATE", "1700000600 +0000")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn write(repo: &Path, file: &str, text: &str) {
    let path = repo.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The branch rewrite end to end, in a repository of its own:
///
/// - BASE holds old names; a branch adds two commits on it (one changes a
///   line of a file whose name changes, one adds a file);
/// - "main" is BASE, then `apply`'s output, then a hand edit elsewhere;
/// - the rewrite gives R(BASE) with exactly `apply`'s tree, commits renamed
///   in their trees, paths, messages and identities, and the same commits
///   on a second run;
/// - `git rebase --onto main R(BASE)` then goes through with no conflict.
#[test]
fn sys_a_branch_made_before_the_rename_rebases_onto_the_renamed_main() {
    let scratch = Scratch::outside("xtask-rename", "branch");
    let repo = scratch.git_init("repo");
    let lines: String = (1..=12)
        .map(|n| format!("line {n} of cortex-a\n"))
        .collect();
    write(&repo, "crates/cortex-a/notes.md", &lines);
    write(
        &repo,
        "crates/cortex-a/src/lib.rs",
        "pub fn cortex_a() {}\n",
    );
    write(
        &repo,
        "README.md",
        "# Cortex-Harness\n\nRun `cortex`, or `cx`.\n",
    );
    write(&repo, "image.bin", "\0cortex\0");
    git(&repo, &["add", "--all"]);
    git(&repo, &["commit", "--quiet", "-m", "Start cortex"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);

    git(&repo, &["switch", "--quiet", "-c", "step"]);
    write(
        &repo,
        "crates/cortex-a/notes.md",
        &lines.replace("line 10 of cortex-a", "line 10, changed on the step"),
    );
    git(
        &repo,
        &["commit", "--quiet", "-am", "Change cortex-a's notes"],
    );
    write(&repo, "crates/cortex-a/src/new.rs", "use cortex_a::x;\n");
    git(&repo, &["add", "--all"]);
    git(
        &repo,
        &["commit", "--quiet", "-m", "Add a file to cortex-a"],
    );

    // main: the tool's output, then a hand edit near the top of the notes.
    git(&repo, &["switch", "--quiet", "main"]);
    let applied = super::apply_tree(&repo, &Formatter::none()).unwrap();
    assert_eq!((applied.written, applied.moved), (3, 2));
    git(&repo, &["add", "--all"]);
    git(&repo, &["commit", "--quiet", "-m", "The tool's output"]);
    let tool_tree = git(&repo, &["rev-parse", "HEAD^{tree}"]);
    let notes = std::fs::read_to_string(repo.join("crates/cineris-a/notes.md")).unwrap();
    write(
        &repo,
        "crates/cineris-a/notes.md",
        &notes.replace("line 2 of", "line 2, by hand, of"),
    );
    git(&repo, &["commit", "--quiet", "-am", "A hand edit"]);
    // A second run of the tool on main changes nothing.
    let again = super::apply_tree(&repo, &Formatter::none()).unwrap();
    assert_eq!((again.written, again.moved), (0, 0));

    let at = branch::Git::at(&repo);
    let first =
        branch::rewrite(&at, &base, &at.commit("step").unwrap(), &Formatter::none()).unwrap();
    assert_eq!(first.commits, 2);
    assert_eq!(
        git(&repo, &["rev-parse", &format!("{}^{{tree}}", first.base)]),
        tool_tree
    );
    assert_eq!(
        git(&repo, &["rev-parse", &format!("{}^", first.base)]),
        base
    );
    let second =
        branch::rewrite(&at, &base, &at.commit("step").unwrap(), &Formatter::none()).unwrap();
    assert_eq!(
        (second.base.as_str(), second.tip.as_str()),
        (first.base.as_str(), first.tip.as_str())
    );

    assert_eq!(
        git(
            &repo,
            &[
                "log",
                "--date=raw",
                "--format=%an <%ae> %ad|%cn <%ce> %cd|%s",
                &format!("{}..{}", first.base, first.tip)
            ]
        ),
        "Cineris <dev@cineris.invalid> 1700000000 +0100|\
         Cineris Test <test@cineris.invalid> 1700000600 +0000|Add a file to cineris-a\n\
         Cineris <dev@cineris.invalid> 1700000000 +0100|\
         Cineris Test <test@cineris.invalid> 1700000600 +0000|Change cineris-a's notes"
    );
    assert_eq!(
        git(
            &repo,
            &[
                "show",
                &format!("{}:crates/cineris-a/src/new.rs", first.tip)
            ]
        ),
        "use cineris_a::x;"
    );
    assert!(
        git(&repo, &["ls-tree", "-r", "--name-only", &first.tip])
            .lines()
            .all(|path| !path.contains("cortex"))
    );
    // No ref is left behind but the ones the test made.
    assert_eq!(
        git(&repo, &["for-each-ref", "--format=%(refname)"]),
        "refs/heads/main\nrefs/heads/step"
    );

    // The command: it names the new branch, finds it there on a second
    // run, and replaces one that holds other commits only with --force,
    // and never while a worktree is rebasing it.
    let options = |force: bool| branch::Options {
        base: base.clone(),
        tip: "step".to_owned(),
        to: "step-renamed".to_owned(),
        force,
    };
    let said = branch::run_in(&at, &options(false), &Formatter::none()).unwrap();
    assert!(
        said.contains(&format!(
            "git rebase --onto origin/main {} step-renamed",
            first.base
        )),
        "{said}"
    );
    assert_eq!(git(&repo, &["rev-parse", "step-renamed"]), first.tip);
    let said = branch::run_in(&at, &options(false), &Formatter::none()).unwrap();
    assert!(said.contains("already holds these commits"), "{said}");
    git(&repo, &["update-ref", "refs/heads/step-renamed", &base]);
    let refused = branch::run_in(&at, &options(false), &Formatter::none()).unwrap_err();
    assert!(refused.contains("--force replaces it"), "{refused}");
    assert_eq!(git(&repo, &["rev-parse", "step-renamed"]), base);
    let rebasing = repo.join(".git").join("rebase-merge");
    std::fs::create_dir_all(&rebasing).unwrap();
    std::fs::write(rebasing.join("head-name"), "refs/heads/step-renamed\n").unwrap();
    let refused = branch::run_in(&at, &options(true), &Formatter::none()).unwrap_err();
    assert!(refused.contains("being rebased"), "{refused}");
    assert_eq!(git(&repo, &["rev-parse", "step-renamed"]), base);
    std::fs::remove_dir_all(&rebasing).unwrap();
    branch::run_in(&at, &options(true), &Formatter::none()).unwrap();
    assert_eq!(git(&repo, &["rev-parse", "step-renamed"]), first.tip);
    git(
        &repo,
        &[
            "rebase",
            "--quiet",
            "--onto",
            "main",
            &first.base,
            "step-renamed",
        ],
    );
    let notes = std::fs::read_to_string(repo.join("crates/cineris-a/notes.md")).unwrap();
    assert!(notes.contains("line 2, by hand, of cineris-a"), "{notes}");
    assert!(notes.contains("line 10, changed on the step\n"), "{notes}");
    assert!(repo.join("crates/cineris-a/src/new.rs").is_file());
    assert_eq!(
        std::fs::read(repo.join("image.bin")).unwrap(),
        b"\0cortex\0"
    );

    // A range with a merge in it is refused.
    git(&repo, &["switch", "--quiet", "-c", "merged", &base]);
    git(
        &repo,
        &["merge", "--quiet", "--no-ff", "-m", "Merge", "step"],
    );
    let error = branch::rewrite(
        &at,
        &base,
        &at.commit("merged").unwrap(),
        &Formatter::none(),
    )
    .err()
    .unwrap();
    assert!(error.contains("2 parents"), "{error}");
}

/// The tool's own files keep their text through the rename, so their code
/// must not name a crate of the workspace: it would name the old crate
/// after the rename. The test kit comes through `crate::testkit`.
#[test]
fn the_tools_kept_code_names_no_crate_of_the_workspace() {
    let root = crate::workspace::root();
    for file in [
        "xtask/src/rename.rs",
        "xtask/src/rename/branch.rs",
        "xtask/src/rename/tests.rs",
    ] {
        let text = std::fs::read_to_string(root.join(file)).unwrap();
        for (index, line) in text.lines().enumerate() {
            // Code, not a string or a comment that names one.
            let code = line.split("//").next().unwrap_or_default();
            if code.contains('"') {
                continue;
            }
            for crate_prefix in ["cortex_", "cineris_"] {
                assert!(
                    !code.contains(&format!("{crate_prefix}testkit"))
                        && !code
                            .trim_start()
                            .starts_with(&format!("use {crate_prefix}")),
                    "{file}:{}: {line}",
                    index + 1
                );
            }
        }
    }
}

/// The repo rules, which the lint job runs, hold every file to the check: a
/// name planted in a page, a crate's path or the alias in a command fails
/// them, and the same text in a kept line does not.
#[test]
fn the_repo_rules_find_a_planted_old_name() {
    let rules = crate::rules::file_rules;
    let found = rules(
        "docs/page.md",
        b"# Cineris\n\nSet CORTEX_LOG.\nThen run `cx --version`.\n",
    );
    let lines: Vec<Option<usize>> = found.iter().map(|violation| violation.line).collect();
    assert_eq!(lines, [Some(3), Some(4)], "{found:?}");
    assert_eq!(rules("crates/cortex-x/src/lib.rs", b"").len(), 1);
    assert!(rules("docs/page.md", b"# Cineris\n\nRun `cin --version`.\n").is_empty());
    let row = b"| **The name clash step 2 found.** Snowflake's CLI installs `cortex`. |\n";
    assert!(rules("docs/build-plan.md", row).is_empty());
    assert_eq!(rules("docs/other.md", row).len(), 1);
}

/// A symbolic link is refused, by `apply` and by `branch` alike: `apply`
/// would read and write through it, and neither renames its target.
#[cfg(unix)]
#[test]
fn sys_a_symbolic_link_is_refused_by_both() {
    let scratch = Scratch::outside("xtask-rename", "links");
    let repo = scratch.git_init("repo");
    write(&repo, "notes.md", "cortex\n");
    std::os::unix::fs::symlink("notes.md", repo.join("link.md")).unwrap();
    git(&repo, &["add", "--all"]);
    git(&repo, &["commit", "--quiet", "-m", "Start"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    write(&repo, "more.md", "more\n");
    git(&repo, &["add", "--all"]);
    git(&repo, &["commit", "--quiet", "-m", "More"]);

    let error = super::apply_tree(&repo, &Formatter::none()).err().unwrap();
    assert!(error.contains("link.md is a symbolic link"), "{error}");
    assert_eq!(
        std::fs::read_to_string(repo.join("notes.md")).unwrap(),
        "cortex\n"
    );

    let at = branch::Git::at(&repo);
    let error = branch::rewrite(&at, &base, &at.commit("HEAD").unwrap(), &Formatter::none())
        .err()
        .unwrap();
    assert!(error.contains("link.md in"), "{error}");
    // It stopped fast-import as asked: no crash report, no ref left behind.
    let crashed = std::fs::read_dir(repo.join(".git")).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("fast_import_crash")
    });
    assert!(!crashed);
    assert_eq!(
        git(&repo, &["for-each-ref", "--format=%(refname)"]),
        "refs/heads/main"
    );
}
