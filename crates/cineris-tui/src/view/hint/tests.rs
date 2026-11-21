use std::time::Duration;

use cineris_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::testing::{DARK, assert_snapshot, problems, sections, variants};
use crate::theme::Theme;
use crate::view::{Header, header};

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

fn running() -> HintRow {
    HintRow {
        left: Hint::Running {
            elapsed: Duration::from_secs(14),
            tick: 0,
            keys: RunningKeys::Steer,
        },
        model: "opus-5.5".to_owned(),
        effort: Some("high".to_owned()),
        speed: Some(Speed {
            name: "fast".to_owned(),
            cooldown_until: None,
        }),
        context: Some(31),
        budget: None,
        limit: None,
        access: None,
        extras: Vec::new(),
    }
}

#[test]
fn the_hint_row_as_the_reference_screens_show_it() {
    let look = look();
    let row = hint_row(&running(), 99, &look);
    assert_eq!(
        row.text(),
        format!(
            "  ⣾ 14s  esc stops · enter steers this turn · tab queues it instead{}opus-5.5 high · fast  ",
            " ".repeat(10)
        )
    );
    assert_eq!(row.width(), 99);
    // `fast` is accent text; the rest is the hint's.
    let fast = row
        .spans()
        .iter()
        .find(|span| span.text() == "fast")
        .unwrap();
    assert_eq!(fast.style(), look.theme.style(Token::AccentText));
    // At 60 columns the left side is cut.
    assert_eq!(
        hint_row(&running(), 59, &look).text(),
        "  ⣾ 14s  esc stops · enter steers …  opus-5.5 high · fast  "
    );
    // Quitting armed, which step 21's input reports.
    let mut armed = running();
    armed.left = Hint::QuitArmed;
    armed.speed = None;
    assert!(
        hint_row(&armed, 99, &look)
            .text()
            .starts_with("  ctrl+c again quits  ")
    );
    // Idle in the first sessions; after a turn, its cost.
    let mut idle = running();
    idle.left = Hint::FirstSessions;
    idle.speed = None;
    assert_eq!(
        hint_row(&idle, 99, &look).text().trim_end(),
        format!(
            "  / commands · @ files · alt+enter new line{}opus-5.5 high",
            " ".repeat(41)
        )
    );
}

#[test]
fn queueing_and_a_cooldown_and_the_context() {
    let look = look();
    let mut row = running();
    row.left = Hint::Running {
        elapsed: Duration::from_secs(3),
        tick: 1,
        keys: RunningKeys::Queue,
    };
    let text = hint_row(&row, 119, &look).text().into_owned();
    assert!(
        text.contains("⣽ 3s  esc stops · enter queues it · tab steers this turn instead"),
        "{text}"
    );
    row.speed = Some(Speed {
        name: "fast".to_owned(),
        cooldown_until: Some("14:32".to_owned()),
    });
    row.context = Some(72);
    let line = hint_row(&row, 119, &look);
    assert!(
        line.text().contains("fast after 14:32 · 72% context"),
        "{}",
        line.text()
    );
    let context = line
        .spans()
        .iter()
        .find(|span| span.text() == "72% context")
        .unwrap();
    assert_eq!(context.style(), look.theme.style(Token::Warning));
    row.context = Some(93);
    let line = hint_row(&row, 119, &look);
    let context = line
        .spans()
        .iter()
        .find(|span| span.text() == "93% context")
        .unwrap();
    assert_eq!(context.style(), look.theme.style(Token::Error));
}

