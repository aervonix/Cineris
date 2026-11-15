//! Statistics and usage through the core (step 16): a session's requests
//! reach `usage/report` from the files, a cost already recorded stays as it
//! was when prices change, nothing is read until asked, and the opt-in
//! refresh asks only the documented endpoint, with only that provider's
//! key, only when the user turned it on and asked.

use std::sync::Arc;
use std::time::Duration;

use cortex_core::{ClientInfo, Core};
use cortex_protocol::methods::{
    Op, OpResult, UsageRefreshParams, UsageRefreshResult, UsageReportParams,
};
use cortex_protocol::usage::{LimitSource, TokenUsage, UsageRange, UsageReport};
use cortex_protocol::{ErrorKind, ProviderId};
use cortex_testkit::{Gate, Reply, ScriptedTransport, ServerReply, SseServer, join};

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
) -> Result<UsageRefreshResult, cortex_core::OpError> {
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
    assert!(cortex_base::private::is_private(&path).unwrap());
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
    server.push(ServerReply::status(401).send(r#"{"error":{"message":"BODY-TEXT-401"}}"#));
    server.push(ServerReply::status(500).send("BODY-TEXT-500"));
    server.push(ServerReply::status(200).send("BODY-TEXT-not-json"));
    let never = Gate::new();
    server.push(ServerReply::status(200).hold_until(&never));
    refresh_config(&world, &server.base_url());
    let core = world.core();
    net(refresh(&core, ClientInfo::user())).unwrap();
    let kept = limits_text(&world).unwrap();
    for _ in 0..3 {
        let result = net(refresh(&core, ClientInfo::user())).unwrap();
        let error = result.providers[0].error.as_ref().unwrap();
        assert!(error.message.contains("openrouter"), "{}", error.message);
        assert!(!error.message.contains("BODY-TEXT"), "{}", error.message);
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
    let (subscriber, handle) = cortex_base::log::start(cortex_base::log::LogOptions::new(
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
    let first = world.core_with(|options| options.zone = Some(cortex_base::Zone::utc()));
    world.run(async {
        let front = Front::start(&world, &first).await;
        front.turn("one").await;
        front.close().await;
        first.shutdown().await;
    });
    let later = world.core_with(|options| options.zone = Some(cortex_base::Zone::utc()));
    world.run(async {
        let front = Front::start(&world, &later).await;
        let second = front.turn("two").await;
        assert_eq!(
            second.outcome,
            cortex_protocol::event::TurnOutcome::Completed
        );
        let third = front.turn("three").await;
        assert_eq!(third.outcome, cortex_protocol::event::TurnOutcome::Limit);
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
    let core = world.core_with(|options| options.zone = Some(cortex_base::Zone::utc()));
    let stats = cortex_stats::StatsDir::new(world.dirs().state.unwrap().join("stats"));
    world.run(async {
        let front = Front::start(&world, &core).await;
        front.turn("go").await;
        // Recorded, and nothing read: the statistics are read only when
        // asked for.
        assert_eq!(cortex_stats::reads(&stats), 0);
        front.close().await;
        core.shutdown().await;
    });
    // The price doubles after the request was made.
    world.raw_user_config(&format!(
        "[session]\ndurability = \"never\"\n\n[credentials]\nstore = \"file\"\n\n{}",
        house_model(4.0)
    ));
    let later = world.core_with(|options| options.zone = Some(cortex_base::Zone::utc()));
    let shown = world.run(report(&later, UsageRange::Today));
    assert_eq!(shown.totals.requests, 1);
    // $1 of input and $2 of output, as priced then.
    assert_eq!(shown.totals.costs[0].usd_nanos, 3_000_000_000);
    assert!(cortex_stats::reads(&stats) > 0);
    let time = shown.time.unwrap();
    assert_eq!(time.turns, 1);
    assert_eq!(time.parts_ms(), time.wall_ms);
}
