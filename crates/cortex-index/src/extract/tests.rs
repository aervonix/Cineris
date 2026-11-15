//! The extractors' cases beside the fixtures: the shapes of text a file
//! can have (line endings, a byte-order mark, bytes that are not UTF-8,
//! very long lines), nesting, and the caps.

use std::time::{Duration, Instant};

use super::extract;
use crate::language::Language;
use crate::outline::{DOC_CHARS, HEAD_CHARS, Kind, MAX_ENTRIES, NAME_CHARS};
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

// The cases of the review of step 18's fixes: hostile prefixes, bounds,
// and five common shapes that had a wrong outline.

/// Runs `f` on a thread with the stack a pass's threads have: what
/// `std::thread::scope` and `std::thread::spawn` give by default.
fn on_a_pass_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("a thread")
        .join()
        .expect("no panic")
}

/// `unit` repeated to about `bytes` bytes, then `tail`.
fn chain(unit: &str, bytes: usize, tail: &str) -> String {
    let mut text = unit.repeat(bytes / unit.len());
    text.push_str(tail);
    text
}

#[test]
fn a_chain_of_template_and_extern_prefixes_needs_no_more_stack_than_one() {
    // Each `template<…>` and `extern "…"` was a level of recursion: a line
    // of eleven kilobytes of them overflowed a pass thread's stack in the
    // dev profile, which aborts the process. Now no input recurses.
    let mut lines: Vec<String> = vec![
        chain("template<> ", 22_000, "int f();\n"),
        chain("extern \"C\" ", 22_000, "int f();\n"),
    ];
    // And a megabyte of each spelling of each, mixed.
    for unit in [
        "template<> extern \"C\" ",
        "extern\"\"template<typename T> ",
    ] {
        lines.push(chain(unit, 1 << 20, "int f();\n"));
    }
    for text in lines {
        let found = on_a_pass_thread(move || shape(Some(Language::Cpp), "a.hpp", &text));
        assert_eq!(found, vec![entry(0, Kind::Function, "f")]);
    }
}

#[test]
fn a_chain_of_extern_prefixes_costs_linear_time() {
    // Each `extern "…"` scanned the rest of its line for a `{`: quadratic
    // in the number of them, 2.5 s for a header of 1 MiB.
    assert_linear(
        Language::Cpp,
        "a.hpp",
        "extern \"C\" ",
        64 << 10,
        "int f();\n",
    );
}

#[test]
fn a_line_of_regular_expressions_that_never_close_costs_linear_time() {
    // Each `/` after `(` may begin a regular expression, and the `[` after
    // it opens a class that never closes: were each scanned to the line's
    // end, the line would cost the square of its length. After the first
    // that does not close, no `/` of its line is tried.
    assert_linear(
        Language::JavaScript,
        "a.js",
        "(/[",
        8 << 10,
        "\nfunction f() {}\n",
    );
}

/// Asserts that outlining `unit` repeated, then `tail` (which defines one
/// thing), costs linear time: eight times the input takes about eight
/// times as long, not sixty-four. Time is compared with time on the same
/// thread, not with a figure: the bound is three times linear, on the best
/// of three runs of each size, taken in turn, and a large run too short to
/// time well (under 25 ms) passes.
fn assert_linear(language: Language, file: &'static str, unit: &str, bytes: usize, tail: &str) {
    let small = chain(unit, bytes, tail);
    let large = chain(unit, bytes * 8, tail);
    let (small_time, large_time) = on_a_pass_thread(move || {
        let time = |text: &str| {
            let start = Instant::now();
            let outline = extract(Some(language), file, text);
            assert_eq!(outline.entries.len(), 1);
            start.elapsed()
        };
        let mut best = (Duration::MAX, Duration::MAX);
        for _ in 0..3 {
            best.0 = best.0.min(time(&small));
            best.1 = best.1.min(time(&large));
        }
        best
    });
    assert!(
        large_time < Duration::from_millis(25) || large_time < small_time * 24,
        "eight times the input took {:.1} times as long ({small_time:?}, then {large_time:?})",
        large_time.as_secs_f64() / small_time.as_secs_f64().max(1e-9)
    );
}

