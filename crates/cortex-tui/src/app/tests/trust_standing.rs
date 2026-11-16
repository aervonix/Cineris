//! What `/status` says of a project trusted for now only (the review of
//! step 23's fixes): `--trust-project`, or a yes the trust store could not
//! keep, which both give the standing `for_the_run`.

use super::*;

#[test]
fn status_says_a_trust_kept_for_now_only_without_naming_one_cause() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    let for_now = session()
        .trust(cortex_protocol::session::TrustStanding::ForTheRun)
        .build();
    opened_with(&mut app, &link, now, for_now);
    let _ = link.take_sent();
    run_command(&mut app, "/status", now);
    let (ticket, _) = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::ConfigGet(_)))
        .expect("config/get");
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::ConfigGet(
            cortex_protocol::methods::ConfigGetResult {
                config: serde_json::Map::new(),
                origins: Default::default(),
            },
        )),
    ));
    drain(&mut app, now);
    let all = notices(&mut app, now);
    assert!(all.contains("trusted for now only"), "{all}");
    assert!(all.contains("--trust-project"), "{all}");
    assert!(!all.contains("for this run (--trust-project)"), "{all}");
}
