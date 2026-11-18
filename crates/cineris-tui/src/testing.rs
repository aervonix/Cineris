//! What the crate's unit tests share: text that is hard to lay out, the
//! snapshot format (text with inline style markers), and checks every view
//! must pass.

#![allow(
    clippy::disallowed_methods,
    reason = "tests read and write their snapshot files"
)]

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use cineris_base::{Rng, SeededRng};
use cineris_term::{Color, ColorDepth, Line, Style};

use crate::app::App;
use crate::glyphs::Glyphs;
use crate::theme::{Rgb, Theme};

/// Windows Terminal's Campbell background: the dark one of the reference
/// screens.
pub(crate) const DARK: Rgb = (0x0c, 0x0c, 0x0c);
/// One Half Light's background: the light one of the reference screens.
pub(crate) const LIGHT: Rgb = (0xfa, 0xfa, 0xfa);

/// Pieces of text that are hard to lay out: wide characters, combining
/// marks, emoji sequences, flags, control characters, tabs and spaces.
pub(crate) const TRICKY: [&str; 18] = [
    "word",
    "a",
    " ",
    "  ",
    "漢字",
    "e\u{301}",
    "👨\u{200d}👩\u{200d}👧",
    "🇫🇷",
    "\u{1b}[31m",
    "\t",
    "\u{9b}2J",
    "long-unbroken-identifier_with_no_spaces",
    "ok,",
    "x\u{0308}\u{0323}",
    "😀",
    "\u{a0}",
    "→",
    "\r\n",
];

/// Text made of up to `pieces` random pieces of [`TRICKY`].
pub(crate) fn tricky_text(rng: &mut SeededRng, pieces: u64) -> String {
    let count = rng.below(pieces + 1);
    let mut text = String::new();
    for _ in 0..count {
        text.push_str(TRICKY[usize::try_from(rng.below(TRICKY.len() as u64)).unwrap_or(0)]);
    }
    text
}

/// The variants a view is snapshotted in: every width in both glyph sets in
/// true colour on a dark background, then at 80 columns in Unicode each
/// other depth and background. Width and glyph set change the layout and do
/// not touch styles; depth and background change styles and do not touch
/// the layout, so this covers each against the other without the whole
/// product.
pub(crate) fn variants() -> Vec<Variant> {
    let mut all = Vec::new();
    for width in [60, 80, 120] {
        for glyphs in [&Glyphs::UNICODE, &Glyphs::ASCII] {
            all.push(Variant::new(width, glyphs, ColorDepth::TrueColor, DARK));
        }
    }
    for (depth, background) in [
        (ColorDepth::TrueColor, LIGHT),
        (ColorDepth::Ansi16, DARK),
        (ColorDepth::Ansi16, LIGHT),
        (ColorDepth::Indexed256, DARK),
        (ColorDepth::Mono, DARK),
    ] {
        all.push(Variant::new(80, &Glyphs::UNICODE, depth, background));
    }
    all
}

/// One way of drawing a view for a snapshot.
#[derive(Debug, Clone)]
pub(crate) struct Variant {
    /// The window's width; views get one less ([`Variant::width`]).
    pub(crate) columns: usize,
    pub(crate) glyphs: &'static Glyphs,
    pub(crate) theme: Theme,
}

impl Variant {
    pub(crate) fn new(
        columns: usize,
        glyphs: &'static Glyphs,
        depth: ColorDepth,
        background: Rgb,
    ) -> Self {
        Self {
            columns,
            glyphs,
            theme: Theme::new(depth, Some(background)),
        }
    }

    /// The columns a line may use: all but the last.
    pub(crate) fn width(&self) -> usize {
        self.columns - 1
    }

    /// The heading of this variant's section of a snapshot.
    pub(crate) fn title(&self) -> String {
        let glyphs = if std::ptr::eq(self.glyphs, &Glyphs::ASCII) {
            "ascii"
        } else {
            "unicode"
        };
        let background = match self.theme.background() {
            crate::theme::Background::Dark => "dark",
            crate::theme::Background::Light => "light",
        };
        let depth = match self.theme.depth() {
            ColorDepth::TrueColor => "true colour",
            ColorDepth::Indexed256 => "256 colours",
            ColorDepth::Ansi16 => "16 colours",
            ColorDepth::Mono => "no colour",
        };
        format!("{} columns, {glyphs}, {depth}, {background}", self.columns)
    }
}

/// Lines as text with inline style markers: the snapshot format.
///
/// A style is written where it changes, as `{…}` with its parts: `fg=` and
/// `bg=` (`a13` a palette colour, `i99` an indexed one, `#7A3CFB` an exact
/// one), then `b` bold, `d` dim, `i` italic, `u` underline, `r` reverse, and
/// `link=…`. `{}` is the plain style. Each line starts plain, and a `{` in
/// the text is written `{{`. A line ends in `|`, so that spaces at its end
/// are seen.
pub(crate) fn markup(lines: &[Line]) -> String {
    let mut out = String::new();
    for line in lines {
        let mut current = (Style::PLAIN, None::<&str>);
        for span in line.spans() {
            let next = (span.style(), span.url());
            if next != current {
                out.push('{');
                out.push_str(&describe(next.0, next.1));
                out.push('}');
                current = next;
            }
            out.push_str(&span.text().replace('{', "{{"));
        }
        out.push_str("|\n");
    }
    out
}

