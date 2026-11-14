//! Fuzzy matching for the completion menus: `scnum` finds
//! `src/lex/scan_number.rs`.
//!
//! **A match** is the query's characters in the candidate in order, with
//! anything between them, ignoring case. **Its score** favours what a
//! person typing a path means:
//!
//! - characters at the start of a part of the path (after `/`, `_`, `-`,
//!   `.` or a space, or a capital after a small letter) and characters one
//!   after another;
//! - a match wholly inside the file's own name, over one that reaches into
//!   its folders;
//! - a shorter path, when all else is equal.
//!
//! The match scored is the tightest one that ends where the first does: the
//! query is found left to right, then right to left back from its end. That
//! is not always the best-scoring alignment, and is linear in the
//! candidate's length.
//!
//! **Speed.** It must filter a large workspace's paths within a frame as
//! the user types. ASCII paths, nearly all of them, are matched byte by
//! byte with no allocation; a query that extends the last one is matched
//! only against what the last one matched. Measured on 100,000 generated
//! paths in `docs/performance-budgets.md`.

/// A list of candidates, prepared once for matching many queries.
#[derive(Debug, Clone, Default)]
pub struct Candidates {
    items: Vec<Item>,
}

#[derive(Debug, Clone)]
struct Item {
    text: String,
    /// The characters, for a candidate that is not ASCII.
    chars: Option<Box<[char]>>,
    /// Where its last part (the file's name) starts: in bytes for ASCII,
    /// in characters otherwise.
    name: usize,
}

impl Candidates {
    /// Prepares `texts`, in the order to show them when nothing has been
    /// typed.
    #[must_use]
    pub fn new(texts: impl IntoIterator<Item = String>) -> Self {
        let items = texts
            .into_iter()
            .map(|text| {
                if text.is_ascii() {
                    let name = text.trim_end_matches('/').rfind('/').map_or(0, |at| at + 1);
                    Item {
                        text,
                        chars: None,
                        name,
                    }
                } else {
                    let chars: Box<[char]> = text.chars().collect();
                    let trimmed = chars.iter().rposition(|c| *c != '/').map_or(0, |at| at + 1);
                    let name = chars[..trimmed]
                        .iter()
                        .rposition(|c| *c == '/')
                        .map_or(0, |at| at + 1);
                    Item {
                        text,
                        chars: Some(chars),
                        name,
                    }
                }
            })
            .collect();
        Self { items }
    }

    /// How many there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The candidate at `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&str> {
        self.items.get(index).map(|item| item.text.as_str())
    }
}

/// One candidate that matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    /// Its index in the [`Candidates`].
    pub index: usize,
    /// How well it matched: higher is better.
    pub score: i32,
}

/// A filter over one list of candidates, remembering what its last query
/// matched so that a longer query looks only there.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    query: String,
    /// The indices the last query matched; `None` when it is not known.
    pool: Option<Vec<u32>>,
}

impl Filter {
    /// A filter that has matched nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets what was matched: the candidates changed.
    pub fn reset(&mut self) {
        self.pool = None;
        self.query.clear();
    }

    /// The best `limit` matches of `query`, best first. An empty query
    /// matches everything, in the candidates' own order.
    pub fn run(&mut self, candidates: &Candidates, query: &str, limit: usize) -> Vec<Hit> {
        let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
        if needle.is_empty() {
            self.query.clear();
            self.pool = None;
            return (0..candidates.len().min(limit))
                .map(|index| Hit { index, score: 0 })
                .collect();
        }
        let narrowing = !self.query.is_empty() && query.starts_with(self.query.as_str());
        let ascii: Option<Vec<u8>> = needle
            .iter()
            .all(char::is_ascii)
            .then(|| needle.iter().map(|c| *c as u8).collect());
        let mut hits = Vec::new();
        let mut pool = Vec::new();
        let mut consider = |index: usize| {
            let item = &candidates.items[index];
            let score = match (&item.chars, &ascii) {
                (None, Some(needle)) => score(item.text.as_bytes(), needle, item.name),
                (None, None) => None,
                (Some(chars), _) => score(chars, &needle, item.name),
            };
            if let Some(score) = score {
                pool.push(u32::try_from(index).unwrap_or(u32::MAX));
                hits.push(Hit { index, score });
            }
        };
        match (&self.pool, narrowing) {
            (Some(previous), true) => previous.iter().for_each(|&index| consider(index as usize)),
            _ => (0..candidates.len()).for_each(consider),
        }
        self.query = query.to_owned();
        self.pool = Some(pool);
        best(&mut hits, candidates, limit);
        hits
    }
}

