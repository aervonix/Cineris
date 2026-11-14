use std::time::Duration;

use cortex_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::testing::{DARK, assert_snapshot, problems, sections, variants};
use crate::theme::Theme;

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

fn texts(lines: &[Line]) -> Vec<String> {
    lines.iter().map(|line| line.text().into_owned()).collect()
}

/// The reference scenario's diff: the edit to `scan_number`.
pub(crate) fn scan_number_diff() -> Vec<DiffLine> {
    let line = |number, change, text: &str| DiffLine {
        number,
        change,
        text: text.to_owned(),
        emphasis: Vec::new(),
    };
    vec![
        line(141, Change::Same, "    while let Some(c) = self.peek() {"),
        line(
            142,
            Change::Removed,
            "        if !c.is_ascii_digit() { break; }",
        ),
        line(
            142,
            Change::Added,
            "        if c == '_' && self.peek_next().is_some_and(|d| d.is_ascii_digit()) {",
        ),
        line(143, Change::Added, "            self.bump();"),
        line(144, Change::Added, "            continue;"),
        line(145, Change::Added, "        }"),
        line(
            146,
            Change::Added,
            "        if !c.is_ascii_digit() { break; }",
        ),
        line(147, Change::Same, "        self.bump();"),
    ]
}

/// The reference scenario's round: a read, an edit and a failing test run.
pub(crate) fn scenario() -> Round {
    Round {
        calls: vec![
            Call {
                verb: "Read".to_owned(),
                target: Target::Path("crates/parser/src/lex.rs".to_owned()),
                state: CallState::Succeeded,
                measure: Measure::Lines { first: 120, last: 178 },
                took: None,
                detail: Detail::None,
            },
            Call {
                verb: "Edit".to_owned(),
                target: Target::Path("crates/parser/src/lex.rs".to_owned()),
                state: CallState::Succeeded,
                measure: Measure::Changes { added: 5, removed: 1 },
                took: None,
                detail: Detail::Diff(scan_number_diff()),
            },
            Call {
                verb: "Run".to_owned(),
                target: Target::Text("cargo test -p parser".to_owned()),
                state: CallState::Failed,
                measure: Measure::Note("exit 101".to_owned()),
                took: Some(Duration::from_millis(3_400)),
                detail: Detail::Output(
                    "test lex::numbers::separators ... ok\ntest lex::numbers::trailing_separator ... FAILED\n\
                     assertion failed: lex(\"1_\").is_err()\n\
                     test result: FAILED. 47 passed; 1 failed; 0 ignored; finished in 0.02s\n"
                        .to_owned(),
                ),
            },
        ],
        open: None,
    }
}

#[test]
fn a_committed_round_opens_a_small_diff_and_a_failures_tail() {
    let lines = round(&scenario(), Placement::Committed, 0, 99, &look());
    assert_eq!(
        texts(&lines),
        [
            format!("╭{}╮", "─".repeat(97)),
            format!("│ ✓ Read lex.rs  lines 120–178{}│", " ".repeat(68)),
            format!("│ ✓ Edit lex.rs  +5 −1{}│", " ".repeat(76)),
            format!(
                "│    141       while let Some(c) = self.peek() {{{}│",
                " ".repeat(50)
            ),
            format!(
                "│    142 −         if !c.is_ascii_digit() {{ break; }}{}│",
                " ".repeat(46)
            ),
            format!(
                "│    142 +         if c == '_' && self.peek_next().is_some_and(|d| d.is_ascii_digit()) {{{}│",
                " ".repeat(10)
            ),
            format!("│    143 +             self.bump();{}│", " ".repeat(63)),
            format!("│    144 +             continue;{}│", " ".repeat(66)),
            format!("│    145 +         }}{}│", " ".repeat(78)),
            format!(
                "│    146 +         if !c.is_ascii_digit() {{ break; }}{}│",
                " ".repeat(46)
            ),
            format!("│    147           self.bump();{}│", " ".repeat(67)),
            format!(
                "│ ✗ Run cargo test -p parser  exit 101 · 3.4s{}│",
                " ".repeat(53)
            ),
            format!(
                "│   test lex::numbers::trailing_separator ... FAILED{}│",
                " ".repeat(46)
            ),
            format!(
                "│   assertion failed: lex(\"1_\").is_err(){}│",
                " ".repeat(58)
            ),
            format!(
                "│   test result: FAILED. 47 passed; 1 failed; 0 ignored; finished in 0.02s{}│",
                " ".repeat(24)
            ),
            format!("╰{}╯", "─".repeat(97)),
        ]
    );
    // Nothing is hidden, so the border says nothing.
    assert!(lines.iter().all(|line| line.width() == 99));
}

