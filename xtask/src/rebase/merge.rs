//! A three-way merge of lines that settles one kind of conflict alone: both
//! sides adding lines at the same place, and nothing else.
//!
//! git stops a rebase where both sides changed the same place. When both only
//! **added** lines there (a test appended to the same module, a row added to
//! the end of the same table, a field added to the same struct) the answer is
//! both additions, the side being rebased onto first. When either side
//! changed or removed a line of the base there, no such answer is safe: two
//! edits of one line kept side by side compile and read, and say the wrong
//! thing. That is a person's.
//!
//! The diff is this file's own (Myers', on whole lines), so that the merge is
//! a pure function of the three texts, and so that an addition is placed as
//! far down as it can go: an addition at a function's end can be read as
//! starting with the line after the function's last `}` (an addition of
//! "`}`, a blank line, `#[test]`, `fn b() {`, its body" before the old `}`),
//! and two additions read that way at one place, put side by side, lose a
//! brace. Pushed down, each is "a blank line, the test, its `}`" after the old
//! `}`, and the two put side by side are whole.

/// What a merge of three texts gave.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Merged {
    /// The merged text, with the line ends of `ours`.
    pub(crate) text: String,
    /// Each place both sides added lines, both kept: its line in the merged
    /// text (from 1), and the lines added there, ours then theirs.
    pub(crate) unions: Vec<Union>,
}

/// One place both sides added lines, and both were kept.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Union {
    /// The first added line's number in the merged text, from 1.
    pub(crate) line: usize,
    pub(crate) ours: Vec<String>,
    pub(crate) theirs: Vec<String>,
}

/// Why a merge was not made: where, in the base, and what each side did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    /// The base's line (from 1) the place starts at.
    pub(crate) line: usize,
    pub(crate) why: String,
}

/// The most edits a diff looks through before it gives up: two texts this
/// far apart are not two additions to one file, and the merge is left to a
/// person rather than searched for at length.
const MOST_EDITS: usize = 4000;

/// A run of lines of `a` (`a_start..a_end`) that the other text has as
/// `b_start..b_end` instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hunk {
    pub(crate) a_start: usize,
    pub(crate) a_end: usize,
    pub(crate) b_start: usize,
    pub(crate) b_end: usize,
}

impl Hunk {
    fn is_addition(&self) -> bool {
        self.a_start == self.a_end
    }
}

/// A text as lines, each without its line end, and how it ended: the line
/// end it uses (`\r\n` when its first line has one) and whether its last line
/// has one.
pub(crate) struct Lines<'a> {
    pub(crate) lines: Vec<&'a str>,
    pub(crate) crlf: bool,
    pub(crate) ends_with_newline: bool,
}

pub(crate) fn lines(text: &str) -> Lines<'_> {
    let crlf = text.find('\n').is_some_and(|at| text[..at].ends_with('\r'));
    let ends_with_newline = text.ends_with('\n');
    let lines = text
        .split_inclusive('\n')
        .map(|line| {
            line.strip_suffix('\n')
                .map_or(line, |line| line.strip_suffix('\r').unwrap_or(line))
        })
        .collect();
    Lines {
        lines,
        crlf,
        ends_with_newline,
    }
}

/// The lines `a` and `b` have in common, along a shortest edit from one to
/// the other, as pairs of their indices in order; `None` past
/// [`MOST_EDITS`].
pub(crate) fn common(a: &[&str], b: &[&str]) -> Option<Vec<(usize, usize)>> {
    // The same lines at the start and at the end cost nothing to find.
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let middle = myers(&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix])?;
    let mut pairs: Vec<(usize, usize)> = (0..prefix).map(|at| (at, at)).collect();
    pairs.extend(middle.into_iter().map(|(x, y)| (x + prefix, y + prefix)));
    pairs.extend((0..suffix).map(|at| (a.len() - suffix + at, b.len() - suffix + at)));
    Some(pairs)
}

