//! A three-way merge of lines that settles two kinds of conflict alone:
//! both sides adding lines at the same place, and one side adding lines just
//! after lines the other replaced with as many or more ([`beside`]).
//!
//! git stops a rebase where both sides changed the same place. When both only
//! **added** lines there (a test appended to the same module, a row added to
//! the end of the same table, a field added to the same struct) the answer is
//! both additions, the side being rebased onto first. When either side
//! changed or removed a line of the base there in any other way, no such
//! answer is safe: two edits of one line kept side by side compile and read,
//! and say the wrong thing. That is a person's. The checks of the text a
//! union makes are `classify`'s: what a line that attaches to what follows
//! it may not be put before, among them. Line ends must be one kind in all
//! three texts, or the merge is refused, so that no line keeps an end it did
//! not have.
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
    /// Each place both sides' lines were put side by side ([`Union`]).
    pub(crate) unions: Vec<Union>,
}

/// One place both sides' lines were put side by side: both added at one
/// place (ours first), or one side added lines just after lines the other
/// replaced (the replacement first).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Union {
    /// The first kept line's number in the merged text, from 1.
    pub(crate) line: usize,
    /// The lines put first, then the lines put after them.
    pub(crate) first: Vec<String>,
    pub(crate) second: Vec<String>,
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
    /// Whether some lines end `\r\n` and others `\n`.
    pub(crate) mixed: bool,
}

pub(crate) fn lines(text: &str) -> Lines<'_> {
    let crlf = text.find('\n').is_some_and(|at| text[..at].ends_with('\r'));
    let ends_with_newline = text.ends_with('\n');
    let ends: Vec<bool> = text
        .split_inclusive('\n')
        .filter(|line| line.ends_with('\n'))
        .map(|line| line.ends_with("\r\n"))
        .collect();
    let mixed = ends.iter().any(|crlf| *crlf) && ends.iter().any(|crlf| !*crlf);
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
        mixed,
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
    // One kind of line end in all three, or a line neither side touched
    // could be written with an end it did not have.
    if let Some(side) = line_ends_differ(&base, &ours, &theirs) {
        return Err(Refusal {
            line: 1,
            why: format!(
                "{side}: the union would write every line with one kind of line end, and \
                 change the ends of lines neither side touched"
            ),
        });
    }
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
            let placed = if start == end {
                Some((ours_text.clone(), theirs_text.clone()))
            } else {
                beside(
                    (&mine[first_i..i], &ours, &ours_text),
                    (&other[first_j..j], &theirs, &theirs_text),
                    start,
                    end,
                )
            };
            let Some((first, second)) = placed else {
                return Err(Refusal {
                    line: start + 1,
                    why: format!(
                        "both sides change line(s) {}..{} of the base: not additions alone",
                        start + 1,
                        end
                    ),
                });
            };
            unions.push(Union {
                line: out.len() + 1,
                first: first.iter().map(|line| (*line).to_owned()).collect(),
                second: second.iter().map(|line| (*line).to_owned()).collect(),
            });
            out.extend(&first);
            out.extend(&second);
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

    // The union is written in ours' line ends, every line alike.
    let end = if ours.crlf { "\r\n" } else { "\n" };
    let mut text = out.join(end);
    // The last line ends with a line end when either side's does: a row
    // appended to a page whose last line has none must not leave the page
    // with none.
    if final_newline(&ours, &theirs) && !out.is_empty() {
        text.push_str(end);
    }
    Ok(Merged { text, unions })
}

/// Why the three texts' line ends are not one kind: a text that mixes
/// `\r\n` and `\n`, or texts that use different ones. A text of one line
/// with no line end has none to differ.
pub(crate) fn line_ends_differ(
    base: &Lines<'_>,
    ours: &Lines<'_>,
    theirs: &Lines<'_>,
) -> Option<&'static str> {
    let named = [("the base", base), ("ours", ours), ("theirs", theirs)];
    if let Some((name, _)) = named.iter().find(|(_, text)| text.mixed) {
        return Some(match *name {
            "the base" => "the base mixes CRLF and LF line ends",
            "ours" => "ours mixes CRLF and LF line ends",
            _ => "theirs mixes CRLF and LF line ends",
        });
    }
    let kinds: Vec<bool> = named
        .iter()
        .filter(|(_, text)| text.lines.len() > 1 || text.ends_with_newline)
        .map(|(_, text)| text.crlf)
        .collect();
    (kinds.iter().any(|crlf| *crlf) && kinds.iter().any(|crlf| !*crlf))
        .then_some("the sides use different line ends (CRLF and LF)")
}

/// Whether the merged text ends with a line end: when either side's does.
pub(crate) fn final_newline(ours: &Lines<'_>, theirs: &Lines<'_>) -> bool {
    ours.ends_with_newline || theirs.ends_with_newline
}

/// A side's lines for the base's `start..end`: shifted by what the side
/// grew before it (`shift`) and within it (`grew`).
fn span<'t>(text: &Lines<'t>, start: usize, end: usize, shift: isize, grew: isize) -> Vec<&'t str> {
    let from = (start as isize + shift) as usize;
    let to = (end as isize + shift + grew) as usize;
    text.lines[from..to].to_vec()
}

/// A side's changes at a place, its text, and its text for the place.
type Side<'s, 't> = (&'s [Hunk], &'s Lines<'t>, &'s Vec<&'t str>);

/// When one side's only change at the place `start..end` is lines it added
/// at the place's end, just after lines the other side changed: those
/// lines, after the other side's text for the place. git stops on an
/// addition next to a changed line (a row added after a row the other side
/// reworded); it is an addition all the same, and the other's change is
/// kept whole.
///
/// Refused (`None`): an addition before the other side's change, and an
/// addition beside lines the other side deleted, or deleted some of. What
/// was added above an item (an attribute, `#[cfg(windows)]`, `#[test]`, a
/// doc comment) or after lines (a `let` that uses them) belongs to those
/// lines; with them gone, or changed under it, it would attach to whatever
/// comes next, and every check would pass.
fn beside<'t>(
    ours: Side<'_, 't>,
    theirs: Side<'_, 't>,
    start: usize,
    end: usize,
) -> Option<(Vec<&'t str>, Vec<&'t str>)> {
    let addition = |side: &Side<'_, 't>| match side.0 {
        // A change that starts at the place's end would end past it: one
        // that starts there is an addition.
        [hunk] if hunk.a_start == end && hunk.a_start > start => {
            Some(side.1.lines[hunk.b_start..hunk.b_end].to_vec())
        }
        _ => None,
    };
    // Every change of the other side there replaces lines with lines.
    let replaces = |side: &Side<'_, 't>| {
        side.0.iter().all(|hunk| {
            !hunk.is_addition() && hunk.b_end - hunk.b_start >= hunk.a_end - hunk.a_start
        })
    };
    if let Some(added) = addition(&theirs).filter(|_| replaces(&ours)) {
        Some((ours.2.clone(), added))
    } else {
        let added = addition(&ours).filter(|_| replaces(&theirs))?;
        Some((theirs.2.clone(), added))
    }
}
