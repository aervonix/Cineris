//! An extension's tools in a session, run by the script engine.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cineris_base::path::AbsPath;
use cineris_base::{SharedClock, SharedTimer};
use cineris_ext::capability::Capabilities;
use cineris_ext::engine::{CallContext, CallError, ScriptRuntime, Source, Target};
use cineris_ext::host::Phase;
use cineris_ext::registration::ToolDef;
use cineris_protocol::lease::RuleAction;
use cineris_tools::{
    BoxFuture, ConcurrencyClass, FileTable, Json, Plan, PlanCx, RunCx, Tool, ToolError, ToolOutput,
    ToolRegistry, ToolResult, ToolSpec,
};

use super::ApprovedExtension;
use super::exec::ExecParts;
use super::host;
use crate::policy::EnginePolicy;

/// Why an extension's runtime could not start.
#[derive(Debug)]
pub(crate) struct Refused {
    /// Why, in a sentence that names the extension.
    why: String,
    /// What helps.
    stand: Refusal,
}

/// What helps a start that was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Approving it again, fixed, or turning it off.
    Plain,
    /// The engine refuses its hash for the rest of the process: a fix
    /// approved, turning it off, or a new process.
    Disabled,
    /// Too many extensions' threads were left running: this process starts
    /// none, so turning it off or a new process.
    NoneStart,
    /// Starts let go fill the cap: it clears within their evaluation time.
    LetGo,
}

impl Refused {
    /// A refusal of the start for `why`, not a disabled one.
    fn by(why: String) -> Self {
        Self {
            why,
            stand: Refusal::Plain,
        }
    }
}

/// What every call of a session's extensions is served with.
pub(crate) struct HostParts {
    pub(crate) policy: Arc<EnginePolicy>,
    pub(crate) timer: SharedTimer,
    /// The session's clock, which the timer moves.
    pub(crate) clock: SharedClock,
    pub(crate) workspace: AbsPath,
    pub(crate) write: Arc<dyn Tool>,
    pub(crate) list: Arc<dyn Tool>,
    pub(crate) search: Arc<dyn Tool>,
    pub(crate) exec: ExecParts,
    /// What `cineris.http.fetch` makes its requests with.
    pub(crate) fetcher: cineris_llm::Fetcher,
    /// Tells the session's front ends something: what `cineris.notify`
    /// says.
    pub(crate) tell: Arc<dyn Fn(String) + Send + Sync>,
    /// What the session's extensions keep: `cineris.state`.
    pub(crate) state: Arc<super::ExtState>,
    /// The project is trusted for the run by the command line
    /// (`--trust-project`): the way out of a project's extension's gate
    /// names no trust question, which is not shown then.
    pub(crate) trust_project: bool,
}

impl std::fmt::Debug for HostParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostParts")
            .field("workspace", &self.workspace)
            .finish_non_exhaustive()
    }
}

/// One approved extension in a session: its runtime, made on first use.
pub(crate) struct LoadedExtension {
    approved: ApprovedExtension,
    capabilities: Capabilities,
    /// The extension's own table of the files its script read, by which its
    /// writes are judged as the model's are by the model's: a file is
    /// overwritten only after the script read it as it is.
    files: Arc<FileTable>,
    runtime: tokio::sync::Mutex<Option<Arc<ScriptRuntime>>>,
    parts: Arc<HostParts>,
    /// Whether the session was told its start is refused while starts let
    /// go fill the cap: once.
    cap_told: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for LoadedExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedExtension")
            .field("name", &self.approved.manifest.name)
            .finish_non_exhaustive()
    }
}

