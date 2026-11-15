//! A small regular-expression matcher for `cargo xtask ci-log --grep`.
//!
//! `xtask` has no regular-expression crate, and every CI job builds `xtask`
//! before it runs anything: a dependency for a search through a log is a
//! cost on every pull request. This is the part of the usual syntax a search
//! of a log needs, about a hundred lines of backtracking matcher, and it only
//! answers whether a line matches.
//!
//! **What it reads:** literals; `.`; classes (`[a-z_]`, `[^0-9]`); the
//! escapes `\d \D \w \W \s \S` and `\` before any other mark for the mark
//! itself (`\t` and `\n` are a tab and a new line); the groups `( )` and
//! `(?: )`; `|`; the repeats `* + ? {n} {n,} {n,m}`; the anchors `^` and `$`;
//! and `(?i)` at the very start, for a match that ignores case. A repeat
//! followed by `?` is read and its laziness ignored, which cannot change
//! whether a line matches. **What it refuses,** with a message: anything
//! else that starts `(?`, look-around and back references among it.
//!
//! A pattern that backtracks without end on a long line (`(a*)*b`) is slow
//! here as it is in any backtracking matcher: it is a tool for a person at a
//! terminal, not for a stranger's pattern.

use crate::Result;

/// The most a count between braces may be: a mistyped `{100000}` is refused
/// instead of searched.
const MOST_REPEATS: usize = 1000;

/// A pattern, ready to be tried on lines.
#[derive(Debug)]
pub(crate) struct Pattern {
    alternatives: Vec<Vec<Piece>>,
    ignore_case: bool,
}

#[derive(Debug)]
struct Piece {
    atom: Atom,
    min: usize,
    max: Option<usize>,
}

#[derive(Debug)]
enum Atom {
    Char(char),
    Any,
    Class { negated: bool, items: Vec<Member> },
    Group(Vec<Vec<Piece>>),
    Start,
    End,
}

#[derive(Debug)]
enum Member {
    Range(char, char),
    Digit(bool),
    Word(bool),
    Space(bool),
}

impl Pattern {
    /// Reads `text`, or says what is wrong with it.
    pub(crate) fn new(text: &str) -> Result<Self> {
        let (text, ignore_case) = match text.strip_prefix("(?i)") {
            Some(rest) => (rest, true),
            None => (text, false),
        };
        let chars: Vec<char> = text.chars().collect();
        let mut at = 0;
        let alternatives = parse_alternatives(&chars, &mut at, 0)?;
        if at != chars.len() {
            return Err(format!("`{text}`: a `)` with no `(` before it"));
        }
        Ok(Pattern {
            alternatives,
            ignore_case,
        })
    }

    /// Whether the pattern matches anywhere in `line`.
    pub(crate) fn is_match(&self, line: &str) -> bool {
        let text: Vec<char> = line.chars().collect();
        let matcher = Matcher {
            text: &text,
            ignore_case: self.ignore_case,
        };
        (0..=text.len()).any(|start| {
            self.alternatives
                .iter()
                .any(|alternative| matcher.sequence(alternative, start, &mut |_| true))
        })
    }
}

fn parse_alternatives(chars: &[char], at: &mut usize, depth: usize) -> Result<Vec<Vec<Piece>>> {
    let mut alternatives = vec![Vec::new()];
    while let Some(&mark) = chars.get(*at) {
        match mark {
            ')' if depth > 0 => break,
            ')' => return Err("a `)` with no `(` before it".to_owned()),
            '|' => {
                *at += 1;
                alternatives.push(Vec::new());
                continue;
            }
            _ => {}
        }
        let atom = parse_atom(chars, at, depth)?;
        let (min, max) = parse_repeat(chars, at)?;
        if let Some(pieces) = alternatives.last_mut() {
            pieces.push(Piece { atom, min, max });
        }
    }
    Ok(alternatives)
}

fn parse_atom(chars: &[char], at: &mut usize, depth: usize) -> Result<Atom> {
    let mark = chars[*at];
    *at += 1;
    Ok(match mark {
        '.' => Atom::Any,
        '^' => Atom::Start,
        '$' => Atom::End,
        '*' | '+' | '?' => return Err(format!("`{mark}` has nothing to repeat")),
        '(' => {
            if chars.get(*at) == Some(&'?') {
                if chars.get(*at + 1) == Some(&':') {
                    *at += 2;
                } else {
                    return Err("only `(?:` and a leading `(?i)` are read after `(?`".to_owned());
                }
            }
            let inner = parse_alternatives(chars, at, depth + 1)?;
            if chars.get(*at) != Some(&')') {
                return Err("a `(` is never closed".to_owned());
            }
            *at += 1;
            Atom::Group(inner)
        }
        '[' => parse_class(chars, at)?,
        '\\' => {
            let escaped = *chars.get(*at).ok_or("a `\\` ends the pattern")?;
            *at += 1;
            match shorthand(escaped) {
                Some(member) => Atom::Class {
                    negated: false,
                    items: vec![member],
                },
                None => Atom::Char(literal_escape(escaped)),
            }
        }
        other => Atom::Char(other),
    })
}

