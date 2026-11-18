//! Keys to actions (`docs/architecture/09-tui.md`, "Default keys").
//!
//! No key is tested for in the interface's code. A key is looked up in the
//! [`KeyMap`], in the [`Context`]s of what has the keyboard (a menu over
//! the editor, say), and what it finds is an [`Action`]. A slash command
//! names the same actions, so everything is reachable both ways and can be
//! tested without a terminal.
//!
//! **Each action belongs to one context.** `Enter` is [`Action::Submit`] in
//! the editor and [`Action::MenuAccept`] in a menu; a menu is looked up
//! first, then the editor under it. [`ACTIONS`] is the one table of every
//! action: its context, its name in `[keys]`, and its default keys.
//!
//! **What terminals merge.** Plain virtual-terminal input sends the same
//! byte for `Shift+Enter` as for `Enter`, and for `Ctrl+Shift+Z` as for
//! `Ctrl+Z`, so those two reach the map as `Enter` and `Ctrl+Z` unless the
//! enhanced keyboard protocol or Windows Terminal's win32 input mode is on.
//! Each action they name has a second key that every terminal sends:
//! `Alt+Enter` and `Ctrl+J` for a new line, `Alt+Z` for redo.
//!
//! **Typing.** A key that is bound to nothing and types a character (a
//! character with no modifier but Shift) is typed. That is the absence of
//! a binding, not a test for a key.

mod parse;

use std::collections::HashMap;
use std::fmt;

use cineris_term::{Key, KeyCode, Mods};

pub use parse::{BindingError, name, parse};

/// What has the keyboard, for looking a key up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// Everywhere: looked up last, whatever has the keyboard.
    Global,
    /// The prompt's editor.
    Editor,
    /// A completion menu or a picker, above the editor.
    Menu,
    /// A picker (`/model`, `/effort`, the session picker), above
    /// [`Context::Menu`]: it filters on what is typed into it, not into the
    /// editor.
    Picker,
    /// A picker with a toggle on `Tab` (the `/effort` picker's fast mode),
    /// above [`Context::Picker`]: there `Tab` toggles and only `Enter`
    /// accepts.
    Toggle,
    /// The history search (`Ctrl+R`).
    Search,
    /// A live tool box or a panel that has the focus (`Ctrl+P`).
    Focus,
    /// A dialog in place of the input box: an approval, a question, trust
    /// (step 23). Its keys answer it.
    Dialog,
    /// The viewer (`Ctrl+O`): moving between boxes, opening one, scrolling,
    /// searching, and the mouse's gestures (since step 25).
    Viewer,
    /// The full-screen presenter's own keys, looked up after the editor's:
    /// scrolling, searching, and the mouse's gestures (since step 25).
    Screen,
}

impl Context {
    /// Its name, as `[keys]` prefixes an action's: `menu.accept`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Editor => "editor",
            Self::Menu => "menu",
            Self::Picker => "picker",
            Self::Toggle => "toggle",
            Self::Search => "search",
            Self::Focus => "focus",
            Self::Dialog => "dialog",
            Self::Viewer => "viewer",
            Self::Screen => "screen",
        }
    }
}

