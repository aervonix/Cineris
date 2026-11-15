//! The golden screens: the reference screens of the chosen look
//! (`docs/architecture/tui-look/`), drawn again by the real views through
//! the inline presenter into the test kit's virtual terminal, and compared
//! cell by cell with the reference files fed into another.
//!
//! - **Every row this step's views draw is compared,** text, style and wide
//!   cells alike, as they are seen: a space is compared by its background
//!   and by whether it is reversed or underlined, which is all of it that
//!   shows. A row another step draws (the input box, menus, dialogs,
//!   panels, the viewer's frame) is counted as not built, by screen, and is
//!   never counted as compared.
//! - **The counts are held:** a screen's compared rows must be the number
//!   [`COMPARED`] gives, so a row skipped by mistake fails the test.
//! - `cargo xtask tui-screens` prints the report. With
//!   `CORTEX_UPDATE_SCREENS=1` it rewrites the rows of the reference files
//!   that differ, from the real views, and prints every changed cell:
//!   a reference screen changes only that way, for the owner to see.

mod scenario;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use cortex_term::{
    Capabilities, Color, ColorDepth, Frame, Inline, Line, Overrides, Platform, Style,
};
use cortex_testkit::vterm::{Cell, VTerm};

use self::scenario::{BACKGROUNDS, Row, SPECS, Spec, compose};

/// The rows each screen compares, by screen: what the views built so far
/// draw (step 22's, and step 21's input box and menus).
/// When a later step builds the rows it owns, its counts go up here.
const COMPARED: [(&str, usize); 17] = [
    ("0", 11),
    ("1", 40),
    ("1b", 10),
    ("1c", 28),
    ("1d", 20),
    ("2a", 7),
    ("2b", 9),
    ("3a", 11),
    ("3b", 11),
    ("4", 35),
    ("5", 17),
    ("6", 42),
    ("7a", 40),
    ("7b", 40),
    ("7c", 40),
    ("8a", 35),
    ("8b", 47),
];

fn reference_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/architecture/tui-look")
}

