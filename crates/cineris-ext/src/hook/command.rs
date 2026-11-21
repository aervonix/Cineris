//! A hook command (`docs/steps/28/answered.md`, "The processes", the hook
//! command's column), compiled from its `[hooks.<name>]` entry.
//!
//! - **Its program** is `command[0]`, named as a manifest's `exec` names
//!   one ([`Program::parse`]): a bare name, found by the core in `PATH`'s
//!   absolute directories outside the workspace and the write roots, or an
//!   absolute path.
//! - **Placeholders are whole arguments**, never part of one: `{path}`,
//!   `{tool}`, `{workspace}`, `{event}`. An argument that holds one and is
//!   not it (`--file={path}`) refuses the entry, so that what a call names
//!   never runs on into an option; so does a whole `{…}` that is not one of
//!   the four, and `{path}` or `{tool}` on an event that is not a tool's.
//! - **`{path}`** is filled with each file the call's plan reads or writes,
//!   resolved and in the system's plain form, one argument each, at most
//!   [`MAX_PATHS`]; for a call that names none (a shell command) the
//!   argument is dropped.
//! - **At most [`EXEC_MAX_ARGS`] arguments** of [`EXEC_MAX_ARG_BYTES`] each,
//!   written and filled.
//! - **Its time:** [`WAITED_DEFAULT`] for a hook that is waited for, at most
//!   [`WAITED_MOST`]; [`OBSERVER_DEFAULT`] for an observer, at most
//!   [`OBSERVER_MOST`].
//! - **A project's** is pinned by [`CommandHook::definition`], the hash of
//!   every part of it that the trust question shows
//!   ([`CommandHook::in_words`]).

use std::time::Duration;

use cineris_base::in_code;
use cineris_policy::trust::Definition;
use cineris_protocol::methods::TrustResourceKind;

use super::Filter;
use crate::event::Event;
use crate::host::{EXEC_MAX_ARG_BYTES, EXEC_MAX_ARGS};
use crate::manifest::Program;
use crate::registration::check_name;
use crate::source::Source;

/// The most files `{path}` is filled with: a call that names more fails
/// the hook.
pub const MAX_PATHS: usize = 64;

/// A waited-for hook's time when its entry gives none.
pub const WAITED_DEFAULT: Duration = Duration::from_secs(5);

/// The longest a hook that is waited for may run.
pub const WAITED_MOST: Duration = Duration::from_secs(60);

/// An observer's time when its entry gives none.
pub const OBSERVER_DEFAULT: Duration = Duration::from_secs(60);

/// The longest an observer may run.
pub const OBSERVER_MOST: Duration = Duration::from_secs(600);

/// The placeholders, as an entry writes them.
const PATH: &str = "{path}";
const TOOL: &str = "{tool}";
const WORKSPACE: &str = "{workspace}";
const EVENT: &str = "{event}";
const PLACEHOLDERS: [&str; 4] = [PATH, TOOL, WORKSPACE, EVENT];

/// A hook's entry, as the configuration holds it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    /// Its event, as written. `None`: the entry gives none.
    pub event: Option<String>,
    /// For a tool event: the tools it runs on.
    pub tools: Vec<String>,
    /// The program and its arguments.
    pub command: Vec<String>,
    /// Whether it only observes.
    pub observe: bool,
    /// Its time, when it gives one.
    pub timeout: Option<Duration>,
    /// Whether it may write the workspace where commands are confined.
    pub write: bool,
    /// Whether it may reach the network where commands are confined.
    pub network: bool,
}

/// One argument of a hook's command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// Given as written.
    Text(String),
    /// `{path}`: each file of the call's plan, one argument each.
    Path,
    /// `{tool}`: the tool the call is matched by.
    Tool,
    /// `{workspace}`: the workspace's root.
    Workspace,
    /// `{event}`: the event's name.
    Event,
}

impl Arg {
    /// The argument as the entry wrote it.
    #[must_use]
    pub fn as_written(&self) -> &str {
        match self {
            Self::Text(text) => text,
            Self::Path => PATH,
            Self::Tool => TOOL,
            Self::Workspace => WORKSPACE,
            Self::Event => EVENT,
        }
    }
}

/// What a hook's placeholders are filled with, for one event.
#[derive(Debug, Clone, Copy)]
pub struct Expansion<'a> {
    /// The tool the call is matched by, for a tool event: for a
    /// `tool_call`, the `source/tool` it names.
    pub tool: Option<&'a str>,
    /// Each file the call's plan reads or writes, resolved, in the
    /// system's plain form, each once.
    pub paths: &'a [String],
    /// The workspace's root, in the system's plain form.
    pub workspace: &'a str,
}

