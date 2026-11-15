//! `usage/report` and `usage/refresh` (`docs/architecture/18-usage.md`).
//!
//! **The report reads local files only:** the aggregates, brought up to
//! date from the tail of the monthly files, and the latest limits. It makes
//! no connection.
//!
//! **The refresh is the one request the usage view may make, and only when
//! the user asks for it** (`cortex usage --refresh`, or `usage/refresh` from
//! a `user` client). For each provider in the user's `usage.refresh` (a
//! setting a project's file cannot set), it asks that provider's documented
//! usage endpoint, with that provider's own credential, through the same
//! transport as model calls, which follows no redirect. The request is a
//! `GET` with no body: nothing of ours is in it. What the answer holds
//! besides its figures is dropped. A refresh that fails, in any way, leaves
//! what was kept as it was, and says why without the answer's text.

use std::sync::Arc;
use std::time::Duration;

use cortex_base::{CancelToken, Zone};
use cortex_config::{Loaded, StateDir};
use cortex_llm::openai_chat::account::{self, AccountUsage};
use cortex_llm::{HttpRequest, Transport};
use cortex_protocol::methods::{
    RefreshOutcome, UsageRefreshParams, UsageRefreshResult, UsageReportParams,
};
use cortex_protocol::usage::{
    Balance, LimitSource, ModelUsage, ProviderLimits, UsageGroup, UsageRange, UsageReport, UsageRow,
};
use cortex_protocol::{ErrorData, ErrorKind, ProviderId};
use cortex_stats::{Aggregates, LimitsFile, Query, Recorder, RecorderOptions, StatsDir};

use crate::core::{Core, CoreOptions, OpError, error};

/// How long one refresh may take, from the request to the end of the
/// answer.
pub(crate) const REFRESH_LIMIT: Duration = Duration::from_secs(20);

/// The most of an answer that is read.
const ANSWER_LIMIT: usize = 256 * 1024;

/// The statistics directory, when there is a state directory.
pub(crate) fn stats_dir(loaded: &Loaded) -> Option<StatsDir> {
    StateDir::from_dirs(&loaded.dirs).map(|state| StatsDir::new(state.stats_dir()))
}

/// The core's recorder: none when statistics are off or there is no state
/// directory. Nothing is read or made here.
pub(crate) fn recorder(user: &Loaded, options: &CoreOptions) -> Option<Recorder> {
    if !user.settings.stats.enabled {
        return None;
    }
    let dir = stats_dir(user)?;
    let mut recorder_options = RecorderOptions::new(dir);
    recorder_options.zone.clone_from(&options.zone);
    recorder_options.retention = user.settings.stats.retention;
    Some(Recorder::new(recorder_options, Arc::clone(&options.clock)))
}

/// The account of today's limits, `[limits.day]`: requests, tokens and
/// money, across every session of the day. It begins with what the day's
/// statistics hold (read here, when a session with a day limit is put
/// together, on the blocking pool, and never when there is none), and every
/// session of this core then draws on the one account. A session that runs
/// past midnight goes on drawing on the day it began in; the next session
/// opened begins a new day. `limits.day.time` is not a day's wall-clock
/// time and is not enforced
/// (`docs/outstanding/a-days-time-limit.md`).
pub(crate) fn day_account(
    inner: &crate::core::Inner,
    limits: &cortex_config::schema::LimitSet,
) -> Option<cortex_agent::Account> {
    if limits.requests.is_none() && limits.tokens.is_none() && limits.cost.is_none() {
        return None;
    }
    let clock = Arc::clone(&inner.options.clock);
    let zone = inner.options.zone.clone().unwrap_or_else(Zone::system);
    let today = zone.date_of(clock.now());
    let mut day = inner
        .day
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((date, account)) = day.as_ref()
        && *date == today
    {
        return Some(account.clone());
    }
    let mut spent = cortex_agent::Spent::default();
    if let Some(dir) = stats_dir(&inner.user)
        && let Ok(mut aggregates) = Aggregates::load(&dir)
    {
        let now = clock.now();
        if let Ok(true) = aggregates.sync(
            &dir,
            today,
            &cortex_stats::month_of(now),
            inner.user.settings.stats.retention,
        ) {
            let _ = aggregates.save(&dir, clock.as_ref());
        }
        let day_text = today.to_string();
        for (key, sum) in &aggregates.total().usage {
            if key.period == day_text {
                spent.requests += sum.requests;
                spent.tokens += sum.input + sum.cache_read + sum.cache_write + sum.output;
                spent.usd_nanos += sum.metered_nanos + sum.estimate_nanos + sum.recorded_nanos;
            }
        }
    }
    let budget = cortex_protocol::lease::Budget {
        wall_clock_ms: None,
        ..crate::assemble::budget(limits)
    };
    let account =
        cortex_agent::Account::with_spent(format!("the day {today}"), budget, clock, spent);
    *day = Some((today, account.clone()));
    Some(account)
}

