//! No word of the maintainers' private working setup in public text
//! (`docs/testing-and-ci.md`, "Public text", since the twelfth tooling
//! step).
//!
//! The docs were made public-facing once (pull request #106): the machines
//! the maintainers build on, their scripts and the way they run sessions
//! went into their own private notes, and the docs say the general thing.
//! Sessions that work from those notes wrote the words back in. This rule
//! fails on them, with what to say instead.
//!
//! - **The words** are in [`LIST`], a data file, so that a change is one
//!   line: each phrase, in any case or in exactly its case, and the public
//!   alternative the message names.
//! - **Public text** is every Markdown file and every workflow, except the
//!   step records written before the rule (the list's `history` lines,
//!   which name their folders). A step record is public text: from the
//!   twelfth tooling step on, it is held to the rule too.
//! - **A product use** of a word (the Desktop column of the budgets) is a
//!   longer phrase on an `allow` line: a hit inside it passes. `AGENTS.md`,
//!   which every agent session loads, takes no `allow`: it names none of the
//!   words at all.
//!
//! This is not the owner guard (`cargo xtask owner-guard`), which looks for
//! the owner's own details, from a file outside the repository. These words
//! are not secret; they are only not the product's.

use std::collections::{BTreeMap, BTreeSet};

use super::Violation;

/// The data file the rule reads, from the repository's root.
pub(crate) const LIST: &str = "xtask/public-words.txt";

/// The one page that takes no `allow`.
const STRICT: &str = "AGENTS.md";

/// The step records' folder.
const STEPS: &str = "docs/steps/";

/// The list, read.
#[derive(Debug, Default)]
pub(crate) struct Words {
    words: Vec<Word>,
    /// Phrases that excuse a hit inside them, in lower case.
    allowed: Vec<String>,
    /// Folders of step records written before the rule, each ending in `/`.
    history: Vec<String>,
}

#[derive(Debug)]
struct Word {
    phrase: String,
    /// Matched in exactly its case; otherwise in any (ASCII) case.
    cased: bool,
    /// What public text says instead.
    say: String,
}

/// Reads the list. A line it cannot read is a violation of the list's own,
/// so that a typo is not a word quietly dropped.
pub(crate) fn parse(text: &str) -> (Words, Vec<Violation>) {
    let mut words = Words::default();
    let mut problems = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (kind, rest) = line.split_once(' ').unwrap_or((line, ""));
        let read = match kind {
            "word" | "cased" => rest.split_once(" => ").and_then(|(phrase, say)| {
                let phrase = phrase.trim();
                (!phrase.is_empty() && !say.trim().is_empty()).then(|| {
                    words.words.push(Word {
                        phrase: phrase.to_owned(),
                        cased: kind == "cased",
                        say: say.trim().to_owned(),
                    });
                })
            }),
            "allow" => rest.split_once(" -- ").and_then(|(phrase, reason)| {
                let phrase = phrase.trim();
                (!phrase.is_empty() && !reason.trim().is_empty())
                    .then(|| words.allowed.push(phrase.to_ascii_lowercase()))
            }),
            "history" => rest.split_once(" -- ").and_then(|(folder, reason)| {
                let folder = folder.trim();
                (folder.starts_with(STEPS) && folder.ends_with('/') && !reason.trim().is_empty())
                    .then(|| words.history.push(folder.to_owned()))
            }),
            _ => None,
        };
        if read.is_none() {
            problems.push(Violation::new(
                LIST,
                Some(index + 1),
                "a line this rule cannot read. It is `word PHRASE => ALTERNATIVE`, `cased \
                 PHRASE => ALTERNATIVE`, `allow PHRASE -- REASON` or `history docs/steps/FOLDER/ \
                 -- REASON` (the top of the file says what each does).",
            ));
        }
    }
    (words, problems)
}

