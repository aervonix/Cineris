use cortex_base::Rng as _;
use cortex_base::SeededRng;
use cortex_term::ColorDepth;
use cortex_testkit::cases::{Outcome, check, ensure};

use super::*;
use crate::glyphs::Glyphs;
use crate::testing::{DARK, problems};
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

/// Feeds `pieces` one at a time, collecting what each commit gives, then
/// finishes.
fn streamed(flavor: Flavor, pieces: &[&str], width: usize, look: &Look) -> (Vec<Line>, Markdown) {
    let mut stream = Markdown::new(flavor);
    let mut lines = Vec::new();
    for piece in pieces {
        stream.push(piece);
        lines.extend(stream.commit(width, look));
        // The live rows are drawn between commits, as a frame would.
        let _ = stream.live(width, look);
    }
    stream.finish();
    lines.extend(stream.commit(width, look));
    (lines, stream)
}

/// The document's lines, fed all at once.
fn at_once(text: &str, width: usize, look: &Look) -> Vec<Line> {
    Markdown::render(Flavor::Answer, text, width, look)
}

/// The document's lines, each paragraph laid out whole when it ends.
fn whole(text: &str, width: usize, look: &Look) -> Vec<Line> {
    let mut stream = Markdown::answer().whole();
    stream.push(text);
    stream.finish();
    stream.commit(width, look)
}

const ANSWER: &str = "\
`scan_number` stops at the first character that is not a digit, so `1_000` ends after the `1`. \
I'll read the scanner and allow `_` between digits.

One test fails: `1_` must be an error, but the scanner now stops before the `_` and returns `1`. \
A separator has to be followed by a digit, so the other case becomes an error:

```rust
if c == '_' {
    if !self.peek_next().is_some_and(|d| d.is_ascii_digit()) {
        return Err(self.error(\"digit separator must be followed by a digit\"));
    }
}
```
";

#[test]
fn an_answer_is_drawn_as_the_reference_screens_draw_it() {
    let lines = at_once(ANSWER, 99, &look());
    assert_eq!(
        texts(&lines),
        [
            "◇ scan_number stops at the first character that is not a digit, so 1_000 ends",
            "  after the 1. I'll read the scanner and allow _ between digits.",
            "",
            "  One test fails: 1_ must be an error, but the scanner now stops before the _",
            "  and returns 1. A separator has to be followed by a digit, so the other case",
            "  becomes an error:",
            "",
            "  if c == '_' {",
            "      if !self.peek_next().is_some_and(|d| d.is_ascii_digit()) {",
            "          return Err(self.error(\"digit separator must be followed by a digit\"));",
            "      }",
            "  }",
        ]
    );
    // At 60 columns prose wraps at 59 and code is cut with an ellipsis.
    let narrow = at_once(ANSWER, 59, &look());
    assert_eq!(
        narrow[0].text(),
        "◇ scan_number stops at the first character that is not a"
    );
    assert!(
        narrow.iter().any(
            |line| line.text() == "      if !self.peek_next().is_some_and(|d| d.is_ascii_digi…"
        )
    );
    for line in &narrow {
        assert!(line.width() <= 59, "{:?}", line.text());
    }
}

