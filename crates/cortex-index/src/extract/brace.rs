//! The engine of the languages whose blocks are braces.
//!
//! It reads each line's code (from the lexer), asks the language's
//! recogniser whether a definition begins there, and counts braces:
//!
//! - **A definition's body** is the first `{` after it outside brackets. A
//!   `;` there first, or a `}` that closes the block it is in, means it has
//!   none (a prototype, a trait method, `struct Unit;`).
//! - **What a body holds** is the recogniser's answer: a type, an `impl`,
//!   an interface or a module holds definitions, one level deeper. A
//!   function's body holds nothing anyone outlines, so nothing in it is
//!   looked at.
//! - **Any other `{`** (an object literal, an initialiser, a statement's
//!   block) is opaque in the same way.
//! - **A signature over several lines** is read until its brackets close,
//!   and its head is those lines joined.

use super::docs::{DocStyle, Docs};
use super::{Builder, line_number};
use crate::lex::{Lexer, Syntax};
use crate::outline::{Entry, HEAD_CHARS, Kind, clip, squeeze};

/// What a definition's body holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Body {
    /// Definitions, one level deeper.
    Holds,
    /// Nothing that is outlined: a function's statements, a struct's
    /// fields.
    Opaque,
}

/// A definition a recogniser found at the start of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Def {
    pub(crate) kind: Kind,
    pub(crate) name: String,
    pub(crate) body: Body,
    /// The definition counts only if a body follows: a C function's
    /// prototype in a source file is not outlined, its definition is.
    pub(crate) needs_body: bool,
}

impl Def {
    pub(crate) fn new(kind: Kind, name: impl Into<String>, body: Body) -> Self {
        Self {
            kind,
            name: name.into(),
            body,
            needs_body: false,
        }
    }

    pub(crate) fn needing_body(mut self) -> Self {
        self.needs_body = true;
        self
    }
}

/// What a recogniser says of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Found {
    /// Nothing is defined here.
    Nothing,
    /// A definition begins here.
    Def(Def),
    /// A block whose contents are at the level outside it: `extern "C" {`,
    /// an anonymous namespace.
    Transparent,
    /// A line that does not part a doc comment from what follows it: an
    /// attribute, an annotation, a decorator, a `template <…>`.
    Attribute,
}

/// Where a line is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Scope<'a> {
    /// The innermost definition whose body holds this line, if any: its
    /// kind and its name.
    pub(crate) container: Option<(Kind, &'a str)>,
    /// The code of the last line before this one that had code on it.
    pub(crate) previous: &'a str,
}

/// A language for this engine.
pub(crate) trait Recogniser {
    fn syntax(&self) -> Syntax;
    fn docs(&self) -> DocStyle;
    /// What begins on this line. `code` is its code, trimmed.
    fn recognise(&mut self, code: &str, scope: &Scope<'_>) -> Found;
    /// A body always opens on the line where the signature ends (Go, and
    /// by its formatters' convention Kotlin, Swift and JavaScript): a
    /// definition with no `{` there has none, and nothing after it is
    /// taken for its body.
    fn same_line_body(&self) -> bool {
        false
    }
}

/// An open brace and what it opened.
#[derive(Debug, Clone, Copy)]
enum Frame {
    /// A definition's body that holds others.
    Holds {
        entry: Option<usize>,
    },
    /// A body or a block whose contents are not outlined. `entry` is set
    /// for a definition that may take its name when it closes (C's
    /// `typedef struct { … } Name;`).
    Opaque {
        entry: Option<usize>,
    },
    Transparent,
}

/// A definition whose body has not begun yet.
#[derive(Debug)]
struct Pending {
    entry: Option<usize>,
    body: Body,
    needs_body: bool,
    transparent: bool,
    /// The bracket depth (`(` and `[`) inside the signature so far.
    brackets: i32,
    /// Lines read since the definition began.
    lines: usize,
    /// Its head so far.
    head: String,
    /// Where in `head` the current line's part begins, when the current
    /// line is part of the head.
    line_part: Option<usize>,
}

/// How many lines a signature may take before its body: past that, the
/// definition is taken to have none.
const SIGNATURE_LINES: usize = 12;

/// How many lines of a signature are joined into its head.
const HEAD_LINES: usize = 4;

/// The state of one file's reading.
struct Reader<'o> {
    out: &'o mut Builder,
    stack: Vec<Frame>,
    opaque: usize,
    /// The entries of the `Holds` frames on the stack, innermost last:
    /// kept as frames are pushed and popped, so that a file nested a
    /// hundred thousand deep costs no more a line than a flat one.
    holds: Vec<Option<usize>>,
    pending: Option<Pending>,
}

