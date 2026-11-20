//! Integration tests (tier 2) of the extension host: the folder, the
//! manifest, the capabilities, the order a host call is checked in, and
//! the engine. No test waits: time is the fake timer's.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "an integration test prepares real files, fails by panicking, and says when it skips"
)]

mod capability;
mod engine;
mod folder;
mod manifest;
mod serve;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use cineris_base::path::{PathOptions, PathResolver, ResolvedPath};
use cineris_ext::folder::Snapshot;
use cineris_ext::host::{Entry, ExecOutput, ExecRequest, Host, HostError, HostFuture, Scope};
use serde_json::Value as Json;

/// A manifest with the given capabilities table, and `main.js`.
pub(crate) fn manifest(capabilities: &str) -> String {
    format!(
        "api = 1\nname = \"probe\"\nversion = \"0.1.0\"\ndescription = \"A test extension\"\nmain = \"main.js\"\n\n[capabilities]\n{capabilities}\n"
    )
}

/// A snapshot of an extension made of these files.
pub(crate) fn snapshot(files: &[(&str, &str)]) -> Arc<Snapshot> {
    let files: BTreeMap<String, Vec<u8>> = files
        .iter()
        .map(|(path, content)| ((*path).to_owned(), content.as_bytes().to_vec()))
        .collect();
    Arc::new(Snapshot::from_files(files))
}

/// An extension whose `main.js` is `script`, with these capabilities.
pub(crate) fn extension(capabilities: &str, script: &str) -> Arc<Snapshot> {
    snapshot(&[
        ("extension.toml", &manifest(capabilities)),
        ("main.js", script),
    ])
}

/// Makes a symbolic link to a file or, with `directory`, to a folder. On
/// Windows that needs a privilege an ordinary account lacks unless
/// Developer Mode is on; the CI runner has it, and there the link must be
/// made. Elsewhere `false` when it could not be, and the test says so and
/// stops.
pub(crate) fn make_link(target: &Path, link: &Path, directory: bool) -> bool {
    #[cfg(unix)]
    let made = {
        let _ = directory;
        std::os::unix::fs::symlink(target, link)
    };
    #[cfg(windows)]
    let made = if directory {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    };
    match made {
        Ok(()) => true,
        Err(error) if std::env::var_os("GITHUB_ACTIONS").is_none() => {
            println!("skipped: cannot make a link here ({error})");
            false
        }
        Err(error) => panic!("the CI runner cannot make a link: {error}"),
    }
}

/// A host that does nothing but record what it was asked, resolving paths
/// with a real resolver over a workspace.
pub(crate) struct FakeHost {
    pub(crate) resolver: Option<PathResolver>,
    pub(crate) asked: Mutex<Vec<String>>,
    pub(crate) state: Mutex<BTreeMap<String, Json>>,
    pub(crate) exec_output: ExecOutput,
}

impl FakeHost {
    pub(crate) fn new() -> Self {
        Self {
            resolver: None,
            asked: Mutex::new(Vec::new()),
            state: Mutex::new(BTreeMap::new()),
            exec_output: ExecOutput {
                code: Some(0),
                stdout: b"out".to_vec(),
                stderr: b"err".to_vec(),
                ..ExecOutput::default()
            },
        }
    }

    pub(crate) fn in_workspace(root: &Path, home: Option<&Path>) -> Self {
        let mut options = PathOptions::new();
        if let Some(home) = home {
            options = options.home(cineris_base::AbsPath::from_os(home).unwrap());
        }
        Self {
            resolver: Some(PathResolver::new(root, options).unwrap()),
            ..Self::new()
        }
    }

    pub(crate) fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    fn note(&self, what: String) {
        self.asked.lock().unwrap().push(what);
    }
}

impl Host for FakeHost {
    fn resolve<'a>(&'a self, path: &'a str) -> HostFuture<'a, Result<ResolvedPath, HostError>> {
        Box::pin(async move {
            let resolver = self
                .resolver
                .as_ref()
                .ok_or_else(|| HostError::Failed("no workspace".to_owned()))?;
            resolver
                .resolve(path)
                .map_err(|error| HostError::Failed(error.to_string()))
        })
    }

    fn exec(&self, request: ExecRequest) -> HostFuture<'_, Result<ExecOutput, HostError>> {
        Box::pin(async move {
            self.note(format!(
                "exec {} {} network={} roots={} timeout={}",
                request.program.as_str(),
                request.args.join(" "),
                request.network,
                request.write_roots.len(),
                request.timeout.as_millis()
            ));
            Ok(self.exec_output.clone())
        })
    }

    fn read(
        &self,
        path: ResolvedPath,
        _offset: u64,
        _limit: Option<u64>,
    ) -> HostFuture<'_, Result<String, HostError>> {
        Box::pin(async move {
            self.note(format!("read {}", path.shown()));
            Ok(format!("text of {}", path.shown()))
        })
    }

    fn write(&self, path: ResolvedPath, text: String) -> HostFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            self.note(format!("write {} {text}", path.shown()));
            Ok(())
        })
    }

    fn list(
        &self,
        path: ResolvedPath,
        _glob: Option<String>,
    ) -> HostFuture<'_, Result<Vec<Entry>, HostError>> {
        Box::pin(async move {
            self.note(format!("list {}", path.shown()));
            // Two entries of the workspace, one inside the manifest's
            // globs and one outside them, whatever was listed.
            let Some(resolver) = &self.resolver else {
                return Ok(Vec::new());
            };
            let entry = |relative: &str| Entry {
                path: resolver.root().join(relative).unwrap(),
                shown: relative.to_owned(),
                folder: false,
                line: None,
                text: None,
            };
            Ok(vec![entry("src/a.rs"), entry("notes.md")])
        })
    }

    fn search(
        &self,
        pattern: String,
        path: ResolvedPath,
        _glob: Option<String>,
    ) -> HostFuture<'_, Result<Vec<Entry>, HostError>> {
        Box::pin(async move {
            self.note(format!("search {pattern} {}", path.shown()));
            Ok(Vec::new())
        })
    }

    fn ask(
        &self,
        question: String,
        _choices: Vec<String>,
    ) -> HostFuture<'_, Result<Option<String>, HostError>> {
        Box::pin(async move {
            self.note(format!("ask {question}"));
            Ok(Some("yes".to_owned()))
        })
    }

    fn notify(&self, text: String) {
        self.note(format!("notify {text}"));
    }

    fn progress(&self, text: String) {
        self.note(format!("progress {text}"));
    }

    fn state_get(
        &self,
        key: String,
        scope: Scope,
    ) -> HostFuture<'_, Result<Option<Json>, HostError>> {
        Box::pin(async move {
            Ok(self
                .state
                .lock()
                .unwrap()
                .get(&format!("{scope:?}/{key}"))
                .cloned())
        })
    }

    fn state_set(
        &self,
        key: String,
        value: Option<Json>,
        scope: Scope,
    ) -> HostFuture<'_, Result<(), HostError>> {
        Box::pin(async move {
            let key = format!("{scope:?}/{key}");
            let mut state = self.state.lock().unwrap();
            match value {
                Some(value) => state.insert(key, value),
                None => state.remove(&key),
            };
            Ok(())
        })
    }

    fn log(&self, line: String) {
        self.note(format!("log {line}"));
    }
}