/// What one screen on one background came to.
struct Outcome {
    rows: usize,
    compared: usize,
    not_built: Vec<(&'static str, usize)>,
    /// Each cell that differs: row, column, the reference's and ours.
    differences: Vec<(usize, usize, String, String)>,
    /// The rows that differ, drawn from our cells, for rewriting.
    rewritten: Vec<(usize, String)>,
    /// How many of the differences are of style alone: the same text in
    /// another colour, weight or slant.
    style_only: usize,
}

fn capabilities(spec: Spec) -> Capabilities {
    Capabilities::detect(
        |_| None,
        Platform::Unix,
        Overrides {
            sync: Some(true),
            keyboard: Some(false),
            color: Some(spec.depth),
            hyperlinks: Some(false),
            unicode: Some(!spec.ascii),
        },
    )
}

/// Draws a screen through the inline presenter into a virtual terminal.
fn draw(spec: Spec, rows: &[Row], live_from: usize) -> VTerm {
    let height = u16::try_from(rows.len()).unwrap();
    let columns = u16::try_from(spec.columns).unwrap();
    let lines: Vec<Line> = rows
        .iter()
        .map(|row| match row {
            Row::Built(line) => line.clone(),
            Row::NotBuilt(_) => Line::new(),
        })
        .collect();
    let mut presenter = Inline::new(columns, height);
    let mut bytes = Vec::new();
    presenter
        .draw(
            &Frame {
                commit: &lines[..live_from],
                live: &lines[live_from..],
                cursor: None,
            },
            &capabilities(spec),
            &mut bytes,
        )
        .unwrap();
    let mut terminal = VTerm::new(columns, height);
    terminal.feed(&bytes);
    terminal
}

/// A reference file, fed into a virtual terminal one row taller than the
/// screen, so that its last line break scrolls nothing away.
fn reference(spec: Spec, background: &str, rows: usize) -> (VTerm, Vec<String>) {
    let path = reference_dir().join(format!("{}-{background}.ans", spec.id));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let text = text.replace("\r\n", "\n");
    let lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mut terminal = VTerm::new(
        u16::try_from(spec.columns).unwrap(),
        u16::try_from(rows + 1).unwrap(),
    );
    terminal.feed(text.replace('\n', "\r\n").as_bytes());
    (terminal, lines)
}

fn compare(spec: Spec, background: (&str, (u8, u8, u8))) -> Outcome {
    compare_screen(spec, background.0, &compose(spec, background.1))
}

/// Compares a composed screen with the reference file of `background`.
fn compare_screen(spec: Spec, background: &str, screen: &scenario::Screen) -> Outcome {
    let ours = draw(spec, &screen.rows, screen.live_from);
    let (theirs, _) = reference(spec, background, screen.rows.len());
    let mut outcome = Outcome {
        rows: screen.rows.len(),
        compared: 0,
        not_built: Vec::new(),
        differences: Vec::new(),
        rewritten: Vec::new(),
        style_only: 0,
    };
    for (index, row) in screen.rows.iter().enumerate() {
        if let Row::NotBuilt(what) = row {
            match outcome.not_built.iter_mut().find(|(name, _)| name == what) {
                Some((_, count)) => *count += 1,
                None => outcome.not_built.push((what, 1)),
            }
            continue;
        }
        outcome.compared += 1;
        let mine = ours.row(index).cells();
        let reference = theirs.row(index).cells();
        let mut differs = false;
        for (column, (a, b)) in reference.iter().zip(mine).enumerate() {
            if !same(a, b) {
                differs = true;
                let text = |cell: &Cell| {
                    if cell.text.is_empty() {
                        " ".to_owned()
                    } else {
                        cell.text.clone()
                    }
                };
                if text(a) == text(b) && a.tail == b.tail {
                    outcome.style_only += 1;
                }
                outcome
                    .differences
                    .push((index, column, describe(a), describe(b)));
            }
        }
        if differs {
            outcome.rewritten.push((index, ansi(mine)));
        }
    }
    // A reference with more rows than the screen we draw differs in every
    // row it has beyond ours.
    if theirs.row(screen.rows.len()).text() != "" {
        outcome.differences.push((
            screen.rows.len(),
            0,
            "more rows".to_owned(),
            "none".to_owned(),
        ));
    }
    outcome
}

/// Whether two cells look the same. A space shows only its background, and
/// whether it is reversed or underlined: its foreground colour, boldness,
/// dimness and slant cannot be seen, so they are not compared. A blank cell
/// is a space.
fn same(a: &Cell, b: &Cell) -> bool {
    let space = |cell: &Cell| (cell.text.is_empty() || cell.text == " ") && !cell.tail;
    if space(a) && space(b) {
        let seen = |style: Style| (style.bg, style.reverse, style.underline);
        return seen(a.style) == seen(b.style);
    }
    a == b
}

fn describe(cell: &Cell) -> String {
    let text = if cell.text.is_empty() {
        " "
    } else {
        &cell.text
    };
    format!("{text:?} {}", style_name(cell.style))
}

fn style_name(style: Style) -> String {
    let colour = |color: Color| match color {
        Color::Default => None,
        Color::Ansi(n) => Some(format!("a{n}")),
        Color::Indexed(n) => Some(format!("i{n}")),
        Color::Rgb(r, g, b) => Some(format!("#{r:02X}{g:02X}{b:02X}")),
    };
    let mut parts = Vec::new();
    if let Some(fg) = colour(style.fg) {
        parts.push(format!("fg={fg}"));
    }
    if let Some(bg) = colour(style.bg) {
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
    format!("{{{}}}", parts.join(" "))
}

/// A row of cells as the reference files write a row: each run of one
/// style after its own full SGR sequence, and a reset at the end.
fn ansi(cells: &[Cell]) -> String {
    let last = cells
        .iter()
        .rposition(|cell| !cell.text.is_empty() || cell.tail)
        .map_or(0, |at| at + 1);
    let mut out = String::new();
    let mut current: Option<Style> = None;
    for cell in &cells[..last] {
        if cell.tail {
            continue;
        }
        if current != Some(cell.style) {
            out.push_str(&sgr(cell.style));
            current = Some(cell.style);
        }
        out.push_str(if cell.text.is_empty() {
            " "
        } else {
            &cell.text
        });
    }
    out.push_str("\x1b[0m");
    out
}

fn sgr(style: Style) -> String {
    let mut parts = vec!["0".to_owned()];
    for (on, code) in [
        (style.bold, "1"),
        (style.dim, "2"),
        (style.italic, "3"),
        (style.underline, "4"),
        (style.reverse, "7"),
    ] {
        if on {
            parts.push(code.to_owned());
        }
    }
    for (color, base) in [(style.fg, 30u16), (style.bg, 40)] {
        match color {
            Color::Default => {}
            Color::Ansi(n) if n < 8 => parts.push((base + u16::from(n)).to_string()),
            Color::Ansi(n) => parts.push((base + 60 + u16::from(n % 16 - 8)).to_string()),
            Color::Indexed(n) => parts.push(format!("{};5;{n}", base + 8)),
            Color::Rgb(r, g, b) => parts.push(format!("{};2;{r};{g};{b}", base + 8)),
        }
    }
    format!("\x1b[{}m", parts.join(";"))
}

/// Compares every screen, and rewrites the references when asked.
fn run() -> (String, usize) {
    let update = std::env::var_os("CORTEX_UPDATE_SCREENS").is_some();
    let mut report = String::new();
    let mut differences = 0;
    let _ = writeln!(report, "screen  background  rows  compared  not built");
    for spec in SPECS {
        let expected = COMPARED
            .iter()
            .find(|(id, _)| *id == spec.id)
            .map(|(_, count)| *count);
        for background in BACKGROUNDS {
            let outcome = compare(spec, background);
            let not_built: Vec<String> = outcome
                .not_built
                .iter()
                .map(|(what, count)| format!("{count} {what}"))
                .collect();
            let _ = writeln!(
                report,
                "{:<7} {:<11} {:>4}  {:>8}  {}",
                spec.id,
                background.0,
                outcome.rows,
                outcome.compared,
                if not_built.is_empty() {
                    "none".to_owned()
                } else {
                    not_built.join(", ")
                }
            );
            if Some(outcome.compared) != expected {
                let _ = writeln!(
                    report,
                    "    {}-{} compared {} rows where {expected:?} are expected",
                    spec.id, background.0, outcome.compared
                );
                differences += 1;
            }
            assert_eq!(
                outcome.compared
                    + outcome
                        .not_built
                        .iter()
                        .map(|(_, count)| count)
                        .sum::<usize>(),
                outcome.rows,
                "every row of screen {} is compared or counted as not built",
                spec.id
            );
            if !outcome.differences.is_empty() {
                let _ = writeln!(
                    report,
                    "    {}-{}: {} cell(s) differ, {} of them in style alone",
                    spec.id,
                    background.0,
                    outcome.differences.len(),
                    outcome.style_only
                );
            }
            for (row, column, theirs, ours) in &outcome.differences {
                let _ = writeln!(
                    report,
                    "    {}-{} row {row} column {column}: {theirs} -> {ours}",
                    spec.id, background.0
                );
            }
            differences += outcome.differences.len();
            if update && !outcome.rewritten.is_empty() {
                rewrite(spec, background.0, &outcome);
            }
        }
    }
    (report, differences)
}

/// Rewrites the rows that differ in one reference file, and in its text
/// copy (from the dark one).
fn rewrite(spec: Spec, background: &str, outcome: &Outcome) {
    let (terminal, mut lines) = reference(spec, background, outcome.rows);
    lines.resize(outcome.rows, String::new());
    for (row, text) in &outcome.rewritten {
        lines[*row] = text.clone();
    }
    let path = reference_dir().join(format!("{}-{background}.ans", spec.id));
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    if background == "dark" {
        let screen = compose(spec, BACKGROUNDS[0].1);
        let ours = draw(spec, &screen.rows, screen.live_from);
        let text: Vec<String> = (0..outcome.rows)
            .map(|row| {
                if matches!(screen.rows[row], Row::NotBuilt(_)) {
                    terminal.row(row).text()
                } else {
                    ours.row(row).text()
                }
            })
            .collect();
        let path = reference_dir().join(format!("{}.txt", spec.id));
        std::fs::write(&path, format!("{}\n", text.join("\n"))).unwrap();
    }
}

#[test]
#[cfg_attr(
    not(feature = "highlight"),
    ignore = "the reference screens' code is highlighted"
)]
fn the_views_draw_the_reference_screens() {
    let (report, differences) = run();
    println!("{report}");
    assert_eq!(
        differences, 0,
        "the views and the reference screens differ:\n{report}"
    );
}

