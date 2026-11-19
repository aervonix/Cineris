//! `cargo xtask clippy -p CRATE`: one crate's lints as the workspace's check
//! sees them (`docs/testing-and-ci.md`, "One crate's lints").
//!
//! `cargo xtask check` lints the whole workspace at once, and cargo then
//! builds each crate with every feature any member asks of it: a crate's
//! own dev-dependency on itself with its test kit, a member's defaults, a
//! feature one member names on another. `cargo clippy -p cineris-cli` builds
//! only what that crate asks for, so a dependency is built with fewer
//! features, and code that is used only under one of the others is dead
//! there: `cineris-tui`'s `clock` field is read only under its `test-kit`
//! feature, which `cineris-tui`'s own dev-dependency on itself turns on, and
//! the crate alone fails with `field clock is never read`. This works out
//! the features the workspace's build turns on, from the manifests, and
//! lints the crates named with those of them that their build reaches.

use std::collections::{BTreeMap, BTreeSet};

use crate::Result;
use crate::workspace::{self, DepKind, Package};

const USAGE: &str = "\
usage: cargo xtask clippy -p CRATE... [-- CLIPPY ARGUMENT...]

Lints the crates named as `cargo xtask check` lints the workspace: every target,
--locked, warnings as errors, and with the features the workspace's build turns
on for each workspace crate the crates' build reaches (a member's defaults,
what any member's dependency on it names, and what those features turn on),
passed as --features CRATE/FEATURE. A plain `cargo clippy -p CRATE` builds a
dependency with only what CRATE asks of it, and code used under a feature that
another member turns on is then dead (`field ... is never read`). The features
it adds are printed first. Arguments after `--` go to clippy, before its own
`-D warnings`.";

/// The features each workspace crate has in a build of the whole workspace:
/// each member's `default` (a member is built with its defaults), every
/// feature a member's dependency on another workspace crate names (of any
/// kind: `--all-targets` builds the dev-dependencies too), and what each of
/// those turns on, in the crate and in its workspace dependencies, to a fixed
/// point. A `dep?/feature` is taken as `dep/feature`, and a dependency under
/// a `[target.…]` table as one on every system: more than the workspace may
/// turn on, never less.
pub(crate) fn workspace_features(packages: &[Package]) -> BTreeMap<String, BTreeSet<String>> {
    let by_name: BTreeMap<&str, &Package> = packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();
    let declared = |crate_name: &str, feature: &str| {
        by_name.get(crate_name).is_some_and(|package| {
            package
                .features
                .iter()
                .any(|(name, _)| name.as_str() == feature)
        })
    };
    let mut on: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut to_do: Vec<(String, String)> = Vec::new();
    for package in packages {
        if declared(&package.name, "default") {
            to_do.push((package.name.clone(), "default".to_owned()));
        }
        for dependency in &package.dependencies {
            if !dependency.in_workspace || !by_name.contains_key(dependency.name.as_str()) {
                continue;
            }
            for feature in &dependency.features {
                to_do.push((dependency.name.clone(), feature.clone()));
            }
        }
    }
    while let Some((crate_name, feature)) = to_do.pop() {
        if !declared(&crate_name, &feature) {
            continue;
        }
        if !on
            .entry(crate_name.clone())
            .or_default()
            .insert(feature.clone())
        {
            continue;
        }
        let Some(package) = by_name.get(crate_name.as_str()) else {
            continue;
        };
        let Some((_, turns_on)) = package.features.iter().find(|(name, _)| *name == feature) else {
            continue;
        };
        for entry in turns_on {
            if entry.starts_with("dep:") {
                continue;
            }
            match entry.split_once('/') {
                Some((dependency, feature)) => {
                    let dependency = dependency.trim_end_matches('?');
                    if by_name.contains_key(dependency) {
                        to_do.push((dependency.to_owned(), feature.to_owned()));
                    }
                }
                None => to_do.push((crate_name.clone(), entry.clone())),
            }
        }
    }
    on
}

