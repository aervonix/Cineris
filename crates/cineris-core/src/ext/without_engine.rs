//! A build without the script engine (`scripts` off; decision 0005;
//! `docs/architecture/12-extensions.md`, "A build without the script
//! engine").
//!
//! Nothing of an extension's script runs in such a build, so no extension
//! is loaded: none of its tools, commands, hooks or prompt text is in a
//! session. The hook commands of the configuration are not an extension's:
//! they run as they do with the engine (`super::hook_command`).
//!
//! **An extension's gates do not lapse for it**, as they do not where the
//! engine cannot load one ("Its gates fail closed"). Each extension that
//! stands approved (the user's own pinned and not turned off; a trusted
//! project's pinned, or approved for the run; whether it is found as it was
//! approved, changed, unreadable or gone) holds its gating hooks
//! (`tool.before`, `prompt.submit`, not observing) on their events, each a
//! hook whose run fails, saying why and the way out:
//!
//! - those it registered at its pin's hash, as the registration cache keeps
//!   them, each on the tools it named;
//! - where the cache does not keep that and it is found as it was approved, a
//!   gate on each gating event its manifest lists, on every call;
//! - where neither is known (it changed, cannot be read, or is gone), both
//!   gates, on every call and every prompt.
//!
//! While the pin store cannot be read, each of the user's extensions found
//! holds the gating hooks of the cache's entries of its name, and a gate on
//! each gating event its manifest lists; a project's, while the trust store
//! cannot be read, holds nothing: no repository gates a session by itself,
//! as with the engine. One approved for this session by a yes the store
//! could not keep stands approved all the same, as the session's finding
//! judges it with the engine, which loads it: it holds its gates.
//!
//! What is held is settled as the session opens and kept for it, as its hook
//! commands are: a change takes effect at the next session. The user is
//! told once of each extension that stands approved and is not loaded.
//!
//! The decision is built in every build, so that its tests run in the
//! ordinary one; only a build without the engine puts it in a session.

#![cfg_attr(
    feature = "scripts",
    allow(
        dead_code,
        reason = "with the script engine extensions are loaded, and only the tests take this \
                  decision"
    )
)]

use cineris_agent::HookEvent as LoopEvent;
use cineris_base::{ContentHash, in_code};
use cineris_ext::event::Event;
use cineris_ext::hook::Filter;
use cineris_ext::registration::HookDef;
use cineris_ext::source::Source;
use cineris_protocol::methods::TrustResourceKind;

use super::hook_points::loop_event;
use super::hooks::named;
use super::manage::{ExtensionListed, ExtensionStanding, Judging, judged, toml_key};

/// Why an extension that stands approved is not loaded in a build without
/// the script engine.
pub(crate) const NO_SCRIPT_ENGINE: &str =
    "this build of Cineris has no script engine, which runs it";

/// A gating hook of an extension that stands approved, held on its event
/// as a hook whose run fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Held {
    /// The extension's name.
    pub(crate) extension: String,
    /// The hook's name: the extension's, `hook.` and the event, numbered
    /// from the second of one event on (`tracker/hook.tool.before.2`).
    pub(crate) hook: String,
    /// Its event: `tool.before` or `prompt.submit`.
    pub(crate) event: LoopEvent,
    /// The tools it is on: `None` for every call.
    pub(crate) filter: Option<Filter>,
    /// The failure its run gives the call or prompt it refuses: short, as
    /// the model is given it.
    pub(crate) why: String,
}

impl Held {
    /// The gate as `extensions/status` lists it: its extension's name, the
    /// hook's name, its event and why it refuses.
    pub(crate) fn listed(&self) -> (String, String, String, String) {
        (
            self.extension.clone(),
            self.hook.clone(),
            self.event.as_str().to_owned(),
            self.why.clone(),
        )
    }
}

/// What a session without the script engine holds of its extensions, and
/// what it tells the user of them, once.
#[derive(Debug, Default)]
pub(crate) struct Holding {
    /// Each gating hook held.
    pub(crate) held: Vec<Held>,
    /// What the session is told, a sentence or two of each extension that
    /// stands approved.
    pub(crate) told: Vec<String>,
}

/// What a session's extensions are judged from, as it opened: what
/// `extensions/status` judges them from again.
#[derive(Debug, Clone)]
pub(crate) struct SessionView {
    pub(crate) resolver: std::sync::Arc<cineris_base::PathResolver>,
    pub(crate) config_dir: Option<std::path::PathBuf>,
    pub(crate) state: Option<cineris_config::StateDir>,
    pub(crate) project: Option<cineris_protocol::ProjectKey>,
    pub(crate) for_the_run: Vec<cineris_policy::Definition>,
    pub(crate) disabled: Vec<String>,
}