/// `\d \D \w \W \s \S` as a member of a class.
fn shorthand(letter: char) -> Option<Member> {
    Some(match letter {
        'd' => Member::Digit(true),
        'D' => Member::Digit(false),
        'w' => Member::Word(true),
        'W' => Member::Word(false),
        's' => Member::Space(true),
        'S' => Member::Space(false),
        _ => return None,
    })
}

fn literal_escape(escaped: char) -> char {
    match escaped {
        't' => '\t',
        'n' => '\n',
        other => other,
    }
}

fn parse_class(chars: &[char], at: &mut usize) -> Result<Atom> {
    let negated = chars.get(*at) == Some(&'^');
    if negated {
        *at += 1;
    }
    let mut items = Vec::new();
    let mut first = true;
    loop {
        let mark = *chars.get(*at).ok_or("a `[` is never closed")?;
        *at += 1;
        // A `]` first in the class is a member, as in the usual syntax.
        if mark == ']' && !first {
            break;
        }
        first = false;
        let low = if mark == '\\' {
            let escaped = *chars.get(*at).ok_or("a `\\` ends the pattern")?;
            *at += 1;
            if let Some(member) = shorthand(escaped) {
                items.push(member);
                continue;
            }
            literal_escape(escaped)
        } else {
            mark
        };
        // `a-z`, but not a `-` before the closing bracket.
        if chars.get(*at) == Some(&'-') && chars.get(*at + 1).is_some_and(|next| *next != ']') {
            let mut high = chars[*at + 1];
            *at += 2;
            if high == '\\' {
                high = literal_escape(*chars.get(*at).ok_or("a `\\` ends the pattern")?);
                *at += 1;
            }
            if high < low {
                return Err(format!("the range `{low}-{high}` runs backwards"));
            }
            items.push(Member::Range(low, high));
        } else {
            items.push(Member::Range(low, low));
        }
    }
    Ok(Atom::Class { negated, items })
}

/// The repeat after an atom: `(1, Some(1))` when there is none.
fn parse_repeat(chars: &[char], at: &mut usize) -> Result<(usize, Option<usize>)> {
    let (min, max) = match chars.get(*at) {
        Some('*') => (0, None),
        Some('+') => (1, None),
        Some('?') => (0, Some(1)),
        Some('{') => return parse_braces(chars, at),
        _ => return Ok((1, Some(1))),
    };
    *at += 1;
    // A lazy repeat matches the same lines.
    if chars.get(*at) == Some(&'?') {
        *at += 1;
    }
    Ok((min, max))
}

fn parse_braces(chars: &[char], at: &mut usize) -> Result<(usize, Option<usize>)> {
    let close = chars[*at..]
        .iter()
        .position(|mark| *mark == '}')
        .ok_or("a `{` is never closed")?
        + *at;
    let inside: String = chars[*at + 1..close].iter().collect();
    let count = |text: &str| {
        text.parse::<usize>()
            .ok()
            .filter(|count| *count <= MOST_REPEATS)
            .ok_or_else(|| format!("`{{{inside}}}`: a count from 0 to {MOST_REPEATS}"))
    };
    let (min, max) = match inside.split_once(',') {
        None => (count(&inside)?, Some(count(&inside)?)),
        Some((low, "")) => (count(low)?, None),
        Some((low, high)) => (count(low)?, Some(count(high)?)),
    };
    if max.is_some_and(|max| max < min) {
        return Err(format!("`{{{inside}}}` runs backwards"));
    }
    *at = close + 1;
    if chars.get(*at) == Some(&'?') {
        *at += 1;
    }
    Ok((min, max))
}

struct Matcher<'a> {
    text: &'a [char],
    ignore_case: bool,
}

