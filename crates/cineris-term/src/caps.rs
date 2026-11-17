//! What the terminal can do: detected from the environment at start, asked
//! of the terminal as it is entered, and overridden by settings
//! (`docs/architecture/09-tui.md`, "Terminal layer").
//!
//! Start-up never waits for an answer. [`Capabilities::detect`] decides from
//! the environment alone, conservatively; the queries in
//! [`Capabilities::QUERIES`] go out just before the first frame; and each reply,
//! when it comes, goes to [`Capabilities::apply`], which upgrades what later
//! frames do. An override always wins over both.

use crate::input::Reply;
use crate::style::ColorDepth;

/// What a setting says about one capability: on or off whatever the
/// terminal says. `None` leaves it to detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Overrides {
    /// Synchronized updates.
    pub sync: Option<bool>,
    /// The enhanced keyboard protocol.
    pub keyboard: Option<bool>,
    /// How many colours.
    pub color: Option<ColorDepth>,
    /// Links in text.
    pub hyperlinks: Option<bool>,
    /// Unicode glyphs, or ASCII ones.
    pub unicode: Option<bool>,
    /// Byte 0x08 is `Backspace` (`true`) or `Ctrl+Backspace` (`false`);
    /// unset, it is `Backspace` where the terminal's erase character is
    /// `^H` (since step 25).
    pub backspace_ctrl_h: Option<bool>,
    /// Bracketed paste: `true` trusts the terminal's brackets and never
    /// takes keys for a paste; `false` does not ask for brackets, and takes
    /// a burst of keys for a paste in every terminal; unset, it asks, and
    /// takes a burst for a paste only where the terminal has not said it
    /// knows the mode (since step 25).
    pub bracketed_paste: Option<bool>,
    /// How long a bracketed paste may pause before it is given up on and
    /// what it holds given as a paste; unset, two seconds (since step 25).
    pub paste_wait: Option<std::time::Duration>,
    /// The terminal takes the clipboard sequence (OSC 52); unset, guessed
    /// (since step 25).
    pub clipboard: Option<bool>,
}

/// The system the terminal runs on, which changes what the environment
/// means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Linux, and any other Unix.
    Unix,
    /// Windows.
    Windows,
}

impl Platform {
    /// The system this program was built for.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

/// What the terminal can do, as far as is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent capabilities of a terminal, each present or not"
)]
pub struct Capabilities {
    /// Frames are wrapped in synchronized-update brackets (mode 2026), so a
    /// terminal that has them shows each frame whole.
    pub sync: bool,
    /// The terminal has the enhanced keyboard protocol, which tells apart
    /// keys that are otherwise the same bytes (Shift+Enter and Enter). Known
    /// only from its reply; until then, `false`.
    pub keyboard: bool,
    /// The terminal has Windows Terminal's win32 input mode (mode 9001),
    /// which reports every key with all its modifiers. Known only from its
    /// reply; until then, `false`. The `keyboard` override covers it too.
    pub win32_input: bool,
    /// How many colours it shows.
    pub color: ColorDepth,
    /// It shows links in text (OSC 8).
    pub hyperlinks: bool,
    /// It shows Unicode glyphs; otherwise the interface uses ASCII ones.
    pub unicode: bool,
    /// Its background colour, from its reply. Used to tint the accent.
    pub background: Option<(u8, u8, u8)>,
    /// The terminal may be asked for the mouse by its sequences (modes 1000
    /// and 1006): on Unix, and even there not while the win32 input mode is
    /// on (the terminal checks). On Windows nothing asks for the mouse, the
    /// console's mouse input included: through a pseudo-console the
    /// terminal's reports reached the program as typed keys when the program
    /// asked for them, and the pseudo-console asks for them itself when the
    /// console's mouse input is on (both measured in step 25; that those
    /// reports come typed too is inferred).
    pub vt_mouse: bool,
    /// Escape sequences are of no use at all (`TERM=dumb`): the interactive
    /// interface should not start.
    pub dumb: bool,
    /// Byte 0x08 is `Backspace` (since step 25): the override, or the
    /// terminal's erase character, which the terminal layer reads as it
    /// enters the terminal.
    pub backspace_ctrl_h: bool,
    /// The terminal said it knows bracketed paste (DECRQM 2004), so its
    /// pastes come bracketed and no burst of keys is taken for one (since
    /// step 25). Known only from its reply; until then, `false`.
    pub paste_confirmed: bool,
    /// The terminal is thought to take the clipboard sequence (OSC 52),
    /// which it never answers (since step 25): not the old console host,
    /// tmux (whose default passes no program's sequence on) or GNU screen.
    pub clipboard: bool,
    overrides: Overrides,
}