impl LoadedExtension {
    /// The extension, ready for its tools to be registered; nothing of it
    /// runs yet.
    ///
    /// # Errors
    ///
    /// When its capabilities do not compile for the workspace.
    pub(crate) fn new(approved: ApprovedExtension, parts: Arc<HostParts>) -> Result<Self, String> {
        let capabilities = Capabilities::compile(&approved.manifest.capabilities, &parts.workspace)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            approved,
            capabilities,
            files: Arc::new(FileTable::new()),
            runtime: tokio::sync::Mutex::new(None),
            cap_told: std::sync::atomic::AtomicBool::new(false),

            parts,
        })
    }

    /// The extension's name.
    pub(crate) fn name(&self) -> &str {
        &self.approved.manifest.name
    }

    /// What the extension was approved as: its manifest, and what it
    /// registered then.
    pub(crate) fn approved(&self) -> &ApprovedExtension {
        &self.approved
    }

    /// What its calls are served with.
    pub(crate) fn parts(&self) -> &Arc<HostParts> {
        &self.parts
    }

    /// Runs the extension's hook `index`, called `name`, with `input`, as
    /// a call of its runtime's own queue (`docs/steps/28/answered.md`, "The
    /// runtime"): its time, `timed`, began when it was asked, so that its
    /// wait for its turn is within it, and a call still waiting when it is
    /// up is withdrawn before it starts. Its host calls are a hook's: they
    /// put no question ([`Phase::Hook`]).
    pub(crate) async fn call_hook(
        self: &Arc<Self>,
        index: usize,
        name: &str,
        input: Json,
        cx: &RunCx,
        (asked, time): (Duration, Duration),
    ) -> HookRan {
        // A run let go before it got here starts nothing, not even its
        // runtime.
        if cx.cancel.is_cancelled() {
            return HookRan::Failed("it was let go before it started".to_owned());
        }
        // A runtime that cannot start fails the hook, a gate closed, with
        // what approves or turns the extension off (point 5). Its start is
        // raced against the run's cancel: one let go meanwhile ends at once,
        // and the start goes on to its end on a task of its own (the early
        // review of the script hooks, L5). Nothing waits for that task, not
        // the session's close: it runs on to its own evaluation time, and
        // is left running and counted, its hash refused, only past that, as
        // any start is. A first evaluation answers no host call, so it does
        // nothing but evaluate; it holds neither the process's exit nor the
        // core's shutdown (the maintainers, on the third re-check of the
        // full review, L3).
        let (started, starting) = tokio::sync::oneshot::channel();
        let extension = Arc::clone(self);
        let watch = cineris_ext::engine::StartWatch::default();
        let watched = watch.clone();
        drop(tokio::spawn(async move {
            let _ = started.send(extension.runtime(&watched).await);
        }));
        let runtime = match cx.cancel.race(starting).await {
            Some(Ok(Ok(runtime))) => runtime,
            Some(Ok(Err(Refused { why, stand }))) => {
                return HookRan::Failed(format!("{why}; {}", self.remedy(stand)));
            }
            Some(Err(_)) => {
                return HookRan::Failed(format!(
                    "the extension `{}` could not start; {}",
                    cineris_base::in_code(self.name()),
                    self.remedy(Refusal::Plain)
                ));
            }
            None => {
                // Nobody waits for the start any more: while it evaluates it
                // counts toward the cap of threads left running, so that
                // starts let go cannot stack (the re-check of the script
                // hooks, (b)).
                watch.let_go();
                return HookRan::Failed(
                    "it was let go before its extension's runtime started".to_owned(),
                );
            }
        };
        let progress = Mutex::new(Vec::new());
        let host = host::CallHost {
            parts: &self.parts,
            cx,
            files: &self.files,
            full_name: name,
            extension: self.name(),
            source: self.approved.source,
            progress: &progress,
            phase: Phase::Hook,
            found_stopped: std::sync::atomic::AtomicBool::new(false),
        };
        let result = runtime
            .call(
                Target::Hook(index),
                input,
                CallContext {
                    host: &host,
                    capabilities: &self.capabilities,
                    time,
                    timer: self.parts.timer.as_ref(),
                    clock: self.parts.clock.as_ref(),
                    cancel: &cx.cancel,
                    asked: Some(asked),
                },
            )
            .await;
        match result {
            Ok(value) => HookRan::Answered(value),
            // Its own program's look at the protected paths stopped the
            // turn: a stop of its own, which the user is told of. One its
            // host call found already made is not.
            Err(CallError::Stopped(note)) if !host.found_stopped() => HookRan::Stopped(note),
            Err(CallError::Stopped(_)) => {
                HookRan::Failed("its turn was stopped, so nothing more of it ran".to_owned())
            }
            Err(CallError::TimedOut) => HookRan::Failed(format!(
                "it did not answer within its {} ms",
                time.as_millis()
            )),
            Err(CallError::Cancelled) => HookRan::Failed("it was let go".to_owned()),
            Err(CallError::Disabled) => HookRan::Failed(format!(
                "{}; {}",
                self.disabled(),
                self.remedy(Refusal::Disabled)
            )),
            Err(error) => HookRan::Failed(error.to_string()),
        }
    }

    /// Why the extension does not run in this process: a thread of it was
    /// left running.
    fn disabled(&self) -> String {
        format!(
            "The extension `{}` is not started in this process: a thread of it did not stop when \
             its time was up, and was left running",
            cineris_base::in_code(self.name())
        )
    }

    /// What approves the extension as it is now, or turns it off: for a
    /// hook of it that cannot run. `disabled`: the engine refuses it for the
    /// rest of the process (a thread of it was left running), so approving
    /// it again does not help (the full review of the script hooks, L5).
    fn remedy(&self, refusal: Refusal) -> String {
        let stand = match refusal {
            Refusal::Plain => super::script_hooks::Stand::Unapproved {
                trust_project: self.parts.trust_project,
            },
            Refusal::Disabled => super::script_hooks::Stand::Disabled,
            Refusal::NoneStart => super::script_hooks::Stand::NoneStart,
            Refusal::LetGo => super::script_hooks::Stand::LetGo,
        };
        super::script_hooks::way_out(self.approved.source, self.name(), None, stand)
    }

    /// The live runtime, made when there is none or the last was torn
    /// down.
    async fn runtime(
        &self,
        watch: &cineris_ext::engine::StartWatch,
    ) -> Result<Arc<ScriptRuntime>, Refused> {
        let mut slot = self.runtime.lock().await;
        // An extension whose thread was left running is not started again
        // in this process: the engine refuses it, for every session.
        if let Some(runtime) = slot.as_ref().filter(|runtime| runtime.is_alive()) {
            return Ok(Arc::clone(runtime));
        }
        // The folder is read where it was found: each name on the way to
        // it, looked at itself, is still a folder and no link (the full
        // review of step 28: a folder on the way swapped for a link would
        // have the read follow it, on Windows perhaps to a share).
        let folder = self.approved.folder.clone();
        let way = cineris_tools::run_blocking(&cineris_base::CancelToken::new(), move |_| {
            super::watched::linked_on_the_way(&folder)
        })
        .await;
        match way {
            Ok(None) => {}
            Ok(Some(linked)) => {
                return Err(Refused::by(format!(
                    "The extension `{}` could not start: `{}` on the way to its folder is no \
                     longer a folder of its own, and a link is not followed to it",
                    self.name(),
                    cineris_base::in_code(&linked.display().to_string())
                )));
            }
            Err(_) => {
                return Err(Refused::by(format!(
                    "The extension `{}` could not start: the way to its folder could not be \
                     looked at",
                    self.name()
                )));
            }
        }
        let runtime = ScriptRuntime::start_watched(
            self.name(),
            Source::Folder {
                path: self.approved.folder.clone(),
                pinned: self.approved.hash,
            },
            Some(self.approved.registration.clone()),
            self.parts.timer.as_ref(),
            watch,
        )
        .await
        .map_err(|error| match error {
            // Only a new hash or a new process starts it: said whole, as the
            // way out says it, not as the engine's sentence (the re-check of
            // the full review, L-c).
            cineris_ext::engine::LoadError::Disabled
            | cineris_ext::engine::LoadError::Abandoned => Refused {
                why: self.disabled(),
                stand: Refusal::Disabled,
            },
            cineris_ext::engine::LoadError::TooManyAbandoned(count) => Refused {
                why: format!(
                    "The extension `{}` could not start: {count} extension threads did not stop \
                     when told to and are still running, so this process starts no extension",
                    cineris_base::in_code(self.name())
                ),
                stand: Refusal::NoneStart,
            },
            // Starts let go fill the cap: refused, a gate failing closed,
            // until one of them ends, within its evaluation time; the user
            // is told once.
            cineris_ext::engine::LoadError::StartsLetGo(count) => {
                let why = format!(
                    "The extension `{}` could not start: {count} extension starts that closed \
                     sessions let go are still evaluating, so this process starts no extension \
                     until one of them ends, which it does within its evaluation time",
                    cineris_base::in_code(self.name())
                );
                if !self
                    .cap_told
                    .swap(true, std::sync::atomic::Ordering::SeqCst)
                {
                    (self.parts.tell)(format!(
                        "{why}. Its gating hooks refuse what they are on until then."
                    ));
                }
                Refused {
                    why,
                    stand: Refusal::LetGo,
                }
            }
            error => Refused::by(format!(
                "The extension `{}` could not start: {error}",
                self.name()
            )),
        })?;
        // The capabilities and the time were taken from the approved
        // manifest: the snapshot whose hash was checked must hold that one
        // (the review of step 28: the two are separate fields).
        if runtime.manifest() != &self.approved.manifest {
            return Err(Refused::by(format!(
                "The extension `{}` could not start: its manifest is not the one approved",
                self.name()
            )));
        }
        let runtime = Arc::new(runtime);
        *slot = Some(Arc::clone(&runtime));
        Ok(runtime)
    }

    /// Runs one of the extension's tools for a call of the loop.
    async fn call_tool(
        &self,
        tool: &str,
        arguments: Json,
        cx: &RunCx,
        out: &mut dyn ToolOutput,
    ) -> ToolResult {
        let runtime = match self
            .runtime(&cineris_ext::engine::StartWatch::default())
            .await
        {
            Ok(runtime) => runtime,
            Err(Refused {
                why,
                stand: Refusal::Plain,
            }) => return ToolResult::error(why),
            Err(Refused { why, stand }) => {
                return ToolResult::error(format!("{why}; {}", self.remedy(stand)));
            }
        };
        let full_name = format!("{}/{tool}", self.name());
        let progress = Mutex::new(Vec::new());

        let host = host::CallHost {
            parts: &self.parts,
            cx,
            files: &self.files,
            full_name: &full_name,
            extension: self.name(),
            source: self.approved.source,
            progress: &progress,
            phase: Phase::Tool,
            found_stopped: std::sync::atomic::AtomicBool::new(false),
        };
        let result = runtime
            .call(
                Target::Tool(tool.to_owned()),
                arguments,
                CallContext {
                    host: &host,
                    capabilities: &self.capabilities,
                    time: self.approved.manifest.limits.time,
                    timer: self.parts.timer.as_ref(),
                    clock: self.parts.clock.as_ref(),
                    cancel: &cx.cancel,
                    asked: None,
                },
            )
            .await;
        for line in progress
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            out.output(&line);
        }
        let label = format!(
            "Result of `{full_name}`, a tool of the extension `{}`, written by its author:",
            self.name()
        );
        match result {
            Ok(value) => {
                let text = match &value {
                    Json::String(text) => text.clone(),
                    Json::Null => "(no result)".to_owned(),
                    // Compact: indentation would grow a deeply nested
                    // result many times over before the output policy cut
                    // it.
                    other => serde_json::to_string(other).unwrap_or_default(),
                };
                ToolResult::success(format!("{label}\n{text}")).with_payload(value)
            }
            Err(CallError::Cancelled) => {
                ToolResult::interrupted(format!("`{full_name}` was interrupted."))
            }
            // A host call found the turn stopped, or stopped it: the
            // script's call ended there, and nothing more of it ran. The
            // result is marked as the stop, so that the round ends at this
            // call, even when it was the last of its response.
            Err(CallError::Stopped(reason)) => ToolResult::denied(format!(
                "`{full_name}` was stopped, and the turn with it. Reason: {reason}"
            ))
            .stopping_turn(reason),
            Err(error) => ToolResult::error(format!("`{full_name}` failed: {error}")),
        }
    }
}

