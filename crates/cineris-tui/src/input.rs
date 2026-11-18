//! The input: the editor with its menus, history, search and focus, driven
//! by terminal events (`docs/architecture/09-tui.md`, "Input").
//!
//! [`Input::handle`] takes an [`Event`]; a key goes through the
//! [`KeyMap`] in the contexts of what has the keyboard, and the action it
//! finds is done by [`Input::act`], which slash commands call too. What the
//! input cannot do itself (send a prompt, interrupt the turn, quit, open the
//! viewer) it hands back as an [`Outcome`] for the caller.
//!
//! **What the caller supplies**, each behind a small interface, because the
//! interface never touches files and holds no agent logic:
//!
//! | What | How |
//! |---|---|
//! | The workspace's paths, listed with the ignore rules | [`Input::set_files`] |
//! | The slash commands | [`Input::set_commands`] |
//! | Earlier prompts for this project | the [`HistoryStore`] given to [`Input::new`]; each prompt sent is pushed to it |
//! | The key bindings from `[keys]` | the [`KeyMap`] given to [`Input::new`] ([`KeyMap::with_table`]), and a new one from a reload ([`Input::set_keymap`]) |
//! | Whether a turn is running | [`Input::set_running`] |
//! | The live tool boxes and the panels | [`Input::focus_mut`] and [`Ring::update`](crate::focus::Ring::update) |
//! | The width of the editor's text | [`Input::set_width`] |
//! | An item to reword in the prompt's own editor (a plan item, a project entry) | [`Input::begin_item_edit`] |
//!
//! **Since step 24,** a key that types a character and is not taken while a
//! box or a panel has the focus goes to the editor, and the focus goes back
//! to the input with it ([`Input::pass_to_editor`]); a panel's own keys
//! ([`crate::keys::PANEL_KEYS`]) come back as [`Outcome::Panel`] while a
//! panel has the focus, for the application to take or pass on. **An item
//! edit** sets the draft aside and edits the item's text in the editor:
//! `Enter` gives [`Outcome::ItemEdited`] (never a prompt, and nothing is
//! pushed to the history) and `Esc` [`Outcome::ItemEditCancelled`]; either
//! way the draft comes back as it was.

use cineris_term::{Event, Key};

use crate::complete::{self, Candidates, Command, Empty, Filter, Kind, Menu, Row, Trigger};
use crate::editor::{Editor, Fragment};
use crate::focus::{Ring, Target};
use crate::history::{HistoryStore, Search, Steps};
use crate::keys::{self, Action, Context, KeyMap};
use crate::paste;

/// The most rows the input box grows to; past them it scrolls.
pub const MAX_ROWS: usize = 8;

/// How a prompt sent while a turn runs is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendMode {
    /// To the running turn, at its next safe boundary.
    #[default]
    Steer,
    /// After the running turn ends, as the next prompt.
    Queue,
}

/// What a prompt is, when sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// A new turn: nothing was running.
    Prompt,
    /// Into the running turn.
    Steer,
    /// After the running turn.
    Queue,
}

/// What the caller is to do after an event.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// Nothing changed.
    Nothing,
    /// The input changed: draw it again.
    Changed,
    /// Send this prompt, chips expanded. It has been pushed to history.
    Submit {
        /// The prompt.
        text: String,
        /// How.
        delivery: Delivery,
    },
    /// Interrupt the running turn.
    Interrupt,
    /// `Ctrl+C` was pressed with nothing running and nothing typed: a
    /// second press quits. The hint row says so.
    QuitArmed,
    /// Quit.
    Quit,
    /// Redraw the live region.
    Redraw,
    /// Open the viewer at the latest tool box.
    OpenViewer,
    /// Open or close the running round's box.
    ToggleRound,
    /// Stop the program as a shell's job.
    Suspend,
    /// Run the external editor on [`Input::external_text`], then give the
    /// result to [`Input::external_result`].
    ExternalEditor,
    /// The focus moved to this target.
    Focused(Target),
    /// `Enter` on a focused box or panel.
    Activate(Target),
    /// `Up` (`false`) or `Down` (`true`) in a focused panel.
    Move(Target, bool),
    /// A picker's row was accepted: the picker's name, and the row's index
    /// among the rows it was opened with.
    Picked(String, usize),
    /// A picker was closed with nothing accepted.
    PickerClosed(String),
    /// A file of the `@` menu was put in the prompt: its path, relative to
    /// the workspace, for the caller to read ahead (step 24).
    FileChosen(String),
    /// A picker's toggle changed: its name, and the new state.
    Toggled(String, bool),
    /// A picker's selected row is to be removed: the picker's name, and
    /// the row's index among the rows it was opened with. The picker stays
    /// open; the caller opens it again with the rows that are left.
    Removed(String, usize),
    /// An action that has no meaning where the keyboard is now.
    Ignored(Action),
    /// One of a panel's own keys ([`crate::keys::PANEL_KEYS`]) while that
    /// panel has the focus (since step 24). The caller decides whether the
    /// panel takes it; one it does not take that types a character goes to
    /// the editor through [`Input::pass_to_editor`].
    Panel(Target, Action),
    /// `Enter` in an item edit ([`Input::begin_item_edit`]): the item's new
    /// text. Nothing was pushed to the history, and the draft set aside is
    /// back in the editor.
    ItemEdited(String),
    /// `Esc` in an item edit: nothing is saved, and the draft set aside is
    /// back in the editor.
    ItemEditCancelled,
}

