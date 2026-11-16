//! Reading the JUnit report nextest writes.
//!
//! Three commands read it: the tier budgets (`tiers`), which need each
//! test's time; `stress`, which needs to know which repeat of a test failed;
//! and `mutate`, which needs the names of the tests a break made fail. A
//! report is the one place nextest says these things in a form meant for a
//! program: its terminal output pads and reorders, and reading it has
//! misled a mutation run before.
//!
//! It reads only what nextest writes: attributes in double quotes, and the
//! `<failure>` and `<error>` children of a `<testcase>`.

use std::fs;
use std::path::Path;

use crate::Result;

/// One `<testcase>` of a report.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TestCase {
    /// nextest's binary id: `cortex-base`, `cortex-agent::it`, `cortex-cli::system`.
    pub(crate) binary: String,
    pub(crate) name: String,
    pub(crate) seconds: f64,
    /// The test failed, timed out or could not be run. A test that passed
    /// on a retry is not failed.
    pub(crate) failed: bool,
    /// The test failed and then passed when it was tried again. The run
    /// counts it as passed; a search for tests that fail some of the time
    /// counts it as one.
    pub(crate) retried: bool,
    /// The repeat this case is from, counted from 0, when the report is of
    /// a `--stress-count` run. nextest names the suite `<binary>@stress-<n>`.
    pub(crate) repeat: Option<u32>,
}

impl TestCase {
    /// `<binary> <name>`: how nextest writes a test, and how a break names
    /// the tests that catch it.
    pub(crate) fn id(&self) -> String {
        format!("{} {}", self.binary, self.name)
    }
}

/// Reads a report from a file. A report with no test in it is an error: a
/// run that listed nothing checked nothing.
pub(crate) fn read(report: &Path) -> Result<Vec<TestCase>> {
    let xml = fs::read_to_string(report)
        .map_err(|error| format!("could not read {}: {error}", report.display()))?;
    let cases = parse(&xml);
    if cases.is_empty() {
        return Err(format!("{} lists no tests", report.display()));
    }
    Ok(cases)
}

/// One report of two runs' reports, as if one run had written both: the
/// second's suites after the first's, under the first's root, whose counts
/// and time are summed. The blocking test job runs tiers 1 and 2 and then
/// the system tests (`checks::ci_runs`), and what reads its report reads
/// one.
pub(crate) fn merge(first: &str, second: &str) -> Result<String> {
    let (open, first_body) = root(first)?;
    let (second_open, second_body) = root(second)?;
    let mut open = open.to_owned();
    for name in ["tests", "skipped", "failures", "errors"] {
        let count = |tag: &str| attribute(tag, name).and_then(|value| value.parse::<u64>().ok());
        if let (Some(one), Some(other)) = (count(&open), count(second_open)) {
            open = with_attribute(&open, name, &(one + other).to_string());
        }
    }
    let time = |tag: &str| attribute(tag, "time").and_then(|value| value.parse::<f64>().ok());
    if let (Some(one), Some(other)) = (time(&open), time(second_open)) {
        open = with_attribute(&open, "time", &format!("{:.3}", one + other));
    }
    Ok(format!("{open}{first_body}{second_body}</testsuites>\n"))
}

/// A report up to the end of its `<testsuites …>` tag, and what lies
/// between that tag and `</testsuites>`.
fn root(xml: &str) -> Result<(&str, &str)> {
    let refused = || "a JUnit report without its <testsuites> element".to_owned();
    let start = xml.find("<testsuites").ok_or_else(refused)?;
    let open_end = start + xml[start..].find('>').ok_or_else(refused)? + 1;
    let close = xml.rfind("</testsuites>").ok_or_else(refused)?;
    if close < open_end {
        return Err(refused());
    }
    Ok((&xml[..open_end], &xml[open_end..close]))
}

/// `tag` with the value of its attribute `name` replaced by `value`, which
/// needs no escape (a number).
fn with_attribute(tag: &str, name: &str, value: &str) -> String {
    let Some(old) = attribute(tag, name) else {
        return tag.to_owned();
    };
    tag.replacen(
        &format!(" {name}=\"{old}\""),
        &format!(" {name}=\"{value}\""),
        1,
    )
}

