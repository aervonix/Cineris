//! The one lexer every extractor reads lines through: it blanks out
//! comments and the inside of strings, so that a brace or a keyword in
//! them is not taken for code.
//!
//! Each line comes back as its **code**: the line with every byte of a
//! comment, and of a string's content, replaced by a space. The quotes of a
//! string stay. So the code has the line's length byte for byte, and a
//! position found in it is the same position in the line. A string or a
//! comment that goes on to the next line is carried over in the lexer's
//! state.
//!
//! It is approximate in the ways the extractors are: no escapes inside a
//! template literal's `${…}`, no C++ raw strings, a heredoc's tag must
//! look like one, and a JavaScript `/` is a regular expression's only
//! where an expression begins and when it closes on its line.
//!
//! It is linear: each byte is looked at a bounded number of times, and no
//! scan is begun again from every position of a line.

use crate::outline::NAME_CHARS;

/// A kind of quote, and how a string it opens behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Quote {
    /// The quote character.
    pub(crate) mark: u8,
    /// Whether a backslash escapes the next character.
    pub(crate) escapes: bool,
    /// Whether the string may go on past the end of its line. One that may
    /// not ends there, so that a stray apostrophe cannot swallow a file.
    pub(crate) multiline: bool,
}

const fn quote(mark: u8, escapes: bool, multiline: bool) -> Quote {
    Quote {
        mark,
        escapes,
        multiline,
    }
}

/// Where a heredoc's body begins and how its end is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Heredocs {
    None,
    /// `<<TAG`, `<<-TAG`, `<<'TAG'`.
    Shell,
    /// `<<~TAG`, `<<-TAG`, `<<TAG` with a capital or a quote.
    Ruby,
    /// `<<<TAG`, `<<<'TAG'`, `<<<"TAG"`.
    Php,
}

/// How a language writes comments and strings.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Syntax {
    pub(crate) line_comments: &'static [&'static str],
    /// A `#` begins a comment only at the start of a word, as in a shell,
    /// where `${#name}` and `a#b` are not comments.
    pub(crate) hash_at_word: bool,
    /// `#[` is an attribute, not a comment (PHP 8).
    pub(crate) hash_attribute: bool,
    pub(crate) block: Option<(&'static str, &'static str)>,
    /// Block comments nest (Rust, Swift, Kotlin).
    pub(crate) nested_block: bool,
    pub(crate) quotes: &'static [Quote],
    /// The quote characters that, three in a row, open a string that ends
    /// at the next three.
    pub(crate) triple: &'static [u8],
    /// Rust's raw strings (`r#"…"#`) and its `'`, which is a character or
    /// a lifetime.
    pub(crate) rust: bool,
    /// C#'s verbatim strings (`@"…"`), in which a backslash is a
    /// backslash.
    pub(crate) verbatim_at: bool,
    pub(crate) heredocs: Heredocs,
    /// Ruby's `=begin` … `=end`.
    pub(crate) ruby_docs: bool,
    /// JavaScript's regular expression literals (`/a\/b/g`), whose content
    /// is blanked as a string's is.
    pub(crate) regex: bool,
}

impl Syntax {
    const BASE: Self = Self {
        line_comments: &[],
        hash_at_word: false,
        hash_attribute: false,
        block: None,
        nested_block: false,
        quotes: &[],
        triple: &[],
        rust: false,
        verbatim_at: false,
        heredocs: Heredocs::None,
        ruby_docs: false,
        regex: false,
    };

    /// C, C++, Java, C#, JavaScript, TypeScript and Go share most of this.
    const C_LIKE: Self = Self {
        line_comments: &["//"],
        block: Some(("/*", "*/")),
        quotes: &[quote(b'"', true, false), quote(b'\'', true, false)],
        ..Self::BASE
    };

