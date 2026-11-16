//! Closing a view of a session, as built in step 27 pending the owner's
//! word (`docs/outstanding/an-observer-that-ends-a-session.md`,
//! `docs/outstanding/a-closed-view-can-still-answer.md`): an observer never
//! ends a session, and a client that closes its view leaves the session's
//! open questions, as one that disconnects does.

use cortex_core::{ClientInfo, Outgoing};
use cortex_protocol::error::code;
use cortex_protocol::methods::{
    ApprovalChoice, Op, OpResult, QuestionResponse, ServerRequest, ServerResponse, SessionRef,
    SessionUndoParams,
};
use serde_json::json;

use crate::{Front, World, calls, says};

const PROTECTED: &str = ".github/workflows/ci.yml";

/// An observer's close never ends a session, though it is the last client
/// on it: only a client that may drive a session ends it.
#[test]
fn an_observers_close_never_ends_a_session() {
    let world = World::new("observer_closes", [says("Hi.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let observer = front.join(ClientInfo::observer()).await;
        front.close().await;
        assert_eq!(core.open_sessions(), vec![front.session.clone()]);
        observer.close().await;
        assert_eq!(
            core.open_sessions(),
            vec![front.session.clone()],
            "an observer's close ended the session"
        );
        // A user that opens it and closes it ends it.
        let user = front.join(ClientInfo::user()).await;
        user.close().await;
        assert_eq!(core.open_sessions(), Vec::new());
    });
}

/// The same for a disconnect: an observer's never ends a session, a user's
/// that leaves nobody does (the core keeps no sessions here).
#[test]
fn an_observers_disconnect_never_ends_a_session() {
    let world = World::new("observer_leaves", [says("Hi.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let observer = front.join(ClientInfo::observer()).await;
        core.disconnect(front.client.id).await;
        core.disconnect(observer.client.id).await;
        assert_eq!(
            core.open_sessions(),
            vec![front.session.clone()],
            "an observer's disconnect ended the session"
        );
        let user = front.join(ClientInfo::user()).await;
        core.disconnect(user.client.id).await;
        assert_eq!(core.open_sessions(), Vec::new());
    });
}

/// A client that closes its view of a session leaves its open questions:
/// its answer is refused, and another's is taken. Opening the session again
/// puts the question to it again.
#[test]
fn a_client_that_closes_its_view_leaves_the_sessions_open_questions() {
    let world = World::new(
        "closed_view",
        [
            calls(&[(
                "c1",
                "write",
                json!({"path": PROTECTED, "content": "on: push\n"}),
            )]),
            says("Refused."),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let other = front.join(ClientInfo::user()).await;
        front.prompt("set up CI").await;
        let (id, _) = other.approval().await;
        other.close().await;
        let refused = other.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
        assert_eq!(refused.code, code::ROLE_NOT_ALLOWED);
        assert_eq!(world.read(PROTECTED), None);

        // Opened again, it is put the question again, and may answer it.
        let opened = other
            .op(Op::SessionOpen(SessionRef {
                session_id: front.session.clone(),
            }))
            .await;
        assert!(matches!(opened, Ok(OpResult::SessionOpen(_))), "{opened:?}");
        other
            .until(|seen| {
                seen.iter()
                    .filter(|outgoing| {
                        matches!(outgoing, Outgoing::Request { id: put, .. } if *put == id)
                    })
                    .count()
                    == 2
            })
            .await;
        other.answer(&id, ApprovalChoice::Deny).unwrap();
        let (same, _) = front.approval().await;
        assert_eq!(same, id);
        let late = front.answer(&id, ApprovalChoice::AllowOnce).unwrap_err();
        assert_eq!(late.code, "request_not_open");
        front.turns(1).await;
        assert_eq!(front.results()[0].1, "denied");
        front.close().await;
        other.close().await;
    });
}

/// A question that ends with its askers (an undo's) ends as no answer when
/// its last asker closes its view, though another client keeps the session
/// open: the undo keeps the file the person edited.
#[test]
fn an_undos_question_ends_when_its_asker_closes_its_view() {
    let world = World::new(
        "undo_view_closed",
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            calls(&[(
                "c2",
                "write",
                json!({"path": "a.txt", "content": "changed\n"}),
            )]),
            says("Changed."),
        ],
    );
    world.put("a.txt", "original\n");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        // Another client, which answers no questions, keeps it open.
        let mut info = ClientInfo::user();
        info.capabilities.questions = false;
        let keeping = front.join(info).await;
        front.turn("change it").await;
        world.put("a.txt", "edited by hand\n");
        let (asking, client) = (core.clone(), front.client.id);
        let op = Op::SessionUndo(SessionUndoParams {
            session_id: front.session.clone(),
            restore_files: true,
        });
        let undoing = tokio::spawn(async move { asking.handle(client, op).await });
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(
                        outgoing,
                        Outgoing::Request {
                            request: ServerRequest::Question(_),
                            ..
                        }
                    )
                })
            })
            .await;
        let id = front
            .seen
            .borrow()
            .iter()
            .find_map(|outgoing| match outgoing {
                Outgoing::Request {
                    id,
                    request: ServerRequest::Question(_),
                } => Some(id.clone()),
                _ => None,
            })
            .unwrap();
        front.close().await;
        undoing.await.unwrap().expect("the undo ends");
        assert_eq!(world.read("a.txt").as_deref(), Some("edited by hand\n"));
        assert_eq!(core.open_sessions(), vec![front.session.clone()]);
        let late = core.respond(
            front.client.id,
            &id,
            ServerResponse::Question(QuestionResponse {
                answer: Some("Put them back".to_owned()),
            }),
        );
        assert_eq!(
            late.map_err(|error| error.code),
            Err("request_not_open".to_owned())
        );
        keeping.close().await;
    });
}
