//! `@` completion over the workspace's files: listed by the core on the
//! first `@` typed and not when the session opens (since the fixes of the
//! review of step 23, finding 12c: a list made at open held 6.6 MB of an
//! idle application's 25 MB budget in a workspace of 20,000 files), then
//! matched off the key's path.

use cineris_protocol::methods::{WorkspaceFilesParams, WorkspaceFilesResult};

use super::*;
use crate::complete::Empty;

/// The `workspace/files` operations sent since the last look, taken with
/// everything else sent.
fn files_asked(link: &Scripted) -> Vec<(Ticket, WorkspaceFilesParams)> {
    link.take_sent()
        .into_iter()
        .filter_map(|(ticket, op)| match op {
            Op::WorkspaceFiles(params) => Some((ticket, params)),
            _ => None,
        })
        .collect()
}

/// The core's answer to `ticket`: `paths`, as `workspace/files` gives them.
fn give_files(app: &mut App, link: &Scripted, ticket: Ticket, paths: Vec<String>, now: Instant) {
    link.give(Inbound::Done(
        ticket,
        Ok(OpResult::WorkspaceFiles(WorkspaceFilesResult {
            paths,
            truncated: false,
        })),
    ));
    drain(app, now);
}

/// The workspace's files are not listed when the session opens, nor while
/// the person types anything but an `@`; the first `@` asks for them once,
/// its menu says they are being listed, and it completes from them when
/// they come. A second `@` asks nothing more.
#[test]
fn the_workspace_files_are_listed_on_the_first_at_and_not_when_the_session_opens() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = app.frame(now);
    assert!(files_asked(&link).is_empty(), "listed at open");
    type_text(&mut app, "look at ", now);
    let _ = app.frame(now);
    assert!(files_asked(&link).is_empty(), "listed before an `@`");

    type_text(&mut app, "@", now);
    let _ = app.frame(now);
    let asked = files_asked(&link);
    assert_eq!(asked.len(), 1, "{asked:#?}");
    let (ticket, params) = asked.into_iter().next().expect("one");
    assert_eq!(params.session_id, SessionId::from(SESSION));
    let menu = app.input().menu().cloned().expect("the file menu");
    assert!(menu.rows.is_empty(), "{menu:#?}");
    assert_eq!(menu.empty, Empty::Listing);
    type_text(&mut app, "lex", now);
    let _ = app.frame(now);
    assert!(files_asked(&link).is_empty(), "asked twice");

    give_files(
        &mut app,
        &link,
        ticket,
        vec!["src/lexer.rs".to_owned(), "README.md".to_owned()],
        now,
    );
    assert!(
        app.wait_for_files(Duration::from_secs(10)),
        "the worker's rows"
    );
    let menu = app.input().menu().cloned().expect("the file menu");
    assert_eq!(menu.rows[0].label, "src/lexer.rs");

    // The menu closed and an `@` typed again: the list is the one given.
    for _ in 0.."@lex".len() {
        key(&mut app, "backspace", now);
    }
    let _ = app.frame(now);
    assert!(app.input().menu().is_none());
    type_text(&mut app, "@READ", now);
    let _ = app.frame(now);
    assert!(files_asked(&link).is_empty(), "listed again");
    assert!(
        app.wait_for_files(Duration::from_secs(10)),
        "the worker's rows"
    );
    let menu = app.input().menu().cloned().expect("the file menu");
    assert_eq!(menu.rows[0].label, "README.md");
}

/// An `@` typed before the session is open has no session to list the
/// files of: they are asked for as soon as it is open, once.
#[test]
fn an_at_typed_before_the_session_is_open_lists_the_files_once_it_is() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    let _ = app.frame(now);
    app.drawn(now);
    let (open, _) = link
        .take_sent()
        .into_iter()
        .find(|(_, op)| matches!(op, Op::SessionNew(_)))
        .expect("the open");
    type_text(&mut app, "@", now);
    let _ = app.frame(now);
    assert!(files_asked(&link).is_empty(), "asked with no session");
    link.give(Inbound::Done(
        open,
        Ok(OpResult::SessionNew(SessionResult {
            session: session().build(),
        })),
    ));
    drain(&mut app, now);
    let _ = app.frame(now);
    let _ = app.frame(now);
    let asked = files_asked(&link);
    assert_eq!(asked.len(), 1, "{asked:#?}");
    assert_eq!(asked[0].1.session_id, SessionId::from(SESSION));
}

