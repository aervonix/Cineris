//! Hooks in the loop (`docs/architecture/12-extensions.md`, "Tier 3:
//! hooks"; `docs/steps/28/answered.md`, "6. Hooks in the loop"), through
//! fake hooks: `prompt.submit` and `tool.before`.
//!
//! A fake hook answers what its test scripted, one answer a run, and keeps
//! what it was given. It starts no process, and a hook that does not answer
//! is held on the manual fake timer: no test waits for real time.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cineris_agent::{
    Decision, Hook, HookEvent, HookInput, HookOutcome, Hooks, LoopHooks, PlannedCall, ToolTarget,
    Verdict,
};
use cineris_protocol::error::code;
use cineris_protocol::event::{Notice, PromptReturnedNotice, TurnOutcome};
use cineris_protocol::item::{Delivery, HookReason, NoticeSource, ToolOutcome, ToolResultItem};
use cineris_protocol::{ErrorKind, Item, Part, Role};
use cineris_testkit::Gate;
use cineris_testkit::timer::FakeTimer;
use cineris_tools::{BoxFuture, Json, Toolbox};
use serde_json::json;
use tokio::sync::Notify;

use crate::{
    Kind, Probe, Rig, Setup, World, assert_valid_history, calls, last_request, lock, says,
    text_of,
};

// ------------------------------------------------------------ fake hooks

/// What a fake hook does on one run.
#[derive(Clone)]
enum Reply {
    /// Answers at once.
    Now(HookOutcome),
    /// Never answers: its time runs out, or the turn is interrupted.
    Never,
    /// Answers once the gate opens.
    AtGate(Gate, HookOutcome),
}

fn answer(value: Json) -> Reply {
    Reply::Now(HookOutcome::Answer(value))
}

fn nothing() -> Reply {
    Reply::Now(HookOutcome::Nothing)
}

fn fails(why: &str) -> Reply {
    Reply::Now(HookOutcome::Failed(why.to_owned()))
}

/// A hook a test scripts.
struct FakeHook {
    name: String,
    event: HookEvent,
    /// The tool its filter names: `None` matches every call.
    tool: Option<String>,
    observes: bool,
    timeout: Option<Duration>,
    /// One reply a run, in order; with none left, it answers nothing.
    replies: Mutex<VecDeque<Reply>>,
    /// What it was given, in order.
    inputs: Mutex<Vec<Json>>,
    wake: Notify,
}

impl FakeHook {
    fn on(name: &str, event: HookEvent) -> Self {
        Self {
            name: name.to_owned(),
            event,
            tool: None,
            observes: false,
            timeout: None,
            replies: Mutex::default(),
            inputs: Mutex::default(),
            wake: Notify::new(),
        }
    }

    fn tool(mut self, tool: &str) -> Self {
        self.tool = Some(tool.to_owned());
        self
    }

    fn observing(mut self) -> Self {
        self.observes = true;
        self
    }

    fn timeout(mut self, time: Duration) -> Self {
        self.timeout = Some(time);
        self
    }

    fn replies(self, replies: impl IntoIterator<Item = Reply>) -> Self {
        lock(&self.replies).extend(replies);
        self
    }

    fn matches(&self, event: HookEvent, target: Option<ToolTarget<'_>>) -> bool {
        self.event == event
            && match (&self.tool, target) {
                (None, _) => true,
                (Some(tool), Some(target)) => target.tool == tool || target.names == Some(tool),
                (Some(_), None) => false,
            }
    }

    fn inputs(&self) -> Vec<Json> {
        lock(&self.inputs).clone()
    }

    /// Waits until it has been run `count` times.
    async fn runs(&self, count: usize) {
        loop {
            if lock(&self.inputs).len() >= count {
                return;
            }
            self.wake.notified().await;
        }
    }
}

impl Hook for FakeHook {
    fn name(&self) -> &str {
        &self.name
    }

    fn observes(&self) -> bool {
        self.observes
    }

    fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    fn run<'a>(&'a self, input: &'a HookInput) -> BoxFuture<'a, HookOutcome> {
        lock(&self.inputs).push(input.to_json());
        self.wake.notify_waiters();
        let reply = lock(&self.replies).pop_front().unwrap_or_else(nothing);
        Box::pin(async move {
            match reply {
                Reply::Now(outcome) => outcome,
                Reply::Never => std::future::pending().await,
                Reply::AtGate(gate, outcome) => {
                    gate.opened().await;
                    outcome
                }
            }
        })
    }
}

/// The hooks of a session a test scripts.
#[derive(Default)]
struct FakeHooks {
    hooks: Mutex<Vec<Arc<FakeHook>>>,
    /// Whether `matching` gives the hooks in the reverse of the order they
    /// were added in.
    reversed: std::sync::atomic::AtomicBool,
    /// How many times each method was asked.
    wanted: AtomicUsize,
    matched: AtomicUsize,
    /// What `check_replacement` refuses with, when set.
    refuse: Mutex<Option<String>>,
    /// The replacements checked, by their plan's summary.
    checked: Mutex<Vec<String>>,
}

impl FakeHooks {
    fn add(&self, hook: FakeHook) -> Arc<FakeHook> {
        let hook = Arc::new(hook);
        lock(&self.hooks).push(Arc::clone(&hook));
        hook
    }

    fn checked(&self) -> Vec<String> {
        lock(&self.checked).clone()
    }
}