/// The best `limit` of `hits`, best first; equal scores go shorter first,
/// then in the candidates' order.
fn best(hits: &mut Vec<Hit>, candidates: &Candidates, limit: usize) {
    let order = |a: &Hit, b: &Hit| {
        b.score
            .cmp(&a.score)
            .then_with(|| {
                candidates.items[a.index]
                    .text
                    .len()
                    .cmp(&candidates.items[b.index].text.len())
            })
            .then_with(|| a.index.cmp(&b.index))
    };
    if hits.len() > limit && limit > 0 {
        hits.select_nth_unstable_by(limit - 1, order);
        hits.truncate(limit);
    }
    hits.sort_unstable_by(order);
    hits.truncate(limit);
}

/// One unit of a candidate: a byte of an ASCII one, a character otherwise.
trait Unit: Copy + Eq {
    fn folded(self) -> Self;
    fn is_separator(self) -> bool;
    fn is_delimiter(self) -> bool;
    fn is_upper(self) -> bool;
    fn is_lower(self) -> bool;
}

impl Unit for u8 {
    fn folded(self) -> Self {
        self.to_ascii_lowercase()
    }
    fn is_separator(self) -> bool {
        self == b'/' || self == b'\\'
    }
    fn is_delimiter(self) -> bool {
        matches!(self, b'_' | b'-' | b'.' | b' ')
    }
    fn is_upper(self) -> bool {
        self.is_ascii_uppercase()
    }
    fn is_lower(self) -> bool {
        self.is_ascii_lowercase()
    }
}

impl Unit for char {
    fn folded(self) -> Self {
        self.to_lowercase().next().unwrap_or(self)
    }
    fn is_separator(self) -> bool {
        self == '/' || self == '\\'
    }
    fn is_delimiter(self) -> bool {
        matches!(self, '_' | '-' | '.' | ' ')
    }
    fn is_upper(self) -> bool {
        self.is_uppercase()
    }
    fn is_lower(self) -> bool {
        self.is_lowercase()
    }
}

/// Points for each character matched.
const MATCHED: i32 = 16;
/// More for one at the start of the candidate or of a part of a path.
const AT_PART: i32 = 10;
/// More for one after `_`, `-`, `.` or a space.
const AFTER_DELIMITER: i32 = 8;
/// More for a capital after a small letter.
const AT_CAPITAL: i32 = 7;
/// More for one right after the last.
const RUN: i32 = 12;
/// Less for each unit skipped between two matched ones, up to [`GAP_MOST`].
const GAP: i32 = 2;
const GAP_MOST: i32 = 16;
/// More when the whole match is in the file's own name.
const IN_NAME: i32 = 24;

/// How well `needle` (folded) matches `hay`, or `None` when it does not.
/// `name` is where the file's own name starts in `hay`.
fn score<T: Unit>(hay: &[T], needle: &[T], name: usize) -> Option<i32> {
    // Left to right: does it match at all, and where does the first match
    // end?
    let mut next = 0;
    let mut end = None;
    for (at, unit) in hay.iter().enumerate() {
        if unit.folded() == needle[next] {
            next += 1;
            if next == needle.len() {
                end = Some(at);
                break;
            }
        }
    }
    let end = end?;
    // Right to left from there: the latest start, so the tightest match.
    let mut left = needle.len();
    let mut start = end;
    for at in (0..=end).rev() {
        if hay[at].folded() == needle[left - 1] {
            left -= 1;
            if left == 0 {
                start = at;
                break;
            }
        }
    }
    let mut total = 0;
    let mut next = 0;
    let mut last: Option<usize> = None;
    for at in start..=end {
        if next == needle.len() {
            break;
        }
        if hay[at].folded() != needle[next] {
            continue;
        }
        total += MATCHED + bonus(hay, at);
        match last {
            Some(last) if last + 1 == at => total += RUN,
            Some(last) => {
                let skipped = i32::try_from(at - last - 1).unwrap_or(i32::MAX);
                total -= skipped.saturating_mul(GAP).min(GAP_MOST);
            }
            None => {}
        }
        last = Some(at);
        next += 1;
    }
    if start >= name {
        total += IN_NAME;
    }
    let length = i32::try_from(hay.len() / 16).unwrap_or(i32::MAX);
    Some(total - length)
}