/// Terminals known to show links, by the environment variable each sets
/// (and, for `TERM_PROGRAM`, its value).
const LINK_TERMINALS: &[&str] = &["iTerm.app", "WezTerm", "vscode", "ghostty", "Hyper"];

/// Terminals on the deny-list for synchronized updates, by `TERM`. A
/// terminal that does not know the mode ignores it, which is why the list is
/// short: these are the ones known to show the sequence or to misbehave.
const NO_SYNC_TERMS: &[&str] = &["dumb", "linux"];

impl Capabilities {
    /// The queries sent as the terminal is entered: whether synchronized updates
    /// are supported (DECRQM 2026), whether the win32 input mode is (DECRQM
    /// 9001), the enhanced keyboard protocol's flags,
    /// the background colour, and last the device attributes, which every
    /// terminal answers.
    pub const QUERIES: &'static [u8] =
        b"\x1b[?2026$p\x1b[?9001$p\x1b[?2004$p\x1b[?u\x1b]11;?\x1b\\\x1b[c";

    /// Decides from the environment, then applies `overrides`. `env` reads
    /// one environment variable.
    pub fn detect(
        env: impl Fn(&str) -> Option<String>,
        platform: Platform,
        overrides: Overrides,
    ) -> Self {
        let var = |name: &str| env(name).filter(|value| !value.is_empty());
        let term = var("TERM").unwrap_or_default();
        let program = var("TERM_PROGRAM").unwrap_or_default();
        let windows_terminal = var("WT_SESSION").is_some();
        let dumb = term == "dumb";
        // On Windows a terminal that sets none of these is taken to be the
        // old console host, which has no synchronized updates and draws few
        // glyphs (`09-tui.md`, "Windows").
        let legacy_console = platform == Platform::Windows
            && !windows_terminal
            && program.is_empty()
            && term.is_empty();

        let color = if var("NO_COLOR").is_some() {
            ColorDepth::Mono
        } else if matches!(var("COLORTERM").as_deref(), Some("truecolor" | "24bit"))
            || windows_terminal
            || LINK_TERMINALS.contains(&program.as_str())
            || term == "xterm-kitty"
            || term.contains("direct")
        {
            ColorDepth::TrueColor
        } else if term.contains("256") {
            ColorDepth::Indexed256
        } else {
            ColorDepth::Ansi16
        };

        let hyperlinks = !dumb
            && var("TMUX").is_none()
            && (windows_terminal
                || LINK_TERMINALS.contains(&program.as_str())
                || term == "xterm-kitty"
                || var("KONSOLE_VERSION").is_some()
                || var("VTE_VERSION")
                    .and_then(|version| version.parse::<u32>().ok())
                    .is_some_and(|version| version >= 5000));

        let unicode = match platform {
            Platform::Windows => !legacy_console,
            Platform::Unix => ["LC_ALL", "LC_CTYPE", "LANG"]
                .iter()
                .find_map(|name| var(name))
                .is_some_and(|locale| {
                    let locale = locale.to_ascii_lowercase();
                    locale.contains("utf-8") || locale.contains("utf8")
                }),
        };

        let mut caps = Self {
            sync: !dumb && !legacy_console && !NO_SYNC_TERMS.contains(&term.as_str()),
            keyboard: false,
            win32_input: false,
            color,
            hyperlinks,
            unicode,
            background: None,
            dumb,
            vt_mouse: platform == Platform::Unix,
            backspace_ctrl_h: overrides.backspace_ctrl_h.unwrap_or(false),
            paste_confirmed: false,
            clipboard: !dumb
                && !legacy_console
                && var("TMUX").is_none()
                && !term.starts_with("screen"),
            overrides,
        };
        caps.enforce();
        caps
    }