/// `usage/report`.
pub(crate) async fn report(
    core: &Core,
    params: &UsageReportParams,
) -> Result<UsageReport, OpError> {
    let inner = &core.inner;
    if params.range == UsageRange::Session {
        return session_report(core, params);
    }
    let settings = &inner.user.settings;
    let recording = settings.stats.enabled;
    let query = Query {
        range: params.range.clone(),
        group_by: params.group_by.clone(),
        project: params.project.clone(),
        week_start: settings.usage.week_start.from_monday(),
        rolling: settings.usage.ranges == cortex_config::schema::UsageRanges::Rolling,
    };
    let dir = stats_dir(&inner.user);
    let zone = inner.options.zone.clone();
    let clock = Arc::clone(&inner.options.clock);
    let retention = settings.stats.retention;
    // The open sessions' limits are as fresh as any on disk, and may not
    // have reached it yet.
    let live: Vec<ProviderLimits> = core
        .open_sessions()
        .iter()
        .filter_map(|session| core.session_usage(session))
        .flat_map(|usage| usage.limits)
        .collect();
    Core::blocking(move || {
        let zone = zone.unwrap_or_else(Zone::system);
        let now = clock.now();
        let (part, mut limits) = match &dir {
            Some(dir) => {
                let mut aggregates = Aggregates::load(dir).map_err(read_failed)?;
                let changed = aggregates
                    .sync(
                        dir,
                        zone.date_of(now),
                        &cortex_stats::month_of(now),
                        retention,
                    )
                    .map_err(read_failed)?;
                if changed {
                    // Kept for the next report; one that cannot be written
                    // is no reason to fail this one.
                    let _ = aggregates.save(dir, clock.as_ref());
                }
                (aggregates.total(), LimitsFile::load(dir))
            }
            None => (cortex_stats::Part::default(), LimitsFile::default()),
        };
        limits.merge(live);
        Ok(cortex_stats::report(
            &part,
            &query,
            &zone,
            now,
            &limits.providers,
            recording,
        ))
    })
    .await
}

fn read_failed(failure: std::io::Error) -> OpError {
    error(
        ErrorKind::Internal,
        "stats_unreadable",
        format!("The statistics could not be read: {failure}."),
    )
}

