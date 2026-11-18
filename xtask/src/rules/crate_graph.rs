//! The crate dependency rules of `docs/architecture/01-workspace.md`, checked
//! against the manifests as `cargo metadata` reports them.
//!
//! [`CRATES`] is that page's "Depends on (workspace)" column. The two change
//! together: when the design changes, the page and this table change in the
//! same pull request.

use std::collections::BTreeSet;

use super::Violation;
use crate::workspace::{DepKind, Package};

const AGENT: &str = "cineris-agent";
const BASE: &str = "cineris-base";
const CLI: &str = "cineris-cli";
const CONFIG: &str = "cineris-config";
const COORDINATOR: &str = "cineris-coordinator";
const CORE: &str = "cineris-core";
const EXT: &str = "cineris-ext";
const INDEX: &str = "cineris-index";
const KNOWLEDGE: &str = "cineris-knowledge";
const LLM: &str = "cineris-llm";
const MCP: &str = "cineris-mcp";
const POLICY: &str = "cineris-policy";
const PROTOCOL: &str = "cineris-protocol";
const RPC: &str = "cineris-rpc";
const RUNS: &str = "cineris-runs";
const SANDBOX: &str = "cineris-sandbox";
const SESSION: &str = "cineris-session";
const STATS: &str = "cineris-stats";
const TERM: &str = "cineris-term";
const TESTKIT: &str = "cineris-testkit";
const TOOLS: &str = "cineris-tools";
const TUI: &str = "cineris-tui";
const USAGE: &str = "cineris-usage";
/// The developer commands. It may depend on anything; nothing depends on it.
const XTASK: &str = "xtask";

/// Each crate and the workspace crates it may depend on. A crate need not use
/// every edge it is allowed. The three crates that arrive after the first
/// release (runs, coordinator, usage) are listed so that their rules are in
/// force the day they are added.
const CRATES: &[(&str, &[&str])] = &[
    (PROTOCOL, &[]),
    (BASE, &[PROTOCOL]),
    (LLM, &[PROTOCOL, BASE]),
    (TOOLS, &[PROTOCOL, BASE, SANDBOX]),
    (SANDBOX, &[BASE]),
    (POLICY, &[PROTOCOL, BASE]),
    (SESSION, &[PROTOCOL, BASE]),
    (CONFIG, &[PROTOCOL, BASE]),
    (STATS, &[PROTOCOL, BASE]),
    (KNOWLEDGE, &[PROTOCOL, BASE]),
    (INDEX, &[BASE]),
    (
        AGENT,
        &[
            PROTOCOL, BASE, LLM, TOOLS, POLICY, SESSION, KNOWLEDGE, STATS,
        ],
    ),
    (EXT, &[PROTOCOL, BASE, POLICY, SANDBOX, TOOLS]),
    (RUNS, &[CORE, SESSION, STATS, KNOWLEDGE]),
    (COORDINATOR, &[RUNS, POLICY]),
    (USAGE, &[BASE, STATS]),
    (MCP, &[PROTOCOL, BASE, TOOLS, SANDBOX]),
    (
        CORE,
        &[
            PROTOCOL, BASE, LLM, TOOLS, SANDBOX, POLICY, SESSION, CONFIG, STATS, KNOWLEDGE, INDEX,
            AGENT, EXT, USAGE, MCP,
        ],
    ),
    (RPC, &[PROTOCOL, BASE, CORE]),
    (TERM, &[BASE]),
    (TUI, &[PROTOCOL, BASE, TERM]),
    (CLI, &[PROTOCOL, BASE, SANDBOX, CORE, RPC, TUI, CONFIG]),
    (TESTKIT, &[PROTOCOL, BASE, LLM, TERM]),
];

/// Workspace crates a crate's tests may use beside the ones it may depend
/// on: a dev-dependency only. `cineris-rpc`'s tests make a core as the binary
/// makes one, which takes the configuration's environment, and the shell
/// and the process runner on the fake timer (step 27).
const DEV_ONLY: &[(&str, &[&str])] = &[(RPC, &[CONFIG, TOOLS])];

/// Third-party crates that only their owner may depend on directly (rule 5),
/// so that a change elsewhere never rebuilds them and replacing one touches
/// one crate.
const CONFINED: &[(&[&str], &[&str], &str)] = &[
    (
        &[
            "reqwest",
            "hyper",
            "hyper-util",
            "hyper-rustls",
            "rustls",
            "tokio-rustls",
            "ring",
        ],
        &[LLM],
        "the HTTP and TLS stack",
    ),
    (&["rmcp"], &[MCP], "the MCP SDK"),
    (
        &["grep-searcher", "grep-regex", "grep-matcher", "globset"],
        &[TOOLS],
        "the search libraries",
    ),
    (&["ignore"], &[TOOLS, INDEX], "the file walker"),
    (&["rquickjs", "mlua"], &[EXT], "the script engine"),
    (
        &["landlock", "seccompiler"],
        &[SANDBOX],
        "the sandbox's backends",
    ),
];

/// The feature of `cineris-sandbox` that holds its backends, and the one
/// crate that may turn it on (rule 8 of `docs/architecture/01-workspace.md`).
const BACKENDS: &str = "backends";