fn bonus<T: Unit>(hay: &[T], at: usize) -> i32 {
    let Some(before) = at.checked_sub(1).map(|before| hay[before]) else {
        return AT_PART;
    };
    if before.is_separator() {
        AT_PART
    } else if before.is_delimiter() {
        AFTER_DELIMITER
    } else if before.is_lower() && hay[at].is_upper() {
        AT_CAPITAL
    } else {
        0
    }
}

/// Where `query` matched in `text`, as the byte ranges of the characters
/// matched: for drawing them in bold. The same alignment the score used.
#[must_use]
pub fn positions(text: &str, query: &str) -> Vec<std::ops::Range<usize>> {
    let needle: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let hay: Vec<(usize, char)> = text.char_indices().collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut next = 0;
    let mut end = None;
    for (at, (_, c)) in hay.iter().enumerate() {
        if c.folded() == needle[next] {
            next += 1;
            if next == needle.len() {
                end = Some(at);
                break;
            }
        }
    }
    let Some(end) = end else {
        return Vec::new();
    };
    let mut left = needle.len();
    let mut found = Vec::new();
    for at in (0..=end).rev() {
        if left > 0 && hay[at].1.folded() == needle[left - 1] {
            left -= 1;
            let (offset, c) = hay[at];
            found.push(offset..offset + c.len_utf8());
        }
    }
    found.reverse();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(paths: &[&str], query: &str) -> Vec<String> {
        let candidates = Candidates::new(paths.iter().map(|p| (*p).to_owned()));
        Filter::new()
            .run(&candidates, query, 8)
            .into_iter()
            .map(|hit| candidates.get(hit.index).unwrap().to_owned())
            .collect()
    }

    #[test]
    fn the_characters_in_order_ignoring_case_match() {
        let paths = ["src/lex/scan_number.rs", "src/main.rs", "README.md"];
        assert_eq!(run(&paths, "scnum"), ["src/lex/scan_number.rs"]);
        assert_eq!(run(&paths, "READ"), ["README.md"]);
        assert!(run(&paths, "xyz").is_empty());
    }

    #[test]
    fn a_match_in_the_file_name_and_at_word_starts_comes_first() {
        let paths = [
            "lexer/tests/mod.rs",
            "src/lex.rs",
            "docs/lexicon/extra.md",
            "src/the_lexer.rs",
        ];
        let found = run(&paths, "lex");
        assert_eq!(found[..2], ["src/lex.rs", "src/the_lexer.rs"]);
    }

    #[test]
    fn a_longer_query_narrows_what_the_last_one_matched() {
        let candidates = Candidates::new(["abc", "abd", "xyz", "abcd"].map(String::from));
        let mut filter = Filter::new();
        assert_eq!(filter.run(&candidates, "ab", 8).len(), 3);
        assert_eq!(filter.run(&candidates, "abc", 8).len(), 2);
        // Widening again looks at everything.
        assert_eq!(filter.run(&candidates, "a", 8).len(), 3);
        assert_eq!(filter.run(&candidates, "", 8).len(), 4);
    }

    #[test]
    fn paths_that_are_not_ascii_match_by_character() {
        let paths = ["docs/Übersicht.md", "src/中文/名字.rs"];
        assert_eq!(run(&paths, "über"), ["docs/Übersicht.md"]);
        assert_eq!(run(&paths, "名"), ["src/中文/名字.rs"]);
        assert_eq!(positions("docs/Übersicht.md", "üb"), [5..7, 7..8]);
    }

    #[test]
    fn at_most_the_limit_best_first() {
        let paths: Vec<String> = (0..20).map(|n| format!("file{n:02}.rs")).collect();
        let candidates = Candidates::new(paths);
        let hits = Filter::new().run(&candidates, "file", 8);
        assert_eq!(hits.len(), 8);
        assert!(hits.windows(2).all(|pair| pair[0].score >= pair[1].score));
    }
}
