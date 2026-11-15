//! What each brace language calls a definition.
//!
//! A recogniser reads the start of one line's code and says what begins
//! there. It sees only that line and where the line is (the definition
//! that holds it, the line of code before it): that is the whole of
//! "approximate".

use super::brace::{Body, Def, Found, Recogniser, Scope};
use super::docs::DocStyle;
use crate::language::Language;
use crate::lex::Syntax;
use crate::outline::Kind;

/// The recogniser of a brace language. `header` says a C or C++ file is a
/// header, where prototypes are the interface and are outlined.
pub(crate) fn recogniser(language: Language, header: bool) -> Box<dyn Recogniser> {
    match language {
        Language::Rust => Box::new(Rust),
        Language::Go => Box::new(Go::default()),
        Language::Java => Box::new(Java),
        Language::CSharp => Box::new(CSharp),
        Language::Kotlin => Box::new(Kotlin),
        Language::Swift => Box::new(Swift),
        Language::C | Language::Cpp => Box::new(Cish { header }),
        Language::JavaScript | Language::TypeScript => Box::new(Js),
        Language::Php => Box::new(Php),
        // The engines of the others are not this one; a shell's is.
        Language::Shell | Language::Python | Language::Ruby | Language::Markdown => {
            Box::new(Shell)
        }
    }
}

// ------------------------------------------------------------- helpers

fn is_ident(character: char) -> bool {
    character.is_alphanumeric() || character == '_' || character == '$'
}

/// The identifier at the start of `text`, and what follows it.
fn ident(text: &str) -> Option<(&str, &str)> {
    let first = text.chars().next()?;
    if !(first.is_alphabetic() || first == '_' || first == '$') {
        return None;
    }
    let end = text
        .char_indices()
        .find(|(_, c)| !is_ident(*c))
        .map_or(text.len(), |(at, _)| at);
    Some((&text[..end], &text[end..]))
}

/// What follows the word `word` at the start of `text`, when it is there
/// as a whole word.
fn word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(word)?;
    if rest.starts_with(is_ident) {
        return None;
    }
    Some(rest.trim_start())
}

/// `text` without any of `words` at its start, one after another.
fn strip_words<'a>(mut text: &'a str, words: &[&str]) -> &'a str {
    'again: loop {
        for candidate in words {
            if let Some(rest) = word(text, candidate) {
                text = rest;
                continue 'again;
            }
        }
        return text;
    }
}

/// Whether `words` has `candidate` among the words stripped from the start
/// of `text` (a `const` before `val`, an `export` before `const`).
fn has_word(text: &str, until: &str, candidate: &str) -> bool {
    let before = &text[..text.len() - until.len()];
    before.split_whitespace().any(|word| word == candidate)
}

/// What follows a bracketed group at the start of `text`, `<…>` or
/// `(…)`: the group is skipped whole, nested ones in it too.
fn skip_group(text: &str, open: char, close: char) -> Option<&str> {
    let mut depth = 0usize;
    for (at, character) in text.char_indices() {
        if character == open {
            depth += 1;
        } else if character == close {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(text[at + close.len_utf8()..].trim_start());
            }
        } else if depth == 0 {
            return None;
        }
    }
    None
}

/// `text` without the annotations at its start: `@Override`,
/// `@Test(timeout = 1)`, `@objc`.
fn strip_annotations(mut text: &str) -> &str {
    while let Some(rest) = text.strip_prefix('@') {
        if rest.starts_with("interface") {
            return text;
        }
        let Some((_, after)) = ident(rest) else {
            return text;
        };
        // A qualified name: `@a.b.C`.
        let mut after = after;
        while let Some((_, more)) = after.strip_prefix('.').and_then(ident) {
            after = more;
        }
        let after = after.trim_start();
        text = if after.starts_with('(') {
            skip_group(after, '(', ')').unwrap_or("")
        } else {
            after
        };
    }
    text
}

/// The words that open a statement, not a declaration.
const STATEMENTS: &[&str] = &[
    "if", "for", "foreach", "while", "switch", "catch", "return", "new", "throw", "else", "do",
    "try", "sizeof", "delete", "case", "goto", "using", "lock", "fixed", "await", "yield",
    "assert", "synchronized", "typeof", "when", "elif", "defer", "go", "select",
];

