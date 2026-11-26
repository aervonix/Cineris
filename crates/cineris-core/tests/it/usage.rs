//! Statistics and usage through the core (step 16): a session's requests
//! reach `usage/report` from the files, a cost already recorded stays as it
//! was when prices change, nothing is read until asked, and the opt-in
//! refresh asks only the documented endpoint, with only that provider's
//! key, only when the user turned it on and asked.

use std::sync::Arc;
use std::time::Duration;

use cineris_core::{ClientInfo, Core};
use cineris_protocol::methods::{
    Op, OpResult, UsageRefreshParams, UsageRefreshResult, UsageReportParams,
};
use cineris_protocol::usage::{LimitSource, TokenUsage, UsageRange, UsageReport};
use cineris_protocol::{ErrorKind, ProviderId};
use cineris_testkit::{Gate, Reply, ScriptedTransport, ServerReply, SseServer, join};

use crate::{Front, World, says};

/// OpenRouter's documented answer for `GET /api/v1/key`, with a label (a
/// shortened form of the key) and ids, which must not be kept.
const KEY_ANSWER: &str = r#"{"data":{"label":"sk-or-v1-abc...xyz","limit":10,"limit_remaining":7.5,"usage":2.5,"creator_user_id":"user_SECRETID","free_model_daily_requests":{"limit":50,"remaining":49}}}"#;

const KEY: &str = "sk-or-test-key-0123456789";

fn with_key(world: &mut World) {
    world.env = world
        .env
        .clone()
        .with("OPENROUTER_API_KEY", KEY)
        .with("ANTHROPIC_API_KEY", "sk-ant-test-not-to-be-sent");
}

fn refresh_config(world: &World, base: &str) {
    world.user_config(&format!(
        "[usage]\nrefresh = [\"openrouter\"]\n\n[providers.openrouter]\nbase_url = \"{base}/api/v1\"\n"
    ));
}

async fn refresh(
    core: &Core,
    info: ClientInfo,
) -> Result<UsageRefreshResult, cineris_core::OpError> {
    let client = core.connect(info);
    match core
        .handle(client.id, Op::UsageRefresh(UsageRefreshParams::default()))
        .await?
    {
        OpResult::UsageRefresh(result) => Ok(result),
        other => panic!("{other:?}"),
    }
}

async fn report(core: &Core, range: UsageRange) -> UsageReport {
    let client = core.connect(ClientInfo::observer());
    match core
        .handle(
            client.id,
            Op::UsageReport(UsageReportParams {
                range,
                group_by: Vec::new(),
                session_id: None,
                project: None,
            }),
        )
        .await
        .unwrap()
    {
        OpResult::UsageReport(report) => report,
        other => panic!("{other:?}"),
    }
}

/// Runs a future on a runtime with its I/O and timer drivers: the HTTP
/// client's sockets need both, which the world's own runtime does not have.
fn net<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn limits_text(world: &World) -> Option<String> {
    std::fs::read_to_string(
        world
            .dirs()
            .state
            .unwrap()
            .join("stats")
            .join("limits.json"),
    )
    .ok()
}