    pub(crate) const RUST: Self = Self {
        quotes: &[quote(b'"', true, true)],
        nested_block: true,
        rust: true,
        ..Self::C_LIKE
    };
    pub(crate) const C: Self = Self::C_LIKE;
    pub(crate) const JAVA: Self = Self {
        triple: b"\"",
        ..Self::C_LIKE
    };
    pub(crate) const CSHARP: Self = Self {
        triple: b"\"",
        verbatim_at: true,
        ..Self::C_LIKE
    };
    pub(crate) const JS: Self = Self {
        quotes: &[
            quote(b'"', true, false),
            quote(b'\'', true, false),
            quote(b'`', true, true),
        ],
        regex: true,
        ..Self::C_LIKE
    };
    pub(crate) const GO: Self = Self {
        quotes: &[
            quote(b'"', true, false),
            quote(b'\'', true, false),
            quote(b'`', false, true),
        ],
        ..Self::C_LIKE
    };
    /// A name in backticks (``fun `doesn't crash`()``) may hold any
    /// character but a backtick and a line break: its content is blanked
    /// as a string's, so a quote in it opens nothing, and the recogniser
    /// reads the name from the line as written.
    pub(crate) const KOTLIN: Self = Self {
        quotes: &[
            quote(b'"', true, false),
            quote(b'\'', true, false),
            quote(b'`', false, false),
        ],
        triple: b"\"",
        nested_block: true,
        ..Self::C_LIKE
    };
    pub(crate) const SWIFT: Self = Self {
        triple: b"\"",
        nested_block: true,
        ..Self::C_LIKE
    };
    pub(crate) const PHP: Self = Self {
        line_comments: &["//", "#"],
        hash_attribute: true,
        quotes: &[quote(b'"', true, true), quote(b'\'', true, true)],
        heredocs: Heredocs::Php,
        ..Self::C_LIKE
    };
    pub(crate) const PYTHON: Self = Self {
        line_comments: &["#"],
        quotes: &[quote(b'"', true, false), quote(b'\'', true, false)],
        triple: b"\"'",
        ..Self::BASE
    };
    pub(crate) const RUBY: Self = Self {
        line_comments: &["#"],
        quotes: &[quote(b'"', true, true), quote(b'\'', true, true)],
        heredocs: Heredocs::Ruby,
        ruby_docs: true,
        ..Self::BASE
    };
    pub(crate) const SHELL: Self = Self {
        line_comments: &["#"],
        hash_at_word: true,
        quotes: &[
            quote(b'"', true, true),
            quote(b'\'', false, true),
            quote(b'`', true, true),
        ],
        heredocs: Heredocs::Shell,
        ..Self::BASE
    };
}

/// What the lexer was in the middle of when a line began.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Code,
    Block(u32),
    Str(Quote),
    Triple(u8),
    RawRust(usize),
    Verbatim,
    Heredoc { tag: String, indented: bool },
    RubyDoc,
}

/// One line, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lexed {
    /// The line with comments and string contents blanked.
    pub(crate) code: String,
    /// The line began inside a comment.
    pub(crate) in_comment: bool,
    /// The line began inside a string or a heredoc.
    pub(crate) in_string: bool,
}

/// Reads a file's lines in order.
#[derive(Debug)]
pub(crate) struct Lexer {
    syntax: Syntax,
    state: State,
    /// A heredoc opened on the line being read: its body starts on the next.
    next_heredoc: Option<(String, bool)>,
    /// A `/` on the line being read looked like a regular expression's
    /// start and did not close: no later `/` of the line is tried, so a
    /// line of them is not scanned to its end from each.
    no_regex: bool,
    /// A shell's arithmetic on the line being read: the `((` and the `))`
    /// counted so far, and up to where. Each pair is counted once, as the
    /// reading passes it.
    arithmetic: Arithmetic,
}

/// A shell's `((` and `))` on a line, counted up to a point of it.
#[derive(Debug, Default)]
struct Arithmetic {
    counted: usize,
    opened: usize,
    closed: usize,
}

