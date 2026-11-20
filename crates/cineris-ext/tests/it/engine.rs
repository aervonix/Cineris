//! The engine: a script runs and reaches nothing but the host functions,
//! registers only while it is first evaluated, and is stopped by its
//! limits without taking anything else down. No test waits: a call's time
//! is the fake timer's, moved on by the test.

use std::sync::Arc;
use std::time::Duration;

use cineris_base::path::{AbsPath, PathStyle};
use cineris_base::{CancelToken, FakeClock};
use cineris_ext::capability::Capabilities;
use cineris_ext::engine::{
    CallContext, CallError, Failure, Limit, LoadError, ScriptRuntime, Source, Target, register,
};
use cineris_ext::folder::Snapshot;
use cineris_ext::manifest::Manifest;
use cineris_testkit::{FakeTimer, block_on, join};
use serde_json::{Value as Json, json};

use crate::{FakeHost, extension, snapshot};

const TIME: Duration = Duration::from_secs(7);

fn timer() -> FakeTimer {
    FakeTimer::manual(&FakeClock::new(cineris_base::UtcTime::UNIX_EPOCH))
}

fn workspace() -> AbsPath {
    AbsPath::parse("/work/app", PathStyle::Posix).unwrap()
}

fn capabilities(snapshot: &Snapshot) -> Capabilities {
    let manifest = Manifest::of(snapshot).unwrap();
    Capabilities::compile(&manifest.capabilities, &workspace()).unwrap()
}

fn start(snapshot: &Arc<Snapshot>, timer: &FakeTimer) -> Result<ScriptRuntime, LoadError> {
    block_on(ScriptRuntime::start(
        "probe",
        Source::Snapshot(Arc::clone(snapshot)),
        None,
        timer,
    ))
}

/// A tool `t` whose `run` is `body`, with these capabilities.
fn tool(capabilities: &str, body: &str) -> Arc<Snapshot> {
    extension(
        capabilities,
        &format!("cineris.tool({{ name: \"t\", description: \"a tool\", run: {body} }});"),
    )
}

fn call(
    runtime: &ScriptRuntime,
    snapshot: &Snapshot,
    host: &FakeHost,
    timer: &FakeTimer,
    input: Json,
) -> Result<Json, CallError> {
    let capabilities = capabilities(snapshot);
    let cancel = CancelToken::new();
    block_on(runtime.call(
        Target::Tool("t".to_owned()),
        input,
        CallContext {
            host,
            capabilities: &capabilities,
            time: TIME,
            timer,
            clock: timer.clock(),
            cancel: &cancel,
        },
    ))
}

/// Runs the tool, moving the clock past its time once its time is
/// running: the call must end on its deadline, however it runs.
fn call_out_of_time(runtime: &ScriptRuntime, snapshot: &Snapshot, timer: &FakeTimer) -> CallError {
    let capabilities = capabilities(snapshot);
    let host = FakeHost::new();
    let cancel = CancelToken::new();
    let (result, ()) = block_on(join(
        runtime.call(
            Target::Tool("t".to_owned()),
            json!({}),
            CallContext {
                host: &host,
                capabilities: &capabilities,
                time: TIME,
                timer,
                clock: timer.clock(),
                cancel: &cancel,
            },
        ),
        async {
            timer.sleeping_for(TIME).await;
            timer.advance(TIME);
            // The grace given to the thread, should it need it.
            timer.advance(Duration::from_secs(5));
        },
    ));
    result.unwrap_err()
}