impl Reader<'_> {
    fn holds_depth(&self) -> usize {
        self.holds.len()
    }

    fn push(&mut self, frame: Frame) {
        match frame {
            Frame::Opaque { .. } => self.opaque += 1,
            Frame::Holds { entry } => self.holds.push(entry),
            Frame::Transparent => {}
        }
        self.stack.push(frame);
    }

    fn pop(&mut self) -> Option<Frame> {
        let frame = self.stack.pop()?;
        match frame {
            Frame::Opaque { .. } => self.opaque = self.opaque.saturating_sub(1),
            Frame::Holds { .. } => {
                self.holds.pop();
            }
            Frame::Transparent => {}
        }
        Some(frame)
    }

    /// Ends the definition waiting for its body: it has none.
    fn without_body(&mut self) {
        if let Some(done) = self.pending.take() {
            self.set_head(&done);
            if done.needs_body
                && let Some(entry) = done.entry
            {
                self.out.drop_last(entry);
            }
        }
    }

    fn set_head(&mut self, done: &Pending) {
        if let Some(entry) = done.entry.and_then(|entry| self.out.get_mut(entry)) {
            // A signature joined from several lines reads as one.
            let head = tidy(&done.head);
            let head = head.trim_end_matches([' ', '{', ';']);
            entry.head = clip(head.strip_suffix(" =").unwrap_or(head), HEAD_CHARS);
        }
    }

    /// Cuts the head where its body or its end begins, at `at` of the
    /// current line, which begins at `start`.
    fn cut_head(pending: &mut Pending, start: usize, at: usize) {
        if let Some(part) = pending.line_part {
            pending.head.truncate(part + at.saturating_sub(start));
        }
    }
}

/// A head joined from several lines, read as one: `f( a, b, )` is
/// `f(a, b)`.
pub(crate) fn tidy(head: &str) -> String {
    squeeze(head)
        .replace("( ", "(")
        .replace(", )", ")")
        .replace(" )", ")")
}

