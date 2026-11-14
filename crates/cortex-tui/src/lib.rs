//! The interactive UI: transcript, editor, dialogs, hint row, slash commands
//! and themes (`docs/architecture/09-tui.md`).
//!
//! ```text
//! events from core ─► state ─► view (pure: state → styled lines) ─► presenter ─► terminal
//! keys ─► key map ─► actions ─► state changes and operations to core
//! ```
//!
//! The crate depends only on `cortex-protocol`, `cortex-base` and
//! `cortex-term`. It holds no agent logic: it shows events and sends
//! operations.
//!
//! # Modules, and the step that builds each
//!
//! | Module | What it holds | Step |
//! |---|---|---|
//! | [`theme`] | Semantic colour tokens, the default theme, the accent and its text tint, dark and light | 22 |
//! | [`glyphs`] | The Unicode and ASCII glyph sets | 22 |
//! | [`layout`] | Styled text at a width: wrapping, cutting with an ellipsis, padding | 22 |
//! | [`rounded`] | The rounded box, with a label in its bottom border | 22 |
//! | [`view`] | View functions, and the [`view::Look`] they draw with | 22 |
//! | `view::*` | The transcript's elements, markdown, the session header and the hint row | 22 |
//! | [`editor`] | The prompt's text by grapheme cluster, with chips, undo and the kill slot | 21 |
//! | [`keys`] | The key map and the [`Action`](keys::Action) enum: every key reaches the interface as an action | 21 |
//! | [`history`] | Prompt history, over a [`HistoryStore`](history::HistoryStore) the caller fills | 21 |
//! | [`paste`] | Paste handling: text, or a chip when long | 21 |
//! | [`complete`] | Completion menus and pickers, and the fuzzy matcher | 21 |
//! | [`focus`] | The `Ctrl+P` focus ring: the input, the live tool boxes, the panels | 21 |
//! | [`input`] | [`Input`](input::Input): the editor, its menus, history and focus driven by events | 21 |
//! | [`external`] | The external-editor hand-off | 21 |
//! | `view::input`, `view::menu` | The input box and the menus | 21 |
//!
//! Each module's code is in its own files, so that the steps that build
//! this crate side by side do not edit the same ones.

// Step 22: the look and the transcript.
pub mod glyphs;
pub mod layout;
pub mod rounded;
pub mod theme;
pub mod view;

// Step 21: the editor.
pub mod complete;
pub mod editor;
pub mod external;
pub mod focus;
pub mod history;
pub mod input;
pub mod keys;
pub mod paste;

#[cfg(test)]
mod testing;