impl Arithmetic {
    /// Whether the code of the line before `at` has opened an arithmetic
    /// expression (`((` or `$((`) and not closed it. `code[..at]` is read
    /// for the first time from where the last call stopped.
    fn open_at(&mut self, code: &[u8], at: usize) -> bool {
        for end in self.counted.max(1)..at {
            match &code[end - 1..=end] {
                b"((" => self.opened += 1,
                b"))" => self.closed += 1,
                _ => {}
            }
        }
        self.counted = self.counted.max(at);
        self.opened > self.closed
    }
}

impl Lexer {
    pub(crate) fn new(syntax: Syntax) -> Self {
        Self {
            syntax,
            state: State::Code,
            next_heredoc: None,
            no_regex: false,
            arithmetic: Arithmetic::default(),
        }
    }

    /// Reads the next line of the file.
    pub(crate) fn line(&mut self, line: &str) -> Lexed {
        let in_comment = matches!(self.state, State::Block(_) | State::RubyDoc);
        let in_string = matches!(
            self.state,
            State::Str(_)
                | State::Triple(_)
                | State::RawRust(_)
                | State::Verbatim
                | State::Heredoc { .. }
        );
        let blanked = |code: Vec<u8>| Lexed {
            code: to_text(code),
            in_comment,
            in_string,
        };
        let bytes = line.as_bytes();
        let mut code = vec![b' '; bytes.len()];
        self.no_regex = false;
        self.arithmetic = Arithmetic::default();

        // Whole-line states first.
        match &self.state {
            State::Heredoc { tag, indented } => {
                let body = if *indented { line.trim_start() } else { line };
                let ends = match self.syntax.heredocs {
                    Heredocs::Php => body
                        .strip_prefix(tag.as_str())
                        .is_some_and(|rest| !rest.starts_with(is_ident_char)),
                    _ => body.trim_end() == tag,
                };
                if ends {
                    self.state = State::Code;
                }
                return blanked(code);
            }
            State::RubyDoc => {
                if line.starts_with("=end") {
                    self.state = State::Code;
                }
                return blanked(code);
            }
            State::Code if self.syntax.ruby_docs && line.starts_with("=begin") => {
                self.state = State::RubyDoc;
                return blanked(code);
            }
            _ => {}
        }

        let mut at = 0;
        while at < bytes.len() {
            let rest = &bytes[at..];
            match self.state {
                State::Code => at = self.code_at(line, at, &mut code),
                State::Block(depth) => {
                    let (open, close) = self.syntax.block.unwrap_or(("/*", "*/"));
                    if rest.starts_with(close.as_bytes()) {
                        at += close.len();
                        self.state = if depth > 1 {
                            State::Block(depth - 1)
                        } else {
                            State::Code
                        };
                    } else if self.syntax.nested_block && rest.starts_with(open.as_bytes()) {
                        at += open.len();
                        self.state = State::Block(depth + 1);
                    } else {
                        at += 1;
                    }
                }
                State::Str(quote) => {
                    if quote.escapes && rest[0] == b'\\' {
                        at += 2;
                    } else if rest[0] == quote.mark {
                        code[at] = quote.mark;
                        at += 1;
                        self.state = State::Code;
                    } else {
                        at += 1;
                    }
                }
                State::Triple(mark) => {
                    if rest[0] == b'\\' {
                        at += 2;
                    } else if rest.starts_with(&[mark, mark, mark]) {
                        code[at..at + 3].fill(mark);
                        at += 3;
                        self.state = State::Code;
                    } else {
                        at += 1;
                    }
                }
                State::RawRust(hashes) => {
                    let closes = rest[0] == b'"'
                        && rest.len() > hashes
                        && rest[1..=hashes].iter().all(|&byte| byte == b'#');
                    if closes {
                        code[at] = b'"';
                        at += 1 + hashes;
                        self.state = State::Code;
                    } else {
                        at += 1;
                    }
                }
                State::Verbatim => {
                    if rest.starts_with(b"\"\"") {
                        at += 2;
                    } else if rest[0] == b'"' {
                        code[at] = b'"';
                        at += 1;
                        self.state = State::Code;
                    } else {
                        at += 1;
                    }
                }
                State::Heredoc { .. } | State::RubyDoc => break,
            }
        }
        // A string that may not go past its line ends with it.
        if let State::Str(quote) = self.state
            && !quote.multiline
        {
            self.state = State::Code;
        }
        if let Some((tag, indented)) = self.next_heredoc.take()
            && self.state == State::Code
        {
            self.state = State::Heredoc { tag, indented };
        }
        blanked(code)
    }

