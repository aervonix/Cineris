//! The full-screen presenter's view of the session (since step 25;
//! `docs/architecture/09-tui.md`, "Full screen as the alternative";
//! `docs/steps/25/answered.md`, point 2): the kept transcript, the rows of
//! the block still open and the live rows, in a window that follows the end
//! until it is moved up, above the same panels and bottom the inline
//! presenter draws.
//!
//! A committed box is drawn as the inline presenter committed it, unless a
//! click opened or closed it here; the viewer (`Ctrl+O`) is the way to it
//! by keys, on the same alternate screen.

use std::collections::BTreeMap;
use std::time::Instant;

use cortex_term::{Button, Cursor, Key, Line, Mouse, MouseKind, Span, Wheel};

use super::App;
use super::viewer::WHEEL_ROWS;
use crate::app::scroll::{Place, Scroll, Source, Window};
use crate::keys::{Action, Context, Gesture};
use crate::theme::Token;
use crate::view::Placement;

/// The full-screen presenter's view: where the window is, the boxes opened
/// or closed here, and what the last frame drew, for the mouse.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScreenView {
    /// The window over the transcript.
    scroll: Scroll,
    /// The boxes a click opened (`true`) or closed, by their place among
    /// the rounds; the others are drawn as they committed.
    open: BTreeMap<usize, bool>,
    /// Where each row of the transcript area the last frame drew came
    /// from: what a click finds. The area is the screen's top.
    shown: Vec<Place>,
    /// Where the left button went down, for a click.
    pressed: Option<(u16, u16)>,
    /// The width the open block's rows were last laid out at.
    width: usize,
}

impl ScreenView {
    /// Forgets the rows made: the look changed.
    pub(super) fn forget_rows(&mut self) {
        self.scroll.rows.clear();
    }
}

/// A block's rows in the full-screen view: a round as it committed, or as a
/// click left it.
fn block_rows(app: &App, open: &BTreeMap<usize, bool>, block: usize, width: usize) -> Vec<Line> {
    let Some(kept) = app.kept.get(block) else {
        return Vec::new();
    };
    let placement = match app
        .kept
        .round_number(block)
        .and_then(|round| open.get(&round))
    {
        Some(&open) => Placement::Viewer {
            selected: false,
            open,
        },
        None => Placement::Committed,
    };
    kept.rows(placement, width, &app.look)
}

impl App {
    /// Whether the application draws with the full-screen presenter.
    #[must_use]
    pub fn full_screen(&self) -> bool {
        self.screen.is_some()
    }

    /// Whether the viewer and the full-screen presenter report the mouse
    /// (`terminal.mouse`).
    #[must_use]
    pub fn mouse(&self) -> bool {
        self.settings.mouse
    }