/// Something the interface does: from a key through the map, or from a
/// slash command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Action {
    // ------------------------------------------------------------ global
    /// Interrupt the running turn; when idle, clear the editor; twice when
    /// idle and empty, quit (`Ctrl+C`).
    Interrupt,
    /// Redraw the live region (`Ctrl+L`).
    Redraw,
    /// Open the viewer at the latest tool box (`Ctrl+O`).
    OpenViewer,
    /// Move the focus to the next of the input, the live tool boxes and the
    /// panels (`Ctrl+P`).
    FocusNext,
    /// Open or close the running round's box in place (`Ctrl+T`).
    ToggleRound,
    /// Stop the program as a shell's job (Unix only). No key by default.
    Suspend,

    // ------------------------------------------------------------ editor
    /// Send the prompt, or steer or queue it while a turn runs (`Enter`).
    Submit,
    /// Start a new line (`Shift+Enter`, `Alt+Enter`, `Ctrl+J`).
    NewLine,
    /// Interrupt the running turn (`Esc`). Nothing when idle.
    Stop,
    /// Quit, when the editor is empty (`Ctrl+D`).
    Quit,
    /// One character left.
    Left,
    /// One character right.
    Right,
    /// To the start of the word before the cursor (`Alt+B`).
    WordLeft,
    /// To the end of the word after the cursor (`Alt+F`).
    WordRight,
    /// To the start of the line (`Ctrl+A`).
    LineStart,
    /// To the end of the line (`Ctrl+E`).
    LineEnd,
    /// To the start of the prompt.
    TextStart,
    /// To the end of the prompt.
    TextEnd,
    /// Up a row; on the first row, the previous prompt in history.
    Up,
    /// Down a row; on the last row, the next prompt in history.
    Down,
    /// Delete the character before the cursor.
    Backspace,
    /// Delete the character after the cursor.
    Delete,
    /// Delete the word before the cursor into the kill slot (`Ctrl+W`).
    KillWordBack,
    /// Delete the word after the cursor into the kill slot (`Alt+D`).
    KillWordForward,
    /// Delete to the end of the line into the kill slot (`Ctrl+K`).
    KillToLineEnd,
    /// Delete to the start of the line into the kill slot (`Ctrl+U`).
    KillToLineStart,
    /// Insert what the kill slot holds (`Ctrl+Y`).
    Yank,
    /// Undo (`Ctrl+Z`).
    Undo,
    /// Redo (`Ctrl+Shift+Z`, `Alt+Z`).
    Redo,
    /// Replace the paste chip beside the cursor with its text (`Ctrl+X`).
    ExpandChip,
    /// Search the prompt history (`Ctrl+R`).
    HistorySearch,
    /// Edit the prompt in the external editor (`Ctrl+G`).
    ExternalEditor,
    /// With a turn running, switch the prompt between steering it and
    /// queueing after it (`Tab`, when there is nothing to complete).
    SwitchSendMode,

    // -------------------------------------------------------------- menu
    /// Select the row above.
    MenuUp,
    /// Select the row below.
    MenuDown,
    /// Accept the selected row (`Tab`, `Enter`).
    MenuAccept,
    /// Close the menu (`Esc`).
    MenuClose,

    // ------------------------------------------------------------ picker
    /// Delete the last character of the picker's filter (`Backspace`).
    PickerErase,
    /// Remove the selected row of a picker that removes (`/permissions`):
    /// `Delete` (since step 23).
    PickerRemove,
    /// Turn the picker's toggle on or off: fast mode in `/effort` (`Tab`).
    ToggleFast,

    // ------------------------------------------------------------ search
    /// The next older match (`Ctrl+R`, `Up`).
    SearchOlder,
    /// The next newer match (`Down`).
    SearchNewer,
    /// Take the match into the editor (`Enter`).
    SearchAccept,
    /// Leave the search, with the editor as it was (`Esc`).
    SearchCancel,
    /// Delete the last character of the search (`Backspace`).
    SearchErase,

    // ------------------------------------------------------------- focus
    /// Open or close the focused box, or open the focused panel's item
    /// (`Enter`).
    FocusActivate,
    /// Give the keyboard back to the input (`Esc`).
    FocusReturn,
    /// Move up in the focused panel.
    FocusUp,
    /// Move down in the focused panel.
    FocusDown,
    /// A panel's own keys (step 24), each reaching the application as
    /// `Outcome::Panel` while a panel has the focus; a box has no use for
    /// them. Skip the focused plan item (`space`).
    FocusSkip,
    /// Reword the focused item, or edit the focused entry (`e`).
    FocusEdit,
    /// Add a plan item after the focused one (`a`).
    FocusAdd,
    /// Delete the focused item or entry (`d`).
    FocusDelete,
    /// Move the focused plan item up (`alt+up`).
    FocusMoveUp,
    /// Move the focused plan item down (`alt+down`).
    FocusMoveDown,
    /// Accept the focused entry; yes to a delete it asks about (`y`).
    FocusAccept,
    /// Disable the focused entry (`n`).
    FocusDisable,
    /// Promote the focused entry to the repository's file (`p`).
    FocusPromote,
    /// Ask again for what the panel shows (`r`).
    FocusRefresh,
    /// The next range of time the panel shows (`tab`).
    FocusRange,
    /// The next grouping the panel shows (`g`).
    FocusGroup,
    /// Close the focused panel (`x`).
    FocusClose,

    // ------------------------------------------------------------ dialog
    /// Allow this call only; trust the project; the first choice
    /// (`y`).
    AllowOnce,
    /// Allow this call and calls like it until the session ends (`s`).
    AllowSession,
    /// Allow this call and calls like it in this project from now on
    /// (`a`).
    AllowProject,
    /// Deny the call; do not trust the project; leave a question
    /// unanswered (`n`, `Esc`).
    Deny,
    /// Deny the call with a note for the model (`e`).
    DenyWithNote,
    /// Never trust the project: kept, so that it is not asked about at
    /// later starts either (`x`).
    Never,
    /// Show the request's detail whole, in the viewer (`Ctrl+O`).
    ShowWhole,
    /// In the full screen, move from the dialog to the transcript, to read
    /// what led to it while it stays up, and back (`Tab`, since step 25).
    DialogTranscript,

    // ------------------------------------------------------------ viewer
    /// Select the box before (`Up`, `k`), or scroll a row up when there is
    /// none.
    ViewerUp,
    /// Select the box after (`Down`, `j`), or scroll a row down.
    ViewerDown,
    /// Open or close the selected box (`Enter`), or the box clicked.
    ViewerToggle,
    /// Scroll a page up (`PageUp`).
    ViewerPageUp,
    /// Scroll a page down (`PageDown`).
    ViewerPageDown,
    /// To the first row (`Home`, `g`).
    ViewerTop,
    /// To the last row (`End`, `G`).
    ViewerBottom,
    /// Search (`/`).
    ViewerSearch,
    /// Find the last search again (`n`).
    ViewerNext,
    /// Return to the session (`Esc`, `q`, `Ctrl+O`).
    ViewerClose,
    /// Scroll up a little: the wheel.
    ViewerScrollUp,
    /// Scroll down a little: the wheel.
    ViewerScrollDown,

    // ------------------------------------------------------------ screen
    /// Scroll the transcript a page up (`PageUp`), which stops following
    /// the end.
    ScreenPageUp,
    /// Scroll a page down (`PageDown`); at the end, follow it again.
    ScreenPageDown,
    /// Scroll a row up (`Shift+Up`).
    ScreenLineUp,
    /// Scroll a row down (`Shift+Down`).
    ScreenLineDown,
    /// Search the transcript (`Ctrl+F`).
    ScreenSearch,
    /// Open or close the box clicked.
    ScreenToggle,
    /// Scroll up a little: the wheel.
    ScreenScrollUp,
    /// Scroll down a little: the wheel.
    ScreenScrollDown,
}