#[test]
fn the_refresh_asks_the_documented_endpoint_with_that_providers_key_and_nothing_else() {
    let mut world = World::new("refresh_asks", []);
    with_key(&mut world);
    let server = SseServer::start();
    server.push(
        ServerReply::status(200)
            .header("content-type", "application/json")
            .send(KEY_ANSWER),
    );
    refresh_config(&world, &server.base_url());
    let core = world.core();
    let result = net(refresh(&core, ClientInfo::user())).unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "one request");
    let request = &requests[0];
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/api/v1/key");
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert!(request.body.is_empty(), "it carries nothing");
    let every_header: String = request
        .headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\n"))
        .collect();
    assert!(
        !every_header.contains("sk-ant"),
        "no other provider's key: {every_header}"
    );

    let outcome = &result.providers[0];
    assert_eq!(outcome.provider, ProviderId::new("openrouter"));
    assert!(outcome.error.is_none(), "{:?}", outcome.error);
    let limits = outcome.limits.as_ref().unwrap();
    assert_eq!(limits.source, LimitSource::Refresh);
    assert_eq!(limits.balances[0].remaining.as_deref(), Some("7.50"));
    assert_eq!(limits.windows[0].remaining, Some(49));
    // Kept, owner-only, and with nothing but the figures.
    let kept = limits_text(&world).unwrap();
    assert!(
        !kept.contains("sk-or"),
        "the key's label is not kept: {kept}"
    );
    assert!(!kept.contains("SECRETID"), "no id is kept: {kept}");
    let path = world
        .dirs()
        .state
        .unwrap()
        .join("stats")
        .join("limits.json");
    assert!(cineris_base::private::is_private(&path).unwrap());
    // And the report shows it, with no connection of its own.
    let shown = net(report(&core, UsageRange::Month));
    assert!(
        shown
            .limits
            .iter()
            .any(|limits| limits.source == LimitSource::Refresh)
    );
    assert_eq!(server.requests().len(), 1, "the report made no request");
}

#[test]
fn a_redirect_takes_the_key_to_no_other_host() {
    let mut world = World::new("refresh_redirect", []);
    with_key(&mut world);
    let elsewhere = SseServer::start();
    let server = SseServer::start();
    server.push(ServerReply::status(302).header("location", &elsewhere.url("/steal")));
    refresh_config(&world, &server.base_url());
    let core = world.core();
    let result = net(refresh(&core, ClientInfo::user())).unwrap();
    assert!(result.providers[0].error.is_some());
    assert!(
        elsewhere.requests().is_empty(),
        "nothing went to the other host"
    );
    assert!(limits_text(&world).is_none(), "nothing was kept");
}

#[test]
fn a_failed_refresh_changes_nothing_kept_and_says_why_without_the_answer() {
    let mut world = World::new("refresh_fails", []);
    with_key(&mut world);
    let server = SseServer::start();
    server.push(
        ServerReply::status(200)
            .header("content-type", "application/json")
            .send(KEY_ANSWER),
    );
    server.push(ServerReply::status(401).send(
        r#"{"error":{"message":"BODY-TEXT-401","code":"BODY-TEXT in the code"},"request_id":"BODY-TEXT-id"}"#,
    ));
    // A code that is plain in its shape and is a key.
    server.push(ServerReply::status(401).send(
        r#"{"error":{"message":"BODY-TEXT-key","code":"sk-or-v1-bodytext0123456789abcdef"}}"#,
    ));
    server.push(ServerReply::status(500).send("BODY-TEXT-500"));
    server.push(ServerReply::status(200).send("BODY-TEXT-not-json"));
    let never = Gate::new();
    server.push(ServerReply::status(200).hold_until(&never));
    refresh_config(&world, &server.base_url());
    let core = world.core();
    net(refresh(&core, ClientInfo::user())).unwrap();
    let kept = limits_text(&world).unwrap();
    for _ in 0..4 {
        let result = net(refresh(&core, ClientInfo::user())).unwrap();
        let error = result.providers[0].error.as_ref().unwrap();
        assert!(error.message.contains("openrouter"), "{}", error.message);
        // Nothing of the answer: not its message, its code or its id.
        let whole = serde_json::to_string(error).unwrap();
        assert!(!whole.contains("BODY-TEXT"), "{whole}");
        assert!(!whole.contains("bodytext"), "{whole}");
        assert_eq!(limits_text(&world).unwrap(), kept, "what was kept stays");
    }
    // One that never answers is given up at its time limit.
    let (result, ()) = net(join(refresh(&core, ClientInfo::user()), async {
        world.timer.sleeping().await;
        world.timer.advance(Duration::from_secs(20));
    }));
    let error = result.unwrap().providers[0].error.clone().unwrap();
    assert_eq!(error.code, "refresh_timeout");
    assert_eq!(limits_text(&world).unwrap(), kept);
    never.open();
}