/// The session range: the open session's own figures.
fn session_report(core: &Core, params: &UsageReportParams) -> Result<UsageReport, OpError> {
    let Some(session) = &params.session_id else {
        return Err(error(
            ErrorKind::Usage,
            "invalid_params",
            "The `session` range needs a `session_id`.",
        ));
    };
    let Some(usage) = core.session_usage(session) else {
        return Err(error(
            ErrorKind::Usage,
            cortex_protocol::error::code::SESSION_NOT_OPEN,
            format!("The session `{session}` is not open: the `session` range is a live one."),
        ));
    };
    let group_by = if params.group_by.is_empty() {
        vec![UsageGroup::Provider, UsageGroup::Model]
    } else {
        params.group_by.clone()
    };
    let rows = usage
        .by_model
        .iter()
        .map(|by: &ModelUsage| UsageRow {
            provider: group_by
                .contains(&UsageGroup::Provider)
                .then(|| by.provider.clone()),
            model: group_by
                .contains(&UsageGroup::Model)
                .then(|| by.model.clone()),
            project: None,
            project_label: None,
            source: group_by
                .contains(&UsageGroup::Source)
                .then(|| cortex_stats::query::SOURCE.to_owned()),
            speed: group_by
                .contains(&UsageGroup::Speed)
                .then(|| by.speed.clone()),
            totals: by.totals.clone(),
            time: None,
        })
        .collect();
    Ok(UsageReport {
        range: UsageRange::Session,
        from: None,
        to: None,
        group_by,
        rows,
        totals: usage.totals.clone(),
        limits: cortex_stats::current_limits(&usage.limits, core.inner.options.clock.now()),
        sources: vec![cortex_stats::query::SOURCE.to_owned()],
        recording: core.inner.stats.is_some(),
        time: None,
        tools: Vec::new(),
        zone: None,
    })
}

/// An amount in billionths as an exact decimal number, with at least two
/// places: `110.00`, `-0.5` as `-0.50`, `0.000000001`.
pub(crate) fn decimal(nanos: i64) -> String {
    let sign = if nanos < 0 { "-" } else { "" };
    let abs = nanos.unsigned_abs();
    let whole = abs / 1_000_000_000;
    let mut fraction = format!("{:09}", abs % 1_000_000_000);
    while fraction.len() > 2 && fraction.ends_with('0') {
        fraction.pop();
    }
    format!("{sign}{whole}.{fraction}")
}

/// What an endpoint reported, as the protocol carries it. Nothing but the
/// figures.
fn limits_of(
    provider: &ProviderId,
    usage: AccountUsage,
    seen: cortex_protocol::Timestamp,
) -> ProviderLimits {
    // A limit of one model cannot be told from another's in a limit
    // window, which names no model: only the key's own are kept (Venice.ai
    // reports each model's configured limits, which the provider's page
    // shows better).
    let windows = usage
        .limits
        .into_iter()
        .filter(|limit| limit.model.is_none())
        .map(|limit| limit.window)
        .collect();
    ProviderLimits {
        provider: provider.clone(),
        windows,
        seen_at: seen,
        source: LimitSource::Refresh,
        balances: usage
            .balances
            .into_iter()
            .map(|balance| Balance {
                currency: balance.currency,
                remaining: balance.remaining.map(decimal),
                limit: balance.limit.map(decimal),
                used: balance.used.map(decimal),
            })
            .collect(),
        available: usage.available,
        next_epoch: usage.next_epoch,
    }
}

fn refusal(provider: &ProviderId, code: &str, message: String) -> RefreshOutcome {
    RefreshOutcome {
        provider: provider.clone(),
        limits: None,
        error: Some(ErrorData::new(ErrorKind::Usage, code, message)),
    }
}

/// `usage/refresh`.
pub(crate) async fn refresh(core: &Core, params: &UsageRefreshParams) -> UsageRefreshResult {
    let inner = &core.inner;
    // The user's own configuration, with no project's layers: the setting
    // is the user's alone, and a project's value of it was never taken.
    let settings = &inner.user.settings;
    let env = &inner.options.env;
    let allowed: Vec<ProviderId> = settings
        .usage
        .refresh
        .iter()
        .map(|name| ProviderId::new(name.as_str()))
        .collect();
    let mut outcomes = Vec::new();
    for wanted in &params.providers {
        if !allowed.contains(wanted) {
            outcomes.push(refusal(
                wanted,
                "refresh_not_enabled",
                format!(
                    "The refresh is not turned on for `{wanted}`: add it to `usage.refresh` in \
                     your own configuration."
                ),
            ));
        }
    }
    let asked: Vec<ProviderId> = if params.providers.is_empty() {
        allowed.clone()
    } else {
        allowed
            .iter()
            .filter(|provider| params.providers.contains(provider))
            .cloned()
            .collect()
    };
    let mut kept = Vec::new();
    for provider in asked {
        let outcome = refresh_one(core, settings, env, &provider).await;
        if let Some(limits) = &outcome.limits {
            kept.push(limits.clone());
        }
        outcomes.push(outcome);
    }
    if !kept.is_empty()
        && let Some(dir) = stats_dir(&inner.user)
    {
        let clock = Arc::clone(&inner.options.clock);
        let written = Core::blocking(move || {
            LimitsFile::update(&dir, kept, clock.as_ref()).map_err(read_failed)
        })
        .await;
        if let Err(failure) = written {
            tracing::warn!(error = %failure.message, "the refreshed limits could not be kept");
        }
    }
    UsageRefreshResult {
        providers: outcomes,
    }
}