/// A mouse gesture a binding can name (since step 25), in the contexts of
/// the viewer and the full-screen presenter only, where the mouse is
/// reported: `click`, `wheel_up` and `wheel_down`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Gesture {
    /// The left button pressed and released on the same cell.
    Click,
    /// The wheel turned up, a notch.
    WheelUp,
    /// The wheel turned down, a notch.
    WheelDown,
}

impl Gesture {
    /// Its name in a binding.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::WheelUp => "wheel_up",
            Self::WheelDown => "wheel_down",
        }
    }

    /// The gesture a binding's text names, if it names one.
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "click" => Some(Self::Click),
            "wheel_up" => Some(Self::WheelUp),
            "wheel_down" => Some(Self::WheelDown),
            _ => None,
        }
    }
}

/// What a binding's text names: a key, or a mouse gesture (since step 25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// A key with its modifiers.
    Key(Key),
    /// A mouse gesture.
    Gesture(Gesture),
}

/// Reads a binding's text: a mouse gesture's name (`click`, `wheel_up`,
/// `wheel_down`), or a key as [`parse`] reads it.
///
/// # Errors
///
/// When it is neither.
pub fn parse_binding(text: &str) -> Result<Binding, BindingError> {
    match Gesture::from_name(text) {
        Some(gesture) => Ok(Binding::Gesture(gesture)),
        None => parse(text).map(Binding::Key),
    }
}

impl Context {
    /// Whether the mouse is reported where this context applies, so that a
    /// gesture may be bound in it.
    #[must_use]
    pub fn takes_gestures(self) -> bool {
        matches!(self, Self::Viewer | Self::Screen)
    }
}

/// One action: what it is, where it applies, its name in `[keys]`, and the
/// keys it has by default.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// The action.
    pub action: Action,
    /// Where it applies.
    pub context: Context,
    /// Its name in `[keys]`, after its context's: `editor.new_line`.
    pub name: &'static str,
    /// Its default keys, as binding text.
    pub keys: &'static [&'static str],
}

const fn entry(
    action: Action,
    context: Context,
    name: &'static str,
    keys: &'static [&'static str],
) -> Entry {
    Entry {
        action,
        context,
        name,
        keys,
    }
}

