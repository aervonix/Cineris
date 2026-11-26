//! Hook commands in a session (`docs/steps/28/answered.md`, points 3 to 6).
//!
//! - **The user's and a project's are kept apart.** The user's own come
//!   from `settings.hooks` (the user's layers, merged as any setting's
//!   are); a project's from `Loaded::project_hooks`, which no merge ever
//!   joins to the user's. A project's hook with the name of one of the
//!   user's (ASCII case aside) is refused and shown: no project file can
//!   name, change or turn off a hook of the user's.
//! - **Each entry is compiled** (`cineris_ext::hook::CommandHook`): one that
//!   does not compile is refused with its reason, its event kept, so that a
//!   refused hook on a gating event fails closed where it would have run.
//! - **A project's waits for trust and is pinned** (kind `hook`, the hash
//!   of what is shown): the trust question lists each one not approved as
//!   it is now, in words ([`asked`]), a yes pins it as shown, and a session
//!   runs it only while the project is trusted and its pin is its hash
//!   ([`in_session`]).

use std::collections::HashMap;

use cineris_base::{in_code, visible};
use cineris_config::Loaded;
use cineris_config::schema::HookSettings;
use cineris_ext::event::Event;
use cineris_ext::hook::command::{Entry, event_of};
use cineris_ext::hook::{CommandHook, Filter};
use cineris_ext::source::Source;
use cineris_policy::{Definition, ProjectTrust, Standing};
use cineris_protocol::methods::{TrustResource, TrustResourceKind};

/// A hook entry that does not run, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Refused {
    /// Its name.
    pub(crate) name: String,
    /// Whose it is.
    pub(crate) source: Source,
    /// Its event, when it names one this build knows: a refused hook that
    /// would gate (`prompt.submit`, `tool.before`, not observing) counts as
    /// a failed one there.
    pub(crate) event: Option<Event>,
    /// Whether it only observes.
    pub(crate) observe: bool,
    /// The tools it is on, for a tool event: `None` when its `tools` could
    /// not be read, so that it is on every tool (failing closed).
    pub(crate) filter: Option<Filter>,
    /// Why, in a sentence that follows its name.
    pub(crate) reason: String,
}

impl Refused {
    /// What the session is told of it.
    pub(crate) fn told(&self) -> String {
        let name = in_code(&self.name);
        match self.source {
            Source::User => format!("Your hook `{name}` does not run: {}.", self.reason),
            Source::Project => {
                format!(
                    "This project's hook `{name}` does not run: {}.",
                    self.reason
                )
            }
        }
    }
}

/// The hook commands a configuration holds, compiled: the user's own and a
/// project's, apart.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Configured {
    /// The user's own, enabled, in name order.
    pub(crate) user: Vec<CommandHook>,
    /// The project's, enabled, in name order: each runs only once the
    /// project is trusted and the hook is approved as it is.
    pub(crate) project: Vec<CommandHook>,
    /// The entries that do not compile, or a project's that takes a name of
    /// the user's.
    pub(crate) refused: Vec<Refused>,
    /// What the configuration's check says once of an entry that compiles,
    /// with whose it is: a gating hook that relies on `{path}` for every
    /// tool (`CommandHook::warning`).
    pub(crate) warnings: Vec<(Source, String)>,
}

/// A hook's entry as `cineris-ext` compiles it.
fn entry_of(settings: &HookSettings) -> Entry {
    Entry {
        event: settings.event.map(|event| event.as_str().to_owned()),
        tools: settings.tools.clone(),
        command: settings.command.clone(),
        observe: settings.observe,
        timeout: settings.timeout,
        write: settings.write,
        network: settings.network,
    }
}

/// `settings`, the entry `name` of `source`, compiled; or why it is
/// refused, its event kept.
fn compiled(name: &str, source: Source, settings: &HookSettings) -> Result<CommandHook, Refused> {
    let entry = entry_of(settings);
    CommandHook::compile(name, source, &entry).map_err(|reason| Refused {
        name: name.to_owned(),
        source,
        event: event_of(&entry),
        observe: entry.observe,
        filter: Filter::new(entry.tools.clone()).ok(),
        reason,
    })
}

