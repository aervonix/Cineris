//! What a day's limits read: every process's request lines of the day,
//! less the ones this process counts as they happen (found by the
//! independent review of step 16: a day's limit held per process).

use cortex_base::{LocalDate, Zone};
use cortex_protocol::SessionId;
use cortex_stats::{Aggregates, DayReader, DaySpent, Line, Record, StatsDir};
use cortex_testkit::role::{Report, Role};

use crate::{clock, recorder, request, scratch, tool};

/// A child that records one request into the directory it is given, and
/// says its recorder's id.
pub(crate) const WRITER: Role = Role::new("cxstats-writer=");

fn day() -> LocalDate {
    LocalDate::parse("2026-11-15").unwrap()
}

/// A request of `session`, `nanos` in cost, as `writer` wrote it.
fn line(day: &str, writer: Option<&str>, session: &str, nanos: u64) -> String {
    let mut record = request(&format!("{day}T10:00:00Z"), "p", "opus", nanos);
    if let Record::Request(request) = &mut record {
        request.session = SessionId::new(session);
    }
    let mut text = serde_json::to_string(&Line {
        day: day.to_owned(),
        writer: writer.map(str::to_owned),
        record,
    })
    .unwrap();
    text.push('\n');
    text
}

fn append(dir: &StatsDir, text: &str) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.events_file("2026-11"))
        .unwrap();
    std::io::Write::write_all(&mut file, text.as_bytes()).unwrap();
}

/// What one request of the test's own costs in tokens.
const TOKENS: u64 = 100 + 1000 + 10 + 50;

fn start(dir: &StatsDir, writer: &str) -> (DayReader, DaySpent) {
    let mut aggregates = Aggregates::load(dir).unwrap();
    aggregates.sync(dir, day(), "2026-11", None).unwrap();
    DayReader::start(dir, day(), &aggregates, Some(writer.to_owned()))
}

#[test]
fn a_days_reader_counts_every_other_processs_requests_and_skips_its_own() {
    let scratch = scratch("a_days_reader_counts");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    // Before the reader starts: its own lines count too, as nothing in
    // memory has them.
    append(&dir, &line("2026-11-15", Some("w_mine"), "s_mine", 1));
    let (mut reader, before) = start(&dir, "w_mine");
    assert_eq!(
        before,
        DaySpent {
            requests: 1,
            tokens: TOKENS,
            usd_nanos: 1
        }
    );
    assert_eq!(reader.own("s_mine").unwrap(), DaySpent::default());
    let text = [
        // This process's own session: charged as it happened, skipped.
        line("2026-11-15", Some("w_mine"), "s_mine", 1_000),
        // This process's session that does not draw on the day: counted.
        line("2026-11-15", Some("w_mine"), "s_other", 10),
        // Another process, in a session of the same name: counted.
        line("2026-11-15", Some("w_theirs"), "s_mine", 20),
        // A build before the writer was written: counted.
        line("2026-11-15", None, "s_old", 40),
        // Another day: not counted.
        line("2026-11-14", Some("w_theirs"), "s_x", 80),
    ]
    .concat();
    append(&dir, &text);
    // Not a request: not counted.
    let mut tool_line = serde_json::to_string(&Line {
        day: "2026-11-15".to_owned(),
        writer: None,
        record: tool("2026-11-15T10:00:00Z", "p", "read"),
    })
    .unwrap();
    tool_line.push('\n');
    append(&dir, &tool_line);
    assert_eq!(
        reader.catch_up().unwrap(),
        DaySpent {
            requests: 3,
            tokens: 3 * TOKENS,
            usd_nanos: 70
        }
    );
    assert_eq!(reader.catch_up().unwrap(), DaySpent::default(), "read once");
}

/// After `cortex stats reset`, a file made again under the month's name is
/// read from its start.
#[test]
fn a_days_reader_reads_a_new_file_from_its_start() {
    let scratch = scratch("a_days_reader_reads_a_new_file");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    append(&dir, &line("2026-11-15", Some("w_theirs"), "s_1", 1).repeat(3));
    let (mut reader, before) = start(&dir, "w_mine");
    assert_eq!(before.requests, 3);
    dir.reset().unwrap();
    append(&dir, &line("2026-11-15", Some("w_theirs"), "s_1", 1));
    assert_eq!(reader.catch_up().unwrap().requests, 1);
}

/// A request line says which recorder wrote it; other lines do not.
#[test]
fn a_request_line_says_which_recorder_wrote_it() {
    let scratch = scratch("a_request_line_says_which_recorder");
    let dir = StatsDir::new(scratch.join("stats"));
    let recorder = recorder(&dir, Zone::utc(), clock("2026-11-15T10:00:00Z"), 12);
    assert!(recorder.writer().starts_with("w_"), "{}", recorder.writer());
    recorder.record(request("2026-11-15T10:00:00Z", "p", "opus", 1));
    recorder.record(tool("2026-11-15T10:00:00Z", "p", "read"));
    recorder.flush();
    let text = std::fs::read_to_string(dir.events_file("2026-11")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(
        lines[0].contains(&format!(r#""writer":"{}""#, recorder.writer())),
        "{text}"
    );
    assert!(!lines[1].contains("writer"), "{text}");
}

/// Two processes' recorders write under ids of their own, so that neither
/// takes the other's spending for its own.
#[test]
fn sys_two_processes_write_under_ids_of_their_own() {
    let scratch = scratch("sys_two_processes_write");
    let dir = scratch.join("stats");
    for _ in 0..2 {
        let status = WRITER
            .command(&dir.to_string_lossy())
            .status()
            .expect("the child starts");
        assert!(status.success());
    }
    let report = Report::in_dir(&dir);
    let ids: Vec<String> = report
        .lines()
        .into_iter()
        .filter(|(name, _)| name == "writer")
        .map(|(_, id)| id)
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert_ne!(ids[0], ids[1]);
    let text =
        std::fs::read_to_string(StatsDir::new(&dir).events_file("2026-11")).unwrap();
    for id in &ids {
        assert!(text.contains(&format!(r#""writer":"{id}""#)), "{text}");
    }
}

/// The child: records one request in `dir` and says its recorder's id.
pub(crate) fn write_one(dir: &str) {
    let dir = StatsDir::new(dir);
    let recorder = recorder(&dir, Zone::utc(), clock("2026-11-15T10:00:00Z"), 12);
    recorder.record(request("2026-11-15T10:00:00Z", "p", "opus", 1));
    recorder.flush();
    Report::in_dir(dir.root()).say("writer", recorder.writer());
}
