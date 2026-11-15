//! The extractors' cases beside the fixtures: the shapes of text a file
//! can have (line endings, a byte-order mark, bytes that are not UTF-8,
//! very long lines), nesting, and the caps.

use super::extract;
use crate::language::Language;
use crate::outline::{Kind, MAX_ENTRIES};
use crate::text::decode;

/// `(depth, kind, name)` of each entry.
fn shape(language: Option<Language>, file: &str, text: &str) -> Vec<(u8, Kind, String)> {
    extract(language, file, text)
        .entries
        .into_iter()
        .map(|entry| (entry.depth, entry.kind, entry.name))
        .collect()
}

fn entry(depth: u8, kind: Kind, name: &str) -> (u8, Kind, String) {
    (depth, kind, name.to_owned())
}

#[test]
fn methods_nest_in_their_impl_and_class_in_every_brace_language() {
    let cases: [(Language, &str, &str, Kind); 9] = [
        (
            Language::Rust,
            "a.rs",
            "impl A {\n    fn m(&self) {}\n}\n",
            Kind::Impl,
        ),
        (
            Language::Java,
            "A.java",
            "class A {\n    void m() {}\n}\n",
            Kind::Class,
        ),
        (
            Language::CSharp,
            "A.cs",
            "class A\n{\n    void m() {}\n}\n",
            Kind::Class,
        ),
        (
            Language::Kotlin,
            "A.kt",
            "class A {\n    fun m() {}\n}\n",
            Kind::Class,
        ),
        (
            Language::Swift,
            "A.swift",
            "class A {\n    func m() {}\n}\n",
            Kind::Class,
        ),
        (
            Language::Cpp,
            "a.hpp",
            "class A {\n    void m();\n};\n",
            Kind::Class,
        ),
        (
            Language::TypeScript,
            "a.ts",
            "class A {\n  m() {}\n}\n",
            Kind::Class,
        ),
        (
            Language::Php,
            "a.php",
            "class A {\n    function m() {}\n}\n",
            Kind::Class,
        ),
        (
            Language::Go,
            "a.go",
            "type A interface {\n\tm()\n}\n",
            Kind::Interface,
        ),
    ];
    for (language, file, text, kind) in cases {
        let found = shape(Some(language), file, text);
        assert_eq!(
            found,
            vec![entry(0, kind, "A"), entry(1, Kind::Method, "m")],
            "{}",
            language.name()
        );
    }
}

#[test]
fn methods_nest_in_their_class_in_the_indentation_languages() {
    assert_eq!(
        shape(
            Some(Language::Python),
            "a.py",
            "class A:\n    def m(self):\n        def inner(): pass\n    class B:\n        def n(self): pass\ndef f(): pass\n"
        ),
        vec![
            entry(0, Kind::Class, "A"),
            entry(1, Kind::Method, "m"),
            entry(1, Kind::Class, "B"),
            entry(2, Kind::Method, "n"),
            entry(0, Kind::Function, "f"),
        ]
    );
    assert_eq!(
        shape(
            Some(Language::Ruby),
            "a.rb",
            "module M\n  class A\n    def m\n    end\n  end\nend\n"
        ),
        vec![
            entry(0, Kind::Module, "M"),
            entry(1, Kind::Class, "A"),
            entry(2, Kind::Method, "m"),
        ]
    );
}

#[test]
fn nothing_inside_a_function_body_is_outlined() {
    assert_eq!(
        shape(
            Some(Language::Rust),
            "a.rs",
            "fn outer() {\n    struct Local;\n    fn inner() {}\n}\nfn after() {}\n"
        ),
        vec![
            entry(0, Kind::Function, "outer"),
            entry(0, Kind::Function, "after")
        ]
    );
}

#[test]
fn line_endings_and_a_byte_order_mark_do_not_change_the_outline() {
    let unix = "/// Doc.\nfn a() {\n}\nstruct B;\n";
    let windows = "\u{FEFF}/// Doc.\r\nfn a() {\r\n}\r\nstruct B;";
    let bytes = windows.as_bytes();
    let decoded = decode(bytes);
    assert_eq!(
        extract(Some(Language::Rust), "a.rs", unix),
        extract(Some(Language::Rust), "a.rs", &decoded)
    );
}

#[test]
fn bytes_that_are_not_utf8_are_read_around() {
    let bytes = b"fn a\xFF() {}\nfn b() {}\n";
    let text = decode(bytes);
    let found = shape(Some(Language::Rust), "a.rs", &text);
    assert_eq!(found.last(), Some(&entry(0, Kind::Function, "b")));
}

#[test]
fn a_very_long_line_is_read_and_its_head_is_cut() {
    let long = format!(
        "fn long({}) {{}}\nfn after() {{}}\n",
        "a: u8, ".repeat(100_000)
    );
    let outline = extract(Some(Language::Rust), "a.rs", &long);
    assert_eq!(outline.entries.len(), 2);
    assert!(outline.entries[0].head.chars().count() <= crate::outline::HEAD_CHARS);
    assert!(outline.entries[0].head.ends_with('…'));
}

#[test]
fn an_outline_stops_at_its_cap_and_says_so() {
    let text: String = (0..MAX_ENTRIES + 10)
        .map(|n| format!("fn f{n}() {{}}\n"))
        .collect();
    let outline = extract(Some(Language::Rust), "a.rs", &text);
    assert_eq!(outline.entries.len(), MAX_ENTRIES);
    assert!(outline.truncated);
    assert!(outline.render().ends_with("definitions.]\n"));
}