const PAGE: &str = "docs/architecture/01-workspace.md";
const HOW_TO_CHANGE: &str = "If the design has changed, change docs/architecture/01-workspace.md \
                             and the tables in xtask/src/rules/crate_graph.rs in the same pull \
                             request.";

/// Checks every package against the rules. `manifests` holds each manifest's
/// text, keyed by the path in [`Package::manifest`].
pub(crate) fn check(packages: &[Package], manifests: &[(String, String)]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for package in packages {
        check_edges(package, &mut violations);
        check_confined(package, &mut violations);
        check_protocol_dependencies(package, &mut violations);
        check_test_hooks(package, &mut violations);
        check_sandbox_backends(package, &mut violations);
        let manifest = manifests
            .iter()
            .find(|(path, _)| *path == package.manifest)
            .map_or("", |(_, text)| text.as_str());
        check_conventions(package, manifest, &mut violations);
    }
    violations
}

/// The features that hold code for tests only: `test-hooks`, where a crate
/// puts its test-only switches (a seeded order of events), `test-support`,
/// where it puts constructors for other crates' tests (`cineris-policy`'s
/// `TestParts`), `pty`, the operating system's half of the test kit's
/// pseudo-terminal driver in `cineris-term`, and `test-kit`, `cineris-tui`'s
/// kit for driving its application (a scripted link in the core's place),
/// which the test kit cannot hold because it may not depend on `cineris-tui`.
const TEST_ONLY: &[&str] = &["test-hooks", "test-support", "pty", "test-kit"];

/// A test-only feature is turned on by dev-dependencies, and by the test kit,
/// which is itself only ever a dev-dependency, and by nothing else. Anything
/// else that turned it on would compile code meant for tests into every
/// build of the product, the released one included. There are three other
/// ways to turn a feature on, and each is refused:
///
/// - a normal or build dependency that asks for it;
/// - the crate that has it listing it in another of its features, `default`
///   above all;
/// - a feature of another crate that passes it on (`cineris-tools/test-hooks`).
fn check_test_hooks(package: &Package, violations: &mut Vec<Violation>) {
    const WHY: &str = "Why: the feature holds code for tests (a seeded order of events, \
                       docs/testing-and-ci.md; constructors for tests; pseudo-terminals; or \
                       the TUI's test kit). Asked for under `[dev-dependencies]`, or by the \
                       test kit, it is built into test builds only. Turned on any other way it \
                       is built into the released binary.";
    if package.name == TESTKIT {
        return;
    }
    for test_only in TEST_ONLY {
        for dependency in &package.dependencies {
            if dependency.kind != DepKind::Dev
                && dependency
                    .features
                    .iter()
                    .any(|feature| feature == test_only)
            {
                violations.push(Violation::new(
                    &package.manifest,
                    None,
                    format!(
                        "`{}` turns on the `{test_only}` feature of `{}` outside its \
                         dev-dependencies.\n    {WHY}\n    Move the feature to the crate's \
                         `[dev-dependencies]` entry.",
                        package.name, dependency.name
                    ),
                ));
            }
        }
        for (feature, turns_on) in &package.features {
            for turned_on in turns_on {
                // `test-hooks`, `dep/test-hooks` or `dep?/test-hooks`.
                let passed_on = turned_on
                    .rsplit_once('/')
                    .is_some_and(|(_, feature)| feature == *test_only);
                if turned_on == test_only || passed_on {
                    violations.push(Violation::new(
                        &package.manifest,
                        None,
                        format!(
                            "the feature `{feature}` of `{}` turns on `{turned_on}`.\n    {WHY}\n    \
                             Take it out of the feature: a crate's tests ask for `{test_only}` in \
                             their `[dev-dependencies]`.",
                            package.name
                        ),
                    ));
                }
            }
        }
    }
}

/// `cineris-sandbox`'s backends are turned on by `cineris-core`'s `sandbox`
/// feature and by nothing else: by a normal dependency that asks for
/// `backends`, or a feature that passes `cineris-sandbox/backends` on, in any
/// other crate. A test of a crate may ask for it in its dev-dependencies.
fn check_sandbox_backends(package: &Package, violations: &mut Vec<Violation>) {
    const WHY: &str = "Why: Landlock and seccomp are linked by the core alone. `cineris-tools` \
                       depends on `cineris-sandbox` for process trees and must not link them \
                       (docs/architecture/01-workspace.md, rule 8).";
    if package.name == CORE {
        return;
    }
    for dependency in &package.dependencies {
        if dependency.name == SANDBOX
            && dependency.kind != DepKind::Dev
            && dependency
                .features
                .iter()
                .any(|feature| feature == BACKENDS)
        {
            violations.push(Violation::new(
                &package.manifest,
                None,
                format!(
                    "`{}` turns on the `{BACKENDS}` feature of `{SANDBOX}`.\n    {WHY}",
                    package.name
                ),
            ));
        }
    }
    for (feature, turns_on) in &package.features {
        for turned_on in turns_on {
            let passed_on = turned_on.split_once('/').is_some_and(|(krate, feature)| {
                krate.trim_end_matches('?') == SANDBOX && feature == BACKENDS
            });
            if passed_on {
                violations.push(Violation::new(
                    &package.manifest,
                    None,
                    format!(
                        "the feature `{feature}` of `{}` turns on `{turned_on}`.\n    {WHY}",
                        package.name
                    ),
                ));
            }
        }
    }
}