impl Hooks for FakeHooks {
    fn wants(&self, event: HookEvent, target: Option<ToolTarget<'_>>) -> bool {
        self.wanted.fetch_add(1, Ordering::SeqCst);
        lock(&self.hooks)
            .iter()
            .any(|hook| hook.matches(event, target))
    }

    fn matching(&self, event: HookEvent, target: Option<ToolTarget<'_>>) -> Vec<Arc<dyn Hook>> {
        self.matched.fetch_add(1, Ordering::SeqCst);
        let mut matched: Vec<Arc<dyn Hook>> = lock(&self.hooks)
            .iter()
            .filter(|hook| hook.matches(event, target))
            .map(|hook| Arc::clone(hook) as Arc<dyn Hook>)
            .collect();
        if self.reversed.load(Ordering::SeqCst) {
            matched.reverse();
        }
        matched
    }

    fn check_replacement(&self, call: &PlannedCall<'_>) -> Result<(), String> {
        lock(&self.checked).push(call.plan.summary.clone());
        lock(&self.refuse).clone().map_or(Ok(()), Err)
    }
}

/// One session with fake hooks and a manual timer for their times.
struct Hooked {
    world: World,
    rig: Rig,
    hooks: Arc<FakeHooks>,
    timer: FakeTimer,
}

fn hooked(test: &str, responses: impl IntoIterator<Item = cineris_testkit::Response>) -> Hooked {
    hooked_with(test, responses, |_, _| {})
}

fn hooked_with(
    test: &str,
    responses: impl IntoIterator<Item = cineris_testkit::Response>,
    set_up: impl FnOnce(&World, &mut Setup),
) -> Hooked {
    let world = World::new(test, responses);
    let hooks = Arc::new(FakeHooks::default());
    let timer = FakeTimer::manual(&world.clock);
    let mut setup = Setup::new(&world);
    setup.hooks = Some(LoopHooks {
        hooks: Arc::clone(&hooks) as Arc<dyn Hooks>,
        timer: Arc::new(timer.clone()),
    });
    set_up(&world, &mut setup);
    let rig = setup.start(&world);
    Hooked {
        world,
        rig,
        hooks,
        timer,
    }
}

// --------------------------------------------------------------- helpers

fn results(rig: &Rig) -> Vec<ToolResultItem> {
    rig.store
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::ToolResult(result) => Some(result),
            _ => None,
        })
        .collect()
}