/// A provider listed twice is asked once, and with statistics off the
/// figures are answered and not kept.
#[test]
fn a_provider_listed_twice_is_asked_once_and_nothing_is_kept_with_statistics_off() {
    let mut world = World::new("refresh_once", []);
    with_key(&mut world);
    let server = SseServer::start();
    server.push(
        ServerReply::status(200)
            .header("content-type", "application/json")
            .send(KEY_ANSWER),
    );
    world.user_config(&format!(
        "[stats]\nenabled = false\n\n[usage]\nrefresh = [\"openrouter\", \"openrouter\"]\n\n\
         [providers.openrouter]\nbase_url = \"{}/api/v1\"\n",
        server.base_url()
    ));
    let core = world.core();
    let result = net(refresh(&core, ClientInfo::user())).unwrap();
    assert_eq!(server.requests().len(), 1);
    assert_eq!(result.providers.len(), 1);
    assert!(result.providers[0].limits.is_some());
    assert!(
        limits_text(&world).is_none(),
        "nothing written with statistics off"
    );
}

#[test]
fn with_the_list_empty_nothing_is_asked_and_a_project_cannot_turn_it_on() {
    let mut world = World::new("refresh_off", []);
    with_key(&mut world);
    // The project lists a provider; the user's own configuration does not.
    world.project_config("[usage]\nrefresh = [\"openrouter\"]\n");
    let transport = ScriptedTransport::new(Vec::<Reply>::new());
    let core = world.core_with(|options| {
        options.transport = Some(Arc::new(transport.clone()));
        options.trust_project = true;
    });
    let result = world.run(refresh(&core, ClientInfo::user())).unwrap();
    assert!(result.providers.is_empty());
    // A provider asked for by name that the user did not list is refused,
    // and not asked.
    let named = world.run(async {
        let client = core.connect(ClientInfo::user());
        core.handle(
            client.id,
            Op::UsageRefresh(UsageRefreshParams {
                providers: vec![ProviderId::new("openrouter")],
            }),
        )
        .await
    });
    let Ok(OpResult::UsageRefresh(named)) = named else {
        panic!("{named:?}");
    };
    assert_eq!(
        named.providers[0].error.as_ref().unwrap().code,
        "refresh_not_enabled"
    );
    // A session in the project does not turn it on either.
    world.run(async {
        let front = Front::start(&world, &core).await;
        let result = refresh(&core, ClientInfo::user()).await.unwrap();
        assert!(result.providers.is_empty());
        front.close().await;
    });
    assert!(transport.requests().is_empty(), "nothing connected");
}

#[test]
fn a_provider_with_no_documented_endpoint_has_no_refresh_and_only_a_user_may_ask() {
    let mut world = World::new("refresh_none", []);
    with_key(&mut world);
    world.user_config("[usage]\nrefresh = [\"anthropic\"]\n");
    let transport = ScriptedTransport::new(Vec::<Reply>::new());
    let core = world.core_with(|options| options.transport = Some(Arc::new(transport.clone())));
    let result = world.run(refresh(&core, ClientInfo::user())).unwrap();
    assert_eq!(
        result.providers[0].error.as_ref().unwrap().code,
        "no_usage_endpoint"
    );
    let refused = world
        .run(refresh(&core, ClientInfo::observer()))
        .unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Usage);
    assert_eq!(refused.code, "role_not_allowed");
    assert!(transport.requests().is_empty());
}

