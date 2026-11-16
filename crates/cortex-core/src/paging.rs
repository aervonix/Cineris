//! The pages of `session/read` (`docs/architecture/11-rpc-protocol.md`,
//! "As built in step 27").
//!
//! The log is paged by a cursor of its own, the number of its items given
//! so far, and the items in progress come whole in the last page, past the
//! page's limit. So nothing moves between pages: an item that completes
//! between two pages joins the log after the cursor, where the next page
//! finds it, and the items in progress are never split by a page's end.
//! (Paged as one list, the log followed by the items in progress, an item
//! that completed before an earlier one moved into the log ahead of the
//! cursor: the next page gave an item drawn already, and never the one that
//! completed. Found by the review of step 23's fixes.)
//!
//! **A cursor names its session and where it stands in the log:** the
//! session's id, the number of the log's items given, and the id of the
//! last of them. A cursor of another session, one past the end of the log,
//! and one whose last item is not where it was (a rewind or an undo moved
//! the session to another branch since) are refused `invalid_cursor`, and
//! the client reads again from the start (the review of step 27's second
//! pull request). An item's place on a branch fixes every item before it,
//! so the last one is enough.

use cortex_protocol::{ErrorKind, Item, ItemId, SessionId};

use crate::core::{OpError, error};

/// One page of a session's items.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Page {
    /// The items of the page: of the log, in its order, and on the last
    /// page the items in progress after them.
    pub(crate) items: Vec<Item>,
    /// The items of the page that are in progress.
    pub(crate) incomplete: Vec<ItemId>,
    /// Where the next page starts, when there is one.
    pub(crate) next_cursor: Option<String>,
}

/// The page of `session`'s `logged`, the log's items on its branch, and
/// `progress`, the items in progress taken with the read's `seq`, that a
/// read asks for: the one item `wanted`, or the log from `cursor`, at most
/// `limit` of it, and on the last page every item in progress.
///
/// # Errors
///
/// `invalid_cursor` when `cursor` is not one a page of this session gave
/// on this branch of its log.
pub(crate) fn page(
    session: &SessionId,
    logged: Vec<Item>,
    progress: Vec<Item>,
    wanted: Option<&ItemId>,
    cursor: Option<&str>,
    limit: Option<u32>,
) -> Result<Page, OpError> {
    // An item that completed between the two looks is in the log already,
    // and is not also given as in progress.
    let progress: Vec<Item> = progress
        .into_iter()
        .filter(|item| {
            item.id()
                .is_some_and(|id| !logged.iter().any(|logged| logged.id() == Some(id)))
        })
        .collect();
    let ids = |items: &[Item]| -> Vec<ItemId> {
        items
            .iter()
            .filter_map(|item| item.id().map(ItemId::new))
            .collect()
    };
    if let Some(wanted) = wanted {
        let is = |item: &Item| item.id() == Some(wanted.as_str());
        if let Some(found) = logged.into_iter().find(is) {
            return Ok(Page {
                items: vec![found],
                incomplete: Vec::new(),
                next_cursor: None,
            });
        }
        let found: Vec<Item> = progress.into_iter().filter(is).collect();
        return Ok(Page {
            incomplete: ids(&found),
            items: found,
            next_cursor: None,
        });
    }
    let start = match cursor {
        None => 0,
        Some(text) => start_of(session, &logged, text).ok_or_else(|| {
            error(
                ErrorKind::Usage,
                cortex_protocol::error::code::INVALID_CURSOR,
                "That cursor is not one this server gave for this session's `session/read`, or \
                 the session's history has changed since (a rewind or an undo): read again from \
                 the start.",
            )
        })?,
    };
    let limit = limit.map_or(usize::MAX, |limit| limit as usize);
    let total = logged.len();
    let next_cursor = |next: usize, last: Option<&Item>| {
        format!(
            "{session} {next} {}",
            last.and_then(Item::id).unwrap_or(NO_ITEM)
        )
    };
    let mut items: Vec<Item> = logged.into_iter().skip(start).take(limit).collect();
    let next = start.saturating_add(items.len());
    if next < total {
        // At `start` with nothing taken (a limit of 0), the cursor is the
        // one given.
        let cursor = match items.last() {
            Some(last) => next_cursor(next, Some(last)),
            None => cursor.map_or_else(|| next_cursor(0, None), str::to_owned),
        };
        return Ok(Page {
            items,
            incomplete: Vec::new(),
            next_cursor: Some(cursor),
        });
    }
    let incomplete = ids(&progress);
    items.extend(progress);
    Ok(Page {
        items,
        incomplete,
        next_cursor: None,
    })
}

/// What a cursor names in place of the last item's id at the start of the
/// log, and for an item with no id.
const NO_ITEM: &str = "-";

/// Where in `logged` the cursor `text` starts a page: `None` when it is not
/// a cursor of `session`, or does not stand where it was in the log.
fn start_of(session: &SessionId, logged: &[Item], text: &str) -> Option<usize> {
    let mut parts = text.splitn(3, ' ');
    let (named, count, last) = (parts.next()?, parts.next()?, parts.next()?);
    if named != session.as_str() {
        return None;
    }
    let count = count.parse::<usize>().ok()?;
    // Past the end of the log there is no item to stand on.
    let at = match count.checked_sub(1) {
        None => NO_ITEM,
        Some(index) => logged.get(index)?.id().unwrap_or(NO_ITEM),
    };
    (at == last).then_some(count)
}

#[cfg(test)]
mod tests {
    use cortex_protocol::item::{AssistantMessageItem, NoticeItem, NoticeSource};

    use super::*;

    fn session() -> SessionId {
        SessionId::new("s_1")
    }