#[test]
fn every_name_is_clipped_like_the_head() {
    let big = "a".repeat(10_000);
    let cases: Vec<(Option<Language>, &str, String)> = vec![
        (Some(Language::Rust), "a.rs", format!("fn {big}() {{}}\n")),
        (Some(Language::Rust), "a.rs", format!("impl {big} {{}}\n")),
        (
            Some(Language::C),
            "a.c",
            format!("typedef struct {{\n}} {big};\n"),
        ),
        (Some(Language::C), "a.c", format!("int {big}(void) {{}}\n")),
        (
            Some(Language::Cpp),
            "a.hpp",
            format!("namespace {big} {{}}\n"),
        ),
        (
            Some(Language::Swift),
            "A.swift",
            format!("func {big}() {{}}\n"),
        ),
        (
            Some(Language::Swift),
            "A.swift",
            format!("extension {big} {{}}\n"),
        ),
        (
            Some(Language::CSharp),
            "A.cs",
            format!("namespace {big} {{}}\n"),
        ),
        (
            Some(Language::TypeScript),
            "a.ts",
            format!("namespace {} {{}}\n", "a.".repeat(5_000)),
        ),
        (
            Some(Language::TypeScript),
            "a.ts",
            format!("declare module \"{big}\" {{}}\n"),
        ),
        (
            Some(Language::JavaScript),
            "a.js",
            format!("class A {{\n  #{big}() {{}}\n}}\n"),
        ),
        (Some(Language::Php), "a.php", format!("namespace {big};\n")),
        (
            Some(Language::Kotlin),
            "A.kt",
            format!("fun `{big}`() {{}}\n"),
        ),
        (Some(Language::Shell), "a.sh", format!("{big}() {{}}\n")),
        (
            Some(Language::Python),
            "a.py",
            format!("def {big}(): pass\n"),
        ),
        (Some(Language::Ruby), "a.rb", format!("def {big}\nend\n")),
        (
            Some(Language::Go),
            "a.go",
            format!("func ({big} T) {big}() {{}}\n"),
        ),
        (
            Some(Language::Markdown),
            "a.md",
            format!("# {}\n", "a ".repeat(100)),
        ),
        (None, "a.txt", format!("{big}\n  b\n")),
    ];
    for (language, file, text) in cases {
        let outline = extract(language, file, &text);
        assert!(!outline.entries.is_empty(), "{file}: no entry");
        for entry in &outline.entries {
            let chars = entry.name.chars().count();
            assert!(chars <= NAME_CHARS, "{file}: a name of {chars} characters");
            assert!(entry.head.chars().count() <= HEAD_CHARS, "{file}: head");
        }
    }
}

#[test]
fn the_caps_hold_on_long_hostile_inputs() {
    let long = "d".repeat(10_000);
    let inputs: Vec<(Option<Language>, &str, String)> = vec![
        (
            Some(Language::Rust),
            "a.rs",
            format!("/// {long}\nfn f() {{}}\n"),
        ),
        (
            Some(Language::Python),
            "a.py",
            format!("def f():\n    \"\"\"{long}\"\"\"\n"),
        ),
        (
            Some(Language::Python),
            "a.py",
            format!("def f():\n    \"\"\"\n    {long}\n"),
        ),
        (
            Some(Language::Ruby),
            "a.rb",
            format!("# {long}\ndef f\nend\n"),
        ),
        (
            Some(Language::Java),
            "A.java",
            format!("/** {long} */\nclass A {{}}\n"),
        ),
        (Some(Language::Markdown), "a.md", "# h\n".repeat(5_000)),
        (None, "a.txt", "a\n b\n".repeat(5_000)),
        (
            Some(Language::C),
            "a.c",
            format!("typedef struct {{\n}} {long};\n"),
        ),
        (
            Some(Language::Python),
            "a.py",
            format!("def f(\n{}", "    x,\n".repeat(10_000)),
        ),
    ];
    for (language, file, text) in inputs {
        let outline = extract(language, file, &text);
        assert!(outline.entries.len() <= MAX_ENTRIES, "{file}");
        for entry in &outline.entries {
            assert!(entry.head.chars().count() <= HEAD_CHARS, "{file}: head");
            assert!(entry.name.chars().count() <= NAME_CHARS, "{file}: name");
            assert!(
                entry
                    .doc
                    .as_ref()
                    .is_none_or(|doc| doc.chars().count() <= DOC_CHARS),
                "{file}: doc"
            );
        }
    }
}

#[test]
fn an_extern_c_function_with_a_body_is_a_function_and_not_a_block() {
    // A plugin's entry point: only a `{` right after the quotes opens an
    // `extern "C" {` block. Its local was outlined as a function.
    let text = "extern \"C\" int plugin_init(int argc) {\n    Widget w(argc);\n    return 0;\n}\n\nint other(void) {\n    return 1;\n}\n";
    assert_eq!(
        shape(Some(Language::Cpp), "plugin.h", text),
        vec![
            entry(0, Kind::Function, "plugin_init"),
            entry(0, Kind::Function, "other")
        ]
    );
    // A block whose brace is on the next line, and one on the same line.
    let text = "extern \"C\"\n{\nint a(void);\n}\nextern \"C\" {\nint b(void);\n}\nextern \"C\" int c(void);\n";
    assert_eq!(
        shape(Some(Language::Cpp), "a.h", text),
        vec![
            entry(0, Kind::Function, "a"),
            entry(0, Kind::Function, "b"),
            entry(0, Kind::Function, "c")
        ]
    );
}

