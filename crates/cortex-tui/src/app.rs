//! The interactive application: the transcript, the input and the dialogs
//! wired to a session through a [`Link`] (`docs/architecture/09-tui.md`;
//! `docs/steps/23/design.md` until it is folded into the step's record).
//!
//! ```text
//! terminal ─► Terminal::next ─► App::terminal ─► Input / Dialog ─► Op ─► Link ─► core
//! core ─► Link::next ─► App::drain ─► Mapper ─► Transcript ─► App::frame ─► Inline
//! ```
//!
//! **Threads.** [`run`] is the interface's loop, on the thread that entered
//! the terminal. It owns the terminal, the presenter, the scheduler and the
//! [`App`], and waits only in `Terminal::next`, which a [`Link`], the
//! highlighter and the history wake through the terminal's waker. It never
//! waits on the core, a disk or the network: a link queues operations and
//! takes what has arrived.
//!
//! **One wake of the loop:** every terminal event already read, each
//! through the input or the open dialog; a key's frame drawn at once; then
//! what arrived from the core, at most [`SLICE`] of it in batches of
//! [`BATCH`], looking for terminal input between batches; then a frame if
//! the scheduler says one is due (at most 60 a second for what streams).
//! Idle, nothing is scheduled and the thread sleeps until something
//! happens: no timer runs.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`link`] | The [`Link`] trait: operations out, events, requests and results in |
//! | `state` | [`App`]: the state, and what each event, key and result does to it |
//! | `items` | The core's items onto the transcript |
//! | `tools` | A tool call's line: verb, target, state, measure and detail |
//! | `dialog` | The dialogs in the input box's place, and their keys |
//! | `wording` | What an error says, and what to do next |

mod dialog;
mod items;
pub mod link;
mod state;
mod tools;
mod wording;

use std::io;
use std::time::{Duration, Instant};

use cortex_protocol::session::SessionSettings;
use cortex_protocol::{ErrorData, SessionId};
/// What the binary hands the terminal layer, through this crate: the
/// `[terminal]` overrides, the colour depth they name, the waker a link
/// calls, and what the terminal can do.
pub use cortex_term::{Capabilities, ColorDepth, Overrides, Waker};
use cortex_term::{Device, Event, Frame, Inline, Scheduler, Terminal, Wake};

pub use link::{Inbound, Link, Ticket};
pub use state::{App, Drained, Effect, SECOND_INTERRUPT};

use crate::glyphs::Glyphs;
use crate::highlight::Highlighter;
use crate::history::HistoryStore;
use crate::keys::KeyMap;
use crate::theme::{Background, Theme};
use crate::view::{Density, Look, Reasoning, ToolOutput};

/// The most time one wake spends on what arrived from the core before it
/// draws: the rest waits in the core's queue, where deltas merge.
pub const SLICE: Duration = Duration::from_millis(1);

/// How many arrivals are taken between two looks for terminal input.
pub const BATCH: usize = 64;

/// How often the spinner turns: its eight frames at 100 ms (09-tui).
pub const SPIN: Duration = Duration::from_millis(100);

/// The glyph set to draw with (`tui.glyphs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlyphSet {
    /// Unicode where the terminal can show it, ASCII otherwise.
    #[default]
    Auto,
    /// The Unicode set.
    Unicode,
    /// The ASCII set.
    Ascii,
}

/// The theme's background (`tui.theme`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// Dark or light, from the background the terminal reports.
    #[default]
    Auto,
    /// The dark one.
    Dark,
    /// The light one.
    Light,
}

/// The `[tui]` settings, in this crate's terms. The binary reads them from
/// the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `tui.prose_width`.
    pub prose_width: usize,
    /// `tui.tool_detail`.
    pub tool_detail: usize,
    /// `tui.tool_output`.
    pub tool_output: ToolOutput,
    /// `tui.reasoning`.
    pub reasoning: Reasoning,
    /// `tui.density`.
    pub density: Density,
    /// `tui.turn_summary`.
    pub turn_summary: bool,
    /// `tui.reduced_motion`.
    pub reduced_motion: bool,
    /// `tui.glyphs`.
    pub glyphs: GlyphSet,
    /// `tui.theme`.
    pub theme: ThemeChoice,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            prose_width: 80,
            tool_detail: 12,
            tool_output: ToolOutput::Auto,
            reasoning: Reasoning::Summary,
            density: Density::Comfortable,
            turn_summary: false,
            reduced_motion: false,
            glyphs: GlyphSet::Auto,
            theme: ThemeChoice::Auto,
        }
    }
}