/// Every action, with its default keys. The order is the order help lists
/// them in.
pub const ACTIONS: &[Entry] = &[
    entry(Action::Interrupt, Context::Global, "interrupt", &["ctrl+c"]),
    entry(Action::Redraw, Context::Global, "redraw", &["ctrl+l"]),
    entry(
        Action::OpenViewer,
        Context::Global,
        "open_viewer",
        &["ctrl+o"],
    ),
    entry(
        Action::FocusNext,
        Context::Global,
        "focus_next",
        &["ctrl+p"],
    ),
    entry(
        Action::ToggleRound,
        Context::Global,
        "toggle_round",
        &["ctrl+t"],
    ),
    entry(Action::Suspend, Context::Global, "suspend", &[]),
    entry(Action::Submit, Context::Editor, "submit", &["enter"]),
    entry(
        Action::NewLine,
        Context::Editor,
        "new_line",
        &["shift+enter", "alt+enter", "ctrl+j"],
    ),
    entry(Action::Stop, Context::Editor, "stop", &["esc"]),
    entry(Action::Quit, Context::Editor, "quit", &["ctrl+d"]),
    entry(Action::Left, Context::Editor, "left", &["left"]),
    entry(Action::Right, Context::Editor, "right", &["right"]),
    entry(
        Action::WordLeft,
        Context::Editor,
        "word_left",
        &["alt+b", "ctrl+left", "alt+left"],
    ),
    entry(
        Action::WordRight,
        Context::Editor,
        "word_right",
        &["alt+f", "ctrl+right", "alt+right"],
    ),
    entry(
        Action::LineStart,
        Context::Editor,
        "line_start",
        &["ctrl+a", "home"],
    ),
    entry(
        Action::LineEnd,
        Context::Editor,
        "line_end",
        &["ctrl+e", "end"],
    ),
    entry(
        Action::TextStart,
        Context::Editor,
        "text_start",
        &["ctrl+home"],
    ),
    entry(Action::TextEnd, Context::Editor, "text_end", &["ctrl+end"]),
    entry(Action::Up, Context::Editor, "up", &["up"]),
    entry(Action::Down, Context::Editor, "down", &["down"]),
    entry(
        Action::Backspace,
        Context::Editor,
        "backspace",
        &["backspace", "shift+backspace"],
    ),
    entry(Action::Delete, Context::Editor, "delete", &["delete"]),
    entry(
        Action::KillWordBack,
        Context::Editor,
        "kill_word_back",
        &["ctrl+w", "alt+backspace", "ctrl+backspace"],
    ),
    entry(
        Action::KillWordForward,
        Context::Editor,
        "kill_word_forward",
        &["alt+d", "ctrl+delete"],
    ),
    entry(
        Action::KillToLineEnd,
        Context::Editor,
        "kill_to_line_end",
        &["ctrl+k"],
    ),
    entry(
        Action::KillToLineStart,
        Context::Editor,
        "kill_to_line_start",
        &["ctrl+u"],
    ),
    entry(Action::Yank, Context::Editor, "yank", &["ctrl+y"]),
    entry(
        Action::Undo,
        Context::Editor,
        "undo",
        &["ctrl+z", "ctrl+_", "ctrl+/"],
    ),
    entry(
        Action::Redo,
        Context::Editor,
        "redo",
        &["ctrl+shift+z", "alt+z"],
    ),
    entry(
        Action::ExpandChip,
        Context::Editor,
        "expand_chip",
        &["ctrl+x"],
    ),
    entry(
        Action::HistorySearch,
        Context::Editor,
        "history_search",
        &["ctrl+r"],
    ),
    entry(
        Action::ExternalEditor,
        Context::Editor,
        "external_editor",
        &["ctrl+g"],
    ),
    entry(
        Action::SwitchSendMode,
        Context::Editor,
        "switch_send_mode",
        &["tab"],
    ),
    entry(Action::MenuUp, Context::Menu, "up", &["up", "shift+tab"]),
    entry(Action::MenuDown, Context::Menu, "down", &["down"]),
    entry(
        Action::MenuAccept,
        Context::Menu,
        "accept",
        &["tab", "enter"],
    ),
    entry(Action::MenuClose, Context::Menu, "close", &["esc"]),
    entry(
        Action::PickerErase,
        Context::Picker,
        "erase",
        &["backspace"],
    ),
    entry(Action::PickerRemove, Context::Picker, "remove", &["delete"]),
    entry(Action::ToggleFast, Context::Toggle, "fast", &["tab"]),
    entry(
        Action::SearchOlder,
        Context::Search,
        "older",
        &["ctrl+r", "up"],
    ),
    entry(Action::SearchNewer, Context::Search, "newer", &["down"]),
    entry(Action::SearchAccept, Context::Search, "accept", &["enter"]),
    entry(
        Action::SearchCancel,
        Context::Search,
        "cancel",
        &["esc", "ctrl+g"],
    ),
    entry(
        Action::SearchErase,
        Context::Search,
        "erase",
        &["backspace"],
    ),
    entry(
        Action::FocusActivate,
        Context::Focus,
        "activate",
        &["enter"],
    ),
    entry(Action::FocusReturn, Context::Focus, "return", &["esc"]),
    entry(Action::FocusUp, Context::Focus, "up", &["up"]),
    entry(Action::FocusDown, Context::Focus, "down", &["down"]),
    entry(Action::FocusSkip, Context::Focus, "skip", &["space"]),
    entry(Action::FocusEdit, Context::Focus, "edit", &["e"]),
    entry(Action::FocusAdd, Context::Focus, "add", &["a"]),
    entry(Action::FocusDelete, Context::Focus, "delete", &["d"]),
    entry(Action::FocusMoveUp, Context::Focus, "move_up", &["alt+up"]),
    entry(
        Action::FocusMoveDown,
        Context::Focus,
        "move_down",
        &["alt+down"],
    ),
    entry(Action::FocusAccept, Context::Focus, "accept", &["y"]),
    entry(Action::FocusDisable, Context::Focus, "disable", &["n"]),
    entry(Action::FocusPromote, Context::Focus, "promote", &["p"]),
    entry(Action::FocusRefresh, Context::Focus, "refresh", &["r"]),
    entry(Action::FocusRange, Context::Focus, "range", &["tab"]),
    entry(Action::FocusGroup, Context::Focus, "group", &["g"]),
    entry(Action::FocusClose, Context::Focus, "close", &["x"]),
    entry(Action::AllowOnce, Context::Dialog, "allow_once", &["y"]),
    entry(
        Action::AllowSession,
        Context::Dialog,
        "allow_session",
        &["s"],
    ),
    entry(
        Action::AllowProject,
        Context::Dialog,
        "allow_project",
        &["a"],
    ),
    entry(Action::Deny, Context::Dialog, "deny", &["n", "esc"]),
    entry(
        Action::DenyWithNote,
        Context::Dialog,
        "deny_with_note",
        &["e"],
    ),
    entry(Action::Never, Context::Dialog, "never", &["x"]),
    entry(
        Action::ShowWhole,
        Context::Dialog,
        "show_whole",
        &["ctrl+o"],
    ),
    entry(
        Action::DialogTranscript,
        Context::Dialog,
        "transcript",
        &["tab"],
    ),
    entry(Action::ViewerUp, Context::Viewer, "up", &["up", "k"]),
    entry(Action::ViewerDown, Context::Viewer, "down", &["down", "j"]),
    entry(
        Action::ViewerToggle,
        Context::Viewer,
        "toggle",
        &["enter", "click"],
    ),
    entry(
        Action::ViewerPageUp,
        Context::Viewer,
        "page_up",
        &["pageup"],
    ),
    entry(
        Action::ViewerPageDown,
        Context::Viewer,
        "page_down",
        &["pagedown"],
    ),
    entry(Action::ViewerTop, Context::Viewer, "top", &["home", "g"]),
    entry(
        Action::ViewerBottom,
        Context::Viewer,
        "bottom",
        &["end", "shift+g"],
    ),
    entry(Action::ViewerSearch, Context::Viewer, "search", &["/"]),
    entry(Action::ViewerNext, Context::Viewer, "next", &["n"]),
    entry(
        Action::ViewerClose,
        Context::Viewer,
        "close",
        &["esc", "q", "ctrl+o"],
    ),
    entry(
        Action::ViewerScrollUp,
        Context::Viewer,
        "scroll_up",
        &["wheel_up"],
    ),
    entry(
        Action::ViewerScrollDown,
        Context::Viewer,
        "scroll_down",
        &["wheel_down"],
    ),
    entry(
        Action::ScreenPageUp,
        Context::Screen,
        "page_up",
        &["pageup"],
    ),
    entry(
        Action::ScreenPageDown,
        Context::Screen,
        "page_down",
        &["pagedown"],
    ),
    entry(
        Action::ScreenLineUp,
        Context::Screen,
        "line_up",
        &["shift+up"],
    ),
    entry(
        Action::ScreenLineDown,
        Context::Screen,
        "line_down",
        &["shift+down"],
    ),
    entry(Action::ScreenSearch, Context::Screen, "search", &["ctrl+f"]),
    entry(Action::ScreenToggle, Context::Screen, "toggle", &["click"]),
    entry(
        Action::ScreenScrollUp,
        Context::Screen,
        "scroll_up",
        &["wheel_up"],
    ),
    entry(
        Action::ScreenScrollDown,
        Context::Screen,
        "scroll_down",
        &["wheel_down"],
    ),
];