#[test]
fn a_live_box_says_how_to_open_or_close_it_and_a_committed_one_how_to_see_more() {
    let look = look();
    let mut running = scenario();
    running.calls[2].state = CallState::Running(Duration::from_millis(1_200));
    running.calls[2].measure = Measure::None;
    running.calls[2].took = None;
    assert!(running.is_running());
    let live = round(&running, Placement::Live, 2, 99, &look);
    assert!(
        live.last()
            .unwrap()
            .text()
            .ends_with(" ctrl+t collapses ─╯")
    );
    assert!(
        texts(&live)
            .iter()
            .any(|text| text.starts_with("│ ⣻ Run cargo test -p parser  1.2s "))
    );
    // A running command's output is closed by default…
    assert!(
        !texts(&live)
            .iter()
            .any(|text| text.contains("trailing_separator"))
    );
    // …and Ctrl+T closes everything.
    running.open = Some(false);
    let closed = round(&running, Placement::Live, 2, 99, &look);
    assert_eq!(closed.len(), 5);
    assert!(
        closed
            .last()
            .unwrap()
            .text()
            .ends_with(" ctrl+t expands ─╯")
    );
    // Opened, the running command shows its last four lines.
    running.open = Some(true);
    let open = round(&running, Placement::Live, 2, 99, &look);
    let text = texts(&open).join("\n");
    assert!(
        text.contains("test lex::numbers::separators ... ok"),
        "{text}"
    );
    // Committed and closed: ctrl+o shows the rest.
    let mut done = scenario();
    done.open = Some(false);
    let committed = round(&done, Placement::Committed, 0, 99, &look);
    assert!(
        committed
            .last()
            .unwrap()
            .text()
            .ends_with(" ctrl+o expands ─╯")
    );
    // In the viewer the selected box's border is the accent's.
    let viewer = round(
        &done,
        Placement::Viewer {
            selected: true,
            open: false,
        },
        0,
        99,
        &look,
    );
    assert_eq!(
        viewer[0].spans()[0].style(),
        look.theme.style(Token::BorderSelected)
    );
    assert!(viewer.last().unwrap().text().ends_with(" enter expands ─╯"));
    let opened = round(
        &done,
        Placement::Viewer {
            selected: true,
            open: true,
        },
        0,
        99,
        &look,
    );
    // Opened in the viewer: the whole output, not a tail.
    assert!(texts(&opened).join("\n").contains("separators ... ok"));
}

#[test]
fn an_open_live_box_shows_at_most_tool_detail_rows() {
    let mut look = look();
    look.tool_detail = 5;
    let mut running = scenario();
    running.calls[2].state = CallState::Running(Duration::from_secs(1));
    running.open = Some(true);
    let lines = round(&running, Placement::Live, 0, 99, &look);
    let detail = lines.len() - 2 - running.calls.len();
    assert_eq!(detail, 5, "{:#?}", texts(&lines));
    assert!(
        texts(&lines)
            .iter()
            .any(|text| text.contains("… 8 more lines"))
    );
    // Committed, the box keeps all of it.
    let mut done = scenario();
    done.open = Some(true);
    let committed = round(&done, Placement::Committed, 0, 99, &look);
    assert_eq!(committed.len() - 2 - done.calls.len(), 8 + 3);
}

#[test]
fn tool_output_always_and_never_and_a_long_diff() {
    let mut look = look();
    look.tool_output = ToolOutput::Never;
    assert_eq!(
        round(&scenario(), Placement::Committed, 0, 99, &look).len(),
        5
    );
    look.tool_output = ToolOutput::Always;
    let all = round(&scenario(), Placement::Committed, 0, 99, &look);
    assert_eq!(all.len(), 2 + 3 + 8 + 3);
    // A diff longer than `tui.tool_detail` starts closed.
    look.tool_output = ToolOutput::Auto;
    look.tool_detail = 7;
    let long = round(&scenario(), Placement::Committed, 0, 99, &look);
    assert!(!texts(&long).iter().any(|text| text.contains("self.peek()")));
    assert!(long.last().unwrap().text().ends_with(" ctrl+o expands ─╯"));
}

