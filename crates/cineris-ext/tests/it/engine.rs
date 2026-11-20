//! The engine as built: it runs a script, it has nothing ambient, and its
//! limits stop a runaway script. No test waits: the interrupt check counts
//! its own calls.

use std::cell::Cell;
use std::rc::Rc;

use cineris_ext::engine::{Engine, Limits, ScriptError};

fn engine() -> Engine {
    Engine::new(Limits::default(), || false).unwrap()
}

#[test]
fn a_script_runs_and_gives_its_value() {
    let engine = engine();
    assert_eq!(engine.eval_json("1 + 2").unwrap(), "3");
    assert_eq!(
        engine
            .eval_json("JSON.parse('{\"a\":[1,2]}').a.map(x => x * 2)")
            .unwrap(),
        "[2,4]"
    );
    assert_eq!(
        engine
            .eval_json("'word count'.split(' ').length + /c(o)u/.exec('count')[1].length")
            .unwrap(),
        "3"
    );
}

#[test]
fn nothing_ambient_is_on_the_global_object() {
    let engine = engine();
    for name in [
        "require",
        "process",
        "std",
        "os",
        "print",
        "console",
        "setTimeout",
        "setInterval",
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
    ] {
        assert_eq!(
            engine.eval_json(&format!("typeof {name}")).unwrap(),
            "\"undefined\"",
            "`{name}` is reachable"
        );
    }
}

#[test]
fn a_dynamic_import_loads_nothing() {
    let engine = engine();
    // With no loader set, a module cannot be found by any name.
    let outcome = engine
        .eval_json(
            "let seen = 'pending'; \
             import('/etc/passwd').then(() => { seen = 'loaded' }, e => { seen = String(e) }); \
             seen",
        )
        .unwrap();
    assert_eq!(outcome, "\"pending\"");
}

#[test]
fn an_endless_loop_is_stopped_by_the_interrupt_check() {
    let polls = Rc::new(Cell::new(0_u32));
    let counted = Rc::clone(&polls);
    let engine = Engine::new(Limits::default(), move || {
        counted.set(counted.get() + 1);
        counted.get() > 3
    })
    .unwrap();
    let error = engine.eval_json("for (;;) {}").unwrap_err();
    assert!(
        matches!(&error, ScriptError::Thrown(text) if text.contains("interrupted")),
        "{error}"
    );
    assert!(polls.get() > 3);
}

#[test]
fn a_memory_bomb_is_stopped_by_the_memory_limit() {
    let engine = Engine::new(
        Limits {
            memory: 8 * 1024 * 1024,
            ..Limits::default()
        },
        || false,
    )
    .unwrap();
    let error = engine
        .eval_json("const a = []; for (;;) { a.push('x'.repeat(1 << 16) + a.length) }")
        .unwrap_err();
    assert!(
        matches!(&error, ScriptError::Thrown(text) if text.contains("out of memory")),
        "{error}"
    );
    let error = engine.eval_json("'x'.repeat(1 << 28).length").unwrap_err();
    assert!(matches!(error, ScriptError::Thrown(_)), "{error}");
}

#[test]
fn endless_recursion_is_stopped_by_the_stack_limit() {
    let engine = engine();
    let error = engine
        .eval_json("function deeper(n) { return deeper(n + 1) + 1 } deeper(0)")
        .unwrap_err();
    assert!(
        matches!(&error, ScriptError::Thrown(text) if text.contains("stack")),
        "{error}"
    );
    // The engine is still usable after it.
    assert_eq!(engine.eval_json("40 + 2").unwrap(), "42");
}