impl Action {
    /// Its row in [`ACTIONS`].
    #[must_use]
    pub fn entry(self) -> &'static Entry {
        ACTIONS
            .iter()
            .find(|entry| entry.action == self)
            .unwrap_or(&ACTIONS[0])
    }

    /// The context it applies in.
    #[must_use]
    pub fn context(self) -> Context {
        self.entry().context
    }

    /// Its name in `[keys]`: the context's name, a dot, its own.
    #[must_use]
    pub fn name(self) -> String {
        let entry = self.entry();
        format!("{}.{}", entry.context.name(), entry.name)
    }

    /// The action `[keys]` names `name` (`editor.new_line`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let (context, own) = name.split_once('.')?;
        ACTIONS
            .iter()
            .find(|entry| entry.context.name() == context && entry.name == own)
            .map(|entry| entry.action)
    }

    /// Whether it is one of a panel's own keys (step 24): a focused panel
    /// is given it, and a focused box has no use for it.
    #[must_use]
    pub fn is_panel_key(self) -> bool {
        PANEL_KEYS.contains(&self)
    }
}

/// A panel's own keys (step 24), in the order help lists them: each comes
/// to the application as `Outcome::Panel` while a panel has the focus.
pub const PANEL_KEYS: [Action; 13] = [
    Action::FocusSkip,
    Action::FocusEdit,
    Action::FocusAdd,
    Action::FocusDelete,
    Action::FocusMoveUp,
    Action::FocusMoveDown,
    Action::FocusAccept,
    Action::FocusDisable,
    Action::FocusPromote,
    Action::FocusRefresh,
    Action::FocusRange,
    Action::FocusGroup,
    Action::FocusClose,
];

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

/// A key as the map compares it: an upper-case letter is Shift with the
/// lower-case one, as terminals that report Shift that way mean (with Ctrl
/// or Alt since step 21; alone since step 25, so that the viewer's `G` is
/// `shift+g` whichever way a terminal sends it). Typing is not affected:
/// a key bound to nothing types what it is.
#[must_use]
pub fn normalize(key: Key) -> Key {
    match key.code {
        KeyCode::Char(c) if c.is_ascii_uppercase() && !key.mods.contains(Mods::SUPER) => Key::with(
            KeyCode::Char(c.to_ascii_lowercase()),
            key.mods | Mods::SHIFT,
        ),
        _ => key,
    }
}