impl Configured {
    /// The hooks of `loaded`: the user's from its settings, the project's
    /// read apart. One turned off (`enabled = false`) is left out.
    pub(crate) fn of(loaded: &Loaded) -> Self {
        let mut configured = Self::default();
        for (name, settings) in &loaded.settings.hooks {
            if !settings.enabled {
                continue;
            }
            match compiled(name, Source::User, settings) {
                Ok(hook) => {
                    configured
                        .warnings
                        .extend(hook.warning().map(|said| (Source::User, said)));
                    configured.user.push(hook);
                }
                Err(refused) => configured.refused.push(refused),
            }
        }
        for hook in &loaded.project_hooks {
            if !hook.settings.enabled {
                continue;
            }
            // The name is the user's, whether their hook is on or off.
            let taken = loaded
                .settings
                .hooks
                .keys()
                .any(|mine| mine.eq_ignore_ascii_case(&hook.name));
            if taken {
                configured.refused.push(Refused {
                    name: hook.name.clone(),
                    source: Source::Project,
                    event: hook
                        .settings
                        .event
                        .and_then(|event| Event::parse(event.as_str())),
                    observe: hook.settings.observe,
                    filter: Filter::new(hook.settings.tools.clone()).ok(),
                    reason: "one of your own hooks has its name, and the name is yours: no \
                             project's file can name, change or turn off a hook of yours"
                        .to_owned(),
                });
                continue;
            }
            match compiled(&hook.name, Source::Project, &hook.settings) {
                Ok(compiled) => {
                    configured
                        .warnings
                        .extend(compiled.warning().map(|said| (Source::Project, said)));
                    configured.project.push(compiled);
                }
                Err(refused) => configured.refused.push(refused),
            }
        }
        configured
    }
}

/// What the trust question lists of a project's hooks, and the definitions
/// a yes pins.
#[derive(Debug, Default)]
pub(crate) struct Asked {
    /// One resource of kind `hook` each.
    pub(crate) resources: Vec<TrustResource>,
    /// The definitions listed, as they were read: a yes pins these.
    pub(crate) pins: Vec<Definition>,
}

/// What the trust question lists of `configured`'s project hooks, for a
/// project whose trust is `stored`: each one not approved as it is now, in
/// words, `changed` when it was approved as something else. `confined`:
/// whether a hook's program would be confined on this system.
pub(crate) fn asked(configured: &Configured, stored: &ProjectTrust, confined: bool) -> Asked {
    let mut asked = Asked::default();
    for hook in &configured.project {
        let definition = hook.definition();
        let changed = match stored.standing(&definition) {
            Standing::Approved => continue,
            Standing::Changed => true,
            _ => false,
        };
        asked.resources.push(TrustResource {
            kind: TrustResourceKind::Hook,
            name: visible(&hook.name).into_owned(),
            detail: detail(hook, changed, confined),
            changed,
        });
        asked.pins.push(definition);
    }
    asked
}

/// What the trust question, and `cineris permissions trust`, show of one
/// of a project's hooks: its name, whether it changed, and what it does in
/// words (`CommandHook::in_words`), every text of its entry as code.
pub(crate) fn detail(hook: &CommandHook, changed: bool, confined: bool) -> String {
    format!(
        "The project's hook `{}`.{} {}",
        in_code(&hook.name),
        if changed {
            " It has changed since you approved it."
        } else {
            ""
        },
        hook.in_words(confined)
    )
}

/// The hook commands a session runs, and what it is told of the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InSession {
    /// The user's own, then the project's approved ones.
    pub(crate) hooks: Vec<CommandHook>,
    /// Each of the user's entries that does not run because it was
    /// refused, and each hook of a trusted project approved before that
    /// has changed since: each is on its event as a hook that fails, so
    /// that a gate it held does not lapse. A project's entry that nobody
    /// approved holds no gate, trusted or not (the review of step 28's
    /// second pull request, H1: an untrusted repository's refused entry
    /// gated every call).
    pub(crate) refused: Vec<Refused>,
    /// What the session is told, once: each of the user's refused entries
    /// and warnings; and, of a trusted project alone, each refused entry,
    /// warning, and hook that is not approved as it is now.
    pub(crate) told: Vec<String>,
}

