//! The loop's part in the repository map: when an epoch is given its map
//! (`crate::context::map` has the rules).

use cortex_protocol::item::NoticeSource;

use super::{Session, Turn};
use crate::context::MapPending;
use crate::store::StoreError;

/// What the loop keeps of the map between boundaries.
#[derive(Debug, Default)]
pub(super) struct MapState {
    /// The map asked for, and the epoch it was asked for.
    pending: Option<(u64, MapPending)>,
    /// The epoch whose first chance has been taken: its wait is spent.
    waited: Option<u64>,
    /// The epoch the source said has no map: it is not asked again in it.
    none: Option<u64>,
}

/// Waits for whichever of two futures ends first.
async fn first<A: Future, B: Future>(one: A, other: B) -> Option<A::Output> {
    let mut one = std::pin::pin!(one);
    let mut other = std::pin::pin!(other);
    std::future::poll_fn(|context| {
        if let std::task::Poll::Ready(value) = one.as_mut().poll(context) {
            return std::task::Poll::Ready(Some(value));
        }
        other.as_mut().poll(context).map(|_| None)
    })
    .await
}

impl Session {
    /// At a chance to give the epoch its map: a turn's start, before its
    /// prompt, and each safe boundary before a request. The epoch's first
    /// chance waits up to the map's wait; every later one takes the map
    /// only if it is ready.
    pub(super) async fn give_map(&mut self, turn: &Turn) -> Result<(), StoreError> {
        let Some(map) = self.context.parts.map.clone() else {
            return Ok(());
        };
        let epoch = self.context.epoch_number;
        if self.context.history.has_map() || self.map.none == Some(epoch) {
            return Ok(());
        }
        // A request made for another epoch was for another history.
        if self.map.pending.as_ref().is_none_or(|(asked, _)| *asked != epoch) {
            let root = self.env.workspace.paths.root().clone();
            let touched: Vec<String> = self
                .env
                .workspace
                .files
                .touched()
                .iter()
                .filter_map(|path| path.strip_prefix(&root))
                .filter(|relative| !relative.is_empty())
                .map(|relative| relative.as_str().to_owned())
                .collect();
            self.map.pending = Some((epoch, map.source.request(touched)));
        }
        let first_chance = self.map.waited != Some(epoch);
        self.map.waited = Some(epoch);
        let Some((_, pending)) = self.map.pending.as_mut() else {
            return Ok(());
        };
        let answer = match pending.try_take() {
            Some(answer) => Some(answer),
            None if first_chance && !map.wait.is_zero() => {
                // An interrupt, or the end of the wait, leaves it for a later
                // boundary.
                let sleep = map.timer.sleep(map.wait);
                turn.cancel
                    .race(first(pending.wait(), sleep))
                    .await
                    .flatten()
            }
            None => None,
        };
        match answer {
            // Not ready: a later boundary takes it.
            None => Ok(()),
            Some(None) => {
                self.map.pending = None;
                self.map.none = Some(epoch);
                Ok(())
            }
            Some(Some(text)) => {
                self.map.pending = None;
                self.record_notice(turn, NoticeSource::Map, text).await
            }
        }
    }

    /// Tells the map's source that a turn began, so that its work in the
    /// background gives way.
    pub(super) fn map_turn_started(&self) {
        if let Some(map) = &self.context.parts.map {
            map.source.turn_started();
        }
    }

    /// Tells the map's source that the turn ended.
    pub(super) fn map_turn_ended(&self) {
        if let Some(map) = &self.context.parts.map {
            map.source.turn_ended();
        }
    }
}