/// The character a key types when nothing is bound to it: a character with
/// no modifier but Shift.
#[must_use]
pub fn typed(key: Key) -> Option<char> {
    let KeyCode::Char(c) = key.code else {
        return None;
    };
    if key.mods.contains(Mods::CTRL)
        || key.mods.contains(Mods::ALT)
        || key.mods.contains(Mods::SUPER)
    {
        return None;
    }
    if key.mods.contains(Mods::SHIFT) {
        return Some(c.to_uppercase().next().unwrap_or(c));
    }
    Some(c)
}

/// A problem with one entry of a binding table. The rest of the table is
/// used, and this entry's action keeps its default keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    /// The action's name as the table gave it.
    pub action: String,
    /// What is wrong.
    pub reason: String,
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[keys] {}: {}", self.action, self.reason)
    }
}

/// Keys to actions, by context, and the mouse's gestures in the contexts
/// that take them (since step 25).
#[derive(Debug, Clone)]
pub struct KeyMap {
    map: HashMap<(Context, Key), Action>,
    gestures: HashMap<(Context, Gesture), Action>,
}

impl Default for KeyMap {
    fn default() -> Self {
        Self::defaults()
    }
}

impl KeyMap {
    /// The default keys of every action.
    #[must_use]
    pub fn defaults() -> Self {
        // Parsed once, then copied: a map is made for every input, and the
        // table grew with the viewer's and the full screen's keys (step 25).
        static DEFAULTS: std::sync::OnceLock<KeyMap> = std::sync::OnceLock::new();
        DEFAULTS.get_or_init(Self::parse_defaults).clone()
    }

    /// The defaults, parsed from the table.
    fn parse_defaults() -> Self {
        let mut map = HashMap::new();
        let mut gestures = HashMap::new();
        for entry in ACTIONS {
            for text in entry.keys {
                match parse_binding(text) {
                    Ok(Binding::Key(key)) => {
                        map.insert((entry.context, normalize(key)), entry.action);
                    }
                    Ok(Binding::Gesture(gesture)) => {
                        gestures.insert((entry.context, gesture), entry.action);
                    }
                    // The table is ours; its own tests parse every key.
                    Err(_) => continue,
                }
            }
        }
        Self { map, gestures }
    }