/// A method or a function declared C-style: `[type] name(` with a type
/// before the name, or none for a constructor of `container`. The name may
/// be qualified (`Parser::parse`, `~Parser`). Returns the name.
fn c_style(code: &str, container: Option<&str>, allow_bare: bool) -> Option<String> {
    let paren = code.find('(')?;
    let before = code[..paren].trim_end();
    if before.is_empty() || before.contains(['=', ';', '"', '{', '}', '.']) {
        return None;
    }
    if let Some((first, _)) = ident(before)
        && STATEMENTS.contains(&first)
    {
        return None;
    }
    // The name: the last identifier, with what qualifies it.
    let name_start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_ident(*c) || *c == ':' || *c == '~')
        .last()
        .map_or(before.len(), |(at, _)| at);
    let name = before[name_start..].trim_start_matches(':');
    let bare = name.trim_start_matches('~');
    if bare.is_empty() || !bare.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return None;
    }
    let kind_part = before[..name_start].trim_end();
    if kind_part.is_empty() {
        let last = bare.rsplit("::").next().unwrap_or(bare);
        if container.is_some_and(|container| container == last) || allow_bare {
            return Some(name.to_owned());
        }
        return None;
    }
    let last_word = kind_part.split_whitespace().last().unwrap_or("");
    if STATEMENTS.contains(&last_word) || matches!(last_word, "else" | "return") {
        return None;
    }
    kind_part
        .ends_with(|c: char| is_ident(c) || matches!(c, '>' | ']' | '?' | '*' | '&'))
        .then(|| name.to_owned())
}

/// The name after a keyword: an identifier, or nothing.
fn named(rest: &str) -> Option<String> {
    ident(rest).map(|(name, _)| name.to_owned())
}

/// A type-like definition: `class Name`, with `kind` and `body`.
fn typed(rest: &str, kind: Kind, body: Body) -> Found {
    match named(rest) {
        Some(name) => Found::Def(Def::new(kind, name, body)),
        None => Found::Nothing,
    }
}

fn inside_type(scope: &Scope<'_>) -> bool {
    scope.container.is_some_and(|(kind, _)| {
        matches!(
            kind,
            Kind::Class | Kind::Struct | Kind::Enum | Kind::Interface | Kind::Impl
        )
    })
}

fn container_name<'a>(scope: &Scope<'a>) -> Option<&'a str> {
    scope.container.map(|(_, name)| name)
}

fn function_or_method(scope: &Scope<'_>) -> Kind {
    if inside_type(scope) {
        Kind::Method
    } else {
        Kind::Function
    }
}

// ---------------------------------------------------------------- Rust

struct Rust;

impl Recogniser for Rust {
    fn syntax(&self) -> Syntax {
        Syntax::RUST
    }

    fn docs(&self) -> DocStyle {
        DocStyle {
            lines: &["///"],
            blocks: &["/**"],
        }
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        if code.starts_with("#[") || code.starts_with("#![") {
            return Found::Attribute;
        }
        let mut rest = code;
        if let Some(after) = word(rest, "pub") {
            rest = if after.starts_with('(') {
                skip_group(after, '(', ')').unwrap_or(after)
            } else {
                after
            };
        }
        loop {
            let before = rest;
            rest = strip_words(rest, &["default", "async", "unsafe", "auto"]);
            if let Some(after) = word(rest, "extern") {
                // `extern "C"`: the quotes stay, the name is blanked.
                rest = match after.strip_prefix('"') {
                    Some(quoted) => quoted.split_once('"').map_or(after, |(_, r)| r.trim_start()),
                    None => after,
                };
                if rest.starts_with('{') {
                    return Found::Transparent;
                }
            }
            if let Some(after) = word(rest, "const")
                && (word(after, "fn").is_some()
                    || word(after, "unsafe").is_some()
                    || word(after, "async").is_some()
                    || word(after, "extern").is_some())
            {
                rest = after;
            }
            if rest == before {
                break;
            }
        }
        if let Some(after) = word(rest, "fn") {
            return match named(after) {
                Some(name) => Found::Def(Def::new(function_or_method(scope), name, Body::Opaque)),
                None => Found::Nothing,
            };
        }
        if let Some(after) = word(rest, "struct").or_else(|| word(rest, "union")) {
            return typed(after, Kind::Struct, Body::Opaque);
        }
        if let Some(after) = word(rest, "enum") {
            return typed(after, Kind::Enum, Body::Opaque);
        }
        if let Some(after) = word(rest, "trait") {
            return typed(after, Kind::Interface, Body::Holds);
        }
        if let Some(after) = word(rest, "mod") {
            return typed(after, Kind::Module, Body::Holds);
        }
        if let Some(after) = word(rest, "type") {
            return typed(after, Kind::Type, Body::Opaque);
        }
        if let Some(after) = word(rest, "const").or_else(|| word(rest, "static")) {
            let after = strip_words(after, &["mut"]);
            return typed(after, Kind::Const, Body::Opaque);
        }
        if let Some(after) = rest.strip_prefix("macro_rules!") {
            return typed(after.trim_start(), Kind::Macro, Body::Opaque);
        }
        if let Some(after) = word(rest, "impl") {
            let after = if after.starts_with('<') {
                skip_group(after, '<', '>').unwrap_or(after)
            } else {
                after
            };
            let target = after
                .split_once('{')
                .map_or(after, |(target, _)| target);
            let target = target
                .split_once(" where")
                .map_or(target, |(target, _)| target);
            let name = crate::outline::squeeze(target);
            if name.is_empty() {
                return Found::Nothing;
            }
            return Found::Def(Def::new(Kind::Impl, crate::outline::clip(&name, 80), Body::Holds));
        }
        Found::Nothing
    }
}