    /// Reads code from `at`, and returns where it stopped: at the end of
    /// the line, or just after where a comment or a string begins (the
    /// state then says which).
    fn code_at(&mut self, line: &str, mut at: usize, code: &mut [u8]) -> usize {
        let bytes = line.as_bytes();
        let syntax = self.syntax;
        while at < bytes.len() {
            let rest = &bytes[at..];
            let byte = bytes[at];
            if !byte.is_ascii() {
                code[at] = byte;
                at += 1;
                continue;
            }
            // Comments.
            if let Some((open, _)) = syntax.block
                && rest.starts_with(open.as_bytes())
            {
                self.state = State::Block(1);
                return at + open.len();
            }
            if let Some(marker) = syntax
                .line_comments
                .iter()
                .find(|marker| rest.starts_with(marker.as_bytes()))
            {
                let hash = *marker == "#";
                let at_word =
                    at == 0 || matches!(bytes[at - 1], b' ' | b'\t' | b';' | b'(' | b'&' | b'|');
                let attribute = hash && syntax.hash_attribute && rest.starts_with(b"#[");
                if !attribute && (!(hash && syntax.hash_at_word) || at_word) {
                    return bytes.len();
                }
            }
            // A regular expression: what is between its slashes is blanked,
            // so a `//`, a `/*`, a quote or a brace in it is not taken for
            // one. (A `/` followed by `/` or `*` is a comment, above.)
            if syntax.regex && byte == b'/' && !self.no_regex && regex_may_start(&code[..at]) {
                if let Some(end) = regex_end(bytes, at) {
                    code[at] = b'/';
                    code[end] = b'/';
                    at = end + 1;
                    continue;
                }
                // It does not close on its line: a division.
                self.no_regex = true;
            }
            // A shell's here-string, `<<<word`, is not a heredoc: its three
            // `<` are code together, so that its second is not read as a
            // heredoc's start.
            if syntax.heredocs == Heredocs::Shell && rest.starts_with(b"<<<") {
                code[at..at + 3].copy_from_slice(b"<<<");
                at += 3;
                continue;
            }
            // Heredocs. `<` is ASCII, so `at` is on a character's start. In
            // a shell's arithmetic, `$(( 1<<bits ))`, `<<` is a shift.
            if byte == b'<'
                && !(syntax.heredocs == Heredocs::Shell && self.arithmetic.open_at(code, at))
                && let Some((tag, indented, len)) = heredoc_at(syntax.heredocs, &line[at..])
            {
                code[at..at + len].copy_from_slice(&bytes[at..at + len]);
                self.next_heredoc = Some((tag, indented));
                at += len;
                continue;
            }
            // Strings.
            if syntax.rust {
                if let Some((hashes, len)) = rust_raw_at(bytes, at) {
                    code[at..at + len].copy_from_slice(&bytes[at..at + len]);
                    self.state = State::RawRust(hashes);
                    return at + len;
                }
                if byte == b'\'' {
                    code[at] = b'\'';
                    // A character is a string; a lifetime is code.
                    if let Some(len) = rust_char_len(rest) {
                        code[at + len - 1] = b'\'';
                        at += len;
                    } else {
                        at += 1;
                    }
                    continue;
                }
            }
            if syntax.verbatim_at && rest.starts_with(b"@\"") {
                code[at..at + 2].copy_from_slice(b"@\"");
                self.state = State::Verbatim;
                return at + 2;
            }
            if syntax.triple.contains(&byte) && rest.starts_with(&[byte, byte, byte]) {
                code[at..at + 3].fill(byte);
                self.state = State::Triple(byte);
                return at + 3;
            }
            if let Some(quote) = syntax.quotes.iter().find(|quote| quote.mark == byte) {
                code[at] = byte;
                self.state = State::Str(*quote);
                return at + 1;
            }
            code[at] = byte;
            at += 1;
        }
        at
    }
}

