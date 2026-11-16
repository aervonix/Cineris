//! `cargo xtask decision new "TITLE"`: the next numbered decision record in
//! `docs/decisions/`, written from the format the index gives, and its row
//! added to the index (`docs/decisions/README.md`, "Format of a record").
//!
//! A record was written by hand: the number found by listing the folder, the
//! file named, the headings copied, the row added to the table. A slip in
//! any of them is found by `links` at best. This does all four.

use std::fs;
use std::path::Path;

use crate::Result;

const USAGE: &str = "\
usage: cargo xtask decision new \"TITLE\" [--status TEXT] [--dry-run]

Writes docs/decisions/NNNN-SLUG.md, NNNN the number after the highest record's,
SLUG the title in lower-case words joined by hyphens, with the headings of the
format in docs/decisions/README.md, the status (\"Provisional\" when --status is
not given) and today's date (UTC); and adds the record's row to the index's
table, after its last row. --dry-run prints what it would write.

The title is the decision in one line, as the index lists it. A parallel step
may take the same number: the later of the two renumbers its record when it
rebases.";

/// The folder of the records, from the repository's root.
const FOLDER: &str = "docs/decisions";

/// The status a record is given when none is.
const PROVISIONAL: &str = "Provisional";

/// The title in lower-case words joined by hyphens: letters and digits kept,
/// anything else a break between words.
fn slug(title: &str) -> String {
    title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

/// The number after the highest of the records named in `names`
/// (`NNNN-….md`).
fn next_number(names: &[String]) -> u32 {
    names
        .iter()
        .filter(|name| {
            Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "md")
        })
        .filter_map(|name| name.split_once('-')?.0.parse::<u32>().ok())
        .max()
        .map_or(1, |highest| highest + 1)
}

/// The record's text.
fn record(number: u32, title: &str, status: &str, date: &str) -> String {
    format!(
        "# {number:04}: {title}\n\
         \n\
         - **Status:** {status}\n\
         - **Date:** {date}\n\
         \n\
         ## Context\n\
         \n\
         What forced a choice.\n\
         \n\
         ## Options\n\
         \n\
         What was considered, with measurements where they exist.\n\
         \n\
         ## Decision\n\
         \n\
         What was chosen.\n\
         \n\
         ## Why\n\
         \n\
         The reasons, in order of weight.\n\
         \n\
         ## What is given up\n\
         \n\
         Where it matters; otherwise delete this section.\n\
         \n\
         ## Revisit when\n\
         \n\
         The evidence that would reopen it.\n"
    )
}

/// The index with the record's row after the last row of its first table
/// (the rows begin `| [NNNN]`), or why it cannot be added.
fn indexed(index: &str, number: u32, file: &str, title: &str, status: &str) -> Result<String> {
    let lines: Vec<&str> = index.split_inclusive('\n').collect();
    let last = lines
        .iter()
        .rposition(|line| line.starts_with("| [0") || line.starts_with("| [1"))
        .ok_or_else(|| {
            format!(
                "{FOLDER}/README.md has no row of a record (`| [NNNN](…) | … |`) to add one after"
            )
        })?;
    // The title is a cell: a `|` in it would end the cell.
    let cell = title.replace('|', "\\|");
    let row = format!("| [{number:04}]({file}) | {cell} | {status} |\n");
    let mut text = String::with_capacity(index.len() + row.len());
    for (at, line) in lines.iter().enumerate() {
        text.push_str(line);
        if at == last {
            if !line.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&row);
        }
    }
    Ok(text)
}

/// Today's date, `YYYY-MM-DD`, in UTC.
fn today() -> String {
    crate::last_good::utc(crate::last_good::now())
        .split(' ')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// What `new` would write: the record's file name and text, and the index.
fn plan(
    names: &[String],
    index: &str,
    title: &str,
    status: &str,
    date: &str,
) -> Result<(String, String, String)> {
    let title = title.trim();
    let slug = slug(title);
    if slug.is_empty() {
        return Err(format!(
            "the title has no letter or digit to name the file by\n\n{USAGE}"
        ));
    }
    if title.contains('\n') {
        return Err("the title is one line".to_owned());
    }
    let number = next_number(names);
    let file = format!("{number:04}-{slug}.md");
    if names
        .iter()
        .any(|name| name.ends_with(&format!("-{slug}.md")))
    {
        return Err(format!(
            "a record named `…-{slug}.md` is there already: give the decision a title of its own"
        ));
    }
    let index = indexed(index, number, &file, title, status)?;
    Ok((file, record(number, title, status, date), index))
}

/// `cargo xtask decision …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let mut words = arguments.iter();
    match words.next().map(String::as_str) {
        Some("new") => {}
        Some("--help" | "-h") | None => return Err(USAGE.to_owned()),
        Some(other) => return Err(format!("`{other}`: the command is `new`\n\n{USAGE}")),
    }
    let (mut title, mut status, mut dry_run) = (None, PROVISIONAL.to_owned(), false);
    while let Some(word) = words.next() {
        match word.as_str() {
            "--dry-run" => dry_run = true,
            "--status" => {
                status = words
                    .next()
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| format!("`--status` needs a value\n\n{USAGE}"))?
                    .clone();
            }
            "--help" | "-h" => return Err(USAGE.to_owned()),
            flag if flag.starts_with("--") => {
                return Err(format!("unknown flag `{flag}`\n\n{USAGE}"));
            }
            text if title.is_none() => title = Some(text.to_owned()),
            extra => {
                return Err(format!(
                    "one title, quoted: `{extra}` came after it\n\n{USAGE}"
                ));
            }
        }
    }
    let title = title.ok_or_else(|| format!("which decision? Give its title\n\n{USAGE}"))?;
    let folder = crate::workspace::root().join(FOLDER);
    let names: Vec<String> = fs::read_dir(&folder)
        .map_err(|error| format!("could not list {FOLDER}: {error}"))?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    let index_path = folder.join("README.md");
    let index = fs::read_to_string(&index_path)
        .map_err(|error| format!("could not read {FOLDER}/README.md: {error}"))?;
    let (file, text, index) = plan(&names, &index, &title, &status, &today())?;
    if dry_run {
        println!("would write {FOLDER}/{file}:\n\n{text}");
        println!("and add to {FOLDER}/README.md the row of {file}");
        return Ok(());
    }
    fs::write(folder.join(&file), text)
        .map_err(|error| format!("could not write {FOLDER}/{file}: {error}"))?;
    fs::write(&index_path, index)
        .map_err(|error| format!("could not write {FOLDER}/README.md: {error}"))?;
    println!("wrote {FOLDER}/{file}, and its row in {FOLDER}/README.md");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    const INDEX: &str = "\
# Decision records