/// Reads the `<testcase>` elements of a report.
pub(crate) fn parse(xml: &str) -> Vec<TestCase> {
    let mut cases = Vec::new();
    for suite in xml.split("<testsuite ").skip(1) {
        let suite_tag = suite.split('>').next().unwrap_or(suite);
        let repeat = attribute(suite_tag, "name").and_then(|name| {
            let (_, number) = name.rsplit_once("@stress-")?;
            number.parse().ok()
        });
        for element in suite.split("<testcase ").skip(1) {
            let tag = element.split('>').next().unwrap_or(element);
            // `<testcase ... />` has no children. Otherwise they run to the
            // closing tag.
            let children = if tag.ends_with('/') {
                ""
            } else {
                let after_tag = &element[tag.len()..];
                after_tag.split("</testcase>").next().unwrap_or(after_tag)
            };
            // nextest always writes a time. A case without one is still a
            // case: dropping it would drop a failure with it.
            let seconds = attribute(tag, "time")
                .and_then(|time| time.parse().ok())
                .unwrap_or(0.0);
            if let (Some(name), Some(binary)) =
                (attribute(tag, "name"), attribute(tag, "classname"))
            {
                cases.push(TestCase {
                    binary: unescape(binary),
                    name: unescape(name),
                    seconds,
                    // Text is escaped in a report, so a `<` begins an
                    // element. A test that passed on a retry has
                    // `<flakyFailure>` children, which neither of these
                    // matches.
                    failed: children.contains("<failure") || children.contains("<error"),
                    retried: children.contains("<flakyFailure") || children.contains("<flakyError"),
                    repeat,
                });
            }
        }
    }
    cases
}

/// What a failed case of a report says about its failure.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Failure {
    pub(crate) binary: String,
    pub(crate) name: String,
    pub(crate) repeat: Option<u32>,
    /// The failure's text as nextest kept it: the panic's message for a
    /// test that panicked, empty for a timeout.
    pub(crate) text: String,
    /// The failure's kind as nextest names it: `test failure with exit
    /// code 101`, `test timeout`.
    pub(crate) kind: String,
    /// What the test wrote to its standard error: where the timeout report
    /// of `cortex_testkit::watch` is.
    pub(crate) stderr: String,
    /// When the test started, as nextest wrote it (`2026-11-15T01:27:48.020+00:00`).
    pub(crate) started: String,
    /// How long it ran, in seconds: for a timeout, until nextest stopped it.
    pub(crate) seconds: f64,
}

/// The failures of a report, in its order: every case with a `<failure>`,
/// an `<error>` or, for a test that passed on its retry, a
/// `<flakyFailure>` or `<flakyError>`, with the first of them.
pub(crate) fn failures(xml: &str) -> Vec<Failure> {
    failures_among(xml, &["failure", "error", "flakyFailure", "flakyError"])
}

/// The cases of a report that failed, in its order: with a `<failure>` or
/// an `<error>`. A test that passed on its retry is not among them, as it
/// is not a failure of the run ([`TestCase::failed`]).
pub(crate) fn failed(xml: &str) -> Vec<Failure> {
    failures_among(xml, &["failure", "error"])
}