impl Matcher<'_> {
    /// Matches `pieces` from `at`, then asks `then` whether what follows
    /// does too, with the position the pieces ended at.
    fn sequence(&self, pieces: &[Piece], at: usize, then: &mut dyn FnMut(usize) -> bool) -> bool {
        match pieces.split_first() {
            None => then(at),
            Some((first, rest)) => self.repeat(first, rest, at, 0, then),
        }
    }

    /// `piece` has matched `count` times; match it again as far as it goes
    /// before the rest, and give up repeats one at a time.
    fn repeat(
        &self,
        piece: &Piece,
        rest: &[Piece],
        at: usize,
        count: usize,
        then: &mut dyn FnMut(usize) -> bool,
    ) -> bool {
        if piece.max.is_none_or(|max| count < max) {
            let mut again = |next: usize| {
                // A repeat that matched nothing would go round for ever.
                if next == at && count >= piece.min {
                    return false;
                }
                self.repeat(piece, rest, next, count + 1, then)
            };
            if self.atom(&piece.atom, at, &mut again) {
                return true;
            }
        }
        count >= piece.min && self.sequence(rest, at, then)
    }

    fn atom(&self, atom: &Atom, at: usize, then: &mut dyn FnMut(usize) -> bool) -> bool {
        let one = |matches: &dyn Fn(char) -> bool| self.text.get(at).is_some_and(|c| matches(*c));
        match atom {
            Atom::Start => at == 0 && then(at),
            Atom::End => at == self.text.len() && then(at),
            Atom::Any => one(&|_| true) && then(at + 1),
            Atom::Char(wanted) => one(&|c| self.same(c, *wanted)) && then(at + 1),
            Atom::Class { negated, items } => {
                one(&|c| self.in_class(items, c) != *negated) && then(at + 1)
            }
            Atom::Group(alternatives) => alternatives
                .iter()
                .any(|alternative| self.sequence(alternative, at, then)),
        }
    }

    fn same(&self, a: char, b: char) -> bool {
        a == b || (self.ignore_case && a.to_lowercase().eq(b.to_lowercase()))
    }

    fn in_class(&self, items: &[Member], c: char) -> bool {
        items.iter().any(|member| match member {
            Member::Range(low, high) => {
                (*low..=*high).contains(&c)
                    || (self.ignore_case
                        && c.to_lowercase()
                            .chain(c.to_uppercase())
                            .any(|other| (*low..=*high).contains(&other)))
            }
            Member::Digit(want) => c.is_ascii_digit() == *want,
            Member::Word(want) => (c.is_alphanumeric() || c == '_') == *want,
            Member::Space(want) => c.is_whitespace() == *want,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Pattern;

    fn matches(pattern: &str, line: &str) -> bool {
        Pattern::new(pattern)
            .unwrap_or_else(|error| panic!("`{pattern}`: {error}"))
            .is_match(line)
    }

    #[test]
    fn a_pattern_matches_anywhere_in_a_line_unless_it_is_anchored() {
        assert!(matches("panicked", "thread 'x' panicked at src/lib.rs"));
        assert!(!matches("^panicked", "thread 'x' panicked"));
        assert!(matches("^thread", "thread 'x' panicked"));
        assert!(matches("rs$", "src/lib.rs"));
        assert!(!matches("rs$", "src/lib.rs:4"));
        assert!(matches("^$", ""));
        assert!(!matches("^$", " "));
    }

    #[test]
    fn marks_classes_and_repeats_are_read_as_the_usual_syntax_has_them() {
        assert!(matches("test .* FAIL", "test a::b ... FAIL"));
        assert!(matches(r"\d+ passed", "12 passed"));
        assert!(!matches(r"^\d+ passed", "x12 passed"));
        assert!(matches("a.c", "xabcx"));
        assert!(matches(r"a\.c", "a.c"));
        assert!(!matches(r"a\.c", "abc"));
        assert!(matches("[a-c]+z", "xxbaz"));
        assert!(!matches("[^a-c]z", "bz"));
        assert!(matches("[^a-c]z", "dz"));
        assert!(matches(r"[\d.]+s", "took 1.5s"));
        assert!(matches("[]x]", "]"));
        assert!(matches("[a-]", "-"));
        assert!(matches("colou?r", "color"));
        assert!(matches("colou?r", "colour"));
        assert!(!matches("^colou?r$", "colouur"));
        assert!(matches("^a{2,3}$", "aaa"));
        assert!(!matches("^a{2,3}$", "aaaa"));
        assert!(!matches("^a{2}$", "a"));
        assert!(matches("^a{2,}$", "aaaaa"));
        assert!(matches(r"\w+\s\S", "ab c"));
        assert!(matches("a.*?b", "xaxxb"));
    }

    #[test]
    fn groups_and_alternatives_take_the_longest_way_that_lets_the_rest_match() {
        assert!(matches("error|warning", "a warning"));
        assert!(!matches("error|warning", "a note"));
        assert!(matches("^(foo|bar)baz$", "barbaz"));
        assert!(!matches("^(foo|bar)baz$", "foobar"));
        assert!(matches("^(?:ab)+$", "ababab"));
        assert!(!matches("^(?:ab)+$", "ababa"));
        // The repeat has to give back what the rest needs.
        assert!(matches("^a*ab$", "aaab"));
        assert!(matches("^(a|ab)(c|bcd)$", "abcd"));
        // A repeat of nothing ends.
        assert!(matches("^(a*)*$", "aaa"));
        assert!(!matches("^(a*)*b", "aaa"));
    }

    #[test]
    fn a_leading_flag_ignores_case_and_the_default_does_not() {
        assert!(matches("(?i)error", "an ERROR here"));
        assert!(matches("(?i)[a-c]x", "BX"));
        assert!(!matches("error", "an ERROR here"));
    }

    #[test]
    fn a_pattern_that_cannot_be_read_says_why() {
        for (pattern, expected) in [
            ("(ab", "never closed"),
            ("ab)", "no `(`"),
            ("[ab", "never closed"),
            ("*a", "nothing to repeat"),
            ("a{2", "never closed"),
            ("a{3,2}", "runs backwards"),
            ("a{5000}", "a count from 0"),
            ("[z-a]", "runs backwards"),
            ("a\\", "ends the pattern"),
            ("(?=a)", "only `(?:`"),
        ] {
            let error = Pattern::new(pattern).unwrap_err();
            assert!(error.contains(expected), "`{pattern}`: {error}");
        }
    }
}