/// An item being edited in the prompt's own editor (since step 24): the
/// draft set aside, whole, and what `Enter` does.
#[derive(Debug)]
struct ItemEdit {
    /// The prompt being written, with its cursor, its chips and its undo.
    draft: Editor,
    /// The first row of the draft in view.
    scroll: usize,
    /// What `Enter` does, as the hint row says it: `saves the item`.
    hint: String,
}

/// The input's state. See the [module documentation](self).
#[derive(Debug)]
pub struct Input {
    keymap: KeyMap,
    editor: Editor,
    width: usize,
    /// The first row of the editor in view.
    scroll: usize,
    history: Box<dyn HistoryStore + Send>,
    steps: Steps,
    search: Option<Search>,
    files: Option<Candidates>,
    file_filter: Filter,
    /// The `@` filtering is done elsewhere, off the key's path (since step
    /// 23): what was last asked, and the query not yet handed out.
    elsewhere: Option<Elsewhere>,
    commands: Vec<Command>,
    command_candidates: Candidates,
    command_filter: Filter,
    menu: Option<Menu>,
    /// The `@` or `/` whose menu `Esc` closed: it stays closed until the
    /// text asks again somewhere else.
    dismissed: Option<Trigger>,
    /// Where the `@` is whose menu the workspace's files could not be
    /// listed for: it says so, and asks for them no more, until the text
    /// has no `@` menu there.
    unlisted: Option<usize>,
    focus: Ring,
    running: bool,
    send_mode: SendMode,
    /// `Ctrl+C` was pressed once, idle and empty.
    quit_armed: bool,
    /// An item being edited in the editor, and the draft set aside for it.
    item_edit: Option<ItemEdit>,
}

impl Input {
    /// An input with `keymap`, over `history`.
    #[must_use]
    pub fn new(keymap: KeyMap, history: Box<dyn HistoryStore + Send>) -> Self {
        Self {
            keymap,
            editor: Editor::new(),
            width: 80,
            scroll: 0,
            history,
            steps: Steps::default(),
            search: None,
            files: None,
            file_filter: Filter::new(),
            elsewhere: None,
            commands: Vec::new(),
            command_candidates: Candidates::default(),
            command_filter: Filter::new(),
            menu: None,
            dismissed: None,
            unlisted: None,
            focus: Ring::new(),
            running: false,
            send_mode: SendMode::Steer,
            quit_armed: false,
            item_edit: None,
        }
    }

    // ------------------------------------------------------------- reading

    /// The editor.
    #[must_use]
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// The open menu or picker, if any.
    #[must_use]
    pub fn menu(&self) -> Option<&Menu> {
        self.menu.as_ref()
    }

    /// The history search, while one is open.
    #[must_use]
    pub fn search(&self) -> Option<&Search> {
        self.search.as_ref()
    }

    /// The prompt the history search has found, if any.
    #[must_use]
    pub fn search_found(&self) -> Option<&Fragment> {
        let found = self.search.as_ref()?.found()?;
        self.history.get(found)
    }

    /// The focus ring.
    #[must_use]
    pub fn focus(&self) -> &Ring {
        &self.focus
    }

    /// The focus ring, for the caller to say what boxes and panels there are.
    pub fn focus_mut(&mut self) -> &mut Ring {
        &mut self.focus
    }

    /// The key map.
    #[must_use]
    pub fn keymap(&self) -> &KeyMap {
        &self.keymap
    }

    /// Whether an item is being edited ([`begin_item_edit`]), and then the
    /// keys the hint row shows for it: `enter saves the item`, `esc
    /// cancels`, each key as the map names it.
    ///
    /// [`begin_item_edit`]: Self::begin_item_edit
    #[must_use]
    pub fn item_edit(&self) -> Option<Vec<String>> {
        let edit = self.item_edit.as_ref()?;
        let named = |action: Action, fallback: &str| {
            self.keymap
                .keys_of(action)
                .into_iter()
                .map(keys::name)
                .find(|text| text == fallback)
                .or_else(|| {
                    self.keymap
                        .keys_of(action)
                        .into_iter()
                        .next()
                        .map(keys::name)
                })
                .unwrap_or_else(|| fallback.to_owned())
        };
        Some(vec![
            format!("{} {}", named(Action::Submit, "enter"), edit.hint),
            format!("{} cancels", named(Action::Stop, "esc")),
        ])
    }

    /// Whether a turn is running.
    #[must_use]
    pub fn running(&self) -> bool {
        self.running
    }

