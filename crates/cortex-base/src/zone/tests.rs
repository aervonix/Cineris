use super::*;

fn at(text: &str) -> UtcTime {
    UtcTime::parse(text).unwrap()
}

fn date(text: &str) -> LocalDate {
    LocalDate::parse(text).unwrap()
}

const LONDON: &str = "GMT0BST,M3.5.0/1,M10.5.0";
const SYDNEY: &str = "AEST-10AEDT,M10.1.0,M4.1.0/3";

#[test]
fn a_date_reads_writes_and_counts() {
    let day = date("2024-02-29");
    assert_eq!(day.to_string(), "2024-02-29");
    assert_eq!(day.add_days(1).to_string(), "2024-03-01");
    assert_eq!(day.first_of_month().to_string(), "2024-02-01");
    assert_eq!(day.month_text(), "2024-02");
    assert_eq!(LocalDate::from_days(day.days()), day);
    // 1 January 2024 was a Monday, 7 January a Sunday.
    assert_eq!(date("2024-01-01").weekday(), 0);
    assert_eq!(date("2024-01-07").weekday(), 6);
    for wrong in ["2023-02-29", "2024-13-01", "2024-1-01", "2024-01-00", "abcd-ef-gh"] {
        assert!(LocalDate::parse(wrong).is_none(), "{wrong}");
    }
}

#[test]
fn a_fixed_zone_moves_the_date_at_its_own_midnight() {
    let new_york_winter = Zone::fixed(-5 * 3600);
    assert_eq!(new_york_winter.name(), "UTC-05:00");
    assert_eq!(
        new_york_winter.date_of(at("2026-11-16T04:59:59Z")),
        date("2026-11-15")
    );
    assert_eq!(
        new_york_winter.date_of(at("2026-11-16T05:00:00Z")),
        date("2026-11-16")
    );
    assert_eq!(
        new_york_winter.start_of(date("2026-11-16")),
        at("2026-11-16T05:00:00Z")
    );
    let tokyo = Zone::fixed(9 * 3600);
    assert_eq!(tokyo.date_of(at("2026-11-15T15:00:00Z")), date("2026-11-16"));
}

#[test]
fn a_posix_rule_changes_offset_on_its_days() {
    let london = Zone::posix(LONDON).unwrap();
    // Summer time begins on the last Sunday of March at 01:00 UTC.
    assert_eq!(london.offset_at(at("2026-03-29T00:59:59Z")), 0);
    assert_eq!(london.offset_at(at("2026-03-29T01:00:00Z")), 3600);
    // And ends on the last Sunday of October at 02:00 summer time.
    assert_eq!(london.offset_at(at("2026-10-25T00:59:59Z")), 3600);
    assert_eq!(london.offset_at(at("2026-10-25T01:00:00Z")), 0);
    // A local day in summer begins at 23:00 UTC the day before.
    assert_eq!(
        london.start_of(date("2026-07-01")),
        at("2026-06-30T23:00:00Z")
    );
    assert_eq!(
        london.start_of(date("2026-03-29")),
        at("2026-03-29T00:00:00Z")
    );
    assert_eq!(london.date_of(at("2026-06-30T23:30:00Z")), date("2026-07-01"));

    // The southern hemisphere: summer spans the new year.
    let sydney = Zone::posix(SYDNEY).unwrap();
    assert_eq!(sydney.offset_at(at("2026-01-15T00:00:00Z")), 11 * 3600);
    assert_eq!(sydney.offset_at(at("2026-07-15T00:00:00Z")), 10 * 3600);

    for wrong in ["", "G", "GMT", "GMT0BST,M13.5.0,M10.5.0", "GMT0BST,M3.5.0"] {
        assert!(PosixRule::parse(wrong).is_none(), "{wrong}");
    }
    assert_eq!(Zone::posix("UTC0").unwrap().offset_at(at("2026-07-01T00:00:00Z")), 0);
    assert_eq!(
        Zone::posix("<+0530>-5:30").unwrap().offset_at(at("2026-07-01T00:00:00Z")),
        19_800
    );
}