| # | Decision | Status |
|---|---|---|
| [0001](0001-a.md) | A | Accepted |
| [0036](0036-b.md) | B | Accepted |

The owner's answers.
";

    #[test]
    fn the_title_names_the_file_in_lower_case_words() {
        assert_eq!(
            slug("The `try` workflow's A/B: two refs, one runner!"),
            "the-try-workflow-s-a-b-two-refs-one-runner"
        );
        assert_eq!(slug("  Ünïcode —  "), "n-code");
        assert_eq!(slug("!!!"), "");
    }

    #[test]
    fn the_number_is_the_one_after_the_highest_record() {
        assert_eq!(
            next_number(&names(&[
                "0001-a.md",
                "0036-b.md",
                "0009-c.md",
                "README.md"
            ])),
            37
        );
        assert_eq!(next_number(&names(&["README.md", "0100-x.txt"])), 1);
        assert_eq!(next_number(&names(&["0999-z.md"])), 1000);
    }

    #[test]
    fn a_record_is_written_with_every_heading_of_the_format_and_its_row_after_the_last() {
        let (file, text, index) = plan(
            &names(&["0001-a.md", "0036-b.md", "README.md"]),
            INDEX,
            "  Two refs on one runner  ",
            PROVISIONAL,
            "2026-11-16",
        )
        .unwrap();
        assert_eq!(file, "0037-two-refs-on-one-runner.md");
        assert!(text.starts_with(
            "# 0037: Two refs on one runner\n\n- **Status:** Provisional\n- **Date:** 2026-11-16\n"
        ));
        for heading in [
            "## Context",
            "## Options",
            "## Decision",
            "## Why",
            "## What is given up",
            "## Revisit when",
        ] {
            assert!(text.contains(&format!("\n{heading}\n")), "{heading}");
        }
        assert_eq!(
            index,
            INDEX.replace(
                "| [0036](0036-b.md) | B | Accepted |\n",
                "| [0036](0036-b.md) | B | Accepted |\n\
                 | [0037](0037-two-refs-on-one-runner.md) | Two refs on one runner | Provisional |\n"
            )
        );
    }

    #[test]
    fn a_pipe_in_the_title_does_not_end_its_cell() {
        let (_, _, index) = plan(
            &names(&["0001-a.md"]),
            INDEX,
            "A | B",
            "Accepted",
            "2026-11-16",
        )
        .unwrap();
        assert!(index.contains("| [0002](0002-a-b.md) | A \\| B | Accepted |\n"));
    }

    #[test]
    fn a_title_that_names_nothing_or_a_record_already_there_is_refused() {
        assert!(plan(&names(&[]), INDEX, "?!", PROVISIONAL, "d").is_err());
        let error = plan(&names(&["0036-b.md"]), INDEX, "b", PROVISIONAL, "d").unwrap_err();
        assert!(error.contains("there already"), "{error}");
        assert!(plan(&names(&[]), "no table\n", "x", PROVISIONAL, "d").is_err());
    }

    #[test]
    fn its_words_are_checked() {
        for (words, says) in [
            (&[][..], "usage"),
            (&["old"][..], "the command is `new`"),
            (&["new"][..], "which decision"),
            (&["new", "a", "b"][..], "one title"),
            (&["new", "a", "--status"][..], "needs a value"),
            (&["new", "a", "--force"][..], "unknown flag"),
        ] {
            let error = run(&names(words)).unwrap_err();
            assert!(error.contains(says), "{words:?}: {error}");
        }
    }

    #[test]
    fn today_is_a_date() {
        let today = today();
        assert_eq!(today.len(), 10, "{today}");
        assert_eq!(today.matches('-').count(), 2, "{today}");
    }
}