impl LoadedExtension {
    /// Runs the extension's command `command` with `input` (`{args,
    /// arguments}`), outside any round, and gives what its script returned
    /// (since step 28's commands). Its host calls are a command's
    /// ([`Phase::Command`]): decided under `<extension>/command.<name>`, a
    /// question put where the engine asks, `cineris.fs.write` refused, its
    /// programs given no write root whatever the manifest's `write` says,
    /// and each refused with the stop of the turn running as it runs, once
    /// that turn is stopped. Its time is `time` (30 s, whatever the manifest
    /// gives its tools: `crate::commands::COMMAND_TIME`), at most
    /// `crate::commands::MAX_COMMAND_TIME`, from when it is asked: its wait
    /// for its runtime and the runtime's start count.
    ///
    /// # Errors
    ///
    /// Why it could not run, or how it failed, in a sentence that names it.
    pub(crate) async fn call_command(
        &self,
        command: &str,
        input: Json,
        cx: &RunCx,
        time: Duration,
    ) -> Result<Json, CommandFailed> {
        // Its time runs from now, its wait for its runtime's turn and the
        // runtime's start included, as a hook's does.
        let asked = self.parts.clock.monotonic();
        let time = time.min(crate::commands::MAX_COMMAND_TIME);
        let full_name = format!("{}/command.{command}", self.name());
        let shown = format!("/{}:{command}", self.name());
        let runtime = match self
            .runtime(&cineris_ext::engine::StartWatch::default())
            .await
        {
            Ok(runtime) => runtime,
            Err(Refused {
                why,
                stand: Refusal::Plain,
            }) => return Err(CommandFailed::Other(why)),
            Err(Refused { why, stand }) => {
                return Err(CommandFailed::Other(format!(
                    "{why}; {}",
                    self.remedy(stand)
                )));
            }
        };
        let progress = Mutex::new(Vec::new());
        let host = host::CallHost {
            parts: &self.parts,
            cx,
            files: &self.files,
            full_name: &full_name,
            extension: self.name(),
            source: self.approved.source,
            progress: &progress,
            phase: Phase::Command,
            found_stopped: std::sync::atomic::AtomicBool::new(false),
        };
        let result = runtime
            .call(
                Target::Command(command.to_owned()),
                input,
                CallContext {
                    host: &host,
                    capabilities: &self.capabilities,
                    time,
                    timer: self.parts.timer.as_ref(),
                    clock: self.parts.clock.as_ref(),
                    cancel: &cx.cancel,
                    asked: Some(asked),
                },
            )
            .await;
        match result {
            Ok(value) => Ok(value),
            Err(CallError::Cancelled) => {
                Err(CommandFailed::Other(format!("`{shown}` was interrupted")))
            }
            // A stop its host call found made, the running turn's: it ended
            // the script there, and is no stop of its own, as a hook's is
            // not: nothing says the command was stopped, and no prompt is
            // held back for it.
            Err(CallError::Stopped(reason)) if host.found_stopped() => {
                Err(CommandFailed::Other(format!(
                    "`{shown}` was ended by the running turn's stop, so nothing more of it ran: \
                     {reason}"
                )))
            }
            Err(CallError::Stopped(reason)) => Err(CommandFailed::Stopped(reason)),
            Err(CallError::TimedOut) => Err(CommandFailed::Other(format!(
                "`{shown}` did not end within its {} s",
                time.as_secs()
            ))),
            Err(CallError::Disabled) => Err(CommandFailed::Other(format!(
                "{}; {}",
                self.disabled(),
                self.remedy(Refusal::Disabled)
            ))),
            Err(error) => Err(CommandFailed::Other(format!("`{shown}` failed: {error}"))),
        }
    }
}

