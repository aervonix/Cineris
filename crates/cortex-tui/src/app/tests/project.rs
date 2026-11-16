//! The project panel in the application (step 24): when it opens and
//! closes, what each key sends, `y` only on a detail drawn whole, the
//! deletion asked in the row, promote while a turn runs, and hostile text
//! in the frame.

use cortex_base::{FakeClock, SharedClock};
use cortex_protocol::event::ProjectUpdated;
use cortex_protocol::ids::EntryId;
use cortex_protocol::methods::{ProjectEntriesResult, ProjectListParams};
use cortex_protocol::project::{EntryAction, EntryChange, EntryStatus, ProjectEntry};

use super::*;
use crate::app::panels::project::TALLER;
use crate::app::ready::{GRACE, QUIET};
use crate::focus::Target;
use crate::view::project::tests::entries;

/// The time the panel reads: 2026-11-16T12:00:00Z, as the view's tests.
fn clock() -> SharedClock {
    Arc::new(FakeClock::starting_at("2026-11-16T12:00:00Z").expect("a time"))
}

fn project_id(app: &App) -> u64 {
    app.panels().find("project").expect("the project panel")
}

/// Answers every `project/list` sent with `entries`.
fn list(app: &mut App, link: &Scripted, entries: &[ProjectEntry], at: Instant) {
    for (ticket, op) in link.take_sent() {
        if let Op::ProjectList(params) = op {
            assert_eq!(
                params,
                ProjectListParams {
                    session_id: Some(SessionId::from(SESSION)),
                    workspace: None,
                }
            );
            link.give(Inbound::Done(
                ticket,
                Ok(OpResult::ProjectList(ProjectEntriesResult {
                    entries: entries.to_vec(),
                    problems: Vec::new(),
                })),
            ));
        }
    }
    drain(app, at);
}

/// An application `rows` high with its session open long before `at`, the
/// project's entries listed as it opened, and a frame drawn.
fn with_entries(at: Instant, rows: usize, entries: &[ProjectEntry]) -> (App, Scripted) {
    let long_before = at - Duration::from_secs(10);
    let (mut app, link) = app(long_before);
    app.set_clock(clock());
    app.resize(98, rows);
    opened(&mut app, &link, long_before);
    list(&mut app, &link, entries, long_before);
    let _ = app.frame(long_before);
    app.drawn(long_before);
    (app, link)
}

/// One key read at `at`, and the frame the loop draws for it.
fn press(app: &mut App, binding: &str, at: Instant) {
    key(app, binding, at);
    let _ = app.frame(at);
    app.drawn(at);
}

/// `/project` typed long before `at`, its `Enter` at `at`, its list
/// answered and the frame drawn: the panel has the focus. The time it is
/// ready.
fn project_focused(
    app: &mut App,
    link: &Scripted,
    entries: &[ProjectEntry],
    at: Instant,
) -> Instant {
    type_text(app, "/project", at - Duration::from_secs(5));
    key(app, "esc", at - Duration::from_secs(5));
    key(app, "enter", at);
    list(app, link, entries, at);
    let _ = app.frame(at);
    app.drawn(at);
    assert_eq!(
        app.input().focus().focused(),
        Target::Panel(project_id(app))
    );
    at + QUIET
}

/// The changes sent so far, taken, each answered as the core answers one
/// that worked: with `changed`, the entries as they are now.
fn answered(
    app: &mut App,
    link: &Scripted,
    changed: &[ProjectEntry],
    at: Instant,
) -> Vec<EntryChange> {
    let mut changes = Vec::new();
    for (ticket, op) in link.take_sent() {
        if let Op::ProjectUpdate(params) = op {
            assert_eq!(params.session_id, Some(SessionId::from(SESSION)));
            assert!(params.add.is_empty());
            changes.extend(params.changes);
            link.give(Inbound::Done(
                ticket,
                Ok(OpResult::ProjectUpdate(ProjectEntriesResult {
                    entries: changed.to_vec(),
                    problems: Vec::new(),
                })),
            ));
        }
    }
    drain(app, at);
    changes
}