/// The cases with one of the elements `wanted`, each with the first of
/// them.
fn failures_among(xml: &str, wanted: &[&str]) -> Vec<Failure> {
    let mut found = Vec::new();
    for suite in xml.split("<testsuite ").skip(1) {
        let suite_tag = suite.split('>').next().unwrap_or(suite);
        let repeat = attribute(suite_tag, "name").and_then(|name| {
            let (_, number) = name.rsplit_once("@stress-")?;
            number.parse().ok()
        });
        for element in suite.split("<testcase ").skip(1) {
            let tag = element.split('>').next().unwrap_or(element);
            if tag.ends_with('/') {
                continue;
            }
            let after_tag = &element[tag.len()..];
            let children = after_tag.split("</testcase>").next().unwrap_or(after_tag);
            let first = wanted
                .iter()
                .filter_map(|child| {
                    // `<failure` must not be the start of `<failures`.
                    let at = children
                        .find(&format!("<{child} "))
                        .into_iter()
                        .chain(children.find(&format!("<{child}>")))
                        .chain(children.find(&format!("<{child}/")))
                        .min()?;
                    Some((at, *child))
                })
                .min();
            let Some((at, child)) = first else {
                continue;
            };
            let (kind, text) = child_text(&children[at..], child);
            let stderr = children
                .find("<system-err>")
                .map(|at| child_text(&children[at..], "system-err").1)
                .unwrap_or_default();
            if let (Some(name), Some(binary)) =
                (attribute(tag, "name"), attribute(tag, "classname"))
            {
                found.push(Failure {
                    binary: unescape(binary),
                    name: unescape(name),
                    repeat,
                    text,
                    kind,
                    stderr,
                    started: attribute(tag, "timestamp")
                        .map(unescape)
                        .unwrap_or_default(),
                    seconds: attribute(tag, "time")
                        .and_then(|time| time.parse().ok())
                        .unwrap_or(0.0),
                });
            }
        }
    }
    found
}

/// One `<testcase>` of a report, where it lies in the text.
struct Span {
    start: usize,
    end: usize,
    binary: String,
    name: String,
}

/// Where each `<testcase>` of a report lies, in the order of the text.
fn spans(xml: &str) -> Vec<Span> {
    const CLOSE: &str = "</testcase>";
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(at) = xml[from..].find("<testcase ") {
        let start = from + at;
        let Some(tag_length) = xml[start..].find('>') else {
            break;
        };
        let tag = &xml[start..start + tag_length];
        let end = if tag.ends_with('/') {
            start + tag_length + 1
        } else {
            match xml[start..].find(CLOSE) {
                Some(close) => start + close + CLOSE.len(),
                None => break,
            }
        };
        if let (Some(name), Some(binary)) = (attribute(tag, "name"), attribute(tag, "classname")) {
            found.push(Span {
                start,
                end,
                binary: unescape(binary),
                name: unescape(name),
            });
        }
        from = end;
    }
    found
}

/// A report in which the cases of `tests` (a binary and a name each) are
/// the ones `rerun` has, in the places of the ones `first` has: what
/// `cargo xtask ci` leaves behind when it has run tests again after a stall
/// (`stall::rerun`). A test that `rerun` has no case for keeps its case
/// from `first`, so that a test that went missing is still the failure it
/// was. The counts of failures and errors, of each suite and of the whole,
/// are made again from the cases, so that the report does not say a
/// failure that is no longer in it.
pub(crate) fn replace_cases(first: &str, rerun: &str, tests: &[(String, String)]) -> String {
    let theirs = spans(rerun);
    let mut out = first.to_owned();
    // From the end, so that a replacement does not move the places of the
    // ones before it.
    for ours in spans(first).iter().rev() {
        if !tests
            .iter()
            .any(|(binary, name)| *binary == ours.binary && *name == ours.name)
        {
            continue;
        }
        if let Some(new) = theirs
            .iter()
            .find(|new| new.binary == ours.binary && new.name == ours.name)
        {
            out.replace_range(ours.start..ours.end, &rerun[new.start..new.end]);
        }
    }
    recount(&out)
}

/// Whether the text of a case has an element named `child`.
fn has_child(case: &str, child: &str) -> bool {
    [" ", ">", "/"]
        .iter()
        .any(|after| case.contains(&format!("<{child}{after}")))
}