fn allowed_for(name: &str) -> Option<&'static [&'static str]> {
    CRATES
        .iter()
        .find(|(krate, _)| *krate == name)
        .map(|(_, allowed)| *allowed)
}

fn check_edges(package: &Package, violations: &mut Vec<Violation>) {
    let name = package.name.as_str();
    let workspace_dependencies = package.dependencies.iter().filter(|dep| dep.in_workspace);

    if name == XTASK {
        return;
    }
    let Some(allowed) = allowed_for(name) else {
        violations.push(Violation::new(
            &package.manifest,
            None,
            format!(
                "`{name}` is not a crate the design knows.\n    Every crate has a row in {PAGE} \
                 saying what it is for and what it may depend on. A new crate needs one of the \
                 reasons that page gives.\n    {HOW_TO_CHANGE}"
            ),
        ));
        return;
    };

    for dependency in workspace_dependencies {
        let to = dependency.name.as_str();
        if to == TESTKIT && dependency.kind == DepKind::Dev {
            continue;
        }
        // A crate's own tests ask for its test-only feature by a
        // dev-dependency on the crate itself (`cineris-base` cannot depend
        // on the test kit, which depends on it). That adds no edge: it
        // turns a feature on, for tests only, which `check_test_hooks`
        // holds to dev-dependencies.
        if to == name && dependency.kind == DepKind::Dev {
            continue;
        }
        if dependency.kind == DepKind::Dev
            && DEV_ONLY
                .iter()
                .any(|(krate, tests)| *krate == name && tests.contains(&to))
        {
            continue;
        }
        if !allowed.contains(&to) {
            let kind = match dependency.kind {
                DepKind::Normal => "",
                DepKind::Dev => " (a dev-dependency)",
                DepKind::Build => " (a build-dependency)",
            };
            let allowed_list = if allowed.is_empty() {
                "no workspace crate".to_owned()
            } else {
                allowed.join(", ")
            };
            violations.push(Violation::new(
                &package.manifest,
                None,
                format!(
                    "`{name}` must not depend on `{to}`{kind}.\n    Allowed for {name}: \
                     {allowed_list}; and cineris-testkit as a dev-dependency.\n    Why: {}\n    \
                     {HOW_TO_CHANGE}",
                    reason(name, to)
                ),
            ));
        }
        // Rule 2: the loop is generic over the provider. It gets the types
        // and traits of cineris-llm, never its HTTP transport.
        if name == AGENT
            && to == LLM
            && (dependency.default_features || dependency.features.iter().any(|f| f == "http"))
        {
            violations.push(Violation::new(
                &package.manifest,
                None,
                format!(
                    "`{AGENT}` must depend on `{LLM}` with `default-features = false` and \
                     without the `http` feature.\n    Why: rule 2 in {PAGE}. The agent loop \
                     depends on traits, not implementations, and never links the HTTP stack. \
                     That is what lets it be tested with a scripted provider in microseconds."
                ),
            ));
        }
    }
}

/// The reason an edge is forbidden, for the failure message.
fn reason(from: &str, to: &str) -> String {
    let ui = [TUI, TERM];
    let engine = [CORE, AGENT, LLM, TOOLS];
    if to == XTASK {
        format!(
            "xtask is the developer tooling. It is not shipped and nothing depends on it ({PAGE})."
        )
    } else if to == TESTKIT {
        format!(
            "rule 6 in {PAGE}. Test support lives in cineris-testkit and is a dev-dependency \
             only, so it is never part of a shipped build."
        )
    } else if from == PROTOCOL {
        format!(
            "rule 1 in {PAGE}. cineris-protocol is the contract. It depends on no workspace \
             crate, so it stays small and stable."
        )
    } else if ui.contains(&from) && engine.contains(&to) {
        format!(
            "rule 3 in {PAGE}. The TUI and the terminal layer see only cineris-protocol types, \
             so a UI edit never rebuilds the core and the UI cannot reach around the protocol."
        )
    } else if [CORE, RPC].contains(&from) && ui.contains(&to) {
        format!(
            "rule 4 in {PAGE}. The core and the RPC server never depend on the TUI or the \
             terminal layer. A front end is a client of the core."
        )
    } else if from == USAGE {
        format!(
            "rule 7 in {PAGE}. cineris-usage reads other tools' files. It depends only on \
             cineris-base and cineris-stats, apart from everything that can send."
        )
    } else {
        format!("`{to}` is not in the \"Depends on (workspace)\" column for `{from}` in {PAGE}.")
    }
}