async fn refresh_one(
    core: &Core,
    settings: &cortex_config::Settings,
    env: &cortex_config::Env,
    provider: &ProviderId,
) -> RefreshOutcome {
    let inner = &core.inner;
    let info = match crate::provider::provider_info(settings, env, provider.as_str()) {
        Ok(info) => info,
        Err(failure) => {
            return refusal(provider, &failure.code, failure.message.clone());
        }
    };
    let request = match account::request(&info) {
        Ok(Some(request)) => request,
        Ok(None) => {
            return refusal(
                provider,
                "no_usage_endpoint",
                format!(
                    "`{provider}` documents no usage endpoint that the key used for model calls \
                     can ask, so it has no refresh."
                ),
            );
        }
        Err(failure) => return refusal(provider, "no_usage_endpoint", failure.to_string()),
    };
    let auth = match inner.providers.auth(settings, env, &info) {
        Ok(auth) => auth,
        Err(failure) => {
            return RefreshOutcome {
                provider: provider.clone(),
                limits: None,
                error: Some(*failure),
            };
        }
    };
    let failed = |code: &str, message: String| RefreshOutcome {
        provider: provider.clone(),
        limits: None,
        error: Some(ErrorData::new(ErrorKind::Transient, code, message)),
    };
    let mut request: HttpRequest = request;
    request.headers.merge(&info.headers);
    // The credential goes on last, as for a model request.
    if let Err(failure) = auth.authorize(&mut request).await {
        return failed("auth_failed", failure.to_string());
    }
    let cancel = CancelToken::new();
    let transport: Arc<dyn Transport> = Arc::clone(&inner.providers.transport);
    let exchange = async {
        let response = transport.send(request, cancel.clone()).await?;
        let mut body = Vec::new();
        let mut stream = response.body;
        while body.len() < ANSWER_LIMIT {
            match stream.next().await {
                Some(Ok(chunk)) => {
                    let room = ANSWER_LIMIT - body.len();
                    body.extend_from_slice(&chunk[..chunk.len().min(room)]);
                }
                Some(Err(failure)) => return Err(failure),
                None => break,
            }
        }
        Ok((response.status, response.headers, body))
    };
    let sleep = inner.providers.timer.sleep(REFRESH_LIMIT);
    let raced = {
        let mut exchange = std::pin::pin!(exchange);
        let mut sleep = std::pin::pin!(sleep);
        std::future::poll_fn(|context| {
            if let std::task::Poll::Ready(done) = exchange.as_mut().poll(context) {
                return std::task::Poll::Ready(Some(done));
            }
            if sleep.as_mut().poll(context).is_ready() {
                return std::task::Poll::Ready(None);
            }
            std::task::Poll::Pending
        })
        .await
    };
    let (status, headers, body) = match raced {
        None => {
            cancel.cancel();
            return failed(
                "refresh_timeout",
                format!(
                    "`{provider}`'s usage endpoint did not answer within {} seconds.",
                    REFRESH_LIMIT.as_secs()
                ),
            );
        }
        Some(Err(failure)) => {
            return failed(
                "refresh_failed",
                format!("`{provider}`'s usage endpoint could not be reached: {failure}."),
            );
        }
        Some(Ok(answer)) => answer,
    };
    let now = inner.options.clock.now();
    match account::read(&info, status, &headers, &body, now) {
        Ok(usage) => RefreshOutcome {
            provider: provider.clone(),
            limits: Some(limits_of(provider, usage, now.to_timestamp())),
            error: None,
        },
        Err(failure) => {
            // The classification names the status and the provider's code;
            // the answer's own text is not carried.
            let mut data = failure.to_error_data_at(now);
            data.message = format!(
                "`{provider}`'s usage endpoint refused the refresh ({}, status {status}).",
                data.code
            );
            RefreshOutcome {
                provider: provider.clone(),
                limits: None,
                error: Some(data),
            }
        }
    }
}