/// The hooks of `configured` that a session runs: the user's own, always;
/// a project's only while the project is trusted (`trusted`) and the hook
/// is approved as it is now, by the store's pin (`stored`) or by a yes the
/// store could not keep (`for_the_run`, by the hashes shown). Without the
/// store's reading nothing of a project's is approved.
pub(crate) fn in_session(
    configured: Configured,
    trusted: bool,
    stored: Option<&ProjectTrust>,
    for_the_run: &[Definition],
) -> InSession {
    // A project's entries are nothing to a session of a project not
    // trusted: none of them is on its event, and none is told (the review
    // of the second pull request, H1: an untrusted repository's refused
    // entry gated every call). The user's refused entries hold their events.
    let (mut refused, projects): (Vec<Refused>, Vec<Refused>) = configured
        .refused
        .into_iter()
        .partition(|refused| refused.source == Source::User);
    let mut told: Vec<String> = refused.iter().map(Refused::told).collect();
    told.extend(
        configured
            .warnings
            .iter()
            .filter(|(source, _)| *source == Source::User || trusted)
            .map(|(_, said)| said.clone()),
    );
    let mut hooks = configured.user;
    if trusted {
        // Told once, and on no event: an entry nobody approved holds no
        // gate. Only one approved as it was and changed since does (below).
        told.extend(projects.iter().map(Refused::told));
        for hook in configured.project {
            let definition = hook.definition();
            let standing = if for_the_run.contains(&definition) {
                Standing::Approved
            } else {
                match stored.map(|stored| stored.standing(&definition)) {
                    Some(Standing::Approved) => Standing::Approved,
                    _ if for_the_run.iter().any(|run| run.name == definition.name) => {
                        Standing::Changed
                    }
                    Some(other) => other,
                    None => Standing::New,
                }
            };
            let name = in_code(&hook.name);
            match standing {
                Standing::Approved => hooks.push(hook),
                Standing::Changed => {
                    told.push(format!(
                        "This project's hook `{name}` has changed since you approved it, so it \
                         does not run. `cineris permissions trust` shows it, to approve it."
                    ));
                    // Approved once, it held its event: it fails there now,
                    // as a changed extension's gating hook does.
                    refused.push(Refused {
                        name: hook.name.clone(),
                        source: Source::Project,
                        event: Some(hook.event),
                        observe: hook.observe,
                        filter: Some(hook.filter.clone()),
                        reason: "it has changed since you approved it: `cineris permissions \
                                 trust` shows it, to approve it"
                            .to_owned(),
                    });
                }
                _ => told.push(format!(
                    "This project's hook `{name}` does not run: you have not approved it. \
                     `cineris permissions trust` shows it, to approve it."
                )),
            }
        }
    }
    InSession {
        hooks,
        refused,
        told,
    }
}

