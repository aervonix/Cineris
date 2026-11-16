//! The usage panel (`docs/architecture/09-tui.md`, "Panels";
//! `docs/architecture/18-usage.md`): what this session used, the history
//! for a range, and each provider's limits.
//!
//! **Opens** with `/usage`, focused, its history asked for at once
//! (`usage/report`, beside the lane) and drawn `reading…` until it comes;
//! or by itself, collapsed to one line in the warning colour, when a
//! provider's limit is near (less than a fifth left), from the session's
//! live usage (`usage.updated`). **Closes** with `x`, or by itself when the
//! limit that opened it is no longer near.
//!
//! **Its keys:** `tab` goes to the next range (today, the week, the month,
//! all time) and `g` to the next grouping (provider then model, project,
//! speed, provider within project), each asking for the report again; `r`
//! asks the providers in `usage.refresh` for their limits (`usage/refresh`,
//! one at a time: while one is out, the mechanism holds the next back).
//! With `usage.refresh` empty, `r` says so and sends nothing: the list is
//! read with `config/get` when `/usage` opens the panel, and again when a
//! reload changes it.

use cortex_base::{SharedClock, Zone};
use cortex_protocol::event::Event;
use cortex_protocol::methods::{
    ConfigGetParams, Op, OpResult, UsageRefreshParams, UsageReportParams,
};
use cortex_protocol::usage::{ProviderLimits, SessionUsage, UsageGroup, UsageReport};
use cortex_protocol::{ErrorData, SessionId};
use cortex_term::Line;
use serde_json::Value;

use super::{Act, Draw, ItemKey, Panel, Seen, Show};
use crate::keys::{Action, KeyMap, name};
use crate::view::panel::PanelRow;
use crate::view::usage::{self as views, RANGES, UsageView, group_words, range_words};

/// What `r` says with no provider in `usage.refresh`.
pub(crate) const NO_REFRESH: &str = "No provider has the refresh turned on, so nothing was \
                                     asked: name one in `usage.refresh` in your own configuration.";

/// The groupings `g` goes through, in order.
const GROUPS: [&[UsageGroup]; 4] = [
    &[UsageGroup::Provider, UsageGroup::Model],
    &[UsageGroup::Project],
    &[UsageGroup::Speed],
    &[UsageGroup::Project, UsageGroup::Provider],
];

/// The usage panel. See the [module documentation](self).
#[derive(Debug)]
pub(crate) struct UsagePanel {
    /// The session's live usage.
    session: Option<SessionUsage>,
    /// The history for the range chosen.
    report: Option<UsageReport>,
    /// Whether the report for the range chosen is on its way.
    reading: bool,
    /// The range chosen, by its place in [`RANGES`].
    range: usize,
    /// The grouping chosen, by its place in [`GROUPS`].
    group: usize,
    /// The providers in `usage.refresh`, once read.
    refresh: Option<Vec<String>>,
    /// The limits the last refresh gave.
    refreshed: Vec<ProviderLimits>,
    /// What the last refresh said.
    said: Vec<String>,
    /// Whether `/usage` opened it: it stays open with no limit near.
    summoned: bool,
    /// Its parts as its state makes them, kept: the mechanism asks for an
    /// item at every place when the list changes, and each would make them
    /// all again.
    parts: Vec<views::Part>,
    clock: SharedClock,
    /// The zone a reset's time of day is said in: read when the panel
    /// first opens, unless given.
    zone: Option<Zone>,
}

impl UsagePanel {
    /// The panel, its times read from `clock`, in `zone` when one is given
    /// (the system's otherwise, read when it first opens).
    pub(crate) fn new(clock: SharedClock, zone: Option<Zone>) -> Self {
        Self {
            session: None,
            report: None,
            reading: false,
            range: 0,
            group: 0,
            refresh: None,
            refreshed: Vec::new(),
            said: Vec::new(),
            summoned: false,
            parts: Vec::new(),
            clock,
            zone,
        }
    }

    /// Its parts made again, after a change of what they come from.
    fn again(&mut self) {
        self.parts = views::parts(&self.view());
    }

