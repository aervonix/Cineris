//! What step 23 added to the core for the interactive application:
//! `session/undo`, the question an undo or a rewind asks before it
//! overwrites a file the person edited, and `workspace/files`.

use cortex_core::{ClientInfo, Outgoing};
use cortex_protocol::event::{Event, Resolution};
use cortex_protocol::methods::{
    Op, OpResult, QuestionResponse, ServerRequest, ServerResponse, SessionUndoParams,
    WorkspaceFilesParams,
};
use cortex_protocol::session::SessionSettings;
use serde_json::json;

use crate::{Front, World, calls, says};

/// The first turn reads and rewrites `a.txt`; the world is left with it
/// changed.
fn changed_world(test: &str) -> World {
    let world = World::new(
        test,
        [
            calls(&[("c1", "read", json!({"path": "a.txt"}))]),
            calls(&[(
                "c2",
                "write",
                json!({"path": "a.txt", "content": "changed\n"}),
            )]),
            says("Changed."),
            says("After the undo."),
        ],
    );
    world.put("a.txt", "original\n");
    world
}

fn undo(front: &Front) -> Op {
    Op::SessionUndo(SessionUndoParams {
        session_id: front.session.clone(),
        restore_files: true,
    })
}

/// The questions this front has been put, with their ids.
fn questions(front: &Front) -> Vec<(cortex_protocol::jsonrpc::RequestId, String, Vec<String>)> {
    front.drain();
    front
        .seen
        .borrow()
        .iter()
        .filter_map(|outgoing| match outgoing {
            Outgoing::Request {
                id,
                request: ServerRequest::Question(question),
            } => Some((
                id.clone(),
                question.question.clone(),
                question.choices.clone(),
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn undo_steps_back_before_the_first_turn_and_puts_its_files_back() {
    let world = changed_world("undo_first");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        // Nothing to undo before a turn.
        let nothing = front.op(undo(&front)).await.unwrap_err();
        assert_eq!(nothing.code, "nothing_to_undo");

        front.turn("change it").await;
        assert_eq!(world.read("a.txt").as_deref(), Some("changed\n"));
        let undone = front.op(undo(&front)).await.unwrap();
        assert!(matches!(undone, OpResult::SessionUndo(_)));
        assert_eq!(world.read("a.txt").as_deref(), Some("original\n"));
        assert!(questions(&front).is_empty(), "nothing was edited since");

        // The conversation went back too: the next request has the new
        // prompt alone.
        front.turn("again").await;
        let request = world.provider.requests().pop().unwrap();
        assert_eq!(request.messages.len(), 1);
        assert_eq!(front.last_text(), "After the undo.");
        front.close().await;
    });
}

/// A file the person changed after the turn is put back only if they say
/// so: one question, and its first answer overwrites.
#[test]
fn an_undo_asks_before_it_overwrites_a_file_edited_since() {
    for (answer, expected) in [
        (Some("Put them back"), "original\n"),
        (Some("Keep them as they are"), "edited by hand\n"),
        (None, "edited by hand\n"),
    ] {
        let world = changed_world("undo_edited");
        let core = world.core();
        world.run(async {
            let front = Front::start(&world, &core).await;
            front.turn("change it").await;
            world.put("a.txt", "edited by hand\n");

            let (asking, client, op) = (core.clone(), front.client.id, undo(&front));
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
            let asked = questions(&front);
            assert_eq!(asked.len(), 1);
            let (id, question, choices) = &asked[0];
            assert!(question.contains("`a.txt`"), "{question}");
            assert_eq!(choices, &["Put them back", "Keep them as they are"]);
            front
                .core
                .respond(
                    front.client.id,
                    id,
                    ServerResponse::Question(QuestionResponse {
                        answer: answer.map(str::to_owned),
                    }),
                )
                .unwrap();
            undoing.await.unwrap().unwrap();
            assert_eq!(world.read("a.txt").as_deref(), Some(expected), "{answer:?}");
            front.close().await;
        });
    }
}

/// A client that answers no questions is not asked, and the file the
/// person edited is kept, as before step 23.
#[test]
fn with_nobody_to_ask_an_edited_file_is_kept() {
    let world = changed_world("undo_nobody");
    let core = world.core();
    world.run(async {
        let mut info = ClientInfo::user();
        info.capabilities.questions = false;
        let front = Front::start_as(&world, &core, info, SessionSettings::default())
            .await
            .unwrap();
        front.turn("change it").await;
        world.put("a.txt", "edited by hand\n");
        front.op(undo(&front)).await.unwrap();
        assert_eq!(world.read("a.txt").as_deref(), Some("edited by hand\n"));
        assert!(questions(&front).is_empty());
        // Said, so the person knows the file stayed.
        assert!(
            front
                .notices()
                .iter()
                .any(|notice| notice.contains("a.txt")),
            "{:?}",
            front.notices()
        );
        front.close().await;
    });
}

/// The asking client goes away: the undo ends as "keep", the session can
/// be used again, and a client watching it is told the question was
/// withdrawn, not answered.
#[test]
fn a_question_whose_client_went_away_ends_as_no_answer() {
    let world = changed_world("undo_gone");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let watching = front.join(ClientInfo::observer()).await;
        front.turn("change it").await;
        world.put("a.txt", "edited by hand\n");
        let (asking, client, op) = (core.clone(), front.client.id, undo(&front));
        let undoing = tokio::spawn(async move { asking.handle(client, op).await });
        front
            .until(|seen| {
                seen.iter()
                    .any(|outgoing| matches!(outgoing, Outgoing::Request { .. }))
            })
            .await;
        core.disconnect(front.client.id).await;
        // The undo ends: it is not left waiting for an answer nobody can
        // give.
        let _ = undoing.await.unwrap();
        assert_eq!(world.read("a.txt").as_deref(), Some("edited by hand\n"));
        watching.drain();
        let resolutions: Vec<Resolution> = watching
            .events()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::RequestResolved(resolved) => Some(resolved.resolution),
                _ => None,
            })
            .collect();
        assert_eq!(resolutions, [Resolution::Cancelled]);
    });
}