// ------------------------------------------------------------------ Go

/// The group a `type (`, `const (` or `var (` opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GoGroup {
    Type,
    Const,
    Var,
}

#[derive(Default)]
struct Go {
    group: Option<GoGroup>,
}

impl Recogniser for Go {
    fn same_line_body(&self) -> bool {
        true
    }

    fn syntax(&self) -> Syntax {
        Syntax::GO
    }

    fn docs(&self) -> DocStyle {
        DocStyle {
            lines: &["//"],
            blocks: &[],
        }
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        if let Some(group) = self.group {
            if code.starts_with(')') {
                self.group = None;
                return Found::Nothing;
            }
            let Some((name, rest)) = ident(code) else {
                return Found::Nothing;
            };
            return match group {
                GoGroup::Var => Found::Nothing,
                GoGroup::Const => Found::Def(Def::new(Kind::Const, name, Body::Opaque)),
                GoGroup::Type => Self::type_def(name, rest.trim_start()),
            };
        }
        if matches!(scope.container, Some((Kind::Interface, _))) {
            // An interface's method: `Read(p []byte) (int, error)`.
            return match ident(code) {
                Some((name, rest)) if rest.starts_with('(') => {
                    Found::Def(Def::new(Kind::Method, name, Body::Opaque))
                }
                _ => Found::Nothing,
            };
        }
        if let Some(after) = word(code, "func") {
            if after.starts_with('(') {
                // A method: `func (s *Server) Start()`.
                let receiver = after.split_once(')').map(|(inside, rest)| (inside, rest.trim_start()));
                let Some((inside, rest)) = receiver else {
                    return Found::Nothing;
                };
                let receiver_type = inside
                    .split_whitespace()
                    .last()
                    .unwrap_or("")
                    .trim_start_matches(['(', '*']);
                let receiver_type = receiver_type.split('[').next().unwrap_or(receiver_type);
                return match ident(rest) {
                    Some((name, _)) => Found::Def(Def::new(
                        Kind::Method,
                        format!("{receiver_type}.{name}"),
                        Body::Opaque,
                    )),
                    None => Found::Nothing,
                };
            }
            return match ident(after) {
                Some((name, _)) => Found::Def(Def::new(Kind::Function, name, Body::Opaque)),
                None => Found::Nothing,
            };
        }
        for (keyword, group) in [
            ("type", GoGroup::Type),
            ("const", GoGroup::Const),
            ("var", GoGroup::Var),
        ] {
            if let Some(after) = word(code, keyword) {
                if after.starts_with('(') {
                    self.group = Some(group);
                    return Found::Nothing;
                }
                let Some((name, rest)) = ident(after) else {
                    return Found::Nothing;
                };
                return match group {
                    GoGroup::Type => Self::type_def(name, rest.trim_start()),
                    GoGroup::Const => Found::Def(Def::new(Kind::Const, name, Body::Opaque)),
                    GoGroup::Var => Found::Nothing,
                };
            }
        }
        Found::Nothing
    }
}

impl Go {
    fn type_def(name: &str, rest: &str) -> Found {
        // Generic parameters: `type List[T any] struct`.
        let rest = if rest.starts_with('[') {
            skip_group(rest, '[', ']').unwrap_or(rest)
        } else {
            rest
        };
        let (kind, body) = if word(rest, "struct").is_some() {
            (Kind::Struct, Body::Opaque)
        } else if word(rest, "interface").is_some() {
            (Kind::Interface, Body::Holds)
        } else {
            (Kind::Type, Body::Opaque)
        };
        Found::Def(Def::new(kind, name, body))
    }
}

// ---------------------------------------------------------------- Java

const JAVA_MODIFIERS: &[&str] = &[
    "public",
    "protected",
    "private",
    "static",
    "final",
    "abstract",
    "sealed",
    "non-sealed",
    "strictfp",
    "default",
    "synchronized",
    "native",
    "transient",
    "volatile",
];

struct Java;

impl Recogniser for Java {
    fn syntax(&self) -> Syntax {
        Syntax::JAVA
    }

