//! The engine of the languages whose blocks are their indentation: Python
//! and Ruby, and the fallback for every file no extractor reads.

use super::docs::{DocStyle, Docs};
use super::{Builder, indent_of, line_number};
use crate::lex::{Lexer, Syntax};
use crate::outline::{DOC_CHARS, Entry, HEAD_CHARS, Kind, clip, squeeze};

/// An open block: its indentation, and what it does with what is in it.
#[derive(Debug, Clone, Copy)]
struct Open {
    indent: usize,
    kind: Kind,
    /// Nothing in it is outlined: a function's body.
    opaque: bool,
    /// It holds definitions at its own level: Ruby's `class << self`.
    transparent: bool,
}

/// The blocks open at a line, and the depth of what begins there.
#[derive(Debug, Default)]
struct Blocks {
    open: Vec<Open>,
}

impl Blocks {
    fn close_to(&mut self, indent: usize) {
        while self.open.last().is_some_and(|open| open.indent >= indent) {
            self.open.pop();
        }
    }

    fn opaque(&self) -> bool {
        self.open.iter().any(|open| open.opaque)
    }

    fn depth(&self) -> u8 {
        let depth = self.open.iter().filter(|open| !open.transparent).count();
        u8::try_from(depth).unwrap_or(u8::MAX)
    }

    fn in_type(&self) -> bool {
        self.open
            .iter()
            .rev()
            .find(|open| !open.opaque)
            .is_some_and(|open| matches!(open.kind, Kind::Class | Kind::Module))
    }
}

fn ident(text: &str) -> Option<(&str, &str)> {
    let first = text.chars().next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    let end = text
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
        .map_or(text.len(), |(at, _)| at);
    Some((&text[..end], &text[end..]))
}

fn word<'a>(text: &'a str, word: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(word)?;
    if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some(rest.trim_start())
}

fn is_constant(name: &str) -> bool {
    name.len() > 1
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && name.starts_with(|c: char| c.is_ascii_uppercase())
}

/// The net count of open brackets in a line of code.
fn brackets(code: &str) -> i32 {
    code.bytes().fold(0, |depth, byte| match byte {
        b'(' | b'[' | b'{' => depth + 1,
        b')' | b']' | b'}' => depth - 1,
        _ => depth,
    })
}

/// The text inside a docstring's opening line: `"""Parses it."""`.
fn docstring(raw: &str) -> Option<Option<String>> {
    let line = raw.trim_start();
    let line = line.trim_start_matches(['r', 'R', 'u', 'U', 'b', 'B', 'f', 'F']);
    let quote = ["\"\"\"", "'''", "\"", "'"]
        .into_iter()
        .find(|quote| line.starts_with(quote))?;
    let inside = &line[quote.len()..];
    let inside = inside.split(quote).next().unwrap_or(inside).trim();
    Some((!inside.is_empty()).then(|| clip(inside, DOC_CHARS)))
}

pub(crate) fn python(text: &str, out: &mut Builder) {
    let mut lexer = Lexer::new(Syntax::PYTHON);
    let mut docs = Docs::default();
    let style = DocStyle {
        lines: &["#"],
        blocks: &[],
    };
    let mut blocks = Blocks::default();
    let mut depth = 0i32;
    // The definition whose header is still open (its brackets), and the
    // one whose docstring may come next.
    let mut header: Option<(usize, String)> = None;
    let mut wants_doc: Option<usize> = None;
    // A docstring that began on the line after its definition and whose
    // text starts on the line after that.
    let mut doc_next_line: Option<usize> = None;

    for (index, raw) in text.lines().enumerate() {
        let lexed = lexer.line(raw);
        let code = lexed.code.trim();
        if let Some(entry) = doc_next_line.take()
            && let Some(entry) = out.get_mut(entry)
            && entry.doc.is_none()
        {
            let line = raw.trim().trim_end_matches(['"', '\'']).trim();
            if !line.is_empty() {
                entry.doc = Some(clip(line, DOC_CHARS));
            }
        }
        if code.is_empty() {
            if !lexed.in_string {
                docs.comment_line(raw, style);
            }
            continue;
        }
        let continuing = depth > 0;
        depth = (depth + brackets(code)).max(0);
        if continuing {
            if let Some((entry, head)) = &mut header {
                head.push(' ');
                head.push_str(raw.trim());
                if depth == 0 {
                    finish_head(out, *entry, head);
                    wants_doc = Some(*entry);
                    header = None;
                }
            }
            continue;
        }
        if let Some(entry) = wants_doc.take()
            && let Some(found) = docstring(raw)
        {
            match found {
                Some(text) => {
                    if let Some(entry) = out.get_mut(entry) {
                        entry.doc = Some(text);
                    }
                }
                None => doc_next_line = Some(entry),
            }
            continue;
        }
        let indent = indent_of(raw);
        blocks.close_to(indent);
        if code.starts_with('@') {
            continue;
        }
        let rest = word(code, "async").unwrap_or(code);
        let found = if blocks.opaque() {
            None
        } else if let Some(after) = word(rest, "def") {
            ident(after).map(|(name, _)| {
                let kind = if blocks.in_type() {
                    Kind::Method
                } else {
                    Kind::Function
                };
                (kind, name.to_owned(), true)
            })
        } else if let Some(after) = word(rest, "class") {
            ident(after).map(|(name, _)| (Kind::Class, name.to_owned(), false))
        } else if blocks.open.is_empty()
            && let Some((name, tail)) = ident(code)
            && is_constant(name)
            && tail.trim_start().starts_with(['=', ':'])
            && !tail.trim_start().starts_with("==")
        {
            Some((Kind::Const, name.to_owned(), false))
        } else {
            None
        };
        let Some((kind, name, opaque)) = found else {
            docs.clear();
            continue;
        };
        let entry = out.push(Entry {
            line: line_number(index),
            depth: blocks.depth(),
            kind,
            name,
            head: String::new(),
            doc: docs.take(),
        });
        let Some(entry) = entry else { continue };
        if kind == Kind::Const {
            finish_head(out, entry, raw.trim());
            continue;
        }
        blocks.open.push(Open {
            indent,
            kind,
            opaque,
            transparent: false,
        });
        if depth > 0 {
            header = Some((entry, raw.trim().to_owned()));
        } else {
            finish_head(out, entry, raw.trim());
            wants_doc = Some(entry);
        }
    }
    if let Some((entry, head)) = header {
        finish_head(out, entry, &head);
    }
}