/// Whether a file is public text the rule reads, before the `history`
/// folders are taken out.
pub(crate) fn is_read(file: &str) -> bool {
    file.ends_with(".md")
        || (file.starts_with(".github/workflows/")
            && (file.ends_with(".yml") || file.ends_with(".yaml")))
}

/// Checks every file of `texts` that is public text. `files` are the
/// repository's files: a `history` folder with none in it is stale.
pub(crate) fn check(
    texts: &BTreeMap<String, String>,
    words: &Words,
    files: &BTreeSet<String>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    for folder in &words.history {
        if !files.iter().any(|file| file.starts_with(folder.as_str())) {
            violations.push(Violation::new(
                LIST,
                None,
                format!("`history {folder}` names a folder with no file in it. Take the line out."),
            ));
        }
    }
    for (file, text) in texts {
        if !is_read(file)
            || words
                .history
                .iter()
                .any(|folder| file.starts_with(folder.as_str()))
        {
            continue;
        }
        let strict = file == STRICT;
        for (index, line) in text.lines().enumerate() {
            for (word, _) in hits(line, words, strict) {
                let excuse = if strict {
                    format!(
                        "{STRICT}, which every agent session loads, names none of these words, \
                         not even in a product use."
                    )
                } else {
                    format!(
                        "If this is the product's own use of the word, add the longer phrase as an \
                         `allow` line to {LIST}."
                    )
                };
                violations.push(Violation::new(
                    file.clone(),
                    Some(index + 1),
                    format!(
                        "`{}` is a word of the maintainers' private setup, not of the product. \
                         Say {} instead. {excuse}",
                        word.phrase, word.say
                    ),
                ));
            }
        }
    }
    violations
}

/// The words found in `line`, each with where it starts: one hit for a
/// place two words cover (`at the desk` and `the desk`), the longer one.
fn hits<'a>(line: &str, words: &'a Words, strict: bool) -> Vec<(&'a Word, usize)> {
    let lower = line.to_ascii_lowercase();
    let mut found: Vec<(usize, usize, &Word)> = Vec::new();
    for word in &words.words {
        let (haystack, needle) = if word.cased {
            (line, word.phrase.clone())
        } else {
            (lower.as_str(), word.phrase.to_ascii_lowercase())
        };
        for (start, _) in haystack.match_indices(needle.as_str()) {
            let end = start + needle.len();
            if !whole(line, start, end, &needle) {
                continue;
            }
            if !strict && allowed(&lower, start, end, &words.allowed) {
                continue;
            }
            found.push((start, end, word));
        }
    }
    // Longest first at each place, then drop what an earlier hit covers.
    found.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    let mut kept: Vec<(usize, usize, &Word)> = Vec::new();
    for hit in found {
        if kept
            .iter()
            .any(|&(start, end, _)| start <= hit.0 && hit.1 <= end)
        {
            continue;
        }
        kept.push(hit);
    }
    kept.into_iter()
        .map(|(start, _, word)| (word, start))
        .collect()
}

/// Whether the phrase at `start..end` stands as whole words at whichever of
/// its ends is a letter or digit.
fn whole(line: &str, start: usize, end: usize, phrase: &str) -> bool {
    let word_char = |c: char| c.is_alphanumeric() || c == '_';
    let first = phrase.chars().next().is_some_and(word_char);
    let last = phrase.chars().next_back().is_some_and(word_char);
    let before = line[..start].chars().next_back();
    let after = line[end..].chars().next();
    (!first || !before.is_some_and(word_char)) && (!last || !after.is_some_and(word_char))
}