    /// An item of the log.
    fn logged(id: &str) -> Item {
        Item::Notice(NoticeItem {
            id: ItemId::new(id),
            source: NoticeSource::Harness,
            text: format!("{id} is in the log"),
            created_at: None,
        })
    }

    /// An item in progress.
    fn running(id: &str) -> Item {
        Item::AssistantMessage(AssistantMessageItem {
            id: ItemId::new(id),
            parts: Vec::new(),
            stop_reason: None,
            raw_stop: None,
            stop_detail: None,
            truncated: false,
            origin: None,
            usage: None,
            error: None,
            created_at: None,
        })
    }

    fn names(page: &Page) -> Vec<&str> {
        page.items.iter().filter_map(Item::id).collect()
    }

    /// Two items in progress at once, and the second completes before the
    /// first between two pages: every item is drawn once, the one that
    /// completed from the log and the other still in progress, and the two
    /// were never split by a page's end. Paged as one list with a limit of
    /// four, the first page held `a` in progress, and the second gave `a`
    /// again and never `b`.
    #[test]
    fn nothing_moves_between_pages_when_items_in_progress_end_out_of_order() {
        let log = ["x0", "x1", "x2"].map(logged).to_vec();
        let both = vec![running("a"), running("b")];

        // The limit falls inside the items in progress: they come whole.
        let first = page(&session(), log.clone(), both.clone(), None, None, Some(4)).unwrap();
        assert_eq!(names(&first), ["x0", "x1", "x2", "a", "b"]);
        assert_eq!(first.incomplete, [ItemId::new("a"), ItemId::new("b")]);
        assert_eq!(first.next_cursor, None);

        // The limit falls inside the log; then `b` completes.
        let first = page(&session(), log.clone(), both, None, None, Some(2)).unwrap();
        assert_eq!(names(&first), ["x0", "x1"]);
        assert!(first.incomplete.is_empty());
        let mut later = log;
        later.push(logged("b"));
        let second = page(
            &session(),
            later,
            vec![running("a")],
            None,
            first.next_cursor.as_deref(),
            Some(2),
        )
        .unwrap();
        assert_eq!(names(&second), ["x2", "b", "a"]);
        assert_eq!(second.incomplete, [ItemId::new("a")]);
        assert_eq!(second.next_cursor, None);
    }

    /// An item that completed between the read's two looks is in the log,
    /// and is given once, as complete; one item is read by its id from
    /// either place; a cursor no page gave is refused.
    #[test]
    fn an_item_is_given_once_and_one_can_be_read_by_its_id() {
        let log = vec![logged("x0"), logged("b")];
        let progress = vec![running("a"), running("b")];
        let whole = page(&session(), log.clone(), progress.clone(), None, None, None).unwrap();
        assert_eq!(names(&whole), ["x0", "b", "a"]);
        assert_eq!(whole.incomplete, [ItemId::new("a")]);

        let one = |id: &str| {
            page(
                &session(),
                log.clone(),
                progress.clone(),
                Some(&ItemId::new(id)),
                None,
                None,
            )
            .unwrap()
        };
        assert_eq!(one("b").items, [logged("b")]);
        assert!(one("b").incomplete.is_empty());
        assert_eq!(one("a").items, [running("a")]);
        assert_eq!(one("a").incomplete, [ItemId::new("a")]);
        assert!(one("z").items.is_empty());

        let refused = page(&session(), log, progress, None, Some("s_1 2026"), None).unwrap_err();
        assert_eq!(refused.code, "invalid_cursor");
    }

    /// A cursor names its session and where it stands in the log: one of
    /// another session, one past the end, one whose last item is not where
    /// it was (a rewind moved the session to another branch), and a bare
    /// number are refused; the same cursor on the same branch, with more
    /// after it, is taken (the review of step 27's second pull request).
    #[test]
    fn a_cursor_is_refused_on_another_session_or_another_branch() {
        let log = ["x0", "x1", "x2"].map(logged).to_vec();
        let first = page(&session(), log.clone(), Vec::new(), None, None, Some(2)).unwrap();
        let cursor = first.next_cursor.clone().expect("more to come");
        let next = |session: &SessionId, log: Vec<Item>, cursor: &str| {
            page(session, log, Vec::new(), None, Some(cursor), Some(2))
        };

        // On the same branch, grown since: taken.
        let mut grown = log.clone();
        grown.push(logged("x3"));
        let second = next(&session(), grown, &cursor).unwrap();
        assert_eq!(names(&second), ["x2", "x3"]);

        let refused = |result: Result<Page, OpError>, why: &str| {
            assert_eq!(result.unwrap_err().code, "invalid_cursor", "{why}");
        };
        refused(
            next(&SessionId::new("s_2"), log.clone(), &cursor),
            "another session",
        );
        // A rewind to after `x0`, and a new answer: the branch is another.
        refused(
            next(
                &session(),
                vec![logged("x0"), logged("y1"), logged("y2")],
                &cursor,
            ),
            "another branch",
        );
        refused(
            next(&session(), vec![logged("x0")], &cursor),
            "the log is shorter",
        );
        refused(next(&session(), log.clone(), "s_1 9 x8"), "past the end");
        refused(next(&session(), log, "2"), "a bare number");
    }

    /// A limit of 0 gives no item, and a cursor that starts where the read
    /// did.
    #[test]
    fn a_page_of_nothing_gives_the_cursor_it_was_given() {
        let log = ["x0", "x1"].map(logged).to_vec();
        let empty = page(&session(), log.clone(), Vec::new(), None, None, Some(0)).unwrap();
        assert!(empty.items.is_empty());
        let cursor = empty.next_cursor.expect("the log is still to read");
        let whole = page(&session(), log, Vec::new(), None, Some(&cursor), None).unwrap();
        assert_eq!(names(&whole), ["x0", "x1"]);
    }
}