/// The changes sent so far, taken and refused with `code` and `message`.
fn refused(
    app: &mut App,
    link: &Scripted,
    code: &str,
    message: &str,
    at: Instant,
) -> Vec<EntryChange> {
    let mut changes = Vec::new();
    for (ticket, op) in link.take_sent() {
        if let Op::ProjectUpdate(params) = op {
            changes.extend(params.changes);
            link.give(Inbound::Done(
                ticket,
                Err(ErrorData::new(ErrorKind::Usage, code, message)),
            ));
        }
    }
    drain(app, at);
    changes
}

fn change(id: &str, action: EntryAction, text: Option<&str>) -> EntryChange {
    EntryChange {
        id: EntryId::from(id),
        action,
        text: text.map(str::to_owned),
    }
}

fn live_text(app: &mut App, at: Instant) -> Vec<String> {
    let (_, live, _) = app.frame(at);
    app.drawn(at);
    text(&live)
}

fn notices(app: &mut App, at: Instant) -> String {
    let mut scrollback = Vec::new();
    let _ = screen(app, at, &mut scrollback);
    scrollback.join("\n")
}

#[test]
fn the_project_opens_by_itself_collapsed_when_entries_wait_and_not_otherwise() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, _link) = with_entries(now, 40, &entries());
    let live = live_text(&mut app, now);
    assert!(
        live.iter()
            .any(|row| row == "◇ Project  3 entries wait for review · 1 in use"),
        "{live:#?}"
    );
    assert!(app.input().focus().on_input(), "it takes no focus");
    // Only an entry accepted: nothing waits, and nothing opens.
    let (mut app, _link) = with_entries(now, 40, &entries()[2..3]);
    assert!(!app.panels().is_open(project_id(&app)));
    let live = live_text(&mut app, now);
    assert!(
        !live.iter().any(|row| row.starts_with("◇ Project")),
        "{live:#?}"
    );
}

#[test]
fn slash_project_lists_again_and_opens_it_focused_with_every_entry() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 60, &entries()[2..3]);
    let _ = project_focused(&mut app, &link, &entries(), now);
    let live = live_text(&mut app, now);
    for row in [
        "◇ Project  3 entries wait for review · 1 in use",
        "    observed 3 times, learned in session s_1 on 2026-11-14",
    ] {
        assert!(live.iter().any(|shown| shown == row), "{row}: {live:#?}");
    }
    // The focus stays on the entry it started on, by its id, as the list
    // comes: the one accepted, which has nothing to accept.
    assert!(
        live.iter()
            .any(|row| row == "    The parser is hand-written."),
        "{live:#?}"
    );
    assert!(
        live.last().is_some_and(|hint| !hint.contains("y accepts")
            && hint.contains("e edits")
            && hint.contains("x closes")),
        "{live:#?}"
    );
    // `x` closes it, though three wait.
    press(&mut app, "x", now + QUIET);
    assert!(!app.panels().is_open(project_id(&app)));
    assert!(app.input().focus().on_input());
}

#[test]
fn y_accepts_the_focused_entry_once_its_detail_was_drawn_and_the_result_replaces_it() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 60, &entries());
    let ready = project_focused(&mut app, &link, &entries(), now);
    press(&mut app, "y", ready);
    let mut accepted = entries()[0].clone();
    accepted.accepted_at = Some(Timestamp::from("2026-11-16T12:00:00Z"));
    assert_eq!(
        answered(&mut app, &link, &[accepted], ready),
        [change("k_1", EntryAction::Accept, None)]
    );
    let live = live_text(&mut app, ready);
    assert!(
        live.iter()
            .any(|row| row == "◇ Project  2 entries wait for review · 2 in use"),
        "{live:#?}"
    );
    // The accepted entry is in use now, after those that wait.
    assert!(
        live.iter()
            .any(|row| row.starts_with("  ✓ command  test: cargo test")),
        "{live:#?}"
    );
}

/// Before the panel is ready, `y` is typing: it goes to the editor, and
/// the focus with it.
#[test]
fn a_y_before_the_panel_is_ready_is_typing() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 60, &entries());
    let _ = project_focused(&mut app, &link, &entries(), now);
    press(&mut app, "y", now + GRACE / 2);
    assert!(answered(&mut app, &link, &[], now).is_empty());
    assert!(app.input().focus().on_input());
}

