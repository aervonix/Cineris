#![cfg_attr(
    not(feature = "highlight"),
    allow(
        dead_code,
        unused_imports,
        reason = "the tests of the worker need the feature"
    )
)]

use std::time::Duration;

use cortex_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::markdown::Markdown;
use crate::testing::DARK;
use crate::theme::Theme;
use crate::view::Look;

/// The longest a test waits for the worker. It is never reached when all is
/// well: the worker answers in milliseconds.
const BOUND: Duration = Duration::from_secs(8);

fn look() -> Look {
    Look::new(
        Theme::new(ColorDepth::TrueColor, Some(DARK)),
        &Glyphs::UNICODE,
    )
}

fn texts(lines: &[cortex_term::Line]) -> Vec<String> {
    lines.iter().map(|line| line.text().into_owned()).collect()
}

#[test]
fn off_colours_nothing_and_starts_nothing() {
    let off = Highlighter::off();
    assert_eq!(off.stage(), Stage::Off);
    assert_eq!(off.open("rust"), None);
    assert_eq!(off.worker_thread(), None);
}

#[cfg(feature = "highlight")]
#[test]
fn the_worker_starts_on_first_use_off_the_callers_thread_and_colours_by_scope() {
    let highlighter = Highlighter::new(|| {});
    assert_eq!(highlighter.stage(), Stage::Idle);
    assert_eq!(highlighter.worker_thread(), None);
    // `open` starts the worker and returns at once. Asked from a thread of
    // its own, it answers within the bound: were the worker run on its
    // caller's thread, the caller would never get its answer.
    let opener = highlighter.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("opener".to_owned())
        .spawn(move || {
            let _ = sender.send((opener.open("rust"), std::thread::current().id()));
        })
        .unwrap();
    let (block, caller) = receiver
        .recv_timeout(BOUND)
        .expect("`open` returned without running the worker on its caller's thread");
    let block = block.unwrap();
    highlighter.line(block, 0, "let s = \"text\"; // a note");
    highlighter.line(block, 1, "if n == 1_000 { return Err(self.e()); }");
    assert!(highlighter.settle(BOUND), "the worker answered in time");
    assert_eq!(highlighter.stage(), Stage::Ready);
    // The work was done on another thread than the caller's.
    let worker = highlighter.worker_thread().unwrap();
    assert_ne!(worker, caller);
    assert_ne!(worker, std::thread::current().id());
    let Outcome::Coloured(first) = highlighter.take(block, 0) else {
        panic!("no colours");
    };
    let text = "let s = \"text\"; // a note";
    let token_of = |word: &str, colours: &Colours| {
        let at = text.find(word).unwrap();
        colours
            .iter()
            .find(|(range, _)| range.contains(&at))
            .map(|(_, token)| *token)
    };
    assert_eq!(token_of("let", &first), Some(Token::SyntaxKeyword));
    assert_eq!(token_of("\"text\"", &first), Some(Token::SyntaxString));
    assert_eq!(token_of("// a note", &first), Some(Token::SyntaxComment));
    assert_eq!(token_of("s =", &first), None);
    // Taken once: the second time it is gone.
    assert_eq!(highlighter.take(block, 0), Outcome::Waiting);
    // An unknown language is plain.
    let unknown = highlighter.open("no-such-language").unwrap();
    highlighter.line(unknown, 0, "anything");
    assert!(highlighter.settle(BOUND));
    assert_eq!(highlighter.take(unknown, 0), Outcome::Plain);
    // A line too long is plain at once, and the block goes on.
    highlighter.line(block, 2, &"x".repeat(LONGEST + 1));
    assert_eq!(highlighter.take(block, 2), Outcome::Plain);
}

/// An answer with a code block, streamed with `highlighter`.
fn streamed(highlighter: &Highlighter) -> Markdown {
    let mut stream = Markdown::answer().with_highlighter(highlighter.clone());
    stream.push("Code:\n\n```rust\nlet n = 1;\n");
    stream
}

#[cfg(feature = "highlight")]
#[test]
fn a_code_line_commits_in_its_colours_once_the_worker_has_them() {
    let look = look();
    let highlighter = Highlighter::new(|| {});
    // The grammars load as the first block opens.
    let mut warm = Markdown::answer().with_highlighter(highlighter.clone());
    warm.push("```rust\nx\n```\n");
    let _ = warm.commit(79, &look);
    assert!(highlighter.settle(BOUND));
    // Held, so that the line is certainly waiting when the commit looks: a
    // quick worker could otherwise have its colours ready already.
    highlighter.hold(true);
    let mut stream = streamed(&highlighter);
    let first = stream.commit(79, &look);
    // The paragraph and the blank row are committed; the code line waits for
    // its colours.
    assert_eq!(texts(&first), ["◇ Code:", ""]);
    highlighter.hold(false);
    assert!(highlighter.settle(BOUND));
    let lines = stream.commit(79, &look);
    assert_eq!(texts(&lines), ["  let n = 1;"]);
    let keyword = lines[0]
        .spans()
        .iter()
        .find(|span| span.text() == "let")
        .unwrap();
    assert_eq!(keyword.style(), look.theme.style(Token::SyntaxKeyword));
}