    /// How a prompt sent now, while a turn runs, would be sent.
    #[must_use]
    pub fn send_mode(&self) -> SendMode {
        self.send_mode
    }

    /// Whether a second `Ctrl+C` quits.
    #[must_use]
    pub fn quit_armed(&self) -> bool {
        self.quit_armed
    }

    /// The width the editor's text wraps at.
    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    /// The first row of the editor's text in view.
    #[must_use]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    /// The contexts a key is looked up in now, in order.
    #[must_use]
    pub fn contexts(&self) -> Vec<Context> {
        if !self.focus.on_input() {
            return vec![Context::Focus];
        }
        if self.search.is_some() {
            return vec![Context::Search];
        }
        match &self.menu {
            Some(menu) if matches!(menu.kind, Kind::Picker(_)) => {
                if menu.toggle.is_some() {
                    vec![Context::Toggle, Context::Picker, Context::Menu]
                } else {
                    vec![Context::Picker, Context::Menu]
                }
            }
            Some(_) => vec![Context::Menu, Context::Editor],
            None => vec![Context::Editor],
        }
    }

    // ------------------------------------------------------------- setting

    /// The width the editor's text wraps at, in columns: the input box's
    /// inside, less the `◆` and the cursor's column.
    pub fn set_width(&mut self, width: usize) {
        self.width = width.max(1);
        self.keep_cursor_in_view();
    }

    /// Replaces the key map (since step 24: a reload of `[keys]`). A key
    /// already taken was taken with the old map; the next is looked up in
    /// this one. Whoever has a dialog or a panel up starts its readiness
    /// again: what the person saw offered was the old map's.
    pub fn set_keymap(&mut self, keymap: KeyMap) {
        self.keymap = keymap;
    }

    /// Edits `text` in the prompt's own editor (since step 24): a plan
    /// item being reworded, or added from nothing, a project entry. What is
    /// being written is set aside whole (its text, chips, cursor and undo),
    /// the editor holds `text` with the cursor at its end, and the input
    /// takes the focus. Until it ends, `Enter` gives
    /// [`Outcome::ItemEdited`] and `Esc` [`Outcome::ItemEditCancelled`],
    /// and either brings the draft back; the hint row says so
    /// ([`item_edit`](Self::item_edit)), `hint` being what `Enter` does
    /// (`saves the item`). Every other rule of the editor holds, but an item
    /// is one line, with no completion menu and no history. An item edit
    /// already under way is ended first, as if cancelled.
    pub fn begin_item_edit(&mut self, text: &str, hint: &str) {
        self.cancel_item_edit();
        let draft = std::mem::take(&mut self.editor);
        let scroll = std::mem::take(&mut self.scroll);
        self.editor.type_str(text);
        self.item_edit = Some(ItemEdit {
            draft,
            scroll,
            hint: hint.to_owned(),
        });
        self.search = None;
        self.menu = None;
        self.dismissed = None;
        self.quit_armed = false;
        self.focus.to_input();
        self.after_edit();
    }

    /// Ends an item edit with nothing saved, the draft back in the editor.
    /// Whether one was under way. For the caller, when what the item was
    /// of is gone (another session is open, the panel closed).
    pub fn cancel_item_edit(&mut self) -> bool {
        let Some(edit) = self.item_edit.take() else {
            return false;
        };
        self.editor = edit.draft;
        self.scroll = edit.scroll;
        self.after_edit();
        true
    }

    /// Whether a turn is running. When it ends, the send mode goes back to
    /// steering.
    pub fn set_running(&mut self, running: bool) {
        if !running {
            self.send_mode = SendMode::Steer;
        }
        self.running = running;
    }

    /// The workspace's paths, for `@` completion, in the order to show
    /// them before anything is typed.
    pub fn set_files(&mut self, paths: Vec<String>) {
        self.set_file_candidates(Candidates::new(paths));
    }

    /// As [`set_files`](Self::set_files), with the paths prepared already.
    pub fn set_file_candidates(&mut self, files: Candidates) {
        self.files = Some(files);
        self.file_filter.reset();
        self.refresh_menu();
    }

    /// Whether the text asks for the workspace's paths: an `@` menu is open
    /// and nothing has been given to fill it, by
    /// [`set_files`](Self::set_files) or
    /// [`filter_files_elsewhere`](Self::filter_files_elsewhere). The
    /// application lists them only then, on the first `@` typed.
    #[must_use]
    pub fn wants_files(&self) -> bool {
        self.files.is_none()
            && self.elsewhere.is_none()
            && self.menu.as_ref().is_some_and(|menu| match menu.kind {
                Kind::Files { start } => self.unlisted != Some(start),
                _ => false,
            })
    }

    /// The workspace's files could not be listed: the open `@` menu says
    /// so, and [`wants_files`](Self::wants_files) is false until the text
    /// has no `@` menu there; a later one asks again.
    pub fn files_unlisted(&mut self) {
        if let Some(menu) = self.menu.as_mut()
            && let Kind::Files { start } = menu.kind
        {
            self.unlisted = Some(start);
            if menu.rows.is_empty() {
                menu.empty = Empty::Unlisted;
            }
        }
    }