/// Which session the application opens, after its first frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Open {
    /// A new one in this workspace.
    New {
        /// The workspace root.
        workspace: String,
        /// The settings to start with (`--model`, `--speed` and the like
        /// come as configuration, not here).
        settings: SessionSettings,
        /// Kept in memory only (`--no-session`).
        ephemeral: bool,
    },
    /// A stored one (`--resume`).
    Resume(SessionId),
    /// The newest of this workspace (`--continue`), or a new one when there
    /// is none.
    Continue {
        /// The workspace root.
        workspace: String,
        /// The settings of a new one.
        settings: SessionSettings,
        /// Whether a new one is kept in memory only.
        ephemeral: bool,
    },
    /// A copy of a stored one (`--fork`).
    Fork(SessionId),
}

/// What the application is started with.
pub struct Start {
    /// The program's version, for the session's header.
    pub version: String,
    /// The workspace as the person writes it (`~/src/parser`), for the
    /// header.
    pub folder: String,
    /// The session to open.
    pub open: Open,
    /// A prompt given on the command line, sent once the session is open.
    pub prompt: Option<String>,
    /// The `[tui]` settings.
    pub settings: Settings,
    /// The key map, from `[keys]`.
    pub keymap: KeyMap,
    /// Notices to show first: a `[keys]` entry that could not be used, a
    /// theme that is not known.
    pub notices: Vec<String>,
    /// Earlier prompts of this project.
    pub history: Box<dyn HistoryStore + Send>,
    /// The highlighter for the process: one, whose waker wakes the loop.
    pub highlighter: Highlighter,
}

impl std::fmt::Debug for Start {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Start")
            .field("version", &self.version)
            .field("open", &self.open)
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

/// Why the application ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Exit {
    /// The person quit.
    Quit,
    /// The core went away: its thread ended, or the connection closed.
    Gone(ErrorData),
}

/// The look for `settings` on a terminal with `caps`.
#[must_use]
pub fn look(settings: &Settings, caps: &Capabilities) -> Look {
    let theme = match settings.theme {
        ThemeChoice::Auto => Theme::for_terminal(caps),
        ThemeChoice::Dark | ThemeChoice::Light => {
            let wanted = if settings.theme == ThemeChoice::Dark {
                Background::Dark
            } else {
                Background::Light
            };
            let auto = Theme::for_terminal(caps);
            if auto.background() == wanted {
                auto
            } else {
                // The palette of the background asked for; the tints are
                // worked out against that background's sample colour.
                let sample = if wanted == Background::Dark {
                    crate::theme::ASSUMED_BACKGROUND
                } else {
                    (0xfa, 0xfa, 0xfa)
                };
                Theme::with_palette(
                    &crate::theme::Palette::for_background(wanted),
                    auto.depth(),
                    Some(sample),
                )
            }
        }
    };
    let glyphs: &'static Glyphs = match settings.glyphs {
        GlyphSet::Auto => Glyphs::for_terminal(caps),
        GlyphSet::Unicode => &Glyphs::UNICODE,
        GlyphSet::Ascii => &Glyphs::ASCII,
    };
    let mut look = Look::new(theme, glyphs);
    look.prose_width = settings.prose_width.max(20);
    look.tool_detail = settings.tool_detail.max(1);
    look.tool_output = settings.tool_output;
    look.reasoning = settings.reasoning;
    look.density = settings.density;
    look.turn_summary = settings.turn_summary;
    look.reduced_motion = settings.reduced_motion;
    look.hyperlinks = caps.hyperlinks;
    look
}

/// The interactive mode's whole life on the interface's thread: enters the
/// terminal with `overrides`, makes the link and what the application is
/// started with (`make`, given the terminal's waker and what it can do),
/// runs it, and puts the terminal back whatever happened.
///
/// # Errors
///
/// When the terminal cannot be entered (standard input or output is not a
/// terminal, `TERM=dumb`, a console that cannot process escape sequences),
/// or read or written.
pub fn interactive(
    overrides: Overrides,
    make: impl FnOnce(&Waker, &Capabilities) -> (Box<dyn Link>, Start),
) -> io::Result<Exit> {
    let mut terminal = Terminal::enter(overrides)?;
    let waker = terminal.waker();
    let caps = *terminal.caps();
    let (link, start) = make(&waker, &caps);
    let (cols, rows) = terminal.size()?;
    let mut app = App::new(
        link,
        start,
        &caps,
        usize::from(cols).saturating_sub(1),
        usize::from(rows),
        Instant::now(),
    );
    let ran = run(&mut terminal, &mut app);
    let left = terminal.leave();
    ran?;
    left?;
    Ok(app.exit().cloned().unwrap_or(Exit::Quit))
}