    fn view(&self) -> UsageView<'_> {
        static UTC: std::sync::OnceLock<Zone> = std::sync::OnceLock::new();
        UsageView {
            session: self.session.as_ref(),
            report: self.report.as_ref(),
            reading: self.reading,
            range: &RANGES[self.range % RANGES.len()],
            group: GROUPS[self.group % GROUPS.len()],
            refreshed: &self.refreshed,
            said: &self.said,
            now: self.clock.now(),
            zone: self
                .zone
                .as_ref()
                .unwrap_or_else(|| UTC.get_or_init(Zone::utc)),
        }
    }

    /// The zone, read once, as the panel first opens.
    fn read_zone(&mut self) {
        if self.zone.is_none() {
            self.zone = Some(Zone::system());
        }
    }

    /// Whether a limit is near: what opens it by itself.
    fn near(&self) -> bool {
        views::near(&self.view()).is_some()
    }

    /// The report for the range and the grouping chosen.
    fn report_op(&mut self) -> Op {
        self.reading = true;
        Op::UsageReport(UsageReportParams {
            range: RANGES[self.range % RANGES.len()].clone(),
            group_by: GROUPS[self.group % GROUPS.len()].to_vec(),
            session_id: None,
            project: None,
        })
    }

    /// Open while `/usage` opened it or a limit is near.
    fn show(&mut self) -> Show {
        if self.summoned || self.near() {
            self.read_zone();
            Show::Open
        } else {
            Show::Close
        }
    }

    /// The list of `usage.refresh` in a `config/get` answer, as the file
    /// would have it (`usage = { refresh = [...] }`) or by its dotted key.
    fn refresh_list(config: &serde_json::Map<String, Value>) -> Option<Vec<String>> {
        let value = config
            .get("usage")
            .and_then(|usage| usage.get("refresh"))
            .or_else(|| config.get("usage.refresh"))?;
        Some(
            value
                .as_array()?
                .iter()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect(),
        )
    }
}

impl Panel for UsagePanel {
    fn name(&self) -> &'static str {
        "usage"
    }

    fn seen(&mut self, seen: &Seen<'_>) -> Show {
        let show = self.saw(seen);
        self.again();
        show
    }

    fn result(&mut self, result: &OpResult) -> Show {
        let show = self.answered(result);
        self.again();
        show
    }

    fn failed(&mut self, error: &ErrorData) -> Show {
        // A report refused (statistics a newer build wrote, a file that
        // cannot be read): said in the panel too, as `cortex stats` says
        // it, where the rows would be.
        if self.reading {
            self.reading = false;
            self.report = None;
            self.said.retain(|said| said != &error.message);
            self.said.push(error.message.clone());
        }
        self.again();
        Show::Same
    }

    fn summon(&mut self) -> Result<(), String> {
        self.summoned = true;
        // Its history is asked for as it opens.
        self.reading = true;
        self.read_zone();
        self.again();
        Ok(())
    }

    fn summoned(&self, _session: Option<&SessionId>) -> Vec<Op> {
        let mut asks = vec![Op::UsageReport(UsageReportParams {
            range: RANGES[self.range % RANGES.len()].clone(),
            group_by: GROUPS[self.group % GROUPS.len()].to_vec(),
            session_id: None,
            project: None,
        })];
        // The user's own list: a project's file cannot add a provider.
        asks.push(Op::ConfigGet(ConfigGetParams {
            session_id: None,
            keys: vec!["usage.refresh".to_owned()],
        }));
        asks
    }

    fn collapsed(&self, draw: &Draw<'_>) -> Vec<Line> {
        views::usage_collapsed(&self.view(), self.summoned, draw.width, draw.look)
    }

    fn expanded(&self, focused: usize, draw: &Draw<'_>) -> Vec<PanelRow> {
        views::usage_rows(&self.view(), Some(focused), draw.width, draw.look)
    }

    fn items(&self) -> usize {
        self.parts.len()
    }

    fn least_rows(&self, focused: usize, _draw: &Draw<'_>) -> usize {
        // Its first row, the warning line when a limit is near, and the
        // focused part's first row.
        1 + usize::from(self.near()) + usize::from(focused < self.items())
    }

    fn item(&self, index: usize) -> Option<ItemKey> {
        let part = self.parts.get(index)?;
        // Nothing here acts on a row: what it shows changing is no reason
        // to hold a key back.
        let id = format!("{part:?}");
        Some(ItemKey {
            content: id.clone(),
            id,
        })
    }

    fn hint(&self, _focused: usize, keymap: &KeyMap) -> Vec<String> {
        self.keys(keymap)
    }

    fn act(&mut self, action: Action, _focused: usize, _session: Option<&SessionId>) -> Act {
        let act = self.key(action);
        self.again();
        act
    }
}