pub(crate) fn run(mut language: Box<dyn Recogniser>, text: &str, out: &mut Builder) {
    let mut lexer = Lexer::new(language.syntax());
    let style = language.docs();
    let mut docs = Docs::default();
    let mut previous = String::new();
    let mut reader = Reader {
        out,
        stack: Vec::new(),
        opaque: 0,
        holds: Vec::new(),
        pending: None,
    };

    for (index, raw) in text.lines().enumerate() {
        let lexed = lexer.line(raw);
        let code_line = lexed.code.as_str();
        let code = code_line.trim();
        if code.is_empty() {
            if !lexed.in_string {
                docs.comment_line(raw, style);
            }
            continue;
        }
        let start = code_line.len() - code_line.trim_start().len();
        let end = start + code.len();
        let part = raw.get(start..end).unwrap_or(code);

        // A signature still open: this line goes on with it.
        let continuing = reader
            .pending
            .as_ref()
            .is_some_and(|pending| pending.brackets > 0);
        if let Some(pending) = &mut reader.pending {
            pending.line_part = None;
            if continuing && pending.lines < HEAD_LINES {
                pending.line_part = Some(pending.head.len() + 1);
                pending.head.push(' ');
                pending.head.push_str(part);
            }
        }
        if !continuing && reader.opaque == 0 {
            let container = reader.holds.last().copied().map(|entry| {
                entry
                    .and_then(|entry| reader.out.entries.get(entry))
                    .map_or((Kind::Block, ""), |entry| (entry.kind, entry.name.as_str()))
            });
            let scope = Scope {
                container,
                previous: &previous,
            };
            match language.recognise(code, &scope) {
                Found::Def(def) => {
                    reader.without_body();
                    let depth = reader.holds_depth();
                    let entry = reader.out.push(Entry {
                        line: line_number(index),
                        depth: u8::try_from(depth).unwrap_or(u8::MAX),
                        kind: def.kind,
                        name: def.name,
                        head: String::new(),
                        doc: docs.take(),
                    });
                    reader.pending = Some(Pending {
                        entry,
                        body: def.body,
                        needs_body: def.needs_body,
                        transparent: false,
                        brackets: 0,
                        lines: 0,
                        head: part.to_owned(),
                        line_part: Some(0),
                    });
                }
                Found::Transparent => {
                    reader.without_body();
                    docs.clear();
                    reader.pending = Some(Pending {
                        entry: None,
                        body: Body::Holds,
                        needs_body: false,
                        transparent: true,
                        brackets: 0,
                        lines: 0,
                        head: String::new(),
                        line_part: None,
                    });
                }
                Found::Attribute => {}
                Found::Nothing => docs.clear(),
            }
        } else {
            docs.clear();
        }

        // Braces, brackets and the end of a declaration.
        for (at, byte) in code_line.bytes().enumerate().skip(start) {
            match byte {
                b'(' | b'[' => {
                    if let Some(pending) = &mut reader.pending {
                        pending.brackets += 1;
                    }
                }
                b')' | b']' => {
                    if let Some(pending) = &mut reader.pending {
                        pending.brackets -= 1;
                    }
                }
                b';' if reader
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.brackets <= 0) =>
                {
                    if let Some(pending) = &mut reader.pending {
                        Reader::cut_head(pending, start, at);
                    }
                    reader.without_body();
                }
                b'{' => {
                    let opened = reader.pending.take_if(|pending| pending.brackets <= 0);
                    let frame = match opened {
                        Some(mut done) => {
                            Reader::cut_head(&mut done, start, at);
                            reader.set_head(&done);
                            match done.body {
                                _ if done.transparent => Frame::Transparent,
                                Body::Holds => Frame::Holds { entry: done.entry },
                                Body::Opaque => Frame::Opaque { entry: done.entry },
                            }
                        }
                        None => Frame::Opaque { entry: None },
                    };
                    reader.push(frame);
                }
                b'}' => {
                    // A definition still waiting for a body inside the block
                    // that closes has none.
                    if reader
                        .pending
                        .as_ref()
                        .is_some_and(|pending| pending.brackets <= 0)
                    {
                        reader.without_body();
                    }
                    let Some(frame) = reader.pop() else {
                        continue;
                    };
                    let entry = match frame {
                        Frame::Opaque { entry } | Frame::Holds { entry } => entry,
                        Frame::Transparent => None,
                    };
                    // `typedef struct { … } Name;`: the name comes last.
                    if let Some(entry) = entry.and_then(|entry| reader.out.get_mut(entry))
                        && entry.name.is_empty()
                    {
                        let name: String = code_line[at + 1..]
                            .trim_start()
                            .trim_start_matches('*')
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '_')
                            .collect();
                        if !name.is_empty() {
                            entry.head = clip(&format!("{} {name}", entry.head), HEAD_CHARS);
                            entry.name = name;
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(open) = &mut reader.pending {
            open.lines += 1;
            let ended_here = open.brackets <= 0 && language.same_line_body();
            if ended_here || open.lines > SIGNATURE_LINES {
                reader.without_body();
            }
        }
        previous.clear();
        previous.push_str(code);
    }
    reader.without_body();
}
