//! The conformance test: every scenario of `cortex_testkit::conformance`
//! in process and over the wire, and what the client was sent compared
//! (`docs/architecture/11-rpc-protocol.md`, "One contract, two carriers").
//! The real binary's `serve --stdio` runs the same list in `cortex-cli`'s
//! system tests.

use cortex_testkit::conformance::{self, Carrier, Scenario, Transcript};

use crate::{InProcess, Wire, World};

/// Runs `scenario` on a world of its own, through the carrier `connect`
/// makes.
fn transcript(
    scenario: &Scenario,
    test: &str,
    connect: impl FnOnce(&World, &crate::CoreThread) -> Box<dyn Carrier>,
) -> Transcript {
    let world = World::new(test);
    for (path, text) in &scenario.files {
        world.put(path, text);
    }
    for reply in scenario.model.clone() {
        world.server.push(reply);
    }
    let core = world.core();
    let mut carrier = connect(&world, &core);
    let workspace = world.workspace.to_str().unwrap().to_owned();
    let root = world.root.to_str().unwrap().to_owned();
    let server = world.server.base_url();
    let known = [(root.as_str(), "<root>"), (server.as_str(), "<server>")];
    let transcript = conformance::run(scenario, carrier.as_mut(), &workspace, &known);
    drop(carrier);
    drop(core);
    transcript
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

/// Every scenario gives the same transcript in process and over the wire.
#[test]
fn every_scenario_is_the_same_in_process_and_over_the_wire() {
    for (number, scenario) in conformance::scenarios().iter().enumerate() {
        let in_process = transcript(scenario, &format!("in-process-{number}"), |_, core| {
            Box::new(InProcess::connect(core))
        });
        let over_the_wire = transcript(scenario, &format!("wire-{number}"), |world, core| {
            Box::new(Wire::connect(core, world.serve_options()).lines)
        });
        assert!(
            !in_process.0.is_empty(),
            "{}: nothing was kept",
            scenario.name
        );
        compare(scenario.name, &in_process, &over_the_wire);
    }
}