#[test]
fn a_brace_in_a_string_or_a_comment_does_not_open_a_block() {
    let text = "fn a() { let s = \"{\"; /* { */ }\n// {\nfn b() {}\n";
    assert_eq!(
        shape(Some(Language::Rust), "a.rs", text),
        vec![entry(0, Kind::Function, "a"), entry(0, Kind::Function, "b")]
    );
}

#[test]
fn a_definition_whose_body_never_comes_lets_what_follows_be_read() {
    // A struct with no body, then an impl whose brace is on a later line.
    let text = "struct Unit;\nimpl<T> X for T\nwhere\n    T: Copy,\n{\n    fn m() {}\n}\n";
    assert_eq!(
        shape(Some(Language::Rust), "a.rs", text),
        vec![
            entry(0, Kind::Struct, "Unit"),
            entry(0, Kind::Impl, "X for T"),
            entry(1, Kind::Method, "m"),
        ]
    );
}

#[test]
fn a_c_prototype_is_outlined_in_a_header_and_not_in_a_source_file() {
    let text = "int f(void);\nint g(void) { return 0; }\n";
    assert_eq!(
        shape(Some(Language::C), "a.c", text),
        vec![entry(0, Kind::Function, "g")]
    );
    assert_eq!(
        shape(Some(Language::Cpp), "a.h", text),
        vec![entry(0, Kind::Function, "f"), entry(0, Kind::Function, "g")]
    );
}

#[test]
fn an_empty_or_binary_looking_text_has_no_outline() {
    for language in Language::ALL {
        assert!(extract(Some(language), "a", "").entries.is_empty());
    }
    assert!(extract(None, "a", "").entries.is_empty());
    assert!(
        extract(None, "a", "plain prose\nwith no indentation\n")
            .entries
            .is_empty()
    );
}

#[test]
fn the_fallback_finds_blocks_two_levels_deep() {
    let text = "a:\n  b:\n    c:\n      d: 1\nflat\n";
    assert_eq!(
        shape(None, "a.yaml", text),
        vec![entry(0, Kind::Block, "a:"), entry(1, Kind::Block, "b:")]
    );
}

#[test]
fn deep_nesting_costs_no_more_a_line_than_a_flat_file() {
    // A hundred thousand nested modules, and as many blocks whose contents
    // are at the level outside them: each line's depth and container are
    // known without walking the stack (the review of step 18). Before, this
    // took seconds; the test's own time limit is the check.
    let nested = "mod a {\n".repeat(100_000) + &"}\n".repeat(100_000);
    let outline = extract(Some(Language::Rust), "a.rs", &nested);
    assert_eq!(outline.entries.len(), MAX_ENTRIES);
    assert_eq!(
        outline.entries.last().map(|entry| entry.depth),
        Some(u8::try_from(MAX_ENTRIES - 1).unwrap_or(u8::MAX))
    );
    let transparent = "extern \"C\" {\n".repeat(100_000) + "fn inside();\n";
    let outline = extract(Some(Language::Rust), "a.rs", &transparent);
    assert_eq!(outline.entries.len(), 1);
}

/// Text made of the pieces the lexer and the recognisers look for, in any
/// order: quotes, comment markers, braces, keywords, heredocs, line breaks
/// and characters outside ASCII.
fn soup(rng: &mut cortex_testkit::SeededRng) -> String {
    const PIECES: &[&str] = &[
        "{",
        "}",
        "(",
        ")",
        "[",
        "]",
        ";",
        ":",
        ",",
        "\"",
        "'",
        "`",
        "\"\"\"",
        "'''",
        "/*",
        "*/",
        "//",
        "#",
        "#[",
        "<<EOF",
        "<<~TAG",
        "<<<X",
        "EOF",
        "=begin",
        "=end",
        "r#\"",
        "\"#",
        "@\"",
        "\\",
        "\n",
        "\r\n",
        "\t",
        "    ",
        " ",
        "fn ",
        "def ",
        "class ",
        "impl ",
        "struct ",
        "func ",
        "function ",
        "type ",
        "const ",
        "mod ",
        "interface ",
        "enum ",
        "extern \"C\" ",
        "namespace ",
        "template <",
        ">",
        "=",
        "=>",
        "é",
        "\u{202e}",
        "\u{0}",
        "x",
        "y1",
        "_",
        "```",
        "---",
        "===",
        "## ",
        "*",
    ];
    let count = cortex_testkit::cases::small(rng, 200);
    (0..count)
        .map(|_| *cortex_testkit::cases::pick(rng, PIECES))
        .collect()
}

#[test]
fn property_no_extractor_panics_or_passes_its_caps_on_any_text() {
    let mut languages: Vec<Option<Language>> = Language::ALL.into_iter().map(Some).collect();
    languages.push(None);
    cortex_testkit::cases::check("extractors_on_any_text", 400, soup, |text| {
        for language in &languages {
            for file in ["a", "a.h"] {
                let outline = extract(*language, file, text);
                if outline.entries.len() > MAX_ENTRIES {
                    return Err("too many entries".into());
                }
                for entry in &outline.entries {
                    if entry.head.chars().count() > crate::outline::HEAD_CHARS {
                        return Err(format!("a head too long: {entry:?}").into());
                    }
                    if entry.name.is_empty() {
                        return Err(format!("an empty name: {entry:?}").into());
                    }
                    if entry.head.contains('\n') {
                        return Err(format!("a head of two lines: {entry:?}").into());
                    }
                }
            }
        }
        Ok(())
    });
}
