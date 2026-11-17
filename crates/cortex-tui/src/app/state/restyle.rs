//! The look and the key map read again from the configuration while the
//! application runs (step 24's hot reload; `docs/architecture/09-tui.md`,
//! "Themes and key bindings"). One watcher, the core's: after a key's
//! frame (at most once a second) and on a focus-in the application asks
//! the core to look at the configuration's files (`config/reload` with
//! `changed_only`); when a reload, whoever's it was, applied a `tui.*` or
//! `keys.*` key, the application asks for those two sections
//! (`config/get`) and builds its look, and its key map when a binding
//! changed, from them, through the function the program gave it
//! ([`App::set_restyle`]), which knows the configuration's types.

use std::time::{Duration, Instant};

use cortex_protocol::methods::{ConfigGetParams, ConfigReloadParams, Op};
use serde_json::{Map, Value};

use super::{App, Pending};
use crate::app::Settings;
use crate::keys::KeyMap;

/// The fewest seconds between two looks a key asks for.
const LOOK_EVERY: Duration = Duration::from_secs(1);

/// What the program's function made of the configuration's `tui` and
/// `keys` sections.
#[derive(Debug, Clone)]
pub struct Restyled {
    /// The look's settings.
    pub settings: Settings,
    /// The key map.
    pub keymap: KeyMap,
    /// What of the look could not be used (a theme not known): said when
    /// the theme changed.
    pub look_notices: Vec<String>,
    /// What of the key map could not be used (a binding the map cannot
    /// take): said when the key map is replaced.
    pub key_notices: Vec<String>,
}

/// Builds [`Restyled`] from `config/get`'s answer for the `tui` and `keys`
/// sections (its dotted keys and their values), or says why it could not:
/// then the look and the key map stay as they were.
pub type Restyle = Box<dyn Fn(&Map<String, Value>) -> Result<Restyled, String> + Send>;

impl App {
    /// The look's settings in force, for the tests.
    #[cfg(test)]
    pub(crate) fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Gives the application the function that reads the look and the key
    /// map from the configuration: with none, a reload of them is not
    /// taken while the application runs.
    pub fn set_restyle(&mut self, restyle: Restyle) {
        self.restyle = Some(restyle);
    }

    /// Input, or the terminal's focus regained, at `at`: the files are to be
    /// looked at after this frame, at most once a second.
    pub(super) fn look_wanted(&mut self, at: Instant) {
        if self
            .last_look
            .is_none_or(|last| at.saturating_duration_since(last) >= LOOK_EVERY)
        {
            self.look_due = true;
        }
    }

    /// After a frame: the look that was due, unless a reload is already
    /// out.
    pub(super) fn look_now(&mut self, at: Instant) {
        if !std::mem::take(&mut self.look_due) {
            return;
        }
        if self.session.is_none()
            || self
                .pending
                .values()
                .any(|pending| matches!(pending, Pending::Reload | Pending::Look))
        {
            return;
        }
        self.last_look = Some(at);
        let ticket = self.link.send(Op::ConfigReload(ConfigReloadParams {
            changed_only: true,
            ..ConfigReloadParams::default()
        }));
        self.pending.insert(ticket, Pending::Look);
    }

    /// What a reload applied: when a key of the look or of the key map is
    /// among it, both sections are asked for again.
    pub(super) fn restyle_if(&mut self, applied: &[String]) {
        let tui = applied.iter().any(|key| key.starts_with("tui."));
        let keys = applied.iter().any(|key| key.starts_with("keys."));
        if !(tui || keys) || self.restyle.is_none() {
            return;
        }
        let theme = applied.iter().any(|key| key == "tui.theme");
        // One already asked for takes what this one changed too: its
        // answer is read after both.
        if let Some(Pending::Restyle {
            keys: asked_keys,
            theme: asked_theme,
        }) = self
            .pending
            .values_mut()
            .find(|pending| matches!(pending, Pending::Restyle { .. }))
        {
            *asked_keys |= keys;
            *asked_theme |= theme;
            return;
        }
        let Some(session) = self.session.as_ref().map(|s| s.session_id.clone()) else {
            return;
        };
        let ticket = self.link.send(Op::ConfigGet(ConfigGetParams {
            session_id: Some(session),
            keys: vec!["tui".to_owned(), "keys".to_owned()],
        }));
        self.pending
            .insert(ticket, Pending::Restyle { keys, theme });
    }

    /// The sections came: the look is built again, in force from the next
    /// frame, and the key map when a binding changed (a key already read
    /// was taken with the old one). A theme chosen with `/theme` for this
    /// run stays unless the file's own theme changed.
    pub(super) fn restyled(&mut self, config: &Map<String, Value>, keys: bool, theme: bool) {
        let Some(restyle) = &self.restyle else {
            return;
        };
        let Restyled {
            mut settings,
            keymap,
            look_notices,
            key_notices,
        } = match restyle(config) {
            Ok(restyled) => restyled,
            Err(why) => {
                self.say(&why);
                return;
            }
        };
        if theme {
            self.theme_picked = None;
        } else if let Some(picked) = self.theme_picked {
            settings.theme = picked;
        }
        self.settings = settings;
        self.look = crate::app::look(&self.settings, &self.caps);
        for dialog in &mut self.dialogs {
            dialog.restyled();
        }
        if keys {
            self.set_keymap(keymap);
        }
        // Each said only with the change it is about: an entry that could
        // not be used is not said again for a change of something else.
        let said = if theme { look_notices } else { Vec::new() };
        for notice in said
            .into_iter()
            .chain(if keys { key_notices } else { Vec::new() })
        {
            self.say(&notice);
        }
    }
}
