//! Events on their way to a front end: a slow client, and what an item in
//! progress holds (`docs/architecture/02-runtime.md`, "Event
//! backpressure"; `docs/architecture/11-rpc-protocol.md`, "Reconnecting
//! and several clients").

use cineris_core::{ClientInfo, Outgoing};
use cineris_protocol::event::{Delta, Event};
use cineris_protocol::methods::{Op, OpResult, SessionReadParams};
use cineris_protocol::{Item, ItemId, Part, StopReason};
use cineris_testkit::{Gate, Response};

use crate::{Front, World, says, settle};

async fn read(
    front: &Front,
    item: Option<ItemId>,
    limit: Option<u32>,
    cursor: Option<String>,
) -> cineris_protocol::methods::SessionReadResult {
    let result = front
        .op(Op::SessionRead(SessionReadParams {
            session_id: front.session.clone(),
            item_id: item,
            limit,
            cursor,
            redacted: false,
        }))
        .await
        .unwrap();
    let OpResult::SessionRead(result) = result else {
        panic!("not items");
    };
    result
}

fn text_of(item: &Item) -> String {
    match item {
        Item::AssistantMessage(message) => message
            .parts
            .iter()
            .filter_map(|part| match part {
                Part::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect(),
        _ => String::new(),
    }
}

fn text_deltas(front: &Front) -> Vec<String> {
    front
        .events()
        .into_iter()
        .filter_map(|event| match event.event {
            Event::ItemDelta(delta) => match delta.delta {
                Delta::Text(text) => Some(text.text),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn an_item_in_progress_can_be_read_with_what_it_has_so_far() {
    let gate = Gate::new();
    let world = World::new(
        "in_progress",
        [Response::new()
            .text_in(&["The build ", "fails because"])
            .wait(&gate)
            .text(" of a missing file.")
            .end(StopReason::End)],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.prompt("why does the build fail?").await;
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(outgoing, Outgoing::Event(event)
                        if matches!(&event.event, Event::ItemDelta(delta)
                            if matches!(&delta.delta, Delta::Text(text) if text.text.contains("because"))))
                })
            })
            .await;

        // A client that connects now reads the session: the prompt, and
        // the answer as far as it has come, marked as not complete.
        let late = front.join(ClientInfo::user()).await;
        let result = read(&late, None, None, None).await;
        assert_eq!(result.items.len(), 2);
        assert!(matches!(result.items[0], Item::UserMessage(_)));
        assert_eq!(text_of(&result.items[1]), "The build fails because");
        let Item::AssistantMessage(partial) = &result.items[1] else {
            panic!("not an assistant message");
        };
        assert_eq!(partial.stop_reason, None);
        assert_eq!(result.incomplete, std::slice::from_ref(&partial.id));
        assert!(result.seq > 0);
        // That one item, by its id.
        let one = read(&late, Some(partial.id.clone()), None, None).await;
        assert_eq!(one.items.len(), 1);
        assert_eq!(one.incomplete.len(), 1);

        gate.open();
        front.turns(1).await;
        let result = read(&late, None, None, None).await;
        assert!(result.incomplete.is_empty());
        assert_eq!(
            text_of(&result.items[1]),
            "The build fails because of a missing file."
        );
        // Paged: one item at a time, with a cursor to the next.
        let first = read(&late, None, Some(1), None).await;
        assert_eq!(first.items.len(), 1);
        let second = read(&late, None, Some(1), first.next_cursor.clone()).await;
        assert_eq!(text_of(&second.items[0]), text_of(&result.items[1]));
        assert_eq!(second.next_cursor, None);
        front.close().await;
    });
}

/// A client behind its queue, holding deltas merged while it read nothing,
/// reads the session while the answer streams: the read's text and the
/// deltas after the read's `seq` hold the answer exactly once. The read
/// fences the client's queue, so no delta after it is merged into one
/// before it (the review of step 23's fixes).
#[test]
fn a_read_by_a_client_behind_and_the_deltas_after_it_hold_the_answer_once() {
    let gate = Gate::new();
    let world = World::new(
        "read_behind",
        [Response::new()
            .text_in(&["The ", "build ", "fails "])
            .wait(&gate)
            .text_in(&["because ", "of ", "a file."])
            .end(StopReason::End)],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        // A second client that reads nothing until the answer has begun:
        // its queue merges the first deltas into one.
        let behind = front.join(ClientInfo::user()).await;
        front.prompt("why does the build fail?").await;
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(outgoing, Outgoing::Event(event)
                        if matches!(&event.event, Event::ItemDelta(delta)
                            if matches!(&delta.delta, Delta::Text(text) if text.text.contains("fails"))))
                })
            })
            .await;
        settle().await;
        let result = read(&behind, None, None, None).await;
        let answer = result
            .items
            .iter()
            .find(|item| matches!(item, Item::AssistantMessage(_)))
            .expect("the answer in progress");
        let mut drawn = text_of(answer);
        assert_eq!(drawn, "The build fails ");
        gate.open();
        front.turns(1).await;
        behind.turns(1).await;
        for event in behind.events() {
            if event.seq > result.seq
                && let Event::ItemDelta(delta) = event.event
                && let Delta::Text(text) = delta.delta
            {
                drawn.push_str(&text.text);
            }
        }
        assert_eq!(drawn, "The build fails because of a file.");
        front.close().await;
    });
}