/// How a command's call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandFailed {
    /// A stop of its own: a program's look at the protected paths, or a
    /// question answered with a stop, with what the user is told.
    Stopped(String),
    /// Anything else, in a sentence that names it.
    Other(String),
}

/// What came of one call of an extension's hook.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HookRan {
    /// It returned this value: `null` for `undefined` and `null`.
    Answered(Json),
    /// Its own program's look at the protected paths stopped its turn, with
    /// what the user is told.
    Stopped(String),
    /// It could not run, threw, or was stopped, and why, in a clause that
    /// follows "the hook failed:".
    Failed(String),
}

/// One tool of an extension, as the registry holds it.
pub(crate) struct ExtensionTool {
    spec: ToolSpec,
    extension: Arc<LoadedExtension>,
}

impl std::fmt::Debug for ExtensionTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionTool")
            .field("name", &self.spec.name)
            .finish_non_exhaustive()
    }
}

impl ExtensionTool {
    /// A tool of `extension`, from what it registered.
    ///
    /// It asks unless a rule allows it, as every tool from outside does,
    /// and it **always runs alone**, whatever its definition says: its
    /// script may make a link, and a call beside it would have been
    /// decided before the link was there (`docs/steps/28/answered.md`,
    /// point 6). `readOnly` is shown to the user and decides nothing.
    fn new(definition: &ToolDef, extension: Arc<LoadedExtension>) -> Self {
        Self {
            spec: spec_of(definition),
            extension,
        }
    }
}

