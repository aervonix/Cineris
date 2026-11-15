//! `cargo xtask replace`: an edit of Rust code that finds its place by
//! tokens, not text, so that it still finds it after `rustfmt` has laid the
//! code out again (`CONTRIBUTING.md`, "Editing from a script").
//!
//! Edit scripts that matched exact text broke whenever a line was wrapped,
//! and a script that fails half way leaves a tree half edited. This uses
//! the matching of `cargo xtask mutate`: white space, comments and a comma
//! before a closing bracket are not tokens. The match must be there exactly
//! once (inside an item, with `--in`), or the command refuses and changes
//! nothing. It prints the lines it changed.
//!
//! `--text` is the same edit for Markdown and any other file that is not
//! Rust: the match is the exact text, which must be in the file exactly
//! once.

use std::fs;
use std::path::{Path, PathBuf};

use crate::Result;
use crate::mutate::lex;

const USAGE: &str = "\
usage: cargo xtask replace FILE (--from CODE | --from-file PATH)
                                (--to CODE | --to-file PATH)
                                [--in ITEM] [--occurrence N] [--dry-run]
       cargo xtask replace FILE --text (--from TEXT | --from-file PATH)
                                       (--to TEXT | --to-file PATH) [--dry-run]
       cargo xtask replace --script FILE [--dry-run]   (many edits, all or none:
                                                         `--script --help`)

Replaces the code that --from matches, by its tokens, with --to, as text.
White space and comments do not count in the match, so it holds however
rustfmt has laid the code out. The match must be in FILE exactly once, or
inside the item --in names (`Round::spawn`; an impl block answers to every
name in its header); --occurrence N picks one of several, counted from 1.
--dry-run prints what would change and writes nothing. Run `cargo xtask fmt`
afterwards: the new code is put in as it was given.

With --text, for Markdown and other files that are not Rust, --from is
matched as exact text, white space and all, and must be in FILE exactly once:
none, or more than one (overlapping ones included), changes nothing.";

/// What `replace` was asked to do.
#[derive(Debug, Default, PartialEq)]
struct Request {
    file: PathBuf,
    from: String,
    to: String,
    within: Option<String>,
    occurrence: Option<usize>,
    dry_run: bool,
    /// Exact text, not Rust tokens.
    text: bool,
}

impl Request {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Request::default();
        let (mut from, mut to, mut file) = (None, None, None);
        let mut rest = arguments.iter();
        while let Some(argument) = rest.next() {
            let mut value = || {
                rest.next()
                    .cloned()
                    .ok_or_else(|| format!("`{argument}` needs a value\n\n{USAGE}"))
            };
            match argument.as_str() {
                "--from" => from = Some(value()?),
                "--to" => to = Some(value()?),
                "--from-file" => from = Some(read(&value()?)?),
                "--to-file" => to = Some(read(&value()?)?),
                "--in" => request.within = Some(value()?),
                "--occurrence" => {
                    let text = value()?;
                    request.occurrence = Some(
                        text.parse()
                            .ok()
                            .filter(|number| *number > 0)
                            .ok_or_else(|| format!("`--occurrence {text}`: a number from 1"))?,
                    );
                }
                "--dry-run" => request.dry_run = true,
                "--text" => request.text = true,
                "--help" | "-h" => return Err(USAGE.to_owned()),
                flag if flag.starts_with('-') => {
                    return Err(format!("unknown flag `{flag}`\n\n{USAGE}"));
                }
                path if file.is_none() => file = Some(PathBuf::from(path)),
                extra => return Err(format!("one file at a time: `{extra}`\n\n{USAGE}")),
            }
        }
        request.file = file.ok_or_else(|| format!("which file?\n\n{USAGE}"))?;
        request.from = from.ok_or_else(|| format!("say what to replace with --from\n\n{USAGE}"))?;
        request.to = to.ok_or_else(|| format!("say what to put there with --to\n\n{USAGE}"))?;
        if request.text && (request.within.is_some() || request.occurrence.is_some()) {
            return Err(format!(
                "--text matches the text once in the whole file: --in and --occurrence are for \
                 Rust code\n\n{USAGE}"
            ));
        }
        Ok(request)
    }
}

fn read(path: &str) -> Result<String> {
    let text =
        fs::read_to_string(path).map_err(|error| format!("could not read {path}: {error}"))?;
    // A file written for the purpose ends with a line break that is not
    // part of the code.
    Ok(text.strip_suffix('\n').unwrap_or(&text).to_owned())
}

