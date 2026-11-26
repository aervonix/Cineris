//! The conformance test: every scenario of `cineris_testkit::conformance`
//! in process and over the wire, and what the client was sent compared
//! (`docs/architecture/11-rpc-protocol.md`, "One contract, two carriers").
//! The real binary's `serve --stdio` runs the same list in `cineris-cli`'s
//! system tests.
//!
//! Each carrier runs every scenario on one core, each scenario in a
//! workspace of its own (`work/sNN`), so that what the core knows of a
//! project never carries from one scenario to the next, and the scenarios
//! cost one core a carrier, not one each.

use cineris_testkit::conformance::{self, Carrier, Transcript};

use crate::{InProcess, Wire, World};

/// Runs every scenario on one world, through the carrier `connect` makes.
fn transcripts(
    test: &str,
    connect: impl FnOnce(&World, &crate::CoreThread) -> Box<dyn Carrier>,
) -> Vec<(&'static str, Transcript)> {
    let world = World::new(test);
    let core = world.core();
    let mut carrier = connect(&world, &core);
    let root = world.root.to_str().unwrap().to_owned();
    let server = world.server.base_url();
    let known = [(root.as_str(), "<root>"), (server.as_str(), "<server>")];
    // A fresh list: a gate a scenario opens is opened for good.
    let mut out = Vec::new();
    for (number, scenario) in conformance::scenarios().into_iter().enumerate() {
        let workspace = world.root.join("work").join(format!("s{number:02}"));
        std::fs::create_dir_all(&workspace).unwrap();
        for (path, text) in &scenario.files {
            let path = workspace.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        for reply in scenario.model.clone() {
            world.server.push(reply);
        }
        let workspace = workspace.to_str().unwrap().to_owned();
        let transcript = conformance::run(&scenario, carrier.as_mut(), &workspace, &known);
        out.push((scenario.name, transcript));
    }
    drop(carrier);
    drop(core);
    out
}

/// Says where two transcripts first differ.
pub fn compare(name: &str, first: &Transcript, second: &Transcript) {
    for (at, (one, other)) in first.0.iter().zip(&second.0).enumerate() {
        assert_eq!(
            one, other,
            "{name}: entry {at} differs between the carriers"
        );
    }
    assert_eq!(
        first.0.len(),
        second.0.len(),
        "{name}: one carrier has more entries: {:#?}",
        if first.0.len() > second.0.len() {
            &first.0[second.0.len()..]
        } else {
            &second.0[first.0.len()..]
        }
    );
}

/// What the `release` example's scenario ("a project's commands and
/// extensions") shows of its own, on one carrier's transcript, so that a
/// change that broke every carrier alike would not pass for agreement: the
/// trust question lists the project's extension `changelog`, the command
/// file's body is the prompt sent, with its arguments put in, `commands/list`
/// has both of the project's commands, and `extensions/status` says the
/// extension is `ready` (the review of step 28's close, L4).
fn the_release_scenario_says_what_the_example_does(transcript: &Transcript) {
    let entries = &transcript.0;
    let asked = entries
        .iter()
        .find(|entry| entry["request"] == "trust/request")
        .unwrap_or_else(|| panic!("no trust question: {entries:#?}"));
    assert!(
        asked["params"]["resources"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|resource| resource["kind"] == "extension" && resource["name"] == "changelog"),
        "{asked:#}"
    );
    let sent = entries
        .iter()
        .filter_map(|entry| {
            let item = &entry["event"]["item"];
            (item["command"]["name"] == "release-notes").then(|| item.to_string())
        })
        .next()
        .unwrap_or_else(|| panic!("no prompt of the command: {entries:#?}"));
    assert!(
        sent.contains(
            "Write the release notes for version 0.2.0 of this project, for the people who use it."
        ) && !sent.contains("{{")
            && !sent.contains("description:"),
        "{sent}"
    );
    let result_of = |method: &str| {
        entries
            .iter()
            .find(|entry| entry["method"] == method)
            .map(|entry| entry["result"].clone())
            .unwrap_or_else(|| panic!("no result of `{method}`: {entries:#?}"))
    };
    let listed = result_of("commands/list");
    let mut names: Vec<&str> = listed["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|command| command["name"].as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["changelog:draft", "release-notes"], "{listed:#}");
    let status = result_of("extensions/status");
    assert!(
        status["extensions"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|extension| extension["name"] == "changelog" && extension["state"] == "ready"),
        "{status:#}"
    );
}

/// Every scenario gives the same transcript in process and over the wire.
#[test]
fn every_scenario_is_the_same_in_process_and_over_the_wire() {
    // Folders of one length: a path's length shows in a token estimate.
    let in_process = transcripts("carrier-a", |_, core| Box::new(InProcess::connect(core)));
    let over_the_wire = transcripts("carrier-b", |world, core| {
        Box::new(Wire::connect(core, world.serve_options()).lines)
    });
    assert_eq!(in_process.len(), conformance::scenarios().len());
    for ((name, first), (_, second)) in in_process.iter().zip(&over_the_wire) {
        assert!(!first.0.is_empty(), "{name}: nothing was kept");
        compare(name, first, second);
    }
    let (_, release) = in_process
        .iter()
        .find(|(name, _)| *name == "a project's commands and extensions")
        .expect("the release example's scenario");
    the_release_scenario_says_what_the_example_does(release);
}
