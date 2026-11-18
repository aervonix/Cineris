//! `cargo xtask perf`: the TUI's budgets measured on the demo, by the method
//! of `docs/measurements/tui-language-spike.md`.
//!
//! It builds the demo with optimisation (the example `demo` of
//! `cineris-term`), then runs the ignored measurement test of `cineris-term`'s
//! integration binary, also optimised, which starts the demo in a
//! pseudo-terminal (ConPTY on Windows) and prints a table: start to first
//! frame, key to frame when idle and while streaming (and the demo's own
//! share of it), frame cost, frame intervals and bytes per frame. What is
//! measured, and how, is that test's documentation
//! (`crates/cineris-term/tests/it/perf.rs`).
//!
//! Then it does the same for the editor demo (the example `editor-demo` of
//! `cineris-tui`, since step 21): key to frame when idle, while streaming,
//! and in an `@` menu over 100,000 paths, each with the harness's own share
//! (`crates/cineris-tui/tests/it/perf.rs`). Then the views' own share, since
//! step 22: real views building and drawing a 30-row live region frame after
//! frame, and the highlighter's first use
//! (`crates/cineris-tui/tests/it/frames.rs`). `--only term`, `--only editor`
//! or `--only views` runs one of the three. With `--json PATH`, the editor's
//! table goes to `PATH.editor`.
//!
//! The rest of the performance budgets (start-up of the real binary, memory,
//! binary size) have commands of their own: `startup` and `size`.

use std::path::PathBuf;

use crate::Result;
use crate::{outside, steps, workspace};

/// What `perf` was asked for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Request {
    runs: Option<u64>,
    seconds: Option<u64>,
    json: Option<String>,
    only: Option<Only>,
}

/// Which demo to measure, when not both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Only {
    /// Step 20's demo of the terminal layer.
    Term,
    /// Step 21's editor demo.
    Editor,
    /// Step 22's views, drawn frame after frame.
    Views,
}

fn parse(arguments: &[String]) -> Result<Request> {
    let mut request = Request::default();
    let mut rest = arguments.iter();
    let number = |value: Option<&String>, flag: &str| -> Result<u64> {
        value
            .and_then(|value| value.parse().ok())
            .filter(|number| *number > 0)
            .ok_or_else(|| format!("`{flag}` takes a number above zero"))
    };
    while let Some(argument) = rest.next() {
        match argument.as_str() {
            "--runs" => request.runs = Some(number(rest.next(), "--runs")?),
            "--seconds" => request.seconds = Some(number(rest.next(), "--seconds")?),
            "--json" => request.json = Some(rest.next().ok_or("`--json` takes a path")?.clone()),
            "--only" => {
                request.only = Some(match rest.next().map(String::as_str) {
                    Some("term") => Only::Term,
                    Some("editor") => Only::Editor,
                    Some("views") => Only::Views,
                    _ => return Err("`--only` takes `term`, `editor` or `views`".to_owned()),
                });
            }
            other => {
                return Err(format!(
                    "unknown argument `{other}`\nusage: cargo xtask perf [--runs N] [--seconds N] [--json PATH] [--only term|editor|views]"
                ));
            }
        }
    }
    Ok(request)
}

/// Whether `cargo xtask perf` would take `arguments`, which it reads and
/// runs nothing: how `try` refuses what it cannot pass on before it builds.
pub(crate) fn check_arguments(arguments: &[String]) -> Result<()> {
    parse(arguments).map(|_| ())
}

/// Where cargo puts the optimised example `name`.
fn demo_binary(name: &str) -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| workspace::root().join("target"), PathBuf::from);
    target
        .join("release")
        .join("examples")
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// A glyph set a demo is run in, and how it is chosen: on Unix the
/// terminal layer takes Unicode from a locale that names UTF-8
/// (`cineris_term::Capabilities::detect`), so each demo runs under one and
/// under the plain `C` locale. On Windows the console decides, and ConPTY
/// gets the ASCII set: there is one run, in what the console gives.
struct GlyphSet {
    name: &'static str,
    locale: Option<&'static str>,
    /// What is added to a `--json` path for this run.
    suffix: &'static str,
}

fn glyph_sets() -> Vec<GlyphSet> {
    if cfg!(windows) {
        vec![GlyphSet {
            name: "the console's (ASCII under ConPTY)",
            locale: None,
            suffix: "",
        }]
    } else {
        vec![
            GlyphSet {
                name: "Unicode",
                locale: Some("C.UTF-8"),
                suffix: "",
            },
            GlyphSet {
                name: "ASCII",
                locale: Some("C"),
                suffix: ".ascii",
            },
        ]
    }
}