/// What `extensions/status` says of an extension standing `standing` in a
/// build without the script engine, where nothing is loaded: why one
/// approved as it is found is not, in place of `ready`. `None` for the
/// rest, whose standing says it.
pub(crate) fn not_loaded(standing: &ExtensionStanding) -> Option<String> {
    matches!(standing, ExtensionStanding::Approved)
        .then(|| format!("approved, and not loaded: {NO_SCRIPT_ENGINE}"))
}

/// What a session without the script engine holds of its extensions, and
/// what it is told of them, judged from what it opened with (`view`): the
/// stores of approvals, the folders and the registration cache read as
/// `extensions/status` reads them. Blocks.
pub(crate) fn in_session(
    view: &SessionView,
    clock: &cineris_base::SharedClock,
    home: Option<&std::path::Path>,
) -> Holding {
    super::manage::judge(
        &super::manage::JudgedFrom {
            resolver: &view.resolver,
            config: view.config_dir.as_deref(),
            state: view.state.as_ref(),
            project: view.project.as_ref(),
            for_the_run: &view.for_the_run,
            disabled: &view.disabled,
            clock,
            home,
        },
        holding,
    )
    .0
}

/// What a session without the script engine holds of the extensions
/// `judging` finds, and what it is told.
pub(crate) fn holding(judging: &Judging<'_>) -> Holding {
    let mut holding = Holding::default();
    for one in judged(judging) {
        let Some(gates) = gates(judging, &one) else {
            continue;
        };
        let out = way_out(one.source, &one.name);
        let code = in_code(&one.name);
        let why = format!(
            "the extension `{code}` is not loaded: {NO_SCRIPT_ENGINE}, so its gate refuses it: \
             {out}"
        );
        let mut events: Vec<LoopEvent> = Vec::new();
        for (hook, event, filter) in gates {
            let Some(event) = loop_event(event) else {
                continue;
            };
            if !events.contains(&event) {
                events.push(event);
            }
            holding.held.push(Held {
                extension: one.name.clone(),
                hook,
                event,
                filter,
                why: why.clone(),
            });
        }
        let whose = match one.source {
            Source::User => "Your extension",
            Source::Project => "This project's extension",
        };
        let at = one
            .folder
            .as_deref()
            .map(|folder| format!(" (`{folder}`)"))
            .unwrap_or_default();
        let how = match &one.standing {
            ExtensionStanding::Changed => " has changed since you approved it, and".to_owned(),
            ExtensionStanding::Gone => " cannot be found where you approved it, and".to_owned(),
            ExtensionStanding::LeftOut(_) => " cannot be read as you approved it, and".to_owned(),
            ExtensionStanding::Unjudged(why) => format!(" cannot be judged ({why}), and"),
            _ => String::new(),
        };
        let mut told = format!("{whose} `{code}`{at}{how} is not loaded: {NO_SCRIPT_ENGINE}.");
        if !events.is_empty() {
            let gates = if events.len() > 1 {
                "Its gates on `prompt.submit` and `tool.before` hold, refusing every prompt, and \
                 every call they are on"
            } else if events[0] == LoopEvent::PromptSubmit {
                "Its gate on `prompt.submit` holds, refusing every prompt"
            } else {
                "Its gate on `tool.before` holds, refusing every call it is on"
            };
            told.push_str(&format!(" {gates}, until you {out}."));
        }
        holding.told.push(told);
    }
    holding
}

