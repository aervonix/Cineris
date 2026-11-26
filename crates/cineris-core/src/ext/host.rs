//! The session's side of an extension's host calls
//! (`docs/steps/28/answered.md`, "The host functions").
//!
//! `cineris_ext::host::serve` has already checked each call against the
//! manifest when it reaches a method here. Each method then has the call
//! decided by the session's permission engine as the same effect a
//! built-in tool would declare, beside an `ext` effect of the extension's
//! tool, so that a rule about the extension (`deny ext tracker/*`) covers
//! its host calls too. The call is decided under the extension tool's
//! name with the built-in tools' default: nothing else about the decision
//! differs from a built-in tool's. A question it needs offers only "allow
//! once" and "deny", and no answer to it keeps a rule. A write of a
//! protected path is refused before the engine is asked, at every level
//! (`super::protected`).
//!
//! **A hook's host calls** (the call's [`Phase::Hook`]): a hook decides by
//! its answer and never puts a question (`docs/steps/28/answered.md`,
//! points 2 and 6), so a call the engine would ask about is refused, naming
//! the rule that would allow it; so is one the engine would allow as only
//! reading where that reading may not be the program's. A hook may not ask
//! the user, nor write (`cineris_ext::host::serve` refuses both first).
//! Everything else is decided as a tool's host call is, the policy's stop
//! of the turn included.

use std::io::Read as _;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use cineris_agent::{Decision, PlannedCall, Policy as _, Verdict};

pub(crate) use crate::policy::HostQuestion;
use cineris_base::path::ResolvedPath;
use cineris_ext::host::{
    ExecOutput, ExecRequest, FS_MAX_BYTES, FetchOutput, FetchRequest, Host, HostError, HostFuture,
    Listing, ManifestGate, Phase, Scope,
};
use cineris_protocol::lease::RuleAction;
use cineris_tools::{
    Effect, FileSnapshot, FileTable, Json, LineRanges, NetworkTarget, NoOutput, Plan, RunCx, Shown,
    ToolSpec, run_blocking,
};
use serde_json::json;

use super::tools::HostParts;

/// The host for one call of an extension: its tool's call in a round.
pub(crate) struct CallHost<'a> {
    pub(crate) parts: &'a HostParts,
    /// The call's run context: its cancellation, the session's paths, and
    /// the call the loop is running.
    pub(crate) cx: &'a RunCx,
    /// The extension's own table of the files its script read.
    pub(crate) files: &'a std::sync::Arc<FileTable>,
    /// The extension tool's full name, `extension/tool`.
    pub(crate) full_name: &'a str,
    /// The extension's name.
    pub(crate) extension: &'a str,
    /// Where the extension comes from: with its name, it keys the values
    /// the extension keeps.
    pub(crate) source: cineris_ext::source::Source,
    /// What the script said it is doing, for the call's output.
    pub(crate) progress: &'a Mutex<Vec<String>>,
    /// Which call of the extension it is: a tool's, or a hook's, whose
    /// host calls put no question.
    pub(crate) phase: Phase,
    /// Set when a host call found its turn stopped already, by the
    /// policy's stop: a stop its call then ends with is not one it made.
    pub(crate) found_stopped: AtomicBool,
}