#[test]
fn a_tool_runs_with_its_arguments_and_gives_json() {
    let timer = timer();
    let snapshot = tool(
        "",
        "(args) => ({ doubled: args.x * 2, words: 'a b c'.split(' ') })",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    assert_eq!(runtime.registration().tools[0].name, "t");
    let result = call(
        &runtime,
        &snapshot,
        &FakeHost::new(),
        &timer,
        json!({"x": 21}),
    )
    .unwrap();
    assert_eq!(result, json!({"doubled": 42, "words": ["a", "b", "c"]}));
}

#[test]
fn an_async_tool_awaits_host_functions_the_manifest_allows() {
    let timer = timer();
    let snapshot = tool(
        "exec = [\"gh\"]",
        "async () => { const r = await cineris.exec('gh', ['issue', 'list']); \
         const kept = await cineris.state.set('k', {n: 1}); \
         return { code: r.code, out: r.stdout, err: r.stderr, kept, back: await cineris.state.get('k') } }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let result = call(&runtime, &snapshot, &host, &timer, json!({})).unwrap();
    assert_eq!(
        result,
        json!({"code": 0, "out": "out", "err": "err", "kept": null, "back": {"n": 1}})
    );
    assert_eq!(
        host.asked(),
        ["exec gh issue list network=false roots=0 timeout=7000"]
    );
}

#[test]
fn a_program_the_manifest_does_not_list_never_reaches_the_host() {
    let timer = timer();
    let snapshot = tool(
        "exec = [\"gh\"]",
        "async () => { try { await cineris.exec('curl', ['https://example.com']); return 'ran' } \
         catch (e) { return String(e.message) } }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let result = call(&runtime, &snapshot, &host, &timer, json!({})).unwrap();
    let text = result.as_str().unwrap();
    assert!(
        text.contains("capabilities.exec") && text.contains("curl"),
        "{text}"
    );
    assert!(host.asked().is_empty(), "{:?}", host.asked());
    // `gh` by another spelling is another program.
    let snapshot = tool(
        "exec = [\"gh\"]",
        "async () => { try { await cineris.exec('/usr/bin/gh', []); return 'ran' } \
         catch (e) { return String(e.message) } }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let result = call(&runtime, &snapshot, &host, &timer, json!({})).unwrap();
    assert!(result.as_str().unwrap().contains("capabilities.exec"));
    assert!(host.asked().is_empty());
}

#[test]
fn nothing_ambient_is_on_the_global_object() {
    let names = [
        "require",
        "process",
        "module",
        "exports",
        "std",
        "os",
        "print",
        "setTimeout",
        "setInterval",
        "setImmediate",
        "fetch",
        "XMLHttpRequest",
        "WebSocket",
        "Deno",
        "Bun",
        "performance",
        "Proxy",
        "scriptArgs",
        "gc",
        "navigator",
        "__cineris_host",
        "__cineris_register",
        "__loadScript",
    ];
    let timer = timer();
    let snapshot = tool(
        "",
        &format!(
            "() => [{}].map(([n, t]) => n + ':' + t)",
            names
                .iter()
                .map(|name| format!("['{name}', typeof globalThis['{name}']]"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let result = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap();
    for entry in result.as_array().unwrap() {
        let entry = entry.as_str().unwrap();
        assert!(entry.ends_with(":undefined"), "reachable: {entry}");
    }
    // What is there is the host functions and their names for `console`.
    let snapshot = tool(
        "",
        "() => [typeof cineris.exec, typeof console.log, Object.isFrozen(cineris), \
         Object.getOwnPropertyDescriptor(globalThis, 'cineris').configurable]",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let result = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap();
    assert_eq!(result, json!(["function", "function", true, false]));
}

#[test]
fn code_made_by_eval_and_by_function_reaches_no_more() {
    let timer = timer();
    let snapshot = tool(
        "",
        "() => [eval('typeof require'), new Function('return typeof process')(), \
         (0, eval)('typeof globalThis.__cineris_host')]",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let result = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap();
    assert_eq!(result, json!(["undefined", "undefined", "undefined"]));
}

#[test]
fn imports_load_only_the_extensions_own_files() {
    let timer = timer();
    let manifest = crate::manifest("");
    let files = [
        ("extension.toml", manifest.as_str()),
        (
            "main.js",
            "import { twice } from './lib/twice.js';\n\
             cineris.tool({ name: 't', run: async () => {\n\
               const outside = [];\n\
               for (const name of ['../outside.js', '/etc/passwd', 'fs', 'node:fs', \
                 'https://example.com/x.js', './missing.js', './lib/../../x.js']) {\n\
                 try { await import(name); outside.push(name + ': loaded') }\n\
                 catch (e) { outside.push(name + ': refused') }\n\
               }\n\
               const lib = await import('./lib/twice.js');\n\
               return { twice: twice(2), again: lib.twice(3), outside };\n\
             } });",
        ),
        ("lib/twice.js", "export const twice = (n) => n * 2;"),
    ];
    let snapshot = snapshot(&files);
    let runtime = start(&snapshot, &timer).unwrap();
    let result = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap();
    assert_eq!(result["twice"], 4);
    assert_eq!(result["again"], 6);
    for entry in result["outside"].as_array().unwrap() {
        assert!(entry.as_str().unwrap().ends_with(": refused"), "{entry}");
    }
}

#[test]
fn registering_after_the_first_evaluation_is_refused() {
    let timer = timer();
    let snapshot = tool(
        "",
        "() => { try { cineris.tool({ name: 'late', run: () => 1 }); return 'registered' } \
         catch (e) { return String(e.message) } }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let result = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap();
    assert!(
        result
            .as_str()
            .unwrap()
            .contains("only while its main module"),
        "{result}"
    );
}

#[test]
fn a_host_function_during_the_first_evaluation_refuses_the_extension() {
    let timer = timer();
    let snapshot = extension("exec = [\"gh\"]", "await cineris.exec('gh', []);");
    let error = start(&snapshot, &timer).unwrap_err();
    assert!(
        matches!(&error, LoadError::Evaluation(text) if text.contains("first evaluated")),
        "{error}"
    );
}

#[test]
fn a_hook_on_an_event_the_manifest_does_not_list_refuses_the_extension() {
    let timer = timer();
    let snapshot = extension(
        "hooks = [\"tool.after\"]",
        "cineris.on('tool.before', () => ({ decision: 'allow' }));",
    );
    let error = start(&snapshot, &timer).unwrap_err();
    assert!(
        matches!(&error, LoadError::Evaluation(text) if text.contains("capabilities.hooks")),
        "{error}"
    );
    // Listed, it registers.
    let snapshot = extension(
        "hooks = [\"tool.before\"]",
        "cineris.on('tool.before', { tools: ['write'] }, () => ({ decision: 'allow' }));",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    assert_eq!(runtime.registration().hooks[0].event, "tool.before");
    assert_eq!(runtime.registration().hooks[0].tools, ["write"]);
}

#[test]
fn a_registration_other_than_the_approved_one_refuses_the_extension() {
    let timer = timer();
    let snapshot = tool("", "() => 1");
    let (_, approved) = block_on(register(Arc::clone(&snapshot), &timer)).unwrap();
    let mut other = approved.clone();
    other.tools[0].description = "something else".to_owned();
    let error = block_on(ScriptRuntime::start(
        "probe",
        Source::Snapshot(Arc::clone(&snapshot)),
        Some(other),
        &timer,
    ))
    .unwrap_err();
    assert_eq!(error, LoadError::Mismatch);
    block_on(ScriptRuntime::start(
        "probe",
        Source::Snapshot(snapshot),
        Some(approved),
        &timer,
    ))
    .unwrap();
}

#[test]
fn an_endless_loop_is_stopped_at_its_time_and_only_its_runtime_goes() {
    let timer = timer();
    let snapshot = tool("", "() => { for (;;) {} }");
    let runtime = start(&snapshot, &timer).unwrap();
    assert_eq!(
        call_out_of_time(&runtime, &snapshot, &timer),
        CallError::TimedOut
    );
    assert!(!runtime.is_alive());
    // Another runtime of the same extension is made and runs.
    let snapshot = tool("", "() => 'fine'");
    let fresh = start(&snapshot, &timer).unwrap();
    assert_eq!(
        call(&fresh, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap(),
        json!("fine")
    );
}

#[test]
fn a_regular_expression_that_backtracks_without_end_is_stopped_too() {
    let timer = timer();
    let snapshot = tool("", "() => /^(a+)+$/.test('a'.repeat(40) + 'b')");
    let runtime = start(&snapshot, &timer).unwrap();
    assert_eq!(
        call_out_of_time(&runtime, &snapshot, &timer),
        CallError::TimedOut
    );
}

#[test]
fn a_memory_bomb_is_stopped_by_the_memory_limit() {
    let timer = timer();
    let snapshot = snapshot(&[
        (
            "extension.toml",
            "api = 1\nname = \"probe\"\nversion = \"1\"\nmain = \"main.js\"\n[limits]\nmemory = 8\n",
        ),
        (
            "main.js",
            "cineris.tool({ name: 't', run: () => { const a = []; for (;;) { a.push('x'.repeat(1 << 16) + a.length) } } });",
        ),
    ]);
    let runtime = start(&snapshot, &timer).unwrap();
    // The thread is held just after it told the call's end, before it
    // ends: whoever heard of the end already sees the runtime gone, and
    // hands it no other call.
    let hold = cineris_testkit::Hold::new(cineris_ext::engine::AFTER_A_CALLS_END_IS_TOLD);
    let error = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap_err();
    assert_eq!(error, CallError::Failed(Failure::Limit(Limit::Memory)));
    hold.reach();
    assert!(!runtime.is_alive());
    hold.release();
}

#[test]
fn the_memory_limit_is_the_one_the_manifest_sets() {
    let timer = timer();
    // About 16 MiB held at once: more than 8, less than 32.
    let script = "cineris.tool({ name: 't', run: () => { const a = []; \
                  for (let i = 0; i < 256; i++) { a.push('x'.repeat(1 << 16) + i) } return a.length } });";
    for (memory, fits) in [(8, false), (32, true)] {
        let snapshot = snapshot(&[
            (
                "extension.toml",
                &format!(
                    "api = 1\nname = \"probe\"\nversion = \"1\"\nmain = \"main.js\"\n[limits]\nmemory = {memory}\n"
                ),
            ),
            ("main.js", script),
        ]);
        let runtime = start(&snapshot, &timer).unwrap();
        let result = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({}));
        if fits {
            assert_eq!(result.unwrap(), json!(256));
        } else {
            assert_eq!(
                result.unwrap_err(),
                CallError::Failed(Failure::Limit(Limit::Memory))
            );
        }
    }
}

#[test]
fn endless_recursion_is_an_error_and_the_runtime_goes_on() {
    let timer = timer();
    let snapshot = extension(
        "",
        "function deeper(n) { return deeper(n + 1) + 1 }\n\
         cineris.tool({ name: 't', run: (args) => args.deep ? deeper(0) : 'fine' });",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let error = call(
        &runtime,
        &snapshot,
        &FakeHost::new(),
        &timer,
        json!({"deep": true}),
    )
    .unwrap_err();
    assert!(
        matches!(&error, CallError::Failed(Failure::Thrown(text)) if text.contains("stack")),
        "{error}"
    );
    assert!(runtime.is_alive());
    assert_eq!(
        call(
            &runtime,
            &snapshot,
            &FakeHost::new(),
            &timer,
            json!({"deep": false})
        )
        .unwrap(),
        json!("fine")
    );
}

#[test]
fn a_result_larger_than_its_cap_is_refused() {
    let timer = timer();
    let snapshot = tool("", "() => 'x'.repeat(2 * 1024 * 1024)");
    let runtime = start(&snapshot, &timer).unwrap();
    let error = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap_err();
    assert!(
        matches!(&error, CallError::Failed(Failure::Result(text)) if text.contains("1 MiB")),
        "{error}"
    );
    assert!(runtime.is_alive());
}

#[test]
fn a_script_cannot_block_its_thread_on_a_shared_wait() {
    let timer = timer();
    let snapshot = tool(
        "",
        "() => { try { Atomics.wait(new Int32Array(new ArrayBuffer(4)), 0, 0); return 'waited' } \
         catch (e) { return 'refused' } }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    assert_eq!(
        call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap(),
        json!("refused")
    );
}

#[test]
fn a_call_that_awaits_what_never_comes_ends_with_an_error() {
    let timer = timer();
    let snapshot = tool("", "() => new Promise(() => {})");
    let runtime = start(&snapshot, &timer).unwrap();
    let error = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap_err();
    assert!(
        matches!(&error, CallError::Failed(Failure::Thrown(text)) if text.contains("never comes")),
        "{error}"
    );
    assert!(runtime.is_alive());
}

#[test]
fn a_thrown_error_is_the_calls_error_with_its_message() {
    let timer = timer();
    let snapshot = tool("", "async () => { throw new Error('no issues found') }");
    let runtime = start(&snapshot, &timer).unwrap();
    let error = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap_err();
    assert!(
        matches!(&error, CallError::Failed(Failure::Thrown(text)) if text.contains("no issues found")),
        "{error}"
    );
    assert!(runtime.is_alive());
}

#[test]
fn a_cancelled_call_ends_and_takes_its_runtime_down() {
    let timer = timer();
    let snapshot = tool("", "() => { for (;;) {} }");
    let runtime = start(&snapshot, &timer).unwrap();
    let capabilities = capabilities(&snapshot);
    let host = FakeHost::new();
    let cancel = CancelToken::new();
    let (result, ()) = block_on(join(
        runtime.call(
            Target::Tool("t".to_owned()),
            json!({}),
            CallContext {
                host: &host,
                capabilities: &capabilities,
                time: TIME,
                timer: &timer,
                clock: timer.clock(),
                cancel: &cancel,
            },
        ),
        async {
            timer.sleeping_for(TIME).await;
            cancel.cancel();
            timer.advance(Duration::from_secs(5));
        },
    ));
    assert_eq!(result.unwrap_err(), CallError::Cancelled);
    assert!(!runtime.is_alive());
}

#[test]
fn a_changed_folder_is_never_evaluated() {
    let scratch = cineris_testkit::scratch!("a_changed_folder_is_never_evaluated");
    scratch.file("ext/extension.toml", crate::manifest(""));
    scratch.file("ext/main.js", "cineris.tool({ name: 't', run: () => 1 });");
    let pinned = Snapshot::read(&scratch.join("ext")).unwrap().hash();
    scratch.file("ext/main.js", "cineris.tool({ name: 't', run: () => 2 });");
    let timer = timer();
    let error = block_on(ScriptRuntime::start(
        "probe",
        Source::Folder {
            path: scratch.join("ext"),
            pinned,
        },
        None,
        &timer,
    ))
    .unwrap_err();
    assert!(matches!(error, LoadError::Changed { .. }), "{error}");
}

/// Reading a definition runs the script's code (a getter, `toJSON`), which
/// may call back into the engine while it registers: each way in ends
/// cleanly, and the process with it is never taken down (the review of
/// step 28: a call that came back in panicked the runtime's thread, and the
/// engine's own check as the runtime was freed then ended the process).
#[test]
fn a_definition_that_calls_back_in_while_it_is_read_is_served_cleanly() {
    let timer = timer();
    // A getter that registers: both are kept.
    let snapshot = extension(
        "",
        "cineris.tool({ get name() { cineris.tool({ name: 'inner', run() {} }); return 'outer' }, \
         run() {} });",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let names: Vec<_> = runtime
        .registration()
        .tools
        .iter()
        .map(|tool| tool.name.clone())
        .collect();
    assert_eq!(names, ["inner", "outer"]);
    // `toJSON` in the schema that registers: the same.
    let snapshot = extension(
        "",
        "cineris.tool({ name: 't', input: { toJSON() { cineris.tool({ name: 'u', run() {} }); \
         return { type: 'object' } } }, run() {} });",
    );
    assert_eq!(
        start(&snapshot, &timer).unwrap().registration().tools.len(),
        2
    );
    // A getter that makes a host call, which is refused while the module
    // is first evaluated: the extension is refused, and nothing more.
    let snapshot = extension(
        "",
        "cineris.tool({ name: 'a', run() {} });\n\
         cineris.tool({ get name() { cineris.log('x'); return 'b' }, run() {} });",
    );
    let error = start(&snapshot, &timer).unwrap_err();
    assert!(
        matches!(&error, LoadError::Evaluation(text) if text.contains("first evaluated")),
        "{error}"
    );
    // The thread was not taken down by a panic: another runtime starts.
    let snapshot = tool("", "() => 1");
    assert!(start(&snapshot, &timer).is_ok());
}

/// The engine runs the script's `Error.prepareStackTrace` as it makes an
/// error, its own included: the hook cannot be set, in the first
/// evaluation or in a call.
#[test]
fn the_stack_trace_hook_cannot_be_set() {
    let timer = timer();
    let snapshot = extension(
        "",
        "Error.prepareStackTrace = () => { cineris.log('x') };\n\
         cineris.tool({ name: 't', run() {} });",
    );
    let error = start(&snapshot, &timer).unwrap_err();
    assert!(matches!(&error, LoadError::Evaluation(_)), "{error}");
    let snapshot = tool(
        "",
        "() => { try { Error.prepareStackTrace = () => { cineris.tool({ name: 'late', run() {} }) } } \
         catch (e) { } \
         try { cineris.tool({ name: 'late', run() {} }) } catch (e) { return e.message } }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let result = call(&runtime, &snapshot, &FakeHost::new(), &timer, json!({})).unwrap();
    assert!(
        result
            .as_str()
            .unwrap()
            .contains("only while its main module"),
        "{result}"
    );
    assert!(runtime.is_alive());
}

/// The arguments of a host call are bounded before they are taken into the
/// harness's memory, which the runtime's limit does not count.
#[test]
fn a_host_calls_arguments_are_bounded_before_they_are_taken() {
    let timer = timer();
    let snapshot = tool(
        "",
        "() => { try { cineris.log('x'.repeat(100 * 1024)); return 'sent' } \
         catch (e) { return e.message } }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let result = call(&runtime, &snapshot, &host, &timer, json!({})).unwrap();
    assert!(result.as_str().unwrap().contains("more than"), "{result}");
    assert!(host.asked().is_empty());
}

/// The host calls a script has not had answered are bounded, so a script
/// cannot queue without end while one call waits (the review of step 28:
/// 8 MiB a turn of a loop, while a question waited); and the calls that
/// only tell are bounded by their counts in a call, which start again with
/// the next call.
#[test]
fn the_host_calls_waiting_for_answers_are_bounded() {
    let timer = timer();
    let snapshot = tool(
        "",
        "async () => { \
           const refused = []; \
           for (let i = 0; i < 40; i++) cineris.state.get('k' + i).catch((e) => refused.push(e.message)); \
           const notices = []; \
           for (let i = 0; i < 12; i++) cineris.notify('n' + i).catch((e) => notices.push(e.message)); \
           for (let i = 0; i < 120; i++) console.log('line ' + i); \
           await null; \
           return { refused: refused.length, first: refused[0], notices }; \
         }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    for _ in 0..2 {
        let host = FakeHost::new();
        let result = call(&runtime, &snapshot, &host, &timer, json!({})).unwrap();
        // Sixteen wait; the rest are refused at once.
        assert_eq!(result["refused"], 24, "{result}");
        assert!(
            result["first"].as_str().unwrap().contains("await them"),
            "{result}"
        );
        // Ten notices, a hundred lines of the log, in each call.
        assert_eq!(result["notices"].as_array().unwrap().len(), 2, "{result}");
        let asked = host.asked();
        let count = |kind: &str| asked.iter().filter(|noted| noted.starts_with(kind)).count();
        assert_eq!(count("notify "), 10);
        assert_eq!(count("log "), 100);
        assert!(runtime.is_alive());
    }
}

/// A host call that finds its turn stopped ends the script's call there:
/// nothing more of the script runs.
#[test]
fn a_host_call_in_a_stopped_turn_ends_the_scripts_call() {
    let timer = timer();
    let snapshot = tool(
        "exec = [\"gh\"]",
        "async () => { try { await cineris.exec('gh', []) } catch (e) { } \
         cineris.notify('went on'); return 'went on' }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost {
        exec_stops: true,
        ..FakeHost::new()
    };
    // No time passes: the thread, waiting for the answer, is woken and
    // ends, and its grace is never needed.
    let result = block_on(call_async(&runtime, &snapshot, &host, &timer));
    assert!(
        matches!(&result, Err(CallError::Stopped(reason)) if reason.contains("stopped the turn")),
        "{result:?}"
    );
    assert!(!runtime.is_alive());
    assert!(
        !host.asked().iter().any(|noted| noted.contains("went on")),
        "{:?}",
        host.asked()
    );
}

/// A program's time is what is left of the call's when it is asked for,
/// not the call's whole time.
#[test]
fn a_programs_time_is_what_the_call_has_left_when_it_starts() {
    let timer = timer();
    let snapshot = tool(
        "exec = [\"gh\"]",
        "async () => { await cineris.ask('go on?'); await cineris.exec('gh', ['x']); }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost {
        slow_ask: Some((timer.clone(), Duration::from_secs(3))),
        ..FakeHost::new()
    };
    let (result, ()) = block_on(join(
        call_async(&runtime, &snapshot, &host, &timer),
        async {
            timer.sleeping_for(Duration::from_secs(3)).await;
            timer.advance(Duration::from_secs(3));
        },
    ));
    result.unwrap();
    // Seven seconds, three of them spent on the question.
    assert_eq!(
        host.asked(),
        ["ask go on?", "exec gh x network=false roots=0 timeout=4000"]
    );
}

/// What a script registers is bounded before it is taken into the
/// harness's memory, which the runtime's limit does not count: a list's
/// length is read first (the review of step 28: four million filters, or
/// arguments, became four million strings).
#[test]
fn what_a_script_registers_is_bounded_before_it_is_taken() {
    let timer = timer();
    let refused = |capabilities: &str, script: &str, says: &str| {
        let error = start(&extension(capabilities, script), &timer).unwrap_err();
        assert!(
            matches!(&error, LoadError::Evaluation(text) if text.contains(says)),
            "{script}: {error}"
        );
    };
    refused(
        "",
        "cineris.command({ name: 'c', arguments: new Array(17).fill('a'), run() {} });",
        "lists at most 16",
    );
    refused(
        "",
        "cineris.command({ name: 'c', arguments: new Array(1e6).fill('a'), run() {} });",
        "lists at most 16",
    );
    refused(
        "hooks = [\"tool.before\"]",
        "cineris.on('tool.before', { tools: new Array(65).fill('write') }, () => ({}));",
        "lists at most 64",
    );
    refused(
        "hooks = [\"tool.before\"]",
        "cineris.on('tool.before', { tools: ['write\\u202e'] }, () => ({}));",
        "cannot be a tool in a hook's filter",
    );
    // At the caps, and as a registry names a tool, it registers.
    let fine = extension(
        "hooks = [\"tool.before\"]",
        "cineris.command({ name: 'c', arguments: new Array(16).fill('a'), run() {} });\n\
         cineris.on('tool.before', { tools: ['probe/run', 'mcp_x/*'] }, () => ({}));",
    );
    let runtime = start(&fine, &timer).unwrap();
    assert_eq!(runtime.registration().commands[0].arguments.len(), 16);
}

/// What a script registers is shown to the model and the user as written:
/// a description or a schema that holds a character acting unseen (a
/// direction override, a control) refuses the extension.
#[test]
fn text_a_script_registers_holds_nothing_that_acts_unseen() {
    let timer = timer();
    for script in [
        "cineris.tool({ name: 't', description: 'safe \\u202e efil', run() {} });",
        "cineris.command({ name: 'c', description: 'a\\u0007b', run() {} });",
        "cineris.command({ name: 'c', description: 'a\\ufe00\\ufe01\\ufe02', run() {} });",
        "cineris.tool({ name: 't', input: { type: 'object', description: 'x\\u200by' }, run() {} });",
        "cineris.tool({ name: 't', input: { type: 'object', properties: { 'a\\u202eb': { type: 'string' } } }, run() {} });",
    ] {
        let error = start(&extension("", script), &timer).unwrap_err();
        assert!(
            matches!(&error, LoadError::Evaluation(text) if text.contains("acts unseen")),
            "{script}: {error}"
        );
    }
    // A line break and a tab are text.
    let fine = extension(
        "",
        "cineris.tool({ name: 't', description: 'one\\n\\ttwo', run() {} });",
    );
    assert!(start(&fine, &timer).is_ok());
}

/// A native function of the engine that loops without looking at the
/// interrupt cannot be stopped: its thread is left running when its grace
/// is up, said so, and counted (the review of step 28).
#[test]
fn a_loop_the_interrupt_never_reaches_leaves_its_thread_and_is_counted() {
    let timer = timer();
    let snapshot = tool(
        "",
        "() => Array.prototype.reverse.call({ length: 2 ** 53 - 1 })",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let before = cineris_ext::engine::abandoned_threads();
    let host = FakeHost::new();
    let (result, ()) = block_on(join(
        call_async(&runtime, &snapshot, &host, &timer),
        async {
            // Its time, then the grace given to its thread, which does not
            // come back.
            timer.sleeping_for(TIME).await;
            timer.advance(TIME);
            timer.sleeping_for(cineris_ext::engine::GRACE).await;
            timer.advance(cineris_ext::engine::GRACE);
        },
    ));
    assert_eq!(result, Err(CallError::TimedOut));
    assert!(runtime.was_abandoned());
    assert!(!runtime.is_alive());
    assert_eq!(cineris_ext::engine::abandoned_threads(), before + 1);
    // The extension is not started again in this process, by any session:
    // its next start is refused at once, saying how it is started again.
    let refused = start(&snapshot, &timer).unwrap_err();
    assert_eq!(refused, LoadError::Disabled);
    assert!(refused.to_string().contains("approved again"), "{refused}");
    // Approved anew, it may be started again.
    cineris_ext::engine::forget_abandoned(&snapshot.hash());
    assert!(!cineris_ext::engine::is_disabled(&snapshot.hash()));
}

/// A main module that will not stop is left running too; past the cap of
/// threads left running, no extension is started at all, and why is said.
#[test]
fn past_the_cap_of_threads_left_running_no_extension_starts() {
    use cineris_ext::engine::{EVALUATION_TIME, GRACE, MAX_ABANDONED};

    let timer = timer();
    // Each extension a different one: one whose thread was left running
    // is not started again.
    let spinning = |index: usize| {
        extension(
            "",
            &format!(
                "// {index}\nArray.prototype.reverse.call({{ length: 2 ** 53 - 1 }});\n\
                 cineris.tool({{ name: 't', run() {{}} }});"
            ),
        )
    };
    for index in 0..MAX_ABANDONED {
        let spinning = spinning(index);
        let (started, ()) = block_on(join(
            ScriptRuntime::start(
                "probe",
                Source::Snapshot(Arc::clone(&spinning)),
                None,
                &timer,
            ),
            async {
                timer.sleeping_for(EVALUATION_TIME).await;
                timer.advance(EVALUATION_TIME);
                timer.sleeping_for(GRACE).await;
                timer.advance(GRACE);
            },
        ));
        assert!(matches!(started, Err(LoadError::Abandoned)), "{started:?}");
        assert!(cineris_ext::engine::is_disabled(&spinning.hash()));
    }
    let fine = tool("", "() => 1");
    let refused = start(&fine, &timer).unwrap_err();
    assert!(
        matches!(refused, LoadError::TooManyAbandoned(count) if count == MAX_ABANDONED),
        "{refused}"
    );
    assert!(refused.to_string().contains("restarts"), "{refused}");
}

/// A call cancelled while it waits its turn is withdrawn: it never runs,
/// and the call the runtime is serving goes on to its end (the reviews of
/// step 28: the cancel stopped the runtime under it, and a withdrawal told
/// by closing the channel could miss a start under way).
#[test]
fn a_call_cancelled_before_its_turn_is_withdrawn_and_never_runs() {
    let timer = timer();
    let snapshot = extension(
        "",
        "cineris.tool({ name: 'slow', run: async () => { await cineris.ask('go on?'); \
         return 'answered' } });\n\
         cineris.tool({ name: 'count', run: () => { globalThis.n = (globalThis.n || 0) + 1; \
         return globalThis.n } });",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost {
        slow_ask: Some((timer.clone(), Duration::from_secs(3))),
        ..FakeHost::new()
    };
    let capabilities = capabilities(&snapshot);
    let first_cancel = CancelToken::new();
    let second_cancel = CancelToken::new();
    let (first, ()) = block_on(join(
        call_named(
            &runtime,
            "slow",
            &host,
            &capabilities,
            &timer,
            &first_cancel,
        ),
        async {
            // Queued behind the first, then cancelled before its turn.
            let mut second = std::pin::pin!(call_named(
                &runtime,
                "count",
                &host,
                &capabilities,
                &timer,
                &second_cancel,
            ));
            let mut polled = false;
            std::future::poll_fn(|context| {
                if polled {
                    return std::task::Poll::Ready(());
                }
                polled = true;
                assert!(second.as_mut().poll(context).is_pending());
                context.waker().wake_by_ref();
                std::task::Poll::Pending
            })
            .await;
            second_cancel.cancel();
            assert_eq!(second.await, Err(CallError::Cancelled));
            timer.sleeping_for(Duration::from_secs(3)).await;
            timer.advance(Duration::from_secs(3));
        },
    ));
    assert_eq!(first, Ok(json!("answered")));
    assert!(runtime.is_alive());
    // The withdrawn call never ran: the next count is the first.
    let third = block_on(call_named(
        &runtime,
        "count",
        &host,
        &capabilities,
        &timer,
        &CancelToken::new(),
    ));
    assert_eq!(third, Ok(json!(1)));
}

/// A call whose future is dropped before it ends stops its runtime: the
/// script does not run on with nobody to stop it.
#[test]
fn a_call_given_up_before_its_end_stops_its_runtime() {
    let timer = timer();
    let snapshot = tool("", "() => { for (;;) {} }");
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let capabilities = capabilities(&snapshot);
    let cancel = CancelToken::new();
    block_on(async {
        let mut call = std::pin::pin!(runtime.call(
            Target::Tool("t".to_owned()),
            json!({}),
            CallContext {
                host: &host,
                capabilities: &capabilities,
                time: TIME,
                timer: &timer,
                clock: timer.clock(),
                cancel: &cancel,
            },
        ));
        // Polled until its time is running: it has started.
        std::future::poll_fn(|context| {
            let _ = call.as_mut().poll(context);
            if timer.running().contains(&TIME) {
                std::task::Poll::Ready(())
            } else {
                std::task::Poll::Pending
            }
        })
        .await;
    });
    assert!(!runtime.is_alive());
}

/// A program the script started is followed to its end when the call is
/// cancelled: what it changed is looked at and recorded after it stops,
/// which the host does at the end of its run (the review of step 28).
#[test]
fn a_cancelled_call_follows_its_program_to_the_end() {
    let timer = timer();
    let snapshot = tool(
        "exec = [\"gh\"]",
        "async () => { await cineris.exec('gh', []); return 'ran' }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost {
        slow_exec: Some((timer.clone(), Duration::from_secs(5))),
        ..FakeHost::new()
    };
    let capabilities = capabilities(&snapshot);
    let cancel = CancelToken::new();
    let (result, ()) = block_on(join(
        runtime.call(
            Target::Tool("t".to_owned()),
            json!({}),
            CallContext {
                host: &host,
                capabilities: &capabilities,
                time: TIME,
                timer: &timer,
                clock: timer.clock(),
                cancel: &cancel,
            },
        ),
        async {
            timer.sleeping_for(Duration::from_secs(5)).await;
            cancel.cancel();
            // The call sees the cancel while its program still runs: a few
            // turns of the other side, before the program ends.
            for _ in 0..8 {
                let mut turned = false;
                std::future::poll_fn(|context| {
                    if turned {
                        std::task::Poll::Ready(())
                    } else {
                        turned = true;
                        context.waker().wake_by_ref();
                        std::task::Poll::Pending
                    }
                })
                .await;
            }
            timer.advance(Duration::from_secs(5));
        },
    ));
    assert_eq!(result, Err(CallError::Cancelled));
    assert!(
        host.asked().iter().any(|noted| noted == "exec finished"),
        "{:?}",
        host.asked()
    );
}

/// A call cancelled just after the thread took it, before it said so, is
/// stopped as any started call is: its time's grace and the count of
/// threads left running apply, though the cancel came first (the review
/// of step 28's fixes: the withdrawal skipped the grace when the thread
/// won).
#[test]
fn a_call_the_thread_took_before_its_cancel_is_stopped_and_counted() {
    let timer = timer();
    let snapshot = tool(
        "",
        "() => Array.prototype.reverse.call({ length: 2 ** 53 - 1 })",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let capabilities = capabilities(&snapshot);
    let cancel = CancelToken::new();
    let before = cineris_ext::engine::abandoned_threads();
    let hold = cineris_testkit::Hold::new(cineris_ext::engine::AFTER_A_CALL_IS_CLAIMED);
    let (result, ()) = block_on(join(
        runtime.call(
            Target::Tool("t".to_owned()),
            json!({}),
            CallContext {
                host: &host,
                capabilities: &capabilities,
                time: TIME,
                timer: &timer,
                clock: timer.clock(),
                cancel: &cancel,
            },
        ),
        async {
            // The thread has taken the call and not said so yet.
            hold.reach();
            cancel.cancel();
            timer.sleeping_for(cineris_ext::engine::GRACE).await;
            hold.release();
            timer.advance(cineris_ext::engine::GRACE);
        },
    ));
    assert_eq!(result, Err(CallError::Cancelled));
    assert!(runtime.was_abandoned());
    assert_eq!(cineris_ext::engine::abandoned_threads(), before + 1);
}

/// A runtime of an extension whose other runtime left its thread running
/// takes no more calls: each live runtime could otherwise leave one more
/// (the review of step 28's fixes).
#[test]
fn a_live_runtime_of_a_disabled_extension_takes_no_more_calls() {
    let timer = timer();
    let snapshot = tool(
        "",
        "() => Array.prototype.reverse.call({ length: 2 ** 53 - 1 })",
    );
    let first = start(&snapshot, &timer).unwrap();
    let second = start(&snapshot, &timer).unwrap();
    let host = FakeHost::new();
    let (result, ()) = block_on(join(call_async(&first, &snapshot, &host, &timer), async {
        timer.sleeping_for(TIME).await;
        timer.advance(TIME);
        timer.sleeping_for(cineris_ext::engine::GRACE).await;
        timer.advance(cineris_ext::engine::GRACE);
    }));
    assert_eq!(result, Err(CallError::TimedOut));
    let before = cineris_ext::engine::abandoned_threads();
    assert_eq!(
        block_on(call_async(&second, &snapshot, &host, &timer)),
        Err(CallError::Disabled)
    );
    assert_eq!(cineris_ext::engine::abandoned_threads(), before);
}

/// A call whose future is given up while it waits its turn is withdrawn:
/// the call the runtime is serving goes on (the review of step 28's fixes:
/// it stopped the runtime under the running call).
#[test]
fn a_call_given_up_before_its_turn_leaves_the_running_call_alone() {
    let timer = timer();
    let snapshot = tool(
        "",
        "async () => { await cineris.ask('go on?'); return 'answered' }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost {
        slow_ask: Some((timer.clone(), Duration::from_secs(3))),
        ..FakeHost::new()
    };
    let capabilities = capabilities(&snapshot);
    let cancel = CancelToken::new();
    let (first, ()) = block_on(join(
        call_async(&runtime, &snapshot, &host, &timer),
        async {
            {
                let mut second = std::pin::pin!(runtime.call(
                    Target::Tool("t".to_owned()),
                    json!({}),
                    CallContext {
                        host: &host,
                        capabilities: &capabilities,
                        time: TIME,
                        timer: &timer,
                        clock: timer.clock(),
                        cancel: &cancel,
                    },
                ));
                let mut polled = false;
                std::future::poll_fn(|context| {
                    if polled {
                        return std::task::Poll::Ready(());
                    }
                    polled = true;
                    let _ = second.as_mut().poll(context);
                    context.waker().wake_by_ref();
                    std::task::Poll::Pending
                })
                .await;
            }
            timer.sleeping_for(Duration::from_secs(3)).await;
            timer.advance(Duration::from_secs(3));
        },
    ));
    assert_eq!(first, Ok(json!("answered")));
    assert!(runtime.is_alive());
}

/// A program the script started is followed to its end when the call's
/// time is up, as on a cancel: what it changed is looked at and recorded
/// after it stops.
#[test]
fn a_call_out_of_time_follows_its_program_to_the_end() {
    let timer = timer();
    let snapshot = tool(
        "exec = [\"gh\"]",
        "async () => { await cineris.exec('gh', []); return 'ran' }",
    );
    let runtime = start(&snapshot, &timer).unwrap();
    let host = FakeHost {
        slow_exec: Some((timer.clone(), TIME + Duration::from_secs(3))),
        ..FakeHost::new()
    };
    let (result, ()) = block_on(join(
        call_async(&runtime, &snapshot, &host, &timer),
        async {
            timer.sleeping_for(TIME + Duration::from_secs(3)).await;
            // The call's time is up while the program runs on.
            timer.advance(TIME);
            for _ in 0..8 {
                let mut turned = false;
                std::future::poll_fn(|context| {
                    if turned {
                        std::task::Poll::Ready(())
                    } else {
                        turned = true;
                        context.waker().wake_by_ref();
                        std::task::Poll::Pending
                    }
                })
                .await;
            }
            timer.advance(Duration::from_secs(3));
        },
    ));
    assert_eq!(result, Err(CallError::TimedOut));
    assert!(
        host.asked().iter().any(|noted| noted == "exec finished"),
        "{:?}",
        host.asked()
    );
}

/// A call of the tool `name`, as a future.
async fn call_named(
    runtime: &ScriptRuntime,
    name: &str,
    host: &FakeHost,
    capabilities: &Capabilities,
    timer: &FakeTimer,
    cancel: &CancelToken,
) -> Result<Json, CallError> {
    runtime
        .call(
            Target::Tool(name.to_owned()),
            json!({}),
            CallContext {
                host,
                capabilities,
                time: TIME,
                timer,
                clock: timer.clock(),
                cancel,
            },
        )
        .await
}

/// [`call`], as a future to run beside the test's own steps.
async fn call_async(
    runtime: &ScriptRuntime,
    snapshot: &Snapshot,
    host: &FakeHost,
    timer: &FakeTimer,
) -> Result<Json, CallError> {
    let capabilities = capabilities(snapshot);
    let cancel = CancelToken::new();
    runtime
        .call(
            Target::Tool("t".to_owned()),
            json!({}),
            CallContext {
                host,
                capabilities: &capabilities,
                time: TIME,
                timer,
                clock: timer.clock(),
                cancel: &cancel,
            },
        )
        .await
}