/// Waits until the front has been put a question.
async fn asked(front: &Front) {
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
}

/// The asking client closes the session while the undo's question is open
/// (the review of step 23): the question is withdrawn, the undo ends
/// without changing the ended session or its files, an answer that comes
/// later is refused, and the session can be opened again at once.
#[test]
fn an_undos_question_ends_when_its_session_is_closed() {
    let world = changed_world("undo_closed");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("change it").await;
        world.put("a.txt", "edited by hand\n");
        let (asking, client, op) = (core.clone(), front.client.id, undo(&front));
        let undoing = tokio::spawn(async move { asking.handle(client, op).await });
        asked(&front).await;
        let (id, ..) = questions(&front).remove(0);
        front.close().await;
        let undone = undoing.await.unwrap();
        assert_eq!(
            undone.map(|_| ()).map_err(|error| error.code),
            Err("session_not_open".to_owned())
        );
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
        assert_eq!(world.read("a.txt").as_deref(), Some("edited by hand\n"));
        let other = core.connect(ClientInfo::user());
        let (reopened, _) = crate::not_now(
            &core,
            &other,
            core.handle(
                other.id,
                Op::SessionOpen(cortex_protocol::methods::SessionRef {
                    session_id: front.session.clone(),
                }),
            ),
        )
        .await;
        reopened.expect("the closed session opens again");
    });
}

/// The core's end withdraws an undo's open question (the review of step
/// 23): the undo ends as no answer, and the edited file is kept.
#[test]
fn an_undos_question_ends_when_the_core_ends() {
    let world = changed_world("undo_shutdown");
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("change it").await;
        world.put("a.txt", "edited by hand\n");
        let (asking, client, op) = (core.clone(), front.client.id, undo(&front));
        let undoing = tokio::spawn(async move { asking.handle(client, op).await });
        asked(&front).await;
        core.shutdown().await;
        let _ = undoing.await.unwrap();
        assert_eq!(world.read("a.txt").as_deref(), Some("edited by hand\n"));
    });
}

/// `workspace/files` walks under the core's token (the review of step 23):
/// a core that is ending lists nothing.
#[test]
fn the_workspace_is_not_walked_once_the_core_is_ending() {
    let world = World::new("workspace_files_ending", [says("Nothing.")]);
    world.put("src/new.rs", "new");
    let cancel = cortex_base::CancelToken::new();
    let core = world.core_with(|options| options.cancel = cancel.clone());
    world.run(async {
        let front = Front::start(&world, &core).await;
        cancel.cancel();
        let listed = front
            .op(Op::WorkspaceFiles(WorkspaceFilesParams {
                session_id: front.session.clone(),
                limit: None,
            }))
            .await
            .unwrap();
        let OpResult::WorkspaceFiles(listed) = listed else {
            panic!("not the files");
        };
        assert!(listed.paths.is_empty(), "{:?}", listed.paths);
    });
}

#[test]
fn the_workspace_is_listed_newest_first_as_the_ignore_rules_leave_it() {
    let world = World::new("workspace_files", [says("Nothing.")]);
    world.put(".gitignore", "target/\n");
    world.put("target/debug/out.bin", "x");
    world.put("old.rs", "old");
    world.put("src/new.rs", "new");
    // A later write is newer, whatever the clock's resolution: the old file
    // is given a time well before.
    let old = world.workspace.join("old.rs");
    let earlier = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(earlier)
        .unwrap();
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        let listed = front
            .op(Op::WorkspaceFiles(WorkspaceFilesParams {
                session_id: front.session.clone(),
                limit: None,
            }))
            .await
            .unwrap();
        let OpResult::WorkspaceFiles(listed) = listed else {
            panic!("not the files");
        };
        assert!(!listed.truncated);
        assert!(
            !listed.paths.iter().any(|path| path.starts_with("target/")),
            "{:?}",
            listed.paths
        );
        let new = listed.paths.iter().position(|path| path == "src/new.rs");
        let old = listed.paths.iter().position(|path| path == "old.rs");
        assert!(new.is_some() && old.is_some(), "{:?}", listed.paths);
        assert!(new < old, "newest first: {:?}", listed.paths);

        let one = front
            .op(Op::WorkspaceFiles(WorkspaceFilesParams {
                session_id: front.session.clone(),
                limit: Some(1),
            }))
            .await
            .unwrap();
        let OpResult::WorkspaceFiles(one) = one else {
            panic!("not the files");
        };
        assert_eq!(one.paths.len(), 1);
        assert!(one.truncated);
        front.close().await;
    });
}
