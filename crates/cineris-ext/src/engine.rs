//! The script engine: QuickJS through `rquickjs` ([decision 0005]), behind
//! the `scripts` feature.
//!
//! A runtime is made with a memory limit, a stack limit and an interrupt
//! check, and a context with only the built-in objects a script needs: no
//! `Performance` (a high-resolution clock) and no `Proxy`. The binding adds
//! nothing of its own to the global object, and the C library's `std` and
//! `os` modules are not compiled.
//!
//! [decision 0005]: ../../../docs/decisions/0005-extension-mechanism.md

use rquickjs::context::intrinsic;
use rquickjs::{CatchResultExt, Context, Runtime};

/// The limits a runtime is made with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The most memory the runtime may allocate, in bytes.
    pub memory: usize,
    /// The most stack the script's calls may use, in bytes.
    pub stack: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory: 32 * 1024 * 1024,
            stack: 256 * 1024,
        }
    }
}

/// Why a script did not give a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    /// The engine could not be made: out of memory at the start.
    #[error("the script engine could not start: {0}")]
    Start(String),
    /// The script threw, or the engine stopped it (interrupted, out of
    /// memory, too deep).
    #[error("{0}")]
    Thrown(String),
}

/// One runtime and its one context.
pub struct Engine {
    // The context holds the runtime; both are dropped together.
    context: Context,
    _runtime: Runtime,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine").finish_non_exhaustive()
    }
}

type Builtins = (
    intrinsic::Date,
    intrinsic::Eval,
    intrinsic::RegExpCompiler,
    intrinsic::RegExp,
    intrinsic::Json,
    intrinsic::MapSet,
    intrinsic::TypedArrays,
    intrinsic::Promise,
    intrinsic::WeakRef,
);

impl Engine {
    /// A runtime with `limits`, stopped whenever `stop` answers true. The
    /// engine asks `stop` every few thousand operations.
    pub fn new(
        limits: Limits,
        stop: impl FnMut() -> bool + 'static,
    ) -> Result<Self, ScriptError> {
        let runtime = Runtime::new().map_err(|error| ScriptError::Start(error.to_string()))?;
        runtime.set_memory_limit(limits.memory);
        runtime.set_max_stack_size(limits.stack);
        runtime.set_interrupt_handler(Some(Box::new(stop)));
        let context = Context::builder()
            .with::<Builtins>()
            .build(&runtime)
            .map_err(|error| ScriptError::Start(error.to_string()))?;
        Ok(Self {
            context,
            _runtime: runtime,
        })
    }

    /// Evaluates `source` as a script and gives its value as JSON text
    /// (`undefined` for a value JSON has no form for).
    pub fn eval_json(&self, source: &str) -> Result<String, ScriptError> {
        self.context.with(|ctx| {
            let value = ctx
                .eval::<rquickjs::Value<'_>, _>(source)
                .catch(&ctx)
                .map_err(|error| ScriptError::Thrown(error.to_string()))?;
            let text = ctx
                .json_stringify(value)
                .catch(&ctx)
                .map_err(|error| ScriptError::Thrown(error.to_string()))?;
            Ok(match text {
                Some(text) => text
                    .to_string()
                    .map_err(|error| ScriptError::Thrown(error.to_string()))?,
                None => "undefined".to_owned(),
            })
        })
    }
}