#[cfg(feature = "highlight")]
#[test]
fn a_frame_never_waits_for_the_worker() {
    let look = look();
    let highlighter = Highlighter::new(|| {});
    let mut warm = Markdown::answer().with_highlighter(highlighter.clone());
    warm.push("```rust\nx\n```\n");
    let _ = warm.commit(79, &look);
    assert!(highlighter.settle(BOUND));
    // The worker takes no job, as if a line had it stuck.
    highlighter.hold(true);
    let mut stream = streamed(&highlighter);
    stream.set_now(Duration::from_secs(100));
    assert_eq!(texts(&stream.commit(79, &look)), ["◇ Code:", ""]);
    // A frame: the line is drawn plain in the live region, at once.
    assert_eq!(texts(&stream.live(79, &look)), ["  let n = 1;"]);
    // Its wait has not run out: it is not committed.
    stream.set_now(Duration::from_secs(100) + WAIT / 2);
    assert!(stream.commit(79, &look).is_empty());
    // Text after it waits behind it, in order.
    stream.push("```\n\nAfter.\n");
    assert!(stream.commit(79, &look).is_empty());
    // When the wait runs out it commits plain, and what follows with it.
    stream.set_now(Duration::from_secs(100) + WAIT);
    let lines = stream.commit(79, &look);
    assert_eq!(texts(&lines), ["  let n = 1;"]);
    // The paragraph after it is still open: its row is live.
    assert_eq!(texts(&stream.live(79, &look)), ["", "  After."]);
    assert!(
        lines[0]
            .spans()
            .iter()
            .all(|span| span.style() == cortex_term::Style::PLAIN)
    );
    highlighter.hold(false);
    assert!(highlighter.settle(BOUND));
}

#[cfg(feature = "highlight")]
#[test]
fn the_first_block_waits_for_the_grammars_no_longer_than_any_line_waits() {
    let look = look();
    let highlighter = Highlighter::new(|| {});
    highlighter.hold(true);
    let mut stream = streamed(&highlighter);
    stream.set_now(Duration::from_secs(5));
    // The grammars may still be loading: the line waits, drawn plain live.
    assert_eq!(texts(&stream.commit(79, &look)), ["◇ Code:", ""]);
    assert_eq!(texts(&stream.live(79, &look)), ["  let n = 1;"]);
    // Its wait runs out: it commits plain.
    stream.set_now(Duration::from_secs(5) + WAIT);
    assert_eq!(texts(&stream.commit(79, &look)), ["  let n = 1;"]);
    highlighter.hold(false);
    assert!(highlighter.settle(BOUND));
}

#[cfg(feature = "highlight")]
#[test]
fn a_code_line_that_waited_while_the_window_shrank_fits_it_and_drops_nothing_after() {
    let look = look();
    let highlighter = Highlighter::new(|| {});
    highlighter.hold(true);
    let mut stream = Markdown::answer().with_highlighter(highlighter.clone());
    stream.set_now(Duration::from_secs(1));
    stream.push(
        "```rust\nlet a_rather_long_name = some_function(with, several, arguments);\n```\n\n",
    );
    let mut lines = stream.commit(79, &look);
    // A paragraph after it, committed at 79 columns while the code line
    // waits: its rows queue behind it.
    let paragraph = "A paragraph after the block, long enough to wrap at seventy-nine \
                     columns, with words enough for two rows of them, and then the end";
    // Quoted: its rows have marks before them.
    stream.push("> ");
    stream.push(paragraph);
    stream.push(" here.\n");
    lines.extend(stream.commit(79, &look));
    // The window shrinks to 30 columns, and the wait runs out.
    stream.set_now(Duration::from_secs(1) + WAIT);
    let after = stream.commit(30, &look);
    assert!(!after.is_empty());
    assert!(
        after.iter().all(|line| line.width() <= 30),
        "{:?}",
        texts(&after)
    );
    // The code line is cut where the window ends, as code is: one row, its
    // end an ellipsis.
    assert!(
        after[0].text().contains("let a_rather"),
        "{:?}",
        after[0].text()
    );
    assert!(after[0].text().ends_with('…'), "{:?}", after[0].text());
    // The quote's rows, wrapped to the window, keep their marks' indent.
    for line in after.iter().skip(1).filter(|line| line.width() > 0) {
        assert!(line.text().starts_with("  "), "{:?}", texts(&after));
    }
    assert!(
        !after
            .iter()
            .skip(1)
            .any(|line| line.text().contains("arguments"))
    );
    lines.extend(after);
    stream.finish();
    lines.extend(stream.commit(30, &look));
    let shown: String = texts(&lines).join(" ");
    let words: Vec<&str> = shown.split_whitespace().collect();
    for word in paragraph.split_whitespace() {
        assert!(words.contains(&word), "{word} is missing from {shown:?}");
    }
    highlighter.hold(false);
    assert!(highlighter.settle(BOUND));
}

#[cfg(feature = "highlight")]
#[test]
fn a_frame_drawn_once_the_colours_are_in_leaves_them_for_the_commit() {
    let look = look();
    let highlighter = Highlighter::new(|| {});
    let mut warm = Markdown::answer().with_highlighter(highlighter.clone());
    warm.push("```rust\nx\n```\n");
    let _ = warm.commit(79, &look);
    assert!(highlighter.settle(BOUND));
    highlighter.hold(true);
    let mut stream = streamed(&highlighter);
    assert_eq!(texts(&stream.commit(79, &look)), ["◇ Code:", ""]);
    highlighter.hold(false);
    assert!(highlighter.settle(BOUND));
    let keyword = |lines: &[cortex_term::Line]| {
        lines
            .iter()
            .flat_map(|line| line.spans().to_vec())
            .find(|span| span.text() == "let")
            .map(|span| span.style())
    };
    let want = Some(look.theme.style(Token::SyntaxKeyword));
    // A frame draws the line live, in its colours, and only looks at them.
    assert_eq!(keyword(&stream.live(79, &look)), want);
    assert_eq!(keyword(&stream.live(79, &look)), want);
    // The commit after it still finds them.
    assert_eq!(keyword(&stream.commit(79, &look)), want);
}