/// The code bytes as text. A character's bytes are kept whole or blanked
/// whole, so this is valid UTF-8; were it not, a byte that is not ASCII
/// would become a space, which keeps the length.
fn to_text(code: Vec<u8>) -> String {
    String::from_utf8(code).unwrap_or_else(|error| {
        error
            .into_bytes()
            .into_iter()
            .map(|byte| {
                if byte.is_ascii() {
                    char::from(byte)
                } else {
                    ' '
                }
            })
            .collect()
    })
}

/// Whether a character can be part of an identifier.
pub(crate) fn is_ident_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// The words after which a JavaScript expression begins, so that a `/`
/// after one is a regular expression's start: `return /a/.test(s)`.
const BEFORE_REGEX: &[&str] = &[
    "return",
    "typeof",
    "case",
    "do",
    "else",
    "in",
    "of",
    "new",
    "delete",
    "void",
    "throw",
    "instanceof",
    "yield",
    "await",
];

/// Whether a `/` after `before`, the code of its line so far, begins a
/// regular expression and not a division: at the line's start, after an
/// operator or an opening bracket, or after a word of [`BEFORE_REGEX`].
///
/// Not after `<`, `>` or `}`, though a regular expression may follow any of
/// them: in JSX, `</div>` closes a tag, `<input value={v} />` closes itself
/// and `<li>Home</li> / {x}` is text, and a regular expression read from
/// there to the next `/` of the line would blank whatever braces are
/// between (`/>{items.map(…)}</ul>`). A regular expression after one of
/// them is rare in code; a JSX tag is not. `=>` is an arrow, and is not
/// JSX. Not after `++` or `--`, which end an operand (`n++ / 2`), a `!`
/// right after a name or a closing bracket (TypeScript's `total! / n`), or
/// a word of [`BEFORE_REGEX`] after a `.`, which is a property (`x.in / 2`).
fn regex_may_start(before: &[u8]) -> bool {
    let Some(last) = before
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t'))
    else {
        return true;
    };
    let previous = last.checked_sub(1).map(|at| before[at]);
    match before[last] {
        b'(' | b',' | b'=' | b':' | b'[' | b'&' | b'|' | b'?' | b'{' | b';' | b'*' | b'%'
        | b'~' | b'^' => return true,
        b'+' | b'-' => return previous != Some(before[last]),
        b'!' => {
            return !previous.is_some_and(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b')' | b']')
            });
        }
        b'>' => return previous == Some(b'='),
        _ => {}
    }
    // The word that ends there, looked at no further back than the
    // longest of the words is long.
    let word_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$';
    let longest = BEFORE_REGEX
        .iter()
        .map(|word| word.len())
        .max()
        .unwrap_or(0);
    let mut start = last + 1;
    while start > 0 && last + 1 - start <= longest && word_byte(before[start - 1]) {
        start -= 1;
    }
    if start > 0 && (word_byte(before[start - 1]) || before[start - 1] == b'.') {
        return false;
    }
    let word = &before[start..=last];
    BEFORE_REGEX
        .iter()
        .any(|candidate| candidate.as_bytes() == word)
}

/// Where the regular expression whose opening `/` is at `open` closes, on
/// the same line: a backslash escapes the next byte, and a `/` inside a
/// class (`[/]`) does not close it.
fn regex_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut at = open + 1;
    let mut class = false;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 1,
            b'[' => class = true,
            b']' => class = false,
            b'/' if !class => return Some(at),
            _ => {}
        }
        at += 1;
    }
    None
}