/// The workspace's paths at the core's cap (100,000), given as
/// `workspace/files` gives them to the first `@`; the `@` is then taken
/// back, leaving the prompt empty.
fn at_the_cap(app: &mut App, link: &Scripted, now: Instant) {
    opened(app, link, now);
    type_text(app, "@", now);
    let _ = app.frame(now);
    let (ticket, _) = files_asked(link)
        .into_iter()
        .next()
        .expect("the files are asked for");
    let paths = (0..100_000)
        .map(|n| format!("crates/part{}/src/module{}/file{n}.rs", n % 97, n % 13))
        .collect();
    give_files(app, link, ticket, paths, now);
    key(app, "backspace", now);
    let _ = app.frame(now);
}

/// At the cap, a key's frame never waits for the match: each key of an `@`
/// query is drawn with the rows the worker gave before it, and the rows for
/// the whole query come after, from the worker.
#[test]
fn at_the_cap_a_keys_frame_never_waits_for_the_file_match() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    at_the_cap(&mut app, &link, now);
    type_text(&mut app, "look @", now);
    let _ = app.frame(now);
    assert!(
        app.wait_for_files(Duration::from_secs(10)),
        "the rows for `@`"
    );
    let first = app.input().menu().cloned().expect("the file menu").rows;
    assert!(!first.is_empty());
    for c in "module7/file12".chars() {
        type_text(&mut app, &c.to_string(), now);
        let _ = app.frame(now);
        // The frame of this key has the older query's rows.
        assert_eq!(
            app.input().menu().cloned().expect("the file menu").rows,
            first
        );
    }
    assert!(
        app.wait_for_files(Duration::from_secs(10)),
        "the rows for the query"
    );
    let rows = app.input().menu().cloned().expect("the file menu").rows;
    assert!(
        rows.iter()
            .all(|row| row.label.contains("module7") && row.label.contains("file12")),
        "{rows:#?}"
    );
}

/// A key's time to its frame with the workspace's paths at the cap: each key
/// of `@module7/file12` handled and its frame drawn, the match left to the
/// worker. The key's budget is 2 ms at the 99th percentile
/// (`docs/performance-budgets.md`). A measurement, in each profile:
///
/// ```text
/// cargo test -p cineris-tui --lib perf_a_keys_frame -- --ignored --nocapture
/// cargo test --release -p cineris-tui --lib perf_a_keys_frame -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement: see its documentation"]
fn perf_a_keys_frame_at_the_file_cap() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    at_the_cap(&mut app, &link, now);
    let mut times = Vec::new();
    for _ in 0..20 {
        for c in "look @module7/file12".chars() {
            let started = Instant::now();
            app.terminal(
                &TermEvent::Key(cineris_term::Key::plain(cineris_term::KeyCode::Char(c))),
                now,
            );
            let _ = app.frame(now);
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        for _ in 0.."look @module7/file12".len() {
            key(&mut app, "backspace", now);
        }
        let _ = app.frame(now);
    }
    times.sort_by(f64::total_cmp);
    let at = |share: usize| times[(times.len() * share / 100).min(times.len() - 1)];
    println!(
        "a key's frame, 100,000 paths, {} keys: median {:.3} ms   p99 {:.3} ms   most {:.3} ms",
        times.len(),
        at(50),
        at(99),
        times[times.len() - 1]
    );
}

/// A device for [`perf_a_keys_frame_through_the_loop`]: a wait hands the
/// loop the next key once the last has been drawn, and the first write
/// after a key times it from being handed over. What is written goes
/// nowhere.
struct Timed {
    keys: std::collections::VecDeque<u8>,
    handed: Option<Instant>,
    times: Arc<Mutex<Vec<f64>>>,
}