/// A report with the `failures` and `errors` of each suite, and of the
/// root, counted from its cases again. A report that cannot be read as one
/// is returned as it is.
fn recount(xml: &str) -> String {
    let Ok((open, body)) = root(xml) else {
        return xml.to_owned();
    };
    let mut failures_total = 0_usize;
    let mut errors_total = 0_usize;
    let mut text = String::new();
    for (index, suite) in body.split("<testsuite ").enumerate() {
        if index == 0 {
            text.push_str(suite);
            continue;
        }
        let tag = suite.split('>').next().unwrap_or(suite);
        let (mut failures, mut errors) = (0, 0);
        for case in spans(suite) {
            let case = &suite[case.start..case.end];
            failures += usize::from(has_child(case, "failure"));
            errors += usize::from(has_child(case, "error"));
        }
        failures_total += failures;
        errors_total += errors;
        let new_tag = with_attribute(tag, "failures", &failures.to_string());
        let new_tag = with_attribute(&new_tag, "errors", &errors.to_string());
        text.push_str("<testsuite ");
        text.push_str(&new_tag);
        text.push_str(&suite[tag.len()..]);
    }
    let open = with_attribute(open, "failures", &failures_total.to_string());
    let open = with_attribute(&open, "errors", &errors_total.to_string());
    let close = &xml[xml.rfind("</testsuites>").unwrap_or(xml.len())..];
    format!("{open}{text}{close}")
}

/// The `type` of the element `child` that `element` begins with, and its
/// text, or its `message` when it has no text.
fn child_text(element: &str, child: &str) -> (String, String) {
    let tag = element.split('>').next().unwrap_or(element);
    let kind = attribute(tag, "type").map(unescape).unwrap_or_default();
    let message = attribute(tag, "message").map(unescape).unwrap_or_default();
    let text = if tag.ends_with('/') {
        String::new()
    } else {
        let body = element.get(tag.len() + 1..).unwrap_or_default();
        let body = body.split(&format!("</{child}>")).next().unwrap_or(body);
        // A retry's element holds elements of its own (quick-junit writes
        // `<stackTrace>`, `<system-out>` and `<system-err>` in a
        // `<flakyFailure>`): its text is the stack trace's, and nothing of
        // the output. Text is escaped, so a `<` begins an element.
        let body = match body.split_once("<stackTrace>") {
            Some((_, trace)) => trace.split("</stackTrace>").next().unwrap_or(trace),
            None => body,
        };
        unescape(body.split('<').next().unwrap_or(body))
    };
    let text = if text.trim().is_empty() {
        message
    } else {
        text
    };
    (kind, text)
}

/// The value of `name="..."` in a start tag.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    loop {
        let start = rest.find(&format!("{name}=\""))?;
        // "name" must not be the tail of "classname".
        let whole_name = start == 0 || rest[..start].ends_with(char::is_whitespace);
        let value = &rest[start + name.len() + 2..];
        if whole_name {
            return value.split('"').next();
        }
        rest = value;
    }
}