/// The loop opens a `turn` span, a `request` span for each request and a
/// `tool` span for each call, inside the session's: what the latency
/// budgets and a trace recorder read (`docs/architecture/02-runtime.md`).
#[test]
fn the_loops_spans_reach_the_subscriber() {
    let world = World::new(
        "spans",
        [
            crate::calls(&[("c1", "read", serde_json::json!({"path": "a.txt"}))]),
            says("Read."),
        ],
    );
    world.put("a.txt", "hello\n");
    let (subscriber, handle) = cineris_base::log::start(cineris_base::log::LogOptions::new(
        Arc::new(world.clock.clone()),
    ))
    .unwrap();
    let _installed = tracing::subscriber::set_default(subscriber);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        front.close().await;
    });
    let timings = handle.timings();
    let turns = timings.named("turn");
    assert_eq!(turns.len(), 1, "{:?}", timings.all());
    let requests = timings.named("request");
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|request| request.parent == Some(turns[0].id))
    );
    assert_eq!(requests[0].field("role"), Some("main"));
    assert_eq!(requests[0].field("attempts"), Some("1"));
    let tools = timings.named("tool");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].field("tool"), Some("read"));
    // Each request's span closes as its request ends: before the tool its
    // response called closes, not when the next request replaces it (found
    // by the review of step 16).
    let order: Vec<&str> = timings.all().iter().map(|timing| timing.name).collect();
    let first_request = order.iter().position(|name| *name == "request").unwrap();
    let tool = order.iter().position(|name| *name == "tool").unwrap();
    assert!(first_request < tool, "{order:?}");
    // Opened inside the request whose response named it, or the turn.
    assert!(
        tools[0].parent == Some(requests[0].id) || tools[0].parent == Some(turns[0].id),
        "{tools:?}"
    );
}

/// The day's limits count what the day's earlier sessions spent, from the
/// statistics, and every session of the day draws on one account.
#[test]
fn a_days_limit_counts_the_days_earlier_sessions() {
    let world = World::new("day_limit", [says("One."), says("Two."), says("Not sent.")]);
    world.user_config("[limits.day]\nrequests = 2\n");
    let first = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    world.run(async {
        let front = Front::start(&world, &first).await;
        front.turn("one").await;
        front.close().await;
        first.shutdown().await;
    });
    let later = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    world.run(async {
        let front = Front::start(&world, &later).await;
        let second = front.turn("two").await;
        assert_eq!(
            second.outcome,
            cineris_protocol::event::TurnOutcome::Completed
        );
        let third = front.turn("three").await;
        assert_eq!(third.outcome, cineris_protocol::event::TurnOutcome::Limit);
        let error = third.error.unwrap();
        assert_eq!(error.code, "request_limit");
        assert!(
            error.message.contains("the day 2026-11-13"),
            "{}",
            error.message
        );
        front.close().await;
    });
    assert_eq!(world.provider.requests().len(), 2);
}

/// Each session holds the day to its own limits, and spends from the one
/// day's purse: a project's lower limit is not lost to the session that
/// opened first (found by the review of step 16).
#[test]
fn each_session_holds_the_day_to_its_own_limits() {
    let world = World::new("day_limits_apart", [says("One."), says("Not sent.")]);
    world.user_config("[limits.day]\nrequests = 5\n");
    let core = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    world.run(async {
        let first = Front::start(&world, &core).await;
        first.turn("one").await;
        first.close().await;
    });
    // The project lowers the day's limit to the one request already spent.
    world.project_config("[limits.day]\nrequests = 1\n");
    world.run(async {
        let second = Front::start(&world, &core).await;
        let end = second.turn("two").await;
        assert_eq!(
            end.outcome,
            cineris_protocol::event::TurnOutcome::Limit,
            "{:?}",
            end.error
        );
        second.close().await;
    });
    assert_eq!(world.provider.requests().len(), 1);
}

/// `cineris stats` reads and shows, and deletes nothing: a monthly file past
/// its retention is pruned only by a writer that saves what it read (found
/// by the review of step 16).
#[test]
fn showing_the_statistics_prunes_nothing() {
    let world = World::new("stats_show", []);
    world.user_config("[stats]\nretention = 1\n");
    let dir = world.dirs().state.unwrap().join("stats");
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("events-2025-01.jsonl");
    std::fs::write(&old, "").unwrap();
    // Aggregates read from their file: only these may prune, so without
    // them a prune would delete nothing either and the test would hold
    // nothing.
    std::fs::write(dir.join("aggregates.json"), b"{\"version\":1}").unwrap();
    let core = world.core();
    let summary = world.run(core.stats_summary()).unwrap();
    assert_eq!(summary.aggregates, cineris_core::AggregatesFound::Read);
    assert_eq!(summary.months, ["2025-01"]);
    assert!(old.exists(), "nothing was deleted");
}