#[test]
#[cfg_attr(
    not(feature = "highlight"),
    ignore = "the reference screens' code is highlighted"
)]
fn a_changed_cell_is_found_and_a_row_not_built_is_not_compared() {
    let spec = SPECS[1];
    let mut screen = compose(spec, BACKGROUNDS[0].1);
    assert!(compare_screen(spec, "dark", &screen).differences.is_empty());
    // One character of one row, changed: the comparison finds that cell.
    let at = 2;
    let Row::Built(line) = &screen.rows[at] else {
        panic!("row {at} is built");
    };
    let changed = line.text().replacen('1', "2", 1);
    screen.rows[at] = Row::Built(Line::plain(changed));
    let outcome = compare_screen(spec, "dark", &screen);
    assert!(
        outcome.differences.iter().any(|(row, ..)| *row == at),
        "the changed row was not found: {:?}",
        outcome.differences
    );
    // A row marked as not built is never compared, whatever it holds.
    let mut screen = compose(spec, BACKGROUNDS[0].1);
    screen.rows[at] = Row::NotBuilt("a test's row");
    let outcome = compare_screen(spec, "dark", &screen);
    assert!(outcome.differences.is_empty());
    // Screen 1 is built whole, but for the row marked here.
    assert_eq!(outcome.compared, screen.rows.len() - 1);
}

