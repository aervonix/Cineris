//! An extension's tools in a session, run by the script engine.

use std::sync::{Arc, Mutex, PoisonError};

use cineris_base::SharedTimer;
use cineris_base::path::AbsPath;
use cineris_ext::capability::Capabilities;
use cineris_ext::engine::{CallContext, CallError, ScriptRuntime, Source, Target};
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

/// What every call of a session's extensions is served with.
pub(crate) struct HostParts {
    pub(crate) policy: Arc<EnginePolicy>,
    pub(crate) timer: SharedTimer,
    pub(crate) workspace: AbsPath,
    pub(crate) write: Arc<dyn Tool>,
    pub(crate) list: Arc<dyn Tool>,
    pub(crate) search: Arc<dyn Tool>,
    pub(crate) exec: ExecParts,
    /// Tells the session's front ends something: what `cineris.notify`
    /// says.
    pub(crate) tell: Arc<dyn Fn(String) + Send + Sync>,
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
            parts,
        })
    }

    fn name(&self) -> &str {
        &self.approved.manifest.name
    }

    /// The live runtime, made when there is none or the last was torn
    /// down.
    async fn runtime(&self) -> Result<Arc<ScriptRuntime>, String> {
        let mut slot = self.runtime.lock().await;
        if let Some(runtime) = slot.as_ref().filter(|runtime| runtime.is_alive()) {
            return Ok(Arc::clone(runtime));
        }
        let runtime = ScriptRuntime::start(
            self.name(),
            Source::Folder {
                path: self.approved.folder.clone(),
                pinned: self.approved.hash,
            },
            Some(self.approved.registration.clone()),
            self.parts.timer.as_ref(),
        )
        .await
        .map_err(|error| format!("The extension `{}` could not start: {error}", self.name()))?;
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
        let runtime = match self.runtime().await {
            Ok(runtime) => runtime,
            Err(error) => return ToolResult::error(error),
        };
        let full_name = format!("{}/{tool}", self.name());
        let progress = Mutex::new(Vec::new());
        let host = host::CallHost {
            parts: &self.parts,
            cx,
            files: &self.files,
            full_name: &full_name,
            extension: self.name(),
            progress: &progress,
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
                    cancel: &cx.cancel,
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
                    other => serde_json::to_string_pretty(other).unwrap_or_default(),
                };
                ToolResult::success(format!("{label}\n{text}")).with_payload(value)
            }
            Err(CallError::Cancelled) => {
                ToolResult::interrupted(format!("`{full_name}` was interrupted."))
            }
            Err(error) => ToolResult::error(format!("`{full_name}` failed: {error}")),
        }
    }
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
            spec: ToolSpec::new(
                definition.name.clone(),
                definition.description.clone(),
                definition.input.clone(),
            )
            .permission(RuleAction::Ask)
            .concurrency(ConcurrencyClass::Exclusive),
            extension,
        }
    }
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
/// extension's name. Nothing runs. An extension whose capabilities do not
/// compile, or whose name the registry refuses, is left out and named in
/// what is returned.
pub(crate) fn register(
    approved: Vec<ApprovedExtension>,
    parts: &Arc<HostParts>,
    registry: &ToolRegistry,
) -> Vec<String> {
    let mut left_out = Vec::new();
    for extension in approved {
        let name = extension.manifest.name.clone();
        let description = extension.manifest.description.clone();
        let definitions = extension.registration.tools.clone();
        let loaded = match LoadedExtension::new(extension, Arc::clone(parts)) {
            Ok(loaded) => Arc::new(loaded),
            Err(error) => {
                left_out.push(format!("The extension `{name}` is not loaded: {error}"));
                continue;
            }
        };
        let tools: Vec<Arc<dyn Tool>> = definitions
            .iter()
            .map(|definition| {
                Arc::new(ExtensionTool::new(definition, Arc::clone(&loaded))) as Arc<dyn Tool>
            })
            .collect();
        if let Err(error) = registry.set_source(&name, &description, tools) {
            left_out.push(format!("The extension `{name}` is not loaded: {error}"));
        }
    }
    left_out
}