/// Whether an `allow` phrase covers `start..end` of the line.
fn allowed(lower: &str, start: usize, end: usize, allowed: &[String]) -> bool {
    allowed.iter().any(|phrase| {
        lower
            .match_indices(phrase.as_str())
            .any(|(at, _)| at <= start && end <= at + phrase.len())
    })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::{check, parse};

    const LIST: &str = "\
# a comment
word the desk => locally
word at the desk => locally
cased the coordinator => the maintainers
word `rx  => the plain `cargo xtask` command
word the desktop => the reference machine
allow the desktop number -- the budgets' column
history docs/steps/08/ -- before the rule
";

    fn run(file: &str, text: &str) -> Vec<String> {
        let (words, problems) = parse(LIST);
        assert!(problems.is_empty(), "{problems:?}");
        let texts = BTreeMap::from([(file.to_owned(), text.to_owned())]);
        let files = BTreeSet::from([file.to_owned(), "docs/steps/08/ci.md".to_owned()]);
        check(&texts, &words, &files)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// A planted word fails, named with its file and line and the public
    /// alternative; a clean page passes.
    #[test]
    fn a_word_of_the_setup_in_a_public_page_fails_with_what_to_say() {
        let found = run("docs/x.md", "Fine.\nIt took 3 s at the desk.\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("docs/x.md:2: `at the desk` is a word"),
            "{found:?}"
        );
        assert!(found[0].contains("Say locally instead"), "{found:?}");
        assert!(run("docs/x.md", "It took 3 s locally.\n").is_empty());
    }

    /// Whole words only: `the desk` is not in `the desktop`, and the case
    /// of a `cased` word decides: the product's Coordinator passes.
    #[test]
    fn words_are_whole_and_a_cased_word_keeps_its_case() {
        assert!(run("docs/x.md", "the desktops of users\n").is_empty());
        assert!(run("docs/x.md", "The Coordinator runs a team.\n").is_empty());
        assert_eq!(run("docs/x.md", "Ask the coordinator's answer.\n").len(), 1);
        assert_eq!(run("docs/x.md", "Run `rx test`.\n").len(), 1);
        assert_eq!(run("docs/x.md", "AT THE DESK\n").len(), 1);
    }

    /// An `allow` phrase excuses the hit inside it, and only that one; in
    /// AGENTS.md it excuses nothing.
    #[test]
    fn an_allowed_phrase_passes_except_in_agents_md() {
        assert!(run("docs/x.md", "the desktop number is what users feel\n").is_empty());
        assert_eq!(
            run("docs/x.md", "the desktop number, on the desktop\n").len(),
            1
        );
        let strict = run("AGENTS.md", "the desktop number\n");
        assert_eq!(strict.len(), 1, "{strict:?}");
        assert!(
            strict[0].contains("names none of these words"),
            "{strict:?}"
        );
    }

    /// The step records before the rule keep their words; a newer one, the
    /// steps' README and a workflow are held to it. Other files are not
    /// read.
    #[test]
    fn old_step_records_keep_their_words_and_new_ones_do_not() {
        assert!(run("docs/steps/08/ci.md", "at the desk\n").is_empty());
        assert_eq!(run("docs/steps/tooling-12/ci.md", "at the desk\n").len(), 1);
        assert_eq!(run("docs/steps/README.md", "at the desk\n").len(), 1);
        assert_eq!(run(".github/workflows/ci.yml", "# at the desk\n").len(), 1);
        assert!(run("crates/a/src/lib.rs", "// at the desk\n").is_empty());
    }

    /// A line the list cannot read, and a `history` folder with nothing in
    /// it, are each a violation of the list.
    #[test]
    fn a_bad_line_or_a_stale_folder_in_the_list_fails() {
        let (_, problems) = parse("word no arrow here\nallow nothing\nhistory docs/x/ -- why\n");
        assert_eq!(problems.len(), 3, "{problems:?}");
        let (words, problems) = parse("history docs/steps/99/ -- before\n");
        assert!(problems.is_empty());
        let found = check(&BTreeMap::new(), &words, &BTreeSet::new());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].message.contains("no file in it"));
    }

    /// The real list reads without a problem.
    #[test]
    fn the_list_in_the_repository_reads() {
        let text = std::fs::read_to_string(crate::workspace::root().join(super::LIST)).unwrap();
        let (words, problems) = parse(&text);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(!words.words.is_empty());
    }
}