#[test]
fn a_typescript_module_declared_by_a_string_is_named_by_the_string() {
    // The usual form in a `.d.ts`. The name was read from the code, where
    // a string's content is blanked: `"       "`.
    let text = "declare module \"express\" {\n  export function json(): void;\n}\ndeclare module 'fs' {}\nnamespace A.B {}\n";
    assert_eq!(
        shape(Some(Language::TypeScript), "index.d.ts", text),
        vec![
            entry(0, Kind::Module, "\"express\""),
            entry(1, Kind::Function, "json"),
            entry(0, Kind::Module, "'fs'"),
            entry(0, Kind::Module, "A.B"),
        ]
    );
}

#[test]
fn a_regular_expression_with_two_slashes_is_not_a_comment() {
    // `/^https?:\/\//` holds `//`: the rest of its line was blanked as a
    // comment, its `{` with it, and the class closed too early.
    let text = "class A {\n  a(u) {\n    if (/^https?:\\/\\//.test(u)) {\n      return 1;\n    }\n    return 0;\n  }\n  b() {}\n  c() {}\n}\nfunction after() {}\n";
    assert_eq!(
        shape(Some(Language::TypeScript), "a.ts", text),
        vec![
            entry(0, Kind::Class, "A"),
            entry(1, Kind::Method, "a"),
            entry(1, Kind::Method, "b"),
            entry(1, Kind::Method, "c"),
            entry(0, Kind::Function, "after"),
        ]
    );
}

#[test]
fn a_regular_expression_with_slash_star_does_not_open_a_comment() {
    // `/\/*$/` (trailing slashes) holds `/*`: a block comment opened and
    // ran to the end of the file.
    let text = "export function trim(p) {\n  return p.replace(/\\/*$/, \"\");\n}\nexport function a() {}\nexport function b() {}\n";
    assert_eq!(
        shape(Some(Language::JavaScript), "a.js", text),
        vec![
            entry(0, Kind::Function, "trim"),
            entry(0, Kind::Function, "a"),
            entry(0, Kind::Function, "b"),
        ]
    );
}

#[test]
fn a_shell_here_string_is_not_a_heredoc() {
    // `<<<word` is a here-string: at its second `<` the lexer saw
    // `<<word`, a heredoc, and blanked the file to a line `word`.
    let text =
        "decode() {\n  base64 -d <<<SGVsbG8=\n}\n\nbuild() {\n  make\n}\n\ndeploy() {\n  :\n}\n";
    assert_eq!(
        shape(Some(Language::Shell), "a.sh", text),
        vec![
            entry(0, Kind::Function, "decode"),
            entry(0, Kind::Function, "build"),
            entry(0, Kind::Function, "deploy"),
        ]
    );
}

#[test]
fn one_unclosed_bracket_in_python_does_not_hide_the_rest_of_the_file() {
    // A file in mid-edit. The bracket's depth never came back to 0, so
    // every line after it was read as part of its statement.
    let text = format!("x = call(\n{}", "def g():\n    pass\n".repeat(50));
    let outline = extract(Some(Language::Python), "a.py", &text);
    assert_eq!(outline.entries.len(), 50);
    // A definition deeper than the line that opened the bracket goes on
    // being read as part of it; one at its level or left of it ends it.
    let text = "class A:\n    def m(self, a,\n        def no(): pass\n    def n(self):\n        pass\ndef f(): pass\n";
    assert_eq!(
        shape(Some(Language::Python), "a.py", text),
        vec![
            entry(0, Kind::Class, "A"),
            entry(1, Kind::Method, "m"),
            entry(1, Kind::Method, "n"),
            entry(0, Kind::Function, "f"),
        ]
    );
}

#[test]
fn a_python_f_string_with_its_own_quotes_inside_does_not_hide_what_follows() {
    // Valid since Python 3.12 (PEP 701): the same quote inside a
    // replacement field. The lexer ends the string at the inner quote and
    // counts the `(` after it; the next definition at the line's level
    // ends that bracket.
    let text =
        "x = f\"{d[\"(\"]}\"\ndef g():\n    pass\nclass C:\n    def m(self):\n        pass\n";
    assert_eq!(
        shape(Some(Language::Python), "a.py", text),
        vec![
            entry(0, Kind::Function, "g"),
            entry(0, Kind::Class, "C"),
            entry(1, Kind::Method, "m"),
        ]
    );
}

