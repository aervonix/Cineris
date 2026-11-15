//! Snapshot tests of outlines, one fixture file per language
//! (`fixtures/outlines/`), each with its outline in
//! `tests/it/snapshots/outlines/`.
//!
//! A snapshot lists every entry: its line, its depth (as indentation), its
//! kind, its name, its head and its doc summary. A change to an extractor
//! shows as a difference to review.

use std::fmt::Write as _;

use cortex_index::{Language, Outline, extract, text};

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