impl UsagePanel {
    /// What it saw does to its state, and whether it opens or closes.
    fn saw(&mut self, seen: &Seen<'_>) -> Show {
        match seen {
            Seen::Event { event, .. } => match event {
                Event::UsageUpdated(updated) => {
                    let was = self.near();
                    self.session = Some(updated.usage.clone());
                    let near = self.near();
                    // Opened by a limit near, closed when none is; else as
                    // it was.
                    match (was, near) {
                        (false, true) => self.show(),
                        (true, false) if !self.summoned => Show::Close,
                        _ => Show::Same,
                    }
                }
                Event::ConfigChanged(changed)
                    if changed.applied.iter().any(|key| key.starts_with("usage.")) =>
                {
                    // Read again when `r` is next pressed.
                    self.refresh = None;
                    Show::Same
                }
                _ => Show::Same,
            },
            Seen::Prompt => Show::Same,
            Seen::Left => {
                let (clock, zone) = (self.clock.clone(), self.zone.take());
                *self = Self::new(clock, zone);
                Show::Close
            }
        }
    }

    /// What an answer does to its state.
    fn answered(&mut self, result: &OpResult) -> Show {
        match result {
            OpResult::UsageReport(report) => {
                // The answer for what was asked last: a range or grouping
                // asked for since is still on its way.
                if report.range == RANGES[self.range % RANGES.len()]
                    && report.group_by == GROUPS[self.group % GROUPS.len()]
                {
                    self.report = Some(report.clone());
                    self.reading = false;
                }
                Show::Same
            }
            OpResult::ConfigGet(got) => {
                self.refresh = Self::refresh_list(&got.config);
                Show::Same
            }
            OpResult::UsageRefresh(refreshed) => {
                self.said.clear();
                if refreshed.providers.is_empty() {
                    self.refresh = Some(Vec::new());
                    self.said.push(NO_REFRESH.to_owned());
                }
                for outcome in &refreshed.providers {
                    if let Some(limits) = &outcome.limits {
                        self.refreshed.retain(|had| had.provider != limits.provider);
                        self.refreshed.push(limits.clone());
                    }
                    if let Some(error) = &outcome.error {
                        self.said.push(format!(
                            "{}: the refresh failed: {}",
                            outcome.provider.as_str(),
                            error.message
                        ));
                    }
                }
                Show::Same
            }
            _ => Show::Same,
        }
    }

    /// The keys the hint row shows.
    fn keys(&self, keymap: &KeyMap) -> Vec<String> {
        let key = |action: Action| keymap.keys_of(action).into_iter().next().map(name);
        let mut keys = Vec::new();
        if let Some(range) = key(Action::FocusRange) {
            let next = &RANGES[(self.range + 1) % RANGES.len()];
            keys.push(format!("{range} shows {}", range_words(next)));
        }
        if let Some(group) = key(Action::FocusGroup) {
            let next = GROUPS[(self.group + 1) % GROUPS.len()];
            keys.push(format!("{group} groups by {}", group_words(next)));
        }
        if self.refresh.as_ref().is_none_or(|list| !list.is_empty())
            && let Some(refresh) = key(Action::FocusRefresh)
        {
            keys.push(format!("{refresh} refreshes the limits"));
        }
        if let Some(close) = key(Action::FocusClose) {
            keys.push(format!("{close} closes"));
        }
        if let Some(back) = key(Action::FocusReturn) {
            keys.push(format!("{back} back to the input"));
        }
        keys
    }

    /// What a key does to its state.
    fn key(&mut self, action: Action) -> Act {
        match action {
            Action::FocusRange => {
                self.range = (self.range + 1) % RANGES.len();
                Act::Send(self.report_op())
            }
            Action::FocusGroup => {
                self.group = (self.group + 1) % GROUPS.len();
                Act::Send(self.report_op())
            }
            Action::FocusRefresh => match &self.refresh {
                Some(list) if list.is_empty() => Act::Say(NO_REFRESH.to_owned()),
                _ => Act::Send(Op::UsageRefresh(UsageRefreshParams::default())),
            },
            Action::FocusClose => {
                self.summoned = false;
                Act::Close
            }
            _ => Act::Pass,
        }
    }
}