/// `line` with the character at `column` (counted in characters, each one
/// column wide) drawn in the style `change` makes of its own.
fn restyle(line: &Line, column: usize, change: impl Fn(Style) -> Style) -> Line {
    let mut out = Line::new();
    let mut at = 0;
    for span in line.spans() {
        for character in span.text().chars() {
            let style = if at == column {
                change(span.style())
            } else {
                span.style()
            };
            out.push(cortex_term::Span::new(character.to_string(), style));
            at += 1;
        }
    }
    out
}

#[test]
#[cfg_attr(
    not(feature = "highlight"),
    ignore = "the reference screens' code is highlighted"
)]
fn a_changed_colour_weight_or_background_is_found_and_an_unseen_one_is_not() {
    // The first built row of screen 1 with a letter and a space in plain
    // ASCII, so that a character is a column.
    let spec = SPECS[1];
    let screen = compose(spec, BACKGROUNDS[0].1);
    let (at, line) = screen
        .rows
        .iter()
        .enumerate()
        .find_map(|(index, row)| match row {
            Row::Built(line) if line.text().is_ascii() && line.text().trim().contains(' ') => {
                Some((index, line.clone()))
            }
            _ => None,
        })
        .expect("a built row of plain text");
    let text = line.text().into_owned();
    let letter = text.find(|c: char| c.is_ascii_alphanumeric()).unwrap();
    let space = letter + text[letter..].find(' ').unwrap();
    let differs_at = |changed: Line, column: usize| {
        let mut screen = compose(spec, BACKGROUNDS[0].1);
        screen.rows[at] = Row::Built(changed);
        let outcome = compare_screen(spec, "dark", &screen);
        outcome
            .differences
            .iter()
            .any(|(row, at_column, ..)| *row == at && *at_column == column)
    };
    let other = |color: Color| match color {
        Color::Rgb(255, 0, 255) => Color::Rgb(0, 255, 0),
        _ => Color::Rgb(255, 0, 255),
    };

    // The same text in another colour, or bold, or reversed: found.
    assert!(differs_at(
        restyle(&line, letter, |style| Style {
            fg: other(style.fg),
            ..style
        }),
        letter
    ));
    assert!(differs_at(
        restyle(&line, letter, |style| Style {
            bold: !style.bold,
            ..style
        }),
        letter
    ));
    assert!(differs_at(
        restyle(&line, space, |style| Style {
            bg: other(style.bg),
            ..style
        }),
        space
    ));
    // A space's foreground cannot be seen, and is not a difference.
    assert!(!differs_at(
        restyle(&line, space, |style| Style {
            fg: other(style.fg),
            ..style
        }),
        space
    ));
}

#[test]
fn a_skipped_row_is_never_counted_as_compared() {
    // A screen whose every row is built compares them all, and one with
    // rows of later steps counts each of those once.
    let whole = compare(SPECS[0], BACKGROUNDS[0]);
    assert!(whole.not_built.is_empty(), "screen 0 is built whole");
    assert_eq!(whole.compared, whole.rows);
    let spec = SPECS.iter().find(|spec| spec.id == "4").copied().unwrap();
    let outcome = compare(spec, BACKGROUNDS[0]);
    let not_built: usize = outcome.not_built.iter().map(|(_, count)| count).sum();
    assert_eq!(
        not_built, 8,
        "screen 4's panels, and its input box's placeholder"
    );
    assert_eq!(outcome.compared, outcome.rows - 8);
    // Every depth is drawn as the reference is.
    assert!(SPECS.iter().any(|spec| spec.depth == ColorDepth::Mono));
}

#[test]
fn the_text_copies_are_the_screens_text() {
    // Each `.txt` beside the screens is the text of its dark `.ans`, row for
    // row: `--write` keeps them so, and a copy edited by hand alone fails.
    for spec in SPECS {
        let path = reference_dir().join(format!("{}.txt", spec.id));
        let copy = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            .replace("\r\n", "\n");
        let copy: Vec<&str> = copy.lines().collect();
        let (_, screen) = reference(spec, "dark", copy.len());
        assert_eq!(copy.len(), screen.len(), "{}.txt has every row", spec.id);
        let (terminal, _) = reference(spec, "dark", screen.len());
        for (row, line) in copy.iter().enumerate() {
            assert_eq!(
                terminal.row(row).text().trim_end(),
                line.trim_end(),
                "{}.txt row {row}",
                spec.id
            );
        }
    }
}