    /// Takes in a reply to one of [`QUERIES`](Self::QUERIES). Returns
    /// whether anything changed.
    pub fn apply(&mut self, reply: &Reply) -> bool {
        let before = *self;
        match *reply {
            Reply::Mode { mode: 2026, state } => match state {
                1..=3 => self.sync = true,
                4 => self.sync = false,
                // 0: the terminal does not know the mode. It ignores it, so
                // the brackets stay: they cost nothing.
                _ => {}
            },
            Reply::Mode { mode: 9001, state } => self.win32_input = matches!(state, 1..=3),
            // Known, set or not: the terminal brackets its pastes (step 25).
            Reply::Mode { mode: 2004, state } => self.paste_confirmed = matches!(state, 1..=4),
            Reply::Keyboard(_) => self.keyboard = true,
            Reply::Background(r, g, b) => self.background = Some((r, g, b)),
            Reply::Mode { .. } | Reply::DeviceAttributes => {}
        }
        self.enforce();
        *self != before
    }

    /// The overrides, as given to [`detect`](Self::detect).
    #[must_use]
    pub fn overrides(&self) -> Overrides {
        self.overrides
    }

    /// Whether a burst of keys in one read is taken for a paste (since step
    /// 25): unless the override trusts the brackets, where the terminal has
    /// not said it knows them, or everywhere when the override says not to
    /// ask for them.
    #[must_use]
    pub fn rates_pastes(&self) -> bool {
        match self.overrides.bracketed_paste {
            Some(true) => false,
            Some(false) => true,
            None => !self.paste_confirmed,
        }
    }

