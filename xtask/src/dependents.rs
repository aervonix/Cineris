//! `cargo xtask test --dependents -p CRATE`: the crate's tests and those of
//! every workspace crate whose tests can see a change to it.
//!
//! A crate's tests see a change to `CRATE` when the crate is built on top
//! of it: it depends on it, as a normal or a build dependency, directly or
//! through other workspace crates. And a crate's tests see it when they
//! depend on such a crate themselves, as a dev-dependency. A dev-dependency
//! is not passed on: the tests of a crate that depends on `cineris-tools` do
//! not use `cineris-tools`' own test helpers, so the chain stops at it.
//!
//! Step 18 changed `cineris-tools`, ran its tests and those of the crate it
//! was working in, and broke a `cineris-core` test it never ran: the core
//! reaches `cineris-tools` through `cineris-agent`.

use std::collections::BTreeSet;

use crate::Result;
use crate::workspace::{self, DepKind, Package};

/// The packages whose tests can see a change to any of `changed`, the
/// changed ones included, in name order. A name that is not a workspace
/// package is an error.
pub(crate) fn dependents(packages: &[Package], changed: &[String]) -> Result<Vec<String>> {
    for name in changed {
        if !packages.iter().any(|package| &package.name == name) {
            return Err(format!("`{name}` is not a package of this workspace"));
        }
    }
    // Built on top of a changed crate: through normal and build edges, to
    // any depth.
    let mut built_on: BTreeSet<String> = changed.iter().cloned().collect();
    loop {
        let before = built_on.len();
        for package in packages {
            let builds_on = package.dependencies.iter().any(|dependency| {
                dependency.in_workspace
                    && dependency.kind != DepKind::Dev
                    && built_on.contains(&dependency.name)
            });
            if builds_on {
                built_on.insert(package.name.clone());
            }
        }
        if built_on.len() == before {
            break;
        }
    }
    // And whose tests use one of those, through a dev-dependency.
    let mut tested: BTreeSet<String> = built_on.clone();
    for package in packages {
        let tests_use = package.dependencies.iter().any(|dependency| {
            dependency.in_workspace
                && dependency.kind == DepKind::Dev
                && built_on.contains(&dependency.name)
        });
        if tests_use {
            tested.insert(package.name.clone());
        }
    }
    Ok(tested.into_iter().collect())
}

/// `--dependents` in a list of arguments, resolved.
#[derive(Debug, PartialEq)]
pub(crate) struct Expanded {
    /// The arguments with the flag taken out and `-p` for each crate that
    /// is named or a dependent of one.
    pub(crate) arguments: Vec<String>,
    /// The crates the arguments named.
    pub(crate) named: Vec<String>,
    /// The crates added: those whose tests can see a change to a named one.
    pub(crate) added: Vec<String>,
}

impl Expanded {
    /// What to tell of it: the crates named and the ones added.
    pub(crate) fn said(&self) -> String {
        format!(
            "--dependents: {} and {} crate(s) whose tests can see it: {}",
            self.named.join(", "),
            self.added.len(),
            if self.added.is_empty() {
                "none".to_owned()
            } else {
                self.added.join(", ")
            }
        )
    }
}

/// The arguments of `cargo xtask test --dependents …`: the packages named
/// replaced by them and their dependents, and the flag taken out. Arguments
/// without the flag come back as they are, and `None` is returned.
pub(crate) fn expand_in(arguments: &[String], packages: &[Package]) -> Result<Option<Expanded>> {
    let (before, after) = split_at_separator(arguments);
    if !before.iter().any(|word| word == "--dependents") {
        return Ok(None);
    }
    let named = workspace::packages_named(&before);
    if named.is_empty() {
        return Err("--dependents needs the crates whose dependents to test: -p CRATE".to_owned());
    }
    let all = dependents(packages, &named)?;
    let added: Vec<String> = all
        .iter()
        .filter(|name| !named.contains(name))
        .cloned()
        .collect();
    Ok(Some(Expanded {
        arguments: replace_packages(&before, &all, &after),
        named,
        added,
    }))
}

/// The arguments of `cargo xtask test --dependents …`, resolved against this
/// workspace, with what was added said. Arguments without the flag come back
/// as they are.
pub(crate) fn expand(arguments: &[String]) -> Result<Vec<String>> {
    let (before, _) = split_at_separator(arguments);
    if !before.iter().any(|word| word == "--dependents") {
        return Ok(arguments.to_vec());
    }
    let metadata = workspace::metadata(&workspace::root())?;
    let Some(expanded) = expand_in(arguments, &metadata.packages)? else {
        return Ok(arguments.to_vec());
    };
    println!("{}", expanded.said());
    Ok(expanded.arguments)
}

/// `arguments` split before their `--`, which goes with what follows it.
fn split_at_separator(arguments: &[String]) -> (Vec<String>, Vec<String>) {
    match arguments.iter().position(|word| word == "--") {
        Some(at) => (arguments[..at].to_vec(), arguments[at..].to_vec()),
        None => (arguments.to_vec(), Vec::new()),
    }
}