#[test]
fn every_state_has_its_mark() {
    let look = look();
    let mut round_ = scenario();
    let marks = [
        (CallState::Waiting, "?"),
        (CallState::Denied, "⊘"),
        (CallState::Interrupted, "■"),
        (CallState::Succeeded, "✓"),
        (CallState::Failed, "✗"),
    ];
    for (state, mark) in marks {
        round_.calls[0].state = state;
        let lines = round(&round_, Placement::Committed, 0, 99, &look);
        assert!(
            lines[1].text().starts_with(&format!("│ {mark} Read")),
            "{state:?}"
        );
    }
    round_.calls[0].state = CallState::Waiting;
    round_.calls[0].measure = Measure::None;
    let lines = round(&round_, Placement::Live, 0, 99, &look);
    assert!(lines[1].text().contains("approval needed"));
    let ascii = Look::new(look.theme.clone(), &Glyphs::ASCII);
    let lines = round(&scenario(), Placement::Committed, 0, 99, &ascii);
    assert!(
        lines[1]
            .text()
            .starts_with("| + Read lex.rs  lines 120-178")
    );
    assert!(lines[2].text().starts_with("| + Edit lex.rs  +5 -1"));
    assert!(
        texts(&lines)
            .iter()
            .any(|text| text.starts_with("| x Run cargo test -p parser  exit 101 | 3.4s"))
    );
}

#[test]
fn paths_names_and_output_are_shown_safely() {
    let look = look();
    let mut round_ = scenario();
    // A Windows path is shortened at its backslashes too.
    round_.calls[0].target = Target::Path(r"C:\work\parser\src\lex.rs".to_owned());
    // Control characters in a file name and a command are made visible.
    round_.calls[1].target = Target::Path("src/evil\u{1b}]0;x\u{7}.rs".to_owned());
    round_.calls[2].target = Target::Text("echo \u{9b}2J\nrm".to_owned());
    // Windows line endings, a carriage return redrawing a progress line, a
    // tab and an escape in output.
    round_.calls[2].detail = Detail::Output(
        "Building [=>  ] 1/3\rBuilding [===] 3/3\r\nfailed:\tsee\u{1b}[31m red\r\nlast\r\n"
            .to_owned(),
    );
    let lines = round(&round_, Placement::Committed, 0, 99, &look);
    let text = texts(&lines).join("\n");
    assert!(text.contains("Read lex.rs  "), "{text}");
    assert!(text.contains("Edit evil\u{241b}]0;x\u{2407}.rs"), "{text}");
    assert!(text.contains("Run echo \u{fffd}2J rm"), "{text}");
    assert!(text.contains("│   Building [===] 3/3 "), "{text}");
    assert!(text.contains("│   failed: see\u{241b}[31m red"), "{text}");
    for width in [10, 40, 59, 79, 119] {
        let lines = round(&round_, Placement::Committed, 0, width, &look);
        let found = problems(&lines, width);
        assert!(found.is_empty(), "{width}: {found:?}");
    }
}

#[test]
fn a_line_too_wide_is_cut_and_keeps_its_tint() {
    let look = look();
    let lines = round(&scenario(), Placement::Committed, 0, 59, &look);
    let added = lines
        .iter()
        .find(|line| line.text().contains("self.peek_next()"))
        .unwrap();
    assert_eq!(
        added.text(),
        "│    142 +         if c == '_' && self.peek_next().is_so… │"
    );
    let tint = look.theme.style(Token::DiffAddLine);
    let ellipsis = added
        .spans()
        .iter()
        .find(|span| span.text().ends_with('…'))
        .unwrap();
    assert_eq!(ellipsis.style(), tint);
    // Wide characters and emoji in a diff are cut between clusters.
    let mut wide = scenario();
    if let Detail::Diff(lines) = &mut wide.calls[1].detail {
        lines[2].text = "    let 名前 = \"👨\u{200d}👩\u{200d}👧 e\u{301}\"; ".repeat(4);
    }
    for width in [20, 59, 79] {
        let lines = round(&wide, Placement::Committed, 0, width, &look);
        assert!(problems(&lines, width).is_empty());
    }
}

#[test]
fn changed_words_are_bold() {
    let look = look();
    let mut round_ = scenario();
    if let Detail::Diff(lines) = &mut round_.calls[1].detail {
        lines[1].emphasis.push(12..30);
    }
    let lines = round(&round_, Placement::Committed, 0, 99, &look);
    let removed = &lines[4];
    let bold: String = removed
        .spans()
        .iter()
        .filter(|span| span.style().bold)
        .map(Span::text)
        .collect();
    assert_eq!(bold, "c.is_ascii_digit()");
}

#[test]
fn snapshot() {
    let mut parts = Vec::new();
    let mut live = scenario();
    live.calls[2].state = CallState::Running(Duration::from_millis(1_200));
    live.calls[2].measure = Measure::None;
    live.calls[2].took = None;
    for variant in variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        let mut lines = round(&scenario(), Placement::Committed, 0, variant.width(), &look);
        lines.extend(round(&live, Placement::Live, 2, variant.width(), &look));
        let mut closed = live.clone();
        closed.open = Some(false);
        lines.extend(round(&closed, Placement::Live, 2, variant.width(), &look));
        parts.push((variant.title(), lines));
    }
    assert_snapshot("round", &sections(&parts));
}