/// A version 2 zone file with the given changes and footer.
fn tzif(changes: &[(i64, usize)], offsets: &[i32], footer: &str) -> Vec<u8> {
    let header = |timecnt: usize, typecnt: usize, charcnt: usize| {
        let mut out = b"TZif2".to_vec();
        out.extend([0u8; 15]);
        for count in [0, 0, 0, timecnt, typecnt, charcnt] {
            out.extend(u32::try_from(count).unwrap().to_be_bytes());
        }
        out
    };
    let mut out = header(0, 1, 1);
    // The version 1 block: one type and its name, nothing else.
    out.extend(0i32.to_be_bytes());
    out.extend([0u8, 0u8]);
    out.push(0);
    out.extend(header(changes.len(), offsets.len(), 1));
    for (time, _) in changes {
        out.extend(time.to_be_bytes());
    }
    for (_, index) in changes {
        out.push(u8::try_from(*index).unwrap());
    }
    for offset in offsets {
        out.extend(offset.to_be_bytes());
        out.extend([0u8, 0u8]);
    }
    out.push(0);
    out.push(b'\n');
    out.extend(footer.as_bytes());
    out.push(b'\n');
    out
}

#[test]
fn a_zone_file_gives_its_transitions_then_its_rule() {
    // Standard time until 2000, then one hour ahead until 2010, then the
    // London rule.
    let y2000 = at("2000-01-01T00:00:00Z").unix_secs();
    let y2010 = at("2010-01-01T00:00:00Z").unix_secs();
    let bytes = tzif(&[(y2000, 1), (y2010, 0)], &[0, 3600], LONDON);
    let zone = Zone::from_tzif(&bytes, "Test/Zone").unwrap();
    assert_eq!(zone.name(), "Test/Zone");
    assert_eq!(zone.offset_at(at("1999-06-01T00:00:00Z")), 0);
    assert_eq!(zone.offset_at(at("2005-01-01T00:00:00Z")), 3600);
    assert_eq!(zone.offset_at(at("2026-07-01T00:00:00Z")), 3600);
    assert_eq!(zone.offset_at(at("2026-12-01T00:00:00Z")), 0);

    assert!(Zone::from_tzif(b"not a zone file", "x").is_none());
    assert!(Zone::from_tzif(&bytes[..30], "x").is_none());
}

#[cfg(not(windows))]
#[test]
fn tz_is_read_as_the_c_library_reads_it() {
    use std::ffi::OsStr;
    let utc = unix_zone(Some(OsStr::new(""))).unwrap();
    assert_eq!(utc.offset_at(at("2026-07-01T00:00:00Z")), 0);
    let rule = unix_zone(Some(OsStr::new(LONDON))).unwrap();
    assert_eq!(rule.offset_at(at("2026-07-01T00:00:00Z")), 3600);
    // A name that would leave the zone directory is not read as a file.
    assert!(unix_zone(Some(OsStr::new("../../etc/passwd"))).is_none());
}

#[test]
fn the_system_zone_answers() {
    // Whatever the machine is set to, the answer is within a day of UTC.
    let zone = Zone::system();
    let offset = zone.offset_at(at("2026-07-01T00:00:00Z"));
    assert!(offset.abs() <= 14 * 3600, "{offset}");
}

#[cfg(not(windows))]
#[test]
fn the_systems_own_zone_files_are_read() {
    // Where the zone database is installed (every CI runner has it), its
    // files read as the rules say.
    let Some(bytes) = read_small(std::path::Path::new("/usr/share/zoneinfo/Europe/London")) else {
        return;
    };
    let london = Zone::from_tzif(&bytes, "Europe/London").unwrap();
    for (time, offset) in [
        ("1990-07-01T00:00:00Z", 3600),
        ("1990-12-01T00:00:00Z", 0),
        ("2026-03-29T01:00:00Z", 3600),
        ("2026-10-25T01:00:00Z", 0),
        ("2045-07-01T00:00:00Z", 3600),
    ] {
        assert_eq!(london.offset_at(at(time)), offset, "{time}");
    }
}