impl CallHost<'_> {
    /// Whether a host call found the call's turn stopped by someone else's
    /// stop: then a stop its call ends with is not its own.
    pub(crate) fn found_stopped(&self) -> bool {
        self.found_stopped.load(Ordering::SeqCst)
    }

    /// Decides one host call: the policy's answer, or the person's when it
    /// asks, as `how` says it is put. `Ok` when it may run. A hook's call
    /// is never put: where the engine would ask, it is refused.
    pub(crate) async fn decide(
        &self,
        summary: String,
        arguments: Json,
        effects: Vec<Effect>,
        preview: Option<&cineris_tools::ChangePreview>,
        how: HostQuestion,
    ) -> Result<(), HostError> {
        let Some(call) = &self.cx.call else {
            return Err(HostError::NotHere(
                "a host call is decided as part of a call of the loop, and this one has none"
                    .to_owned(),
            ));
        };
        let spec = ToolSpec::new(self.full_name, "", json!({"type": "object"}))
            .permission(RuleAction::Allow);
        // The call's own effects first: a question shows the first one (a
        // path, a host) as what it is about. The `ext` effect after them
        // puts the extension's rules to the call too.
        let summary = match self.asker() {
            Some(asker) => format!("{asker}: {summary}"),
            None => summary,
        };
        let mut plan = Plan::new(summary, ());
        plan.effects.extend(effects);
        plan.effects.push(Effect::Ext {
            tool: self.full_name.to_owned(),
        });
        let planned = PlannedCall {
            session: &call.session,
            turn: &call.turn,
            call_id: &call.call_id,
            item: &call.item,
            spec: &spec,
            arguments: &arguments,
            plan: &plan,
            preview,
            hooks: None,
        };
        // A hook decides by its answer: its call is decided once and never
        // put (`docs/steps/28/answered.md`, point 2): its turn's stop, then
        // the engine's outcome, a question refused, saying what would allow
        // it. A command's call is refused with the stop of the turn running
        // as it runs (its context's turn), made while its script runs, as a
        // hook's is with its turn's: its own ids are of no turn, which the
        // policy's look at the call's turn reads as none (the review of step
        // 28's commands). With no turn running as it runs (its context's
        // turn is 0), there is no stop to look at.
        if self.phase == Phase::Hook || (self.phase == Phase::Command && self.cx.turn != 0) {
            if let Some(reason) = self.parts.policy.stopped_for_hooks(self.cx.turn) {
                self.found_stopped.store(true, Ordering::SeqCst);
                return Err(HostError::Stopped(reason));
            }
            if self.phase == Phase::Hook {
                return for_a_hook(self.parts.policy.outcome(&planned), how.read_only_asks);
            }
        }
        match self.parts.policy.decide(&planned) {
            // What the session's shell calls read-only may not be the
            // program that runs: asked, as `how` says.
            Decision::AllowReadOnly if how.read_only_asks => {}
            Decision::Allow | Decision::AllowReadOnly => return Ok(()),
            Decision::Deny { reason } => {
                return Err(HostError::Denied(
                    reason.unwrap_or_else(|| "the permission rules refuse it".to_owned()),
                ));
            }
            Decision::Stop { reason } => {
                self.found_stopped.store(true, Ordering::SeqCst);
                return Err(HostError::Stopped(reason));
            }
            Decision::Ask => {}
            _ => return Err(HostError::Denied("it was not allowed".to_owned())),
        }
        match self.parts.policy.ask_with(planned, Some(how)).await {
            Verdict::Allow => Ok(()),
            Verdict::Deny { note } => Err(HostError::Denied(
                note.unwrap_or_else(|| "the user refused it".to_owned()),
            )),
            Verdict::Stop { reason } => {
                // The turn stops here, as it would have at a call of
                // the round's own: every call of it not yet started is
                // decided again and answered with the stop. (A stop
                // already in force is what this one says, if there was
                // one: its reason is the same.)
                // A command's runs outside any turn: its stop stops the
                // turn running as it runs, if one is (its context's turn),
                // and never turn 0, the number of what is of no turn; its
                // own stop is told and refuses the next prompt
                // (`crate::commands`).
                if self.phase != Phase::Command || self.cx.turn != 0 {
                    self.parts.policy.stop_turn(self.cx.turn, reason.clone());
                }
                Err(HostError::Stopped(reason))
            }
            _ => Err(HostError::Denied("it was not allowed".to_owned())),
        }
    }

    /// Who a command's question is from, as it says first: the command,
    /// and whose it is (since step 28's commands). `None` for any other
    /// call, whose tool names it.
    fn asker(&self) -> Option<String> {
        if self.phase != Phase::Command {
            return None;
        }
        let command = self.full_name.split_once("/command.").map_or_else(
            || self.full_name.to_owned(),
            |(ext, name)| format!("/{ext}:{name}"),
        );
        Some(format!(
            "The command `{}` of {} extension `{}`",
            cineris_base::in_code(&command),
            match self.source {
                cineris_ext::source::Source::User => "your",
                cineris_ext::source::Source::Project => "the project's",
            },
            cineris_base::in_code(self.extension)
        ))
    }

    /// Whose values the extension's `cineris.state` reads and writes.
    fn owner(&self) -> super::state::Owner {
        super::state::Owner::new(self.source, self.extension)
    }

    /// The call's context with the extension's own table of files: the `write` tool
    /// judges the extension's writes by what its script read.
    fn extension_cx(&self) -> RunCx {
        let mut cx = self.cx.clone();
        cx.files = std::sync::Arc::clone(self.files);
        cx
    }
}