/// The edited text, and what to print about it, for `--text`: `from` must
/// be in `source` exactly once, overlapping occurrences counted.
fn edit_text(source: &str, request: &Request, shown_as: &str) -> Result<(String, String)> {
    if request.from.is_empty() {
        return Err("--from is empty".to_owned());
    }
    let starts: Vec<usize> = source
        .char_indices()
        .map(|(at, _)| at)
        .filter(|at| source[*at..].starts_with(&request.from))
        .collect();
    let range = match starts.as_slice() {
        [] => {
            return Err(format!(
                "--from is not in {shown_as} as text. Nothing changed."
            ));
        }
        [at] => *at..*at + request.from.len(),
        several => {
            return Err(format!(
                "--from is in {shown_as} {} times, at lines {}. Nothing changed. Make it longer.",
                several.len(),
                several
                    .iter()
                    .map(|at| lex::line_of(source, *at).to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    };
    if request.from == request.to {
        return Err("--to is the same text as --from. Nothing changed.".to_owned());
    }
    Ok(replaced(source, range, &request.to, shown_as))
}

/// `source` with `range` replaced by `to`, and the lines that changed.
fn replaced(
    source: &str,
    range: std::ops::Range<usize>,
    to: &str,
    shown_as: &str,
) -> (String, String) {
    let mut edited = String::with_capacity(source.len() + to.len());
    edited.push_str(&source[..range.start]);
    edited.push_str(to);
    edited.push_str(&source[range.end..]);

    // The whole lines the change touches, before and after.
    let first = lex::line_of(source, range.start);
    let last_before = lex::line_of(source, range.end);
    let last_after = lex::line_of(&edited, range.start + to.len());
    let lines = |text: &str, from: usize, to: usize, mark: char| {
        text.lines()
            .enumerate()
            .skip(from - 1)
            .take(to + 1 - from)
            .map(|(index, line)| format!("{mark} {:>5}  {line}\n", index + 1))
            .collect::<String>()
    };
    let shown = format!(
        "{shown_as}: lines {first}-{last_before} are now {first}-{last_after}\n{}{}",
        lines(source, first, last_before, '-'),
        lines(&edited, first, last_after, '+'),
    );
    (edited, shown)
}

/// The edited source, and what to print about it.
fn edit(source: &str, request: &Request, shown_as: &str) -> Result<(String, String)> {
    if request.text {
        return edit_text(source, request, shown_as);
    }
    // A match by tokens begins at the first token of --from and ends at its
    // last: a comment before or after them is passed over, so the file's
    // own comment there would stay, and --to's would be put in beside it.
    let own = lex::tokens(&request.from);
    if let (Some(first), Some(last)) = (own.first(), own.last())
        && (!request.from[..first.start].trim().is_empty()
            || !request.from[last.end..].trim().is_empty())
    {
        return Err(
            "--from begins or ends with a comment, which a match by tokens passes over: the \
             file's comment would stay and --to's be added beside it. Start and end --from \
             with code, or match it as text (`--text`, or `@@ text` in a script). Nothing \
             changed."
                .to_owned(),
        );
    }
    let tokens = lex::tokens(source);
    let inside = request
        .within
        .as_deref()
        .map(|item| format!(" inside `{item}`"))
        .unwrap_or_default();
    let range = lex::locate_in(
        source,
        &tokens,
        request.within.as_deref(),
        &request.from,
        request.occurrence,
    )
    .map_err(|missing| match missing {
        lex::Missing::NothingToFind => {
            "--from has no code in it: it is all comments and white space, which a \
                                         match by tokens passes over. Match it as text: `--text`, \
                                         or `@@ text` in a script."
                .to_owned()
        }
        lex::Missing::NoSuchItem { segment, within } => {
            if within.is_empty() {
                format!("{shown_as} has no item called `{segment}`")
            } else {
                format!("`{within}` in {shown_as} has no item called `{segment}`")
            }
        }
        lex::Missing::NotFound => match comma_before_a_bracket(source, &tokens, request) {
            Some(line) => format!(
                "--from ends in a comma, and in {shown_as} that comma (line {line}) comes before \
                 a closing bracket, where it is the formatter's and not a token; --from has it as \
                 a token, so it cannot match. End --from before the comma, and --to too. Nothing \
                 changed."
            ),
            None => format!("--from is not in {shown_as}{inside}. Nothing changed."),
        },
        lex::Missing::Ambiguous(lines) => format!(
            "--from is in {shown_as}{inside} {} times, at lines {}. Nothing changed. Make it \
             longer, name the item with --in, or pick one with --occurrence.",
            lines.len(),
            lines
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        lex::Missing::NoSuchOccurrence { asked, found } => format!(
            "--occurrence {asked}: --from is in {shown_as}{inside} {found} time(s). Nothing \
             changed."
        ),
    })?;
    if lex::same_tokens(&source[range.clone()], &request.to) {
        return Err("--to is the same code as what --from matched. Nothing changed.".to_owned());
    }
    Ok(replaced(source, range, &request.to, shown_as))
}

const SCRIPT_USAGE: &str = "\
usage: cargo xtask replace --script FILE [--dry-run]

Makes every edit of FILE, in order, or none of them. An edit is:

    @@ file: crates/cortex-tui/src/theme.rs
    @@ in: Theme::light          (optional, Rust only)
    @@ occurrence: 2             (optional, Rust only)
    @@ text                      (optional: exact text in a .rs file too)
    @@ from
    the code or text to find, its lines as they are
    @@ to
    what to put there
    @@ end

A `.rs` file is matched by tokens, as `replace FILE --from` does; any other
file, or one with `@@ text`, by exact text. Each edit's --from must be in its
file exactly once, as the edits before it left the file. If any edit cannot
be made, the command names it and writes nothing. Lines outside an edit that
begin with `#`, and empty ones, are comments.";

/// One edit of a script, and where it was written.
#[derive(Debug, PartialEq)]
struct ScriptEdit {
    /// Its number in the script, from 1, and its first line.
    number: usize,
    line: usize,
    request: Request,
}

/// Reads a script of edits ([`SCRIPT_USAGE`]).
fn parse_script(script: &str) -> Result<Vec<ScriptEdit>> {
    #[derive(PartialEq)]
    enum In {
        Between,
        Header,
        From,
        To,
    }
    let mut edits = Vec::new();
    let mut state = In::Between;
    let (mut from, mut to) = (Vec::<&str>::new(), Vec::<&str>::new());
    let mut current: Option<ScriptEdit> = None;
    let mut forced_text = false;
    for (index, line) in script.lines().enumerate() {
        let at = index + 1;
        let marker = line.strip_prefix("@@ ").map(str::trim_end);
        let fail = |what: &str| Err(format!("the script's line {at}: {what}\n\n{SCRIPT_USAGE}"));
        match (&state, marker) {
            (In::Between, None) => {
                if !(line.trim().is_empty() || line.starts_with('#')) {
                    return fail("text outside an edit: begin one with `@@ file: PATH`");
                }
            }
            (In::Between, Some(marker)) => {
                let Some(path) = marker.strip_prefix("file:").map(str::trim) else {
                    return fail("an edit begins with `@@ file: PATH`");
                };
                if path.is_empty() {
                    return fail("`@@ file:` names no file");
                }
                current = Some(ScriptEdit {
                    number: edits.len() + 1,
                    line: at,
                    request: Request {
                        file: PathBuf::from(path),
                        ..Request::default()
                    },
                });
                forced_text = false;
                state = In::Header;
            }
            (In::Header, Some(marker)) => {
                let Some(edit) = current.as_mut() else {
                    return fail("an edit has no `@@ file:` line");
                };
                if marker == "from" {
                    state = In::From;
                } else if marker == "text" {
                    forced_text = true;
                } else if let Some(item) = marker.strip_prefix("in:") {
                    edit.request.within = Some(item.trim().to_owned());
                } else if let Some(number) = marker.strip_prefix("occurrence:") {
                    edit.request.occurrence = Some(
                        number
                            .trim()
                            .parse()
                            .ok()
                            .filter(|number| *number > 0)
                            .ok_or_else(|| {
                                format!("the script's line {at}: an occurrence is a number from 1")
                            })?,
                    );
                } else {
                    return fail(&format!(
                        "`@@ {marker}`: an edit's header is `in:`, `occurrence:`, `text` and \
                         then `from`"
                    ));
                }
            }
            (In::Header, None) => return fail("an edit's header is followed by `@@ from`"),
            (In::From, Some("to")) => state = In::To,
            (In::To, Some("end")) => {
                let Some(mut edit) = current.take() else {
                    return fail("an edit has no `@@ file:` line");
                };
                edit.request.from = from.join("\n");
                edit.request.to = to.join("\n");
                let rust = edit
                    .request
                    .file
                    .extension()
                    .is_some_and(|extension| extension == "rs");
                edit.request.text = forced_text || !rust;
                if edit.request.text
                    && (edit.request.within.is_some() || edit.request.occurrence.is_some())
                {
                    return Err(format!(
                        "edit {} (the script's line {}): `in:` and `occurrence:` are for Rust \
                         code matched by its tokens",
                        edit.number, edit.line
                    ));
                }
                edits.push(edit);
                from.clear();
                to.clear();
                state = In::Between;
            }
            (In::From | In::To, Some(other))
                if ["file:", "in:", "occurrence:", "text", "from", "to", "end"]
                    .iter()
                    .any(|known| other.starts_with(known)) =>
            {
                return fail(&format!(
                    "`@@ {other}` inside an edit's text: `@@ from` is followed by `@@ to`, and \
                     `@@ to` by `@@ end`"
                ));
            }
            (In::From, _) => from.push(line),
            (In::To, _) => to.push(line),
        }
    }
    if state != In::Between {
        return Err(format!(
            "the script ends inside edit {}: end each edit with `@@ end`\n\n{SCRIPT_USAGE}",
            edits.len() + 1
        ));
    }
    if edits.is_empty() {
        return Err(format!("the script has no edit in it\n\n{SCRIPT_USAGE}"));
    }
    Ok(edits)
}

/// The files the edits leave, each with its new text, in the order the
/// script first names them, and what to print; or the first edit that
/// cannot be made, named. `read` gives a file's text as it is now.
fn apply_script(
    edits: &[ScriptEdit],
    mut read: impl FnMut(&Path) -> Result<String>,
) -> Result<(Vec<(PathBuf, String)>, String)> {
    let mut files: Vec<(PathBuf, String)> = Vec::new();
    let mut shown = String::new();
    for step in edits {
        let path = &step.request.file;
        let index = match files.iter().position(|(known, _)| known == path) {
            Some(index) => index,
            None => {
                let text = read(path).map_err(|error| {
                    format!(
                        "edit {} of the script (its line {}): {error}. The script was not \
                         applied: no file was written.",
                        step.number, step.line
                    )
                })?;
                files.push((path.clone(), text));
                files.len() - 1
            }
        };
        let (edited, lines) = edit(&files[index].1, &step.request, &path.display().to_string())
            .map_err(|error| {
                format!(
                    "edit {} of the script (its line {}): {error} The script was not applied: \
                     no file was written.",
                    step.number, step.line
                )
            })?;
        files[index].1 = edited;
        shown.push_str(&format!("edit {}: {lines}", step.number));
    }
    Ok((files, shown))
}

/// Where a `--from` that ends in a comma, and is not found, is found
/// without it: a comma before a closing bracket is not a token of the
/// source, and one at the end of `--from` is, so such a match can never be
/// made (`mutate`'s pre-flight says the same of a break's `find`).
fn comma_before_a_bracket(source: &str, tokens: &[lex::Token], request: &Request) -> Option<usize> {
    if lex::code_texts(&request.from).last() != Some(&",") {
        return None;
    }
    let shorter = request.from.trim_end().strip_suffix(',')?;
    match lex::locate_in(
        source,
        tokens,
        request.within.as_deref(),
        shorter,
        request.occurrence,
    ) {
        Ok(range) => Some(lex::line_of(source, range.end)),
        Err(lex::Missing::Ambiguous(lines)) => lines.first().copied(),
        Err(_) => None,
    }
}

/// `cargo xtask replace --script FILE [--dry-run]`
fn run_script(arguments: &[String]) -> Result<()> {
    let mut script = None;
    let mut dry_run = false;
    let mut rest = arguments.iter();
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--script" => {
                script = Some(
                    rest.next()
                        .ok_or_else(|| format!("`--script` needs a file\n\n{SCRIPT_USAGE}"))?,
                );
            }
            "--dry-run" => dry_run = true,
            "--help" | "-h" => return Err(SCRIPT_USAGE.to_owned()),
            other => {
                return Err(format!(
                    "`{other}`: a script names its files and edits itself\n\n{SCRIPT_USAGE}"
                ));
            }
        }
    }
    let script = script.ok_or_else(|| SCRIPT_USAGE.to_owned())?;
    let text = fs::read_to_string(script)
        .map_err(|error| format!("could not read the script {script}: {error}"))?;
    let edits = parse_script(&text)?;
    // One file is one file however the script spells it (`a.rs`, `./a.rs`,
    // another case on Windows, a link to it): each is keyed by where it
    // really is, shown from the working directory when it is under it.
    let here = std::env::current_dir()
        .and_then(fs::canonicalize)
        .unwrap_or_default();
    let mut edits = edits;
    for edit in &mut edits {
        if let Ok(real) = fs::canonicalize(&edit.request.file) {
            edit.request.file = real
                .strip_prefix(&here)
                .map_or_else(|_| real.clone(), Path::to_path_buf);
        }
    }
    let (files, shown) = apply_script(&edits, |path| {
        fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))
    })?;
    print!("{shown}");
    if dry_run {
        println!(
            "(a dry run: {} edit(s) of {} file(s) can all be made; nothing was written)",
            edits.len(),
            files.len()
        );
        return Ok(());
    }
    write_all(&files)?;
    println!("{} edit(s) made, in {} file(s).", edits.len(), files.len());
    Ok(())
}