    /// Hands the `@` filtering out (since step 23): each query the text asks
    /// is given by [`take_file_query`](Self::take_file_query), with a
    /// generation, and the rows for it come back by
    /// [`file_rows`](Self::file_rows). Until they do, the menu keeps the rows
    /// it had. The paths [`set_files`](Self::set_files) gave are not used.
    pub fn filter_files_elsewhere(&mut self) {
        self.elsewhere = Some(Elsewhere::default());
        self.refresh_menu();
    }

    /// The `@` query to filter for, once: `None` when the text has asked
    /// nothing new since the last call.
    pub fn take_file_query(&mut self) -> Option<FileQuery> {
        self.elsewhere.as_mut()?.pending.take()
    }

    /// The rows for the query of `generation`. Rows for any query but the
    /// latest are dropped: an older answer never replaces a newer one.
    pub fn file_rows(&mut self, generation: u64, rows: Vec<Row>) {
        let Some(elsewhere) = &self.elsewhere else {
            return;
        };
        if generation != elsewhere.generation {
            return;
        }
        if let Some(menu) = self
            .menu
            .as_mut()
            .filter(|menu| matches!(menu.kind, Kind::Files { .. }))
        {
            menu.refill(rows, Empty::NoMatch);
        }
    }

    /// The slash commands, for `/` completion.
    pub fn set_commands(&mut self, commands: Vec<Command>) {
        self.command_candidates = Candidates::new(commands.iter().map(|c| c.name.clone()));
        self.commands = commands;
        self.command_filter.reset();
        self.refresh_menu();
    }

    /// Opens a picker over the input.
    pub fn open_picker(&mut self, picker: Menu) {
        self.search = None;
        self.menu = Some(picker);
    }

    /// The text to give the external editor: the prompt, with its chips.
    #[must_use]
    pub fn external_text(&self) -> Fragment {
        self.editor.fragment()
    }

    /// What the external editor gave back: the prompt is replaced with it,
    /// as one step of undo. `None` leaves the prompt as it was.
    pub fn external_result(&mut self, edited: Option<Fragment>) {
        if let Some(fragment) = edited
            && fragment != self.editor.fragment()
        {
            self.editor.set(fragment);
        }
        self.after_edit();
    }

    // ------------------------------------------------------------ handling

    /// Takes one event.
    pub fn handle(&mut self, event: &Event) -> Outcome {
        match event {
            // A repeat types as the key does.
            Event::Key(key) | Event::Repeat(key) => self.key(*key),
            Event::Paste(text) => self.paste(text),
            _ => Outcome::Nothing,
        }
    }

    fn key(&mut self, key: Key) -> Outcome {
        let contexts = self.contexts();
        if let Some(action) = self.keymap.lookup(&contexts, key) {
            let outcome = self.act(action);
            // A panel's own key that a focused box has no use for is a key
            // the box does not take: one that types goes to the editor.
            if matches!(outcome, Outcome::Ignored(ignored) if ignored.is_panel_key())
                && !self.focus.on_input()
            {
                return self.pass_to_editor(key);
            }
            return outcome;
        }
        self.quit_armed = false;
        if !self.focus.on_input() {
            return self.pass_to_editor(key);
        }
        let Some(c) = keys::typed(key) else {
            return Outcome::Nothing;
        };
        self.type_char(c)
    }

    /// A key that a focused box or panel did not take (since step 24): one
    /// that types a character goes to the editor, and the focus goes back
    /// to the input with it, so that no keystroke is lost and none is taken
    /// by surprise; any other does nothing. [`Outcome::Focused`] when the
    /// focus moved.
    pub fn pass_to_editor(&mut self, key: Key) -> Outcome {
        let Some(c) = keys::typed(key) else {
            return Outcome::Nothing;
        };
        self.quit_armed = false;
        if self.focus.on_input() {
            return self.type_char(c);
        }
        self.focus.to_input();
        self.type_char(c);
        Outcome::Focused(Target::Input)
    }

    /// `c` typed where the input has the keyboard: the history search, a
    /// picker's filter, or the editor.
    fn type_char(&mut self, c: char) -> Outcome {
        if let Some(search) = &mut self.search {
            search.push(self.history.as_ref(), c);
            return Outcome::Changed;
        }
        if let Some(menu) = self
            .menu
            .as_mut()
            .filter(|m| matches!(m.kind, Kind::Picker(_)))
        {
            menu.filter.push(c);
            menu.refilter();
            return Outcome::Changed;
        }
        self.editor.type_char(c);
        self.after_edit();
        Outcome::Changed
    }