impl Host for CallHost<'_> {
    fn resolve<'a>(&'a self, path: &'a str) -> HostFuture<'a, Result<ResolvedPath, HostError>> {
        Box::pin(async move {
            let paths = std::sync::Arc::clone(&self.cx.paths);
            let path = path.to_owned();
            run_blocking(&self.cx.cancel, move |_| paths.resolve(&path))
                .await
                .map_err(|error| HostError::Failed(error.to_string()))?
                .map_err(|error| HostError::Failed(error.to_string()))
        })
    }

    fn exec(&self, request: ExecRequest) -> HostFuture<'_, Result<ExecOutput, HostError>> {
        Box::pin(async move {
            super::exec::run(&self.parts.exec, request, self, &super::exec::Decided::Host).await
        })
    }

    fn read(
        &self,
        path: ResolvedPath,
        offset: u64,
        limit: Option<u64>,
    ) -> HostFuture<'_, Result<String, HostError>> {
        Box::pin(async move {
            self.decide(
                format!(
                    "{}: read `{}`",
                    self.full_name,
                    cineris_base::in_code(&path.data())
                ),
                json!({"path": path.data()}),
                vec![Effect::Read(path.clone())],
                None,
                HostQuestion::default(),
            )
            .await?;
            // The decision may have waited on a person: the path must still
            // lead where the manifest and the engine judged it to.
            self.still_leads(&path).await?;
            let at = path.to_fs_path();
            let bytes = run_blocking(&self.cx.cancel, move |_| read_capped(&at))
                .await
                .map_err(|error| HostError::Failed(error.to_string()))??;
            // A file with a NUL byte is binary, as the `read` tool judges,
            // however well it decodes.
            let text = String::from_utf8(bytes)
                .ok()
                .filter(|text| !text.contains('\0'))
                .ok_or_else(|| {
                    HostError::Failed(format!("`{}` is not text (UTF-8)", path.shown()))
                })?;
            // The script read it as it is: its writes to it are judged by
            // this, and by the lines it was given, which are what it saw.
            let snapshot = FileSnapshot::of_content(text.as_bytes(), None);
            let total = text.lines().count() as u64;
            let first = offset.saturating_add(1);
            let last = limit.map_or(total, |limit| offset.saturating_add(limit).min(total));
            let mut lines = LineRanges::new();
            if first <= last {
                lines.insert(first, last);
            }
            self.files.record_read(
                path.abs(),
                snapshot.clone(),
                Shown {
                    hash: snapshot.hash,
                    lines,
                    total_lines: total,
                    turn: self.cx.turn,
                    kept: false,
                },
            );
            let lines = text.split_inclusive('\n');
            let start = usize::try_from(offset).unwrap_or(usize::MAX);
            Ok(match limit {
                Some(limit) => lines
                    .skip(start)
                    .take(usize::try_from(limit).unwrap_or(usize::MAX))
                    .collect(),
                None => lines.skip(start).collect(),
            })
        })
    }

    fn write(&self, path: ResolvedPath, text: String) -> HostFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            // A protected path is refused at every level, `unrestricted`
            // included, before the engine is asked or the tool planned:
            // nothing looks at a host write afterwards, as the look at the
            // protected paths does after a command.
            self.refuse_protected(&path).await?;
            let arguments = json!({"path": path.data(), "content": text});
            let cx = self.extension_cx();
            let plan_cx = cx.plan_cx();
            let tool = std::sync::Arc::clone(&self.parts.write);
            let planned_arguments = arguments.clone();
            let plan = run_blocking(&cx.cancel, move |_| tool.plan(&planned_arguments, &plan_cx))
                .await
                .map_err(|error| HostError::Failed(error.to_string()))?
                .map_err(|error| HostError::Failed(error.to_string()))?;
            // The tool resolved the path again: it writes only where the
            // manifest judged.
            same_place(&plan.effects, &path)?;
            let preview = match self.parts.write.preview(&plan, &cx) {
                Some(work) => run_blocking(&cx.cancel, move |_| work()).await.ok(),
                None => None,
            };
            self.decide(
                format!(
                    "{}: write `{}`",
                    self.full_name,
                    cineris_base::in_code(&path.data())
                ),
                arguments,
                plan.effects.clone(),
                preview.as_ref(),
                HostQuestion::default(),
            )
            .await?;
            let result =
                cineris_tools::execute(self.parts.write.as_ref(), plan, &cx, &mut NoOutput).await;
            if result.is_error() {
                return Err(HostError::Failed(result.text()));
            }
            // The model's own table hears that the file changed under it.
            self.cx.files.note_changed(path.abs());
            Ok(())
        })
    }

    fn list(
        &self,
        path: ResolvedPath,
        glob: Option<String>,
        gate: ManifestGate,
    ) -> HostFuture<'_, Result<Listing, HostError>> {
        Box::pin(async move {
            let mut arguments = json!({"path": path.data()});
            if let Some(glob) = glob {
                arguments["glob"] = Json::String(glob);
            }
            self.through(&self.parts.list, arguments, &path, gate).await
        })
    }

    fn search(
        &self,
        pattern: String,
        path: ResolvedPath,
        glob: Option<String>,
        gate: ManifestGate,
    ) -> HostFuture<'_, Result<Listing, HostError>> {
        Box::pin(async move {
            let mut arguments = json!({"pattern": pattern, "path": path.data()});
            if let Some(glob) = glob {
                arguments["glob"] = Json::String(glob);
            }
            self.through(&self.parts.search, arguments, &path, gate)
                .await
        })
    }

    fn fetch(
        &self,
        request: FetchRequest,
        host: String,
        literal: Vec<String>,
    ) -> HostFuture<'_, Result<FetchOutput, HostError>> {
        Box::pin(async move {
            let method = cineris_llm::FetchMethod::parse(&request.method)
                .map_err(|refusal| HostError::Failed(refusal.to_string()))?;
            // Only the host itself, when the manifest names it literally,
            // may be a loopback or a private address; a name or a `*` never
            // reaches one.
            let mut allowance = cineris_llm::AddressAllowance::none();
            for name in &literal {
                allowance.add_host(name);
            }
            let timeout = request.timeout;
            let request = client_request(method, request, host.clone());
            // What the client would refuse before its lookup is refused
            // before anyone is asked about it, and before the question is
            // made of it.
            cineris_llm::fetch::check(&request, &allowance)
                .map_err(|refusal| HostError::Denied(refusal.to_string()))?;
            let shown = shown_url(&request.url);
            let sends = what_a_fetch_sends(&request);
            self.decide(
                format!(
                    "{}: {} `{}`",
                    self.full_name,
                    method.as_str(),
                    cineris_base::in_code(&shown)
                ),
                json!({"url": shown, "method": method.as_str()}),
                vec![Effect::Network(NetworkTarget::Host(host))],
                None,
                HostQuestion {
                    extra: sends,
                    ..HostQuestion::default()
                },
            )
            .await?;
            let response = self
                .parts
                .fetcher
                .fetch(request, &allowance, timeout, self.cx.cancel.clone())
                .await
                .map_err(|error| match error {
                    cineris_llm::FetchError::Refused(refusal) => {
                        HostError::Denied(refusal.to_string())
                    }
                    other => HostError::Failed(other.to_string()),
                })?;
            Ok(FetchOutput {
                status: response.status,
                headers: response.headers,
                body: response.body,
                cut: response.cut,
            })
        })
    }

    fn ask(
        &self,
        question: String,
        choices: Vec<String>,
    ) -> HostFuture<'_, Result<Option<String>, HostError>> {
        Box::pin(async move {
            let Some(call) = &self.cx.call else {
                return Ok(None);
            };
            Ok(self
                .parts
                .policy
                .put_question(
                    &call.session,
                    &call.turn,
                    match self.asker() {
                        Some(asker) => format!("{asker} asks: {question}"),
                        None => question,
                    },
                    choices,
                    self.extension,
                )
                .await)
        })
    }

    fn notify(&self, text: String) {
        (self.parts.tell)(format!("The extension `{}` says: {text}", self.extension));
    }

    fn progress(&self, text: String) {
        let mut progress = self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The latest is what the call is doing: earlier lines are kept, at
        // most a few.
        if progress.len() >= 20 {
            progress.remove(0);
        }
        progress.push(format!("{text}\n"));
    }

    fn state_get(
        &self,
        key: String,
        scope: Scope,
    ) -> HostFuture<'_, Result<Option<Json>, HostError>> {
        // No permission question: `state` touches only the extension's own
        // values, keyed by where it comes from and its name.
        Box::pin(async move { self.parts.state.get(self.owner(), key, scope).await })
    }

    fn state_set(
        &self,
        key: String,
        value: Option<Json>,
        scope: Scope,
    ) -> HostFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            // A command's run is of no turn: its record names none.
            let turn = self
                .cx
                .call
                .as_ref()
                .filter(|_| self.phase != Phase::Command)
                .map(|call| call.turn.clone());
            self.parts
                .state
                .set(self.owner(), key, value, scope, turn)
                .await
        })
    }

    fn log(&self, line: String) {
        tracing::info!(extension = self.extension, "{line}");
    }
}

