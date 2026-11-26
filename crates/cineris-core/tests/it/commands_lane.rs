//! An extension's command as a lane sends it, and a running turn's stop
//! while a command's script runs (the review of step 28's commands). One
//! world and one session kept in memory.

use cineris_core::{ClientInfo, OpError, Outgoing};
use cineris_protocol::error::code;
use cineris_protocol::event::{Event, TurnOutcome};
use cineris_protocol::methods::{
    ApprovalChoice, CommandCall, Op, OpResult, SessionRef, TurnInterruptParams, TurnStartParams,
};
use cineris_protocol::session::SessionSettings;
use cineris_protocol::{ErrorKind, Item, SessionEvent, StopReason};
use cineris_testkit::{Gate, Hold, Response};
use serde_json::json;

use crate::{Front, World, says};

/// One command that says what it was given, one for each host call that a
/// running turn's stop refuses: a program, a fetch and a read; and one that
/// reads a file that looks like a secret, which is asked about.
const COMMANDS: &str = "cineris.command({ name: 'say', description: 'Says', run(input) {\n\
       return 'said ' + input.args;\n\
     } });\n\
     cineris.command({ name: 'exec', description: 'Runs', async run() {\n\
       await cineris.exec('probe-echo', []);\n\
       return 'ran';\n\
     } });\n\
     cineris.command({ name: 'fetch', description: 'Fetches', async run() {\n\
       await cineris.http.fetch('https://api.example.com/x');\n\
       return 'fetched';\n\
     } });\n\
     cineris.command({ name: 'read', description: 'Reads', async run() {\n\
       await cineris.fs.read('notes.txt');\n\
       return 'read';\n\
     } });\n\
     cineris.command({ name: 'peek', description: 'Reads the secrets', async run() {\n\
       await cineris.fs.read('.env');\n\
       return 'peeked';\n\
     } });\n";

/// The protected path the model writes, which nobody can be asked about.
const PROTECTED: &str = ".github/workflows/ci.yml";

/// A turn whose model waits for `gate`, then writes a protected path: with
/// nobody to ask and `fail_on_denial`, that stops the turn.
fn stopped_at(gate: &Gate) -> Response {
    Response::new()
        .wait(gate)
        .tool_call(
            "w1",
            "write",
            &json!({"path": PROTECTED, "content": "on: push\n"}),
        )
        .end(StopReason::ToolCalls)
}

/// A `turn/start` that names `name`.
fn command(front: &Front, name: &str, args: &str) -> Op {
    Op::TurnStart(TurnStartParams {
        session_id: front.session.clone(),
        input: Vec::new(),
        delivery: None,
        command: Some(CommandCall {
            name: name.to_owned(),
            args: args.to_owned(),
        }),
    })
}

/// What came of an operation, in words, for a test's message.
fn shown(ended: &Result<OpResult, OpError>) -> String {
    match ended {
        Ok(_) => "ok".to_owned(),
        Err(error) => format!("{} {:?}: {}", error.code, error.kind, error.message),
    }
}

/// Opens the session again for a client that closed it.
async fn reopen(front: &Front) {
    front
        .op(Op::SessionOpen(SessionRef {
            session_id: front.session.clone(),
        }))
        .await
        .expect("the session opens again");
}

/// How many turns have started, of what a client was sent.
fn started(seen: &[Outgoing]) -> usize {
    seen.iter()
        .filter(|outgoing| {
            matches!(
                outgoing,
                Outgoing::Event(SessionEvent {
                    event: Event::TurnStarted(_),
                    ..
                })
            )
        })
        .count()
}