    /// Runs `act` on the full-screen view's window, with what it is drawn
    /// from at `width`: the kept blocks, then the rows of the block still
    /// open and the live rows.
    fn with_screen<T>(
        &self,
        view: &mut ScreenView,
        width: usize,
        live: &[Line],
        act: impl FnOnce(&mut Scroll, &mut Source<'_>) -> T,
    ) -> T {
        let mut scroll = std::mem::take(&mut view.scroll);
        let open = &view.open;
        let mut make = |block: usize| block_rows(self, open, block, width);
        let mut source = Source {
            blocks: self.kept.len(),
            make: &mut make,
            live,
            width,
        };
        let result = act(&mut scroll, &mut source);
        view.scroll = scroll;
        result
    }

    /// The rows under the kept blocks: those committed of the block still
    /// open, then the live rows.
    fn screen_live(&self, width: usize) -> Vec<Line> {
        let mut rows = self.transcript.open_rows().to_vec();
        rows.extend(self.transcript.live(width, &self.look, self.tick));
        rows
    }

    /// The full-screen presenter's frame for `now`: every row of the window,
    /// top to bottom (the transcript area, the panels' block, the bottom),
    /// and where the cursor goes, if anywhere.
    pub fn screen_frame(&mut self, now: Instant) -> (Vec<Line>, Option<Cursor>) {
        let width = self.width.max(1);
        let Some(mut view) = self.screen.take() else {
            return (Vec::new(), None);
        };
        // The block still open was laid out at another width: again, at
        // this one (an answer resized while it streams).
        if view.width != width {
            if view.width != 0 {
                self.transcript.rewrap_open(width, &self.look);
            }
            view.width = width;
        }
        let (_, boxes) = self.prepare(now);
        let live = self.screen_live(width);
        // The live rows the panels may leave the transcript, as inline: the
        // full screen's transcript area scrolls, so they count no further.
        let live_len = live.len().min(4);
        let bottom = self.bottom(now, width, &boxes, live_len);
        let rows = self.rows.max(1);
        let height = rows
            .saturating_sub(bottom.block.len() + bottom.lines.len())
            .max(1);
        let mut window = self.with_screen(&mut view, width, &live, |scroll, source| {
            scroll.window(source, height)
        });
        if !window.at_end {
            self.more_below(&mut window, width);
        }
        view.shown = window.places;
        let mut lines = window.rows;
        let top = lines.len() + bottom.block.len();
        lines.extend(bottom.block);
        lines.extend(bottom.lines);
        let cursor = bottom.cursor.map(|(row, col)| Cursor {
            row: top + row,
            col,
        });
        self.screen = Some(view);
        (lines, cursor)
    }

    /// The last row of a window that does not follow the end says there is
    /// more below, and how to go there.
    fn more_below(&self, window: &mut Window, width: usize) {
        let Some(last) = window.rows.last_mut() else {
            return;
        };
        let keys = self
            .input
            .keymap()
            .keys_of(Action::ScreenPageDown)
            .first()
            .map(|key| crate::keys::name(*key))
            .unwrap_or_else(|| "pgdn".to_owned());
        let text = format!("  more below · {keys} goes to the end");
        *last = crate::layout::cut(
            &Line::from(Span::new(text, self.look.theme.style(Token::Hint))),
            width,
            self.look.glyphs.ellipsis,
        );
        if let Some(place) = window.places.last_mut() {
            *place = Place::Empty;
        }
    }

    /// A key the full-screen view may take: one of its context's, bound to
    /// nothing where the input has the keyboard (since step 25). Says
    /// whether it took it.
    pub(super) fn screen_key(&mut self, key: Key) -> bool {
        if self.screen.is_none() || self.viewer.is_some() || !self.dialogs.is_empty() {
            return false;
        }
        let keymap = self.input.keymap();
        let Some(action) = keymap
            .lookup(&[Context::Screen], key)
            .filter(|action| action.context() == Context::Screen)
        else {
            return false;
        };
        // The editor's, a menu's or a focused panel's own keys come first.
        let contexts = self.input.contexts();
        if keymap
            .lookup(&contexts, key)
            .is_some_and(|other| other.context() != Context::Global)
        {
            return false;
        }
        self.screen_act(action, None);
        true
    }

    /// The mouse in the full-screen view: the wheel scrolls, and a click on
    /// a committed box's row opens or closes it, each through the key map's
    /// gestures.
    pub(super) fn screen_mouse(&mut self, mouse: Mouse) -> bool {
        let Some(view) = self.screen.as_mut() else {
            return false;
        };
        let gesture = match mouse.kind {
            MouseKind::Wheel(Wheel::Up) => Some(Gesture::WheelUp),
            MouseKind::Wheel(Wheel::Down) => Some(Gesture::WheelDown),
            MouseKind::Press(Button::Left) => {
                view.pressed = Some((mouse.col, mouse.row));
                None
            }
            MouseKind::Release(_) => view
                .pressed
                .take()
                .filter(|&pressed| pressed == (mouse.col, mouse.row))
                .map(|_| Gesture::Click),
            _ => None,
        };
        let Some(action) = gesture.and_then(|gesture| {
            self.input
                .keymap()
                .lookup_gesture(&[Context::Screen], gesture)
        }) else {
            return false;
        };
        // A click under a dialog does nothing: the dialog has the keys.
        if action == Action::ScreenToggle && !self.dialogs.is_empty() {
            return true;
        }
        self.screen_act(action, Some(usize::from(mouse.row)));
        true
    }

    /// Does `action` in the full-screen view; `row` is the screen row a
    /// click was on.
    fn screen_act(&mut self, action: Action, row: Option<usize>) {
        let width = self.width.max(1);
        let live = self.screen_live(width);
        let Some(mut view) = self.screen.take() else {
            return;
        };
        let page = view.scroll.height.max(1);
        match action {
            Action::ScreenPageUp => self.with_screen(&mut view, width, &live, |scroll, source| {
                scroll.up(source, page.saturating_sub(1).max(1));
            }),
            Action::ScreenPageDown => {
                self.with_screen(&mut view, width, &live, |scroll, source| {
                    scroll.down(source, page.saturating_sub(1).max(1));
                })
            }
            Action::ScreenLineUp => self.with_screen(&mut view, width, &live, |scroll, source| {
                scroll.up(source, 1);
            }),
            Action::ScreenLineDown => {
                self.with_screen(&mut view, width, &live, |scroll, source| {
                    scroll.down(source, 1);
                })
            }
            Action::ScreenScrollUp => {
                self.with_screen(&mut view, width, &live, |scroll, source| {
                    scroll.up(source, WHEEL_ROWS);
                })
            }
            Action::ScreenScrollDown => {
                self.with_screen(&mut view, width, &live, |scroll, source| {
                    scroll.down(source, WHEEL_ROWS);
                });
            }
            Action::ScreenToggle => {
                let clicked =
                    row.and_then(|row| view.shown.get(row))
                        .and_then(|place| match place {
                            Place::Kept { block, .. } => {
                                self.kept.round_number(*block).map(|round| (*block, round))
                            }
                            _ => None,
                        });
                if let Some((block, round)) = clicked {
                    // Open if it showed no detail as committed; then each
                    // click turns it over.
                    let now_open = match view.open.get(&round) {
                        Some(&open) => !open,
                        None => !self.kept.get(block).is_some_and(|kept| {
                            kept.round().is_some_and(|round| {
                                crate::view::round::shows_detail(round, &self.look)
                            })
                        }),
                    };
                    view.open.insert(round, now_open);
                    view.scroll.rows.forget(block);
                }
            }
            _ => {}
        }
        self.screen = Some(view);
    }

    /// Follows the end again: a prompt was sent, or a dialog opened.
    pub(super) fn screen_follow(&mut self) {
        if let Some(view) = self.screen.as_mut() {
            view.scroll.follow();
        }
    }
}