    fn docs(&self) -> DocStyle {
        DocStyle::JAVADOC
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        let rest = strip_annotations(code);
        if rest.is_empty() {
            return Found::Attribute;
        }
        let rest = strip_words(rest, JAVA_MODIFIERS);
        for (keyword, kind) in [
            ("class", Kind::Class),
            ("interface", Kind::Interface),
            ("@interface", Kind::Interface),
            ("enum", Kind::Enum),
            ("record", Kind::Struct),
        ] {
            if let Some(after) = word(rest, keyword) {
                return typed(after, kind, Body::Holds);
            }
        }
        if inside_type(scope) {
            // A generic method: `<T> List<T> copy(`.
            let rest = if rest.starts_with('<') {
                skip_group(rest, '<', '>').unwrap_or(rest)
            } else {
                rest
            };
            if let Some(name) = c_style(rest, container_name(scope), false) {
                return Found::Def(Def::new(Kind::Method, name, Body::Opaque));
            }
        }
        Found::Nothing
    }
}

// ------------------------------------------------------------------ C#

const CSHARP_MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "readonly",
    "virtual",
    "override",
    "abstract",
    "sealed",
    "async",
    "partial",
    "extern",
    "unsafe",
    "new",
    "const",
    "volatile",
    "required",
    "file",
    "ref",
];

struct CSharp;

impl Recogniser for CSharp {
    fn syntax(&self) -> Syntax {
        Syntax::CSHARP
    }

    fn docs(&self) -> DocStyle {
        DocStyle {
            lines: &["///"],
            blocks: &["/**"],
        }
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        if code.starts_with('[') {
            return Found::Attribute;
        }
        let rest = strip_words(code, CSHARP_MODIFIERS);
        if let Some(after) = word(rest, "namespace") {
            let name: String = after
                .chars()
                .take_while(|c| is_ident(*c) || *c == '.')
                .collect();
            if name.is_empty() {
                return Found::Nothing;
            }
            return Found::Def(Def::new(Kind::Module, name, Body::Holds));
        }
        let rest_after_record = word(rest, "record").map(|after| {
            word(after, "struct")
                .or_else(|| word(after, "class"))
                .unwrap_or(after)
        });
        if let Some(after) = rest_after_record {
            return typed(after, Kind::Struct, Body::Holds);
        }
        for (keyword, kind, body) in [
            ("class", Kind::Class, Body::Holds),
            ("interface", Kind::Interface, Body::Holds),
            ("struct", Kind::Struct, Body::Holds),
            ("enum", Kind::Enum, Body::Opaque),
            ("delegate", Kind::Type, Body::Opaque),
        ] {
            if let Some(after) = word(rest, keyword) {
                if kind == Kind::Type {
                    // `delegate void Handler(object sender)`.
                    return match c_style(after, None, false) {
                        Some(name) => Found::Def(Def::new(Kind::Type, name, Body::Opaque)),
                        None => Found::Nothing,
                    };
                }
                return typed(after, kind, body);
            }
        }
        if inside_type(scope) {
            if word(rest, "event").is_some() {
                return Found::Nothing;
            }
            if let Some(name) = c_style(rest, container_name(scope), false) {
                return Found::Def(Def::new(Kind::Method, name, Body::Opaque));
            }
            // A property: `public int Count { get; }`, `string Name => …;`.
            // An initialiser's `=` before the brace makes it a field.
            let arrow = rest.find("=>");
            let opens = match (rest.find('{'), arrow) {
                (Some(brace), Some(arrow)) => Some(brace.min(arrow)),
                (brace, arrow) => brace.or(arrow),
            };
            let assigned = rest
                .match_indices('=')
                .any(|(at, _)| Some(at) != arrow && opens.is_none_or(|open| at < open));
            if let Some(open) = opens
                && !assigned
                && !rest[..open].contains('(')
            {
                let words: Vec<&str> = rest[..open].split_whitespace().collect();
                if words.len() >= 2
                    && let Some(name) = words.last()
                    && ident(name).is_some_and(|(_, tail)| tail.is_empty())
                {
                    return Found::Def(Def::new(Kind::Property, *name, Body::Opaque));
                }
            }
        }
        Found::Nothing
    }
}

// -------------------------------------------------------------- Kotlin

const KOTLIN_MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "internal",
    "open",
    "abstract",
    "final",
    "override",
    "sealed",
    "data",
    "annotation",
    "inner",
    "value",
    "inline",
    "noinline",
    "crossinline",
    "suspend",
    "tailrec",
    "operator",
    "infix",
    "external",
    "lateinit",
    "const",
    "expect",
    "actual",
    "enum",
];

struct Kotlin;