    /// The defaults with `table` over them: each entry names an action
    /// (`editor.submit`) and the keys it takes instead of its defaults (an
    /// empty list unbinds it). A key given to an action is taken from any
    /// other action of the same context, and a key given to a global action
    /// from every context. An entry that cannot be used is
    /// reported and its action keeps its defaults.
    #[must_use]
    pub fn with_table<'a>(
        table: impl IntoIterator<Item = (&'a str, &'a [String])>,
    ) -> (Self, Vec<TableError>) {
        let mut keymap = Self::defaults();
        let mut errors = Vec::new();
        // Global actions last, so that a key given to one is taken from
        // every context whatever order the table lists them in.
        let mut table: Vec<(&str, &[String])> = table.into_iter().collect();
        table.sort_by_key(|(name, _)| name.starts_with("global."));
        for (name, keys) in table {
            let Some(action) = Action::from_name(name) else {
                errors.push(TableError {
                    action: name.to_owned(),
                    reason: "there is no such action".to_owned(),
                });
                continue;
            };
            let parsed: Result<Vec<Binding>, BindingError> =
                keys.iter().map(|text| parse_binding(text)).collect();
            let parsed = match parsed {
                Ok(parsed) => parsed,
                Err(error) => {
                    errors.push(TableError {
                        action: name.to_owned(),
                        reason: error.to_string(),
                    });
                    continue;
                }
            };
            let context = action.context();
            // A gesture where the mouse is not reported can never come.
            if let Some(Binding::Gesture(gesture)) = parsed
                .iter()
                .find(|binding| matches!(binding, Binding::Gesture(_)))
                .filter(|_| !context.takes_gestures())
            {
                errors.push(TableError {
                    action: name.to_owned(),
                    reason: format!(
                        "`{}` is the mouse's, which only the viewer's and the full screen's \
                         actions take",
                        gesture.name()
                    ),
                });
                continue;
            }
            let keys: Vec<Key> = parsed
                .iter()
                .filter_map(|binding| match binding {
                    Binding::Key(key) => Some(normalize(*key)),
                    Binding::Gesture(_) => None,
                })
                .collect();
            keymap.map.retain(|_, bound| *bound != action);
            keymap.gestures.retain(|_, bound| *bound != action);
            // A key bound to a global action is taken from every context:
            // `global.suspend = ["ctrl+z"]` must not stay `editor.undo`.
            if context == Context::Global {
                keymap.map.retain(|(_, key), _| !keys.contains(key));
            }
            for key in keys {
                keymap.map.insert((context, key), action);
            }
            for binding in parsed {
                if let Binding::Gesture(gesture) = binding {
                    keymap.gestures.insert((context, gesture), action);
                }
            }
        }
        (keymap, errors)
    }

    /// The action `key` stands for, looked up in `contexts` in order, then
    /// in [`Context::Global`].
    #[must_use]
    pub fn lookup(&self, contexts: &[Context], key: Key) -> Option<Action> {
        let key = normalize(key);
        contexts
            .iter()
            .chain(std::iter::once(&Context::Global))
            .find_map(|context| self.map.get(&(*context, key)).copied())
    }

    /// The action a mouse gesture stands for, looked up in `contexts` in
    /// order (since step 25). No global action takes a gesture.
    #[must_use]
    pub fn lookup_gesture(&self, contexts: &[Context], gesture: Gesture) -> Option<Action> {
        contexts
            .iter()
            .find_map(|context| self.gestures.get(&(*context, gesture)).copied())
    }

    /// The gestures bound to `action`, in a stable order.
    #[must_use]
    pub fn gestures_of(&self, action: Action) -> Vec<Gesture> {
        let mut gestures: Vec<Gesture> = self
            .gestures
            .iter()
            .filter(|(_, bound)| **bound == action)
            .map(|((_, gesture), _)| *gesture)
            .collect();
        gestures.sort();
        gestures
    }

    /// The keys bound to `action`, in a stable order: for help and hints.
    #[must_use]
    pub fn keys_of(&self, action: Action) -> Vec<Key> {
        let mut keys: Vec<Key> = self
            .map
            .iter()
            .filter(|(_, bound)| **bound == action)
            .map(|((_, key), _)| *key)
            .collect();
        keys.sort_by_key(|key| name(*key));
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_key_parses_and_no_context_binds_a_key_twice() {
        let mut seen: HashMap<(Context, Binding), Action> = HashMap::new();
        for entry in ACTIONS {
            for text in entry.keys {
                let binding = match parse_binding(text).unwrap_or_else(|e| panic!("{e}")) {
                    Binding::Key(key) => Binding::Key(normalize(key)),
                    gesture @ Binding::Gesture(_) => {
                        assert!(entry.context.takes_gestures(), "{text} in {}", entry.action);
                        gesture
                    }
                };
                if let Some(other) = seen.insert((entry.context, binding), entry.action) {
                    panic!("{text} is bound to both {other} and {}", entry.action);
                }
            }
        }
    }

    /// The viewer's keys are as step 23 had them, now in the map, with the
    /// mouse's gestures beside them (step 25); the full screen's are looked
    /// up in their own context.
    #[test]
    fn the_viewer_and_the_full_screen_have_their_keys_and_the_mouse() {
        let map = KeyMap::defaults();
        let viewer = [Context::Viewer];
        for (key, action) in [
            (Key::plain(KeyCode::Up), Action::ViewerUp),
            (Key::plain(KeyCode::Char('k')), Action::ViewerUp),
            (Key::plain(KeyCode::Char('j')), Action::ViewerDown),
            (Key::plain(KeyCode::Enter), Action::ViewerToggle),
            (Key::plain(KeyCode::Char('g')), Action::ViewerTop),
            // `G`, however the terminal sends it.
            (Key::plain(KeyCode::Char('G')), Action::ViewerBottom),
            (
                Key::with(KeyCode::Char('g'), Mods::SHIFT),
                Action::ViewerBottom,
            ),
            (Key::plain(KeyCode::Char('/')), Action::ViewerSearch),
            (Key::plain(KeyCode::Char('q')), Action::ViewerClose),
            (
                Key::with(KeyCode::Char('o'), Mods::CTRL),
                Action::ViewerClose,
            ),
        ] {
            assert_eq!(map.lookup(&viewer, key), Some(action), "{key:?}");
        }
        assert_eq!(
            map.lookup_gesture(&viewer, Gesture::Click),
            Some(Action::ViewerToggle)
        );
        assert_eq!(
            map.lookup_gesture(&viewer, Gesture::WheelDown),
            Some(Action::ViewerScrollDown)
        );
        let screen = [Context::Editor, Context::Screen];
        assert_eq!(
            map.lookup(&screen, Key::plain(KeyCode::PageUp)),
            Some(Action::ScreenPageUp)
        );
        assert_eq!(
            map.lookup(&screen, Key::with(KeyCode::Char('f'), Mods::CTRL)),
            Some(Action::ScreenSearch)
        );
        // The editor's own keys come first: `Home` is the line's start.
        assert_eq!(
            map.lookup(&screen, Key::plain(KeyCode::Home)),
            Some(Action::LineStart)
        );
        assert_eq!(map.lookup_gesture(&[Context::Editor], Gesture::Click), None);
        // An upper-case letter alone is still typed where nothing binds it.
        assert_eq!(
            map.lookup(&[Context::Editor], Key::plain(KeyCode::Char('G'))),
            None
        );
    }

    /// A gesture can be bound in the viewer's and the full screen's
    /// contexts, and is refused anywhere else, where the mouse is never
    /// reported.
    #[test]
    fn a_gesture_is_bound_only_where_the_mouse_is_reported() {
        let wheel = vec!["wheel_up".to_owned(), "k".to_owned()];
        let click = vec!["click".to_owned()];
        let (map, errors) = KeyMap::with_table([
            ("viewer.up", wheel.as_slice()),
            ("editor.submit", click.as_slice()),
        ]);
        assert_eq!(
            map.lookup_gesture(&[Context::Viewer], Gesture::WheelUp),
            Some(Action::ViewerUp)
        );
        // The wheel up was the scroll's: bound to `viewer.up`, it is that
        // action's, and the scroll keeps no gesture.
        assert_eq!(map.gestures_of(Action::ViewerUp), vec![Gesture::WheelUp]);
        assert!(map.gestures_of(Action::ViewerScrollUp).is_empty());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].reason.contains("click"), "{errors:?}");
        assert_eq!(
            map.lookup(&[Context::Editor], Key::plain(KeyCode::Enter)),
            Some(Action::Submit)
        );
    }

    #[test]
    fn names_are_unique_and_read_back() {
        let mut names = std::collections::HashSet::new();
        for entry in ACTIONS {
            let name = entry.action.name();
            assert!(names.insert(name.clone()), "{name} twice");
            assert_eq!(Action::from_name(&name), Some(entry.action));
        }
    }

    #[test]
    fn a_menu_is_looked_up_before_the_editor_under_it() {
        let map = KeyMap::defaults();
        let enter = Key::plain(KeyCode::Enter);
        assert_eq!(map.lookup(&[Context::Editor], enter), Some(Action::Submit));
        assert_eq!(
            map.lookup(&[Context::Menu, Context::Editor], enter),
            Some(Action::MenuAccept)
        );
        let tab = Key::plain(KeyCode::Tab);
        assert_eq!(
            map.lookup(&[Context::Toggle, Context::Picker, Context::Menu], tab),
            Some(Action::ToggleFast)
        );
        assert_eq!(
            map.lookup(&[Context::Picker, Context::Menu], tab),
            Some(Action::MenuAccept)
        );
        let ctrl_c = Key::with(KeyCode::Char('c'), Mods::CTRL);
        assert_eq!(
            map.lookup(&[Context::Search], ctrl_c),
            Some(Action::Interrupt)
        );
    }

    #[test]
    fn a_table_rebinds_unbinds_and_reports_what_it_cannot_use() {
        let submit = vec!["ctrl+s".to_owned()];
        let none: Vec<String> = Vec::new();
        let bad = vec!["hyper+x".to_owned()];
        let (map, errors) = KeyMap::with_table([
            ("editor.submit", submit.as_slice()),
            ("editor.yank", none.as_slice()),
            ("editor.nonsense", none.as_slice()),
            ("editor.undo", bad.as_slice()),
        ]);
        let editor = [Context::Editor];
        assert_eq!(
            map.lookup(&editor, Key::with(KeyCode::Char('s'), Mods::CTRL)),
            Some(Action::Submit)
        );
        assert_eq!(map.lookup(&editor, Key::plain(KeyCode::Enter)), None);
        assert_eq!(
            map.lookup(&editor, Key::with(KeyCode::Char('y'), Mods::CTRL)),
            None
        );
        assert_eq!(
            map.lookup(&editor, Key::with(KeyCode::Char('z'), Mods::CTRL)),
            Some(Action::Undo)
        );
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn a_key_bound_to_a_global_action_is_taken_from_every_context() {
        let keys = vec!["ctrl+z".to_owned()];
        let (map, errors) = KeyMap::with_table([("global.suspend", keys.as_slice())]);
        assert!(errors.is_empty());
        // Listed before an editor action that names the same key: the
        // global one still has it.
        let (later, _) = KeyMap::with_table([
            ("global.suspend", keys.as_slice()),
            ("editor.undo", keys.as_slice()),
        ]);
        assert_eq!(
            later.lookup(
                &[Context::Editor],
                Key::with(KeyCode::Char('z'), Mods::CTRL)
            ),
            Some(Action::Suspend)
        );
        let ctrl_z = Key::with(KeyCode::Char('z'), Mods::CTRL);
        assert_eq!(
            map.lookup(&[Context::Editor], ctrl_z),
            Some(Action::Suspend)
        );
        let underscore = Key::with(KeyCode::Char('_'), Mods::CTRL);
        assert_eq!(
            map.lookup(&[Context::Editor], underscore),
            Some(Action::Undo)
        );
    }

    #[test]
    fn an_upper_case_letter_with_ctrl_is_shift_with_it() {
        let map = KeyMap::defaults();
        let reported = Key::with(KeyCode::Char('Z'), Mods::CTRL | Mods::SHIFT);
        assert_eq!(map.lookup(&[Context::Editor], reported), Some(Action::Redo));
    }
}