#[test]
fn a_narrow_row_drops_segments_from_the_right_and_under_50_keeps_the_model_and_context() {
    let look = look();
    let mut row = running();
    row.context = Some(80);
    row.access = Some("sandbox off".to_owned());
    row.extras = vec!["2 queued".to_owned()];
    let wide = hint_row(&row, 159, &look).text().into_owned();
    assert!(
        wide.ends_with("opus-5.5 high · fast · 80% context · sandbox off · 2 queued  "),
        "{wide}"
    );
    let mid = hint_row(&row, 79, &look).text().into_owned();
    assert!(!mid.contains("2 queued"), "{mid}");
    assert!(mid.contains("opus-5.5 high"), "{mid}");
    let narrow = hint_row(&row, 45, &look).text().into_owned();
    assert_eq!(narrow.trim(), "opus-5.5 high · 80% context");
    for width in [1, 5, 20, 45, 59, 79, 119] {
        assert!(
            problems(&[hint_row(&row, width, &look)], width).is_empty(),
            "{width}"
        );
    }
}

#[test]
fn snapshot() {
    let header_state = Header {
        version: "0.1.0".to_owned(),
        model: "opus-5.5".to_owned(),
        effort: Some("high".to_owned()),
        folder: "~/src/parser".to_owned(),
        branch: Some("main".to_owned()),
        profile: Some("Knows 4 commands and 3 conventions here. /project shows them.".to_owned()),
    };
    let mut parts = Vec::new();
    for variant in variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        let width = variant.width();
        let mut lines = header(&header_state, width, &look);
        lines.push(hint_row(&running(), width, &look));
        let mut cost = running();
        cost.left = Hint::Cost {
            turn: "$0.03".to_owned(),
            session: "$0.61".to_owned(),
        };
        cost.speed = None;
        cost.context = Some(91);
        lines.push(hint_row(&cost, width, &look));
        let mut keys = running();
        keys.left = Hint::Keys(vec![
            "esc denies".to_owned(),
            "ctrl+o shows it whole".to_owned(),
        ]);
        lines.push(hint_row(&keys, width, &look));
        parts.push((variant.title(), lines));
    }
    assert_snapshot("header-and-hint", &sections(&parts));
}

#[test]
fn a_tab_or_a_line_break_in_a_one_row_field_reaches_no_line() {
    // Every field these views draw on one row, given a tab and a line
    // break: each is a space on the screen.
    let look = look();
    let odd = "a\tb\nc";
    let header = header(
        &Header {
            version: odd.to_owned(),
            model: odd.to_owned(),
            effort: Some(odd.to_owned()),
            folder: odd.to_owned(),
            branch: Some(odd.to_owned()),
            profile: Some(odd.to_owned()),
        },
        99,
        &look,
    );
    let row = HintRow {
        left: Hint::Cost {
            turn: odd.to_owned(),
            session: odd.to_owned(),
        },
        model: odd.to_owned(),
        effort: Some(odd.to_owned()),
        speed: Some(Speed {
            name: odd.to_owned(),
            cooldown_until: Some(odd.to_owned()),
        }),
        context: Some(31),
        budget: Some(odd.to_owned()),
        limit: Some(odd.to_owned()),
        access: Some(odd.to_owned()),
        extras: vec![odd.to_owned()],
    };
    let mut keys = row.clone();
    keys.left = Hint::Keys(vec![odd.to_owned()]);
    let mut lines = header;
    lines.push(hint_row(&row, 199, &look));
    lines.push(hint_row(&keys, 199, &look));
    let mut summary_look = look.clone();
    summary_look.turn_summary = true;
    lines.extend(crate::view::turn_summary(
        Duration::from_secs(3),
        None,
        Some(odd),
        99,
        &summary_look,
    ));
    lines.extend(crate::view::round(
        &crate::view::Round {
            calls: vec![crate::view::Call {
                verb: odd.to_owned(),
                target: crate::view::Target::Path(format!("src/{odd}.rs")),
                state: crate::view::CallState::Succeeded,
                measure: crate::view::Measure::Note(odd.to_owned()),
                took: Some(Duration::from_millis(100)),
                detail: crate::view::Detail::None,
                hook: None,
            }],
            open: Some(false),
        },
        crate::view::Placement::Committed,
        0,
        99,
        &look,
    ));
    let found = problems(&lines, 199);
    assert!(found.is_empty(), "{found:?}");
    assert!(lines.iter().any(|line| line.text().contains("a b c")));
}