/// A user's catalog entry for a model priced at a dollar a million input
/// tokens and `output` a million output tokens.
fn house_model(output: f64) -> String {
    format!(
        "[model]\nmain = \"house-model\"\n\n[models.house]\nprovider = \"anthropic\"\nid = \
         \"house-model\"\nname = \"House model\"\nprotocol = \"anthropic-messages\"\n\
         context_window = 200000\nmax_output = 8000\ninput = [\"text\"]\n\n[models.house.prices]\n\
         input = 1.0\noutput = {output}\n"
    )
}

#[test]
fn a_turn_reaches_the_report_from_the_files_and_its_cost_stays_as_it_was_priced() {
    let world = World::new(
        "usage_recorded",
        [says("Done.").tokens(TokenUsage {
            input: 1_000_000,
            output: 1_000_000,
            ..TokenUsage::default()
        })],
    );
    world.raw_user_config(&format!(
        "[session]\ndurability = \"never\"\n\n[credentials]\nstore = \"file\"\n\n{}",
        house_model(2.0)
    ));
    let core = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    let stats = cineris_stats::StatsDir::new(world.dirs().state.unwrap().join("stats"));
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        // Recorded, and nothing read: the statistics are read only when
        // asked for.
        assert_eq!(cineris_stats::reads(&stats), 0);
        front.close().await;
        core.shutdown().await;
    });
    // The price doubles after the request was made.
    world.raw_user_config(&format!(
        "[session]\ndurability = \"never\"\n\n[credentials]\nstore = \"file\"\n\n{}",
        house_model(4.0)
    ));
    let later = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    let shown = world.run(report(&later, UsageRange::Today));
    assert_eq!(shown.totals.requests, 1);
    // $1 of input and $2 of output, as priced then.
    assert_eq!(shown.totals.costs[0].usd_nanos, 3_000_000_000);
    assert!(cineris_stats::reads(&stats) > 0);
    let time = shown.time.unwrap();
    assert_eq!(time.turns, 1);
    assert_eq!(time.parts_ms(), time.wall_ms);
}

