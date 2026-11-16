//! `cargo xtask mutate --check --fix-in`: where a break's code is now.
//!
//! A refactor that renames a function, or moves code into another, leaves a
//! break whose `in:` names an item that no longer holds its `find`. The
//! break says so, and says nothing of where the code went. This looks for
//! it: the item that holds the `find` now.
//!
//! It offers a place only when there is one. A `find` that is nowhere in its
//! file (the code is gone, or is in another file), in more than one place,
//! or in no item at all is not guessed at: the message says why, and the
//! break is left for a person to correct.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use super::lex;
use super::list::{self, Break};
use crate::Result;

/// What was found for one break whose `in:` no longer holds its code.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Fix {
    pub(crate) id: String,
    pub(crate) list: String,
    pub(crate) line: usize,
    pub(crate) file: String,
    /// The `in:` it has.
    pub(crate) old: String,
    /// The `in:` to use, with the line of the code, or why none is offered.
    pub(crate) found: std::result::Result<Found, String>,
}

/// An `in:` that holds the code.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) anchor: String,
    /// The line of the code in its file.
    pub(crate) line: usize,
}

impl Fix {
    /// What to tell a person: the break, what is wrong with its `in:`, and
    /// the line to use, or why there is none.
    pub(crate) fn describe(&self) -> String {
        let head = format!(
            "{}:{}: break `{}`: its `in: {}` no longer holds its `find` in `{}`.",
            self.list, self.line, self.id, self.old, self.file
        );
        match &self.found {
            Ok(found) => format!(
                "{head}\n  The code is now at line {} of `{}`. Use:\n      in: {}",
                found.line, self.file, found.anchor
            ),
            Err(why) => format!("{head}\n  No `in:` is offered: {why}"),
        }
    }
}

/// The breaks whose `in:` no longer holds their code, each with the `in:`
/// that does, or the reason there is none. A break that applies, or has no
/// `in:`, or fails for another reason (its `find` is there twice in the
/// item it names), is not one of them.
pub(crate) fn propose_all(root: &Path, breaks: &[Break]) -> Vec<Fix> {
    // Each file's text and tokens, read once.
    let mut files: BTreeMap<&str, Option<(String, Vec<lex::Token>)>> = BTreeMap::new();
    let mut fixes = Vec::new();
    for one in breaks {
        if one.anchor.is_none() {
            continue;
        }
        let read = files.entry(one.file.as_str()).or_insert_with(|| {
            let source = fs::read_to_string(root.join(&one.file)).ok()?;
            let code = lex::code_tokens(&source);
            let tokens = lex::without_layout_commas(&source, &code);
            Some((source, tokens))
        });
        let Some((source, tokens)) = read else {
            continue;
        };
        if let Some(fix) = propose(one, source, tokens) {
            fixes.push(fix);
        }
    }
    fixes
}

/// [`propose_all`], for one break and the source it changes.
pub(crate) fn propose(one: &Break, source: &str, tokens: &[lex::Token]) -> Option<Fix> {
    let anchor = one.anchor.as_deref()?;
    let located = one.locate_with_items(source, tokens, Some(anchor));
    match located {
        Err(
            lex::Missing::NoSuchItem { .. }
            | lex::Missing::NotFound
            | lex::Missing::NoSuchOccurrence { .. },
        ) => {}
        // It applies, or its trouble is not where it is.
        _ => return None,
    }
    Some(Fix {
        id: one.id.clone(),
        list: one.list.clone(),
        line: one.line,
        file: one.file.clone(),
        old: anchor.to_owned(),
        found: look(one, source, tokens),
    })
}

/// Where the code of `one` is in `source`, as an `in:` that names that
/// place and no other.
fn look(one: &Break, source: &str, tokens: &[lex::Token]) -> std::result::Result<Found, String> {
    let places = one.find_everywhere(source, tokens);
    let range = match places.as_slice() {
        [] => {
            return Err(format!(
                "its `find` is nowhere in `{}`: the code is gone, or it moved to another file \
                 (then change `file:`).",
                one.file
            ));
        }
        [only] => only.clone(),
        several => {
            let described: Vec<String> = several
                .iter()
                .map(|range| {
                    let chain = lex::enclosing_items(source, tokens, range.start);
                    let line = lex::line_of(source, range.start);
                    match chain.last() {
                        Some(item) => format!("line {line} (in `{}`)", item.name),
                        None => format!("line {line} (in no item)"),
                    }
                })
                .collect();
            return Err(format!(
                "its `find` is now in {} places, {}. Say which is meant, with a longer `find` or \
                 an `in:` and `occurrence:` by hand.",
                several.len(),
                described.join(", ")
            ));
        }
    };
    if one.occurrence.is_some_and(|asked| asked != 1) {
        return Err(
            "it asks for an `occurrence` above 1, and its `find` is in one place now: correct \
             `occurrence:` by hand."
                .to_owned(),
        );
    }
    let line = lex::line_of(source, range.start);
    let chain = lex::enclosing_items(source, tokens, range.start);
    if chain.is_empty() {
        return Err(format!(
            "its `find` is at line {line}, in no item (the top of the file). Leave `in:` out."
        ));
    }
    // The innermost item's name; then with the item around it, and the one
    // around that, until the name picks out this one item and this place.
    for names in 1..=chain.len() {
        let candidate = chain[chain.len() - names..]
            .iter()
            .map(|item| item.name.as_str())
            .collect::<Vec<_>>()
            .join("::");
        let located = one.locate_with_items(source, tokens, Some(&candidate));
        if let Ok((found, items)) = located
            && found == range
            && items.len() == 1
        {
            return Ok(Found {
                anchor: candidate,
                line,
            });
        }
    }
    Err(format!(
        "its `find` is at line {line}, in `{}`, and no name of that item and the ones around it \
         picks it out from the others of the file.",
        chain.last().map_or("", |item| item.name.as_str())
    ))
}