impl cineris_term::Device for Timed {
    fn size(&self) -> std::io::Result<(u16, u16)> {
        Ok((100, 30))
    }
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        std::hint::black_box(bytes);
        if let Some(at) = self.handed.take() {
            self.times
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(at.elapsed().as_secs_f64() * 1000.0);
        }
        Ok(())
    }
    fn wait(
        &mut self,
        _timeout: Option<Duration>,
        into: &mut Vec<u8>,
    ) -> std::io::Result<cineris_term::Ready> {
        // The next key only once the last one's frame is written: the loop
        // reads on while keys come, and draws them together.
        if self.handed.is_some() {
            return Ok(cineris_term::Ready::Timeout);
        }
        let Some(key) = self.keys.pop_front() else {
            return Err(std::io::Error::other("the keys have all been typed"));
        };
        into.push(key);
        self.handed = Some(Instant::now());
        Ok(cineris_term::Ready::Input)
    }
    fn waker(&self) -> cineris_term::Waker {
        cineris_term::Waker::new(|| {})
    }
    fn suspend(&mut self) {}
    fn leave(&mut self) {}
}

/// The harness's own share of a key's time to its frame, through the real
/// loop (`app::run`): the terminal layer's decoding, the application, the
/// views, the presenter and the frame's bytes, from the key handed to the
/// loop to the end of the write of the frame that shows it, with no
/// terminal behind it. A character typed and taken back by turns,
/// `CINERIS_PERF_RUNS` keys (2,000 unless given), into an open session. It
/// is the figure the budget holds the harness to where the terminal's own
/// part is not ours (`docs/outstanding/key-latency-over-conpty.md`): on a
/// hosted Windows runner, `gh workflow run try --ref BRANCH -f os=windows
/// -f command="perf -p cineris-tui"`. The system's read and write are not in
/// it. A measurement:
///
/// ```text
/// cargo test --release -p cineris-tui --lib perf_a_keys_frame_through -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement: see its documentation"]
fn perf_a_keys_frame_through_the_loop() {
    let keys: usize = std::env::var("CINERIS_PERF_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_000);
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let times = Arc::new(Mutex::new(Vec::with_capacity(keys)));
    let device = Timed {
        keys: (0..keys)
            .map(|n| if n % 2 == 0 { b'x' } else { 0x7f })
            .collect(),
        handed: None,
        times: Arc::clone(&times),
    };
    let mut terminal = cineris_term::Terminal::with_device(device, caps()).expect("the terminal");
    let ended = run(&mut terminal, &mut app);
    assert!(
        matches!(&ended, Err(error) if error.to_string() == "the keys have all been typed"),
        "{ended:?}"
    );
    let mut times = times.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(times.len(), keys, "a key with no frame");
    times.sort_by(f64::total_cmp);
    let at = |share: usize| times[(times.len() * share / 100).min(times.len() - 1)];
    println!(
        "a key to its frame through the loop, the harness's share, {} keys: median {:.3} ms   \
         p99 {:.3} ms   most {:.3} ms",
        times.len(),
        at(50),
        at(99),
        times[times.len() - 1]
    );
}

/// A `workspace/files` that fails (the core's walk, a session closed under
/// it) says why once, and its `@` menu says the files could not be listed:
/// frames drawn while that menu is open ask nothing more (a failure that
/// lasts was a request, an error and a frame over and over). The next `@`
/// asks again, once.
#[test]
fn a_failed_file_listing_says_why_once_and_the_next_at_asks_again() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    let _ = link.take_sent();
    type_text(&mut app, "@", now);
    let _ = app.frame(now);
    let asked = files_asked(&link);
    assert_eq!(asked.len(), 1, "{asked:#?}");
    link.give(Inbound::Done(
        asked[0].0,
        Err(ErrorData::new(
            ErrorKind::Internal,
            "store_failed",
            "The walk of the workspace failed.",
        )),
    ));
    drain(&mut app, now);
    let mut seen = Vec::new();
    for _ in 0..4 {
        let _ = screen(&mut app, now, &mut seen);
        let asked = files_asked(&link);
        assert!(
            asked.is_empty(),
            "asked again with the menu open: {asked:#?}"
        );
    }
    let menu = app.input().menu().cloned().expect("the file menu");
    assert_eq!(menu.empty, Empty::Unlisted);
    let live = screen(&mut app, now, &mut seen);
    assert!(
        live.iter().any(|line| line.contains("could not be listed")),
        "{live:#?}"
    );
    let all = [seen.clone(), live].concat().join("\n");
    assert_eq!(
        all.matches("walk of the workspace failed").count(),
        1,
        "said once: {all}"
    );

    key(&mut app, "backspace", now);
    let _ = screen(&mut app, now, &mut seen);
    assert!(files_asked(&link).is_empty(), "asked with no `@`");
    type_text(&mut app, "@", now);
    let mut again = Vec::new();
    for _ in 0..3 {
        let _ = screen(&mut app, now, &mut seen);
        again.extend(files_asked(&link));
    }
    assert_eq!(again.len(), 1, "{again:#?}");
    let menu = app.input().menu().cloned().expect("the file menu");
    assert_eq!(menu.empty, Empty::Listing);
}