/// The workspace crates a build of `named` with every target reaches: each
/// crate named, its dependencies of every kind (its tests and examples are
/// built), and theirs of the kinds a library's build needs (normal and
/// build), to the end.
pub(crate) fn reached(named: &[String], packages: &[Package]) -> BTreeSet<String> {
    let by_name: BTreeMap<&str, &Package> = packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut to_do: Vec<(String, bool)> = named.iter().map(|name| (name.clone(), true)).collect();
    // A crate reached as a dependency first and named later still has its
    // dev-dependencies built: it is visited again as named.
    let mut visited_as_named: BTreeSet<String> = BTreeSet::new();
    while let Some((name, as_named)) = to_do.pop() {
        let first = found.insert(name.clone());
        let named_now = as_named && visited_as_named.insert(name.clone());
        if !first && !named_now {
            continue;
        }
        let Some(package) = by_name.get(name.as_str()) else {
            continue;
        };
        for dependency in &package.dependencies {
            let built = as_named || dependency.kind != DepKind::Dev;
            if built && dependency.in_workspace && by_name.contains_key(dependency.name.as_str()) {
                to_do.push((dependency.name.clone(), false));
            }
        }
    }
    found
}

/// The `--features` value for linting `named`: `CRATE/FEATURE` for each
/// feature the workspace turns on in each crate the build reaches, in name
/// order, split in two: those cargo takes on its command line (of a crate
/// named or a direct dependency of one: cargo refuses `CRATE/FEATURE` for
/// any other, "the package does not contain these features"), and those it
/// does not, which the build then leaves off.
pub(crate) fn features_for(named: &[String], packages: &[Package]) -> (Vec<String>, Vec<String>) {
    let on = workspace_features(packages);
    let reached = reached(named, packages);
    let mut nameable: BTreeSet<&str> = named.iter().map(String::as_str).collect();
    for package in packages
        .iter()
        .filter(|package| named.contains(&package.name))
    {
        nameable.extend(
            package
                .dependencies
                .iter()
                .filter(|dependency| dependency.in_workspace)
                .map(|dependency| dependency.name.as_str()),
        );
    }
    let mut taken = Vec::new();
    let mut left = Vec::new();
    for (crate_name, features) in on {
        if !reached.contains(&crate_name) {
            continue;
        }
        let list = if nameable.contains(crate_name.as_str()) {
            &mut taken
        } else {
            &mut left
        };
        list.extend(
            features
                .into_iter()
                .map(|feature| format!("{crate_name}/{feature}")),
        );
    }
    (taken, left)
}

/// What `cargo xtask clippy` was asked for: the crates, and clippy's own
/// arguments.
#[derive(Debug, PartialEq)]
pub(crate) struct Request {
    pub(crate) packages: Vec<String>,
    pub(crate) rest: Vec<String>,
}

impl Request {
    pub(crate) fn parse(arguments: &[String]) -> Result<Self> {
        let mut request = Request {
            packages: Vec::new(),
            rest: Vec::new(),
        };
        let mut words = arguments.iter();
        while let Some(word) = words.next() {
            match word.as_str() {
                "--" => {
                    request.rest = words.cloned().collect();
                    break;
                }
                "-p" | "--package" => request.packages.push(
                    words
                        .next()
                        .ok_or_else(|| format!("`{word}` needs a crate\n\n{USAGE}"))?
                        .clone(),
                ),
                "--help" | "-h" => return Err(USAGE.to_owned()),
                other => {
                    if let Some(package) = other.strip_prefix("--package=") {
                        request.packages.push(package.to_owned());
                    } else {
                        return Err(format!("unknown argument `{other}`\n\n{USAGE}"));
                    }
                }
            }
        }
        if request.packages.is_empty() {
            return Err(format!(
                "name a crate with -p: the whole workspace is `cargo xtask check clippy`\n\n{USAGE}"
            ));
        }
        Ok(request)
    }
}