fn describe(style: Style, link: Option<&str>) -> String {
    let mut parts = Vec::new();
    let color = |color: Color| match color {
        Color::Default => None,
        Color::Ansi(index) => Some(format!("a{index}")),
        Color::Indexed(index) => Some(format!("i{index}")),
        Color::Rgb(r, g, b) => Some(format!("#{r:02X}{g:02X}{b:02X}")),
    };
    if let Some(fg) = color(style.fg) {
        parts.push(format!("fg={fg}"));
    }
    if let Some(bg) = color(style.bg) {
        parts.push(format!("bg={bg}"));
    }
    for (on, name) in [
        (style.bold, "b"),
        (style.dim, "d"),
        (style.italic, "i"),
        (style.underline, "u"),
        (style.reverse, "r"),
    ] {
        if on {
            parts.push(name.to_owned());
        }
    }
    if let Some(link) = link {
        parts.push(format!("link={link}"));
    }
    parts.join(" ")
}

/// A snapshot made of sections, one for each variant: the heading, then the
/// lines.
pub(crate) fn sections(parts: &[(String, Vec<Line>)]) -> String {
    let mut out = String::new();
    for (title, lines) in parts {
        let _ = writeln!(out, "== {title}");
        out.push_str(&markup(lines));
        out.push('\n');
    }
    out
}

/// Compares `actual` with the snapshot `name` in the crate's `snapshots`
/// folder. With `CINERIS_UPDATE_SNAPSHOTS` set, the snapshot is written
/// instead, to be reviewed like code: a change in a view's snapshot is a
/// change in what users see.
pub(crate) fn assert_snapshot(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("snapshots")
        .join(format!("{name}.txt"));
    if std::env::var_os("CINERIS_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("no snapshot {}: {error}", path.display()))
        // A checkout on Windows may have turned the line endings.
        .replace("\r\n", "\n");
    if expected != actual {
        let first = expected
            .lines()
            .zip(actual.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
        panic!(
            "{name} differs from its snapshot, first at line {}.\n--- snapshot\n{}\n--- now\n{}\n\
             If the change is intended, run the test with CINERIS_UPDATE_SNAPSHOTS=1 and review \
             the difference.",
            first + 1,
            expected
                .lines()
                .skip(first)
                .take(6)
                .collect::<Vec<_>>()
                .join("\n"),
            actual
                .lines()
                .skip(first)
                .take(6)
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
}

/// The frame `app` draws at `now` in a window of `columns` by `rows`, as
/// snapshot text: the lines the frame commits to the scrollback, the live
/// region, and where the cursor goes, each line with its styles
/// ([`markup`]). The application is resized first, as the loop resizes it
/// for a window of that size; the frame is the application's real one
/// (`App::frame`), so what it consumes (the lines committed) is consumed.
pub(crate) fn app_frame(app: &mut App, now: Instant, columns: usize, rows: usize) -> String {
    // Views get one column less than the window.
    app.resize(columns.saturating_sub(1), rows);
    let (commit, live, cursor) = app.frame(now);
    let mut out = format!("== {columns} columns, {rows} rows\n");
    let _ = writeln!(out, "-- committed: {} lines", commit.len());
    out.push_str(&markup(&commit));
    let _ = writeln!(out, "-- live region: {} lines", live.len());
    out.push_str(&markup(&live));
    let _ = writeln!(out, "cursor at row {}, column {}|", cursor.row, cursor.col);
    out
}

/// Draws `app`'s frame at `now` in a window of `columns` by `rows`
/// ([`app_frame`]) and compares it with the snapshot `name` (a file
/// `snapshots/<name>.txt`, named `app-…` by convention). With
/// `CINERIS_UPDATE_SNAPSHOTS` set it writes the snapshot instead, as every
/// other snapshot of the crate is, to be reviewed like code. The one call a
/// test of the application makes to say "this is what it shows".
pub(crate) fn assert_app_frame(
    name: &str,
    app: &mut App,
    now: Instant,
    columns: usize,
    rows: usize,
) {
    assert_snapshot(name, &app_frame(app, now, columns, rows));
}

/// What is wrong with lines drawn for `width` columns: a line wider than
/// that, or a control character that reached a line (which the renderer
/// would show as a symbol, but which means the content skipped `sanitize`).
pub(crate) fn problems(lines: &[Line], width: usize) -> Vec<String> {
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if line.width() > width {
            found.push(format!(
                "line {index} is {} columns, over {width}: {:?}",
                line.width(),
                line.text()
            ));
        }
        if line
            .text()
            .chars()
            .any(|c| cineris_term::sanitize::visible(c).is_some())
        {
            found.push(format!(
                "line {index} holds a control character: {:?}",
                line.text()
            ));
        }
        let unseen = unseen(&line.text());
        if !unseen.is_empty() {
            found.push(format!(
                "line {index} holds what acts unseen, {unseen:?}: {:?}",
                line.text()
            ));
        }
    }
    found
}

/// The characters of `text` that act unseen and that `cineris_term::sanitize`
/// writes as escapes (a bidirectional control, a character of no width, a
/// tag character, a selector that follows nothing it picks): one in drawn
/// text skipped `sanitize`. The two joiners, which it keeps, are not.
pub(crate) fn unseen(text: &str) -> Vec<char> {
    use cineris_base::visible::is_selector;
    let mut found = Vec::new();
    // Whether the character before is one a selector may follow.
    let mut base = false;
    for c in text.chars() {
        let joiner = matches!(c, '\u{200c}' | '\u{200d}');
        let unseen = if is_selector(c) {
            !base
        } else {
            cineris_base::acts_unseen(c) && !c.is_control() && !joiner
        };
        if unseen {
            found.push(c);
        }
        base = !is_selector(c) && !cineris_base::acts_unseen(c);
    }
    found
}