/// The refresh keeps a provider's error code only when it is a plain one;
/// a request's failure reached the statistics with its code as the provider
/// client gave it, for months (found by the independent review of step 16).
/// The one rule now holds for both.
#[test]
fn a_providers_error_code_reaches_the_statistics_only_when_it_is_plain() {
    let failing = cineris_testkit::Response::new().error(cineris_llm::error::ProviderError::new(
        cineris_base::ErrorKind::Provider,
        "your prompt 'deploy with key sk-live-0123456789' was refused",
        "Refused.",
    ));
    let world = World::new("error_code_plain", [failing]);
    let core = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    world.run(async {
        let front = Front::start(&world, &core).await;
        let _ = front.turn("go").await;
        front.close().await;
        core.shutdown().await;
    });
    let dir = world.dirs().state.unwrap().join("stats");
    let text = std::fs::read_to_string(dir.join("events-2026-11.jsonl")).unwrap();
    assert!(text.contains(r#""kind":"request""#), "{text}");
    assert!(!text.contains("sk-live"), "{text}");
    assert!(text.contains(r#""error":"other""#), "{text}");
}

/// `usage/report`'s session range answers only a client that has the
/// session open, as `usage/get` does (found by the independent review of
/// step 16).
#[test]
fn a_client_that_has_not_opened_a_session_gets_no_report_of_it() {
    let world = World::new("report_open_for", [says("One.")]);
    let core = world.core();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        let params = || UsageReportParams {
            range: UsageRange::Session,
            group_by: Vec::new(),
            session_id: Some(front.session.clone()),
            project: None,
        };
        let other = core.connect(ClientInfo::observer());
        let refused = core
            .handle(other.id, Op::UsageReport(params()))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "session_not_open");
        let mine = front.op(Op::UsageReport(params())).await;
        assert!(mine.is_ok(), "{mine:?}");
        front.close().await;
    });
}

/// A range from a newer client is refused, not answered as all time.
#[test]
fn a_range_this_build_does_not_know_is_refused() {
    let world = World::new("report_unknown_range", []);
    let core = world.core();
    let client = core.connect(ClientInfo::observer());
    let refused = world
        .run(core.handle(
            client.id,
            Op::UsageReport(UsageReportParams {
                range: UsageRange::Unknown("fortnight".to_owned()),
                group_by: Vec::new(),
                session_id: None,
                project: None,
            }),
        ))
        .unwrap_err();
    assert_eq!(refused.code, "unknown_value");
    assert!(refused.message.contains("fortnight"), "{}", refused.message);
}

/// The end of a process waits for a writer that is stuck at most two
/// seconds, where it waited for as long as the writer was stuck (found by
/// the independent review of step 16). The writer is held for real: another
/// process (the test) holds the limits file's lock and does not let it go,
/// so the writer, which keeps the latest limits a response reported, waits
/// in the lock with the end's flush waiting on it.
#[test]
fn a_stuck_statistics_writer_holds_the_end_at_most_two_seconds() {
    let window = cineris_protocol::usage::LimitWindow {
        measure: cineris_protocol::usage::LimitMeasure::Requests,
        speed: None,
        limit: Some(100),
        remaining: Some(99),
        resets_at: None,
    };
    let response = cineris_testkit::Response::new()
        .event(cineris_llm::ModelEvent::Limits(vec![window]))
        .text("One.")
        .end(cineris_protocol::StopReason::End);
    let world = World::new("stuck_writer", [response]);
    let limits = world
        .dirs()
        .state
        .unwrap()
        .join("stats")
        .join("limits.json");
    let held = cineris_base::private::hold_lock(&limits)
        .unwrap()
        .expect("this file system locks");
    let core = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        front.close().await;
        let ((), ()) = join(core.shutdown(), async {
            while !world.timer.running().contains(&Duration::from_secs(2)) {
                world.timer.sleeping().await;
                tokio::task::yield_now().await;
            }
            world.timer.advance(Duration::from_secs(2));
        })
        .await;
    });
    assert!(world.timer.sleeps().contains(&Duration::from_secs(2)));
    assert!(
        !limits.exists(),
        "the writer was not stuck: it wrote the limits under another's lock"
    );
    // Let the writer go, so that its thread can end with the process.
    drop(held);
}

/// A newer build's aggregates are not reported as if they were the
/// monthly files' alone, and `cineris stats` says what it found; one that
/// could not be read is moved aside, and `cineris stats` says where.
#[test]
fn the_report_and_cineris_stats_say_what_the_aggregates_file_was() {
    let world = World::new("aggregates_found", [says("One.")]);
    let dir = world.dirs().state.unwrap().join("stats");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("aggregates.json"), r#"{"version":2}"#).unwrap();
    let core = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    let client = core.connect(ClientInfo::observer());
    let today = UsageReportParams {
        range: UsageRange::Today,
        group_by: Vec::new(),
        session_id: None,
        project: None,
    };
    let refused = world
        .run(core.handle(client.id, Op::UsageReport(today.clone())))
        .unwrap_err();
    assert_eq!(refused.code, "stats_newer");
    let summary = world.run(core.stats_summary()).unwrap();
    assert_eq!(
        summary.aggregates,
        cineris_core::AggregatesFound::Newer { version: 2 }
    );
    // One that does not parse.
    std::fs::write(dir.join("aggregates.json"), b"{\"version\":1,").unwrap();
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("one").await;
        front.close().await;
        core.shutdown().await;
    });
    let summary = world.run(core.stats_summary()).unwrap();
    assert_eq!(summary.set_aside.len(), 1, "{:?}", summary.set_aside);
    assert!(
        summary.set_aside[0].starts_with('~')
            && summary.set_aside[0].contains("aggregates.json.unreadable-2026-11-13T"),
        "{:?}",
        summary.set_aside
    );
    assert_eq!(summary.requests, 1);
    let report = world.run(core.handle(client.id, Op::UsageReport(today)));
    assert!(report.is_ok(), "{report:?}");
}