    fn enforce(&mut self) {
        let o = self.overrides;
        if let Some(sync) = o.sync {
            self.sync = sync;
        }
        if let Some(keyboard) = o.keyboard {
            self.keyboard = keyboard;
            self.win32_input &= keyboard;
        }
        if let Some(color) = o.color {
            self.color = color;
        }
        if let Some(hyperlinks) = o.hyperlinks {
            self.hyperlinks = hyperlinks;
        }
        if let Some(unicode) = o.unicode {
            self.unicode = unicode;
        }
        if let Some(clipboard) = o.clipboard {
            self.clipboard = clipboard;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(vars: &[(&str, &str)], platform: Platform) -> Capabilities {
        detect_with(vars, platform, Overrides::default())
    }

    /// The clipboard sequence is thought taken except by the old console
    /// host, tmux and GNU screen; the override has the last word (step 25).
    #[test]
    fn the_clipboard_is_guessed_from_the_terminal_and_overridden() {
        let xterm = [("TERM", "xterm-256color")];
        assert!(detect(&xterm, Platform::Unix).clipboard);
        assert!(
            !detect(
                &[("TERM", "xterm-256color"), ("TMUX", "/tmp/t,1,0")],
                Platform::Unix
            )
            .clipboard
        );
        assert!(!detect(&[("TERM", "screen-256color")], Platform::Unix).clipboard);
        assert!(
            !detect(&[], Platform::Windows).clipboard,
            "the old console host"
        );
        assert!(detect(&[("WT_SESSION", "1")], Platform::Windows).clipboard);
        let told = |clipboard| Overrides {
            clipboard: Some(clipboard),
            ..Overrides::default()
        };
        assert!(detect_with(&[("TMUX", "/tmp/t,1,0")], Platform::Unix, told(true)).clipboard);
        assert!(!detect_with(&xterm, Platform::Unix, told(false)).clipboard);
    }

    /// A terminal that says it knows bracketed paste, set or not, has no
    /// burst of keys taken for a paste; one that does not know it, or does
    /// not say, has. The override trusts the brackets, or asks for none
    /// (step 25).
    #[test]
    fn bursts_are_taken_for_pastes_only_where_brackets_are_not_known() {
        let xterm = [("TERM", "xterm-256color")];
        for (state, rates) in [(1, false), (2, false), (3, false), (4, false), (0, true)] {
            let mut caps = detect(&xterm, Platform::Unix);
            assert!(caps.rates_pastes(), "not known until it says");
            caps.apply(&Reply::Mode { mode: 2004, state });
            assert_eq!(caps.rates_pastes(), rates, "state {state}");
        }
        let told = |bracketed_paste| Overrides {
            bracketed_paste: Some(bracketed_paste),
            ..Overrides::default()
        };
        assert!(!detect_with(&xterm, Platform::Unix, told(true)).rates_pastes());
        let mut caps = detect_with(&xterm, Platform::Unix, told(false));
        caps.apply(&Reply::Mode {
            mode: 2004,
            state: 1,
        });
        assert!(caps.rates_pastes(), "not asked for: every burst is weighed");
    }

    fn detect_with(
        vars: &[(&str, &str)],
        platform: Platform,
        overrides: Overrides,
    ) -> Capabilities {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        Capabilities::detect(
            move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()),
            platform,
            overrides,
        )
    }

    #[test]
    fn a_plain_linux_terminal_gets_the_conservative_set() {
        let caps = detect(&[("TERM", "xterm")], Platform::Unix);
        assert!(caps.sync);
        assert!(!caps.keyboard);
        assert_eq!(caps.color, ColorDepth::Ansi16);
        assert!(!caps.hyperlinks);
        assert!(!caps.unicode);
        assert!(!caps.dumb);
    }

    #[test]
    fn the_environment_upgrades_it() {
        let caps = detect(
            &[
                ("TERM", "xterm-256color"),
                ("COLORTERM", "truecolor"),
                ("LANG", "en_GB.UTF-8"),
                ("VTE_VERSION", "7600"),
            ],
            Platform::Unix,
        );
        assert_eq!(caps.color, ColorDepth::TrueColor);
        assert!(caps.unicode && caps.hyperlinks);
        assert_eq!(
            detect(&[("TERM", "screen-256color")], Platform::Unix).color,
            ColorDepth::Indexed256
        );
        // Inside tmux a link may not reach the outer terminal.
        assert!(
            !detect(
                &[("TERM_PROGRAM", "WezTerm"), ("TMUX", "x")],
                Platform::Unix
            )
            .hyperlinks
        );
        assert_eq!(
            detect(
                &[("TERM", "xterm-256color"), ("NO_COLOR", "1")],
                Platform::Unix
            )
            .color,
            ColorDepth::Mono
        );
        let dumb = detect(&[("TERM", "dumb")], Platform::Unix);
        assert!(dumb.dumb && !dumb.sync);
        assert!(!detect(&[("TERM", "linux")], Platform::Unix).sync);
    }

    #[test]
    fn windows_terminal_and_the_old_console_host() {
        let wt = detect(&[("WT_SESSION", "x")], Platform::Windows);
        assert!(wt.sync && wt.unicode && wt.hyperlinks);
        assert_eq!(wt.color, ColorDepth::TrueColor);
        let conhost = detect(&[], Platform::Windows);
        assert!(!conhost.sync && !conhost.unicode && !conhost.hyperlinks);
        assert_eq!(conhost.color, ColorDepth::Ansi16);
    }

    #[test]
    fn replies_upgrade_and_overrides_win() {
        let mut caps = detect(&[("TERM", "xterm")], Platform::Unix);
        assert!(caps.apply(&Reply::Keyboard(0)));
        assert!(caps.keyboard);
        assert!(caps.apply(&Reply::Background(1, 2, 3)));
        assert_eq!(caps.background, Some((1, 2, 3)));
        assert!(caps.apply(&Reply::Mode {
            mode: 2026,
            state: 4
        }));
        assert!(!caps.sync);
        assert!(!caps.apply(&Reply::DeviceAttributes));
        assert!(!caps.win32_input);
        assert!(caps.apply(&Reply::Mode {
            mode: 9001,
            state: 2
        }));
        assert!(caps.win32_input);
        assert!(!detect(&[], Platform::Windows).win32_input);

        let overrides = Overrides {
            sync: Some(true),
            keyboard: Some(false),
            color: Some(ColorDepth::Indexed256),
            hyperlinks: Some(true),
            unicode: Some(true),
            ..Overrides::default()
        };
        let mut caps = detect_with(&[("TERM", "dumb")], Platform::Unix, overrides);
        assert!(caps.sync && caps.hyperlinks && caps.unicode);
        assert_eq!(caps.color, ColorDepth::Indexed256);
        caps.apply(&Reply::Keyboard(1));
        caps.apply(&Reply::Mode {
            mode: 2026,
            state: 4,
        });
        caps.apply(&Reply::Mode {
            mode: 9001,
            state: 1,
        });
        assert!(!caps.keyboard && !caps.win32_input && caps.sync);
        assert_eq!(caps.overrides(), overrides);
    }
}