/// The arguments for cargo: clippy of the crates, every target, `--locked`,
/// the features, and the warnings as errors after clippy's own arguments.
pub(crate) fn cargo_arguments(request: &Request, features: &[String]) -> Vec<String> {
    let mut arguments = vec!["clippy".to_owned()];
    for package in &request.packages {
        arguments.extend(["-p".to_owned(), package.clone()]);
    }
    arguments.extend(["--all-targets".to_owned(), "--locked".to_owned()]);
    if !features.is_empty() {
        arguments.extend(["--features".to_owned(), features.join(",")]);
    }
    arguments.push("--".to_owned());
    arguments.extend(request.rest.iter().cloned());
    arguments.extend(["-D".to_owned(), "warnings".to_owned()]);
    arguments
}

/// `cargo xtask clippy -p CRATE… [-- ARGS]`
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let request = Request::parse(arguments)?;
    let packages = workspace::metadata(&workspace::root())?.packages;
    if let Some(unknown) = request
        .packages
        .iter()
        .find(|name| !packages.iter().any(|package| &package.name == *name))
    {
        return Err(format!("`{unknown}` is not a crate of the workspace"));
    }
    let (features, left) = features_for(&request.packages, &packages);
    if !features.is_empty() {
        println!(
            "note: with the features the workspace's build turns on: {}",
            features.join(", ")
        );
    }
    if !left.is_empty() {
        println!(
            "note: the workspace's build also turns on {}, which cargo cannot be asked for \
             here (a dependency of a dependency); name its crate with -p too if a lint in it \
             differs from `cargo xtask check`'s",
            left.join(", ")
        );
    }
    // It prints the command itself.
    crate::steps::cargo(&cargo_arguments(&request, &features))
}

#[cfg(test)]
mod tests {
    use super::{Request, cargo_arguments, features_for, reached, workspace_features};
    use crate::workspace::{DepKind, Dependency, Package};

    fn dependency(name: &str, kind: DepKind, features: &[&str]) -> Dependency {
        Dependency {
            name: name.to_owned(),
            kind,
            in_workspace: true,
            default_features: true,
            features: features
                .iter()
                .map(|feature| (*feature).to_owned())
                .collect(),
            optional: false,
        }
    }

    fn package(name: &str, dependencies: Vec<Dependency>, features: &[(&str, &[&str])]) -> Package {
        Package {
            name: name.to_owned(),
            manifest: format!("crates/{name}/Cargo.toml"),
            publish_disabled: true,
            dependencies,
            targets: Vec::new(),
            features: features
                .iter()
                .map(|(name, turns_on)| {
                    (
                        (*name).to_owned(),
                        turns_on.iter().map(|each| (*each).to_owned()).collect(),
                    )
                })
                .collect(),
        }
    }

