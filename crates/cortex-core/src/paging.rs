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

use cortex_protocol::{ErrorKind, Item, ItemId};

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

/// The page of `logged`, the log's items, and `progress`, the items in
/// progress taken with the read's `seq`, that a read asks for: the one item
/// `wanted`, or the log from `cursor`, at most `limit` of it, and on the
/// last page every item in progress.
///
/// # Errors
///
/// `invalid_cursor` when `cursor` is not one a page gave.
pub(crate) fn page(
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
        Some(text) => text.parse::<usize>().map_err(|_| {
            error(
                ErrorKind::Usage,
                cortex_protocol::error::code::INVALID_CURSOR,
                "That cursor is not one this server gave for `session/read`: read again from the \
                 start.",
            )
        })?,
    };
    let limit = limit.map_or(usize::MAX, |limit| limit as usize);
    let total = logged.len();
    let mut items: Vec<Item> = logged.into_iter().skip(start).take(limit).collect();
    let next = start.saturating_add(items.len());
    if next < total {
        return Ok(Page {
            items,
            incomplete: Vec::new(),
            next_cursor: Some(next.to_string()),
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

#[cfg(test)]
mod tests {
    use cortex_protocol::item::{AssistantMessageItem, NoticeItem, NoticeSource};

    use super::*;

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
        let first = page(log.clone(), both.clone(), None, None, Some(4)).unwrap();
        assert_eq!(names(&first), ["x0", "x1", "x2", "a", "b"]);
        assert_eq!(first.incomplete, [ItemId::new("a"), ItemId::new("b")]);
        assert_eq!(first.next_cursor, None);

        // The limit falls inside the log; then `b` completes.
        let first = page(log.clone(), both, None, None, Some(2)).unwrap();
        assert_eq!(names(&first), ["x0", "x1"]);
        assert!(first.incomplete.is_empty());
        let mut later = log;
        later.push(logged("b"));
        let second = page(
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
        let whole = page(log.clone(), progress.clone(), None, None, None).unwrap();
        assert_eq!(names(&whole), ["x0", "b", "a"]);
        assert_eq!(whole.incomplete, [ItemId::new("a")]);

        let one = |id: &str| {
            page(
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

        let refused = page(log, progress, None, Some("s_1 2026"), None).unwrap_err();
        assert_eq!(refused.code, "invalid_cursor");
    }
}