fn check_confined(package: &Package, violations: &mut Vec<Violation>) {
    let name = package.name.as_str();
    for dependency in package.dependencies.iter().filter(|dep| !dep.in_workspace) {
        let confined = CONFINED
            .iter()
            .find(|(crates, _, _)| crates.contains(&dependency.name.as_str()));
        if let Some((_, owners, what)) = confined
            && !owners.contains(&name)
        {
            violations.push(Violation::new(
                &package.manifest,
                None,
                format!(
                    "`{name}` must not depend on `{}` directly.\n    It is part of {what}, which \
                     only {} may link (rule 5 in {PAGE}), so that a change elsewhere does not \
                     rebuild it and replacing it touches one crate.\n    {HOW_TO_CHANGE}",
                    dependency.name,
                    owners.join(" and "),
                ),
            ));
        }
    }
}

/// What `cineris-protocol` links in a normal build, and the one more thing
/// it may link behind a feature.
const PROTOCOL_ALWAYS: [&str; 2] = ["serde", "serde_json"];
const PROTOCOL_BEHIND_A_FEATURE: [&str; 1] = ["schemars"];

/// Rule 1 goes further than workspace crates: the contract every other
/// crate builds on links serde and serde_json, and nothing else unless a
/// feature asks. Schema generation is that feature, and only `xtask` and
/// the schema test turn it on.
fn check_protocol_dependencies(package: &Package, violations: &mut Vec<Violation>) {
    if package.name != PROTOCOL {
        return;
    }
    for dependency in package.dependencies.iter().filter(|dep| !dep.in_workspace) {
        let name = dependency.name.as_str();
        let allowed = match dependency.kind {
            // Tests may use what they need; a normal build never sees it.
            DepKind::Dev => true,
            DepKind::Build => false,
            DepKind::Normal => {
                (PROTOCOL_ALWAYS.contains(&name) && !dependency.optional)
                    || (PROTOCOL_BEHIND_A_FEATURE.contains(&name) && dependency.optional)
            }
        };
        if !allowed {
            violations.push(Violation::new(
                &package.manifest,
                None,
                format!(
                    "`{PROTOCOL}` must not depend on `{name}` this way.\n    In a normal build \
                     it depends on {} and nothing else, and on {} only as an optional \
                     dependency behind the `schema` feature (rule 1 in {PAGE}).\n    Why: every \
                     crate depends on the protocol, so everything it links is linked \
                     everywhere and rebuilt first.\n    {HOW_TO_CHANGE}",
                    PROTOCOL_ALWAYS.join(" and "),
                    PROTOCOL_BEHIND_A_FEATURE.join(", "),
                ),
            ));
        }
    }
}

/// Conventions every crate follows, from the same page and from
/// `docs/testing-and-ci.md`.
fn check_conventions(package: &Package, manifest: &str, violations: &mut Vec<Violation>) {
    let name = package.name.as_str();
    let mut problem = |message: String| {
        violations.push(Violation::new(&package.manifest, None, message));
    };

    if name != XTASK {
        let expected = format!("crates/{name}/Cargo.toml");
        if !name.starts_with("cineris-") || package.manifest != expected {
            problem(format!(
                "a crate is named `cineris-<part>` and lives in `crates/cineris-<part>/` ({PAGE})."
            ));
        }
    }
    if !package.publish_disabled {
        problem(format!(
            "`{name}` must set `publish.workspace = true` (which is `publish = false`). The \
             crates are not published (decision 0038)."
        ));
    }
    if !has_workspace_lints(manifest) {
        problem(format!(
            "`{name}` must use the shared lints:\n    [lints]\n    workspace = true"
        ));
    }
    for target in &package.targets {
        let is = |kind: &str| target.kinds.iter().any(|k| k == kind);
        if is("lib") && target.doctest {
            problem(format!(
                "`{name}` must turn doc tests off:\n    [lib]\n    doctest = false\n    Each \
                 crate's doc tests are another test binary to link. Examples that must compile \
                 are ordinary tests (docs/testing-and-ci.md)."
            ));
        }
        if is("test") && !["it", "system"].contains(&target.name.as_str()) {
            problem(format!(
                "test target `{}`: a crate has one integration-test binary, `tests/it/main.rs`, \
                 with modules inside it. System tests are `tests/system/main.rs`. Every extra \
                 test binary is another link step (docs/testing-and-ci.md).",
                target.name
            ));
        }
    }

    let test_targets: BTreeSet<&str> = package
        .targets
        .iter()
        .filter(|target| target.kinds.iter().any(|k| k == "test"))
        .map(|target| target.name.as_str())
        .collect();
    if test_targets.contains("system") && ![CLI, TESTKIT].contains(&name) {
        problem(
            "system tests (`tests/system/`) belong to cineris-cli and cineris-testkit \
             (docs/testing-and-ci.md)."
                .to_owned(),
        );
    }
}

