//! Snapshot tests of outlines, one fixture file per language
//! (`fixtures/outlines/`), each with its outline in
//! `tests/it/snapshots/outlines/`.
//!
//! A snapshot lists every entry: its line, its depth (as indentation), its
//! kind, its name, its head and its doc summary. A change to an extractor
//! shows as a difference to review.

use std::fmt::Write as _;

use cineris_index::{Language, Outline, extract, text};

use super::{assert_snapshot, fixtures};

/// The snapshot's text of an outline.
fn listing(outline: &Outline) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "language: {}",
        outline.language.map_or("(fallback)", Language::name)
    );
    for entry in &outline.entries {
        let _ = write!(
            out,
            "{:>4} {:indent$}{} {}  | {}",
            entry.line,
            "",
            entry.kind.word(),
            entry.name,
            entry.head,
            indent = usize::from(entry.depth) * 2
        );
        if let Some(doc) = &entry.doc {
            let _ = write!(out, "  — {doc}");
        }
        out.push('\n');
    }
    if outline.truncated {
        out.push_str("(truncated)\n");
    }
    out
}

fn check(file: &str) {
    let path = fixtures().join("outlines").join(file);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let text = text::decode(&bytes).replace("\r\n", "\n");
    let language = Language::detect(file, &text);
    let outline = extract(language, file, &text);
    assert_snapshot(&format!("outlines/{file}.txt"), &listing(&outline));
}

#[test]
fn rust() {
    check("sample.rs");
}

#[test]
fn python() {
    check("sample.py");
}

#[test]
fn typescript() {
    check("sample.ts");
}

#[test]
fn javascript() {
    check("sample.js");
}

#[test]
fn java() {
    check("Sample.java");
}

#[test]
fn csharp() {
    check("Sample.cs");
}

#[test]
fn c() {
    check("sample.c");
}

#[test]
fn cpp() {
    check("sample.hpp");
}

#[test]
fn go() {
    check("sample.go");
}

#[test]
fn php() {
    check("sample.php");
}

#[test]
fn ruby() {
    check("sample.rb");
}

#[test]
fn kotlin() {
    check("Sample.kt");
}

#[test]
fn swift() {
    check("Sample.swift");
}

#[test]
fn shell() {
    check("sample.sh");
}

#[test]
fn markdown() {
    check("sample.md");
}

#[test]
fn fallback() {
    check("sample.yaml");
}

// What one outline of a hostile file costs in memory: before the fixes to
// step 18, an extractor kept a frame for each `{`, every heading and every
// line, and a name of the file's size, up to 26 times the file in all.

/// The most one outline of a file of 1 MiB may raise the process's peak
/// resident memory, in KiB. What the extractors keep is bounded; what is
/// left is the lexer's copy of the line being read and the line before
/// it, which is twice the longest line.
const PEAK_KIB: u64 = 4096;

/// Outlines `text` and checks how far that raised the process's peak
/// resident memory. Each test runs in a process of its own under nextest,
/// so the peak is this test's; it is reset to what is resident now just
/// before (`/proc/self/clear_refs`), so that making the text does not
/// hide what the outline costs. Measured where the system says it
/// (Linux).
fn check_peak(language: Option<Language>, file: &str, text: &str) {
    let reset = std::fs::write("/proc/self/clear_refs", "5").is_ok();
    let Some(before) = super::resident_kib("VmHWM:") else {
        if cfg!(target_os = "linux") {
            panic!("Linux says its peak resident memory");
        }
        return;
    };
    let outline = extract(language, file, text);
    let after = super::resident_kib("VmHWM:").expect("the figure again");
    assert!(!outline.entries.is_empty() || language.is_none());
    let grew = after.saturating_sub(before);
    assert!(
        grew <= PEAK_KIB,
        "outlining {} KiB of {file} raised the peak by {grew} KiB (the peak was reset first: \
         {reset})",
        text.len() / 1024
    );
}

/// `unit` repeated to 1 MiB, with `head` before it and `tail` after,
/// made in one allocation.
fn mebibyte(head: &str, unit: &str, tail: &str) -> String {
    let mut text = String::with_capacity(head.len() + (1 << 20) + tail.len());
    text.push_str(head);
    while text.len() + unit.len() <= head.len() + (1 << 20) {
        text.push_str(unit);
    }
    text.push_str(tail);
    text
}

#[test]
fn outlining_a_line_of_a_million_open_braces_takes_little_memory() {
    check_peak(
        Some(Language::Rust),
        "a.rs",
        &mebibyte("fn f() ", "{", "\n"),
    );
}

#[test]
fn outlining_a_name_of_a_megabyte_takes_little_memory() {
    check_peak(
        Some(Language::Rust),
        "a.rs",
        &mebibyte("fn ", "a", "() {}\n"),
    );
}

#[test]
fn outlining_a_megabyte_of_headings_takes_little_memory() {
    check_peak(Some(Language::Markdown), "a.md", &mebibyte("", "# a\n", ""));
}

#[test]
fn outlining_a_megabyte_of_short_lines_takes_little_memory() {
    check_peak(None, "a.txt", &mebibyte("", "a\n", ""));
}