impl Recogniser for Kotlin {
    fn same_line_body(&self) -> bool {
        true
    }

    fn syntax(&self) -> Syntax {
        Syntax::KOTLIN
    }

    fn docs(&self) -> DocStyle {
        DocStyle::JAVADOC
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        let annotated = strip_annotations(code);
        if annotated.is_empty() {
            return Found::Attribute;
        }
        let rest = strip_words(annotated, KOTLIN_MODIFIERS);
        let is_enum = has_word(annotated, rest, "enum");
        if let Some(after) = word(rest, "companion") {
            let after = word(after, "object").unwrap_or(after);
            let name = named(after).unwrap_or_else(|| "companion object".to_owned());
            return Found::Def(Def::new(Kind::Class, name, Body::Holds));
        }
        if let Some(after) = word(rest, "fun") {
            if let Some(after) = word(after, "interface") {
                return typed(after, Kind::Interface, Body::Holds);
            }
            let after = if after.starts_with('<') {
                skip_group(after, '<', '>').unwrap_or(after)
            } else {
                after
            };
            // An extension function: `fun String.shout()`.
            let before_paren = after.split('(').next().unwrap_or(after);
            let name = before_paren.rsplit('.').next().unwrap_or(before_paren).trim();
            return match ident(name) {
                Some((name, _)) => {
                    Found::Def(Def::new(function_or_method(scope), name, Body::Opaque))
                }
                None => Found::Nothing,
            };
        }
        if let Some(after) = word(rest, "class") {
            let kind = if is_enum { Kind::Enum } else { Kind::Class };
            return typed(after, kind, Body::Holds);
        }
        if let Some(after) = word(rest, "interface") {
            return typed(after, Kind::Interface, Body::Holds);
        }
        if let Some(after) = word(rest, "object") {
            return typed(after, Kind::Class, Body::Holds);
        }
        if let Some(after) = word(rest, "typealias") {
            return typed(after, Kind::Type, Body::Opaque);
        }
        if has_word(annotated, rest, "const")
            && let Some(after) = word(rest, "val")
        {
            return typed(after, Kind::Const, Body::Opaque);
        }
        Found::Nothing
    }
}

// --------------------------------------------------------------- Swift

const SWIFT_MODIFIERS: &[&str] = &[
    "public",
    "private",
    "fileprivate",
    "internal",
    "open",
    "package",
    "static",
    "final",
    "override",
    "mutating",
    "nonmutating",
    "convenience",
    "required",
    "lazy",
    "weak",
    "unowned",
    "dynamic",
    "optional",
    "indirect",
    "prefix",
    "postfix",
    "infix",
    "nonisolated",
    "isolated",
    "distributed",
];

struct Swift;

impl Recogniser for Swift {
    fn same_line_body(&self) -> bool {
        true
    }

    fn syntax(&self) -> Syntax {
        Syntax::SWIFT
    }

    fn docs(&self) -> DocStyle {
        DocStyle {
            lines: &["///"],
            blocks: &["/**"],
        }
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        let mut rest = strip_annotations(code);
        if rest.is_empty() {
            return Found::Attribute;
        }
        loop {
            let before = rest;
            rest = strip_words(rest, SWIFT_MODIFIERS);
            // `private(set)`.
            if rest.starts_with('(') {
                rest = skip_group(rest, '(', ')').unwrap_or(rest);
            }
            // `class` before `func` or `var` is a modifier.
            if let Some(after) = word(rest, "class")
                && ["func", "var", "let", "subscript", "override", "final", "static"]
                    .iter()
                    .any(|next| word(after, next).is_some())
            {
                rest = after;
            }
            if rest == before {
                break;
            }
        }
        if let Some(after) = word(rest, "func") {
            let name: String = after
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '(' && *c != '<')
                .collect();
            if name.is_empty() {
                return Found::Nothing;
            }
            return Found::Def(Def::new(function_or_method(scope), name, Body::Opaque));
        }
        for special in ["init", "deinit", "subscript"] {
            if let Some(after) = rest.strip_prefix(special)
                && (after.is_empty() || after.starts_with(['(', '?', '!', '<', ' ', '{']))
            {
                return Found::Def(Def::new(Kind::Method, special, Body::Opaque));
            }
        }
        for (keyword, kind) in [
            ("class", Kind::Class),
            ("struct", Kind::Struct),
            ("enum", Kind::Enum),
            ("protocol", Kind::Interface),
            ("actor", Kind::Class),
        ] {
            if let Some(after) = word(rest, keyword) {
                return typed(after, kind, Body::Holds);
            }
        }
        if let Some(after) = word(rest, "extension") {
            let target: String = after
                .chars()
                .take_while(|c| is_ident(*c) || *c == '.' || *c == '<' || *c == '>')
                .collect();
            if target.is_empty() {
                return Found::Nothing;
            }
            return Found::Def(Def::new(Kind::Impl, target, Body::Holds));
        }
        if let Some(after) = word(rest, "typealias") {
            return typed(after, Kind::Type, Body::Opaque);
        }
        Found::Nothing
    }
}