/// The `workspace/warm` operations sent since the last look, taken with
/// everything else sent.
fn warmed(link: &Scripted) -> Vec<(Ticket, Vec<String>)> {
    link.take_sent()
        .into_iter()
        .filter_map(|(ticket, op)| match op {
            Op::WorkspaceWarm(params) => {
                assert_eq!(params.session_id, SessionId::from(SESSION));
                Some((ticket, params.paths))
            }
            _ => None,
        })
        .collect()
}

/// A file chosen from the `@` menu is read ahead by the core, at most 16 a
/// prompt, and a folder not at all; the core's refusal is not said, since
/// nothing the person asked for depends on it (step 24, the features
/// table's prefetch).
#[test]
fn a_file_chosen_with_at_is_read_ahead_and_a_refusal_is_not_said() {
    let now = Instant::now();
    let (mut app, link) = app(now);
    opened(&mut app, &link, now);
    type_text(&mut app, "@", now);
    let _ = app.frame(now);
    let (ticket, _) = files_asked(&link).into_iter().next().expect("listed");
    give_files(
        &mut app,
        &link,
        ticket,
        vec!["src/lexer.rs".to_owned(), "src/".to_owned()],
        now,
    );
    let choose = |app: &mut App, typed: &str| {
        type_text(app, typed, now);
        let _ = app.frame(now);
        assert!(
            app.wait_for_files(Duration::from_secs(10)),
            "the worker's rows"
        );
        key(app, "tab", now);
        type_text(app, " ", now);
    };
    // The `@` typed first is completed.
    assert!(
        app.wait_for_files(Duration::from_secs(10)),
        "the worker's rows"
    );
    type_text(&mut app, "lex", now);
    let _ = app.frame(now);
    assert!(app.wait_for_files(Duration::from_secs(10)));
    key(&mut app, "tab", now);
    let sent = warmed(&link);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].1, ["src/lexer.rs"]);
    link.give(Inbound::Done(
        sent[0].0,
        Err(ErrorData::new(
            cineris_protocol::ErrorKind::Usage,
            "session_not_open",
            "refused",
        )),
    ));
    drain(&mut app, now);
    assert!(
        !notices(&mut app, now).contains("refused"),
        "a warm's refusal was said"
    );

    // A folder is not read ahead.
    choose(&mut app, " @src/");
    assert!(warmed(&link).is_empty(), "a folder was read ahead");

    // Sixteen in all for one prompt, then none until it is sent.
    for _ in 0..20 {
        choose(&mut app, "@lex");
    }
    assert_eq!(warmed(&link).len(), 15, "more than 16 for one prompt");
    key(&mut app, "enter", now);
    let _ = link.take_sent();
    choose(&mut app, "@lex");
    assert_eq!(warmed(&link).len(), 1, "the next prompt reads ahead again");
}