/// Whether a manifest has a `[lints]` table that says `workspace = true`.
fn has_workspace_lints(manifest: &str) -> bool {
    let mut in_lints = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_lints = line == "[lints]";
        } else if in_lints && line.replace(' ', "") == "workspace=true" {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{CRATES, check};
    use crate::workspace::{DepKind, Dependency, Package, Target};

    const GOOD_MANIFEST: &str = "[package]\nname = \"x\"\n\n[lints]\nworkspace = true\n";

    fn krate(name: &str, dependencies: Vec<Dependency>) -> Package {
        Package {
            name: name.to_owned(),
            manifest: format!("crates/{name}/Cargo.toml"),
            publish_disabled: true,
            dependencies,
            targets: vec![Target {
                name: name.replace('-', "_"),
                kinds: vec!["lib".to_owned()],
                doctest: false,
            }],
            features: Vec::new(),
        }
    }

    fn dep(name: &str) -> Dependency {
        Dependency {
            name: name.to_owned(),
            kind: DepKind::Normal,
            in_workspace: name.starts_with("cineris-") || name == "xtask",
            default_features: true,
            features: Vec::new(),
            optional: false,
        }
    }

    fn dev(name: &str) -> Dependency {
        Dependency {
            kind: DepKind::Dev,
            ..dep(name)
        }
    }

    /// The messages for one package with a well-formed manifest.
    fn messages(package: &Package) -> Vec<String> {
        let manifests = [(package.manifest.clone(), GOOD_MANIFEST.to_owned())];
        check(std::slice::from_ref(package), &manifests)
            .into_iter()
            .map(|violation| violation.to_string())
            .collect()
    }

    fn assert_one(messages: &[String], needles: &[&str]) {
        assert_eq!(messages.len(), 1, "{messages:#?}");
        for needle in needles {
            assert!(
                messages[0].contains(needle),
                "`{needle}` is not in:\n{}",
                messages[0]
            );
        }
    }

    #[test]
    fn every_edge_the_table_allows_passes() {
        for (name, allowed) in CRATES {
            let mut dependencies: Vec<Dependency> = allowed.iter().map(|to| dep(to)).collect();
            if *name == "cineris-agent" {
                for dependency in &mut dependencies {
                    dependency.default_features = dependency.name != "cineris-llm";
                }
            }
            dependencies.push(dev("cineris-testkit"));
            assert_eq!(
                messages(&krate(name, dependencies)),
                Vec::<String>::new(),
                "{name}"
            );
        }
    }

    #[test]
    fn the_table_has_no_cycles() {
        // Depth-first from every crate: following allowed edges must never
        // come back to where it started.
        fn reaches(from: &str, target: &str, depth: usize) -> bool {
            assert!(
                depth <= CRATES.len(),
                "a cycle that does not pass through {target}"
            );
            let allowed = CRATES
                .iter()
                .find(|(name, _)| *name == from)
                .map_or(&[][..], |c| c.1);
            allowed
                .iter()
                .any(|next| *next == target || reaches(next, target, depth + 1))
        }
        for (name, _) in CRATES {
            assert!(!reaches(name, name, 0), "{name} can reach itself");
        }
    }

    #[test]
    fn the_tui_must_not_depend_on_the_core() {
        let found = messages(&krate(
            "cineris-tui",
            vec![dep("cineris-protocol"), dep("cineris-core")],
        ));
        assert_one(
            &found,
            &[
                "crates/cineris-tui/Cargo.toml: `cineris-tui` must not depend on `cineris-core`",
                "Allowed for cineris-tui: cineris-protocol, cineris-base, cineris-term",
                "rule 3",
            ],
        );
    }

    #[test]
    fn the_terminal_layer_must_not_depend_on_the_agent_llm_or_tools() {
        for engine in [
            "cineris-agent",
            "cineris-llm",
            "cineris-tools",
            "cineris-core",
        ] {
            assert_one(
                &messages(&krate("cineris-term", vec![dep(engine)])),
                &["rule 3"],
            );
        }
    }

    #[test]
    fn the_core_and_rpc_must_not_depend_on_the_ui() {
        for from in ["cineris-core", "cineris-rpc"] {
            for ui in ["cineris-tui", "cineris-term"] {
                assert_one(&messages(&krate(from, vec![dep(ui)])), &["rule 4"]);
            }
        }
    }

    #[test]
    fn the_protocol_depends_on_no_workspace_crate() {
        let found = messages(&krate("cineris-protocol", vec![dep("cineris-base")]));
        assert_one(
            &found,
            &["rule 1", "Allowed for cineris-protocol: no workspace crate"],
        );
    }

    #[test]
    fn a_forbidden_dev_dependency_is_still_forbidden() {
        let found = messages(&krate("cineris-tui", vec![dev("cineris-core")]));
        assert_one(
            &found,
            &[
                "must not depend on `cineris-core` (a dev-dependency)",
                "rule 3",
            ],
        );
    }

    #[test]
    fn the_test_kit_is_a_dev_dependency_only() {
        assert!(messages(&krate("cineris-agent", vec![dev("cineris-testkit")])).is_empty());
        assert_one(
            &messages(&krate("cineris-agent", vec![dep("cineris-testkit")])),
            &["rule 6"],
        );
    }

    #[test]
    fn the_protocol_links_serde_and_serde_json_and_nothing_else() {
        let third_party = |name: &str, optional: bool| Dependency {
            in_workspace: false,
            optional,
            ..dep(name)
        };
        let as_built = vec![
            third_party("serde", false),
            third_party("serde_json", false),
            third_party("schemars", true),
        ];
        assert!(messages(&krate("cineris-protocol", as_built.clone())).is_empty());

        // Schema generation in every build.
        let mut always = as_built.clone();
        always[2].optional = false;
        assert_one(
            &messages(&krate("cineris-protocol", always)),
            &["must not depend on `schemars` this way", "rule 1"],
        );

        // Anything new.
        let mut more = as_built.clone();
        more.push(third_party("thiserror", false));
        assert_one(
            &messages(&krate("cineris-protocol", more)),
            &["must not depend on `thiserror` this way"],
        );
        let mut build_script = as_built;
        build_script.push(Dependency {
            kind: DepKind::Build,
            ..third_party("cc", false)
        });
        assert_one(
            &messages(&krate("cineris-protocol", build_script)),
            &["must not depend on `cc` this way"],
        );

        // The rule is about the protocol crate only.
        assert!(
            messages(&krate(
                "cineris-base",
                vec![third_party("thiserror", false)]
            ))
            .is_empty()
        );
    }

    #[test]
    fn the_agent_takes_the_llm_crate_without_its_http_transport() {
        let traits_only = Dependency {
            default_features: false,
            ..dep("cineris-llm")
        };
        assert!(messages(&krate("cineris-agent", vec![traits_only.clone()])).is_empty());

        let with_defaults = dep("cineris-llm");
        assert_one(
            &messages(&krate("cineris-agent", vec![with_defaults])),
            &["rule 2"],
        );

        let with_http = Dependency {
            features: vec!["http".to_owned()],
            ..traits_only
        };
        assert_one(
            &messages(&krate("cineris-agent", vec![with_http])),
            &["rule 2"],
        );
    }

    #[test]
    fn test_hooks_are_turned_on_by_dev_dependencies_only() {
        let hooks = |dependency: Dependency| Dependency {
            features: vec!["test-hooks".to_owned()],
            ..dependency
        };
        // As the loop's tests do: the crate itself without, its tests with.
        let in_tests = krate(
            "cineris-agent",
            vec![dep("cineris-tools"), hooks(dev("cineris-tools"))],
        );
        assert!(messages(&in_tests).is_empty(), "{:#?}", messages(&in_tests));

        let in_the_product = krate("cineris-core", vec![hooks(dep("cineris-tools"))]);
        assert_one(
            &messages(&in_the_product),
            &[
                "`cineris-core` turns on the `test-hooks` feature of `cineris-tools`",
                "[dev-dependencies]",
            ],
        );

        let in_a_build_script = krate(
            "cineris-core",
            vec![Dependency {
                kind: DepKind::Build,
                ..hooks(dep("cineris-tools"))
            }],
        );
        assert_one(&messages(&in_a_build_script), &["`test-hooks`"]);
    }

    #[test]
    fn test_support_is_turned_on_by_dev_dependencies_only() {
        let support = |dependency: Dependency| Dependency {
            features: vec!["test-support".to_owned()],
            ..dependency
        };
        // As the core's tests make an engine: the crate without, its tests
        // with, and the crate's own tests through a dev-dependency on
        // itself.
        let in_tests = krate(
            "cineris-core",
            vec![dep("cineris-policy"), support(dev("cineris-policy"))],
        );
        assert!(messages(&in_tests).is_empty(), "{:#?}", messages(&in_tests));
        let own_tests = krate("cineris-policy", vec![support(dev("cineris-policy"))]);
        assert!(
            messages(&own_tests).is_empty(),
            "{:#?}",
            messages(&own_tests)
        );

        let in_the_product = krate("cineris-core", vec![support(dep("cineris-policy"))]);
        assert_one(
            &messages(&in_the_product),
            &[
                "`cineris-core` turns on the `test-support` feature of `cineris-policy`",
                "[dev-dependencies]",
            ],
        );
    }

    #[test]
    fn a_crate_turns_its_own_test_hooks_on_for_its_tests_through_a_dev_dependency_on_itself() {
        let hooks = |dependency: Dependency| Dependency {
            features: vec!["test-hooks".to_owned()],
            ..dependency
        };
        // `cineris-base` cannot depend on the test kit, which depends on it:
        // its tests ask for `cineris_base::hook`'s points by a
        // dev-dependency on itself.
        let own_tests = krate("cineris-base", vec![hooks(dev("cineris-base"))]);
        assert!(
            messages(&own_tests).is_empty(),
            "{:#?}",
            messages(&own_tests)
        );

        // The same, as a normal dependency, would be in the product.
        let in_the_product = messages(&krate("cineris-base", vec![hooks(dep("cineris-base"))]));
        assert!(
            in_the_product.iter().any(|message| message
                .contains("`cineris-base` turns on the `test-hooks` feature of `cineris-base`")),
            "{in_the_product:#?}"
        );
        assert!(
            in_the_product
                .iter()
                .any(|message| message.contains("must not depend on `cineris-base`")),
            "{in_the_product:#?}"
        );
        // A dev-dependency on another crate is judged as any other.
        let on_another = krate("cineris-base", vec![dev("cineris-llm")]);
        assert_one(
            &messages(&on_another),
            &["`cineris-base` must not depend on `cineris-llm`"],
        );

        // The test kit turns the points of `cineris-base` on for every crate
        // that has it as a dev-dependency; any other crate that did so
        // would put them in the product.
        let in_the_kit = krate("cineris-testkit", vec![hooks(dep("cineris-base"))]);
        assert!(
            messages(&in_the_kit).is_empty(),
            "{:#?}",
            messages(&in_the_kit)
        );
        let elsewhere = krate("cineris-llm", vec![hooks(dep("cineris-base"))]);
        assert_one(
            &messages(&elsewhere),
            &["`cineris-llm` turns on the `test-hooks` feature of `cineris-base`"],
        );
    }

    #[test]
    fn pseudo_terminals_are_turned_on_by_tests_and_the_test_kit_only() {
        let pty = |dependency: Dependency| Dependency {
            features: vec!["pty".to_owned()],
            ..dependency
        };
        let in_tests = krate(
            "cineris-tui",
            vec![dep("cineris-term"), pty(dev("cineris-term"))],
        );
        assert!(messages(&in_tests).is_empty(), "{:#?}", messages(&in_tests));
        let in_the_kit = krate("cineris-testkit", vec![pty(dep("cineris-term"))]);
        assert!(
            messages(&in_the_kit).is_empty(),
            "{:#?}",
            messages(&in_the_kit)
        );
        let in_the_product = krate("cineris-tui", vec![pty(dep("cineris-term"))]);
        assert_one(
            &messages(&in_the_product),
            &["`cineris-tui` turns on the `pty` feature of `cineris-term`"],
        );
    }

    #[test]
    fn the_tuis_test_kit_is_turned_on_by_dev_dependencies_only() {
        let kit = |dependency: Dependency| Dependency {
            features: vec!["test-kit".to_owned()],
            ..dependency
        };
        // As `cineris-tui`'s own tests do, and as another crate's would.
        let own_tests = krate("cineris-tui", vec![kit(dev("cineris-tui"))]);
        assert!(
            messages(&own_tests).is_empty(),
            "{:#?}",
            messages(&own_tests)
        );
        let in_tests = krate(
            "cineris-cli",
            vec![dep("cineris-tui"), kit(dev("cineris-tui"))],
        );
        assert!(messages(&in_tests).is_empty(), "{:#?}", messages(&in_tests));

        // In the product: by a normal dependency, by default, or passed on.
        let in_the_product = krate("cineris-cli", vec![kit(dep("cineris-tui"))]);
        assert_one(
            &messages(&in_the_product),
            &["`cineris-cli` turns on the `test-kit` feature of `cineris-tui`"],
        );
        let by_default = Package {
            features: vec![
                ("default".to_owned(), vec!["test-kit".to_owned()]),
                ("test-kit".to_owned(), Vec::new()),
            ],
            ..krate("cineris-tui", Vec::new())
        };
        assert_one(
            &messages(&by_default),
            &["the feature `default` of `cineris-tui` turns on `test-kit`"],
        );
        let passed_on = Package {
            features: vec![("ui".to_owned(), vec!["cineris-tui/test-kit".to_owned()])],
            ..krate("cineris-cli", vec![dep("cineris-tui")])
        };
        assert_one(
            &messages(&passed_on),
            &["the feature `ui` of `cineris-cli` turns on `cineris-tui/test-kit`"],
        );
    }

    #[test]
    fn only_the_core_turns_the_sandbox_backends_on() {
        let backends = |dependency: Dependency| Dependency {
            features: vec!["backends".to_owned()],
            ..dependency
        };
        // The core turns them on through a feature of its own.
        let core = Package {
            features: vec![(
                "sandbox".to_owned(),
                vec!["cineris-sandbox/backends".to_owned()],
            )],
            ..krate("cineris-core", vec![dep("cineris-sandbox")])
        };
        assert!(messages(&core).is_empty(), "{:#?}", messages(&core));
        // The tools may not, by their dependency or by a feature.
        let tools = krate("cineris-tools", vec![backends(dep("cineris-sandbox"))]);
        assert_one(
            &messages(&tools),
            &["`cineris-tools` turns on the `backends` feature of `cineris-sandbox`"],
        );
        let forwards = Package {
            features: vec![(
                "confine".to_owned(),
                vec!["cineris-sandbox?/backends".to_owned()],
            )],
            ..krate("cineris-tools", vec![dep("cineris-sandbox")])
        };
        assert_one(
            &messages(&forwards),
            &["the feature `confine` of `cineris-tools` turns on `cineris-sandbox?/backends`"],
        );
        // Only `cineris-sandbox` names the backends themselves.
        let direct = krate(
            "cineris-tools",
            vec![dep("cineris-sandbox"), dep("landlock")],
        );
        assert_one(&messages(&direct), &["`landlock`"]);
    }

    #[test]
    fn no_feature_turns_test_hooks_on() {
        let with_features = |name: &str, features: &[(&str, &[&str])]| Package {
            features: features
                .iter()
                .map(|(feature, turns_on)| {
                    (
                        (*feature).to_owned(),
                        turns_on.iter().map(|one| (*one).to_owned()).collect(),
                    )
                })
                .collect(),
            ..krate(name, vec![dep("cineris-tools")])
        };

        // Having the feature is not turning it on.
        let has_it = with_features("cineris-agent", &[("test-hooks", &[]), ("default", &[])]);
        assert!(messages(&has_it).is_empty(), "{:#?}", messages(&has_it));

        // On by default, in the crate that has it.
        let by_default = with_features(
            "cineris-agent",
            &[("default", &["test-hooks"]), ("test-hooks", &[])],
        );
        assert_one(
            &messages(&by_default),
            &["the feature `default` of `cineris-agent` turns on `test-hooks`"],
        );

        // Passed on by a feature of a crate that depends on it, with or
        // without the mark for an optional dependency.
        for passed_on in ["cineris-tools/test-hooks", "cineris-tools?/test-hooks"] {
            let forwards = with_features("cineris-agent", &[("seeded", &[passed_on])]);
            assert_one(
                &messages(&forwards),
                &[&format!(
                    "the feature `seeded` of `cineris-agent` turns on `{passed_on}`"
                )],
            );
        }

        // Another feature with a name that only ends the same way.
        let other = with_features("cineris-agent", &[("x", &["cineris-tools/no-test-hooks"])]);
        assert!(messages(&other).is_empty(), "{:#?}", messages(&other));
    }

    #[test]
    fn the_usage_crate_stays_apart_from_everything_that_can_send() {
        assert!(
            messages(&krate(
                "cineris-usage",
                vec![dep("cineris-base"), dep("cineris-stats")]
            ))
            .is_empty()
        );
        assert_one(
            &messages(&krate("cineris-usage", vec![dep("cineris-llm")])),
            &["rule 7"],
        );
    }

    #[test]
    fn nothing_depends_on_xtask_and_xtask_may_depend_on_anything() {
        assert_one(
            &messages(&krate("cineris-base", vec![dep("xtask")])),
            &["not shipped"],
        );

        let mut xtask = krate("xtask", vec![dep("cineris-protocol"), dep("cineris-core")]);
        xtask.manifest = "xtask/Cargo.toml".to_owned();
        assert!(messages(&xtask).is_empty());
    }

    #[test]
    fn an_edge_outside_the_named_rules_points_at_the_table() {
        let found = messages(&krate("cineris-session", vec![dep("cineris-tools")]));
        assert_one(
            &found,
            &["is not in the \"Depends on (workspace)\" column for `cineris-session`"],
        );
    }

    #[test]
    fn a_crate_the_design_does_not_know_is_reported() {
        assert_one(
            &messages(&krate("cineris-extra", vec![])),
            &["is not a crate the design knows"],
        );
    }

    #[test]
    fn heavy_third_party_crates_have_one_owner() {
        assert!(messages(&krate("cineris-llm", vec![dep("reqwest")])).is_empty());
        assert!(messages(&krate("cineris-mcp", vec![dep("rmcp")])).is_empty());
        assert!(messages(&krate("cineris-index", vec![dep("ignore")])).is_empty());

        assert_one(
            &messages(&krate("cineris-agent", vec![dep("reqwest")])),
            &["rule 5", "HTTP and TLS"],
        );
        assert_one(
            &messages(&krate("cineris-core", vec![dep("rmcp")])),
            &["rule 5", "the MCP SDK"],
        );
        assert_one(
            &messages(&krate("cineris-policy", vec![dep("globset")])),
            &["rule 5"],
        );
        assert_one(
            &messages(&krate("cineris-tui", vec![dep("rquickjs")])),
            &["rule 5"],
        );
    }

    #[test]
    fn manifest_conventions_are_checked() {
        let mut publishable = krate("cineris-base", vec![]);
        publishable.publish_disabled = false;
        assert_one(&messages(&publishable), &["publish.workspace = true"]);

        let mut doctests = krate("cineris-base", vec![]);
        doctests.targets[0].doctest = true;
        assert_one(&messages(&doctests), &["doctest = false"]);

        let mut misplaced = krate("cineris-base", vec![]);
        misplaced.manifest = "cineris-base/Cargo.toml".to_owned();
        assert_one(
            &messages(&misplaced),
            &["lives in `crates/cineris-<part>/`"],
        );

        let no_lints = krate("cineris-base", vec![]);
        let manifests = [(
            no_lints.manifest.clone(),
            "[package]\nname = \"cineris-base\"\n".to_owned(),
        )];
        let found = check(&[no_lints], &manifests);
        assert_eq!(found.len(), 1);
        assert!(found[0].message.contains("[lints]"));
    }

    #[test]
    fn a_crate_has_one_integration_test_binary() {
        let test_target = |name: &str| Target {
            name: name.to_owned(),
            kinds: vec!["test".to_owned()],
            doctest: false,
        };

        let mut good = krate("cineris-agent", vec![]);
        good.targets.push(test_target("it"));
        assert!(messages(&good).is_empty());

        let mut extra = krate("cineris-agent", vec![]);
        extra.targets.push(test_target("it"));
        extra.targets.push(test_target("more"));
        assert_one(
            &messages(&extra),
            &["test target `more`", "one integration-test binary"],
        );

        let mut cli = krate("cineris-cli", vec![]);
        cli.targets.push(test_target("system"));
        assert!(messages(&cli).is_empty());

        let mut elsewhere = krate("cineris-session", vec![]);
        elsewhere.targets.push(test_target("system"));
        assert_one(
            &messages(&elsewhere),
            &["belong to cineris-cli and cineris-testkit"],
        );
    }
}