/// A Rust raw string's opening (`r"`, `r#"`, `br##"`) at `at`: the number
/// of `#` and the opening's length.
fn rust_raw_at(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    if at > 0 && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_') {
        return None;
    }
    let mut index = at;
    if bytes.get(index) == Some(&b'b') {
        index += 1;
    }
    if bytes.get(index) != Some(&b'r') {
        return None;
    }
    index += 1;
    let hashes = bytes[index..].iter().take_while(|&&b| b == b'#').count();
    index += hashes;
    (bytes.get(index) == Some(&b'"')).then_some((hashes, index + 1 - at))
}

/// The length of a Rust character literal at the start of `bytes`
/// (`'a'`, `'\n'`, `'\''`, `'\u{1F600}'`, `'é'`), or `None` for a
/// lifetime.
fn rust_char_len(bytes: &[u8]) -> Option<usize> {
    if bytes.get(1) == Some(&b'\\') {
        // Looked for no further than the longest escape, `\u{10FFFF}`.
        let close = bytes.iter().skip(3).take(11).position(|&b| b == b'\'')?;
        return Some(close + 4);
    }
    // One character, of up to four bytes, then the quote.
    (2..=5)
        .find(|&end| {
            bytes.get(end) == Some(&b'\'')
                && std::str::from_utf8(&bytes[1..end]).is_ok_and(|text| text.chars().count() == 1)
        })
        .map(|end| end + 1)
}