/// A hook command, compiled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandHook {
    /// Its name.
    pub name: String,
    /// Whose it is: the user's own configuration, or a project's.
    pub source: Source,
    /// The event it runs on.
    pub event: Event,
    /// For a tool event, the tools it runs on.
    pub filter: Filter,
    /// Its program.
    pub program: Program,
    /// Its arguments, the placeholders among them.
    pub args: Vec<Arg>,
    /// Whether it only observes.
    pub observe: bool,
    /// How long it may run.
    pub timeout: Duration,
    /// Whether it may write the workspace where commands are confined.
    pub write: bool,
    /// Whether it may reach the network where commands are confined.
    pub network: bool,
}

/// The event `entry` names, when it names one: what a refused entry is
/// still known by, so that one on a gating event fails closed.
#[must_use]
pub fn event_of(entry: &Entry) -> Option<Event> {
    entry.event.as_deref().and_then(Event::parse)
}

impl CommandHook {
    /// Compiles the entry `name` of `source`.
    ///
    /// # Errors
    ///
    /// Why it is refused, in a sentence: a name not by the registry's
    /// rules, no event, `tools` on an event that is not a tool's or with an
    /// entry that is not a tool, no command, a program that is not a bare
    /// name or an absolute path, a placeholder that is part of an argument
    /// or not one of the four or not of the event, more than
    /// [`EXEC_MAX_ARGS`] arguments or one longer than
    /// [`EXEC_MAX_ARG_BYTES`], or a time past its most.
    pub fn compile(name: &str, source: Source, entry: &Entry) -> Result<Self, String> {
        check_name(name).map_err(|_| {
            format!(
                "`{}` cannot be a hook's name: 1 to 64 of ASCII letters, digits, `_`, `-` and `.`",
                in_code(cineris_base::text::truncate_bytes(name, 80))
            )
        })?;
        let event = match entry.event.as_deref() {
            None => return Err("it names no `event`".to_owned()),
            Some(written) => Event::parse(written).ok_or_else(|| {
                format!(
                    "`{}` is not an event: {}",
                    in_code(cineris_base::text::truncate_bytes(written, 80)),
                    Event::ALL.map(Event::as_str).join(", ")
                )
            })?,
        };
        let tool_event = matches!(event, Event::ToolBefore | Event::ToolAfter);
        if !tool_event && !entry.tools.is_empty() {
            return Err(format!(
                "`tools` is for `tool.before` and `tool.after`, and its event is `{}`",
                event.as_str()
            ));
        }
        let filter = Filter::new(entry.tools.clone())?;
        let Some((program, rest)) = entry.command.split_first() else {
            return Err("its `command` is empty: it needs a program".to_owned());
        };
        if rest.len() > EXEC_MAX_ARGS {
            return Err(format!(
                "its `command` has {} arguments, and a program is given at most {EXEC_MAX_ARGS}",
                rest.len()
            ));
        }
        if PLACEHOLDERS
            .iter()
            .any(|placeholder| program.contains(placeholder))
        {
            return Err("its program is a placeholder: the first word is the program".to_owned());
        }
        let program = Program::parse(program.clone())
            .map_err(|(shown, why)| format!("its program is `{shown}`{why}"))?;
        let mut args = Vec::with_capacity(rest.len());
        for (index, written) in rest.iter().enumerate() {
            let number = index + 1;
            if written.len() > EXEC_MAX_ARG_BYTES {
                return Err(format!(
                    "its argument {number} is longer than {} KiB",
                    EXEC_MAX_ARG_BYTES / 1024
                ));
            }
            let arg = match written.as_str() {
                PATH => Arg::Path,
                TOOL => Arg::Tool,
                WORKSPACE => Arg::Workspace,
                EVENT => Arg::Event,
                _ => {
                    if let Some(placeholder) = PLACEHOLDERS
                        .iter()
                        .find(|placeholder| written.contains(*placeholder))
                    {
                        return Err(format!(
                            "its argument {number}, `{}`, holds `{placeholder}` as part of it: a \
                             placeholder is a whole argument, so that what a call names is never \
                             read as part of an option. Give it as an argument of its own",
                            in_code(cineris_base::text::truncate_bytes(written, 80))
                        ));
                    }
                    if is_placeholder_shaped(written) {
                        return Err(format!(
                            "its argument {number}, `{}`, is not a placeholder: they are {}",
                            in_code(cineris_base::text::truncate_bytes(written, 80)),
                            PLACEHOLDERS.join(", ")
                        ));
                    }
                    Arg::Text(written.clone())
                }
            };
            if matches!(arg, Arg::Path | Arg::Tool) && !tool_event {
                return Err(format!(
                    "its argument {number} is `{}`, which only a tool event fills, and its event \
                     is `{}`",
                    arg.as_written(),
                    event.as_str()
                ));
            }
            args.push(arg);
        }
        let (default, most) = if entry.observe {
            (OBSERVER_DEFAULT, OBSERVER_MOST)
        } else {
            (WAITED_DEFAULT, WAITED_MOST)
        };
        let timeout = entry.timeout.unwrap_or(default);
        if timeout.is_zero() || timeout > most {
            return Err(format!(
                "its `timeout` is {}, and a hook {} runs for at most {}",
                shown_time(timeout),
                if entry.observe {
                    "that observes"
                } else {
                    "that is waited for"
                },
                shown_time(most)
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            source,
            event,
            filter,
            program,
            args,
            observe: entry.observe,
            timeout,
            write: entry.write,
            network: entry.network,
        })
    }

    /// Whether it runs on a call of `tool` (`named`: for a `tool_call`, the
    /// `source/tool` it names). Always, for an event that is not a tool's.
    #[must_use]
    pub fn runs_on(&self, tool: &str, named: Option<&str>) -> bool {
        self.filter.matches(tool, named)
    }

    /// Its arguments, each placeholder filled from `with`.
    ///
    /// # Errors
    ///
    /// When `{path}` would be more than [`MAX_PATHS`] files, the arguments
    /// more than [`EXEC_MAX_ARGS`], or one of them longer than
    /// [`EXEC_MAX_ARG_BYTES`]; or a tool's placeholder has no tool to fill
    /// it.
    pub fn expand(&self, with: &Expansion<'_>) -> Result<Vec<String>, String> {
        let mut args = Vec::with_capacity(self.args.len());
        for arg in &self.args {
            match arg {
                Arg::Text(text) => args.push(text.clone()),
                Arg::Path => {
                    if with.paths.len() > MAX_PATHS {
                        return Err(format!(
                            "the call names {} files, and `{PATH}` is filled with at most \
                             {MAX_PATHS}",
                            with.paths.len()
                        ));
                    }
                    args.extend(with.paths.iter().cloned());
                }
                Arg::Tool => args.push(
                    with.tool
                        .ok_or_else(|| format!("`{TOOL}` has no tool to name"))?
                        .to_owned(),
                ),
                Arg::Workspace => args.push(with.workspace.to_owned()),
                Arg::Event => args.push(self.event.as_str().to_owned()),
            }
        }
        if args.len() > EXEC_MAX_ARGS {
            return Err(format!(
                "its arguments, filled, are {}, and a program is given at most {EXEC_MAX_ARGS}",
                args.len()
            ));
        }
        if args.iter().any(|arg| arg.len() > EXEC_MAX_ARG_BYTES) {
            return Err(format!(
                "an argument, filled, is longer than {} KiB",
                EXEC_MAX_ARG_BYTES / 1024
            ));
        }
        Ok(args)
    }

    /// The command as its entry wrote it, each word as code would show it
    /// if it held a space: for what the user is shown.
    #[must_use]
    pub fn written(&self) -> Vec<&str> {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(Arg::as_written))
            .collect()
    }