/// Undoes the five escapes XML has for attribute values.
fn unescape(value: &str) -> String {
    if !value.contains('&') {
        return value.to_owned();
    }
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{TestCase, parse};

    /// A report as nextest writes it, with the attributes in both orders, a
    /// test with output, a failure, a timeout, and a test that passed on
    /// its retry.
    pub(crate) const REPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="7" failures="2" errors="0" time="0.9">
    <testsuite name="cortex-base" tests="1" disabled="0" errors="0" failures="0">
        <testcase name="text::tests::wraps_at_the_width" classname="cortex-base" timestamp="2026-11-12T13:25:40.000+00:00" time="0.004">
        </testcase>
    </testsuite>
    <testsuite name="cortex-agent::it" tests="3" disabled="0" errors="0" failures="2">
        <testcase classname="cortex-agent::it" name="loop_::retries_after_a_stall" time="0.250"/>
        <testcase name="loop_::stops_at_the_refused_call" classname="cortex-agent::it" time="0.031">
            <failure message="thread &apos;loop_::stops_at_the_refused_call&apos; panicked" type="test failure with exit code 101">assertion `left == right` failed
  left: [&quot;a&quot;]
 right: [&quot;a&quot;, &quot;b&quot;]</failure>
            <system-out>running 1 test</system-out>
        </testcase>
        <testcase name="loop_::never_ends" classname="cortex-agent::it" time="10.002">
            <failure type="test timeout"/>
        </testcase>
    </testsuite>
    <testsuite name="cortex-cli::system" tests="3" disabled="0" errors="0" failures="0">
        <testcase name="sys_version_is_byte_identical_under_both_names" classname="cortex-cli::system" time="0.5">
            <system-out>cortex 0.0.0</system-out>
        </testcase>
        <testcase name="rules::tests::sys_the_repository_follows_its_own_rules" classname="xtask::bin/xtask" time="0.125">
        </testcase>
        <testcase name="timing_sensitive::sys_passes_the_second_time" classname="cortex-cli::system" time="0.2">
            <flakyFailure message="first try" type="test failure with exit code 101">no</flakyFailure>
        </testcase>
    </testsuite>
</testsuites>
"#;

    pub(crate) fn case(binary: &str, name: &str, seconds: f64) -> TestCase {
        TestCase {
            binary: binary.to_owned(),
            name: name.to_owned(),
            seconds,
            failed: false,
            retried: false,
            repeat: None,
        }
    }

    fn failed(binary: &str, name: &str, seconds: f64) -> TestCase {
        TestCase {
            failed: true,
            ..case(binary, name, seconds)
        }
    }

    #[test]
    fn the_report_is_read_whatever_the_attribute_order() {
        assert_eq!(
            parse(REPORT),
            [
                case("cortex-base", "text::tests::wraps_at_the_width", 0.004),
                case("cortex-agent::it", "loop_::retries_after_a_stall", 0.25),
                failed(
                    "cortex-agent::it",
                    "loop_::stops_at_the_refused_call",
                    0.031
                ),
                failed("cortex-agent::it", "loop_::never_ends", 10.002),
                case(
                    "cortex-cli::system",
                    "sys_version_is_byte_identical_under_both_names",
                    0.5
                ),
                case(
                    "xtask::bin/xtask",
                    "rules::tests::sys_the_repository_follows_its_own_rules",
                    0.125
                ),
                TestCase {
                    retried: true,
                    ..case(
                        "cortex-cli::system",
                        "timing_sensitive::sys_passes_the_second_time",
                        0.2
                    )
                },
            ]
        );
    }

    #[test]
    fn a_failure_belongs_to_its_own_test_and_not_to_the_one_before() {
        let cases = parse(REPORT);
        let failures: Vec<&str> = cases
            .iter()
            .filter(|case| case.failed)
            .map(|case| case.name.as_str())
            .collect();

        assert_eq!(
            failures,
            ["loop_::stops_at_the_refused_call", "loop_::never_ends"]
        );
    }

    #[test]
    fn a_test_that_passed_on_its_retry_is_not_a_failure() {
        let cases = parse(REPORT);
        let flaky = cases
            .iter()
            .find(|case| case.name.ends_with("passes_the_second_time"))
            .unwrap();

        assert!(!flaky.failed);
    }

    #[test]
    fn a_stress_run_says_which_repeat_each_case_is_from() {
        let report = r#"<testsuites name="nextest-run" tests="4" failures="1">
    <testsuite name="probe@stress-0" tests="2" failures="0">
        <testcase name="tests::steady" classname="probe" time="0.004"/>
        <testcase name="tests::odd_fails" classname="probe" time="0.004"/>
    </testsuite>
    <testsuite name="probe@stress-1" tests="2" failures="1">
        <testcase name="tests::steady" classname="probe" time="0.006"/>
        <testcase name="tests::odd_fails" classname="probe" time="0.006">
            <failure message="fails on 1" type="test failure with exit code 101">fails on 1</failure>
        </testcase>
    </testsuite>
</testsuites>"#;

        let cases = parse(report);

        let seen: Vec<(Option<u32>, &str, bool)> = cases
            .iter()
            .map(|case| (case.repeat, case.name.as_str(), case.failed))
            .collect();
        assert_eq!(
            seen,
            [
                (Some(0), "tests::steady", false),
                (Some(0), "tests::odd_fails", false),
                (Some(1), "tests::steady", false),
                (Some(1), "tests::odd_fails", true),
            ]
        );
    }

    #[test]
    fn an_escaped_name_is_read_as_it_was_written() {
        let report = r#"<testsuite name="a"><testcase name="tests::a&lt;b&gt;&amp;c" classname="a" time="1"/></testsuite>"#;

        assert_eq!(parse(report)[0].name, "tests::a<b>&c");
        assert_eq!(parse(report)[0].id(), "a tests::a<b>&c");
    }

    #[test]
    fn a_case_without_a_time_is_still_a_case_and_its_failure_still_a_failure() {
        let report = r#"<testsuite name="a"><testcase name="tests::x" classname="a"><failure/></testcase><testcase name="tests::y" classname="a" time="soon"/></testsuite>"#;

        assert_eq!(
            parse(report),
            [failed("a", "tests::x", 0.0), case("a", "tests::y", 0.0)]
        );
    }

    #[test]
    fn a_test_that_could_not_be_run_is_a_failure() {
        let report = r#"<testsuite name="a"><testcase name="tests::x" classname="a" time="0.1"><error type="could not start">no such file</error></testcase></testsuite>"#;

        assert_eq!(parse(report), [failed("a", "tests::x", 0.1)]);
    }

    #[test]
    fn a_test_that_needed_its_retry_is_marked() {
        let cases = parse(REPORT);
        let retried: Vec<&str> = cases
            .iter()
            .filter(|case| case.retried)
            .map(|case| case.name.as_str())
            .collect();

        assert_eq!(retried, ["timing_sensitive::sys_passes_the_second_time"]);
    }

    #[test]
    fn a_failures_text_kind_and_standard_error_are_read_and_a_pass_has_none() {
        let failures = super::failures(REPORT);
        let seen: Vec<(&str, &str, &str)> = failures
            .iter()
            .map(|failure| {
                (
                    failure.name.as_str(),
                    failure.kind.as_str(),
                    failure.text.lines().next().unwrap_or(""),
                )
            })
            .collect();

        assert_eq!(
            seen,
            [
                (
                    "loop_::stops_at_the_refused_call",
                    "test failure with exit code 101",
                    "assertion `left == right` failed"
                ),
                ("loop_::never_ends", "test timeout", ""),
                (
                    "timing_sensitive::sys_passes_the_second_time",
                    "test failure with exit code 101",
                    "no"
                ),
            ]
        );
        assert!(failures[0].text.contains(" right: [\"a\", \"b\"]"));
        // No text: the message stands in for it.
        let xml = r#"<testsuite name="a"><testcase name="x" classname="a"><error message="could not start" type="error"></error><system-err>the end
</system-err></testcase></testsuite>"#;
        let failure = &super::failures(xml)[0];
        assert_eq!(failure.text, "could not start");
        assert_eq!(failure.stderr, "the end\n");
        // A retry's element, as quick-junit writes one: its text is the
        // stack trace, and its own output is not part of it.
        let xml = r#"<testsuite name="a"><testcase name="t" classname="a"><flakyFailure message="first try" type="test failure"><stackTrace>thread &apos;t&apos; panicked at a.rs:1:1:
lost the race</stackTrace><system-out>running 1 test</system-out><system-err>noise</system-err></flakyFailure></testcase></testsuite>"#;
        let failure = &super::failures(xml)[0];
        assert_eq!(
            failure.text,
            "thread 't' panicked at a.rs:1:1:\nlost the race"
        );
        // With no trace, the message stands in, never the nested markup.
        let xml = r#"<testsuite name="a"><testcase name="t" classname="a"><flakyFailure message="first try" type="x"><system-out>running 1 test</system-out></flakyFailure></testcase></testsuite>"#;
        assert_eq!(super::failures(xml)[0].text, "first try");
    }

    #[test]
    fn an_escaped_ampersand_is_not_read_as_the_start_of_another_escape() {
        // The name `a&lt;b`, which is not the name `a<b`.
        let report = r#"<testsuite name="a"><testcase name="tests::a&amp;lt;b" classname="a" time="1"/></testsuite>"#;

        assert_eq!(parse(report)[0].name, "tests::a&lt;b");
    }

    #[test]
    fn two_runs_reports_merge_into_one_with_every_case_and_the_counts_summed() {
        let second = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="1" failures="1" errors="0" time="2.5">
    <testsuite name="cortex-tui::it" tests="1" disabled="0" errors="0" failures="1">
        <testcase name="children::sys_the_shell_restores" classname="cortex-tui::it" time="2.5">
            <failure type="test timeout"/>
        </testcase>
    </testsuite>
</testsuites>
"#;
        let merged = super::merge(REPORT, second).unwrap();

        let mut expected = parse(REPORT);
        expected.extend(parse(second));
        assert_eq!(parse(&merged), expected);
        assert_eq!(
            super::failures(&merged).len(),
            super::failures(REPORT).len() + 1
        );
        let open = merged.lines().nth(1).unwrap();
        assert_eq!(
            open,
            r#"<testsuites name="nextest-run" tests="8" failures="3" errors="0" time="3.400">"#
        );
        assert!(merged.starts_with(r#"<?xml version="1.0""#));
        assert_eq!(merged.matches("<testsuites").count(), 1);
        assert!(merged.trim_end().ends_with("</testsuites>"));
    }

    #[test]
    fn the_cases_that_failed_leave_out_a_test_that_passed_on_its_retry() {
        let names = |xml: &str| -> Vec<String> {
            super::failed(xml)
                .into_iter()
                .map(|failure| failure.name)
                .collect()
        };
        assert_eq!(
            names(REPORT),
            ["loop_::stops_at_the_refused_call", "loop_::never_ends"]
        );
        // `failures` has the retry's too.
        assert_eq!(super::failures(REPORT).len(), 3);
    }

    #[test]
    fn cases_put_in_the_place_of_others_leave_the_rest_and_the_counts_true() {
        let rerun = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="nextest-run" tests="2" failures="0" errors="0" time="0.2">
    <testsuite name="cortex-agent::it" tests="2" disabled="0" errors="0" failures="0">
        <testcase name="loop_::never_ends" classname="cortex-agent::it" timestamp="2026-11-16T09:00:00.000+00:00" time="0.040"/>
    </testsuite>
</testsuites>
"#;
        let tests = [
            (
                "cortex-agent::it".to_owned(),
                "loop_::never_ends".to_owned(),
            ),
            // Not in the re-run's report: its case stays as it was.
            (
                "cortex-agent::it".to_owned(),
                "loop_::stops_at_the_refused_call".to_owned(),
            ),
        ];
        let combined = super::replace_cases(REPORT, rerun, &tests);

        let cases = parse(&combined);
        assert_eq!(cases.len(), parse(REPORT).len());
        let timeout = cases
            .iter()
            .find(|case| case.name == "loop_::never_ends")
            .unwrap();
        assert!(!timeout.failed);
        assert!((timeout.seconds - 0.04).abs() < 1e-9);
        // The other cases are the same, in the same order, and the one the
        // re-run had no case for is still a failure.
        let mut expected = parse(REPORT);
        expected[3] = timeout.clone();
        assert_eq!(cases, expected);
        assert_eq!(super::failed(&combined).len(), 1);
        // The counts are of the cases that are there.
        assert!(
            combined.contains(
                r#"<testsuites name="nextest-run" tests="7" failures="1" errors="0" time="0.9">"#
            ),
            "{combined}"
        );
        assert!(combined.contains(
            r#"<testsuite name="cortex-agent::it" tests="3" disabled="0" errors="0" failures="1">"#
        ));
        assert!(combined.contains(
            r#"<testsuite name="cortex-base" tests="1" disabled="0" errors="0" failures="0">"#
        ));
        // A name that is only the end of another is another test.
        let none = super::replace_cases(
            REPORT,
            rerun,
            &[("cortex-agent::it".to_owned(), "never_ends".to_owned())],
        );
        assert_eq!(parse(&none), parse(REPORT));
    }

    #[test]
    fn a_report_without_its_root_is_not_merged() {
        assert!(super::merge(REPORT, "<testsuite name=\"a\"></testsuite>").is_err());
        assert!(super::merge("", REPORT).is_err());
    }
}