/// Rewrites the `in:` of each break that has an `in:` to offer, in the lists
/// that hold them: those lines, and nothing else. Returns how many.
pub(crate) fn write(root: &Path, fixes: &[Fix]) -> Result<usize> {
    let mut by_list: BTreeMap<&str, Vec<(&Fix, &Found)>> = BTreeMap::new();
    for fix in fixes {
        if let Ok(found) = &fix.found {
            by_list
                .entry(fix.list.as_str())
                .or_default()
                .push((fix, found));
        }
    }
    let mut written = 0;
    for (list, mut fixes) in by_list {
        let path = root.join(list);
        let mut text = fs::read_to_string(&path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        // From the end of the file: a value of several lines becomes one
        // line, and the lines after it move up.
        fixes.sort_by_key(|(fix, _)| std::cmp::Reverse(fix.line));
        for (fix, found) in fixes {
            let Some(rewritten) = list::with_anchor(&text, fix.line, &found.anchor) else {
                return Err(format!(
                    "{}:{}: no `in:` of break `{}` was found to rewrite",
                    fix.list, fix.line, fix.id
                ));
            };
            text = rewritten;
            written += 1;
        }
        fs::write(&path, text)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::super::drive::tests::tree;
    use super::super::lex;
    use super::super::list::{parse, with_anchor};
    use super::{Break, Found, propose, propose_all, write};

    /// A file after a refactor: `limit` became `ceiling`, `Budget::new`
    /// moved into `Budget::with`, and two items hold one line.
    const SOURCE: &str = "\
pub fn ceiling(seconds: u64) -> u64 {
    clamp(seconds, 1, 600)
}

pub struct Budget;

impl Budget {
    pub fn with(seconds: u64) -> Self {
        let kept = seconds.max(1);
        Budget::of(kept)
    }

    pub fn other(seconds: u64) -> u64 {
        seconds.max(2)
    }
}

mod first {
    pub fn same() -> u8 { 7 }
}

mod second {
    pub fn same() -> u8 { 7 }
}

fn top() { work(); }
";

    fn one(anchor: &str, find: &str, extra: &str) -> Break {
        let anchor = if anchor.is_empty() {
            String::new()
        } else {
            format!("in: {anchor}\n")
        };
        parse(
            "mutations/demo.breaks",
            &format!(
                "[break] b\nrule: r\nfile: crates/demo/src/lib.rs\n{anchor}find: {find}\nreplace: 0\n{extra}"
            ),
        )
        .unwrap()
        .remove(0)
    }

    fn fixed(anchor: &str, find: &str) -> Option<Result<String, String>> {
        propose(&one(anchor, find, ""), SOURCE, &lex::tokens(SOURCE)).map(|fix| {
            fix.found.map(|found| {
                assert!(found.line > 0);
                found.anchor
            })
        })
    }

    #[test]
    fn a_renamed_function_is_found_by_its_code() {
        assert_eq!(
            fixed("limit", "clamp(seconds, 1, 600)"),
            Some(Ok("ceiling".to_owned()))
        );
    }

    #[test]
    fn code_moved_into_a_method_is_named_by_the_method() {
        assert_eq!(
            fixed("Budget::new", "Budget::of(kept)"),
            Some(Ok("with".to_owned()))
        );
        assert_eq!(
            fixed("gone", "seconds.max(2)"),
            Some(Ok("other".to_owned()))
        );
    }

    #[test]
    fn a_break_that_applies_or_has_no_in_is_left_alone() {
        assert_eq!(fixed("ceiling", "clamp(seconds, 1, 600)"), None);
        assert_eq!(
            propose(
                &one("", "clamp(seconds, 1, 600)", ""),
                SOURCE,
                &lex::tokens(SOURCE)
            ),
            None
        );
    }

    #[test]
    fn code_in_several_items_is_not_guessed() {
        let error = fixed("gone", "pub fn same() -> u8 { 7 }")
            .unwrap()
            .unwrap_err();

        assert!(error.contains("now in 2 places"), "{error}");
        assert!(
            error.contains("line 19 (in `first`)") && error.contains("line 23 (in `second`)"),
            "{error}"
        );
    }

    #[test]
    fn code_that_is_nowhere_is_not_guessed() {
        let error = fixed("gone", "clamp(seconds, 9, 9)").unwrap().unwrap_err();

        assert!(
            error.contains("nowhere in `crates/demo/src/lib.rs`"),
            "{error}"
        );
    }

    #[test]
    fn code_in_no_item_is_told_to_leave_in_out() {
        let source = "fn a() {}\nwork_at_top();\n";
        let fix = propose(
            &one("gone", "work_at_top();", ""),
            source,
            &lex::tokens(source),
        )
        .unwrap();

        assert!(
            fix.found.unwrap_err().contains("in no item"),
            "an `in` was offered for code that is in no item"
        );
    }

    #[test]
    fn a_name_that_picks_out_two_items_gets_the_one_around_it() {
        // Two `new`s, one in each `impl`: `new` alone is two items.
        let source = "\
struct A;
struct B;
impl A { fn new() -> u8 { a_only() } fn other() { shared(); } }
impl B { fn new() -> u8 { b_only() } fn other() { shared(); } }
";
        let tokens = lex::tokens(source);
        let named = |find: &str| {
            propose(&one("gone", find, ""), source, &tokens)
                .unwrap()
                .found
        };

        assert_eq!(
            named("b_only()"),
            Ok(Found {
                anchor: "B::new".to_owned(),
                line: 4
            }),
            "`new` alone is two items, so the `impl` it is in is named"
        );
    }

    #[test]
    fn the_in_lines_are_rewritten_and_nothing_else_is() {
        let list = "\
# A comment.
[break] first
rule: r
file: crates/demo/src/lib.rs
in:
    Budget::new
find: Budget::of(kept)
replace: 0
caught-by:
    a::b

[break] second
in: limit
rule: r
file: crates/demo/src/lib.rs
find: clamp(seconds, 1, 600)
replace: 0
";
        let root = tree(
            "fix_write",
            &[
                ("crates/demo/src/lib.rs", SOURCE),
                ("mutations/demo.breaks", list),
            ],
        );
        let breaks = parse("mutations/demo.breaks", list).unwrap();

        let fixes = propose_all(&root, &breaks);
        let written = write(&root, &fixes).unwrap();
        let after = std::fs::read_to_string(root.join("mutations/demo.breaks")).unwrap();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(fixes.len(), 2);
        assert_eq!(written, 2);
        assert_eq!(
            after,
            list.replace("in: limit", "in: ceiling")
                .replace("in:\n    Budget::new", "in: with")
        );
    }

    #[test]
    fn an_in_is_replaced_in_its_own_break_and_with_its_own_line_ending() {
        let text = "[break] a\r\nin: x\r\nfind: 1\r\n\r\n[break] b\r\nin: y\r\n";

        assert_eq!(
            with_anchor(text, 5, "z").unwrap(),
            "[break] a\r\nin: x\r\nfind: 1\r\n\r\n[break] b\r\nin: z\r\n"
        );
        assert_eq!(
            with_anchor(text, 1, "z").unwrap(),
            "[break] a\r\nin: z\r\nfind: 1\r\n\r\n[break] b\r\nin: y\r\n"
        );
        // No `in:` in that break: the next break's is not taken.
        assert_eq!(
            with_anchor("[break] a\nfind: 1\n[break] b\nin: y\n", 1, "z"),
            None
        );
    }

    #[test]
    fn a_text_in_a_string_moved_into_another_item_is_found_by_its_text() {
        let source = "fn old() { say(\"too slow\"); }\nfn fresh() { say(\"too busy\"); }\n";
        let list = "[break] b\nrule: r\nfile: crates/demo/src/lib.rs\nin: old\nin-string: busy\nreplace: free\n";
        let one = parse("mutations/demo.breaks", list).unwrap().remove(0);

        let fix = propose(&one, source, &lex::tokens(source)).unwrap();

        assert_eq!(fix.found.map(|found| found.anchor), Ok("fresh".to_owned()));
        // Where it still is, nothing is offered.
        let list = list.replace("in: old", "in: fresh");
        let one = parse("mutations/demo.breaks", &list).unwrap().remove(0);
        assert_eq!(propose(&one, source, &lex::tokens(source)), None);
    }
}