#[test]
fn blocks_are_drawn_as_09_says() {
    let text = "\
# Heading *one*

- first item
- second item, long enough that it wraps onto a second row here
  1. nested
  2. numbers

> a quote
> that goes on

---

| tool | time |
|---|--:|
| read | 1s |
| run | 12s |

See [the docs](https://e.example/d) and <https://e.example>.
";
    let lines = at_once(text, 50, &look());
    assert_eq!(
        texts(&lines),
        [
            "◇ Heading one",
            "",
            "  – first item",
            "  – second item, long enough that it wraps onto a",
            "    second row here",
            "    1. nested",
            "    2. numbers",
            "",
            "  │ a quote that goes on",
            "",
            "  ────────────────────────────────────────────────",
            "",
            "  tool  time",
            "  ──────────",
            "  read    1s",
            "  run    12s",
            "",
            "  See the docs (https://e.example/d) and",
            "  https://e.example.",
        ]
    );
    let heading = &lines[0];
    assert!(heading.spans()[1].style().bold);
    assert!(
        heading
            .spans()
            .iter()
            .any(|span| span.text() == "one" && span.style().italic)
    );
    // With hyperlinks the address is the link's, and not written out.
    let mut linked = look();
    linked.hyperlinks = true;
    let lines = at_once("See [the docs](https://e.example/d).", 50, &linked);
    assert_eq!(texts(&lines), ["◇ See the docs."]);
    let span = lines[0]
        .spans()
        .iter()
        .find(|span| span.text() == "the docs")
        .unwrap();
    assert_eq!(span.url(), Some("https://e.example/d"));
    assert!(span.style().underline);
}

#[test]
fn a_stream_cut_short_ends_what_it_has() {
    let look = look();
    // An unclosed fence ends with the text.
    let (lines, _) = streamed(
        Flavor::Answer,
        &["Code:\n\n```\nlet a = 1;\nlet b"],
        40,
        &look,
    );
    assert_eq!(texts(&lines), ["◇ Code:", "", "  let a = 1;", "  let b"]);
    // A paragraph interrupted mid-word, a table mid-row, a list mid-item.
    for (text, last) in [
        ("A sentence that stops in the mid", "  mid"),
        ("| a | b |\n|---|---|\n| 1 |", "  1"),
        ("- one\n- tw", "  – tw"),
    ] {
        let (lines, _) = streamed(Flavor::Answer, &[text], 30, &look);
        assert_eq!(
            lines.last().map(|line| line.text().into_owned()).as_deref(),
            Some(last),
            "{text:?}"
        );
    }
}

#[test]
fn content_reaches_no_line_unsanitised_and_no_line_is_too_wide() {
    let look = look();
    let text = "Escape \u{1b}[31mred\u{1b}[0m and C1 \u{9b}2J and a bell\u{7}.\n\n```\nprintf '\u{1b}]0;title\u{7}'\n\tindented\n```\n\n| k\u{1b} | v |\n|---|---|\n| \u{9b}x | 漢字漢字 |\n";
    for width in [1, 2, 10, 30, 59, 79, 119] {
        let lines = at_once(text, width, &look);
        let found = problems(&lines, width);
        assert!(found.is_empty(), "{width}: {found:?}");
        if width >= 30 {
            let all: String = texts(&lines).concat();
            assert!(
                all.contains('\u{241b}'),
                "the escape is shown as its symbol at {width}"
            );
        }
    }
    // Streamed, split at every point: a line's text reaches the paragraph
    // early, before the line ends, and the rest of it after; both are
    // sanitised.
    for split in (0..=text.len()).filter(|&at| text.is_char_boundary(at)) {
        let (lines, _) = streamed(Flavor::Answer, &[&text[..split], &text[split..]], 30, &look);
        let found = problems(&lines, 30);
        assert!(found.is_empty(), "split at {split}: {found:?}");
    }
}

#[test]
fn a_mark_left_open_holds_its_row_for_two_more_then_stays_literal() {
    let look = look();
    let words = "word ".repeat(40);
    // `**` never closes: the first row is held while three more begin, then
    // committed as it is.
    let text = format!("**{words}");
    let mut stream = Markdown::answer();
    stream.push(&text);
    let committed = stream.commit(30, &look);
    assert_eq!(committed.len(), 7, "{:?}", texts(&committed));
    assert_eq!(stream.forced(), 1);
    assert!(committed[0].text().starts_with("◇ **word"));
    // A closing mark after that cannot reach back: the asterisks stay.
    stream.push("end**\n");
    stream.finish();
    let rest = stream.commit(30, &look);
    assert!(rest.last().unwrap().text().ends_with("end**"));
    // A mark that closes within two rows is matched, and nothing is forced.
    let mut short = Markdown::answer();
    short.push("**bold for a while** and then plain words that go on for a few rows of text\n");
    short.finish();
    let lines = short.commit(30, &look);
    assert_eq!(short.forced(), 0);
    assert!(
        lines[0]
            .spans()
            .iter()
            .any(|span| span.text().starts_with("bold") && span.style().bold)
    );
}

/// Documents for the commit properties: the reference answer, and each
/// construct, with marks that span rows and marks that never close.
fn documents() -> Vec<String> {
    vec![
        ANSWER.to_owned(),
        "A paragraph with **bold that spans several words** and `code that does too`, \
         a [link with words](https://e.example/a \"its title\"), *emphasis*, and words \
         after it all, enough for many rows.\n\nA second one.\n"
            .to_owned(),
        "Text with an unclosed `tick and **unclosed bold that goes on for long enough to \
         hold rows, more and more words follow it here, and more, and more still.\n"
            .to_owned(),
        "# A heading\n\n- an item that is long enough to wrap\n- another\n  - nested *em*\n\n> quote \
         **b** and\nlazy line\n\n1. one\n2. two\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nafter\n"
            .to_owned(),
        "漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字漢字 and e\u{301}e\u{301} \
         👨\u{200d}👩\u{200d}👧 families 🇫🇷 flags in a long line of text\n"
            .to_owned(),
        "line one  \nhard break, then a\\\nbackslash break, then &amp; &copy; \\* escapes\r\nand CRLF\r\n"
            .to_owned(),
        "| x |\nnot a delimiter\n| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\ntext\n".to_owned(),
        "```\nunclosed fence\nwith `ticks` and **marks**".to_owned(),
    ]
}

#[test]
fn token_by_token_commits_what_all_at_once_does_split_at_every_point() {
    // A pull request checks every seventh split of each document at one
    // width; the nightly run checks every split at three.
    let look = look();
    let (step, widths): (usize, &[usize]) = if cortex_testkit::cases::exhaustive() {
        (1, &[20, 59, 79])
    } else {
        (7, &[59])
    };
    for document in documents() {
        for &width in widths {
            let expected = at_once(&document, width, &look);
            let boundaries: Vec<usize> = (0..=document.len())
                .filter(|&at| document.is_char_boundary(at))
                .collect();
            for &split in boundaries.iter().step_by(step) {
                let (lines, _) = streamed(
                    Flavor::Answer,
                    &[&document[..split], &document[split..]],
                    width,
                    &look,
                );
                assert_eq!(
                    texts(&lines),
                    texts(&expected),
                    "split at {split} of {document:?} at {width}"
                );
                assert_eq!(
                    lines, expected,
                    "styles differ: split at {split} at {width}"
                );
            }
        }
    }
}

/// Where two runs' lines first differ, for a failure message.
fn difference(a: &[Line], b: &[Line]) -> String {
    let at = a
        .iter()
        .zip(b)
        .position(|(x, y)| x != y)
        .unwrap_or(a.len().min(b.len()));
    let show = |lines: &[Line]| {
        lines
            .iter()
            .skip(at.saturating_sub(1))
            .take(3)
            .map(|line| {
                format!(
                    "{:?} {:?}",
                    line.text(),
                    line.spans()
                        .iter()
                        .map(|s| (s.style(), s.url()))
                        .collect::<Vec<_>>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n    ")
    };
    format!(
        "at line {at} of {} and {}:\n  first:\n    {}\n  second:\n    {}",
        a.len(),
        b.len(),
        show(a),
        show(b)
    )
}

/// A random document of blocks, with markup that opens and closes.
fn random_document(rng: &mut SeededRng) -> String {
    // Runs long enough to be decided inside, with marks in them: one word
    // in twenty, so that the documents stay short.
    const LONG: [&str; 4] = [
        "/_static/long_name_of_a_file_that_goes_on_for_more_than_sixty_four_bytes_",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa_",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb*",
        "ccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc`",
    ];
    const WORDS: [&str; 18] = [
        // Control characters, which reach no line as they are.
        "\u{7}",
        "\u{1b}[31m",
        "word",
        "the",
        "scanner",
        "**bold",
        "bold**",
        "*em",
        "em*",
        "`code",
        "code`",
        "[link",
        "](https://e.example)",
        "漢字",
        "e\u{301}",
        "_x_",
        "&amp;",
        "\\*",
    ];
    let mut text = String::new();
    for _ in 0..rng.below(8) {
        let start = match rng.below(9) {
            0 => "# ",
            1 => "- ",
            2 => "1. ",
            3 => "> ",
            4 => "  ",
            5 => "| a | b |\n|---|---|\n| ",
            6 => "```\n",
            _ => "",
        };
        text.push_str(start);
        for _ in 0..rng.below(30) {
            let word = if rng.below(20) == 0 {
                LONG[usize::try_from(rng.below(LONG.len() as u64)).unwrap()]
            } else {
                WORDS[usize::try_from(rng.below(WORDS.len() as u64)).unwrap()]
            };
            text.push_str(word);
            // Now and then no space at all: words run together.
            text.push_str(match rng.below(12) {
                0 => "\n",
                1 => "",
                _ => " ",
            });
        }
        text.push('\n');
        if rng.below(2) == 0 {
            text.push('\n');
        }
    }
    text
}

/// `text` cut into pieces at random places, on character boundaries.
fn random_split(rng: &mut SeededRng, text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut at = usize::try_from(rng.below(12)).unwrap() + 1;
        at = at.min(rest.len());
        while !rest.is_char_boundary(at) {
            at += 1;
        }
        pieces.push(rest[..at].to_owned());
        rest = &rest[at..];
    }
    pieces
}

#[test]
fn token_by_token_commits_what_all_at_once_does_for_random_documents() {
    let look = look();
    check(
        "token by token commits what all at once does for random documents",
        50,
        |rng| {
            let document = random_document(rng);
            let pieces = random_split(rng, &document);
            let width = 10 + usize::try_from(rng.below(70)).unwrap();
            (document, pieces, width)
        },
        |(document, pieces, width)| -> Outcome {
            let pieces: Vec<&str> = pieces.iter().map(String::as_str).collect();
            let (lines, stream) = streamed(Flavor::Answer, &pieces, *width, &look);
            let expected = at_once(document, *width, &look);
            ensure!(
                lines == expected,
                "token by token and all at once differ: {}",
                difference(&lines, &expected)
            );
            // Where no row was committed before its marks closed, the lines are
            // those of the document laid out whole: nothing was committed early.
            if stream.forced() == 0 {
                let laid_whole = whole(document, *width, &look);
                ensure!(
                    lines == laid_whole,
                    "streamed and whole differ: {}",
                    difference(&lines, &laid_whole)
                );
            }
            let found = problems(&lines, *width);
            ensure!(found.is_empty(), "{found:?}");
            Ok(())
        },
    );
}

#[test]
fn rows_commit_while_the_paragraph_goes_on() {
    // Only the growing row stays live: the live region does not grow with
    // the paragraph.
    let look = look();
    let mut stream = Markdown::answer();
    for _ in 0..200 {
        stream.push("several plain words ");
        let _ = stream.commit(40, &look);
        assert!(
            stream.live(40, &look).len() <= 2,
            "{:?}",
            texts(&stream.live(40, &look))
        );
    }
}

#[test]
fn nothing_finished_is_parsed_again() {
    // A long document, token by token: the inline parser reads each byte a
    // bounded number of times, however long the document is. Parsing a
    // finished block again would make this grow with the document.
    let look = look();
    // Paragraphs long enough that, were committed rows never dropped, the
    // decisions spaced past `KEPT` would still parse too much.
    let paragraph =
        "Some **words** with `code` and more words to fill the rows of a paragraph. ".repeat(200);
    let document = format!("{paragraph}\n\n").repeat(2);
    let mut stream = Markdown::answer();
    for piece in document.split_inclusive(' ') {
        stream.push(piece);
        let _ = stream.commit(80, &look);
    }
    stream.finish();
    let _ = stream.commit(80, &look);
    let ratio = stream.parsed_bytes() / document.len();
    assert!(ratio < 60, "each byte parsed {ratio} times");
}

#[test]
fn snapshot() {
    // Every element markdown draws, in every variant: a heading, inline code,
    // emphasis, a link, lists, a quote, a rule, a table and a code block.
    // (A table too wide for the room is `table::tests`'s.)
    let document = "\
# The *scanner* and `scan_number`

A paragraph with **bold**, *italic*, `code`, a [link](https://e.example/docs) and e\u{301} 漢字 👨\u{200d}👩\u{200d}👧.

- an item
- another, long enough to wrap onto a second row at sixty columns
  1. nested
> a quote with `code`

---

| tool | what it did | took |
|---|---|--:|
| read | read the scanner from line 120 to line 178 | 0.1s |
| run | ran the tests of the parser crate | 3.4s |

```rust
if c == '_' && self.peek_next().is_some_and(|d| d.is_ascii_digit()) { continue; }
```
";
    let mut parts = Vec::new();
    for variant in crate::testing::variants() {
        let look = Look::new(variant.theme.clone(), variant.glyphs);
        parts.push((
            variant.title(),
            Markdown::render(Flavor::Answer, document, variant.width(), &look),
        ));
    }
    crate::testing::assert_snapshot("markdown", &crate::testing::sections(&parts));
}

/// Feeds `text` in pieces of `size` bytes (on character boundaries),
/// committing after each, and checks the live rows stay few. Returns the
/// lines and the stream.
fn fed(
    text: &str,
    size: usize,
    width: usize,
    look: &Look,
    live_most: usize,
) -> (Vec<Line>, Markdown) {
    let mut stream = Markdown::answer();
    let mut lines = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let mut end = (at + size).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        stream.push(&text[at..end]);
        lines.extend(stream.commit(width, look));
        let live = stream.live(width, look);
        assert!(
            live.len() <= live_most,
            "{} live rows: {:?}",
            live.len(),
            texts(&live)
        );
        at = end;
    }
    stream.finish();
    lines.extend(stream.commit(width, look));
    (lines, stream)
}

#[test]
fn a_very_long_word_does_not_keep_its_rows_live_or_cost_more_than_its_length() {
    let look = look();
    // A blob with no white space in it, as a model may write a hash or a
    // base64 string in prose, with wide characters and an entity inside.
    let blob = "AbCdEf0123456789+/漢字&amp;".repeat(150);
    let text = format!("The data: {blob} and after.\n");
    // A decision every 64 bytes lets a row and a half build up between two.
    let (lines, stream) = fed(&text, 7, 40, &look, 6);
    assert_eq!(lines, at_once(&text, 40, &look));
    let ratio = stream.parsed_bytes() / text.len();
    assert!(ratio < 60, "each byte parsed {ratio} times");
    assert!(problems(&lines, 40).is_empty());
}

#[test]
fn deep_nesting_and_many_marks_cost_no_more_than_their_length() {
    let look = look();
    let quotes = format!("{}deep\n", "> ".repeat(300));
    // At forty levels the items' text has one column left.
    let lists: String = (0..40)
        .map(|depth| format!("{}- level {depth}\n", "  ".repeat(depth)))
        .collect();
    let stars = "*a".repeat(1_500);
    let brackets = "[x ".repeat(800);
    for text in [quotes, lists, stars, brackets] {
        // With one column, an item's rows are many.
        let (lines, stream) = fed(&text, 13, 79, &look, 32);
        assert_eq!(lines, at_once(&text, 79, &look), "{}", &text[..40]);
        // At worst `LIMIT` bytes kept and a decision every `LONG_RUN`.
        let ratio = stream.parsed_bytes() / text.len().max(1);
        assert!(
            ratio < 2 * super::LIMIT / super::LONG_RUN,
            "each byte of {:?}… parsed {ratio} times",
            &text[..20]
        );
        assert!(problems(&lines, 79).is_empty());
    }
}

#[test]
fn a_decision_inside_a_long_word_never_commits_what_the_rest_of_it_changes() {
    // Each run is decided inside, 64 bytes in: where the text before the
    // cut would end with a mark, the decision waits for a byte after which
    // nothing can change. A mark here still open is held, then frozen; none
    // of them closes later, so the whole layout shows them as they are too.
    let look = look();
    let a = "a".repeat(62);
    let b = "b".repeat(30);
    let documents = [
        // An `_` at the cut is inside a word, and closes nothing.
        format!("x _{a}_{b} and after\n"),
        // One backtick at the cut is the start of two.
        format!("x `{a}``{b} and after\n"),
        // `*` and `**`, whose pairing the rule of three decides: a run cut
        // short would pair with the mark before it.
        format!("x *{a}**{b} and after\n"),
        format!("x **{a}*{b} and after\n"),
        // A link whose `](` meets at the cut, and an entity across it.
        format!("x {a}[link](https://e.example/x){b} and after\n"),
        format!("x {a}&amp;{b} and after\n"),
        // A path in prose with `_` all through it.
        format!("see /_static/{a}_name_of_it_{a}_and_on and after\n"),
    ];
    for document in &documents {
        let expected = whole(document, 30, &look);
        for split in (0..=document.len()).filter(|&at| document.is_char_boundary(at)) {
            let (lines, _) = streamed(
                Flavor::Answer,
                &[&document[..split], &document[split..]],
                30,
                &look,
            );
            assert_eq!(
                lines,
                expected,
                "{document:?} split at {split}: {}",
                difference(&lines, &expected)
            );
        }
    }
}

/// What `lines` show, without white space or the answer's mark: equal for
/// two layouts of the same text at any widths.
fn letters(lines: &[Line]) -> String {
    lines
        .iter()
        .flat_map(|line| line.text().chars().collect::<Vec<_>>())
        .filter(|c| !c.is_whitespace() && *c != '◇')
        .collect()
}

#[test]
fn a_window_resized_while_a_paragraph_streams_loses_and_repeats_nothing() {
    // Bold across a row's break, so that no clean break drops the committed
    // rows' text; and a link, whose address the layout adds and a row may
    // start inside.
    let look = look();
    let text = "Some words before the **bold phrase across** a break, then words and \
                [a link](https://e.example/a/long/path/to/somewhere/far) and words \
                after it, many more to fill rows, and more still to the end.\n";
    let expected = letters(&whole(text, 79, &look));
    // Every third split in the nightly run, every seventh on a pull request.
    let step = if cortex_testkit::cases::exhaustive() {
        3
    } else {
        7
    };
    for (first, second) in [(40, 100), (40, 20), (100, 30), (25, 60)] {
        for split in (0..=text.len()).step_by(step) {
            let mut stream = Markdown::answer();
            stream.push(&text[..split]);
            let mut lines = stream.commit(first, &look);
            let _ = stream.live(first, &look);
            // Live at the new width before more text arrives, as a frame
            // after a resize draws it: what is committed and what is live
            // show the text so far once.
            let live = stream.live(second, &look);
            let mut shown = lines.clone();
            shown.extend(live);
            assert_eq!(
                letters(&shown),
                letters(&whole(&text[..split], 79, &look)),
                "{first} to {second}, split at {split}, live"
            );
            stream.push(&text[split..]);
            let after = stream.commit(second, &look);
            stream.finish();
            let last = stream.commit(second, &look);
            assert!(
                problems(&after, second).is_empty() && problems(&last, second).is_empty(),
                "{first} to {second}, split at {split}"
            );
            lines.extend(after);
            lines.extend(last);
            // Nothing was held long enough to be forced: the marks are those
            // of the text laid out whole.
            assert_eq!(stream.forced(), 0, "{first} to {second}, split at {split}");
            assert_eq!(
                letters(&lines),
                expected,
                "{first} to {second}, split at {split}: {:?}",
                texts(&lines)
            );
        }
    }
}

#[test]
fn a_table_with_only_a_header_too_wide_for_the_room_shows_the_header() {
    let look = look();
    let lines = at_once(
        "| a rather long header | another long one |\n|---|---|\n",
        30,
        &look,
    );
    let shown = texts(&lines).join("\n");
    assert!(shown.contains("a rather long header"), "{shown}");
    assert!(shown.contains("another long one"), "{shown}");
}

#[test]
fn a_forced_row_freezes_the_marks_inside_a_construct_it_holds() {
    // An emphasis that takes in a backtick, whose closing backtick comes
    // rows later and would take in the emphasis's closing mark. When the
    // emphasis's rows are forced, the backtick inside it is frozen with
    // them: the rows already committed stay right, and no word is lost or
    // shown twice.
    let look = look();
    let text = "*one two three four five six seven `eight nine* ten eleven twelve` \
                thirteen fourteen fifteen sixteen.\n";
    let strip = |lines: &[Line]| -> String {
        letters(lines)
            .chars()
            .filter(|c| !matches!(c, '*' | '`'))
            .collect()
    };
    let expected = strip(&whole(text, 20, &look));
    for split in (0..=text.len()).filter(|&at| text.is_char_boundary(at)) {
        let (lines, stream) =
            streamed(Flavor::Answer, &[&text[..split], &text[split..]], 20, &look);
        assert!(
            stream.forced() > 0,
            "split at {split}: the emphasis is forced"
        );
        assert_eq!(
            strip(&lines),
            expected,
            "split at {split}: {:?}",
            texts(&lines)
        );
        assert!(problems(&lines, 20).is_empty());
    }
}

#[test]
fn rows_that_all_start_inside_a_code_span_cost_no_more_than_their_length() {
    // Text whose rows all start inside a code span: no break between them is
    // ever clean, so past `LIMIT` bytes kept one is forced. The work is the
    // text's length times a bound: twice the text, about twice the work.
    let look = look();
    // Past `LIMIT` the cuts are forced where a code span goes on, and the
    // text after each is parsed alone: every word is still shown once.
    let strip = |lines: &[Line]| -> String { letters(lines).replace('`', "") };
    let work = |count: usize| {
        let text = "`a ``b ".repeat(count);
        let mut stream = Markdown::answer();
        let mut lines = Vec::new();
        for piece in text.as_bytes().chunks(13) {
            stream.push(std::str::from_utf8(piece).unwrap());
            lines.extend(stream.commit(79, &look));
        }
        stream.finish();
        lines.extend(stream.commit(79, &look));
        assert!(stream.forced() > 0, "a cut was forced");
        assert_eq!(strip(&lines), strip(&whole(&text, 79, &look)));
        stream.parsed_bytes()
    };
    // Both past `LIMIT`, where unbounded kept text would cost four times as
    // much for twice the text; bounded, the first 4 KB cost the same in
    // both, and twice the text costs under three times as much.
    let (once, twice) = (work(700), work(1_400));
    assert!(
        twice * 10 < once * 30,
        "{once} bytes parsed for 4.9 KB, {twice} for 9.8 KB"
    );
}

#[test]
fn a_window_resized_again_and_again_keeps_every_row_committed_at_each_width() {
    // The window dragged: three widths one after another, with text arriving
    // between them. Each resize keeps the rows committed at the widths
    // before it, those of the resize before included.
    let look = look();
    let text = "Some words before the **bold phrase across** a break, then words and \
                [a link](https://e.example/a/long/path/to/somewhere/far) and words \
                after it, many more to fill rows, and more still, and on to the end \
                of a paragraph long enough for three widths.\n";
    // And text whose rows all start inside a code span: no break between
    // them is clean, so the rows of the first resize are still kept at the
    // second.
    let ticks = "`a ``b ".repeat(120);
    // A sample of the splits on a pull request; every eleventh in the
    // nightly run.
    let (steps, tick_steps) = if cortex_testkit::cases::exhaustive() {
        (11, 97)
    } else {
        (53, 301)
    };
    for (text, step) in [(text, steps), (ticks.as_str(), tick_steps)] {
        resized_three_times(text, step, &look);
    }
}

/// `text` streamed in three parts at three widths, split at every `step`th
/// place: every word once. (The marks are left out: a mark held open across
/// a resize may be frozen, and shown.)
fn resized_three_times(text: &str, step: usize, look: &Look) {
    let words = |lines: &[Line]| letters(lines).replace(['*', '`'], "");
    let expected = words(&whole(text, 79, look));
    for widths in [[60, 30, 90], [30, 70, 40], [80, 50, 25]] {
        for first in (0..text.len()).step_by(step) {
            let second = first + (text.len() - first) / 2;
            let mut stream = Markdown::answer();
            let mut lines = Vec::new();
            for (piece, width) in [&text[..first], &text[first..second], &text[second..]]
                .into_iter()
                .zip(widths)
            {
                stream.push(piece);
                let committed = stream.commit(width, look);
                assert!(problems(&committed, width).is_empty());
                lines.extend(committed);
            }
            stream.finish();
            lines.extend(stream.commit(widths[2], look));
            assert_eq!(
                words(&lines),
                expected,
                "{widths:?} at {first} and {second}: {:?}",
                texts(&lines)
            );
        }
    }
}

#[test]
fn a_run_made_only_of_marks_keeps_little_text_and_costs_its_length() {
    // A run of one mark with no white space in it: decided inside where the
    // byte after the cut cannot change what ends there, so the text kept,
    // which a frame's live rows parse, stays bounded, and so does the work.
    let look = look();
    // A sample of the marks on a pull request, one of each kind of rule;
    // every one in the nightly run.
    let marks: &[&str] = if cortex_testkit::cases::exhaustive() {
        &[
            "!", "_", "*", "`", "\\", "&", "<", "]", "[", ">", "](", "*_",
        ]
    } else {
        &["!", "*_", "&", "\\", "]("]
    };
    for &mark in marks {
        let text = format!("x {} y\n", mark.repeat(6_000 / mark.len()));
        let mut stream = Markdown::answer();
        let mut lines = Vec::new();
        let mut most = 0;
        for piece in text.as_bytes().chunks(97) {
            stream.push(std::str::from_utf8(piece).unwrap());
            lines.extend(stream.commit(79, &look));
            most = most.max(stream.kept_bytes());
        }
        stream.finish();
        lines.extend(stream.commit(79, &look));
        assert!(most < super::LIMIT + 1_024, "{mark:?}: {most} bytes kept");
        let ratio = stream.parsed_bytes() / text.len();
        assert!(
            ratio < 2 * super::LIMIT / super::LONG_RUN,
            "{mark:?}: each byte parsed {ratio} times"
        );
        assert_eq!(
            letters(&lines),
            letters(&whole(&text, 79, &look)),
            "{mark:?}"
        );
        assert!(problems(&lines, 79).is_empty());
    }
}

#[test]
fn an_entity_or_an_autolink_cut_by_white_space_holds_nothing() {
    // At a word's end, white space follows: `<…` and `&…` there can never
    // become an autolink or an entity, and hold no row.
    let look = look();
    let text = "a <abcdefghijklmnopqrstuvwxyz0123456789 and &abcdefghijklmnopqrstuvwxyz \
                and more words after them\n";
    let (lines, stream) = streamed(Flavor::Answer, &[text], 12, &look);
    assert_eq!(stream.forced(), 0);
    assert_eq!(lines, whole(text, 12, &look));
}