/// The aggregates a report read are kept for the next: a report with
/// nothing new opens the aggregates' file no more; one after the file was
/// put in place by another process reads it again, even when the other
/// process put it there while a report was reading the one before, and
/// what was kept is given back after a quiet minute (step 24's third part,
/// the usage panel's year within one frame).
#[test]
fn the_aggregates_are_read_again_only_when_they_are_not_the_files() {
    let world = World::new("usage_kept", [says("Done.")]);
    let stats = cineris_stats::StatsDir::new(world.dirs().state.unwrap().join("stats"));
    // A turn recorded, and the core that recorded it ended, so that its
    // statistics are on the disk and nothing writes them while the
    // reports count what they read.
    let recording = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    world.run(async {
        let front = Front::start(&world, &recording).await;
        front.turn("go").await;
        front.close().await;
        recording.shutdown().await;
    });
    let core = world.core_with(|options| options.zone = Some(cineris_base::Zone::utc()));
    // The recording core's end saved the aggregates: the first report reads
    // them (the aggregates, the month's events and the limits), and keeps
    // them.
    let before = cineris_stats::reads(&stats);
    let _ = world.run(report(&core, UsageRange::Today));
    let reading = cineris_stats::reads(&stats) - before;
    let before = cineris_stats::reads(&stats);
    let kept = world.run(report(&core, UsageRange::Today));
    assert_eq!(kept.totals.requests, 1);
    assert_eq!(
        cineris_stats::reads(&stats) - before,
        reading - 1,
        "the aggregates kept were read again"
    );

    // Another process puts a file of its own in place, at the same length.
    let replace = || {
        let path = stats.aggregates_file();
        let copy = stats.root().join("aggregates.json.other");
        std::fs::copy(&path, &copy).unwrap();
        std::fs::rename(&copy, &path).unwrap();
    };
    replace();
    let before = cineris_stats::reads(&stats);
    let _ = world.run(report(&core, UsageRange::Today));
    assert_eq!(
        cineris_stats::reads(&stats) - before,
        reading,
        "another's file was not read"
    );

    // A file put in place while a report reads the one before: the next
    // report reads it, since what was kept is of the file read, not of the
    // one there when the report ended.
    replace();
    let read = cineris_testkit::Hold::new(cineris_core::AGGREGATES_READ);
    std::thread::scope(|scope| {
        let replacing = scope.spawn(|| {
            read.reach();
            replace();
            read.release();
        });
        let _ = world.run(report(&core, UsageRange::Today));
        replacing.join().unwrap();
    });
    let before = cineris_stats::reads(&stats);
    let _ = world.run(report(&core, UsageRange::Today));
    assert_eq!(
        cineris_stats::reads(&stats) - before,
        reading,
        "the file put in place during the read was taken for the one read"
    );

    // Kept again; and given back after a quiet minute.
    let _ = world.run(report(&core, UsageRange::Today));
    world.timer.advance(std::time::Duration::from_secs(61));
    world.run(crate::settle());
    let before = cineris_stats::reads(&stats);
    let _ = world.run(report(&core, UsageRange::Today));
    assert_eq!(
        cineris_stats::reads(&stats) - before,
        reading,
        "the aggregates were still kept after a quiet minute"
    );
    world.run(core.shutdown());
}