fn text(result: &ToolResultItem) -> String {
    result
        .content
        .iter()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn user_texts(rig: &Rig) -> Vec<String> {
    rig.store
        .items()
        .into_iter()
        .filter_map(|item| match item {
            Item::UserMessage(user) => Some(
                user.parts
                    .iter()
                    .filter_map(|part| match part {
                        Part::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn returned(rig: &Rig) -> Vec<PromptReturnedNotice> {
    rig.events
        .notices()
        .into_iter()
        .filter_map(|notice| match notice {
            Notice::PromptReturned(returned) => Some(returned),
            _ => None,
        })
        .collect()
}

fn reason(hook: &str, reason: &str) -> HookReason {
    HookReason {
        hook: hook.to_owned(),
        reason: Some(reason.to_owned()),
    }
}

/// A toolbox with the fake tools and a `tool_call` that touches nothing:
/// what a hook's filter and a replacement are held to for `tool_call` is by
/// its name and its `tool` argument.
fn with_tool_call(world: &World) -> Toolbox {
    world
        .toolbox()
        .with(Probe::new("tool_call", Kind::Plain, &world.log, &world.gates))
}

// ---------------------------------------------------------- no hook at all

/// Runs one turn of a round (a read, a write, a command, a steering
/// message delivered after it) and returns what front ends and the log saw.
fn the_same_turn(test: &str, hooks: Option<&Arc<FakeHooks>>) -> (Vec<String>, Vec<Item>, usize) {
    let response = calls(&[
        ("c1", "read", json!({"path": "a"})),
        ("c2", "write", json!({"path": "b", "gate": "w"})),
        ("c3", "shell", json!({"id": "make"})),
    ]);
    let world = World::new(test, [response, says("Done.")]);
    let mut setup = Setup::new(&world);
    setup.hooks = hooks.map(|hooks| LoopHooks {
        hooks: Arc::clone(hooks) as Arc<dyn Hooks>,
        timer: Arc::new(FakeTimer::manual(&world.clock)),
    });
    let rig = setup.start(&world);
    rig.policy.decide_for("shell", Decision::Ask);
    rig.policy.answer_all(Verdict::Allow);
    world.run(async {
        rig.prompt("go");
        world.log.has("start write b").await;
        rig.steer("and then b");
        world.gates.open("w");
        rig.events.turns(1).await;
    });
    assert_valid_history(&last_request(&world.provider).messages);
    (
        rig.events.labels(),
        rig.store.items(),
        world.provider.requests().len(),
    )
}

/// With no hook whose filter matches, every point is passed at once: the
/// loop asks only the cheap check, never runs or even lists a hook, and
/// the turn is the one a session with no hooks has, event for event.
#[test]
fn with_no_hook_that_matches_every_point_is_passed_and_nothing_changes() {
    let without = the_same_turn("hooks_none", None);
    let empty = Arc::new(FakeHooks::default());
    let with_none = the_same_turn("hooks_empty", Some(&empty));
    // Hooks on other tools, and on an event this turn has none of.
    let elsewhere = Arc::new(FakeHooks::default());
    let other = elsewhere.add(FakeHook::on("other", HookEvent::ToolBefore).tool("solo"));
    let with_others = the_same_turn("hooks_elsewhere", Some(&elsewhere));
    assert_eq!(with_none, without);
    assert_eq!(with_others, without);
    for hooks in [&empty, &elsewhere] {
        assert_eq!(hooks.matched.load(Ordering::SeqCst), 0, "no hook was listed");
        assert!(
            hooks.wanted.load(Ordering::SeqCst) >= 5,
            "every point asked: three calls and two prompts"
        );
    }
    assert!(other.inputs().is_empty(), "nothing ran");
}

// ------------------------------------------------------------- tool.before

/// A hook that answers `deny` refuses a call the policy allowed: it does
/// not run, its result names the hook and its reason, and the round goes
/// on.
#[test]
fn a_hook_that_denies_refuses_a_call_the_policy_allowed() {
    let response = calls(&[
        ("c1", "write", json!({"path": "a"})),
        ("c2", "read", json!({"path": "r"})),
    ]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_denies", [response, says("Fine.")]);
    let hook = hooks.add(
        FakeHook::on("no-writes", HookEvent::ToolBefore)
            .tool("write")
            .replies([answer(
                json!({"decision": "deny", "reason": "nothing is written here"}),
            )]),
    );
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(world.log.count("start write a"), 0);
    assert_eq!(world.log.count("end read r"), 1, "the read was not hooked");
    let results = results(&rig);
    assert_eq!(results[0].outcome, ToolOutcome::Denied);
    let said = text(&results[0]);
    assert!(said.contains("`no-writes`"), "{said}");
    assert!(said.contains("nothing is written here"), "{said}");
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Completed);
    assert_eq!(world.provider.requests().len(), 2, "the round went on");
    assert!(rig.policy.asked().is_empty());
    // The hook was given the call as the contract says.
    let input = &hook.inputs()[0];
    assert_eq!(input["event"], "tool.before");
    assert_eq!(input["tool"], "write");
    assert_eq!(input["call_id"], "c1");
    assert_eq!(input["arguments"], json!({"path": "a"}));
    assert_eq!(input["plan"], "write a");
    assert_eq!(input["session_id"], "s_1");
    assert!(input["turn_id"].is_string() && input["workspace"].is_string());
}

/// `ask` turns a call the policy allowed into a question, which shows the
/// hook's reason; the call runs once it is allowed.
#[test]
fn a_hook_that_asks_turns_an_allowed_call_into_a_question_with_its_reason() {
    let response = calls(&[("c1", "write", json!({"path": "a"}))]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_asks", [response, says("Fine.")]);
    hooks.add(
        FakeHook::on("look-first", HookEvent::ToolBefore).replies([answer(
            json!({"decision": "ask", "reason": "a write outside src"}),
        )]),
    );
    rig.policy.answer_for("write", Verdict::Allow);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let asked = rig.policy.asked_with();
    assert_eq!(asked.len(), 1, "the allowed call was asked about");
    let shown = asked[0].hooks.clone().expect("the question shows its hooks");
    assert_eq!(shown.asked, [reason("look-first", "a write outside src")]);
    assert_eq!(shown.replaced, None);
    assert_eq!(world.log.count("end write a"), 1, "allowed, it ran");
}

/// `allow` never lifts the policy's question: the call is asked about as
/// the policy said, and the question shows no hook.
#[test]
fn a_hooks_allow_never_lifts_the_policys_question() {
    let response = calls(&[("c1", "write", json!({"path": "a"}))]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_allow_asks", [response, says("Fine.")]);
    hooks.add(
        FakeHook::on("yes", HookEvent::ToolBefore)
            .replies([answer(json!({"decision": "allow", "reason": "fine by me"}))]),
    );
    rig.policy.decide_for("write", Decision::Ask);
    rig.policy.answer_for(
        "write",
        Verdict::Deny {
            note: Some("not now".to_owned()),
        },
    );
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    let asked = rig.policy.asked_with();
    assert_eq!(asked.len(), 1, "the policy's question was put");
    assert_eq!(asked[0].hooks, None, "an allow puts nothing in it");
    assert_eq!(world.log.count("start write a"), 0);
}

/// A call the policy denied or stopped is not shown to hooks.
#[test]
fn a_call_the_policy_denied_or_stopped_is_not_shown_to_hooks() {
    let response = calls(&[
        ("c1", "write", json!({"path": "a"})),
        ("c2", "shell", json!({"id": "deploy"})),
    ]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_not_shown", [response, says("Unused.")]);
    let hook = hooks.add(FakeHook::on("all", HookEvent::ToolBefore));
    rig.policy.decide_for("write", Decision::Deny { reason: None });
    rig.policy.decide_for(
        "shell",
        Decision::Stop {
            reason: "the run stops at a denial".to_owned(),
        },
    );
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert!(hook.inputs().is_empty(), "{:?}", hook.inputs());
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Denied);
}

/// Hooks on one event run together, and their answers combine as `deny`
/// over `ask` over `allow`: the same in whatever order they come.
#[test]
fn hooks_answers_combine_as_deny_over_ask_over_allow_in_any_order() {
    for reversed in [false, true] {
        let response = calls(&[("c1", "write", json!({"path": "a"}))]);
        let Hooked {
            world, rig, hooks, ..
        } = hooked(&format!("hook_combine_{reversed}"), [response, says("Fine.")]);
        hooks.reversed.store(reversed, Ordering::SeqCst);
        // The first is held until the others have been run: they run
        // together, not one after another.
        let held = Gate::new();
        let first = hooks.add(
            FakeHook::on("a-allow", HookEvent::ToolBefore).replies([Reply::AtGate(
                held.clone(),
                HookOutcome::Answer(json!({"decision": "allow"})),
            )]),
        );
        let asks = hooks.add(
            FakeHook::on("b-ask", HookEvent::ToolBefore)
                .replies([answer(json!({"decision": "ask"}))]),
        );
        let denies = hooks.add(
            FakeHook::on("c-deny", HookEvent::ToolBefore)
                .replies([answer(json!({"decision": "deny", "reason": "no"}))]),
        );
        world.run(async {
            rig.prompt("go");
            first.runs(1).await;
            asks.runs(1).await;
            denies.runs(1).await;
            held.open();
            rig.events.turns(1).await;
        });
        let results = results(&rig);
        assert_eq!(results[0].outcome, ToolOutcome::Denied, "reversed: {reversed}");
        assert!(text(&results[0]).contains("`c-deny` refused it: no"));
        assert!(rig.policy.asked().is_empty(), "a denial is not asked about");
    }
}

/// While its hooks are asked, a call counts as one whose plan is not known
/// yet: a later call that could conflict with it does not start.
#[test]
fn while_its_hooks_are_asked_a_call_holds_back_what_could_conflict_with_it() {
    let response = calls(&[
        ("c1", "write", json!({"path": "a"})),
        ("c2", "read", json!({"path": "a"})),
    ]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_holds_back", [response, says("Fine.")]);
    let held = Gate::new();
    let hook = hooks.add(
        FakeHook::on("slow", HookEvent::ToolBefore)
            .tool("write")
            .replies([Reply::AtGate(held.clone(), HookOutcome::Nothing)]),
    );
    world.run(async {
        rig.prompt("go");
        hook.runs(1).await;
        // The read is decided, and could start but for the write.
        rig.policy.decisions(2).await;
        crate::settle().await;
        assert_eq!(world.log.count("start read a"), 0);
        held.open();
        rig.events.turns(1).await;
    });
    let lines = world.log.lines();
    let wrote = lines.iter().position(|line| line == "end write a").unwrap();
    let read = lines.iter().position(|line| line == "start read a").unwrap();
    assert!(wrote < read, "{lines:?}");
}

/// A call decided again (the re-decide step) is shown to its hooks again,
/// with its new plan and the model's own arguments, and what they said
/// before is dropped: here a replacement, which then never runs.
#[test]
fn a_call_decided_again_is_shown_to_its_hooks_again_with_the_models_arguments() {
    let response = calls(&[
        ("c1", "plain", json!({"id": "q"})),
        ("c2", "write", json!({"path": "b"})),
    ]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_redecide", [response, says("Done.")]);
    rig.policy.decide_for("plain q", Decision::Ask);
    rig.policy.wait_at(&world.gates);
    rig.policy.answer_for("plain", Verdict::Allow);
    let hook = hooks.add(
        FakeHook::on("swap", HookEvent::ToolBefore)
            .tool("write")
            .replies([answer(json!({"arguments": {"path": "c"}})), nothing()]),
    );
    world.run(async {
        rig.prompt("go");
        rig.policy.questions(1).await;
        hook.runs(1).await;
        crate::settle().await;
        // A rule is taken back while the write waits behind the question.
        rig.policy.narrow();
        world.gates.open("ask plain");
        hook.runs(2).await;
        rig.events.turns(1).await;
    });
    let inputs = hook.inputs();
    assert_eq!(inputs.len(), 2);
    for input in &inputs {
        assert_eq!(input["arguments"], json!({"path": "b"}), "the model's own");
    }
    assert_eq!(world.log.count("end write b"), 1);
    assert_eq!(world.log.count("start write c"), 0, "the replacement was dropped");
    assert_eq!(rig.policy.asked(), ["plain q"], "the replacement was never asked about");
    assert_eq!(results(&rig)[1].replaced, None);
}

// -------------------------------------------------------------- replacing

/// A replacement from one hook is checked against the schema, planned by
/// the tool and decided by the policy again, held to the protected paths,
/// and then always put to the user, with the hook's name and both sets of
/// arguments, though the policy allows it. The hooks are not run on it.
/// It runs as the user allowed it, and its result and item say which hook
/// changed the arguments to what.
#[test]
fn a_replacement_is_planned_decided_checked_and_always_put_to_the_user() {
    let response = calls(&[("c1", "write", json!({"path": "a"}))]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_replaces", [response, says("Done.")]);
    let hook = hooks.add(FakeHook::on("elsewhere", HookEvent::ToolBefore).replies([answer(
        json!({"arguments": {"path": "b"}, "reason": "b is the copy"}),
    )]));
    rig.policy.answer_for("write", Verdict::Allow);
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(hook.inputs().len(), 1, "the hooks were not run on the replacement");
    assert_eq!(rig.policy.decided(), ["write a", "write b"], "decided again");
    assert_eq!(hooks.checked(), ["write b"], "checked before it was put");
    let asked = rig.policy.asked_with();
    assert_eq!(asked.len(), 1, "put to the user though the policy allows it");
    assert_eq!(asked[0].summary, "write b");
    assert_eq!(asked[0].arguments, json!({"path": "b"}));
    let replaced = asked[0]
        .hooks
        .clone()
        .and_then(|hooks| hooks.replaced)
        .expect("the question shows the replacement");
    assert_eq!(replaced.hook, "elsewhere");
    assert_eq!(replaced.arguments, json!({"path": "b"}));
    assert_eq!(replaced.original, Some(json!({"path": "a"})));
    assert_eq!(replaced.reason.as_deref(), Some("b is the copy"));

    assert_eq!(world.log.count("end write b"), 1);
    assert_eq!(world.log.count("start write a"), 0);
    let result = &results(&rig)[0];
    let item = result.replaced.clone().expect("the item names the replacement");
    assert_eq!(item.hook, "elsewhere");
    assert_eq!(item.arguments, json!({"path": "b"}));
    assert_eq!(item.original, None, "the model's own are in its call");
    let said = text(result);
    assert!(said.starts_with("The hook `elsewhere` put these arguments"), "{said}");
    assert!(said.contains(r#"{"path":"b"}"#), "{said}");
    assert!(said.contains("write b ok"), "{said}");
    // The model is told it in the next request.
    let request = last_request(&world.provider);
    let tool = request
        .messages
        .iter()
        .find(|message| message.role == Role::Tool)
        .expect("the result");
    assert!(format!("{tool:?}").contains("The hook `elsewhere`"));
}

/// A replacement the user refuses does not run, and the model is told
/// which hook had changed the arguments.
#[test]
fn a_replacement_the_user_refuses_does_not_run() {
    let response = calls(&[("c1", "write", json!({"path": "a"}))]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_replace_refused", [response, says("Done.")]);
    hooks.add(
        FakeHook::on("elsewhere", HookEvent::ToolBefore)
            .replies([answer(json!({"arguments": {"path": "b"}}))]),
    );
    rig.policy.answer_for("write", Verdict::Deny { note: None });
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(world.log.count("start write b"), 0);
    let result = &results(&rig)[0];
    assert_eq!(result.outcome, ToolOutcome::Denied);
    assert!(text(result).contains("The hook `elsewhere`"));
    assert!(text(result).contains("The user did not allow this call."));
}

/// A replacement the policy refuses, or stops the turn over, stands: it is
/// not put to the user.
#[test]
fn a_replacement_the_policy_denies_or_stops_stands() {
    for stops in [false, true] {
        let response = calls(&[("c1", "write", json!({"path": "a"}))]);
        let Hooked {
            world, rig, hooks, ..
        } = hooked(&format!("hook_replace_policy_{stops}"), [response, says("Done.")]);
        hooks.add(
            FakeHook::on("elsewhere", HookEvent::ToolBefore)
                .replies([answer(json!({"arguments": {"path": "b"}}))]),
        );
        let decision = if stops {
            Decision::Stop {
                reason: "b is out of bounds".to_owned(),
            }
        } else {
            Decision::Deny {
                reason: Some("b is out of bounds".to_owned()),
            }
        };
        rig.policy.decide_for("write b", decision);
        world.run(async {
            rig.prompt("go");
            rig.events.turns(1).await;
        });
        assert!(rig.policy.asked().is_empty(), "stops: {stops}");
        assert!(hooks.checked().is_empty(), "a refusal needs no check");
        assert_eq!(world.log.count("start write"), 0);
        let result = &results(&rig)[0];
        assert_eq!(result.outcome, ToolOutcome::Denied);
        assert!(text(result).contains("b is out of bounds"), "{}", text(result));
        let outcome = &rig.events.ends()[0].outcome;
        if stops {
            assert_eq!(*outcome, TurnOutcome::Denied);
        } else {
            assert!(text(result).contains("`elsewhere` replaced its arguments"));
            assert_eq!(*outcome, TurnOutcome::Completed);
        }
    }
}

/// A replacement that does not fit the tool's schema, or that its tool
/// refuses when it plans it, is refused, and nobody is asked.
#[test]
fn a_replacement_the_schema_or_its_tool_refuses_is_refused() {
    for (replacement, says_why) in [
        (json!("not an object"), "do not fit the tool's schema"),
        (json!({"bad": true}), "ones its tool refuses"),
    ] {
        let response = calls(&[("c1", "write", json!({"path": "a"}))]);
        let Hooked {
            world, rig, hooks, ..
        } = hooked("hook_replace_unfit", [response, says("Done.")]);
        hooks.add(
            FakeHook::on("odd", HookEvent::ToolBefore)
                .replies([answer(json!({"arguments": replacement}))]),
        );
        world.run(async {
            rig.prompt("go");
            rig.events.turns(1).await;
        });
        let result = &results(&rig)[0];
        assert_eq!(result.outcome, ToolOutcome::Denied, "{replacement}");
        assert!(text(result).contains(says_why), "{}", text(result));
        assert!(rig.policy.asked().is_empty());
        assert_eq!(rig.policy.decided(), ["write a"], "not decided");
    }
}

/// For `tool_call`, a replacement may not name another tool; one that
/// keeps the tool it names is put to the user as any other.
#[test]
fn a_tool_call_replacement_may_not_name_another_tool() {
    for (replacement, refused) in [
        (json!({"tool": "tracker/delete", "arguments": {}}), true),
        (json!({"tool": "tracker/find", "arguments": {"q": "x"}}), false),
    ] {
        let response = calls(&[(
            "c1",
            "tool_call",
            json!({"tool": "tracker/find", "arguments": {}}),
        )]);
        let Hooked {
            world, rig, hooks, ..
        } = hooked_with("hook_tool_call", [response, says("Done.")], |world, setup| {
            setup.toolbox = Some(with_tool_call(world));
        });
        // A filter on the tool a `tool_call` names matches it.
        let hook = hooks.add(
            FakeHook::on("narrow", HookEvent::ToolBefore)
                .tool("tracker/find")
                .replies([answer(json!({"arguments": replacement}))]),
        );
        rig.policy.answer_all(Verdict::Allow);
        world.run(async {
            rig.prompt("go");
            rig.events.turns(1).await;
        });
        assert_eq!(hook.inputs().len(), 1);
        let result = &results(&rig)[0];
        if refused {
            assert_eq!(result.outcome, ToolOutcome::Denied);
            assert!(text(result).contains("call another tool (`tracker/delete`)"));
            assert!(rig.policy.asked().is_empty());
        } else {
            assert_eq!(result.outcome, ToolOutcome::Success);
            assert_eq!(rig.policy.asked().len(), 1);
        }
    }
}

/// Two hooks that both replace the arguments refuse the call: it does not
/// run with arguments neither hook accepted.
#[test]
fn two_hooks_that_replace_the_arguments_deny_the_call() {
    let response = calls(&[("c1", "write", json!({"path": "a"}))]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_two_replace", [response, says("Done.")]);
    hooks.add(
        FakeHook::on("to-b", HookEvent::ToolBefore)
            .replies([answer(json!({"arguments": {"path": "b"}}))]),
    );
    hooks.add(
        FakeHook::on("to-c", HookEvent::ToolBefore)
            .replies([answer(json!({"arguments": {"path": "c"}}))]),
    );
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(world.log.count("start write"), 0);
    let result = &results(&rig)[0];
    assert_eq!(result.outcome, ToolOutcome::Denied);
    assert!(text(result).contains("`to-b` and `to-c` each replaced its arguments"));
    assert!(rig.policy.asked().is_empty());
}

/// A replacement whose plan the core refuses (one that would write a
/// protected path) is refused before anyone is asked.
#[test]
fn a_replacement_the_protected_path_check_refuses_is_never_put_to_the_user() {
    let response = calls(&[("c1", "write", json!({"path": "a"}))]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_replace_protected", [response, says("Done.")]);
    *lock(&hooks.refuse) = Some("`.git/hooks/pre-commit` is a protected path".to_owned());
    hooks.add(
        FakeHook::on("sneaky", HookEvent::ToolBefore)
            .replies([answer(json!({"arguments": {"path": "b"}}))]),
    );
    world.run(async {
        rig.prompt("go");
        rig.events.turns(1).await;
    });
    assert_eq!(hooks.checked(), ["write b"]);
    assert!(rig.policy.asked().is_empty());
    assert_eq!(world.log.count("start write"), 0);
    let result = &results(&rig)[0];
    assert_eq!(result.outcome, ToolOutcome::Denied);
    assert!(text(result).contains("may not be put to the user"));
    assert!(text(result).contains("is a protected path"));
}

// ------------------------------------------------------- failure and time

/// A hook that fails, or answers with something outside its contract (a
/// field the event does not take, a decision it does not know), counts as
/// `deny`, with the failure named.
#[test]
fn a_hook_that_fails_or_breaks_its_contract_refuses_the_call() {
    for (reply, named) in [
        (fails("it exited with 2"), "it exited with 2"),
        (
            answer(json!({"decision": "maybe"})),
            "`decision` is `maybe`",
        ),
        (answer(json!({"context": "x"})), "`context`"),
        (answer(json!({"decision": "allow", "later": 1})), "`later`"),
        (answer(json!(["deny"])), "not a JSON object"),
    ] {
        let response = calls(&[("c1", "write", json!({"path": "a"}))]);
        let Hooked {
            world, rig, hooks, ..
        } = hooked("hook_fails", [response, says("Done.")]);
        hooks.add(FakeHook::on("broken", HookEvent::ToolBefore).replies([reply]));
        world.run(async {
            rig.prompt("go");
            rig.events.turns(1).await;
        });
        assert_eq!(world.log.count("start write a"), 0, "{named}");
        let said = text(&results(&rig)[0]);
        assert!(said.contains("the hook `broken` failed"), "{said}");
        assert!(said.contains(named), "{said}");
    }
}

/// A hook that does not answer within its time refuses, on the session's
/// timer: 5 s by default, and 60 s at most whatever it names.
#[test]
fn a_hook_that_does_not_answer_in_its_time_refuses_on_the_sessions_timer() {
    for (named, waited) in [(None, 5), (Some(120), 60), (Some(2), 2)] {
        let response = calls(&[("c1", "write", json!({"path": "a"}))]);
        let Hooked {
            world,
            rig,
            hooks,
            timer,
        } = hooked("hook_times_out", [response, says("Done.")]);
        let mut hook = FakeHook::on("stuck", HookEvent::ToolBefore).replies([Reply::Never]);
        if let Some(named) = named {
            hook = hook.timeout(Duration::from_secs(named));
        }
        hooks.add(hook);
        let waited = Duration::from_secs(waited);
        world.run(async {
            rig.prompt("go");
            timer.sleeping_for(waited).await;
            assert_eq!(timer.running(), [waited], "{named:?}");
            timer.advance(waited);
            rig.events.turns(1).await;
        });
        let said = text(&results(&rig)[0]);
        assert!(
            said.contains(&format!("did not answer within {} s", waited.as_secs())),
            "{said}"
        );
        assert_eq!(world.log.count("start write a"), 0);
    }
}

/// An observer is never waited for and its answer is not read: the call
/// runs while it is still running, its refusal changes nothing, and it is
/// given 60 s by default and 10 minutes at most.
#[test]
fn an_observer_is_never_waited_for_and_its_answer_is_not_read() {
    for (named, given) in [(None, 60), (Some(3_600), 600)] {
        let response = calls(&[
            ("c1", "write", json!({"path": "a"})),
            ("c2", "write", json!({"path": "b"})),
        ]);
        let Hooked {
            world,
            rig,
            hooks,
            timer,
        } = hooked("hook_observer", [response, says("Done.")]);
        let mut watching = FakeHook::on("watching", HookEvent::ToolBefore)
            .observing()
            .tool("write")
            .replies([Reply::Never, answer(json!({"decision": "deny"}))]);
        if let Some(named) = named {
            watching = watching.timeout(Duration::from_secs(named));
        }
        let watching = hooks.add(watching);
        let given = Duration::from_secs(given);
        world.run(async {
            rig.prompt("go");
            rig.events.turns(1).await;
            assert_eq!(timer.running(), [given], "the first still runs");
            timer.advance(given);
            crate::settle().await;
        });
        assert_eq!(watching.inputs().len(), 2);
        assert_eq!(world.log.count("end write a"), 1);
        assert_eq!(world.log.count("end write b"), 1, "its deny was not read");
        assert!(rig.policy.asked().is_empty());
        assert!(timer.running().is_empty(), "stopped at its time");
    }
}

// ----------------------------------------------------------- prompt.submit

/// A prompt a hook refuses is not recorded and never reaches the model: the
/// turn it would start ends `denied`, and the front end is given the text
/// back with the hook's reason. The next prompt is taken as usual.
#[test]
fn a_prompt_a_hook_refuses_ends_its_turn_denied_unrecorded_and_is_given_back() {
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_prompt_denied", [says("Hello.")]);
    let hook = hooks.add(FakeHook::on("no-keys", HookEvent::PromptSubmit).replies([
        answer(json!({"decision": "deny", "reason": "it names a key"})),
        nothing(),
    ]));
    world.run(async {
        rig.prompt("use the key sk-123");
        rig.events.turns(1).await;
    });
    assert_eq!(world.provider.requests().len(), 0, "the model was sent nothing");
    assert!(rig.store.items().is_empty(), "nothing was recorded");
    let end = &rig.events.ends()[0];
    assert_eq!(end.outcome, TurnOutcome::Denied);
    let error = end.error.as_ref().expect("it says why");
    assert_eq!(error.kind, ErrorKind::Permission);
    assert_eq!(error.code, code::PROMPT_DENIED);
    assert!(error.message.contains("`no-keys`: it names a key"), "{error:?}");
    let returned = returned(&rig);
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].parts, [Part::text("use the key sk-123")]);
    assert_eq!(returned[0].delivery, None);
    assert_eq!(returned[0].hooks, [reason("no-keys", "it names a key")]);
    let input = &hook.inputs()[0];
    assert_eq!(input["event"], "prompt.submit");
    assert_eq!(input["prompt"], "use the key sk-123");

    world.run(async {
        rig.prompt("say hello");
        rig.events.turns(2).await;
    });
    assert_eq!(rig.events.ends()[1].outcome, TurnOutcome::Completed);
    assert_eq!(user_texts(&rig), ["say hello"]);
    assert_valid_history(&last_request(&world.provider).messages);
}

/// A steering message a hook refuses is dropped and given back, and the
/// running turn goes on without it.
#[test]
fn a_steering_message_a_hook_refuses_is_dropped_while_the_turn_goes_on() {
    let response = calls(&[("c1", "read", json!({"path": "a", "gate": "tool"}))]);
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_steer_denied", [response, says("Done.")]);
    hooks.add(FakeHook::on("no-keys", HookEvent::PromptSubmit).replies([
        nothing(),
        answer(json!({"decision": "deny", "reason": "it names a key"})),
    ]));
    world.run(async {
        rig.prompt("go");
        world.log.has("start read a").await;
        rig.steer("the key is sk-123");
        world.gates.open("tool");
        rig.events.turns(1).await;
    });
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Completed);
    assert_eq!(world.provider.requests().len(), 2, "the turn went on");
    assert_eq!(user_texts(&rig), ["go"], "the steering message was dropped");
    let returned = returned(&rig);
    assert_eq!(returned.len(), 1);
    assert_eq!(returned[0].parts, [Part::text("the key is sk-123")]);
    assert_eq!(returned[0].delivery, Some(Delivery::Steer));
    assert!(returned[0].message.contains("this steering message"));
    assert_valid_history(&last_request(&world.provider).messages);
}

/// A hook's context is a notice after the prompt, under a line that names
/// the hook; with two hooks, each is a notice of its own.
#[test]
fn a_hooks_context_follows_the_prompt_as_a_notice_naming_the_hook() {
    let Hooked {
        world, rig, hooks, ..
    } = hooked("hook_context", [says("Seen.")]);
    hooks.add(
        FakeHook::on("ticket", HookEvent::PromptSubmit)
            .replies([answer(json!({"context": "Ticket 42 is about the login page."}))]),
    );
    hooks.add(FakeHook::on("quiet", HookEvent::PromptSubmit));
    world.run(async {
        rig.prompt("fix it");
        rig.events.turns(1).await;
    });
    let items = rig.store.items();
    let prompt = items
        .iter()
        .position(|item| matches!(item, Item::UserMessage(_)))
        .expect("the prompt");
    let Item::Notice(notice) = &items[prompt + 1] else {
        panic!("a notice follows the prompt: {items:?}");
    };
    assert_eq!(notice.source, NoticeSource::Hook);
    assert!(notice.text.starts_with("The hook `ticket` adds this"), "{}", notice.text);
    assert!(notice.text.ends_with("Ticket 42 is about the login page."));
    assert_eq!(items.len(), prompt + 3, "the prompt, one notice, the answer");
    let request = last_request(&world.provider);
    let sent: Vec<String> = request.messages.iter().map(text_of).collect();
    assert!(sent.iter().any(|text| text.contains("Ticket 42")), "{sent:?}");
}

/// A prompt hook that fails or does not answer in its time refuses the
/// prompt, with the failure named.
#[test]
fn a_prompt_hook_that_fails_or_times_out_refuses_the_prompt() {
    for stuck in [false, true] {
        let Hooked {
            world,
            rig,
            hooks,
            timer,
        } = hooked("hook_prompt_fails", [says("Unused.")]);
        let reply = if stuck {
            Reply::Never
        } else {
            fails("its program is missing")
        };
        hooks.add(FakeHook::on("checker", HookEvent::PromptSubmit).replies([reply]));
        world.run(async {
            rig.prompt("go");
            if stuck {
                timer.sleeping_for(Duration::from_secs(5)).await;
                timer.advance(Duration::from_secs(5));
            }
            rig.events.turns(1).await;
        });
        assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Denied);
        assert_eq!(world.provider.requests().len(), 0);
        let named = &returned(&rig)[0].hooks[0];
        let why = named.reason.clone().unwrap_or_default();
        assert!(why.starts_with("It failed: "), "{why}");
        let expected = if stuck {
            "did not answer within 5 s"
        } else {
            "its program is missing"
        };
        assert!(why.contains(expected), "{why}");
    }
}

/// An interrupt while a prompt's hooks are asked ends the turn at once:
/// the prompt is not recorded, and it is given back.
#[test]
fn an_interrupt_while_the_prompts_hooks_are_asked_gives_the_prompt_back() {
    let Hooked {
        world,
        rig,
        hooks,
        timer,
    } = hooked("hook_prompt_interrupted", [says("Unused.")]);
    hooks.add(FakeHook::on("slow", HookEvent::PromptSubmit).replies([Reply::Never]));
    world.run(async {
        rig.prompt("go");
        timer.sleeping().await;
        let _ = rig.handle.interrupt();
        rig.events.turns(1).await;
    });
    assert_eq!(rig.events.ends()[0].outcome, TurnOutcome::Interrupted);
    assert!(rig.store.items().is_empty());
    let returned = returned(&rig);
    assert_eq!(returned[0].parts, [Part::text("go")]);
    assert!(returned[0].hooks.is_empty());
    assert!(timer.running().is_empty(), "the hook's wait was given up");
}

// ------------------------------------------------------------- one story

/// The hooks of one session, turn after turn: a prompt with context, then a
/// call a hook replaced and the user allowed; a prompt a hook refused; a
/// call a hook asked about, beside a steering message a hook refused. The
/// history stays whole throughout.
#[test]
fn hooks_across_the_turns_of_one_session() {
    let Hooked {
        world, rig, hooks, ..
    } = hooked(
        "hook_story",
        [
            calls(&[("c1", "write", json!({"path": "a"}))]),
            says("Written."),
            calls(&[("c2", "shell", json!({"id": "test", "gate": "sh"}))]),
            says("Tested."),
        ],
    );
    hooks.add(FakeHook::on("prompts", HookEvent::PromptSubmit).replies([
        answer(json!({"context": "Work in the copy."})),
        answer(json!({"decision": "deny", "reason": "not on Fridays"})),
        nothing(),
        answer(json!({"decision": "deny", "reason": "too vague"})),
    ]));
    hooks.add(FakeHook::on("calls", HookEvent::ToolBefore).replies([
        answer(json!({"arguments": {"path": "copy"}})),
        answer(json!({"decision": "ask", "reason": "tests take a while"})),
    ]));
    rig.policy.answer_all(Verdict::Allow);

    world.run(async {
        rig.prompt("write a");
        rig.events.turns(1).await;
        rig.prompt("deploy");
        rig.events.turns(2).await;
        rig.prompt("run the tests");
        world.log.has("start shell test").await;
        rig.steer("faster");
        world.gates.open("sh");
        rig.events.turns(3).await;
    });

    let outcomes: Vec<TurnOutcome> = rig.events.ends().into_iter().map(|end| end.outcome).collect();
    assert_eq!(
        outcomes,
        [TurnOutcome::Completed, TurnOutcome::Denied, TurnOutcome::Completed]
    );
    assert_eq!(world.log.count("end write copy"), 1);
    assert_eq!(world.log.count("end shell test"), 1);
    assert_eq!(rig.policy.asked(), ["write copy", "shell test"]);
    assert_eq!(user_texts(&rig), ["write a", "run the tests"]);
    let returned: Vec<String> = returned(&rig)
        .into_iter()
        .flat_map(|returned| returned.parts)
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.text),
            _ => None,
        })
        .collect();
    assert_eq!(returned, ["deploy", "faster"]);
    assert_eq!(world.provider.requests().len(), 4);
    assert_valid_history(&last_request(&world.provider).messages);
}