/// The spec of an extension's tool: its name, description and schema as
/// registered; it asks, and it runs alone, whatever it says of itself.
fn spec_of(definition: &ToolDef) -> ToolSpec {
    ToolSpec::new(
        definition.name.clone(),
        definition.description.clone(),
        definition.input.clone(),
    )
    .permission(RuleAction::Ask)
    .concurrency(ConcurrencyClass::Exclusive)
}

impl Tool for ExtensionTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn plan(&self, args: &Json, _cx: &PlanCx) -> Result<Plan, ToolError> {
        // What the script will do is not known until it runs: each host
        // call it makes is decided then, on its own. The call of the tool
        // itself is `tool_call`'s `ext` effect.
        Ok(Plan::new(
            format!("Run the extension's tool `{}`", self.spec.name),
            args.clone(),
        )
        .with_concurrency(cineris_tools::Concurrency::Exclusive))
    }

    fn run<'a>(
        &'a self,
        plan: Plan,
        cx: &'a RunCx,
        out: &'a mut dyn ToolOutput,
    ) -> BoxFuture<'a, ToolResult> {
        Box::pin(async move {
            let arguments = match plan.into_args::<Json>() {
                Ok(arguments) => arguments,
                Err(error) => return error.into_result(),
            };
            self.extension
                .call_tool(&self.spec.name, arguments, cx, out)
                .await
        })
    }

    fn preview(&self, _plan: &Plan, _cx: &RunCx) -> Option<cineris_tools::Preview> {
        // What it changes is asked about by each of its host calls.
        None
    }
}