/// What `cortex stats` shows: what is stored, where, and how large.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatsSummary {
    /// Whether events are recorded (`stats.enabled`).
    pub enabled: bool,
    /// The directory, with `~` for the home directory.
    pub dir: Option<String>,
    /// How many months a monthly file stays after its own.
    pub retention: u32,
    /// The months that have a file.
    pub months: Vec<String>,
    /// The bytes it all takes.
    pub bytes: u64,
    /// Requests on record, all time.
    pub requests: u64,
    /// Turns on record, all time.
    pub turns: u64,
    /// Tool calls on record, all time.
    pub tool_calls: u64,
    /// Events dropped because the writer's queue was full.
    pub dropped: u64,
    /// Lines that could not be read.
    pub unreadable: u64,
}

impl Core {
    /// What the statistics hold. It reads the aggregates, brought up to
    /// date.
    ///
    /// # Errors
    ///
    /// When the files cannot be read.
    pub async fn stats_summary(&self) -> Result<StatsSummary, OpError> {
        let user = Arc::clone(&self.inner.user);
        let zone = self.inner.options.zone.clone();
        let clock = Arc::clone(&self.inner.options.clock);
        let home = user.dirs.home.clone();
        Self::blocking(move || {
            let settings = &user.settings.stats;
            let mut summary = StatsSummary {
                enabled: settings.enabled,
                dir: None,
                retention: settings.retention,
                months: Vec::new(),
                bytes: 0,
                requests: 0,
                turns: 0,
                tool_calls: 0,
                dropped: 0,
                unreadable: 0,
            };
            let Some(dir) = stats_dir(&user) else {
                return Ok(summary);
            };
            summary.dir = Some(shortened(dir.root(), home.as_deref()));
            summary.months = dir.months().map_err(read_failed)?;
            let mut aggregates = Aggregates::load(&dir).map_err(read_failed)?;
            let now = clock.now();
            let zone = zone.unwrap_or_else(Zone::system);
            aggregates
                .sync(
                    &dir,
                    zone.date_of(now),
                    &cortex_stats::month_of(now),
                    settings.retention,
                )
                .map_err(read_failed)?;
            let total = aggregates.total();
            summary.requests = total.usage.values().map(|sum| sum.requests).sum();
            summary.turns = total.time.values().map(|time| time.turns).sum();
            summary.tool_calls = total.tools.values().map(|tool| tool.calls).sum();
            summary.dropped = total.dropped;
            summary.unreadable = total.unreadable;
            summary.bytes = dir.size();
            Ok(summary)
        })
        .await
    }

    /// Deletes every statistics file: `cortex stats reset`. Returns how
    /// many were removed.
    ///
    /// # Errors
    ///
    /// When a file cannot be removed.
    pub async fn stats_reset(&self) -> Result<u64, OpError> {
        let Some(dir) = stats_dir(&self.inner.user) else {
            return Ok(0);
        };
        Self::blocking(move || dir.reset().map_err(read_failed)).await
    }
}

/// A path with the home directory written `~`.
fn shortened(path: &std::path::Path, home: Option<&std::path::Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        return std::path::Path::new("~").join(rest).display().to_string();
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_amount_is_written_exactly() {
        assert_eq!(decimal(110_000_000_000), "110.00");
        assert_eq!(decimal(-500_000_000), "-0.50");
        assert_eq!(decimal(1), "0.000000001");
        assert_eq!(decimal(50_230_000_000), "50.23");
        assert_eq!(decimal(0), "0.00");
    }
}