#[test]
fn a_kotlin_function_named_in_backticks_is_outlined() {
    // A name in backticks may hold spaces, brackets and an apostrophe,
    // which must not open a character literal that blanks the line's `{`.
    let text = "class ParserTest {\n    @Test\n    fun `parses an empty file`() {\n    }\n    fun String.`reads (twice)`() {}\n    fun `doesn't crash`() {\n        val x = 1\n    }\n    fun plain() {}\n}\nfun after() {}\n";
    assert_eq!(
        shape(Some(Language::Kotlin), "ParserTest.kt", text),
        vec![
            entry(0, Kind::Class, "ParserTest"),
            entry(1, Kind::Method, "parses an empty file"),
            entry(1, Kind::Method, "reads (twice)"),
            entry(1, Kind::Method, "doesn't crash"),
            entry(1, Kind::Method, "plain"),
            entry(0, Kind::Function, "after"),
        ]
    );
}

#[test]
fn an_outline_is_truncated_only_when_more_definitions_than_the_cap_survive() {
    // In a C source file a prototype is outlined only if a body follows,
    // and a `typedef struct { … }` only if a name follows its close.
    let bodies: String = (0..MAX_ENTRIES)
        .map(|n| format!("int f{n}(void) {{ return 0; }}\n"))
        .collect();
    for tail in [
        "int prototype(void);\n",
        "typedef struct {\n    int x;\n} ;\n",
        "typedef struct {\n    int x;\n",
        // One taken out frees its place for the next, which may be taken
        // out too: at its close, and when another definition begins before
        // its body.
        "typedef struct {\n    int x;\n} ;\nint prototype(void);\n",
        "typedef struct\nint prototype(void);\n",
    ] {
        let outline = extract(Some(Language::C), "a.c", &format!("{bodies}{tail}"));
        assert_eq!(outline.entries.len(), MAX_ENTRIES, "{tail}");
        assert!(!outline.truncated, "{tail}");
    }
    for tail in [
        "int g(void) { return 1; }\n",
        "typedef struct {\n    int x;\n} Named;\n",
    ] {
        let outline = extract(Some(Language::C), "a.c", &format!("{bodies}{tail}"));
        assert_eq!(outline.entries.len(), MAX_ENTRIES, "{tail}");
        assert!(outline.truncated, "{tail}");
    }
    // A prototype that is dropped frees its place for a definition after
    // it, which is then the one too many.
    let text = format!("{bodies}int prototype(void);\nint g(void) {{ return 1; }}\n");
    let outline = extract(Some(Language::C), "a.c", &text);
    assert!(outline.truncated);
    assert_eq!(
        outline.entries.last().map(|entry| entry.name.as_str()),
        Some("f999")
    );
}

#[test]
fn markdown_keeps_its_first_headings_and_the_files_shallowest_level() {
    // Only the first headings are kept, but the depth of each is counted
    // from the shallowest level in the whole file.
    let text = "## a\n".repeat(MAX_ENTRIES + 5) + "# last\n";
    let outline = extract(Some(Language::Markdown), "a.md", &text);
    assert_eq!(outline.entries.len(), MAX_ENTRIES);
    assert!(outline.truncated);
    assert!(outline.entries.iter().all(|entry| entry.depth == 1));
    let text = "## a\n".repeat(MAX_ENTRIES);
    let outline = extract(Some(Language::Markdown), "a.md", &text);
    assert_eq!(outline.entries.len(), MAX_ENTRIES);
    assert!(!outline.truncated);
}

#[test]
fn blocks_that_open_no_definition_nest_as_the_definitions_around_them() {
    // Braces that open no definition are counted, not kept one by one:
    // below a definition's body, above it, and inside a signature.
    let text = "{\n    fn hidden() {}\n}\nmod m {\n    { }\n    fn a() {}\n    fn b() -> [u8; { 2 }] {\n        struct Local;\n    }\n    fn c() {}\n}\nfn after() {}\n";
    assert_eq!(
        shape(Some(Language::Rust), "a.rs", text),
        vec![
            entry(0, Kind::Module, "m"),
            entry(1, Kind::Function, "a"),
            entry(1, Kind::Function, "b"),
            entry(1, Kind::Function, "c"),
            entry(0, Kind::Function, "after"),
        ]
    );
    // A body that opens while a brace of its signature is still open: that
    // brace is closed after the body, at the level the module holds.
    let text = "mod m {\n    fn f(x: [u8; {\n    ]) {\n        struct Local;\n    }\n    }\n    fn g() {}\n}\nfn after() {}\n";
    assert_eq!(
        shape(Some(Language::Rust), "a.rs", text),
        vec![
            entry(0, Kind::Module, "m"),
            entry(1, Kind::Function, "f"),
            entry(1, Kind::Function, "g"),
            entry(0, Kind::Function, "after"),
        ]
    );
}