/// Registers each approved extension's tools with `registry`, under the
/// extension's name, and puts the hooks it registered on their events in
/// `hooks`. Nothing runs. An extension whose capabilities do not compile,
/// or whose name the registry refuses, is left out and named in what is
/// returned, with what is told of it: the session holds its gates.
pub(crate) fn register(
    approved: Vec<ApprovedExtension>,
    parts: &Arc<HostParts>,
    registry: &ToolRegistry,
    hooks: &super::ScriptHooks,
) -> Vec<(String, String)> {
    let mut left_out = Vec::new();
    for extension in approved {
        let name = extension.manifest.name.clone();
        let description = extension.manifest.description.clone();
        let definitions = extension.registration.tools.clone();
        let loaded = match LoadedExtension::new(extension, Arc::clone(parts)) {
            Ok(loaded) => Arc::new(loaded),
            Err(error) => {
                left_out.push((
                    name.clone(),
                    format!("The extension `{name}` is not loaded: {error}"),
                ));
                continue;
            }
        };
        let tools: Vec<Arc<dyn Tool>> = definitions
            .iter()
            .map(|definition| {
                Arc::new(ExtensionTool::new(definition, Arc::clone(&loaded))) as Arc<dyn Tool>
            })
            .collect();
        match registry.set_source(&name, &description, tools) {
            // Its hooks are the extension's: on their events only while it
            // is loaded as it was approved.
            Ok(()) => hooks.add(&loaded),
            Err(error) => left_out.push((
                name.clone(),
                format!("The extension `{name}` is not loaded: {error}"),
            )),
        }
    }
    left_out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extensions_tool_asks_and_runs_alone_whatever_it_says_of_itself() {
        for read_only in [false, true] {
            let spec = spec_of(&ToolDef {
                name: "find".to_owned(),
                description: "Finds".to_owned(),
                input: serde_json::json!({"type": "object"}),
                read_only,
            });
            assert_eq!(spec.permission, RuleAction::Ask);
            assert_eq!(spec.concurrency, ConcurrencyClass::Exclusive);
        }
    }
}