/// - A command's `turn/start` is on the session from when it is sent: one
///   still waiting in its lane behind another command, when an interrupt is
///   sent behind both, ends interrupted with the one running, before its
///   script runs; nothing is recorded and no turn starts. The lane is the
///   core's API as `cineris_rpc::lane` drives it: `Core::early` as each
///   operation is sent, then `Core::handle` one at a time, in that order.
/// - A running turn stopped while a steered command's script runs refuses
///   the script's next host call with the stop, before it is made: a
///   program, a fetch and a read. The command fails, and it is no stop of
///   its own: nothing says it was stopped, and the next prompt goes.
/// - An interrupt reaches only the commands sent before it: one sent behind
///   it through the same lane runs, its step after the interrupt's (L2).
/// - A command's question still open when the running turn stops: allowed
///   after, its host call is refused with the stop (L1). Its asker leaves,
///   the turn's own write then has nobody to ask and stops it, and the asker,
///   back, is put the question again.
/// - A command sent through the lane whose step is refused before it begins
///   (its client left the session) leaves nothing on the session: a later
///   one of the same client and call is its own, and an interrupt sent
///   between does not reach it (L3).
/// - A command sent through the lane, held before its script, as its session
///   closes: the close reaches its run, which ends interrupted (M2).
#[test]
fn a_queued_command_is_interrupted_and_a_running_turns_stop_refuses_a_commands_host_calls() {
    let gates = [Gate::new(), Gate::new(), Gate::new()];
    let held = Gate::new();
    let world = World::new(
        "commands_lane",
        gates
            .iter()
            .map(stopped_at)
            .chain([says("Said."), stopped_at(&held), says("Said.")]),
    );
    world.put("notes.txt", "notes\n");
    world.put(".env", "KEY=1\n");
    let tools = world.dir.join("tools");
    crate::ext_exec::place_echo(&tools, "probe-echo");
    let env = crate::ext_exec::path_with(&world, &[&tools], &[]);
    let probe = crate::ext_exec::extension_running(
        &world,
        "probe",
        "exec = [\"probe-echo\"]\nnet = [\"api.example.com\"]\nread = [\"**\"]",
        COMMANDS,
    );
    let core = world.core_with(|options| {
        options.extensions = vec![probe];
        options.shell_env = Some(env);
        options.fail_on_denial = true;
    });
    world.run(async {
        let front = Front::start_as(
            &world,
            &core,
            ClientInfo::headless(),
            SessionSettings::default(),
        )
        .await
        .unwrap();
        let id = front.client.id;
        let early = |op: &Op| front.core.early(id, op);
        let interrupt = Op::TurnInterrupt(TurnInterruptParams::new(front.session.clone()));

        // The first runs, held before its script; the second and the
        // interrupt are sent behind it.
        let (first, second) = (
            command(&front, "probe:say", "one"),
            command(&front, "probe:say", "two"),
        );
        early(&first);
        let scripts = Hold::new(cineris_core::A_SCRIPT_COMMAND_RUNS);
        let mut running = std::pin::pin!(front.core.handle(id, first));
        scripts.drive_until_reached(running.as_mut()).await;
        early(&second);
        early(&interrupt);
        scripts.release();
        for (which, ended) in [
            ("the running one", running.await),
            ("the queued one", front.core.handle(id, second).await),
        ] {
            let error = ended.expect_err(which);
            assert_eq!(
                (error.code.as_str(), error.kind.clone()),
                (code::COMMAND_FAILED, ErrorKind::Interrupted),
                "{which}: {}",
                error.message
            );
        }
        assert_eq!(scripts.arrivals(), 1, "the queued command's script ran");
        drop(scripts);
        front.core.handle(id, interrupt.clone()).await.unwrap();
        assert!(
            front.ends().is_empty(),
            "an interrupted command started a turn"
        );

        // Each host call, in a turn of its own stopped while the command's
        // script waits to run.
        for (turn, (gate, name)) in gates
            .iter()
            .zip(["probe:exec", "probe:fetch", "probe:read"])
            .enumerate()
        {
            front.prompt("change the workflow").await;
            front.until(|seen| started(seen) > turn).await;
            let scripts = Hold::new(cineris_core::A_SCRIPT_COMMAND_RUNS);
            let mut steered = std::pin::pin!(front.core.handle(id, command(&front, name, "")));
            scripts.drive_until_reached(steered.as_mut()).await;
            gate.open();
            front.turns(turn + 1).await;
            scripts.release();
            drop(scripts);
            let error = steered.await.expect_err(name);
            assert!(
                error.code == code::COMMAND_FAILED
                    && error.message.contains("running turn's stop")
                    && error.message.contains("ci.yml"),
                "/{name}: {}: {}",
                error.code,
                error.message
            );
        }
        let ends = front.ends();
        assert!(
            ends.len() == 3 && ends.iter().all(|end| end.outcome == TurnOutcome::Denied),
            "{ends:?}"
        );
        assert!(
            !front
                .notices()
                .iter()
                .any(|notice| notice.contains("The command `/probe:")),
            "a stop the command found was told as its own: {:?}",
            front.notices()
        );
        assert!(
            !front
                .items()
                .iter()
                .any(|item| matches!(item, Item::UserMessage(user) if user.command.is_some())),
            "a command's prompt was recorded"
        );

        // L2: sent in this order, an interrupt then a command.
        let after = command(&front, "probe:say", "after");
        early(&interrupt);
        early(&after);
        front.core.handle(id, interrupt.clone()).await.unwrap();
        let ended = front.core.handle(id, after).await;
        assert!(
            ended.is_ok(),
            "a command sent after the interrupt was stopped by it: {}",
            shown(&ended)
        );
        front.turns(4).await;

        // L1: the running turn is the fifth; the user's front end is asked.
        let user = front.join(ClientInfo::user()).await;
        front.prompt("change the workflow").await;
        front.until(|seen| started(seen) > 4).await;
        let mut peeking = std::pin::pin!(front.core.handle(id, command(&front, "probe:peek", "")));
        let question = {
            let mut asked = std::pin::pin!(user.approval());
            std::future::poll_fn(|context| {
                assert!(
                    peeking.as_mut().poll(context).is_pending(),
                    "the command ended before it was asked"
                );
                asked.as_mut().poll(context).map(|(id, _)| id)
            })
            .await
        };
        user.close().await;
        held.open();
        front.turns(5).await;
        assert_eq!(front.ends()[4].outcome, TurnOutcome::Denied);
        reopen(&user).await;
        user.answer(&question, ApprovalChoice::AllowOnce).unwrap();
        let error = peeking
            .await
            .expect_err("the command read the secrets after the running turn stopped");
        assert!(
            error.code == code::COMMAND_FAILED
                && error.message.contains("running turn's stop")
                && error.message.contains("ci.yml"),
            "{}: {}",
            error.code,
            error.message
        );

        // L3: refused before its step begins, its client gone from the
        // session.
        let again = command(&front, "probe:say", "again");
        user.core.early(user.client.id, &again);
        user.close().await;
        let error = user
            .core
            .handle(user.client.id, again.clone())
            .await
            .expect_err("a client that left the session started a turn in it");
        assert_eq!(error.code, code::SESSION_NOT_OPEN, "{}", error.message);
        reopen(&user).await;
        early(&interrupt);
        front.core.handle(id, interrupt.clone()).await.unwrap();
        user.core.early(user.client.id, &again);
        let ended = user.core.handle(user.client.id, again).await;
        assert!(
            ended.is_ok(),
            "a command took the run of one refused before it: {}",
            shown(&ended)
        );
        front.turns(6).await;

        // M2: held before its script as the session closes.
        user.close().await;
        let last = command(&front, "probe:say", "last");
        early(&last);
        let scripts = Hold::new(cineris_core::A_SCRIPT_COMMAND_RUNS);
        let mut closing = std::pin::pin!(front.core.handle(id, last));
        scripts.drive_until_reached(closing.as_mut()).await;
        front.close().await;
        scripts.release();
        drop(scripts);
        let error = closing
            .await
            .expect_err("a command ran on after its session closed");
        assert_eq!(
            (error.code.as_str(), error.kind.clone()),
            (code::COMMAND_FAILED, ErrorKind::Interrupted),
            "{}",
            error.message
        );
    });
}