#[test]
fn y_on_an_entry_whose_detail_does_not_fit_says_to_make_the_window_taller() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 20, &entries());
    let ready = project_focused(&mut app, &link, &entries(), now);
    let live = live_text(&mut app, ready);
    assert!(
        live.iter().any(|row| row.starts_with("  ? command")),
        "the entry is drawn: {live:#?}"
    );
    assert!(
        !live.iter().any(|row| row.contains("not run in this build")),
        "its detail is cut: {live:#?}"
    );
    let hint = live.last().cloned().unwrap_or_default();
    assert!(hint.contains("make the window taller to accept"), "{hint}");
    assert!(!hint.contains("y accepts"), "{hint}");
    // Read with no frame after it, so that the frame below commits what
    // it said.
    key(&mut app, "y", ready);
    assert!(
        answered(&mut app, &link, &[], ready).is_empty(),
        "nothing sent"
    );
    let said = notices(&mut app, ready);
    let said = said.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(said.contains(TALLER), "{said}");
    // Taller, the same key accepts.
    app.resize(98, 60);
    let later = ready + Duration::from_secs(2);
    let _ = live_text(&mut app, later);
    press(&mut app, "y", later);
    assert_eq!(
        answered(&mut app, &link, &[], later),
        [change("k_1", EntryAction::Accept, None)]
    );
}

#[test]
fn p_promotes_and_while_a_turn_runs_the_hint_says_so_and_the_refusal_is_said() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 60, &entries());
    app.resize(119, 60);
    let ready = project_focused(&mut app, &link, &entries(), now);
    press(&mut app, "p", ready);
    assert_eq!(
        answered(&mut app, &link, &[], ready),
        [change("k_1", EntryAction::Promote, None)]
    );
    // A turn of this session runs: the key says it waits for its end, and
    // the core refuses a promote meanwhile, with its sentence.
    link.give(event(Event::TurnStarted(TurnStarted {})));
    drain(&mut app, ready);
    let live = live_text(&mut app, ready);
    let hint = live.last().cloned().unwrap_or_default();
    assert!(hint.contains("p promotes after this turn"), "{hint}");
    let later = ready + Duration::from_secs(2);
    let _ = live_text(&mut app, later);
    press(&mut app, "p", later);
    let sentence = "A turn of this project is running: promote the entry once it ends.";
    assert_eq!(
        refused(&mut app, &link, "turn_running", sentence, later),
        [change("k_1", EntryAction::Promote, None)]
    );
    assert!(notices(&mut app, later).contains(sentence));
    // An entry of the repository's file, the third waiting, has nothing to
    // promote.
    for _ in 0..2 {
        key(&mut app, "down", later);
    }
    let hint = live_text(&mut app, later)
        .last()
        .cloned()
        .unwrap_or_default();
    assert!(
        hint.contains("y accepts") && !hint.contains("promotes"),
        "{hint}"
    );
}

#[test]
fn d_asks_in_the_row_y_then_deletes_once_ready_and_any_other_key_keeps_it() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 60, &entries());
    let ready = project_focused(&mut app, &link, &entries(), now);
    press(&mut app, "d", ready);
    let live = live_text(&mut app, ready);
    assert!(
        live.iter()
            .any(|row| row == "    delete this entry? y deletes · any other key keeps it"),
        "{live:#?}"
    );
    assert!(answered(&mut app, &link, &[], ready).is_empty());
    // Any other key keeps it.
    press(&mut app, "n", ready + GRACE);
    assert!(answered(&mut app, &link, &[], ready + GRACE).is_empty());
    let live = live_text(&mut app, ready + GRACE);
    assert!(
        !live.iter().any(|row| row.contains("delete this entry?")),
        "{live:#?}"
    );
    // Asked again, `y` deletes: the entry goes, though the result does not
    // carry it.
    let at = ready + Duration::from_secs(2);
    press(&mut app, "d", at);
    press(&mut app, "y", at + GRACE);
    assert_eq!(
        answered(&mut app, &link, &[], at + GRACE),
        [change("k_1", EntryAction::Delete, None)]
    );
    let live = live_text(&mut app, at + GRACE);
    assert!(
        live.iter()
            .any(|row| row == "◇ Project  2 entries wait for review · 1 in use"),
        "{live:#?}"
    );
}