/// A heredoc's opening at the start of `rest`: its tag, whether its end
/// may be indented, and the opening's length.
fn heredoc_at(kind: Heredocs, rest: &str) -> Option<(String, bool, usize)> {
    let (after, indented, open_len) = match kind {
        Heredocs::None => return None,
        Heredocs::Php => (rest.strip_prefix("<<<")?, true, 3),
        Heredocs::Shell => {
            let after = rest.strip_prefix("<<")?;
            if after.starts_with('<') {
                return None;
            }
            let (after, indented, open_len) = match after.strip_prefix('-') {
                Some(after) => (after, true, 3),
                None => (after, false, 2),
            };
            // `cat << EOF`: the tag may follow spaces.
            let tag = after.trim_start_matches([' ', '\t']);
            (tag, indented, open_len + after.len() - tag.len())
        }
        Heredocs::Ruby => {
            let after = rest.strip_prefix("<<")?;
            match after.strip_prefix(['~', '-']) {
                Some(after) => (after, true, 3),
                None => (after, false, 2),
            }
        }
    };
    let quoted = after.starts_with(['\'', '"']);
    let after_quote = if quoted { &after[1..] } else { after };
    let tag: String = after_quote
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(NAME_CHARS + 1)
        .collect();
    // A tag longer than a name is not one: the lexer would hold it for every
    // line after.
    if tag.len() > NAME_CHARS {
        return None;
    }
    let first = tag.chars().next()?;
    // A tag looks like one: a letter or `_` first, and in Ruby a capital
    // unless quoted, so that `a << b` and `1<<2` are not heredocs.
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if kind == Heredocs::Ruby && !quoted && !first.is_ascii_uppercase() {
        return None;
    }
    let len = open_len + tag.len() + if quoted { 2 } else { 0 };
    Some((tag, indented, len.min(rest.len())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(syntax: Syntax, text: &str) -> Vec<String> {
        let mut lexer = Lexer::new(syntax);
        text.lines().map(|line| lexer.line(line).code).collect()
    }

    #[test]
    fn comments_and_strings_are_blanked_and_keep_their_length() {
        let text = "fn a() { \"}\" } // }\n/* {\n} */ fn b() {}";
        let lines = lex(Syntax::RUST, text);
        for (line, code) in text.lines().zip(&lines) {
            assert_eq!(line.len(), code.len());
        }
        assert_eq!(lines[0].trim_end(), "fn a() { \" \" }");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[2].trim(), "fn b() {}");
    }

    #[test]
    fn rust_lifetimes_are_code_and_characters_are_strings() {
        let lines = lex(Syntax::RUST, "fn a<'a>(x: &'a str) -> char { '{' }");
        assert_eq!(lines[0], "fn a<'a>(x: &'a str) -> char { ' ' }");
        let lines = lex(Syntax::RUST, "let c = '\\''; fn b() {}");
        assert_eq!(lines[0], "let c = '  '; fn b() {}");
        let lines = lex(Syntax::RUST, "let s = r#\"a \" { \"#; fn b() {}");
        assert!(!lines[0].contains('{') || lines[0].ends_with("fn b() {}"));
        assert_eq!(lines[0].matches('{').count(), 1, "{}", lines[0]);
    }

    #[test]
    fn a_string_that_may_not_span_lines_ends_with_its_line() {
        let lines = lex(Syntax::C, "char *s = \"open\nint f() {}");
        assert_eq!(lines[1], "int f() {}");
        let lines = lex(Syntax::JS, "const t = `a\n{ b }\n`; function f() {}");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[2], "`; function f() {}");
    }

    #[test]
    fn triple_quotes_python_comments_and_nested_blocks() {
        let lines = lex(
            Syntax::PYTHON,
            "x = \"\"\"\ndef no():\n\"\"\"\ndef yes(): # c",
        );
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[3].trim_end(), "def yes():");
        let lines = lex(Syntax::KOTLIN, "/* a /* b */ c */ fun f() {}");
        assert_eq!(lines[0].trim(), "fun f() {}");
    }

    #[test]
    fn heredocs_are_skipped_to_their_tag() {
        let lines = lex(Syntax::SHELL, "cat <<EOF\nf() {\nEOF\ng() {");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[3], "g() {");
        // An arithmetic shift is not a heredoc, by a number or by a name.
        let lines = lex(Syntax::SHELL, "x=$((1<<2))\nf() {");
        assert_eq!(lines[1], "f() {");
        let lines = lex(Syntax::SHELL, "mask=$(( 1<<bits ))\nf() {\nbits\ng() {");
        assert_eq!(lines[1], "f() {");
        let lines = lex(Syntax::SHELL, "(( n <<= 2 ))\nf() {");
        assert_eq!(lines[1], "f() {");
        // A heredoc after an arithmetic expression that closed is one.
        let lines = lex(Syntax::SHELL, "echo $((1)) <<EOF\nf() {\nEOF\ng() {");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[3], "g() {");
        // A tag after spaces, as `cat << EOF` writes it.
        let lines = lex(Syntax::SHELL, "cat << EOF\nf() {\nEOF\ng() {");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[3], "g() {");
        let lines = lex(Syntax::SHELL, "cat <<- 'EOF'\nf() {\n\tEOF\ng() {");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[3], "g() {");
        // A tag longer than a name is not one.
        let long = format!("cat <<{}\nf() {{", "A".repeat(NAME_CHARS + 1));
        assert_eq!(lex(Syntax::SHELL, &long)[1], "f() {");
        let longest = format!("cat <<{}\nf() {{", "A".repeat(NAME_CHARS));
        assert_eq!(lex(Syntax::SHELL, &longest)[1].trim(), "");
        let lines = lex(Syntax::RUBY, "x = <<~SQL\n  def no\n  SQL\ndef yes");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[3], "def yes");
        let lines = lex(Syntax::RUBY, "list << item\ndef yes");
        assert_eq!(lines[1], "def yes");
        let lines = lex(
            Syntax::PHP,
            "$a = <<<EOT\nfunction no() {\nEOT;\nfunction yes() {",
        );
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[3], "function yes() {");
        let lines = lex(Syntax::RUBY, "=begin\ndef no\n=end\ndef yes");
        assert_eq!(lines[1].trim(), "");
        assert_eq!(lines[3], "def yes");
        // A shell's here-string is code, not a heredoc from its second `<`.
        let lines = lex(
            Syntax::SHELL,
            "base64 -d <<<SGVsbG8=\nf() {\nSGVsbG8\ng() {",
        );
        assert_eq!(lines[0], "base64 -d <<<SGVsbG8=");
        assert_eq!(lines[1], "f() {");
    }

    #[test]
    fn a_regular_expression_is_blanked_as_a_string_and_a_division_is_code() {
        let cases = [
            // What a regular expression holds is not a comment, a quote or
            // a brace; the slashes stay.
            ("x = /a{'b/g; // c", "x = /    /g;"),
            (
                "if (/^https?:\\/\\//.test(u)) {",
                "if (/            /.test(u)) {",
            ),
            (
                "return p.replace(/\\/*$/, \"\");",
                "return p.replace(/    /, \"\");",
            ),
            ("return /a/.test(s)", "return / /.test(s)"),
            ("x = typeof /a/", "x = typeof / /"),
            // A `/` inside a class does not close it.
            ("x = /[/{]/", "x = /    /"),
            // At the line's start.
            ("/a{/.exec(s)", "/  /.exec(s)"),
            // A division: after a name, a number, a closing bracket, a
            // string, or a word that is not a keyword.
            ("a = b / c / d;", "a = b / c / d;"),
            ("n = (a) / 2 / 1;", "n = (a) / 2 / 1;"),
            ("x = 'a' / 'b' /", "x = ' ' / ' ' /"),
            ("x = returned / 2 / 1", "x = returned / 2 / 1"),
            // One that does not close on its line is a division, and no
            // later `/` of the line is tried.
            ("x = /[ (/a/", "x = /[ (/a/"),
            // JSX: a closing tag and a tag that closes itself, and text
            // after a tag.
            ("<p>{a}</p>{b && <i/>}", "<p>{a}</p>{b && <i/>}"),
            (
                "<li>Home</li> / {open && <b>x</b>}",
                "<li>Home</li> / {open && <b>x</b>}",
            ),
            // After an arrow or a `!` that negates, a regular expression.
            ("f = (a) => /a{/.test(a)", "f = (a) => /  /.test(a)"),
            ("if (!/a{/.test(s)) {", "if (!/  /.test(s)) {"),
            ("x = a + /a{/", "x = a + /  /"),
            // An operand's end: `++`, `--`, TypeScript's `!`, a property
            // named as a keyword.
            ("y = n++ / 2 + {} / 1;", "y = n++ / 2 + {} / 1;"),
            ("y = i-- / 2 + {} / 1;", "y = i-- / 2 + {} / 1;"),
            ("y = total! / n + {} / 1;", "y = total! / n + {} / 1;"),
            ("y = f()! / n + {} / 1;", "y = f()! / n + {} / 1;"),
            ("y = x.in / 2 + {} / 1;", "y = x.in / 2 + {} / 1;"),
            (
                "<input value={v} />{list.map(f)}<br/>",
                "<input value={v} />{list.map(f)}<br/>",
            ),
        ];
        for (line, code) in cases {
            assert_eq!(lex(Syntax::JS, line)[0].trim_end(), code, "{line}");
        }
        // Only JavaScript's lexer knows them.
        assert_eq!(lex(Syntax::C, "x = /a{/")[0], "x = /a{/");
    }

    #[test]
    fn a_hash_in_a_shell_word_is_not_a_comment() {
        let lines = lex(Syntax::SHELL, "n=${#list[@]} # count");
        assert_eq!(lines[0].trim_end(), "n=${#list[@]}");
        let lines = lex(Syntax::PHP, "#[Attribute] # comment");
        assert_eq!(lines[0].trim_end(), "#[Attribute]");
    }

    #[test]
    fn verbatim_strings_take_a_backslash_as_it_is() {
        let lines = lex(Syntax::CSHARP, "var p = @\"C:\\\"; class A {}");
        assert_eq!(lines[0].matches('{').count(), 1);
        assert!(lines[0].ends_with("; class A {}"));
    }

    #[test]
    fn characters_outside_ascii_keep_their_bytes() {
        let lines = lex(Syntax::RUST, "fn é() { \"ü\" } // ä");
        assert_eq!(lines[0].trim_end(), "fn é() { \"  \" }");
    }
}