/// Writes every file, each first beside itself and then moved into place,
/// so that a file that cannot be written leaves every file as it was.
fn write_all(files: &[(PathBuf, String)]) -> Result<()> {
    let beside = |path: &Path| {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".replace-new");
        path.with_file_name(name)
    };
    for (index, (path, text)) in files.iter().enumerate() {
        // The new file takes the old one's permissions (an executable
        // script, a hook, stays one).
        let written = fs::write(beside(path), text).and_then(|()| {
            let permissions = fs::metadata(path)?.permissions();
            fs::set_permissions(beside(path), permissions)
        });
        if let Err(error) = written {
            for (written, _) in &files[..=index] {
                let _ = fs::remove_file(beside(written));
            }
            return Err(format!(
                "could not write beside {}: {error}. No file was changed.",
                path.display()
            ));
        }
    }
    for (index, (path, _)) in files.iter().enumerate() {
        if let Err(error) = fs::rename(beside(path), path) {
            let changed: Vec<String> = files[..index]
                .iter()
                .map(|(path, _)| path.display().to_string())
                .collect();
            for (left, _) in &files[index..] {
                let _ = fs::remove_file(beside(left));
            }
            return Err(format!(
                "could not replace {}: {error}. Changed before it: {}",
                path.display(),
                if changed.is_empty() {
                    "none".to_owned()
                } else {
                    changed.join(", ")
                }
            ));
        }
    }
    Ok(())
}

