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

const PROMPT: &str = "1_000 lexes as 1. Fix scan_number so digit separators work, and add a test.";
const SUMMARY: &str = "The loop breaks on any non-digit, so `_` ends the literal. Check whether a trailing `_` is meant to be an error first.";

#[test]
fn blocks_lead_with_their_glyph_and_wrap_two_columns_in() {
    let look = look();
    assert_eq!(
        texts(&user(PROMPT, 59, &look)),
        [
            "◆ 1_000 lexes as 1. Fix scan_number so digit separators",
            "  work, and add a test.",
        ]
    );
    let lines = user(PROMPT, 99, &look);
    assert_eq!(lines[0].spans()[0].style(), look.theme.style(Token::User));
    assert!(lines[0].spans()[2].style().bold);
    assert_eq!(
        texts(&notice(
            "The sandbox stopped a command writing ~/.cargo/credentials.toml.",
            59,
            &look
        )),
        [
            "· The sandbox stopped a command writing",
            "  ~/.cargo/credentials.toml."
        ]
    );
    assert_eq!(
        texts(&error(
            "The request failed after 5 attempts.",
            Some("Press enter to try the turn again."),
            99,
            &look
        )),
        [
            "✗ The request failed after 5 attempts.",
            "  Press enter to try the turn again."
        ]
    );
    assert_eq!(
        texts(&reasoning(SUMMARY, Duration::from_secs(6), 99, &look)),
        [
            "∴ Reasoned for 6s",
            "  The loop breaks on any non-digit, so _ ends the literal. Check whether a",
            "  trailing _ is meant to be an error first.",
        ]
    );
    let lines = reasoning(SUMMARY, Duration::from_secs(6), 99, &look);
    // After its two columns, the summary is dim italic.
    assert!(lines[1].spans()[1..].iter().all(|span| span.style().italic));
}

#[test]
fn reasoning_shows_its_duration_alone_when_asked() {
    let mut look = look();
    look.reasoning = Reasoning::Duration;
    assert_eq!(
        texts(&reasoning(SUMMARY, Duration::from_secs(72), 99, &look)),
        ["∴ Reasoned for 1m 12s"]
    );
    look.reasoning = Reasoning::Summary;
    look.density = Density::Compact;
    assert_eq!(
        reasoning(SUMMARY, Duration::from_secs(6), 99, &look).len(),
        1
    );
}

#[test]
fn live_reasoning_shows_its_latest_words() {
    let look = look();
    let text = "First the scanner. Then the loop breaks on any non-digit and `_` ends the literal; \
                before changing it, check whether a trailing";
    let lines = reasoning_live(text, Duration::from_secs(3), 3, 99, &look);
    assert_eq!(lines[0].text(), "⢿ Reasoning  3s");
    assert_eq!(lines[0].spans()[0].style(), look.theme.style(Token::Accent));
    let latest = lines[1].text();
    assert!(latest.ends_with("check whether a trailing"), "{latest}");
    assert!(latest.starts_with("  …"), "{latest}");
    assert_eq!(lines[1].width(), 80);
    // Nothing yet: the first row alone.
    assert_eq!(reasoning_live("", Duration::ZERO, 0, 99, &look).len(), 1);
}

#[test]
fn the_turn_summary_is_off_unless_asked_for() {
    let mut look = look();
    assert!(
        turn_summary(
            Duration::from_secs(72),
            Some(12_400),
            Some("$0.08"),
            99,
            &look
        )
        .is_empty()
    );
    look.turn_summary = true;
    assert_eq!(
        texts(&turn_summary(
            Duration::from_secs(72),
            Some(12_400),
            Some("$0.08"),
            99,
            &look
        )),
        ["  1m 12s · 12.4k tokens · $0.08"]
    );
    assert_eq!(count(412), "412");
    assert_eq!(count(1_250_000), "1.2M");
}

#[test]
fn content_is_sanitised_and_never_too_wide() {
    let look = look();
    // Control characters, and what acts unseen and is not one: a
    // bidirectional override, a zero-width space and a tag character.
    let hostile =
        "a\u{1b}[2Jb\u{9b}c\u{7}\td 漢字👨\u{200d}👩\u{200d}👧 e\u{301} f\u{202e}\u{200b}\u{e0041} "
            .repeat(5);
    for width in [1, 2, 10, 59, 79, 119] {
        let mut lines = listing(&hostile, &[hostile.clone(), hostile.clone()], width, &look);
        lines.extend(user(&hostile, width, &look));
        lines.extend(notice(&hostile, width, &look));
        lines.extend(error(&hostile, Some(&hostile), width, &look));
        lines.extend(reasoning(&hostile, Duration::from_secs(1), width, &look));
        lines.extend(reasoning_live(
            &hostile,
            Duration::from_secs(1),
            0,
            width,
            &look,
        ));
        let found = problems(&lines, width);
        assert!(found.is_empty(), "{width}: {found:?}");
    }
}

#[test]
fn snapshot() {
    let mut parts = Vec::new();
    for variant in variants() {
        let mut look = Look::new(variant.theme.clone(), variant.glyphs);
        look.turn_summary = true;
        let width = variant.width();
        let mut lines = user(PROMPT, width, &look);
        lines.extend(notice(
            "Context compacted in the background: 148.2k → 23.6k tokens (a 3.1k summary, the last 6 turns kept).",
            width,
            &look,
        ));
        lines.extend(error(
            "The request failed after 5 attempts: the connection to the provider was reset.",
            Some("Press enter to try the turn again, or pick another model with /model."),
            width,
            &look,
        ));
        lines.extend(reasoning(SUMMARY, Duration::from_secs(6), width, &look));
        lines.extend(reasoning_live(
            SUMMARY,
            Duration::from_secs(3),
            3,
            width,
            &look,
        ));
        lines.extend(turn_summary(
            Duration::from_secs(72),
            Some(12_400),
            Some("$0.08"),
            width,
            &look,
        ));
        parts.push((variant.title(), lines));
    }
    assert_snapshot("blocks", &sections(&parts));
}
