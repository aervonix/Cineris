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