/// Runs the application on `terminal` until it ends: the interface's
/// loop. See the [module documentation](self).
///
/// # Errors
///
/// When the terminal cannot be read or written: after an emergency (a
/// signal, a panic on this thread) every write fails, and the loop ends.
pub fn run<D: Device>(terminal: &mut Terminal<D>, app: &mut App) -> io::Result<()> {
    let (cols, rows) = terminal.size()?;
    let mut presenter = Inline::new(cols, rows);
    app.resize(usize::from(presenter.width()), usize::from(rows));
    let mut scheduler = Scheduler::new();
    scheduler.input();
    loop {
        let now = Instant::now();
        if scheduler.wake(now) == Wake::Now {
            draw(terminal, &mut presenter, app, now)?;
            scheduler.drawn(now);
            app.drawn(Instant::now());
        }
        if app.exit().is_some() {
            break;
        }
        scheduler.animate(app.animating().then_some(SPIN), now);
        let timeout = match scheduler.wake(now) {
            Wake::Now => Some(Duration::ZERO),
            Wake::At(at) => Some(at.saturating_duration_since(now)),
            Wake::Idle => None,
        };

        // Every terminal event already read; a key's frame at once.
        let mut next = terminal.next(timeout)?;
        let mut input = false;
        while let Some(event) = next.take() {
            input |= take(terminal, &mut presenter, &mut scheduler, app, &event)?;
            next = terminal.next(Some(Duration::ZERO))?;
        }
        if input {
            let now = Instant::now();
            draw(terminal, &mut presenter, app, now)?;
            scheduler.drawn(now);
            app.drawn(Instant::now());
            if app.exit().is_some() {
                break;
            }
        }

        // What arrived from the core, a slice of it.
        let began = Instant::now();
        loop {
            let drained = app.drain(Instant::now(), BATCH);
            if drained.request {
                scheduler.input();
            } else if drained.changed {
                scheduler.stream();
            }
            if app.exit().is_some() || !drained.more {
                break;
            }
            if began.elapsed() >= SLICE {
                // The rest waits in the core's queue, for the next frame.
                scheduler.stream();
                break;
            }
            // Keys first, between batches.
            if let Some(event) = terminal.next(Some(Duration::ZERO))?
                && take(terminal, &mut presenter, &mut scheduler, app, &event)?
            {
                scheduler.input();
                scheduler.stream();
                break;
            }
        }
        if app.exit().is_some() {
            let now = Instant::now();
            draw(terminal, &mut presenter, app, now)?;
            break;
        }
    }
    let caps = *terminal.caps();
    presenter.finish(&caps, terminal)
}

/// One terminal event: what it does to the presenter and the scheduler,
/// then to the application. Says whether it was input.
fn take<D: Device>(
    terminal: &mut Terminal<D>,
    presenter: &mut Inline,
    scheduler: &mut Scheduler,
    app: &mut App,
    event: &Event,
) -> io::Result<bool> {
    match event {
        Event::Resize { cols, rows } => {
            presenter.resize(*cols, *rows);
            app.resize(usize::from(presenter.width()), usize::from(*rows));
            scheduler.input();
        }
        Event::Resumed => {
            presenter.restart();
            scheduler.input();
        }
        Event::Reply(_) => {
            app.set_caps(terminal.caps());
            scheduler.input();
        }
        _ => {}
    }
    let input = app.terminal(event, Instant::now());
    for effect in app.take_effects() {
        match effect {
            Effect::Redraw => presenter.redraw(),
            Effect::Suspend => {
                let caps = *terminal.caps();
                presenter.finish(&caps, terminal)?;
                terminal.suspend();
                // On Unix the program stops here; on resuming, the terminal
                // gives `Event::Resumed`, and the presenter starts again.
                presenter.restart();
                scheduler.input();
            }
        }
    }
    Ok(input)
}

fn draw<D: Device>(
    terminal: &mut Terminal<D>,
    presenter: &mut Inline,
    app: &mut App,
    now: Instant,
) -> io::Result<()> {
    let (commit, live, cursor) = app.frame(now);
    let caps = *terminal.caps();
    presenter.draw(
        &Frame {
            commit: &commit,
            live: &live,
            cursor: Some(cursor),
        },
        &caps,
        terminal,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
