//! The TZif reader against a file whose counts are not what it holds
//! (found by the independent review of step 16).

#![allow(unsafe_code, reason = "a test that limits its own address space")]

/// A version 2 file: a first block with one local time type, then a second
/// header that says it has 4,294,967,295 of them, and nothing after it:
/// 94 bytes.
fn lying_file() -> Vec<u8> {
    let header = |typecnt: u32| {
        let mut header = Vec::new();
        header.extend_from_slice(b"TZif2");
        header.extend_from_slice(&[0u8; 15]);
        // isutcnt, isstdcnt, leapcnt, timecnt, typecnt, charcnt
        for count in [0, 0, 0, 0, typecnt, 0] {
            header.extend_from_slice(&u32::to_be_bytes(count));
        }
        header
    };
    let mut bytes = header(1);
    // The first block's one type: offset 0, not daylight, abbreviation 0.
    bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    bytes.extend(header(u32::MAX));
    bytes
}

/// The reader allocated for the counts the header gave before it checked
/// that the file held them. Under an address-space limit of 4 GiB (a
/// container, `ulimit -v`, or a machine whose RAM and swap are below the
/// 16 GiB asked for) a 94-byte file aborted the process, and the
/// statistics' writer thread reads the zone, so the session went with it.
#[cfg(target_os = "linux")]
#[test]
fn a_zone_file_that_lies_about_its_counts_is_refused_without_a_large_allocation() {
    let limit = libc::rlimit {
        rlim_cur: 4 << 30,
        rlim_max: 4 << 30,
    };
    // SAFETY: a plain system call on this test's own process, with a
    // pointer to a value that lives across it (nextest runs each test in a
    // process of its own, so no other test is limited).
    let set = unsafe { libc::setrlimit(libc::RLIMIT_AS, &raw const limit) };
    assert_eq!(set, 0);
    let bytes = lying_file();
    assert_eq!(bytes.len(), 94);
    assert!(cineris_base::zone::Transitions::parse(&bytes).is_none());
}

/// The same file, and one whose first block lies, are refused on every
/// system: the counts are checked against the file's length.
#[test]
fn a_zone_file_shorter_than_its_counts_is_refused() {
    assert!(cineris_base::zone::Transitions::parse(&lying_file()).is_none());
    let mut first_lies = lying_file();
    // The first header's `timecnt`: a billion transitions in 94 bytes.
    first_lies[32..36].copy_from_slice(&u32::to_be_bytes(1_000_000_000));
    assert!(cineris_base::zone::Transitions::parse(&first_lies).is_none());
}