// ----------------------------------------------------------- C and C++

const C_MODIFIERS: &[&str] = &[
    "static",
    "inline",
    "virtual",
    "explicit",
    "constexpr",
    "consteval",
    "constinit",
    "friend",
    "extern",
    "__inline",
    "__forceinline",
];

struct Cish {
    header: bool,
}

impl Recogniser for Cish {
    fn syntax(&self) -> Syntax {
        Syntax::C
    }

    fn docs(&self) -> DocStyle {
        DocStyle {
            lines: &["///", "//!", "//"],
            blocks: &["/**", "/*!"],
        }
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        let top = scope.container.is_none_or(|(kind, _)| kind == Kind::Module);
        if let Some(directive) = code.strip_prefix('#') {
            let directive = directive.trim_start();
            if top && let Some(after) = word(directive, "define") {
                let Some((name, rest)) = ident(after) else {
                    return Found::Nothing;
                };
                // An include guard defines a name to nothing.
                if rest.trim().is_empty() && !rest.starts_with('(') {
                    return Found::Nothing;
                }
                return Found::Def(Def::new(Kind::Macro, name, Body::Opaque));
            }
            return Found::Attribute;
        }
        if let Some(after) = word(code, "template") {
            let rest = skip_group(after, '<', '>').unwrap_or("");
            if rest.is_empty() {
                return Found::Attribute;
            }
            return self.recognise(rest, scope);
        }
        if code.ends_with(':') && ["public:", "private:", "protected:"].contains(&code) {
            return Found::Nothing;
        }
        if let Some(after) = word(code, "extern")
            && after.starts_with('"')
        {
            return if after.contains('{') {
                Found::Transparent
            } else {
                // `extern "C" int f(void);`
                self.recognise(after.split_once('"').map_or("", |(_, r)| r).split_once('"').map_or("", |(_, r)| r.trim_start()), scope)
            };
        }
        if let Some(after) = word(code, "namespace") {
            let name: String = after
                .chars()
                .take_while(|c| is_ident(*c) || *c == ':')
                .collect();
            if name.is_empty() {
                return Found::Transparent;
            }
            return Found::Def(Def::new(Kind::Module, name, Body::Holds));
        }
        if let Some(after) = word(code, "typedef") {
            for (keyword, kind) in [
                ("struct", Kind::Struct),
                ("union", Kind::Struct),
                ("enum", Kind::Enum),
            ] {
                if let Some(tagged) = word(after, keyword) {
                    if !after.contains('{') && !tagged.contains('{') && tagged.ends_with(';') {
                        break;
                    }
                    // `typedef struct Tag {`, or a name that comes at the close.
                    let name = ident(tagged)
                        .filter(|(_, rest)| rest.trim_start().starts_with('{') || rest.trim().is_empty())
                        .map_or_else(String::new, |(name, _)| name.to_owned());
                    return Found::Def(Def::new(kind, name, Body::Opaque));
                }
            }
            // `typedef int Size;`, `typedef void (*Handler)(int);`
            let name = if let Some((_, inner)) = after.split_once("(*") {
                inner.split(')').next().unwrap_or("").trim()
            } else {
                after
                    .trim_end_matches(';')
                    .split(|c: char| !is_ident(c))
                    .rfind(|word| !word.is_empty())
                    .unwrap_or("")
            };
            return match ident(name) {
                Some((name, _)) => Found::Def(Def::new(Kind::Type, name, Body::Opaque)),
                None => Found::Nothing,
            };
        }
        if let Some(after) = word(code, "using")
            && let Some((name, rest)) = ident(after)
            && rest.trim_start().starts_with('=')
        {
            return Found::Def(Def::new(Kind::Type, name, Body::Opaque));
        }
        let rest = strip_words(code, C_MODIFIERS);
        for (keyword, kind) in [
            ("class", Kind::Class),
            ("struct", Kind::Struct),
            ("union", Kind::Struct),
        ] {
            if let Some(after) = word(rest, keyword) {
                // A forward declaration, or a variable of the type.
                if !rest.contains('{') && rest.ends_with(';') {
                    return Found::Nothing;
                }
                let after = strip_annotations(after);
                let after = after
                    .strip_prefix("alignas")
                    .and_then(|a| skip_group(a.trim_start(), '(', ')'))
                    .unwrap_or(after);
                let Some((name, tail)) = ident(after) else {
                    return Found::Nothing;
                };
                // `struct point p = {…}` or `struct point *make(…)` is not one.
                let tail = tail.trim_start();
                if !(tail.is_empty() || tail.starts_with(['{', ':']) || word(tail, "final").is_some()) {
                    break;
                }
                return Found::Def(Def::new(kind, name, Body::Holds));
            }
        }
        if let Some(after) = word(rest, "enum") {
            let after = strip_words(after, &["class", "struct"]);
            if !rest.contains('{') && rest.ends_with(';') {
                return Found::Nothing;
            }
            return typed(after, Kind::Enum, Body::Opaque);
        }
        let in_type = inside_type(scope);
        // A function: GNU style puts its type on the line before.
        let bare = top
            && scope.previous.ends_with(|c: char| is_ident(c) || c == '*')
            && !scope.previous.starts_with('#');
        if let Some(name) = c_style(rest, container_name(scope), bare) {
            let kind = if in_type || name.contains("::") {
                Kind::Method
            } else {
                Kind::Function
            };
            let def = Def::new(kind, name, Body::Opaque);
            return Found::Def(if self.header || in_type { def } else { def.needing_body() });
        }
        Found::Nothing
    }
}