impl CallHost<'_> {
    /// Runs the `list` or `search` tool for the script, its walk gated by
    /// the manifest over the session's own gate, after the policy allowed
    /// the walk.
    async fn through(
        &self,
        tool: &std::sync::Arc<dyn cineris_tools::Tool>,
        arguments: Json,
        checked: &ResolvedPath,
        gate: ManifestGate,
    ) -> Result<Listing, HostError> {
        let cx = self
            .cx
            .clone()
            .with_gate(gate.over(std::sync::Arc::clone(&self.cx.gate)));
        let plan_cx = cx.plan_cx();
        let planner = std::sync::Arc::clone(tool);
        let planned_arguments = arguments.clone();
        let plan = run_blocking(&cx.cancel, move |_| {
            planner.plan(&planned_arguments, &plan_cx)
        })
        .await
        .map_err(|error| HostError::Failed(error.to_string()))?
        .map_err(|error| HostError::Failed(error.to_string()))?;
        same_place(&plan.effects, checked)?;
        self.decide(
            format!("{}: {}", self.full_name, plan.summary),
            arguments,
            plan.effects.clone(),
            None,
            HostQuestion::default(),
        )
        .await?;
        let result = cineris_tools::execute(tool.as_ref(), plan, &cx, &mut NoOutput).await;
        if result.is_error() {
            return Err(HostError::Failed(result.text()));
        }
        Ok(Listing {
            text: result.text(),
            payload: result.payload().cloned().unwrap_or(Json::Null),
        })
    }
}