/// Each gating hook `one` holds, by its name, event and the tools it is
/// on (`None`: every call), as it stands: `None` when it does not stand
/// approved, and nothing of it is held.
fn gates(
    judging: &Judging<'_>,
    one: &ExtensionListed,
) -> Option<Vec<(String, Event, Option<Filter>)>> {
    let manifest_events = || {
        found_manifest(judging, one.source, &one.name)
            .map(|manifest| manifest.capabilities.hooks.clone())
            .unwrap_or_default()
    };
    match &one.standing {
        ExtensionStanding::TurnedOff
        | ExtensionStanding::NotApproved
        | ExtensionStanding::NotUsed(_)
        | ExtensionStanding::ProjectNotTrusted => None,
        // No repository gates a session by itself: a project's whose store
        // cannot be read holds nothing.
        ExtensionStanding::Unjudged(_) if one.source == Source::Project => None,
        // What the cache keeps of its name, whatever its hash, and its
        // manifest's events as found: only ever adding.
        ExtensionStanding::Unjudged(_) => {
            let mut gates: Vec<(String, Event, Option<Filter>)> = Vec::new();
            if let Some(cache) = judging.cache {
                for (_, entry) in cache.iter().filter(|(_, entry)| entry.name == one.name) {
                    gates.extend(registered(&one.name, &entry.registration.hooks));
                }
            }
            gates.extend(on_every_call(&one.name, &manifest_events()));
            Some(gates)
        }
        ExtensionStanding::Approved
        | ExtensionStanding::Changed
        | ExtensionStanding::Gone
        | ExtensionStanding::LeftOut(_) => {
            let pin = pin_of(judging, one.source, &one.name)?;
            let cached = judging
                .cache
                .and_then(|cache| cache.get(&pin))
                .filter(|entry| entry.name == one.name);
            Some(match cached {
                Some(entry) => registered(&one.name, &entry.registration.hooks),
                // Found as it was approved: its manifest is the one approved.
                None if one.standing == ExtensionStanding::Approved => {
                    on_every_call(&one.name, &manifest_events())
                }
                // What was approved is known nowhere: both gates.
                None => on_every_call(&one.name, &[Event::PromptSubmit, Event::ToolBefore]),
            })
        }
    }
}

/// The gating hooks, not observing, of what `extension` registered, each on
/// the tools it named; one whose tools cannot be read is on every call.
fn registered(extension: &str, hooks: &[HookDef]) -> Vec<(String, Event, Option<Filter>)> {
    named(extension, hooks, None)
        .into_iter()
        .filter(|(_, _, event, definition)| event.gates() && !definition.observe)
        .map(|(_, name, event, definition)| {
            (name, event, Filter::new(definition.tools.clone()).ok())
        })
        .collect()
}

/// A gate of `extension` on each gating event of `events`, on every call.
fn on_every_call(extension: &str, events: &[Event]) -> Vec<(String, Event, Option<Filter>)> {
    let mut gates: Vec<(String, Event, Option<Filter>)> = Vec::new();
    for event in events {
        if event.gates() && !gates.iter().any(|(_, held, _)| held == event) {
            gates.push((format!("{extension}/hook.{}", event.as_str()), *event, None));
        }
    }
    gates
}

/// The manifest of `source`'s `name` as it was found.
fn found_manifest<'a>(
    judging: &'a Judging<'_>,
    source: Source,
    name: &str,
) -> Option<&'a cineris_ext::manifest::Manifest> {
    let discovered = match source {
        Source::User => judging.user,
        Source::Project => judging.project,
    }?;
    discovered
        .found
        .iter()
        .find(|found| found.source == source && found.manifest.name == name)
        .map(|found| &found.manifest)
}

/// The hash `source`'s `name` is pinned at: the user's in their pins, a
/// trusted project's for the run or in its entry of the trust store.
fn pin_of(judging: &Judging<'_>, source: Source, name: &str) -> Option<ContentHash> {
    match source {
        Source::User => judging
            .pins
            .as_ref()
            .ok()?
            .get(&TrustResourceKind::Extension, name)
            .map(|pin| pin.hash),
        Source::Project if judging.project_trusted => judging
            .for_the_run
            .iter()
            .find(|run| run.kind == TrustResourceKind::Extension && run.name == name)
            .map(|run| run.hash)
            .or_else(|| {
                judging
                    .trust
                    .as_ref()
                    .ok()?
                    .pinned_of(&TrustResourceKind::Extension)
                    .into_iter()
                    .find(|(pinned, _)| pinned == name)
                    .map(|(_, hash)| hash)
            }),
        Source::Project => None,
    }
}

/// What takes an extension's gates away in a build without the script
/// engine, which cannot approve one again: turning it off, or the project's
/// trust; or a build that has the engine.
fn way_out(source: Source, name: &str) -> String {
    match source {
        Source::User => format!(
            "turn it off (`cineris ext disable {}`, which sets `[extensions.{}] enabled = false` \
             in your configuration, from the next session), or use a build of Cineris that has \
             the script engine",
            in_code(name),
            toml_key(&in_code(name))
        ),
        Source::Project => "take the project's trust away (`cineris permissions untrust`), or \
                            use a build of Cineris that has the script engine"
            .to_owned(),
    }
}

#[cfg(test)]
mod tests;