/// Sets an indentation language's head: the header without its `:` and
/// without a comment after it.
fn finish_head(out: &mut Builder, entry: usize, head: &str) {
    if let Some(entry) = out.get_mut(entry) {
        let head = super::brace::tidy(head);
        let head = head.split(" #").next().unwrap_or(&head);
        let head = head.strip_suffix(':').unwrap_or(head).trim_end();
        entry.head = clip(head, HEAD_CHARS);
    }
}

pub(crate) fn ruby(text: &str, out: &mut Builder) {
    let mut lexer = Lexer::new(Syntax::RUBY);
    let mut docs = Docs::default();
    let style = DocStyle {
        lines: &["#"],
        blocks: &[],
    };
    let mut blocks = Blocks::default();
    for (index, raw) in text.lines().enumerate() {
        let lexed = lexer.line(raw);
        let code = lexed.code.trim();
        if code.is_empty() {
            if !lexed.in_string && !lexed.in_comment {
                docs.comment_line(raw, style);
            }
            continue;
        }
        let indent = indent_of(raw);
        blocks.close_to(indent);
        if code == "end" || code.starts_with("end ") || code.starts_with("end.") {
            continue;
        }
        let found = if blocks.opaque() {
            None
        } else if let Some(after) = word(code, "def") {
            // `def name`, `def self.name`, `def name=(value)`, `def ==(other)`.
            let name: String = after
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '(' && *c != ';')
                .collect();
            let kind = if blocks.in_type() {
                Kind::Method
            } else {
                Kind::Function
            };
            (!name.is_empty()).then_some((kind, name, true, false))
        } else if let Some(after) = word(code, "class") {
            if after.starts_with("<<") {
                // `class << self`: what is in it is the class's.
                blocks.open.push(Open {
                    indent,
                    kind: Kind::Class,
                    opaque: false,
                    transparent: true,
                });
                docs.clear();
                continue;
            }
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            (!name.is_empty()).then_some((Kind::Class, name, false, false))
        } else if let Some(after) = word(code, "module") {
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                .collect();
            (!name.is_empty()).then_some((Kind::Module, name, false, false))
        } else if let Some((name, tail)) = ident(code)
            && is_constant(name)
            && tail.trim_start().starts_with('=')
            && !tail.trim_start().starts_with("==")
        {
            Some((Kind::Const, name.to_owned(), false, true))
        } else {
            None
        };
        let Some((kind, name, opaque, single)) = found else {
            docs.clear();
            continue;
        };
        let head = squeeze(raw.trim());
        let head = head.strip_suffix("; end").unwrap_or(&head);
        let _ = out.push(Entry {
            line: line_number(index),
            depth: blocks.depth(),
            kind,
            name,
            head: clip(head, HEAD_CHARS),
            doc: docs.take(),
        });
        // A one-line definition (`def a; end`) opens no block.
        let one_line = code.ends_with("end") && code.contains(';');
        if !single && !one_line {
            blocks.open.push(Open {
                indent,
                kind,
                opaque,
                transparent: false,
            });
        }
    }
}

/// How deep the fallback goes: top-level blocks and what is directly in
/// them.
const FALLBACK_DEPTH: u8 = 2;

/// The outline of a file no extractor reads: each line with more indented
/// lines right under it is a block.
pub(crate) fn fallback(text: &str, out: &mut Builder) {
    let lines: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .collect();
    let mut blocks = Blocks::default();
    for (position, &(index, raw)) in lines.iter().enumerate() {
        let indent = indent_of(raw);
        blocks.close_to(indent);
        let Some(&(_, next)) = lines.get(position + 1) else {
            break;
        };
        if indent_of(next) <= indent {
            continue;
        }
        let line = raw.trim();
        let comment = ["#", "//", "--", ";", "*", "/*", "<!--", "%"]
            .iter()
            .any(|marker| line.starts_with(marker));
        let depth = blocks.depth();
        blocks.open.push(Open {
            indent,
            kind: Kind::Block,
            opaque: false,
            transparent: comment,
        });
        if comment || depth >= FALLBACK_DEPTH {
            continue;
        }
        let head = clip(&squeeze(line), 80);
        let _ = out.push(Entry {
            line: line_number(index),
            depth,
            kind: Kind::Block,
            name: head.clone(),
            head,
            doc: None,
        });
    }
}