impl CallHost<'_> {
    /// Refuses a write of `path` when it is a protected path or lies under
    /// one (`super::protected`), at every level: the extension host's own
    /// check, as the manifest's is, made before the engine is asked.
    async fn refuse_protected(&self, path: &ResolvedPath) -> Result<(), HostError> {
        let protect = self
            .parts
            .policy
            .engine()
            .lease()
            .grant
            .sandbox
            .protect
            .clone();
        let paths = std::sync::Arc::clone(&self.cx.paths);
        let target = path.clone();
        let refused = run_blocking(&self.cx.cancel, move |_| {
            super::protected::refusal(&paths, &target, &protect)
        })
        .await
        .map_err(|error| HostError::Failed(error.to_string()))?;
        match refused {
            Some(why) => {
                // Nobody is asked, and the script may catch the error: the
                // log says it.
                tracing::warn!(
                    extension = self.extension,
                    path = %path.shown(),
                    "refused an extension's write of a protected path"
                );
                Err(HostError::Denied(why))
            }
            None => Ok(()),
        }
    }

    /// Whether `path` leads, now, where it led when it was checked.
    async fn still_leads(&self, path: &ResolvedPath) -> Result<(), HostError> {
        let again = self.resolve(&path.abs().to_native_string()).await?;
        if again.abs() == path.abs() {
            Ok(())
        } else {
            Err(HostError::Failed(format!(
                "`{}` changed while the call was being decided",
                path.shown()
            )))
        }
    }
}