/// `cargo xtask perf [--runs N] [--seconds N] [--json PATH] [--only
/// term|editor|views]`.
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = parse(arguments)?;
    let all = request.only.is_none();
    for glyphs in glyph_sets() {
        if all || request.only == Some(Only::Term) {
            println!(
                "\n== the terminal demo, in the {} glyph set ==",
                glyphs.name
            );
            terminal_demo(&request, &glyphs)?;
        }
        if all || request.only == Some(Only::Editor) {
            println!("\n== the editor demo, in the {} glyph set ==", glyphs.name);
            editor_demo(&request, &glyphs)?;
        }
    }
    if all || request.only == Some(Only::Views) {
        views()?;
    }
    Ok(())
}

/// Step 20's demo against the TUI's budgets.
fn terminal_demo(request: &Request, glyphs: &GlyphSet) -> Result<()> {
    steps::cargo(&[
        "build",
        "--locked",
        "--release",
        "-p",
        "cineris-term",
        "--example",
        "demo",
        "--features",
        "demo",
    ])?;
    let demo = demo_binary("demo");
    let demo = demo
        .to_str()
        .ok_or("the target directory's path is not Unicode")?;
    let runs = request.runs.map(|runs| runs.to_string());
    let seconds = request.seconds.map(|seconds| seconds.to_string());
    let json = request
        .json
        .as_ref()
        .map(|json| format!("{json}{}", glyphs.suffix));
    let mut variables = vec![("CINERIS_PERF_DEMO", demo)];
    if let Some(locale) = glyphs.locale {
        variables.push(("LC_ALL", locale));
    }
    if let Some(runs) = &runs {
        variables.push(("CINERIS_PERF_RUNS", runs));
    }
    if let Some(seconds) = &seconds {
        variables.push(("CINERIS_PERF_SECONDS", seconds));
    }
    if let Some(json) = &json {
        variables.push(("CINERIS_PERF_JSON", json));
    }
    outside::run_ignored(
        &["--release", "--package", "cineris-term", "--test", "it"],
        "perf::perf_the_demo_against_the_budgets",
        &variables,
    )
}

/// Step 22's views: their share of a frame with a 30-row live region, and
/// the highlighter's first use.
fn views() -> Result<()> {
    outside::run_ignored(
        &["--release", "--package", "cineris-tui", "--test", "it"],
        "frames::perf_frames_of_the_views_with_a_30_row_live_region",
        &[],
    )
}

/// Step 21's editor demo against the key-latency budgets.
fn editor_demo(request: &Request, glyphs: &GlyphSet) -> Result<()> {
    steps::cargo(&[
        "build",
        "--locked",
        "--release",
        "-p",
        "cineris-tui",
        "--example",
        "editor-demo",
        "--features",
        "demo",
    ])?;
    let demo = demo_binary("editor-demo");
    let demo = demo
        .to_str()
        .ok_or("the target directory's path is not Unicode")?;
    let seconds = request.seconds.map(|seconds| seconds.to_string());
    let json = request
        .json
        .as_ref()
        .map(|json| format!("{json}.editor{}", glyphs.suffix));
    let mut variables = vec![("CINERIS_PERF_EDITOR_DEMO", demo)];
    if let Some(locale) = glyphs.locale {
        variables.push(("LC_ALL", locale));
    }
    if let Some(seconds) = &seconds {
        variables.push(("CINERIS_PERF_SECONDS", seconds));
    }
    if let Some(json) = &json {
        variables.push(("CINERIS_PERF_JSON", json));
    }
    outside::run_ignored(
        &["--release", "--package", "cineris-tui", "--test", "it"],
        "perf::perf_the_editor_against_the_key_budgets",
        &variables,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn arguments() {
        assert_eq!(parse(&[]).unwrap(), Request::default());
        assert_eq!(
            parse(&strings(&[
                "--runs",
                "5",
                "--seconds",
                "3",
                "--json",
                "out.json"
            ]))
            .unwrap(),
            Request {
                runs: Some(5),
                seconds: Some(3),
                json: Some("out.json".to_owned()),
                only: None,
            }
        );
        assert_eq!(
            parse(&strings(&["--only", "editor"])).unwrap().only,
            Some(Only::Editor)
        );
        assert_eq!(
            parse(&strings(&["--only", "views"])).unwrap().only,
            Some(Only::Views)
        );
        assert!(parse(&strings(&["--only", "both"])).is_err());
        assert!(parse(&strings(&["--runs", "0"])).is_err());
        assert!(parse(&strings(&["--runs"])).is_err());
        assert!(parse(&strings(&["--frames"])).is_err());
    }
}