// ---------------------------------------- JavaScript and TypeScript

const JS_MEMBER_MODIFIERS: &[&str] = &[
    "static",
    "public",
    "private",
    "protected",
    "readonly",
    "abstract",
    "override",
    "declare",
    "async",
    "get",
    "set",
    "accessor",
];

struct Js;

impl Js {
    /// Whether what follows `=` in a declaration is a function.
    fn is_function(value: &str) -> bool {
        let value = strip_words(value, &["async"]);
        // `= (` alone: parameters that go on over the next lines.
        word(value, "function").is_some() || value.contains("=>") || value == "("
    }
}

impl Recogniser for Js {
    fn same_line_body(&self) -> bool {
        true
    }

    fn syntax(&self) -> Syntax {
        Syntax::JS
    }

    fn docs(&self) -> DocStyle {
        DocStyle::JAVADOC
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        if code.starts_with('@') {
            let rest = strip_annotations(code);
            if rest.is_empty() {
                return Found::Attribute;
            }
            return self.recognise(rest, scope);
        }
        if inside_type(scope) {
            return Self::member(code, scope);
        }
        let rest = strip_words(code, &["export", "default", "declare"]);
        let exported = rest.len() < code.len();
        if let Some(after) = word(strip_words(rest, &["async"]), "function") {
            let after = after.trim_start_matches('*').trim_start();
            let name = named(after).unwrap_or_else(|| "default".to_owned());
            return Found::Def(Def::new(Kind::Function, name, Body::Opaque));
        }
        if let Some(after) = word(strip_words(rest, &["abstract"]), "class") {
            let name = named(after)
                .filter(|name| name != "extends" && name != "implements")
                .unwrap_or_else(|| "default".to_owned());
            return Found::Def(Def::new(Kind::Class, name, Body::Holds));
        }
        if let Some(after) = word(rest, "interface") {
            return typed(after, Kind::Interface, Body::Holds);
        }
        if let Some(after) = word(strip_words(rest, &["const"]), "enum") {
            return typed(after, Kind::Enum, Body::Opaque);
        }
        if let Some(after) = word(rest, "type")
            && let Some((name, tail)) = ident(after)
            && tail.trim_start().starts_with(['=', '<'])
        {
            return Found::Def(Def::new(Kind::Type, name, Body::Opaque));
        }
        for keyword in ["namespace", "module"] {
            if let Some(after) = word(rest, keyword) {
                let name: String = after
                    .chars()
                    .take_while(|c| is_ident(*c) || *c == '.' || *c == '"' || *c == '\'' || *c == ' ')
                    .collect::<String>()
                    .trim()
                    .to_owned();
                // `module.exports` is not a module declaration.
                if name.is_empty() || after.starts_with('.') {
                    continue;
                }
                return Found::Def(Def::new(Kind::Module, name, Body::Holds));
            }
        }
        for keyword in ["const", "let", "var"] {
            if let Some(after) = word(rest, keyword) {
                let Some((name, tail)) = ident(after) else {
                    return Found::Nothing;
                };
                let tail = tail.trim_start();
                // A type annotation: `const f: Handler = (…) => …`.
                let value = tail.split_once('=').map(|(_, value)| value.trim_start());
                let Some(value) = value else {
                    return Found::Nothing;
                };
                if value.starts_with('>') {
                    return Found::Nothing;
                }
                if Self::is_function(value) {
                    return Found::Def(Def::new(Kind::Function, name, Body::Opaque));
                }
                if exported {
                    return Found::Def(Def::new(Kind::Const, name, Body::Opaque));
                }
                return Found::Nothing;
            }
        }
        // `exports.name = function`, `module.exports.name = (…) =>`.
        let assigned = rest
            .strip_prefix("module.exports.")
            .or_else(|| rest.strip_prefix("exports."));
        if let Some(after) = assigned
            && let Some((name, tail)) = ident(after)
            && let Some(value) = tail.trim_start().strip_prefix('=')
            && Self::is_function(value.trim_start())
        {
            return Found::Def(Def::new(Kind::Function, name, Body::Opaque));
        }
        Found::Nothing
    }
}