/// Each hook of `definitions` on an event `allowed` lists (`None`: any), with
/// its place among them and its name: the extension's, `hook.` and the
/// event, and a number from the second on one event on.
pub(crate) fn named<'a>(
    extension: &str,
    definitions: &'a [cineris_ext::registration::HookDef],
    allowed: Option<&[Event]>,
) -> Vec<(usize, String, Event, &'a cineris_ext::registration::HookDef)> {
    let mut counts: HashMap<Event, usize> = HashMap::new();
    let mut each = Vec::new();
    for (index, definition) in definitions.iter().enumerate() {
        let Some(event) = Event::parse(&definition.event)
            .filter(|event| allowed.is_none_or(|allowed| allowed.contains(event)))
        else {
            continue;
        };
        let count = counts.entry(event).or_insert(0);
        *count += 1;
        let name = if *count == 1 {
            format!("{extension}/hook.{}", event.as_str())
        } else {
            format!("{extension}/hook.{}.{count}", event.as_str())
        };
        each.push((index, name, event, definition));
    }
    each
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use cineris_config::schema::HookEventName;
    use cineris_config::{LayerKind, Origin, ProjectHook};

    use super::*;

    fn settings(event: HookEventName, command: &[&str]) -> HookSettings {
        HookSettings {
            event: Some(event),
            command: command.iter().map(|word| (*word).to_owned()).collect(),
            ..HookSettings::default()
        }
    }

    fn project_hook(name: &str, settings: HookSettings) -> ProjectHook {
        ProjectHook {
            name: name.to_owned(),
            settings,
            origin: Origin {
                layer: LayerKind::Project,
                defined_in: None,
                file: Some(PathBuf::from("/w/.cineris/config.toml")),
                line: Some(1),
                detail: None,
            },
        }
    }

    fn loaded(user: &[(&str, HookSettings)], project: Vec<ProjectHook>) -> Loaded {
        let mut options = cineris_config::LoadOptions::new(cineris_config::Env::empty(
            cineris_config::Platform::CURRENT,
        ));
        // Nothing of the machine's is read: no managed file, no user's.
        options.dirs.managed_config = None;
        options.dirs.user_config = None;
        let mut loaded = cineris_config::load(&options).unwrap();
        for (name, settings) in user {
            loaded
                .settings
                .hooks
                .insert((*name).to_owned(), settings.clone());
        }
        loaded.project_hooks = project;
        loaded
    }

    /// The user's and a project's are apart: a project's with a name of
    /// the user's is refused and shown, whether the user's is on or off,
    /// in any case of the name; a project's of another name is kept.
    #[test]
    fn a_projects_hook_never_takes_a_name_of_the_users() {
        let fmt = settings(HookEventName::ToolAfter, &["fmt", "{path}"]);
        let off = HookSettings {
            enabled: false,
            ..settings(HookEventName::TurnEnd, &["log"])
        };
        let configured = Configured::of(&loaded(
            &[("fmt", fmt.clone()), ("audit", off)],
            vec![
                project_hook("FMT", settings(HookEventName::ToolBefore, &["deny-all"])),
                project_hook("audit", settings(HookEventName::TurnEnd, &["quiet"])),
                project_hook("lint", settings(HookEventName::ToolAfter, &["lint"])),
            ],
        ));
        let names = |hooks: &[CommandHook]| -> Vec<String> {
            hooks.iter().map(|hook| hook.name.clone()).collect()
        };
        assert_eq!(
            names(&configured.user),
            ["fmt"],
            "the user's off one is left out"
        );
        assert_eq!(names(&configured.project), ["lint"]);
        let refused: Vec<(&str, Source, bool)> = configured
            .refused
            .iter()
            .map(|refused| {
                (
                    refused.name.as_str(),
                    refused.source,
                    !refused.observe && refused.event.is_some_and(Event::gates),
                )
            })
            .collect();
        assert_eq!(
            refused,
            [
                ("FMT", Source::Project, true),
                ("audit", Source::Project, false)
            ]
        );
        assert!(configured.refused[0].told().contains("the name is yours"));
    }

    /// An entry that does not compile is refused with its reason and its
    /// event, so that a refused gate is known as one.
    #[test]
    fn an_entry_that_does_not_compile_is_refused_with_its_event() {
        let configured = Configured::of(&loaded(
            &[
                (
                    "gate",
                    settings(HookEventName::ToolBefore, &["check", "--file={path}"]),
                ),
                ("bare", HookSettings::default()),
            ],
            Vec::new(),
        ));
        assert!(configured.user.is_empty());
        let gate = configured
            .refused
            .iter()
            .find(|refused| refused.name == "gate")
            .unwrap();
        assert_eq!(gate.event, Some(Event::ToolBefore));
        assert!(gate.reason.contains("as part of it"), "{}", gate.reason);
        assert!(gate.told().starts_with("Your hook `gate` does not run"));
        let bare = configured
            .refused
            .iter()
            .find(|refused| refused.name == "bare")
            .unwrap();
        assert_eq!(bare.event, None);
        assert!(bare.reason.contains("names no `event`"), "{}", bare.reason);
    }

    fn one_project_hook() -> Configured {
        Configured::of(&loaded(
            &[],
            vec![project_hook(
                "lint",
                HookSettings {
                    tools: vec!["edit".to_owned()],
                    write: true,
                    ..settings(HookEventName::ToolAfter, &["lint", "`x`", "{path}"])
                },
            )],
        ))
    }

    /// A trusted project's entry that does not compile is told once and is
    /// on no event: nobody approved it, so it holds no gate (the review of
    /// step 28's second pull request, H1, and its re-check). An untrusted
    /// project's is not even told. The user's holds its event either way.
    #[test]
    fn a_projects_refused_entry_is_on_no_event_trusted_or_not() {
        let refused = settings(HookEventName::ToolBefore, &["check", "--file={path}"]);
        let configured = Configured::of(&loaded(
            &[("mine", refused.clone())],
            vec![project_hook("theirs", refused)],
        ));
        for trusted in [false, true] {
            let session = in_session(configured.clone(), trusted, None, &[]);
            let on: Vec<&str> = session
                .refused
                .iter()
                .map(|refused| refused.name.as_str())
                .collect();
            assert_eq!(on, ["mine"], "trusted: {trusted}");
            assert_eq!(
                session.told.iter().any(|told| told.contains("`theirs`")),
                trusted,
                "{:?}",
                session.told
            );
        }
    }

    /// The trust question lists a project's hook in words, every text as
    /// code, and pins it by the hash of what it shows; one approved as it
    /// is is not listed again, one changed says so.
    #[test]
    fn the_trust_question_lists_a_projects_hook_and_pins_it_as_shown() {
        let configured = one_project_hook();
        let fresh = asked(&configured, &ProjectTrust::untrusted(), true);
        assert_eq!(fresh.resources.len(), 1);
        let shown = &fresh.resources[0];
        assert_eq!(shown.kind, TrustResourceKind::Hook);
        assert_eq!(shown.name, "lint");
        assert!(!shown.changed);
        for said in [
            "The project's hook `lint`.",
            "`lint` `\\`x\\`` `{path}`",
            "on `tool.after`",
            "for `edit`",
            "writes the workspace",
            "does not reach the network",
        ] {
            assert!(shown.detail.contains(said), "{said:?} in {}", shown.detail);
        }
        assert_eq!(fresh.pins, [configured.project[0].definition()]);
        let open = asked(&configured, &ProjectTrust::untrusted(), false);
        assert!(
            open.resources[0]
                .detail
                .contains(cineris_ext::hook::command::NOT_CONFINED)
        );
    }

    /// A project's hook runs only while the project is trusted and the
    /// hook is approved as it is now: by the store's pin, or for the run by
    /// the hash a yes the store could not keep was given; another hash is
    /// a change.
    #[test]
    fn a_projects_hook_runs_only_trusted_and_approved_as_it_is() {
        let configured = one_project_hook();
        let user = Configured::of(&loaded(
            &[("mine", settings(HookEventName::TurnEnd, &["log"]))],
            Vec::new(),
        ));
        let definition = configured.project[0].definition();
        let names = |session: &InSession| -> Vec<String> {
            session.hooks.iter().map(|hook| hook.name.clone()).collect()
        };

        let untrusted = in_session(
            configured.clone(),
            false,
            None,
            std::slice::from_ref(&definition),
        );
        assert!(untrusted.hooks.is_empty(), "never without trust");
        assert!(untrusted.told.is_empty());

        let unread = in_session(configured.clone(), true, None, &[]);
        assert!(
            unread.hooks.is_empty(),
            "nothing approved without the store"
        );
        assert!(unread.told[0].contains("you have not approved it"));

        let for_the_run = in_session(
            configured.clone(),
            true,
            None,
            std::slice::from_ref(&definition),
        );
        assert_eq!(names(&for_the_run), ["lint"]);

        let other = Definition {
            hash: cineris_base::ContentHash::of(b"another"),
            ..definition.clone()
        };
        let changed = in_session(configured.clone(), true, None, &[other]);
        assert!(changed.hooks.is_empty());
        assert!(changed.told[0].contains("has changed since you approved it"));

        // The user's run whatever the project's trust.
        let mine = in_session(user, false, None, &[]);
        assert_eq!(names(&mine), ["mine"]);

        // By the store's pin: the hash shown is approved, another is a
        // change.
        let scratch = cineris_testkit::Scratch::outside("cineris-core-unit", "hooks_pinned");
        let clock: cineris_base::SharedClock = std::sync::Arc::new(
            cineris_testkit::FakeClock::starting_at("2026-11-21T10:00:00Z").unwrap(),
        );
        let key =
            cineris_base::PathResolver::new(&scratch.dir("work"), cineris_base::PathOptions::new())
                .unwrap()
                .root()
                .project_key();
        let store = |name: &str, pinned: &Definition| {
            let store = cineris_policy::TrustStore::new(scratch.join(name), clock.clone());
            store
                .trust(&key, "~/work", std::slice::from_ref(pinned), false)
                .unwrap();
            store.project(&key).unwrap()
        };
        let approved = store("approved.toml", &definition);
        let pinned = in_session(configured.clone(), true, Some(&approved), &[]);
        assert_eq!(names(&pinned), ["lint"]);
        let elsewhere = store(
            "changed.toml",
            &Definition {
                hash: cineris_base::ContentHash::of(b"another"),
                ..definition.clone()
            },
        );
        let changed = in_session(configured, true, Some(&elsewhere), &[]);
        assert!(changed.hooks.is_empty());
        assert!(changed.told[0].contains("has changed since you approved it"));
        // Approved once, it is on its event still, as a hook that fails.
        let [held] = changed.refused.as_slice() else {
            panic!("{:?}", changed.refused);
        };
        assert_eq!(held.name, "lint");
        assert!(held.reason.contains("changed since you approved it"));
    }
}