    /// The shape of the workspace that broke `cargo clippy -p cineris-cli`:
    /// the TUI turns its test kit on for its own tests, the core's default
    /// turns the sandbox's backends on, the LLM crate's defaults are off
    /// where the CLI asks for it and on where it is a member.
    fn workspace() -> Vec<Package> {
        let mut llm_off = dependency("llm", DepKind::Normal, &[]);
        llm_off.default_features = false;
        let mut outside = dependency("serde", DepKind::Normal, &["derive"]);
        outside.in_workspace = false;
        vec![
            package(
                "cli",
                vec![
                    dependency("core", DepKind::Normal, &[]),
                    dependency("tui", DepKind::Normal, &[]),
                    llm_off,
                    outside,
                ],
                &[],
            ),
            package(
                "core",
                vec![
                    dependency("sandbox", DepKind::Normal, &[]),
                    dependency("llm", DepKind::Normal, &["http"]),
                    dependency("testkit", DepKind::Dev, &[]),
                ],
                &[
                    ("default", &["sandbox"]),
                    ("sandbox", &["sandbox/backends"]),
                    ("mcp", &["dep:mcp"]),
                ],
            ),
            package(
                "tui",
                vec![dependency("tui", DepKind::Dev, &["test-kit"])],
                &[
                    ("default", &["highlight"]),
                    ("highlight", &["dep:syntect"]),
                    ("test-kit", &[]),
                    ("demo", &[]),
                ],
            ),
            package(
                "sandbox",
                Vec::new(),
                &[("backends", &["dep:landlock", "optional?/x"])],
            ),
            package(
                "llm",
                Vec::new(),
                &[("default", &["http"]), ("http", &[]), ("other", &[])],
            ),
            package(
                "testkit",
                vec![dependency("base", DepKind::Normal, &["test-hooks"])],
                &[],
            ),
            package("base", Vec::new(), &[("test-hooks", &[])]),
        ]
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).to_owned()).collect()
    }

    #[test]
    fn the_workspaces_features_are_every_members_defaults_and_what_any_member_names() {
        let on = workspace_features(&workspace());
        let of = |name: &str| {
            on.get(name)
                .map(|features| features.iter().map(String::as_str).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        assert_eq!(of("core"), ["default", "sandbox"], "mcp is not asked for");
        assert_eq!(of("sandbox"), ["backends"], "through core's default");
        assert_eq!(of("tui"), ["default", "highlight", "test-kit"]);
        assert_eq!(of("llm"), ["default", "http"]);
        assert_eq!(of("base"), ["test-hooks"]);
        assert_eq!(of("cli"), Vec::<&str>::new());
        assert!(!on.contains_key("serde"), "a crate outside the workspace");
    }

    #[test]
    fn a_build_reaches_the_named_crates_dev_dependencies_and_no_one_elses() {
        let found = reached(&strings(&["cli"]), &workspace());
        assert_eq!(
            found.into_iter().collect::<Vec<_>>(),
            ["cli", "core", "llm", "sandbox", "tui"],
            "core's dev-dependency on the test kit is not built"
        );
        let found = reached(&strings(&["sandbox", "core"]), &workspace());
        assert!(
            found.contains("testkit") && found.contains("base"),
            "{found:?}"
        );
        // Reached as a dependency first, named later: its dev-dependencies
        // are built all the same.
        let found = reached(&strings(&["core", "cli"]), &workspace());
        assert!(found.contains("testkit"), "{found:?}");
    }

    #[test]
    fn a_crate_is_linted_with_the_features_the_workspace_gives_what_it_reaches() {
        let (taken, left) = features_for(&strings(&["cli"]), &workspace());
        assert_eq!(
            taken,
            [
                "core/default",
                "core/sandbox",
                "llm/default",
                "llm/http",
                "tui/default",
                "tui/highlight",
                "tui/test-kit",
            ]
        );
        assert_eq!(
            left,
            ["sandbox/backends"],
            "a dependency of a dependency: cargo refuses to be asked for it"
        );
        assert_eq!(
            features_for(&strings(&["base"]), &workspace()),
            (strings(&["base/test-hooks"]), Vec::new())
        );
        let (taken, left) = features_for(&strings(&["cli", "core"]), &workspace());
        assert!(taken.contains(&"sandbox/backends".to_owned()), "{taken:?}");
        assert!(left.contains(&"base/test-hooks".to_owned()), "{left:?}");
    }

    #[test]
    fn the_command_is_the_checks_clippy_for_the_crates_named() {
        let request =
            Request::parse(&strings(&["-p", "cli", "--package=tui", "--", "-W", "x"])).unwrap();
        assert_eq!(request.packages, ["cli", "tui"]);
        assert_eq!(
            cargo_arguments(&request, &strings(&["tui/test-kit", "llm/default"])).join(" "),
            "clippy -p cli -p tui --all-targets --locked --features tui/test-kit,llm/default -- \
             -W x -D warnings"
        );
        assert_eq!(
            cargo_arguments(&Request::parse(&strings(&["-p", "x"])).unwrap(), &[]).join(" "),
            "clippy -p x --all-targets --locked -- -D warnings"
        );
        for (words, said) in [
            (&[][..], "name a crate with -p"),
            (&["-p"], "needs a crate"),
            (&["--workspace"], "unknown argument"),
        ] {
            let error = Request::parse(&strings(words)).unwrap_err();
            assert!(error.contains(said), "{words:?}: {error}");
        }
    }
}