    /// The definition a project's hook is pinned by: its kind, its name,
    /// and the hash of every part of it that [`Self::in_words`] shows, in a
    /// canonical form: its event, its tools, its program and arguments as
    /// written, whether it observes, its time, `write` and `network`. Each
    /// list is counted first, so that no two entries hash alike.
    #[must_use]
    pub fn definition(&self) -> Definition {
        let tools = self.filter.entries();
        let written = self.written();
        let (tool_count, word_count) = (tools.len().to_string(), written.len().to_string());
        let millis = self.timeout.as_millis().to_string();
        let flag = |on: bool| if on { "true" } else { "false" };
        let mut parts: Vec<&str> = vec!["event", self.event.as_str(), "tools", &tool_count];
        parts.extend(tools.iter().map(String::as_str));
        parts.extend(["command", word_count.as_str()]);
        parts.extend(written.iter().copied());
        parts.extend([
            "observe",
            flag(self.observe),
            "timeout",
            &millis,
            "write",
            flag(self.write),
            "network",
            flag(self.network),
        ]);
        Definition::of(TrustResourceKind::Hook, self.name.clone(), parts)
    }

    /// What the trust question, and `cineris permissions trust`, say of it
    /// in words: the command, the event and what it sees, the tools, its
    /// time and whether it is waited for, and what its program may do,
    /// where commands are confined (`confined`) and where they are not.
    /// Every text of the entry's is shown as code ([`in_code`]).
    #[must_use]
    pub fn in_words(&self, confined: bool) -> String {
        let code = |text: &str| format!("`{}`", in_code(text));
        let command: Vec<String> = self.written().into_iter().map(&code).collect();
        let mut words = vec![format!(
            "It runs {} on `{}`, which sees {}.",
            command.join(" "),
            self.event.as_str(),
            self.event.sees()
        )];
        if matches!(self.event, Event::ToolBefore | Event::ToolAfter) {
            words.push(if self.filter.is_every() {
                "It runs for every tool.".to_owned()
            } else {
                let tools: Vec<String> = self
                    .filter
                    .entries()
                    .iter()
                    .map(|tool| code(tool))
                    .collect();
                format!("It runs for {}.", tools.join(", "))
            });
        }
        words.push(if self.observe {
            format!(
                "It only observes: it is not waited for, runs for at most {}, and what it answers \
                 is not read.",
                shown_time(self.timeout)
            )
        } else {
            format!(
                "It is waited for, for at most {}, and what it answers is used{}.",
                shown_time(self.timeout),
                if self.event.gates() {
                    "; if it fails, the event is refused"
                } else {
                    ""
                }
            )
        });
        words.push(if confined {
            format!(
                "Its program is confined: it reads what the session's commands may read, {}, and \
                 {}.",
                if self.write {
                    "writes the workspace, and what it writes is not recorded for undo"
                } else {
                    "writes nothing"
                },
                if self.network {
                    "may reach the network"
                } else {
                    "does not reach the network"
                }
            )
        } else {
            NOT_CONFINED.to_owned()
        });
        words.push(
            "Each start is decided by the permission rules as a command; one they would ask \
             about does not run."
                .to_owned(),
        );
        words.join(" ")
    }
}