/// What a hook's host call comes to, by the engine's `outcome` of it: it is
/// never put to anyone. Allowed, it runs; one the engine allows as only
/// reading runs too, unless that reading may not be the program's
/// (`read_only_asks`: another shell's own command or alias, or Windows' own
/// lookup), when it counts as a question; a question is refused, naming the
/// rules that would allow it (for a program, its `lines`), or why none can.
fn for_a_hook(outcome: cineris_policy::Outcome, read_only_asks: bool) -> Result<(), HostError> {
    let lead = "a hook may not put a question, and the permission engine would ask about this";
    let rule = |effect: &str, pattern: &str| {
        let pattern = serde_json::to_string(pattern).unwrap_or_default();
        format!(
            "`{}`",
            cineris_base::in_code(&format!(
                "effect = \"{effect}\", match = {pattern}, action = \"allow\""
            ))
        )
    };
    let refused = match outcome {
        cineris_policy::Outcome::Allow { read_only: false } => return Ok(()),
        cineris_policy::Outcome::Allow { read_only: true } if !read_only_asks => return Ok(()),
        // No rule can change it: the engine gives such a line its read-only
        // outcome whatever the rules say (the maintainers' call on the full
        // review of the script hooks, M2).
        cineris_policy::Outcome::Allow { read_only: true } => "a hook may not put a question, and \
                                                              on this system a hook cannot run a \
                                                              line the engine reads as only \
                                                              reading: the program that runs may \
                                                              be another than the one the line \
                                                              names (the shell's own command or \
                                                              alias, or the one Windows' own \
                                                              lookup finds). A hook command can \
                                                              run it: the user's own runs where \
                                                              the engine would ask"
            .to_owned(),
        cineris_policy::Outcome::Deny { reason } => return Err(HostError::Denied(reason)),
        cineris_policy::Outcome::Ask(question) if question.managed => format!(
            "{lead}: a rule of the managed configuration asks about it, for a person to decide \
             each time, so no hook can run it"
        ),
        cineris_policy::Outcome::Ask(question) => {
            let rules: Vec<String> = question
                .lifts
                .iter()
                .map(|lift| rule(lift.effect.as_str(), &lift.pattern))
                .collect();
            let reasons = cineris_base::visible(&question.reasons.join(" ")).into_owned();
            match rules.as_slice() {
                [] => format!("{lead} ({reasons}), and no permission rule can allow it unasked"),
                [one] => {
                    format!(
                        "{lead} ({reasons}). A permission rule that allows it lets it run: {one}"
                    )
                }
                more => format!(
                    "{lead} ({reasons}). The permission rules that allow it let it run: {}",
                    more.join(" and ")
                ),
            }
        }
    };
    Err(HostError::NotHere(refused))
}

/// Refuses a plan that touches a path other than `checked`, the one the
/// manifest judged: a tool resolves the path it is given again, and a
/// folder above it may have become a link in between (the review of step
/// 28).
fn same_place(effects: &[Effect], checked: &ResolvedPath) -> Result<(), HostError> {
    let elsewhere = effects.iter().any(|effect| match effect {
        Effect::Read(path) | Effect::ReadTree(path) | Effect::Write(path) => {
            path.abs() != checked.abs()
        }
        _ => false,
    });
    if elsewhere {
        Err(HostError::Failed(format!(
            "`{}` changed while the call was being made",
            checked.shown()
        )))
    } else {
        Ok(())
    }
}

/// A file's bytes, opened without following a link, at most the cap.
#[allow(
    clippy::disallowed_methods,
    reason = "reading a file blocks; it runs on the blocking pool"
)]
fn read_capped(path: &std::path::Path) -> Result<Vec<u8>, HostError> {
    let (file, _) = cineris_base::open_regular(path)
        .map_err(|error| HostError::Failed(format!("it cannot be read: {error}")))?;
    let mut bytes = Vec::new();
    file.take(FS_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| HostError::Failed(format!("it cannot be read: {error}")))?;
    if bytes.len() > FS_MAX_BYTES {
        return Err(HostError::Failed(
            "the file is larger than the 1 MiB `cineris.fs.read` gives".to_owned(),
        ));
    }
    Ok(bytes)
}

/// The client's request for a script's fetch: with the host the manifest
/// and the decision were about, so that the client refuses a URL it reads
/// as another.
fn client_request(
    method: cineris_llm::FetchMethod,
    request: FetchRequest,
    host: String,
) -> cineris_llm::FetchRequest {
    cineris_llm::FetchRequest {
        method,
        url: request.url,
        headers: request.headers,
        body: request.body,
        host: Some(host),
    }
}

/// The most of a fetch's URL before its query that its question shows,
/// and the most of its headers' names: 200 bytes each, as a program's input
/// is shown.
const FETCH_SHOWN_BYTES: usize = 200;