/// The question changes the focused row: a `y` read before the grace
/// after it is drawn is no answer, and goes to the editor.
#[test]
fn a_y_straight_after_d_is_typing_not_a_delete() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 60, &entries());
    let ready = project_focused(&mut app, &link, &entries(), now);
    key(&mut app, "d", ready);
    key(&mut app, "y", ready);
    let _ = live_text(&mut app, ready);
    assert!(answered(&mut app, &link, &[], ready).is_empty());
    assert!(app.input().focus().on_input());
}

#[test]
fn n_turns_an_entry_off_and_e_edits_its_text_in_the_editor() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 60, &entries());
    let ready = project_focused(&mut app, &link, &entries(), now);
    press(&mut app, "n", ready);
    assert_eq!(
        answered(&mut app, &link, &[], ready),
        [change("k_1", EntryAction::Disable, None)]
    );
    let at = ready + Duration::from_secs(2);
    let _ = live_text(&mut app, at);
    press(&mut app, "e", at);
    type_text(&mut app, ", all of them", at);
    press(&mut app, "enter", at);
    assert_eq!(
        answered(&mut app, &link, &[], at),
        [change(
            "k_1",
            EntryAction::Edit,
            Some("the parser's tests, all of them")
        )]
    );
    assert_eq!(
        app.input().focus().focused(),
        Target::Panel(project_id(&app))
    );
}

#[test]
fn a_project_updated_event_changes_the_rows_and_closes_it_when_nothing_waits() {
    let now = Instant::now() + Duration::from_secs(20);
    let (mut app, link) = with_entries(now, 40, &entries());
    let project = project_id(&app);
    assert!(app.panels().is_open(project));
    let mut changed = entries();
    for entry in &mut changed {
        entry.accepted_at = Some(Timestamp::from("2026-11-16T12:00:00Z"));
        entry.status = EntryStatus::Active;
    }
    link.give(event(Event::ProjectUpdated(ProjectUpdated {
        entries: changed[..3].to_vec(),
        removed: vec![EntryId::from("k_4")],
    })));
    drain(&mut app, now);
    assert!(!app.panels().is_open(project), "nothing waits");
    // A new proposal opens it again.
    let mut proposal = entries()[1].clone();
    proposal.id = EntryId::from("k_5");
    link.give(event(Event::ProjectUpdated(ProjectUpdated {
        entries: vec![proposal],
        removed: Vec::new(),
    })));
    drain(&mut app, now);
    assert!(app.panels().is_open(project));
    let live = live_text(&mut app, now);
    assert!(
        live.iter()
            .any(|row| row == "◇ Project  1 entry waits for review · 3 in use"),
        "{live:#?}"
    );
}

#[test]
fn hostile_entries_are_drawn_visible_in_the_frame() {
    let now = Instant::now() + Duration::from_secs(20);
    let hostile = "\u{1b}[2J\u{202e}gone\u{200b}\u{9b}31m\u{e0041}";
    let mut entries = entries();
    for entry in &mut entries {
        entry.text = format!("{hostile}{}", entry.text);
        entry.evidence = Some(hostile.to_owned());
        if let Some(command) = &mut entry.command {
            command.command = format!("{hostile}{}", command.command);
            command.cwd = Some(hostile.to_owned());
        }
    }
    let (mut app, link) = with_entries(now, 60, &entries);
    let _ = project_focused(&mut app, &link, &entries, now);
    for columns in [20, 60, 120] {
        app.resize(columns - 1, 60);
        let (_, live, _) = app.frame(now);
        let problems = crate::testing::problems(&live, columns - 1);
        assert!(problems.is_empty(), "at {columns}: {problems:#?}");
        if columns < 120 {
            continue;
        }
        let shown = text(&live);
        assert!(
            shown.iter().any(|row| row.starts_with("◇ Project")),
            "the panel is drawn: {shown:#?}"
        );
        let escaped = shown
            .iter()
            .filter(|row| row.contains("\\u{202e}gone\\u{200b}"))
            .count();
        // The four entries' rows, and the focused command's line, folder,
        // text and evidence.
        assert!(escaped >= 7, "{escaped}: {shown:#?}");
    }
}