    /// A paste: text, never keys. With a box or a panel focused, the input
    /// takes the focus back and the paste with it.
    fn paste(&mut self, text: &str) -> Outcome {
        self.quit_armed = false;
        let moved = !self.focus.on_input();
        if moved {
            self.focus.to_input();
        }
        if let Some(search) = &mut self.search {
            for c in text.chars().map(|c| if c.is_control() { ' ' } else { c }) {
                search.push(self.history.as_ref(), c);
            }
            return Outcome::Changed;
        }
        if let Some(menu) = self
            .menu
            .as_mut()
            .filter(|m| matches!(m.kind, Kind::Picker(_)))
        {
            menu.filter.extend(text.chars().filter(|c| !c.is_control()));
            menu.refilter();
            return Outcome::Changed;
        }
        self.editor.insert(paste::fragment(text));
        self.after_edit();
        if moved {
            Outcome::Focused(Target::Input)
        } else {
            Outcome::Changed
        }
    }

    /// Does `action`: what a key bound to it does, or a slash command that
    /// names it.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm an action: the list of what each does, kept together"
    )]
    pub fn act(&mut self, action: Action) -> Outcome {
        if action != Action::Interrupt {
            self.quit_armed = false;
        }
        let width = self.width;
        let edited = |input: &mut Self, changed: bool| {
            if changed {
                input.after_edit();
                Outcome::Changed
            } else {
                Outcome::Nothing
            }
        };
        match action {
            // ---------------------------------------------------- global
            Action::Interrupt => self.interrupt(),
            Action::Redraw => Outcome::Redraw,
            Action::OpenViewer => Outcome::OpenViewer,
            Action::ToggleRound => Outcome::ToggleRound,
            Action::Suspend => Outcome::Suspend,
            Action::FocusNext => {
                self.search = None;
                Outcome::Focused(self.focus.advance())
            }

            // ---------------------------------------------------- editor
            Action::Submit if self.item_edit.is_some() => {
                let text = self.editor.expanded();
                self.cancel_item_edit();
                Outcome::ItemEdited(text)
            }
            Action::Submit => self.submit(),
            // An item is one line.
            Action::NewLine if self.item_edit.is_some() => Outcome::Ignored(action),
            Action::NewLine => {
                self.editor.new_line();
                edited(self, true)
            }
            Action::Stop if self.item_edit.is_some() => {
                self.cancel_item_edit();
                Outcome::ItemEditCancelled
            }
            Action::Stop => {
                if self.running {
                    Outcome::Interrupt
                } else {
                    Outcome::Nothing
                }
            }
            Action::Quit => {
                if self.editor.is_empty() {
                    Outcome::Quit
                } else {
                    Outcome::Nothing
                }
            }
            Action::Left => {
                let moved = self.editor.left();
                edited(self, moved)
            }
            Action::Right => {
                let moved = self.editor.right();
                edited(self, moved)
            }
            Action::WordLeft => {
                let moved = self.editor.word_left();
                edited(self, moved)
            }
            Action::WordRight => {
                let moved = self.editor.word_right();
                edited(self, moved)
            }
            Action::LineStart => {
                let moved = self.editor.line_start();
                edited(self, moved)
            }
            Action::LineEnd => {
                let moved = self.editor.line_end();
                edited(self, moved)
            }
            Action::TextStart => {
                let moved = self.editor.text_start();
                edited(self, moved)
            }
            Action::TextEnd => {
                let moved = self.editor.text_end();
                edited(self, moved)
            }
            Action::Up => {
                if self.editor.up(width) {
                    return edited(self, true);
                }
                // An item being edited is not stepped away from.
                if self.item_edit.is_some() {
                    return Outcome::Nothing;
                }
                match self
                    .steps
                    .older(self.history.as_ref(), self.editor.fragment())
                {
                    Some(fragment) => {
                        self.editor.set(fragment);
                        self.editor.set_cursor(0);
                        self.dismiss_menu();
                        edited(self, true)
                    }
                    None => Outcome::Nothing,
                }
            }
            Action::Down => {
                if self.editor.down(width) {
                    return edited(self, true);
                }
                if self.item_edit.is_some() {
                    return Outcome::Nothing;
                }
                match self
                    .steps
                    .newer(self.history.as_ref(), self.editor.fragment())
                {
                    Some(fragment) => {
                        self.editor.set(fragment);
                        self.dismiss_menu();
                        edited(self, true)
                    }
                    None => Outcome::Nothing,
                }
            }
            Action::Backspace => {
                let changed = self.editor.backspace();
                edited(self, changed)
            }
            Action::Delete => {
                let changed = self.editor.delete();
                edited(self, changed)
            }
            Action::KillWordBack => {
                let changed = self.editor.kill_word_back();
                edited(self, changed)
            }
            Action::KillWordForward => {
                let changed = self.editor.kill_word_forward();
                edited(self, changed)
            }
            Action::KillToLineEnd => {
                let changed = self.editor.kill_to_line_end();
                edited(self, changed)
            }
            Action::KillToLineStart => {
                let changed = self.editor.kill_to_line_start();
                edited(self, changed)
            }
            Action::Yank => {
                let changed = self.editor.yank();
                edited(self, changed)
            }
            Action::Undo => {
                let changed = self.editor.undo();
                edited(self, changed)
            }
            Action::Redo => {
                let changed = self.editor.redo();
                edited(self, changed)
            }
            Action::ExpandChip => {
                let changed = self.editor.expand_chip();
                edited(self, changed)
            }
            Action::HistorySearch if self.item_edit.is_some() => Outcome::Ignored(action),
            Action::HistorySearch => {
                self.menu = None;
                self.search = Some(Search::new());
                Outcome::Changed
            }
            Action::ExternalEditor => Outcome::ExternalEditor,
            Action::SwitchSendMode => {
                if !self.running {
                    return Outcome::Ignored(action);
                }
                self.send_mode = match self.send_mode {
                    SendMode::Steer => SendMode::Queue,
                    SendMode::Queue => SendMode::Steer,
                };
                Outcome::Changed
            }

            // ------------------------------------------------------ menu
            Action::MenuUp => self.with_menu(action, Menu::up),
            Action::MenuDown => self.with_menu(action, Menu::down),
            Action::MenuAccept => self.accept(),
            Action::MenuClose => self.close_menu(),

            // ---------------------------------------------------- picker
            Action::PickerErase => match self.menu.as_mut() {
                Some(menu) if matches!(menu.kind, Kind::Picker(_)) => {
                    menu.filter.pop();
                    menu.refilter();
                    Outcome::Changed
                }
                _ => Outcome::Ignored(action),
            },
            Action::PickerRemove => match self.menu.as_ref() {
                Some(menu) => match (&menu.kind, menu.picked()) {
                    (Kind::Picker(name), Some(index)) => Outcome::Removed(name.clone(), index),
                    _ => Outcome::Ignored(action),
                },
                None => Outcome::Ignored(action),
            },
            Action::ToggleFast => match self.menu.as_mut() {
                Some(Menu {
                    kind: Kind::Picker(name),
                    toggle: Some(toggle),
                    ..
                }) => {
                    toggle.on = !toggle.on;
                    Outcome::Toggled(name.clone(), toggle.on)
                }
                _ => Outcome::Ignored(action),
            },

            // ---------------------------------------------------- search
            Action::SearchOlder => self.with_search(action, |search, store| search.older(store)),
            Action::SearchNewer => self.with_search(action, |search, store| search.newer(store)),
            Action::SearchErase => self.with_search(action, |search, store| search.pop(store)),
            Action::SearchAccept => {
                let Some(search) = self.search.take() else {
                    return Outcome::Ignored(action);
                };
                if let Some(entry) = search.found().and_then(|found| self.history.get(found)) {
                    // As it was written: a chip comes back a chip.
                    let fragment = entry.clone();
                    self.editor.set(fragment);
                    self.after_edit();
                }
                Outcome::Changed
            }
            Action::SearchCancel => {
                if self.search.take().is_some() {
                    Outcome::Changed
                } else {
                    Outcome::Ignored(action)
                }
            }

            // ----------------------------------------------------- focus
            Action::FocusActivate => self.on_focus(action, Outcome::Activate),
            Action::FocusReturn => {
                if self.focus.on_input() {
                    return Outcome::Ignored(action);
                }
                self.focus.to_input();
                Outcome::Focused(Target::Input)
            }
            Action::FocusUp => self.on_focus(action, |target| Outcome::Move(target, false)),
            Action::FocusDown => self.on_focus(action, |target| Outcome::Move(target, true)),
            // A panel's own keys are the focused panel's to take or pass on;
            // a box has no use for them.
            Action::FocusSkip
            | Action::FocusEdit
            | Action::FocusAdd
            | Action::FocusDelete
            | Action::FocusMoveUp
            | Action::FocusMoveDown
            | Action::FocusAccept
            | Action::FocusDisable
            | Action::FocusPromote
            | Action::FocusRefresh
            | Action::FocusRange
            | Action::FocusGroup
            | Action::FocusClose => match self.focus.focused() {
                target @ Target::Panel(_) => Outcome::Panel(target, action),
                Target::Input | Target::ToolBox(_) => Outcome::Ignored(action),
            },

            // ---------------------------------------------------- dialog
            // A dialog's keys are the dialog's: the input never has them.
            Action::AllowOnce
            | Action::AllowSession
            | Action::AllowProject
            | Action::Deny
            | Action::DenyWithNote
            | Action::Never
            | Action::ShowWhole
            | Action::DialogTranscript => Outcome::Ignored(action),

            // ---------------------------------------- viewer and screen
            // The viewer's and the full screen's keys are the
            // application's (step 25): the input never has them.
            Action::ViewerUp
            | Action::ViewerDown
            | Action::ViewerToggle
            | Action::ViewerPageUp
            | Action::ViewerPageDown
            | Action::ViewerTop
            | Action::ViewerBottom
            | Action::ViewerSearch
            | Action::ViewerNext
            | Action::ViewerClose
            | Action::ViewerScrollUp
            | Action::ViewerScrollDown
            | Action::ScreenPageUp
            | Action::ScreenPageDown
            | Action::ScreenLineUp
            | Action::ScreenLineDown
            | Action::ScreenSearch
            | Action::ScreenToggle
            | Action::ScreenScrollUp
            | Action::ScreenScrollDown => Outcome::Ignored(action),
        }
    }

    // ---------------------------------------------------------- internals

    /// A key that went elsewhere (a dialog took it) was pressed after the
    /// interrupt key: a later one does not quit at once.
    pub fn disarm_quit(&mut self) {
        self.quit_armed = false;
    }

    /// The interrupt key while the editor is hidden (a dialog, the viewer
    /// has its place): a running turn is interrupted; otherwise the text,
    /// the search and the menus, which cannot be seen, are left as they are,
    /// and the first press arms the quit, which the hint row says, and the
    /// second quits (the review of step 23).
    pub fn interrupt_hidden(&mut self) -> Outcome {
        if self.running {
            self.quit_armed = false;
            return Outcome::Interrupt;
        }
        if self.quit_armed {
            Outcome::Quit
        } else {
            self.quit_armed = true;
            Outcome::QuitArmed
        }
    }

    fn interrupt(&mut self) -> Outcome {
        if self.running {
            self.quit_armed = false;
            return Outcome::Interrupt;
        }
        self.search = None;
        if let Some(Menu {
            kind: Kind::Picker(name),
            ..
        }) = self.menu.take()
        {
            return Outcome::PickerClosed(name);
        }
        if !self.editor.is_empty() {
            self.quit_armed = false;
            self.editor.clear();
            self.after_edit();
            return Outcome::Changed;
        }
        if self.quit_armed {
            Outcome::Quit
        } else {
            self.quit_armed = true;
            Outcome::QuitArmed
        }
    }

    fn submit(&mut self) -> Outcome {
        if self.editor.expanded().trim().is_empty() {
            return Outcome::Nothing;
        }
        let prompt = self.editor.take();
        let text = prompt.expanded();
        // Kept as written, chips and all, within history's bounds.
        self.history.push(&crate::history::kept(&prompt));
        self.steps.reset();
        self.menu = None;
        self.dismissed = None;
        self.scroll = 0;
        let delivery = match (self.running, self.send_mode) {
            (false, _) => Delivery::Prompt,
            (true, SendMode::Steer) => Delivery::Steer,
            (true, SendMode::Queue) => Delivery::Queue,
        };
        Outcome::Submit { text, delivery }
    }

    fn accept(&mut self) -> Outcome {
        let Some(menu) = self.menu.as_ref() else {
            return Outcome::Ignored(Action::MenuAccept);
        };
        match &menu.kind {
            Kind::Picker(name) => {
                let name = name.clone();
                let picked = menu.picked();
                self.menu = None;
                match picked {
                    Some(index) => Outcome::Picked(name, index),
                    None => Outcome::PickerClosed(name),
                }
            }
            Kind::Files { .. } | Kind::Commands => {
                let Some(row) = menu.selected().map(|at| menu.rows[at].clone()) else {
                    // Nothing to accept: the menu stays, saying so.
                    return Outcome::Nothing;
                };
                let cursor = self.editor.cursor();
                let Some(trigger) = complete::trigger(self.editor.text(), cursor) else {
                    self.menu = None;
                    return Outcome::Changed;
                };
                let (inserted, chosen) = match trigger {
                    Trigger::File { .. } => (complete::file_token(&row.label), true),
                    Trigger::Command { .. } => (format!("/{} ", row.label), false),
                };
                self.editor
                    .replace(trigger.range(cursor), Fragment::text(inserted));
                self.menu = None;
                self.after_edit();
                if chosen {
                    Outcome::FileChosen(row.label)
                } else {
                    Outcome::Changed
                }
            }
        }
    }

    fn close_menu(&mut self) -> Outcome {
        let Some(menu) = self.menu.take() else {
            return Outcome::Ignored(Action::MenuClose);
        };
        match menu.kind {
            Kind::Picker(name) => Outcome::PickerClosed(name),
            _ => {
                self.dismissed = complete::trigger(self.editor.text(), self.editor.cursor());
                Outcome::Changed
            }
        }
    }

    fn dismiss_menu(&mut self) {
        if self
            .menu
            .as_ref()
            .is_some_and(|m| !matches!(m.kind, Kind::Picker(_)))
        {
            self.menu = None;
        }
        self.dismissed = complete::trigger(self.editor.text(), self.editor.cursor());
    }

    fn with_menu(&mut self, action: Action, f: impl FnOnce(&mut Menu)) -> Outcome {
        match self.menu.as_mut() {
            Some(menu) => {
                f(menu);
                Outcome::Changed
            }
            None => Outcome::Ignored(action),
        }
    }

    fn with_search(
        &mut self,
        action: Action,
        f: impl FnOnce(&mut Search, &dyn HistoryStore),
    ) -> Outcome {
        match self.search.as_mut() {
            Some(search) => {
                f(search, self.history.as_ref());
                Outcome::Changed
            }
            None => Outcome::Ignored(action),
        }
    }

    fn on_focus(&mut self, action: Action, f: impl FnOnce(Target) -> Outcome) -> Outcome {
        match self.focus.focused() {
            Target::Input => Outcome::Ignored(action),
            target => f(target),
        }
    }

    /// After the text or the cursor changed: the menu follows the text, and
    /// the cursor stays in view.
    fn after_edit(&mut self) {
        self.refresh_menu();
        self.keep_cursor_in_view();
    }

    fn keep_cursor_in_view(&mut self) {
        let layout = self.editor.layout(self.width);
        let row = layout.cursor_row;
        if row < self.scroll {
            self.scroll = row;
        } else if row >= self.scroll + MAX_ROWS {
            self.scroll = row + 1 - MAX_ROWS;
        }
        let most = layout.rows.len().saturating_sub(MAX_ROWS);
        self.scroll = self.scroll.min(most);
    }

    /// Opens, refills or closes the completion menu for the text at the
    /// cursor. A picker is left alone.
    fn refresh_menu(&mut self) {
        if self
            .menu
            .as_ref()
            .is_some_and(|m| matches!(m.kind, Kind::Picker(_)))
        {
            return;
        }
        // An item being edited is text, not a prompt: `/` and `@` open
        // nothing, and `Enter` saves it.
        if self.item_edit.is_some() {
            self.menu = None;
            return;
        }
        let trigger = complete::trigger(self.editor.text(), self.editor.cursor());
        let file_menu = matches!(trigger, Some(Trigger::File { .. }));
        if !matches!(&trigger, Some(Trigger::File { start, .. }) if self.unlisted == Some(*start)) {
            self.unlisted = None;
        }
        if !file_menu && let Some(elsewhere) = self.elsewhere.as_mut() {
            // No `@` menu now: one asked for again later is filtered again.
            elsewhere.asked = None;
            elsewhere.pending = None;
        }
        let Some(trigger) = trigger else {
            self.menu = None;
            self.dismissed = None;
            return;
        };
        if let Some(dismissed) = &self.dismissed {
            if same_place(dismissed, &trigger) {
                self.menu = None;
                return;
            }
            self.dismissed = None;
        }
        if let (Trigger::File { start, query }, Some(elsewhere)) =
            (&trigger, self.elsewhere.as_mut())
        {
            let kind = Kind::Files { start: *start };
            let open = self.menu.as_ref().is_some_and(|menu| menu.kind == kind);
            // A query asked again in an open menu (the cursor moved, nothing
            // typed) is not filtered again; a menu that opens asks.
            if !open || elsewhere.asked.as_ref() != Some(&(*start, query.clone())) {
                elsewhere.asked = Some((*start, query.clone()));
                elsewhere.generation += 1;
                elsewhere.pending = Some(FileQuery {
                    generation: elsewhere.generation,
                    query: query.clone(),
                });
            }
            // The rows of the last answer stay until this one's come.
            if !open {
                self.menu = Some(Menu::completion(kind, Vec::new(), Empty::Listing));
            }
            return;
        }
        let (kind, rows, empty) = match &trigger {
            Trigger::File { start, query } => match &self.files {
                Some(files) => (
                    Kind::Files { start: *start },
                    complete::rows(&mut self.file_filter, files, query, |_| String::new()),
                    Empty::NoMatch,
                ),
                None if self.unlisted == Some(*start) => {
                    (Kind::Files { start: *start }, Vec::new(), Empty::Unlisted)
                }
                None => (Kind::Files { start: *start }, Vec::new(), Empty::Listing),
            },
            Trigger::Command { query } => {
                let commands = &self.commands;
                (
                    Kind::Commands,
                    complete::rows(
                        &mut self.command_filter,
                        &self.command_candidates,
                        query,
                        |i| {
                            commands
                                .get(i)
                                .map(|c| c.description.clone())
                                .unwrap_or_default()
                        },
                    ),
                    Empty::NoMatch,
                )
            }
        };
        match self.menu.as_mut() {
            Some(menu) if menu.kind == kind => menu.refill(rows, empty),
            _ => self.menu = Some(Menu::completion(kind, rows, empty)),
        }
    }
}

/// An `@` query to filter for, handed out by
/// [`Input::take_file_query`]: rows for it are taken back only while it is
/// the latest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileQuery {
    /// Which query it is: each new one has the next.
    pub generation: u64,
    /// What follows the `@`.
    pub query: String,
}

/// The `@` filtering done elsewhere.
#[derive(Debug, Default)]
struct Elsewhere {
    /// The latest query's generation.
    generation: u64,
    /// The latest query, and the `@` it follows.
    asked: Option<(usize, String)>,
    /// The latest query, until it is handed out.
    pending: Option<FileQuery>,
}

/// Whether two triggers are for the same `@` or the same `/`.
fn same_place(a: &Trigger, b: &Trigger) -> bool {
    match (a, b) {
        (Trigger::File { start: x, .. }, Trigger::File { start: y, .. }) => x == y,
        (Trigger::Command { .. }, Trigger::Command { .. }) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
