use cortex_base::Clock;
use cortex_protocol::usage::{LimitMeasure, LimitSource, LimitWindow, ProviderLimits};
use cortex_protocol::{ProviderId, Timestamp};
use cortex_stats::{LimitsFile, StatsDir, current_limits};

use crate::{clock, scratch, time};

/// Limits from response headers: `remaining` requests of 100 left, resetting
/// a minute after `seen`.
pub(crate) fn headers_limits(provider: &str, seen: &str, remaining: u64) -> ProviderLimits {
    let reset = time(seen)
        .checked_add(std::time::Duration::from_secs(60))
        .unwrap();
    ProviderLimits {
        provider: ProviderId::new(provider),
        windows: vec![LimitWindow {
            measure: LimitMeasure::Requests,
            speed: None,
            limit: Some(100),
            remaining: Some(remaining),
            resets_at: Some(reset.to_timestamp()),
        }],
        seen_at: Timestamp::new(seen),
        source: LimitSource::Headers,
        balances: Vec::new(),
        available: None,
        next_epoch: None,
    }
}

#[test]
fn a_limit_past_its_reset_time_is_not_known_rather_than_full() {
    let seen = headers_limits("anthropic", "2026-11-15T10:00:00Z", 5);
    let before = current_limits(std::slice::from_ref(&seen), time("2026-11-15T10:00:59Z"));
    assert_eq!(before[0].windows[0].remaining, Some(5));
    let after = current_limits(&[seen], time("2026-11-15T10:01:00Z"));
    let window = &after[0].windows[0];
    // Not 100, the full limit: nothing is known of it until the next
    // request.
    assert_eq!(window.remaining, None);
    assert_eq!(window.limit, Some(100));
    assert_eq!(after[0].seen_at.as_str(), "2026-11-15T10:00:00Z");
}

#[test]
fn the_latest_figures_of_each_provider_and_source_are_kept() {
    let scratch = scratch("the_latest_figures");
    let dir = StatsDir::new(scratch.join("stats"));
    let clock = clock("2026-11-15T10:00:00Z");
    LimitsFile::update(
        &dir,
        [
            headers_limits("anthropic", "2026-11-15T10:00:00Z", 5),
            headers_limits("openrouter", "2026-11-15T10:00:00Z", 9),
        ],
        clock.as_ref() as &dyn Clock,
    )
    .unwrap();
    // An older figure does not replace a newer one.
    LimitsFile::update(
        &dir,
        [
            headers_limits("anthropic", "2026-11-15T09:00:00Z", 1),
            headers_limits("openrouter", "2026-11-15T10:05:00Z", 7),
        ],
        clock.as_ref(),
    )
    .unwrap();
    let file = LimitsFile::load(&dir);
    let remaining: Vec<(&str, Option<u64>)> = file
        .providers
        .iter()
        .map(|limits| (limits.provider.as_str(), limits.windows[0].remaining))
        .collect();
    assert_eq!(remaining, [("anthropic", Some(5)), ("openrouter", Some(7))]);
}
