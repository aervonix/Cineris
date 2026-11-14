//! [`brief`]: a long text in an assertion's message, cut to what can be
//! read.
//!
//! A failed assertion that prints a whole request, or a notice of ten
//! kilobytes of bulk, puts the result it is about off the screen: step 15's
//! review lost one that way. `assert!(text.contains(x), "{}", brief(&text))`
//! prints a short text whole and a long one as its size, its first words
//! and its last.

/// How long a text is shown whole.
const WHOLE: usize = 400;

/// How much of each end of a longer text is shown.
const END: usize = 120;

/// `text` if it is short; otherwise `N bytes: <the start> … <the end>`,
/// each end cut at a character's edge.
#[must_use]
pub fn brief(text: &str) -> String {
    if text.len() <= WHOLE {
        return text.to_owned();
    }
    let mut head = END;
    while !text.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = text.len() - END;
    while !text.is_char_boundary(tail) {
        tail += 1;
    }
    format!(
        "{} bytes: {} … {}",
        text.len(),
        &text[..head],
        &text[tail..]
    )
}

#[cfg(test)]
mod tests {
    use super::brief;

    #[test]
    fn a_short_text_is_whole_and_a_long_one_is_its_size_and_ends() {
        assert_eq!(brief("the result"), "the result");
        let long = format!("start {} end of it", "x".repeat(10_000));
        let shown = brief(&long);
        assert!(shown.starts_with("10016 bytes: start xxx"), "{shown}");
        assert!(shown.ends_with("xxx end of it"), "{shown}");
        assert!(shown.len() < 300, "{}", shown.len());
        // A character across the cut is not split.
        let wide = "é".repeat(500);
        let shown = brief(&wide);
        assert!(shown.starts_with("1000 bytes: é"), "{shown}");
    }
}