/// Myers' shortest edit, with each round's frontier kept to walk back
/// along: the pairs of lines it keeps.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<(usize, usize)>> {
    let (n, m) = (a.len() as isize, b.len() as isize);
    let most = (n + m) as usize;
    let offset = most as isize + 1;
    let mut v = vec![0_isize; 2 * most + 3];
    // Each round's frontier before it ran, the diagonals -d-1..=d+1.
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let index = |k: isize| (k + offset) as usize;
    for d in 0..=most.min(MOST_EDITS) as isize {
        trace.push(v[index(-d - 1)..=index(d + 1)].to_vec());
        for k in (-d..=d).step_by(2) {
            let mut x = if k == -d || (k != d && v[index(k - 1)] < v[index(k + 1)]) {
                v[index(k + 1)]
            } else {
                v[index(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[index(k)] = x;
            if x >= n && y >= m {
                return Some(walk_back(&trace, n, m));
            }
        }
    }
    None
}

/// The pairs kept, read back from the frontiers of [`myers`].
fn walk_back(trace: &[Vec<isize>], n: isize, m: isize) -> Vec<(usize, usize)> {
    let (mut x, mut y) = (n, m);
    let mut pairs = Vec::new();
    for (d, frontier) in trace.iter().enumerate().rev() {
        let d = d as isize;
        // The frontier holds the diagonals -d-1..=d+1.
        let at = |k: isize| frontier[(k + d + 1) as usize];
        let k = x - y;
        let previous = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let previous_x = at(previous);
        let previous_y = previous_x - previous;
        while x > previous_x && y > previous_y {
            x -= 1;
            y -= 1;
            pairs.push((x as usize, y as usize));
        }
        if d > 0 {
            x = previous_x;
            y = previous_y;
        }
    }
    pairs.reverse();
    pairs
}

/// The runs between the pairs `common` keeps: what `b` has in place of each
/// part of `a`. Each run of added lines is then pushed as far down as it
/// goes ([`push_down`]).
pub(crate) fn hunks(a: &[&str], b: &[&str]) -> Option<Vec<Hunk>> {
    let pairs = common(a, b)?;
    let mut found = Vec::new();
    let (mut x, mut y) = (0, 0);
    for (at_a, at_b) in pairs.into_iter().chain([(a.len(), b.len())]) {
        if at_a > x || at_b > y {
            found.push(Hunk {
                a_start: x,
                a_end: at_a,
                b_start: y,
                b_end: at_b,
            });
        }
        x = at_a + 1;
        y = at_b + 1;
    }
    push_down(a, b, &mut found);
    Some(found)
}

/// Pushes each run of added lines down past a line of `a` it starts with:
/// "`}`, then X" added before a `}` is the same text as "X, then `}`" added
/// after it. A run goes no further than the next run, so that the two stay
/// apart.
pub(crate) fn push_down(a: &[&str], b: &[&str], hunks: &mut [Hunk]) {
    for at in 0..hunks.len() {
        let next = hunks.get(at + 1).map_or(a.len(), |next| next.a_start);
        let hunk = &mut hunks[at];
        if !hunk.is_addition() {
            continue;
        }
        while hunk.a_start < next
            && hunk.b_end < b.len()
            && b[hunk.b_start] == b[hunk.b_end]
            && a[hunk.a_start] == b[hunk.b_end]
        {
            hunk.a_start += 1;
            hunk.a_end += 1;
            hunk.b_start += 1;
            hunk.b_end += 1;
        }
    }
}

/// The three texts merged, when the two sides' changes are apart or are
/// additions at one place; the first place where they are not, otherwise.
pub(crate) fn merge(base: &str, ours: &str, theirs: &str) -> Result<Merged, Refusal> {
    let (base, ours, theirs) = (lines(base), lines(ours), lines(theirs));
    let too_far = |side: &str| Refusal {
        line: 1,
        why: format!(
            "{side} is more than {MOST_EDITS} lines' edits from the base: not two sets of \
             additions"
        ),
    };
    let mine = hunks(&base.lines, &ours.lines).ok_or_else(|| too_far("ours"))?;
    let other = hunks(&base.lines, &theirs.lines).ok_or_else(|| too_far("theirs"))?;

    let mut out: Vec<&str> = Vec::new();
    let mut unions = Vec::new();
    // Where each side's text is ahead of the base's, at the cursor.
    let (mut shift_ours, mut shift_theirs) = (0_isize, 0_isize);
    let (mut i, mut j) = (0, 0);
    let mut cursor = 0;
    while i < mine.len() || j < other.len() {
        // The next place either side changed, grown to every change of
        // either side that overlaps it or touches it.
        let start = match (mine.get(i), other.get(j)) {
            (Some(one), Some(two)) => one.a_start.min(two.a_start),
            (Some(one), None) => one.a_start,
            (None, Some(two)) => two.a_start,
            (None, None) => break,
        };
        let mut end = start;
        let (first_i, first_j) = (i, j);
        loop {
            if let Some(one) = mine.get(i).filter(|one| one.a_start <= end) {
                end = end.max(one.a_end);
                i += 1;
            } else if let Some(two) = other.get(j).filter(|two| two.a_start <= end) {
                end = end.max(two.a_end);
                j += 1;
            } else {
                break;
            }
        }
        out.extend(&base.lines[cursor..start]);
        let grown = |side: &[Hunk], from: usize, to: usize| -> isize {
            side[from..to]
                .iter()
                .map(|hunk| {
                    (hunk.b_end - hunk.b_start) as isize - (hunk.a_end - hunk.a_start) as isize
                })
                .sum()
        };
        let ours_grew = grown(&mine, first_i, i);
        let theirs_grew = grown(&other, first_j, j);
        let ours_text = span(&ours, start, end, shift_ours, ours_grew);
        let theirs_text = span(&theirs, start, end, shift_theirs, theirs_grew);
        let ours_changed = i > first_i;
        let theirs_changed = j > first_j;
        if ours_changed && theirs_changed && ours_text != theirs_text {
            if start != end {
                return Err(Refusal {
                    line: start + 1,
                    why: format!(
                        "both sides change line(s) {}..{} of the base: not additions alone",
                        start + 1,
                        end
                    ),
                });
            }
            unions.push(Union {
                line: out.len() + 1,
                ours: ours_text.iter().map(|line| (*line).to_owned()).collect(),
                theirs: theirs_text.iter().map(|line| (*line).to_owned()).collect(),
            });
            out.extend(&ours_text);
            out.extend(&theirs_text);
        } else if ours_changed {
            out.extend(&ours_text);
        } else {
            out.extend(&theirs_text);
        }
        shift_ours += ours_grew;
        shift_theirs += theirs_grew;
        cursor = end;
    }
    out.extend(&base.lines[cursor..]);

    let end = if ours.crlf { "\r\n" } else { "\n" };
    let mut text = out.join(end);
    // The last line ends as ours' does, or as theirs' when only theirs
    // changed the end.
    let ends = if ours.lines.len() == base.lines.len() && ours.lines == base.lines {
        theirs.ends_with_newline
    } else {
        ours.ends_with_newline
    };
    if ends && !out.is_empty() {
        text.push_str(end);
    }
    Ok(Merged { text, unions })
}

/// A side's lines for the base's `start..end`: shifted by what the side
/// grew before it (`shift`) and within it (`grew`).
fn span<'t>(text: &Lines<'t>, start: usize, end: usize, shift: isize, grew: isize) -> Vec<&'t str> {
    let from = (start as isize + shift) as usize;
    let to = (end as isize + shift + grew) as usize;
    text.lines[from..to].to_vec()
}
