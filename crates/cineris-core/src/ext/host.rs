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
//! once" and "deny", and no answer to it keeps a rule.

use std::io::Read as _;
use std::sync::Mutex;

use cineris_agent::{Decision, PlannedCall, Policy as _, Verdict};

pub(crate) use crate::policy::HostQuestion;
use cineris_base::path::ResolvedPath;
use cineris_ext::host::{
    ExecOutput, ExecRequest, FS_MAX_BYTES, Host, HostError, HostFuture, Listing, ManifestGate,
    Scope,
};
use cineris_protocol::lease::RuleAction;
use cineris_tools::{
    Effect, FileSnapshot, FileTable, Json, LineRanges, NoOutput, Plan, RunCx, Shown, ToolSpec,
    run_blocking,
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
    /// What the script said it is doing, for the call's output.
    pub(crate) progress: &'a Mutex<Vec<String>>,
}

impl CallHost<'_> {
    /// Decides one host call: the policy's answer, or the person's when it
    /// asks, as `how` says it is put. `Ok` when it may run.
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
        };
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
            Decision::Stop { reason } => return Err(HostError::Stopped(reason)),
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
                self.parts.policy.stop_turn(self.cx.turn, reason.clone());
                Err(HostError::Stopped(reason))
            }
            _ => Err(HostError::Denied("it was not allowed".to_owned())),
        }
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
        Box::pin(async move { super::exec::run(&self.parts.exec, request, self).await })
    }

    fn read(
        &self,
        path: ResolvedPath,
        offset: u64,
        limit: Option<u64>,
    ) -> HostFuture<'_, Result<String, HostError>> {
        Box::pin(async move {
            self.decide(
                format!("{}: read `{}`", self.full_name, path.shown()),
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
                format!("{}: write `{}`", self.full_name, path.shown()),
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
                .put_question(&call.session, &call.turn, question, choices, self.extension)
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
        _key: String,
        _scope: Scope,
    ) -> HostFuture<'_, Result<Option<Json>, HostError>> {
        Box::pin(async move {
            Err(HostError::Failed(
                "`cineris.state` is not available in this build yet".to_owned(),
            ))
        })
    }

    fn state_set(
        &self,
        _key: String,
        _value: Option<Json>,
        _scope: Scope,
    ) -> HostFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            Err(HostError::Failed(
                "`cineris.state` is not available in this build yet".to_owned(),
            ))
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

#[cfg(test)]
mod tests {
    use cineris_base::path::{PathOptions, PathResolver};
    use cineris_testkit::Scratch;

    use super::*;

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