/// A fetch's URL as its question shows it: without its query or fragment,
/// which may carry a credential (`?…` or `#…` says one was there), cut at
/// [`FETCH_SHOWN_BYTES`] (`…` says it was), and with every character that
/// acts unseen made visible. Its path is shown, and a path may be a
/// credential too (a webhook's): the person is shown where it goes.
fn shown_url(url: &str) -> String {
    let (before, mark) = match url.find(['?', '#']) {
        Some(at) => (&url[..at], format!("{}…", &url[at..=at])),
        None => (url, String::new()),
    };
    let start = cineris_base::text::truncate_bytes(before, FETCH_SHOWN_BYTES);
    let cut = if start.len() < before.len() {
        "…"
    } else {
        ""
    };
    format!("{}{cut}{mark}", cineris_base::visible(start))
}

/// What a fetch sends besides where it goes, which no permission rule
/// judges: the size of its query and of its body, and its headers' count,
/// size and names (cut at [`FETCH_SHOWN_BYTES`]), never their values.
fn what_a_fetch_sends(request: &cineris_llm::FetchRequest) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(at) = request.url.find('?') {
        let query = request.url[at + 1..].split('#').next().unwrap_or_default();
        parts.push(format!("a query of {} bytes", query.len()));
    }
    if !request.body.is_empty() {
        parts.push(format!("a body of {} bytes", request.body.len()));
    }
    if !request.headers.is_empty() {
        let bytes: usize = request
            .headers
            .iter()
            .map(|(name, value)| name.len() + value.len())
            .sum();
        let names = request
            .headers
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let shown = cineris_base::text::truncate_bytes(&names, FETCH_SHOWN_BYTES);
        let more = if shown.len() < names.len() { "…" } else { "" };
        let count = match request.headers.len() {
            1 => "a header".to_owned(),
            count => format!("{count} headers"),
        };
        parts.push(format!(
            "{count} of {bytes} bytes ({}{more})",
            cineris_base::visible(shown)
        ));
    }
    let (last, rest) = parts.split_last()?;
    let listed = if rest.is_empty() {
        last.clone()
    } else {
        format!("{} and {last}", rest.join(", "))
    };
    Some(format!(
        "The fetch sends {listed}, which no permission rule judges."
    ))
}

#[cfg(test)]
mod tests {
    use cineris_base::path::{PathOptions, PathResolver};
    use cineris_testkit::Scratch;

    use super::*;

    fn a_fetch(url: &str) -> FetchRequest {
        FetchRequest {
            method: "GET".to_owned(),
            url: url.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout: std::time::Duration::from_secs(1),
        }
    }

    /// A hook's host call is never put: the engine's allow runs it; its
    /// allow as only reading runs it unless that reading may not be the
    /// program's (Windows, PowerShell, `cmd`), when it is refused, naming no
    /// rule, since none changes that outcome, and saying a hook command can
    /// run it; a question is refused, naming the rules the engine would keep,
    /// or the managed configuration's ask.
    #[test]
    fn a_hooks_host_call_is_never_put_and_a_read_only_line_another_shell_reads_is_refused() {
        use cineris_policy::{Outcome, Question};
        let question = |managed: bool| Question {
            reasons: vec!["The command is not known to be safe.".to_owned()],
            lifts: vec![cineris_protocol::lease::Rule {
                effect: cineris_protocol::lease::Effect::Exec,
                pattern: "npm test".to_owned(),
                action: RuleAction::Allow,
            }],
            storable: false,
            managed,
        };
        assert_eq!(
            for_a_hook(Outcome::Allow { read_only: false }, true),
            Ok(())
        );
        assert_eq!(
            for_a_hook(Outcome::Allow { read_only: true }, false),
            Ok(())
        );
        let Err(HostError::NotHere(read_only)) =
            for_a_hook(Outcome::Allow { read_only: true }, true)
        else {
            panic!("a line read as only reading in another shell ran in a hook");
        };
        assert!(
            read_only.contains("on this system a hook cannot run a line the engine reads as only")
                && read_only.contains("A hook command can run it")
                && !read_only.contains("rule"),
            "{read_only}"
        );
        let Err(HostError::NotHere(asked)) = for_a_hook(Outcome::Ask(question(false)), false)
        else {
            panic!("a question was not refused");
        };
        assert!(
            asked.contains("effect = \"exec\", match = \"npm test\""),
            "{asked}"
        );
        let Err(HostError::NotHere(managed)) = for_a_hook(Outcome::Ask(question(true)), false)
        else {
            panic!("a managed question was not refused");
        };
        assert!(managed.contains("managed configuration"), "{managed}");
        assert_eq!(
            for_a_hook(
                Outcome::Deny {
                    reason: "a rule".to_owned()
                },
                false
            ),
            Err(HostError::Denied("a rule".to_owned()))
        );
    }

