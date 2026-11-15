//! What a day's limits read: every process's request lines of the day,
//! less the ones its own purse counted as they were made (found by the
//! independent review of step 16: a day's limit held per process).

use cortex_base::{LocalDate, Zone};
use cortex_protocol::SessionId;
use cortex_stats::{Aggregates, DayReader, DaySpent, Line, Record, StatsDir};
use cortex_testkit::role::{Report, Role};

use crate::{clock, recorder, request, scratch, tool};

/// A child that makes a recorder and says its id.
pub(crate) const WRITER: Role = Role::new("cxstats-writer=");

fn day() -> LocalDate {
    LocalDate::parse("2026-11-15").unwrap()
}

/// A request of `day`, `nanos` in cost, charged to `purse`.
fn line(day: &str, purse: Option<&str>, nanos: u64) -> String {
    let mut record = request(&format!("{day}T10:00:00Z"), "p", "opus", nanos);
    if let Record::Request(request) = &mut record {
        request.session = SessionId::new("s_1");
        request.charged_to = purse.map(str::to_owned);
    }
    let mut text = serde_json::to_string(&Line {
        day: day.to_owned(),
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

fn start(dir: &StatsDir, purse: &str) -> (DayReader, DaySpent) {
    let aggregates = Aggregates::load(dir).unwrap();
    let (mut reader, mut spent) = DayReader::start(dir, day(), &aggregates, Some(purse.to_owned()));
    spent.add(reader.catch_up().unwrap());
    (reader, spent)
}

#[test]
fn a_days_reader_counts_every_request_but_its_own_purses() {
    let scratch = scratch("a_days_reader_counts");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    // Before the reader starts, a line its purse counted cannot be there:
    // the purse is new. Another purse's is counted.
    append(&dir, &line("2026-11-15", Some("w_other/2026-11-15"), 1));
    let (mut reader, before) = start(&dir, "w_mine/2026-11-15");
    assert_eq!(
        before,
        DaySpent {
            requests: 1,
            tokens: TOKENS,
            usd_nanos: 1
        }
    );
    let text = [
        // This purse's own: charged as it was made, skipped.
        line("2026-11-15", Some("w_mine/2026-11-15"), 1_000),
        // A request no day's limit counted (a session without one): counted.
        line("2026-11-15", None, 10),
        // Another process's purse: counted.
        line("2026-11-15", Some("w_theirs/2026-11-15"), 20),
        // This process's purse of the day before, for a session that ran
        // past midnight: counted, as today's spending.
        line("2026-11-15", Some("w_mine/2026-11-14"), 40),
        // Another day: not counted.
        line("2026-11-14", Some("w_theirs/2026-11-14"), 80),
        // A blank line, as two writers that both ended a cut line leave.
        "\n".to_owned(),
    ]
    .concat();
    append(&dir, &text);
    // Not a request: not counted.
    let mut tool_line = serde_json::to_string(&Line {
        day: "2026-11-15".to_owned(),
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
/// read from its start, even where the file system gives it the deleted
/// file's inode number: the reader holds the file it read, so that number
/// is not free.
#[test]
fn a_days_reader_reads_a_new_file_from_its_start() {
    let scratch = scratch("a_days_reader_reads_a_new_file");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    append(
        &dir,
        &line("2026-11-15", Some("w_theirs/2026-11-15"), 1).repeat(3),
    );
    let (mut reader, before) = start(&dir, "w_mine/2026-11-15");
    assert_eq!(before.requests, 3);
    dir.reset().unwrap();
    assert_eq!(reader.catch_up().unwrap(), DaySpent::default());
    append(&dir, &line("2026-11-15", Some("w_theirs/2026-11-15"), 1));
    assert_eq!(reader.catch_up().unwrap().requests, 1);
}

/// A file cut back in place is not read again from where it was cut:
/// what was counted stays counted, and nothing is counted twice.
#[test]
fn a_days_reader_counts_nothing_twice_when_a_file_is_cut_back() {
    let scratch = scratch("a_days_reader_cut_back");
    let dir = StatsDir::new(scratch.join("stats"));
    std::fs::create_dir_all(dir.root()).unwrap();
    append(&dir, &line("2026-11-15", None, 1).repeat(3));
    let (mut reader, before) = start(&dir, "w_mine/2026-11-15");
    assert_eq!(before.requests, 3);
    let one = line("2026-11-15", None, 1);
    std::fs::OpenOptions::new()
        .write(true)
        .open(dir.events_file("2026-11"))
        .unwrap()
        .set_len(one.len() as u64)
        .unwrap();
    assert_eq!(reader.catch_up().unwrap(), DaySpent::default());
    append(&dir, &one);
    assert_eq!(reader.catch_up().unwrap().requests, 1);
}

/// Two processes' recorders have ids of their own, from which each
/// process's day's purse makes its id: neither takes the other's spending
/// for its own.
#[test]
fn sys_two_processes_have_recorders_of_their_own() {
    let scratch = scratch("sys_two_processes_have_recorders");
    let dir = scratch.join("stats");
    std::fs::create_dir_all(&dir).unwrap();
    for _ in 0..2 {
        let status = WRITER
            .command(&dir.to_string_lossy())
            .status()
            .expect("the child starts");
        assert!(status.success());
    }
    let ids: Vec<String> = Report::in_dir(&dir)
        .lines()
        .into_iter()
        .filter(|(name, _)| name == "writer")
        .map(|(_, id)| id)
        .collect();
    assert_eq!(ids.len(), 2, "{ids:?}");
    assert!(ids.iter().all(|id| id.starts_with("w_")), "{ids:?}");
    assert_ne!(ids[0], ids[1]);
}

/// The child: makes a recorder in `dir`, at the same fake time as the
/// other child, and says its id.
pub(crate) fn write_one(dir: &str) {
    let dir = StatsDir::new(dir);
    let recorder = recorder(&dir, Zone::utc(), clock("2026-11-15T10:00:00Z"), 12);
    Report::in_dir(dir.root()).say("writer", recorder.writer());
}
