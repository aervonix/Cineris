//! The full-screen presenter against the virtual terminal (since step 25):
//! for every frame, one write, inside a synchronized update, nothing
//! cleared and then drawn again, no screen cleared, and the screen and the
//! cursor as the frame says; and the same after every resize, from 20 to
//! 160 columns and back.

use std::io;

use cortex_base::{Rng, SeededRng};
use cortex_term::{
    Capabilities, Cursor, Drawn, Full, Line, Overrides, Platform, Screen, Sink, Span, Style,
};
use cortex_testkit::VTerm;
use cortex_testkit::cases::{self, Outcome};

/// Records each call the presenter makes.
#[derive(Debug, Default)]
struct Writes(Vec<Vec<u8>>);

impl Sink for Writes {
    fn write_frame(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.0.push(bytes.to_vec());
        Ok(())
    }
}

fn caps(sync: bool) -> Capabilities {
    let mut caps = Capabilities::detect(
        |name| (name == "TERM").then(|| "xterm-256color".to_owned()),
        Platform::Unix,
        Overrides::default(),
    );
    caps.sync = sync;
    caps
}

/// A virtual terminal on its alternate screen, with the presenter on it.
struct Rig {
    term: VTerm,
    full: Full,
    caps: Capabilities,
    frames: usize,
}

impl Rig {
    fn new(cols: u16, rows: u16, sync: bool) -> Self {
        let mut term = VTerm::new(cols, rows);
        term.feed(b"$ earlier output\r\n$ cortex --fullscreen\r\n\x1b[?1049h");
        Self {
            term,
            full: Full::new(cols, rows),
            caps: caps(sync),
            frames: 0,
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(cols, rows);
        self.full.resize(cols, rows);
    }

    /// Draws one frame and checks every property of it.
    fn frame(&mut self, lines: &[Line], cursor: Option<Cursor>) -> Result<(), String> {
        let mut writes = Writes::default();
        let drawn = self
            .full
            .draw(&Screen { lines, cursor }, &self.caps, &mut writes)
            .map_err(|error| error.to_string())?;
        self.frames += 1;
        let frame = self.frames;
        match (drawn, writes.0.len()) {
            (Drawn::Skipped, 0) => return self.check(lines, cursor),
            (Drawn::Written(length), 1) if length == writes.0[0].len() => {}
            (drawn, count) => {
                return Err(format!(
                    "frame {frame}: {drawn:?} in {count} write(s), not one"
                ));
            }
        }
        let bytes = writes.0.remove(0);
        self.term.watch();
        self.term.feed(&bytes);
        let report = self.term.report();
        // Every row is the presenter's: none is above the region.
        let problems = report.problems(0, self.caps.sync);
        if !problems.is_empty() {
            return Err(format!(
                "frame {frame}: {}\n  bytes: {:?}\n{}",
                problems.join("; "),
                String::from_utf8_lossy(&bytes),
                self.term.dump()
            ));
        }
        self.check(lines, cursor)
    }

    /// The screen holds the frame's rows, each cut to the width a row may
    /// use, and the cursor is where it said.
    fn check(&self, lines: &[Line], cursor: Option<Cursor>) -> Result<(), String> {
        let (cols, rows) = self.term.size();
        let width = usize::from(cols).saturating_sub(1).max(1);
        let want: Vec<String> = (0..usize::from(rows))
            .map(|row| {
                lines
                    .get(row)
                    .map(|line| clip(&line.text(), width))
                    .unwrap_or_default()
            })
            .collect();
        let have: Vec<String> = self
            .term
            .screen()
            .iter()
            .map(|row| row.trim_end().to_owned())
            .collect();
        if have != want {
            return Err(format!(
                "frame {}: the screen is wrong\n  want: {want:?}\n  have: {have:?}",
                self.frames
            ));
        }
        match cursor.filter(|cursor| cursor.row < usize::from(rows)) {
            Some(cursor) => {
                let at = self.term.cursor();
                let want = (cursor.row, cursor.col.min(width));
                if at != want || !self.term.modes().cursor_visible {
                    return Err(format!(
                        "frame {}: the cursor is at {at:?}, not {want:?}",
                        self.frames
                    ));
                }
            }
            None if self.term.modes().cursor_visible => {
                return Err(format!("frame {}: the cursor is shown", self.frames));
            }
            None => {}
        }
        Ok(())
    }
}

/// A line's text as the screen shows it, cut to `cols`.
fn clip(text: &str, cols: usize) -> String {
    let mut out = String::new();
    let mut width = 0;
    for (_, grapheme, columns) in cortex_base::text::graphemes(text) {
        if width + columns > cols {
            break;
        }
        out.push_str(grapheme);
        width += columns;
    }
    out.trim_end().to_owned()
}

/// A screen's worth of rows, some long, some wide, some styled.
fn random_rows(rng: &mut SeededRng, count: usize) -> Vec<Line> {
    const WORDS: &[&str] = &["alpha", "β", "漢字", "😀", "box", "│ edge │", "", "x"];
    (0..count)
        .map(|_| {
            let mut line = Line::new();
            for _ in 0..rng.below(12) {
                let word = WORDS[rng.below(WORDS.len() as u64) as usize];
                let style = if rng.below(3) == 0 {
                    Style::PLAIN.bold()
                } else {
                    Style::PLAIN
                };
                line.push(Span::new(format!("{word} "), style));
            }
            line
        })
        .collect()
}

/// A script of frames: the rows and the cursor of each.
#[derive(Debug)]
struct Script(Vec<(Vec<Line>, Option<Cursor>)>);

fn script(rng: &mut SeededRng) -> Script {
    let mut rows = random_rows(rng, 12);
    let mut frames = Vec::new();
    for _ in 0..20 {
        // A few rows change, as a frame that streams or scrolls does.
        for _ in 0..rng.below(4) {
            let row = rng.below(12) as usize;
            rows[row] = random_rows(rng, 1).remove(0);
        }
        if rng.below(5) == 0 {
            // The whole screen moves up a row: following the end.
            rows.remove(0);
            rows.push(random_rows(rng, 1).remove(0));
        }
        let cursor = (rng.below(2) == 0).then(|| Cursor {
            row: rng.below(14) as usize,
            col: rng.below(70) as usize,
        });
        frames.push((rows.clone(), cursor));
    }
    Script(frames)
}

#[test]
fn every_frame_is_one_write_with_nothing_cleared_and_the_right_screen() {
    for sync in [true, false] {
        cases::check(
            "full-screen frames",
            30,
            script,
            |script: &Script| -> Outcome {
                let mut rig = Rig::new(60, 12, sync);
                for (rows, cursor) in &script.0 {
                    rig.frame(rows, *cursor)?;
                }
                Ok(())
            },
        );
    }
}

/// Every width from 20 to 160 and back, and some heights: the frame after
/// each resize is drawn whole, and the screen is right.
#[test]
fn a_resize_to_any_width_draws_the_screen_whole_and_right() {
    let mut rng = SeededRng::new(25);
    let mut rig = Rig::new(80, 20, true);
    let mut rows = random_rows(&mut rng, 20);
    rig.frame(&rows, None).unwrap();
    for cols in (20..=160).chain((20..160).rev()) {
        let height = 10 + (cols % 15);
        rig.resize(cols, height);
        rows = random_rows(&mut rng, usize::from(height));
        rig.frame(&rows, Some(Cursor { row: 1, col: 3 })).unwrap();
    }
}