#[test]
fn a_client_that_falls_behind_gets_fewer_deltas_one_resync_and_every_whole_event() {
    let gate = Gate::new();
    let pieces: Vec<String> = (0..200).map(|n| format!("word{n} ")).collect();
    let before: Vec<&str> = pieces[..100].iter().map(String::as_str).collect();
    let after: Vec<&str> = pieces[100..].iter().map(String::as_str).collect();
    let world = World::new(
        "slow_client",
        [Response::new()
            .text_in(&before)
            .wait(&gate)
            .text_in(&after)
            .end(StopReason::End)],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        // A second client with a short queue, which reads nothing for now.
        let slow = front
            .join(ClientInfo {
                queue: 4,
                ..ClientInfo::user()
            })
            .await;
        front.prompt("say a lot").await;
        // The client that keeps up sees the text arrive.
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(outgoing, Outgoing::Event(event)
                        if matches!(&event.event, Event::ItemDelta(delta)
                            if matches!(&delta.delta, Delta::Text(text) if text.text.contains("word99"))))
                })
            })
            .await;
        settle().await;
        // The slow one's queue did not grow with the deltas: a hundred
        // pieces arrived, and it holds a handful of events.
        assert!(slow.client.queue.len() <= 8, "{}", slow.client.queue.len());

        // It catches up: what it is owed is one resync for the item, and
        // the item's content so far is there to be read.
        slow.drain();
        let resyncs: Vec<ItemId> = slow
            .events()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::ItemResync(resync) => Some(resync.item_id),
                _ => None,
            })
            .collect();
        assert_eq!(resyncs.len(), 1, "{:?}", slow.events());
        let partial = read(&slow, Some(resyncs[0].clone()), None, None).await;
        assert!(text_of(&partial.items[0]).ends_with("word99 "));
        assert_eq!(partial.incomplete, resyncs);

        gate.open();
        front.turns(1).await;
        slow.turns(1).await;
        // No delta of the item after its resync, and its completion whole.
        let late: Vec<String> = text_deltas(&slow)
            .into_iter()
            .filter(|text| text.contains("word150"))
            .collect();
        assert!(late.is_empty(), "deltas after the resync: {late:?}");
        let whole = pieces.concat();
        assert!(slow.items().iter().any(|item| text_of(item) == whole));
        // Nothing but deltas was lost: every other event the client that
        // kept up was sent, the slow one was sent too.
        let whole_events = |front: &Front| -> Vec<u64> {
            front
                .events()
                .iter()
                .filter(|event| {
                    !matches!(event.event, Event::ItemDelta(_) | Event::ItemResync(_))
                })
                .map(|event| event.seq)
                .collect()
        };
        let kept_up = whole_events(&front);
        let fell_behind = whole_events(&slow);
        let from = fell_behind.first().copied().unwrap_or(0);
        let expected: Vec<u64> = kept_up.into_iter().filter(|seq| *seq >= from).collect();
        assert_eq!(fell_behind, expected);

        // The client that kept up got all of the text in its deltas.
        assert_eq!(text_deltas(&front).concat(), whole);
        front.close().await;
        slow.close().await;
    });
}

