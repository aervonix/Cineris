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
//! template literal's `${…}`, no C++ raw strings, and a heredoc's tag must
//! look like one.

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
    pub(crate) const KOTLIN: Self = Self {
        triple: b"\"",
        nested_block: true,
        ..Self::C_LIKE
    };
    pub(crate) const SWIFT: Self = Self::KOTLIN;
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
}

impl Lexer {
    pub(crate) fn new(syntax: Syntax) -> Self {
        Self {
            syntax,
            state: State::Code,
            next_heredoc: None,
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
            // Heredocs. `<` is ASCII, so `at` is on a character's start.
            if byte == b'<'
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
        let close = bytes.iter().skip(3).position(|&b| b == b'\'')?;
        return (close <= 10).then_some(close + 4);
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
            match after.strip_prefix('-') {
                Some(after) => (after, true, 3),
                None => (after, false, 2),
            }
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
        .collect();
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
        // An arithmetic shift is not a heredoc.
        let lines = lex(Syntax::SHELL, "x=$((1<<2))\nf() {");
        assert_eq!(lines[1], "f() {");
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