/// `before` without `--dependents` and its package choices, with `-p` for
/// each of `packages`, then `after`.
fn replace_packages(before: &[String], packages: &[String], after: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut words = before.iter();
    while let Some(word) = words.next() {
        if word == "--dependents" || word.starts_with("--package=") {
            continue;
        }
        if word == "-p" || word == "--package" {
            words.next();
            continue;
        }
        if word.starts_with("-p") && !word.starts_with("--") {
            continue;
        }
        out.push(word.clone());
    }
    for package in packages {
        out.extend(["-p".to_owned(), package.clone()]);
    }
    out.extend(after.iter().cloned());
    out
}

#[cfg(test)]
mod tests {
    use super::{dependents, expand_in, replace_packages};
    use crate::workspace::{DepKind, Dependency, Package};

    fn package(name: &str, edges: &[(&str, DepKind)]) -> Package {
        Package {
            name: name.to_owned(),
            manifest: format!("crates/{name}/Cargo.toml"),
            publish_disabled: true,
            dependencies: edges
                .iter()
                .map(|(to, kind)| Dependency {
                    name: (*to).to_owned(),
                    kind: *kind,
                    in_workspace: true,
                    default_features: true,
                    features: Vec::new(),
                    optional: false,
                })
                .collect(),
            targets: Vec::new(),
            features: Vec::new(),
        }
    }

    /// base <- tools <- agent <- core <- cli; testkit dev-uses tools and is
    /// a dev-dependency of agent; stats is unrelated; gen build-depends on
    /// base; lone dev-depends on core.
    /// Listed with each crate before what it depends on, so that one pass
    /// over the list finds only the direct dependents.
    fn workspace() -> Vec<Package> {
        use DepKind::{Build, Dev, Normal};
        vec![
            package("cli", &[("core", Normal)]),
            package("core", &[("agent", Normal)]),
            package("agent", &[("tools", Normal), ("testkit", Dev)]),
            package("tools", &[("base", Normal)]),
            package("base", &[]),
            package("testkit", &[("base", Normal), ("tools", Dev)]),
            package("stats", &[("base", Dev)]),
            package("gen", &[("base", Build)]),
            package("lone", &[("core", Dev)]),
            package("other", &[]),
        ]
    }

    #[test]
    fn an_indirect_dependent_is_included() {
        let found = dependents(&workspace(), &["tools".to_owned()]).expect("known");
        // core reaches tools only through agent; cli through core; lone's
        // tests use core.
        assert_eq!(found, ["agent", "cli", "core", "lone", "testkit", "tools"]);
    }

    #[test]
    fn a_dev_dependency_is_not_passed_on() {
        // testkit dev-uses tools, and agent dev-uses testkit: agent is in
        // only because it builds on tools, so a change to testkit alone
        // reaches agent and nothing past it that uses it only for tests.
        let found = dependents(&workspace(), &["testkit".to_owned()]).expect("known");
        assert_eq!(found, ["agent", "testkit"]);
    }

    #[test]
    fn build_and_dev_edges_to_the_crate_count() {
        let found = dependents(&workspace(), &["base".to_owned()]).expect("known");
        assert_eq!(
            found,
            [
                "agent", "base", "cli", "core", "gen", "lone", "stats", "testkit", "tools"
            ]
        );
        assert!(!found.contains(&"other".to_owned()));
    }

    #[test]
    fn an_unknown_crate_is_an_error() {
        let error = dependents(&workspace(), &["nope".to_owned()]).expect_err("unknown");
        assert!(error.contains("`nope`"), "{error}");
    }

    #[test]
    fn the_packages_named_are_replaced_and_the_rest_kept() {
        let words = |text: &str| text.split(' ').map(str::to_owned).collect::<Vec<_>>();
        let out = replace_packages(
            &words(
                "--dependents -p tools --profile fast -pbase --package=x --package y -E test(a)",
            ),
            &["agent".to_owned(), "tools".to_owned()],
            &words("-- --nocapture"),
        );
        assert_eq!(
            out,
            words("--profile fast -E test(a) -p agent -p tools -- --nocapture")
        );
    }

    #[test]
    fn the_flag_is_resolved_against_the_packages_given_and_said() {
        let words = |text: &str| text.split(' ').map(str::to_owned).collect::<Vec<_>>();
        let expanded = expand_in(
            &words("--dependents -p testkit -E x -- --nocapture"),
            &workspace(),
        )
        .expect("known")
        .expect("the flag is there");
        assert_eq!(expanded.named, ["testkit"]);
        assert_eq!(expanded.added, ["agent"]);
        assert_eq!(
            expanded.arguments,
            words("-E x -p agent -p testkit -- --nocapture")
        );
        assert_eq!(
            expanded.said(),
            "--dependents: testkit and 1 crate(s) whose tests can see it: agent"
        );
        // Without the flag, or with it only past the command's own `--`,
        // there is nothing to resolve.
        assert_eq!(expand_in(&words("-p tools"), &workspace()), Ok(None));
        assert_eq!(
            expand_in(&words("-p tools -- --dependents"), &workspace()),
            Ok(None)
        );
        let error = expand_in(&words("--dependents"), &workspace()).unwrap_err();
        assert!(error.contains("-p CRATE"), "{error}");
    }
}