/// A client that stops reading during one long item: the item's deltas
/// merge into one, which never fills the queue by count, so the queue is
/// full by the bytes it holds. The client then holds a bounded number of
/// bytes and one `item.resync`, `session/read` gives the whole item so far,
/// and every whole event still reaches it.
#[test]
fn a_reader_that_stops_during_one_long_item_holds_bounded_bytes_and_one_resync() {
    let gate = Gate::new();
    let pieces: Vec<String> = (0..200)
        .map(|n| format!("{n:04} {}\n", "x".repeat(1018)))
        .collect();
    let before: Vec<&str> = pieces[..100].iter().map(String::as_str).collect();
    let after: Vec<&str> = pieces[100..].iter().map(String::as_str).collect();
    let world = World::new(
        "stopped_reader_bytes",
        [Response::new()
            .text_in(&before)
            .wait(&gate)
            .text_in(&after)
            .end(StopReason::End)],
    );
    world.provider.set_output_limit(false);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        // A queue of 8 places: 64 KiB of deltas.
        let stopped = front
            .join(ClientInfo {
                queue: 8,
                ..ClientInfo::user()
            })
            .await;
        let bound = 8 * cineris_core::events::DELTA_BYTES_PER_PLACE;
        front.prompt("say a lot").await;
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(outgoing, Outgoing::Event(event)
                        if matches!(&event.event, Event::ItemDelta(delta)
                            if matches!(&delta.delta, Delta::Text(text) if text.text.contains("0099 "))))
                })
            })
            .await;
        settle().await;
        // A hundred kibibytes arrived: it holds its bound and no more, in
        // a handful of events.
        let held = stopped.client.queue.delta_bytes();
        assert!(held <= bound + 1024, "{held} bytes held, the bound is {bound}");
        assert!(stopped.client.queue.len() <= 8, "{}", stopped.client.queue.len());
        // What it reads, without taking anything from its queue, is the
        // whole item so far.
        let answer = front
            .events()
            .into_iter()
            .find_map(|event| match event.event {
                Event::ItemDelta(delta) => Some(delta.item_id),
                _ => None,
            })
            .expect("the answer's id");
        let partial = read(&stopped, Some(answer.clone()), None, None).await;
        assert_eq!(text_of(&partial.items[0]), before.concat());
        assert_eq!(partial.incomplete, std::slice::from_ref(&answer));

        // The rest of the turn comes while it still reads nothing, its
        // queue full of bytes: whole events go in all the same.
        gate.open();
        front.turns(1).await;
        stopped.turns(1).await;
        let resyncs: Vec<ItemId> = stopped
            .events()
            .into_iter()
            .filter_map(|event| match event.event {
                Event::ItemResync(resync) => Some(resync.item_id),
                _ => None,
            })
            .collect();
        assert_eq!(resyncs, [answer], "{:?}", stopped.events());
        // What it was sent of the text is less than what came.
        assert!(text_deltas(&stopped).concat().len() < before.concat().len());
        // No delta of the item after its resync, and its completion whole.
        assert!(
            !text_deltas(&stopped)
                .iter()
                .any(|text| text.contains("0150 ")),
            "deltas after the resync"
        );
        let whole = pieces.concat();
        assert!(stopped.items().iter().any(|item| text_of(item) == whole));
        // Every whole event the client that kept up was sent, it was sent.
        let whole_events = |front: &Front| -> Vec<u64> {
            front
                .events()
                .iter()
                .filter(|event| {
                    !matches!(event.event, Event::ItemDelta(_) | Event::ItemResync(_))
                })
                .map(|event| event.seq)
                .collect()
        };
        let from = whole_events(&stopped).first().copied().unwrap_or(0);
        let expected: Vec<u64> = whole_events(&front)
            .into_iter()
            .filter(|seq| *seq >= from)
            .collect();
        assert_eq!(whole_events(&stopped), expected);
        front.close().await;
        stopped.close().await;
    });
}

/// `session/read` pages the log by a cursor of its own, and gives the items
/// in progress whole in the last page, past its limit: a client that reads
/// a running session a page at a time draws each item once, and the answer
/// in progress only in the last page (settled in step 27; paged as one
/// list, an item in progress that completed before an earlier one moved
/// between pages).
#[test]
fn the_items_in_progress_come_whole_in_the_last_page_of_a_read() {
    let gate = Gate::new();
    let world = World::new(
        "read_pages",
        [
            says("First."),
            Response::new()
                .text_in(&["Second "])
                .wait(&gate)
                .text(" answer.")
                .end(StopReason::End),
        ],
    );
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        front.prompt("two").await;
        front
            .until(|seen| {
                seen.iter().any(|outgoing| {
                    matches!(outgoing, Outgoing::Event(event)
                        if matches!(&event.event, Event::ItemDelta(delta)
                            if matches!(&delta.delta, Delta::Text(text) if text.text.contains("Second"))))
                })
            })
            .await;
        let late = front.join(ClientInfo::user()).await;
        let mut pages = Vec::new();
        let mut cursor = None;
        loop {
            let page = read(&late, None, Some(1), cursor).await;
            cursor = page.next_cursor.clone();
            pages.push(page);
            if cursor.is_none() {
                break;
            }
        }
        let (last, before) = pages.split_last().unwrap();
        assert!(!before.is_empty(), "the log has more than one item");
        for page in before {
            assert_eq!(page.items.len(), 1, "{:?}", page.items);
            assert!(page.incomplete.is_empty(), "an item in progress before the last page");
        }
        // The last item of the log, and the answer in progress past the
        // limit.
        assert_eq!(last.items.len(), 2, "{:?}", last.items);
        let Item::AssistantMessage(answer) = &last.items[1] else {
            panic!("not the answer in progress: {:?}", last.items);
        };
        assert_eq!(text_of(&last.items[1]), "Second ");
        assert_eq!(last.incomplete, std::slice::from_ref(&answer.id));
        let mut ids: Vec<&str> = pages
            .iter()
            .flat_map(|page| page.items.iter().filter_map(Item::id))
            .collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "an item drawn twice");

        gate.open();
        front.turns(2).await;
        front.close().await;
        late.close().await;
    });
}