/// The words for a hook whose program is not confined on this system
/// (Windows, no Landlock, the sandbox turned off): its `write` and
/// `network` bind nothing there.
pub const NOT_CONFINED: &str = "Its program is not confined on this system: it can do anything \
                                you can, including changing Cineris's own settings and approvals, \
                                whatever its `write` and `network` say.";

/// Whether `text` is shaped as a placeholder, `{…}` with a name in it.
fn is_placeholder_shaped(text: &str) -> bool {
    text.strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .is_some_and(|inner| {
            !inner.is_empty()
                && inner
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
}

/// A time as the user is shown it: whole seconds or minutes where it is
/// one, milliseconds otherwise.
fn shown_time(time: Duration) -> String {
    let millis = time.as_millis();
    if millis.is_multiple_of(60_000) && millis >= 60_000 {
        let minutes = millis / 60_000;
        format!("{minutes} minute{}", if minutes == 1 { "" } else { "s" })
    } else if millis.is_multiple_of(1000) {
        let seconds = millis / 1000;
        format!("{seconds} second{}", if seconds == 1 { "" } else { "s" })
    } else {
        format!("{millis} ms")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(event: &str, command: &[&str]) -> Entry {
        Entry {
            event: Some(event.to_owned()),
            command: command.iter().map(|word| (*word).to_owned()).collect(),
            ..Entry::default()
        }
    }

    fn compile(entry: &Entry) -> Result<CommandHook, String> {
        CommandHook::compile("fmt", Source::User, entry)
    }

    #[test]
    fn an_entry_compiles_with_its_placeholders_whole() {
        let hook = compile(&entry(
            "tool.after",
            &[
                "cargo",
                "fmt",
                "--",
                "{path}",
                "{tool}",
                "{workspace}",
                "{event}",
            ],
        ))
        .unwrap();
        assert_eq!(hook.program, Program::Name("cargo".to_owned()));
        assert_eq!(
            hook.args,
            [
                Arg::Text("fmt".to_owned()),
                Arg::Text("--".to_owned()),
                Arg::Path,
                Arg::Tool,
                Arg::Workspace,
                Arg::Event
            ]
        );
        assert_eq!(hook.timeout, WAITED_DEFAULT);
        let observer = compile(&Entry {
            observe: true,
            ..entry("turn.end", &["/usr/bin/logger"])
        })
        .unwrap();
        assert_eq!(
            observer.program,
            Program::Path("/usr/bin/logger".to_owned())
        );
        assert_eq!(observer.timeout, OBSERVER_DEFAULT);
    }

    #[test]
    fn a_placeholder_inside_an_argument_refuses_the_entry() {
        for inside in [
            "--file={path}",
            "{path}x",
            "a{tool}",
            "--at={workspace}",
            "{event}s",
        ] {
            let refused = compile(&entry("tool.before", &["fmt", inside])).unwrap_err();
            assert!(refused.contains("as part of it"), "{inside}: {refused}");
        }
        let unknown = compile(&entry("tool.before", &["fmt", "{file}"])).unwrap_err();
        assert!(unknown.contains("is not a placeholder"), "{unknown}");
        // Braces that are no placeholder's are text.
        let text = compile(&entry("tool.before", &["fmt", "--x={json}", "{}", "{a b}"])).unwrap();
        assert_eq!(text.args.len(), 3);
        let program = compile(&entry("tool.before", &["{path}"])).unwrap_err();
        assert!(program.contains("program is a placeholder"), "{program}");
    }

    #[test]
    fn a_tools_placeholders_and_filter_are_for_tool_events_only() {
        for (event, command) in [
            ("turn.end", &["log", "{path}"][..]),
            ("prompt.submit", &["log", "{tool}"][..]),
        ] {
            let refused = compile(&entry(event, command)).unwrap_err();
            assert!(refused.contains("only a tool event fills"), "{refused}");
        }
        let refused = compile(&Entry {
            tools: vec!["edit".to_owned()],
            ..entry("session.start", &["log"])
        })
        .unwrap_err();
        assert!(
            refused.contains("is for `tool.before` and `tool.after`"),
            "{refused}"
        );
        assert!(compile(&entry("turn.end", &["log", "{workspace}", "{event}"])).is_ok());
    }

    #[test]
    fn an_entry_without_an_event_a_program_or_a_good_name_is_refused() {
        let none = Entry {
            event: None,
            ..entry("x", &["log"])
        };
        assert!(compile(&none).unwrap_err().contains("names no `event`"));
        assert!(
            compile(&entry("tool.during", &["log"]))
                .unwrap_err()
                .contains("is not an event")
        );
        assert!(
            compile(&entry("tool.after", &[]))
                .unwrap_err()
                .contains("is empty")
        );
        for program in [
            "./fmt",
            "bin/fmt",
            "",
            " fmt",
            "C:fmt",
            "//server/share/fmt",
        ] {
            let refused = compile(&entry("tool.after", &[program])).unwrap_err();
            assert!(refused.contains("its program is"), "{program}: {refused}");
        }
        let named = CommandHook::compile("a b", Source::User, &entry("turn.end", &["log"]));
        assert!(named.unwrap_err().contains("cannot be a hook's name"));
    }

    #[test]
    fn its_time_is_held_to_its_kinds_most() {
        let waited = |seconds: u64, observe: bool| {
            compile(&Entry {
                timeout: Some(Duration::from_secs(seconds)),
                observe,
                ..entry("turn.end", &["log"])
            })
        };
        assert!(waited(60, false).is_ok());
        assert!(waited(61, false).unwrap_err().contains("at most 1 minute"));
        assert!(waited(600, true).is_ok());
        assert!(
            waited(601, true)
                .unwrap_err()
                .contains("at most 10 minutes")
        );
        assert!(waited(0, false).is_err());
    }

    #[test]
    fn the_arguments_are_capped_as_written_and_as_filled() {
        let mut command = vec!["log"; EXEC_MAX_ARGS + 2];
        command[0] = "log";
        let many = compile(&entry("tool.after", &command)).unwrap_err();
        assert!(many.contains("at most 256"), "{many}");
        let long = "a".repeat(EXEC_MAX_ARG_BYTES + 1);
        assert!(
            compile(&entry("tool.after", &["log", &long]))
                .unwrap_err()
                .contains("longer")
        );

        let hook = compile(&entry("tool.after", &["fmt", "--", "{path}"])).unwrap();
        let paths: Vec<String> = (0..MAX_PATHS).map(|index| format!("/w/{index}")).collect();
        fn with(paths: &[String]) -> Expansion<'_> {
            Expansion {
                tool: Some("edit"),
                paths,
                workspace: "/w",
            }
        }
        let filled = hook.expand(&with(&paths)).unwrap();
        assert_eq!(filled.len(), MAX_PATHS + 1);
        assert_eq!(filled[1], "/w/0", "one argument each");
        let more: Vec<String> = (0..=MAX_PATHS).map(|index| format!("/w/{index}")).collect();
        assert!(
            hook.expand(&with(&more))
                .unwrap_err()
                .contains("at most 64")
        );
        // A call that names no file drops the argument.
        assert_eq!(hook.expand(&with(&[])).unwrap(), ["--"]);

        let mut near = vec!["fmt"; EXEC_MAX_ARGS];
        near.push("{path}");
        let near = compile(&entry("tool.after", &near)).unwrap();
        let two = ["/w/a".to_owned(), "/w/b".to_owned()];
        assert!(near.expand(&with(&two)).unwrap_err().contains("filled"));
    }

    #[test]
    fn the_placeholders_are_filled_from_the_event() {
        let hook = compile(&entry(
            "tool.before",
            &["check", "{tool}", "{workspace}", "{event}", "{path}"],
        ))
        .unwrap();
        let paths = ["/w/a b.rs".to_owned()];
        let filled = hook
            .expand(&Expansion {
                tool: Some("tracker/find"),
                paths: &paths,
                workspace: "/w",
            })
            .unwrap();
        assert_eq!(filled, ["tracker/find", "/w", "tool.before", "/w/a b.rs"]);
    }

    #[test]
    fn its_pin_changes_with_everything_it_shows_and_nothing_else() {
        let base = Entry {
            tools: vec!["edit".to_owned()],
            ..entry("tool.after", &["fmt", "{path}"])
        };
        let pin = |entry: &Entry| compile(entry).unwrap().definition();
        let first = pin(&base);
        assert_eq!(first.kind, TrustResourceKind::Hook);
        assert_eq!(first.name, "fmt");
        assert_eq!(first, pin(&base.clone()));
        let changed = [
            Entry {
                event: Some("tool.before".to_owned()),
                ..base.clone()
            },
            Entry {
                tools: vec!["write".to_owned()],
                ..base.clone()
            },
            Entry {
                tools: Vec::new(),
                ..base.clone()
            },
            Entry {
                command: vec!["fmt".to_owned(), "--check".to_owned(), "{path}".to_owned()],
                ..base.clone()
            },
            Entry {
                observe: true,
                ..base.clone()
            },
            Entry {
                timeout: Some(Duration::from_secs(6)),
                ..base.clone()
            },
            Entry {
                write: true,
                ..base.clone()
            },
            Entry {
                network: true,
                ..base.clone()
            },
        ];
        for other in &changed {
            assert_ne!(pin(other).hash, first.hash, "{other:?}");
        }
        // A word moved from the tools to the command is another entry.
        let moved_a = Entry {
            tools: vec!["edit".to_owned(), "command".to_owned()],
            ..entry("tool.after", &["fmt"])
        };
        let moved_b = Entry {
            tools: vec!["edit".to_owned()],
            ..entry("tool.after", &["command", "fmt"])
        };
        assert_ne!(pin(&moved_a).hash, pin(&moved_b).hash);
        // Its time as written or by default is the same time.
        let default_time = Entry {
            timeout: Some(WAITED_DEFAULT),
            ..base.clone()
        };
        assert_eq!(pin(&default_time).hash, first.hash);
    }

    #[test]
    fn its_words_show_every_text_as_code() {
        let hook = compile(&Entry {
            tools: vec!["edit".to_owned()],
            write: true,
            ..entry("tool.after", &["fmt", "a`b\u{202e}", "{path}"])
        })
        .unwrap();
        let words = hook.in_words(true);
        assert!(words.contains("`fmt` `a\\`b"), "{words}");
        assert!(!words.contains('\u{202e}'), "{words}");
        assert!(words.contains("`{path}`"), "{words}");
        assert!(words.contains("for `edit`"), "{words}");
        assert!(words.contains("writes the workspace"), "{words}");
        assert!(words.contains("does not reach the network"), "{words}");
        assert!(words.contains("for at most 5 seconds"), "{words}");
        let open = hook.in_words(false);
        assert!(open.contains(NOT_CONFINED), "{open}");
    }
}
