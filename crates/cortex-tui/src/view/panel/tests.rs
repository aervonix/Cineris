use cortex_term::ColorDepth;

use super::*;
use crate::glyphs::Glyphs;
use crate::testing::{self, DARK};
use crate::theme::Theme;

fn look(glyphs: &'static Glyphs) -> Look {
    Look::new(Theme::new(ColorDepth::TrueColor, Some(DARK)), glyphs)
}

#[test]
fn the_more_and_folded_rows_are_dim_and_name_the_key() {
    let unicode = look(&Glyphs::UNICODE);
    let muted = unicode.theme.style(Token::Muted);
    let cut_row = more(3, 80, &unicode);
    assert_eq!(cut_row.text(), "  … 3 more");
    assert_eq!(cut_row.spans()[1].style(), muted);
    let folded_row = folded(2, Some("ctrl+p"), 80, &unicode);
    assert_eq!(folded_row.text(), "+2 panels · ctrl+p");
    assert_eq!(folded_row.spans()[0].style(), muted);
    assert_eq!(folded(1, None, 80, &unicode).text(), "+1 panel");
    let ascii = look(&Glyphs::ASCII);
    assert_eq!(more(3, 80, &ascii).text(), "  ... 3 more");
    assert_eq!(
        folded(2, Some("ctrl+p"), 80, &ascii).text(),
        "+2 panels | ctrl+p"
    );
}

#[test]
fn every_row_keeps_to_its_width_and_shows_what_acts_unseen() {
    for glyphs in [&Glyphs::UNICODE, &Glyphs::ASCII] {
        let look = look(glyphs);
        for width in 1..=40 {
            let lines = [
                more(12_345, width, &look),
                folded(7, Some("ctrl+\u{202e}p\u{1b}[2J"), width, &look),
                header(
                    "Plan\u{200b}\nhere",
                    &[Span::plain("  2 of 3")],
                    width,
                    &look,
                ),
            ];
            let problems = testing::problems(&lines, width);
            assert!(problems.is_empty(), "at {width}: {problems:#?}");
        }
    }
}