impl Js {
    /// A member of a class or an interface.
    fn member(code: &str, scope: &Scope<'_>) -> Found {
        let rest = strip_words(code, JS_MEMBER_MODIFIERS);
        let rest = rest.trim_start_matches('*').trim_start();
        let (name, tail) = match rest.strip_prefix('#') {
            Some(private) => match ident(private) {
                Some((name, tail)) => (format!("#{name}"), tail),
                None => return Found::Nothing,
            },
            None => match ident(rest) {
                Some((name, tail)) => (name.to_owned(), tail),
                None => return Found::Nothing,
            },
        };
        if STATEMENTS.contains(&name.as_str()) {
            return Found::Nothing;
        }
        let tail = tail.trim_start().trim_start_matches(['?', '!']).trim_start();
        let interface = matches!(scope.container, Some((Kind::Interface, _)));
        if tail.starts_with('(') || tail.starts_with('<') {
            return Found::Def(Def::new(Kind::Method, name, Body::Opaque));
        }
        if !interface
            && let Some(value) = tail.strip_prefix('=')
            && Self::is_function(value.trim_start())
        {
            return Found::Def(Def::new(Kind::Method, name, Body::Opaque));
        }
        Found::Nothing
    }
}

// ----------------------------------------------------------------- PHP

struct Php;

impl Recogniser for Php {
    fn syntax(&self) -> Syntax {
        Syntax::PHP
    }

    fn docs(&self) -> DocStyle {
        DocStyle::JAVADOC
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        if code.starts_with("#[") {
            return Found::Attribute;
        }
        let code = code.strip_prefix("<?php").map_or(code, str::trim_start);
        if let Some(after) = word(code, "namespace") {
            let name: String = after
                .chars()
                .take_while(|c| is_ident(*c) || *c == '\\')
                .collect();
            if name.is_empty() {
                return Found::Nothing;
            }
            return Found::Def(Def::new(Kind::Module, name, Body::Holds));
        }
        let rest = strip_words(
            code,
            &[
                "abstract",
                "final",
                "readonly",
                "public",
                "protected",
                "private",
                "static",
            ],
        );
        for (keyword, kind) in [
            ("class", Kind::Class),
            ("interface", Kind::Interface),
            ("trait", Kind::Interface),
            ("enum", Kind::Enum),
        ] {
            if let Some(after) = word(rest, keyword) {
                return typed(after, kind, Body::Holds);
            }
        }
        if let Some(after) = word(rest, "function") {
            let after = after.trim_start_matches('&').trim_start();
            return match named(after) {
                Some(name) => Found::Def(Def::new(function_or_method(scope), name, Body::Opaque)),
                None => Found::Nothing,
            };
        }
        if let Some(after) = word(rest, "const") {
            return typed(after, Kind::Const, Body::Opaque);
        }
        Found::Nothing
    }
}

// --------------------------------------------------------------- Shell

struct Shell;

impl Recogniser for Shell {
    fn syntax(&self) -> Syntax {
        Syntax::SHELL
    }

    fn docs(&self) -> DocStyle {
        DocStyle {
            lines: &["#"],
            blocks: &[],
        }
    }

    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found {
        if scope.container.is_some() {
            return Found::Nothing;
        }
        let shell_name = |text: &str| -> Option<(String, String)> {
            let end = text
                .char_indices()
                .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')))
                .map_or(text.len(), |(at, _)| at);
            (end > 0).then(|| (text[..end].to_owned(), text[end..].trim_start().to_owned()))
        };
        if let Some(after) = word(code, "function") {
            return match shell_name(after) {
                Some((name, _)) => Found::Def(Def::new(Kind::Function, name, Body::Opaque)),
                None => Found::Nothing,
            };
        }
        if let Some((name, rest)) = shell_name(code)
            && let Some(after) = rest.strip_prefix('(')
            && after.trim_start().starts_with(')')
        {
            return Found::Def(Def::new(Kind::Function, name, Body::Opaque));
        }
        Found::Nothing
    }
}