    #[test]
    fn the_clients_request_carries_the_host_that_was_checked() {
        let request = FetchRequest {
            headers: vec![("x-a".to_owned(), "1".to_owned())],
            body: b"b".to_vec(),
            ..a_fetch("https://a.example/x")
        };
        let made = client_request(
            cineris_llm::FetchMethod::Get,
            request,
            "a.example".to_owned(),
        );
        assert_eq!(made.host.as_deref(), Some("a.example"));
        assert_eq!(made.url, "https://a.example/x");
        assert_eq!(made.headers, [("x-a".to_owned(), "1".to_owned())]);
        assert_eq!(made.body, b"b");
    }

    #[test]
    fn a_fetchs_url_is_shown_without_its_query_and_with_nothing_unseen() {
        assert_eq!(shown_url("https://a.example/x"), "https://a.example/x");
        assert_eq!(
            shown_url("https://a.example/x?token=s3cret"),
            "https://a.example/x?…"
        );
        assert_eq!(
            shown_url("https://a.example/x#s3cret"),
            "https://a.example/x#…"
        );
        assert!(!shown_url("https://a.example/\u{202e}x").contains('\u{202e}'));
    }

    #[test]
    fn what_a_fetch_sends_names_its_headers_and_never_their_values() {
        let get = cineris_llm::FetchRequest::get;
        assert_eq!(what_a_fetch_sends(&get("https://a.example/#part")), None);
        let sends = what_a_fetch_sends(&cineris_llm::FetchRequest {
            headers: vec![("authorization".to_owned(), "Bearer s3cret".to_owned())],
            body: b"{}".to_vec(),
            ..get("https://a.example/?token=s3cret#part")
        })
        .unwrap();
        assert_eq!(
            sends,
            "The fetch sends a query of 12 bytes, a body of 2 bytes and a header of 26 bytes \
             (authorization), which no permission rule judges."
        );
        let query = what_a_fetch_sends(&get("https://a.example/x?d=abc")).unwrap();
        assert!(query.contains("a query of 5 bytes"), "{query}");
    }

    #[test]
    fn what_a_fetchs_question_shows_is_bounded() {
        let long = format!("https://a.example/{}", "p".repeat(10 * 1024));
        let shown = shown_url(&long);
        assert!(shown.len() <= FETCH_SHOWN_BYTES + 4, "{}", shown.len());
        assert!(shown.ends_with('…'), "{shown}");
        let named = cineris_llm::FetchRequest {
            headers: (0..64)
                .map(|index| (format!("x-a-long-header-name-{index:02}"), "1".to_owned()))
                .collect(),
            ..cineris_llm::FetchRequest::get("https://a.example/")
        };
        let sends = what_a_fetch_sends(&named).unwrap();
        assert!(sends.contains("64 headers of"), "{sends}");
        assert!(sends.len() < FETCH_SHOWN_BYTES + 120, "{}", sends.len());
        assert!(sends.contains("…)"), "{sends}");
    }

    /// A tool's plan that touches a path other than the one the manifest
    /// judged is refused; the `ext` effect and the same path are not.
    #[test]
    fn a_plan_that_touches_another_path_than_the_one_checked_is_refused() {
        let scratch = Scratch::outside("cineris-core-unit", "ext_same_place");
        let paths = PathResolver::new(&scratch.dir("ws"), PathOptions::new()).unwrap();
        let checked = paths.resolve("out/a.txt").unwrap();
        let other = paths.resolve("out/b.txt").unwrap();
        let ext = Effect::Ext {
            tool: "probe/t".to_owned(),
        };
        assert!(same_place(&[Effect::Write(checked.clone()), ext.clone()], &checked).is_ok());
        for effect in [
            Effect::Write(other.clone()),
            Effect::Read(other.clone()),
            Effect::ReadTree(other),
        ] {
            assert!(same_place(&[ext.clone(), effect], &checked).is_err());
        }
    }
}