/// `cargo xtask replace …`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    if arguments.iter().any(|argument| argument == "--script") {
        return run_script(arguments);
    }
    let request = Request::parse(arguments)?;
    let path: &Path = &request.file;
    let source = fs::read_to_string(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    let (edited, shown) = edit(&source, &request, &path.display().to_string())?;
    print!("{shown}");
    if request.dry_run {
        println!("(a dry run: nothing was written)");
        return Ok(());
    }
    fs::write(path, edited).map_err(|error| format!("could not write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{Request, apply_script, edit, parse_script};

    /// One function as written by hand, and as `rustfmt` lays it out.
    const BEFORE_FORMAT: &str = "fn budget(tier: Tier, os: Os) -> f64 { match (tier, os) { (Tier::Unit, _) => 20.0, (Tier::System, Os::Windows) => 90.0, _ => 60.0 } }\n";
    const AFTER_FORMAT: &str = "\
fn budget(tier: Tier, os: Os) -> f64 {
    match (tier, os) {
        (Tier::Unit, _) => 20.0,
        // The Windows runners are slower.
        (Tier::System, Os::Windows) => 90.0,
        _ => 60.0,
    }
}
";

    fn request(from: &str, to: &str) -> Request {
        Request {
            from: from.to_owned(),
            to: to.to_owned(),
            ..Request::default()
        }
    }

    #[test]
    fn a_match_holds_before_and_after_rustfmt() {
        let asked = request(
            "(Tier::System, Os::Windows) => 90.0,",
            "(Tier::System, Os::Windows) => 120.0,",
        );
        for source in [BEFORE_FORMAT, AFTER_FORMAT] {
            let (edited, shown) = edit(source, &asked, "budget.rs").unwrap();
            assert!(
                edited.contains("(Tier::System, Os::Windows) => 120.0,"),
                "{edited}"
            );
            assert!(!edited.contains("90.0"), "{edited}");
            // Only the match changed.
            assert_eq!(edited.len(), source.len() + 1, "{edited}");
            assert!(shown.contains("+ "), "{shown}");
        }
        // Laid out across lines in --from too, it still matches.
        let wrapped = request("(Tier::Unit,\n   _)\n  => 20.0", "(Tier::Unit, _) => 25.0");
        let (edited, _) = edit(AFTER_FORMAT, &wrapped, "budget.rs").unwrap();
        assert!(edited.contains("(Tier::Unit, _) => 25.0,"), "{edited}");
    }

    #[test]
    fn the_changed_lines_are_printed() {
        let asked = request("_ => 60.0", "_ => 61.0");
        let (_, shown) = edit(AFTER_FORMAT, &asked, "budget.rs").unwrap();
        assert_eq!(
            shown,
            "budget.rs: lines 6-6 are now 6-6\n-     6          _ => 60.0,\n+     6          _ => 61.0,\n"
        );
    }

    #[test]
    fn a_match_whose_comma_comes_before_a_bracket_is_told_why_it_cannot_be_made() {
        let source = "fn f() {\n    run(\n        first,\n        second,\n    )\n}\n";
        let error = edit(source, &request("second,", "third,"), "a.rs").unwrap_err();
        assert!(
            error.contains("that comma (line 4) comes before a closing bracket")
                && error.contains("End --from before the comma"),
            "{error}"
        );
        // Without the comma it is made.
        let (edited, _) = edit(source, &request("second", "third"), "a.rs").unwrap();
        assert!(edited.contains("        third,\n"), "{edited}");
        // A --from that is not there without its comma either is just not
        // there.
        let error = edit(source, &request("fourth,", "third,"), "a.rs").unwrap_err();
        assert!(error.contains("--from is not in a.rs"), "{error}");
    }

    #[test]
    fn a_match_by_tokens_that_begins_or_ends_with_a_comment_is_refused() {
        let source = "/// Old.\nfn f() {}\n";
        for from in [
            "/// Old.\nfn f() {}",
            "fn f() {} // trailing",
            "/* a */ fn f() {}",
        ] {
            let error = edit(source, &request(from, "/// New.\nfn f() {}"), "a.rs").unwrap_err();
            assert!(
                error.contains("begins or ends with a comment"),
                "{from:?}: {error}"
            );
        }
        // A comment inside the code is fine: it is passed over, and the
        // match is still the code's.
        let (edited, _) = edit(
            "fn f() { a(); /* x */ b(); }\n",
            &request("a(); /* y */ b();", "c();"),
            "a.rs",
        )
        .unwrap();
        assert_eq!(edited, "fn f() { c(); }\n");
    }

    #[test]
    fn a_match_of_comments_alone_is_told_to_be_text() {
        let asked = request("/// The changed lines are printed.", "/// Printed.");
        let error = edit(
            "/// The changed lines are printed.\nfn f() {}\n",
            &asked,
            "a.rs",
        )
        .unwrap_err();
        assert!(
            error.contains("all comments and white space") && error.contains("`@@ text`"),
            "{error}"
        );
        // As text it is made.
        let text = Request {
            text: true,
            ..request("/// The changed lines are printed.", "/// Printed.")
        };
        let (edited, _) = edit(
            "/// The changed lines are printed.\nfn f() {}\n",
            &text,
            "a.rs",
        )
        .unwrap();
        assert_eq!(edited, "/// Printed.\nfn f() {}\n");
    }

    #[test]
    fn a_match_that_is_not_there_once_changes_nothing() {
        let missing = edit(AFTER_FORMAT, &request("=> 75.0", "=> 1.0"), "budget.rs");
        assert!(missing.unwrap_err().contains("is not in budget.rs"));

        let twice = edit(AFTER_FORMAT, &request("Tier::", "Level::"), "budget.rs");
        let error = twice.unwrap_err();
        assert!(error.contains("2 times, at lines 3, 5"), "{error}");

        let mut second = request("Tier::", "Level::");
        second.occurrence = Some(2);
        let (edited, _) = edit(AFTER_FORMAT, &second, "budget.rs").unwrap();
        assert!(
            edited.contains("(Tier::Unit, _)") && edited.contains("(Level::System"),
            "{edited}"
        );

        let mut inside = request("match", "match");
        inside.within = Some("cost".to_owned());
        let error = edit(AFTER_FORMAT, &inside, "budget.rs").unwrap_err();
        assert!(error.contains("no item called `cost`"), "{error}");

        let same = edit(
            AFTER_FORMAT,
            &request("_ => 60.0", "_  =>  60.0"),
            "budget.rs",
        );
        assert!(same.unwrap_err().contains("the same code"));
    }

    const MARKDOWN: &str = "# Title\n\nSome text, and *more* text.\n\n- a list\n- a list\n";

    fn text_request(from: &str, to: &str) -> Request {
        Request {
            text: true,
            ..request(from, to)
        }
    }

    #[test]
    fn text_is_replaced_exactly_once_or_not_at_all() {
        let (edited, shown) = edit(
            MARKDOWN,
            &text_request("*more* text", "**more** text"),
            "a.md",
        )
        .unwrap();
        assert_eq!(
            edited,
            "# Title\n\nSome text, and **more** text.\n\n- a list\n- a list\n"
        );
        assert_eq!(
            shown,
            "a.md: lines 3-3 are now 3-3\n-     3  Some text, and *more* text.\n+     3  Some text, \
             and **more** text.\n"
        );
        // White space is text here: a match laid out otherwise is no match.
        let wrapped = edit(MARKDOWN, &text_request("*more*\ntext", "x"), "a.md");
        assert!(wrapped.unwrap_err().contains("is not in a.md"));
        // Twice is refused, with the lines, and so is an overlapping pair.
        let twice = edit(MARKDOWN, &text_request("- a list", "- one"), "a.md").unwrap_err();
        assert!(twice.contains("2 times, at lines 5, 6"), "{twice}");
        let overlapping = edit("aaa\n", &text_request("aa", "b"), "x.md").unwrap_err();
        assert!(overlapping.contains("2 times"), "{overlapping}");
        let same = edit(MARKDOWN, &text_request("# Title", "# Title"), "a.md").unwrap_err();
        assert!(same.contains("the same text"), "{same}");
        assert!(edit(MARKDOWN, &text_request("", "x"), "a.md").is_err());
    }

    #[test]
    fn a_request_is_read() {
        let strings = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        let asked = Request::parse(&strings(&[
            "src/a.rs",
            "--from",
            "x + 1",
            "--to",
            "x + 2",
            "--in",
            "f",
            "--occurrence",
            "2",
            "--dry-run",
        ]))
        .unwrap();
        assert_eq!(asked.file.to_str(), Some("src/a.rs"));
        assert_eq!((asked.from.as_str(), asked.to.as_str()), ("x + 1", "x + 2"));
        assert_eq!(asked.within.as_deref(), Some("f"));
        assert_eq!(asked.occurrence, Some(2));
        assert!(asked.dry_run);
        for (words, expected) in [
            (&["--from", "x", "--to", "y"][..], "which file?"),
            (&["a.rs", "--to", "y"], "--from"),
            (&["a.rs", "--from", "x"], "--to"),
            (
                &["a.rs", "b.rs", "--from", "x", "--to", "y"],
                "one file at a time",
            ),
            (
                &["a.rs", "--from", "x", "--to", "y", "--occurrence", "0"],
                "a number from 1",
            ),
            (&["a.rs", "--from"], "needs a value"),
            (
                &["a.md", "--text", "--from", "x", "--to", "y", "--in", "f"],
                "--in and --occurrence are for Rust",
            ),
        ] {
            let error = Request::parse(&strings(words)).unwrap_err();
            assert!(error.contains(expected), "{words:?}: {error}");
        }
    }

    /// A script of three edits over two files: Rust by tokens, Markdown by
    /// its text, and a second edit of the first file.
    const SCRIPT: &str = "\
# The budgets move.
@@ file: budget.rs
@@ in: budget
@@ from
(Tier::System,
    Os::Windows) => 90.0
@@ to
(Tier::System, Os::Windows) => 120.0
@@ end

@@ file: notes.md
@@ from
90 s on Windows
@@ to
120 s on Windows
@@ end
@@ file: budget.rs
@@ from
_ => 60.0
@@ to
_ => 61.0
@@ end
";

    fn files(listed: &[(&str, &str)]) -> impl FnMut(&Path) -> crate::Result<String> {
        let listed: Vec<(PathBuf, String)> = listed
            .iter()
            .map(|(path, text)| (PathBuf::from(path), (*text).to_owned()))
            .collect();
        move |path: &Path| {
            listed
                .iter()
                .find(|(known, _)| known == path)
                .map(|(_, text)| text.clone())
                .ok_or_else(|| format!("no file {}", path.display()))
        }
    }

    #[test]
    fn a_script_is_read_into_its_edits() {
        let edits = parse_script(SCRIPT).unwrap();

        assert_eq!(edits.len(), 3);
        assert_eq!(edits[0].line, 2);
        assert_eq!(edits[0].request.within.as_deref(), Some("budget"));
        assert!(!edits[0].request.text, "a .rs file is matched by tokens");
        assert_eq!(
            edits[0].request.from,
            "(Tier::System,\n    Os::Windows) => 90.0"
        );
        assert!(edits[1].request.text, "any other file by its text");
        assert_eq!(edits[1].request.to, "120 s on Windows");
        assert_eq!(edits[2].number, 3);
        // `@@ text` makes a Rust file's match exact.
        let forced =
            parse_script("@@ file: a.rs\n@@ text\n@@ from\nx\n@@ to\ny\n@@ end\n").unwrap();
        assert!(forced[0].request.text);
    }

    #[test]
    fn every_edit_of_a_script_is_made_in_order() {
        let edits = parse_script(SCRIPT).unwrap();
        let (files, shown) = apply_script(
            &edits,
            files(&[
                ("budget.rs", AFTER_FORMAT),
                (
                    "notes.md",
                    "The system tier: 60 s on Linux, 90 s on Windows.\n",
                ),
            ]),
        )
        .unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, PathBuf::from("budget.rs"));
        assert!(files[0].1.contains("(Tier::System, Os::Windows) => 120.0,"));
        assert!(files[0].1.contains("_ => 61.0,"));
        assert_eq!(
            files[1].1,
            "The system tier: 60 s on Linux, 120 s on Windows.\n"
        );
        assert!(shown.starts_with("edit 1: budget.rs: lines"), "{shown}");
        assert!(shown.contains("edit 3: budget.rs"), "{shown}");
    }

    #[test]
    fn a_script_with_one_edit_that_cannot_be_made_is_refused_whole_and_names_it() {
        let edits = parse_script(SCRIPT).unwrap();
        for (notes, expected) in [
            ("No budget here.\n", "--from is not in notes.md as text"),
            (
                "90 s on Windows, and 90 s on Windows again.\n",
                "--from is in notes.md 2 times",
            ),
        ] {
            let error = apply_script(
                &edits,
                files(&[("budget.rs", AFTER_FORMAT), ("notes.md", notes)]),
            )
            .unwrap_err();

            assert!(
                error.starts_with("edit 2 of the script (its line 11): "),
                "{error}"
            );
            assert!(error.contains(expected), "{error}");
            assert!(error.contains("no file was written"), "{error}");
        }
        // A file that is not there is named the same way.
        let error = apply_script(&edits, files(&[("budget.rs", AFTER_FORMAT)])).unwrap_err();
        assert!(error.starts_with("edit 2 of the script (its line 11): no file notes.md"));
    }

    #[test]
    fn an_edit_sees_what_the_edits_before_it_left() {
        // The second edit finds what the first put there.
        let script = "@@ file: a.md\n@@ from\none\n@@ to\ntwo\n@@ end\n@@ file: a.md\n@@ \
                      from\ntwo\n@@ to\nthree\n@@ end\n";
        let (files, _) =
            apply_script(&parse_script(script).unwrap(), files(&[("a.md", "one\n")])).unwrap();
        assert_eq!(files[0].1, "three\n");
    }

    #[test]
    fn a_script_that_is_not_well_formed_is_refused_with_its_line() {
        for (script, expected) in [
            ("stray\n", "line 1: text outside an edit"),
            (
                "@@ from\nx\n",
                "line 1: an edit begins with `@@ file: PATH`",
            ),
            (
                "@@ file: a.rs\nx\n",
                "line 2: an edit's header is followed by `@@ from`",
            ),
            (
                "@@ file: a.rs\n@@ from\nx\n@@ end\n",
                "line 4: `@@ end` inside an edit's text",
            ),
            (
                "@@ file: a.rs\n@@ from\nx\n@@ to\ny\n",
                "ends inside edit 1",
            ),
            ("# only a comment\n", "has no edit in it"),
            (
                "@@ file: a.md\n@@ in: x\n@@ from\nx\n@@ to\ny\n@@ end\n",
                "`in:` and `occurrence:` are for Rust",
            ),
            (
                "@@ file: a.rs\n@@ occurrence: 0\n",
                "line 2: an occurrence is a number from 1",
            ),
        ] {
            let error = parse_script(script).unwrap_err();
            assert!(error.contains(expected), "{script:?}: {error}");
        }
    }

    #[test]
    fn a_file_named_two_ways_is_one_file_and_keeps_its_permissions() {
        let dir =
            std::env::temp_dir().join(format!("cortex-xtask-replace-two-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let a = dir.join("a.sh");
        std::fs::write(&a, "one\ntwo\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&a, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let other = dir.join("sub").join("..").join("a.sh");
        let script = dir.join("edits");
        std::fs::write(
            &script,
            format!(
                "@@ file: {}\n@@ from\none\n@@ to\nONE\n@@ end\n@@ file: {}\n@@ from\ntwo\n@@ \
                 to\nTWO\n@@ end\n",
                a.display(),
                other.display()
            ),
        )
        .unwrap();

        super::run(&["--script".to_owned(), script.display().to_string()]).unwrap();

        // Both edits are in it: the second saw what the first left.
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "ONE\nTWO\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "the file kept its permissions");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_script_on_real_files_writes_all_of_them_or_none() {
        let dir = std::env::temp_dir().join(format!("cortex-xtask-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.rs");
        let b = dir.join("b.md");
        let script = dir.join("edits");
        let write_script = |b_from: &str| {
            std::fs::write(
                &script,
                format!(
                    "@@ file: {}\n@@ from\nlet x = 1;\n@@ to\nlet x = 2;\n@@ end\n@@ file: \
                     {}\n@@ from\n{b_from}\n@@ to\nnew\n@@ end\n",
                    a.display(),
                    b.display()
                ),
            )
            .unwrap();
        };
        let run = || super::run(&["--script".to_owned(), script.display().to_string()]);
        std::fs::write(&a, "fn f() { let x = 1; }\n").unwrap();
        std::fs::write(&b, "old\n").unwrap();

        // The second edit finds nothing: the first file is left as it was.
        write_script("missing");
        assert!(run().unwrap_err().contains("edit 2 of the script"));
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "fn f() { let x = 1; }\n"
        );
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "old\n");
        // A dry run writes nothing either.
        write_script("old");
        super::run(&[
            "--script".to_owned(),
            script.display().to_string(),
            "--dry-run".to_owned(),
        ])
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "fn f() { let x = 1; }\n"
        );

        run().unwrap();
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "fn f() { let x = 2; }\n"
        );
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "new\n");
        // Nothing is left beside them.
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["a.rs", "b.md", "edits"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
