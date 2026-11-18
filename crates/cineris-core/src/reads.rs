//! Two reads the interface's panels make (step 24): a session's stored
//! artifact, such as the whole output of a check (`session/artifact`), and
//! files a person named with `@` read ahead into the system's cache
//! (`workspace/warm`; `docs/architecture/15-efficiency.md`, "Prefetch",
//! step 3). And the commands a project's own files name, for the first run
//! (`project/detect`, since step 25).

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the reads here run on the blocking pool or on a thread of their own"
)]

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use cineris_protocol::ErrorKind;
use cineris_protocol::methods::{
    Empty, ProjectDetectParams, ProjectDetectResult, SessionArtifactParams, SessionArtifactResult,
    WorkspaceWarmParams,
};

use crate::clients::ClientId;
use crate::core::{Core, OpError, error};

/// The most of an artifact `session/artifact` gives: past it the text is
/// cut, and says so.
pub(crate) const ARTIFACT_TEXT: usize = 1 << 20;

/// The most files one `workspace/warm` reads ahead.
pub(crate) const WARM_FILES: usize = 16;

/// How much of each file is read ahead: enough for the first page the
/// `read` tool gives.
pub(crate) const WARM_BYTES: u64 = 64 * 1024;

/// `session/artifact`: the artifact's text, up to [`ARTIFACT_TEXT`], and
/// its whole size.
pub(crate) async fn artifact(
    core: &Core,
    client: ClientId,
    params: SessionArtifactParams,
) -> Result<SessionArtifactResult, OpError> {
    let live = core.open_for(client, &params.session_id)?;
    let content = Arc::clone(&live.content);
    let id = params.artifact;
    Core::blocking(move || {
        let mut reader = content.open_artifact(&id).map_err(|failure| {
            error(
                ErrorKind::Usage,
                "unknown_artifact",
                format!("The session has no artifact `{}`: {failure}", id.as_str()),
            )
        })?;
        let mut kept = Vec::new();
        let limit = u64::try_from(ARTIFACT_TEXT).unwrap_or(u64::MAX);
        let read = |failure: std::io::Error| {
            error(
                ErrorKind::Internal,
                "store_failed",
                format!("The artifact could not be read: {failure}"),
            )
        };
        (&mut reader)
            .take(limit)
            .read_to_end(&mut kept)
            .map_err(read)?;
        let rest = std::io::copy(&mut reader, &mut std::io::sink()).map_err(read)?;
        let cut = rest > 0;
        let bytes = kept.len() as u64 + rest;
        Ok(SessionArtifactResult {
            text: text_of(&kept, cut),
            bytes,
            cut,
        })
    })
    .await
}

/// Bytes as text: not UTF-8 is `U+FFFD`, and a character the cut split at
/// the end is left out rather than shown as one.
fn text_of(bytes: &[u8], cut: bool) -> String {
    let whole = if cut {
        match std::str::from_utf8(bytes) {
            // A character begun and not finished at the very end: the cut's.
            Err(failure) if failure.error_len().is_none() => &bytes[..failure.valid_up_to()],
            _ => bytes,
        }
    } else {
        bytes
    };
    String::from_utf8_lossy(whole).into_owned()
}

/// `workspace/warm`: answers at once, and reads the files on a thread of
/// its own. Only a file inside the workspace that the agent could read
/// without asking is read ([`cineris_tools::ReadGate`]), found through the
/// session's path rules, which refuse a link to a network share before the
/// network is asked; what is read is dropped.
pub(crate) fn warm(
    core: &Core,
    client: ClientId,
    params: WorkspaceWarmParams,
) -> Result<Empty, OpError> {
    let live = core.open_for(client, &params.session_id)?;
    let workspace = live.handle.state().workspace;
    let gate = cineris_agent::Policy::read_gate(live.policy.as_ref());
    let paths: Vec<String> = params.paths.into_iter().take(WARM_FILES).collect();
    if paths.is_empty() {
        return Ok(Empty {});
    }
    // A thread of its own, not the blocking pool: a read stalled on a disk
    // must not hold the core's end, and nothing waits for it.
    let _ = std::thread::Builder::new()
        .name("cineris-warm".to_owned())
        .spawn(move || warm_files(Path::new(&workspace), &paths, gate.as_ref()));
    Ok(Empty {})
}

/// Reads the start of each file the rules let through, and drops it.
/// Returns how many were read, for the tests.
pub(crate) fn warm_files(
    workspace: &Path,
    paths: &[String],
    gate: &dyn cineris_tools::ReadGate,
) -> usize {
    let Ok(resolver) = cineris_base::PathResolver::new(workspace, cineris_base::PathOptions::new())
    else {
        return 0;
    };
    let mut warmed = 0;
    for path in paths {
        let Ok(resolved) = resolver.resolve(path) else {
            continue;
        };
        if !resolved.is_inside_workspace() || !gate.may_read(resolved.abs()) {
            continue;
        }
        let Ok(file) = std::fs::File::open(resolved.to_fs_path()) else {
            continue;
        };
        if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
            continue;
        }
        if std::io::copy(&mut file.take(WARM_BYTES), &mut std::io::sink()).is_ok() {
            warmed += 1;
        }
    }
    warmed
}

/// `project/detect`: the commands the session's project names in its own
/// files, each read through the session's path resolver and its read
/// rules, on the blocking pool. Nothing is kept.
pub(crate) async fn detect(
    core: &Core,
    client: ClientId,
    params: &ProjectDetectParams,
) -> Result<ProjectDetectResult, OpError> {
    let live = core.open_for(client, &params.session_id)?;
    let workspace = live.handle.state().workspace;
    let gate = cineris_agent::Policy::read_gate(live.policy.as_ref());
    Core::blocking(move || {
        Ok(ProjectDetectResult {
            commands: detect_in(Path::new(&workspace), gate.as_ref()),
        })
    })
    .await
}

/// The commands the files at `workspace`'s root name, each file read only
/// when it is inside the workspace, the gate lets it through, and it is a
/// file no larger than `cineris_knowledge::detect::MAX_FILE`.
pub(crate) fn detect_in(
    workspace: &Path,
    gate: &dyn cineris_tools::ReadGate,
) -> Vec<cineris_protocol::project::DetectedCommand> {
    let Ok(resolver) = cineris_base::PathResolver::new(workspace, cineris_base::PathOptions::new())
    else {
        return Vec::new();
    };
    cineris_knowledge::detect::detect(|name| {
        let resolved = resolver.resolve(name).ok()?;
        if !resolved.is_inside_workspace() || !gate.may_read(resolved.abs()) {
            return None;
        }
        let file = std::fs::File::open(resolved.to_fs_path()).ok()?;
        if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
            return None;
        }
        let mut text = String::new();
        file.take(cineris_knowledge::detect::MAX_FILE + 1)
            .read_to_string(&mut text)
            .ok()?;
        (text.len() as u64 <= cineris_knowledge::detect::MAX_FILE).then_some(text)
    })
}

#[cfg(test)]
mod tests {
    use cineris_base::AbsPath;
    use cineris_testkit::Scratch;

    use super::*;

    /// A gate that refuses every path named `secret`.
    struct NoSecrets;

    impl cineris_tools::ReadGate for NoSecrets {
        fn may_read(&self, path: &AbsPath) -> bool {
            path.file_name() != Some("secret")
        }
    }

    /// Only a file inside the workspace that the gate lets through is read
    /// ahead: one outside it, one the gate refuses, a directory and one that
    /// is not there are passed over.
    #[test]
    fn only_a_file_inside_the_workspace_the_gate_lets_through_is_read_ahead() {
        let scratch = Scratch::outside("cineris-core-unit", "warm_files");
        scratch.file("ws/src/lib.rs", "pub fn f() {}\n");
        scratch.file("ws/secret", "key\n");
        scratch.file("outside.txt", "not the workspace's\n");
        std::fs::create_dir_all(scratch.join("ws/dir")).unwrap();
        let paths: Vec<String> = [
            "src/lib.rs",
            "secret",
            "../outside.txt",
            "dir",
            "missing.rs",
        ]
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
        assert_eq!(warm_files(&scratch.dir("ws"), &paths, &NoSecrets), 1);
    }

    /// Detection reads only the files the gate lets through, inside the
    /// workspace: a manifest the rules close, or one that is a link out of
    /// the workspace, gives nothing.
    #[test]
    fn detection_reads_only_what_the_gate_lets_through_inside_the_workspace() {
        let scratch = Scratch::outside("cineris-core-unit", "detect_in");
        scratch.file("ws/go.mod", "module x\n");
        scratch.file("ws/sub/Cargo.toml", "[package]\n");
        scratch.file("outside/Makefile", "test:\n\tx\n");
        std::fs::create_dir_all(scratch.join("ws/package.json")).unwrap();
        let found = detect_in(&scratch.dir("ws"), &NoSecrets);
        let lines: Vec<&str> = found.iter().map(|found| found.command.as_str()).collect();
        assert_eq!(lines, ["go build ./...", "go test ./...", "go vet ./..."]);

        /// A gate that refuses `go.mod`.
        struct NoGo;
        impl cineris_tools::ReadGate for NoGo {
            fn may_read(&self, path: &AbsPath) -> bool {
                path.file_name() != Some("go.mod")
            }
        }
        assert!(detect_in(&scratch.dir("ws"), &NoGo).is_empty());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                scratch.join("outside/Makefile"),
                scratch.join("ws/Makefile"),
            )
            .unwrap();
            assert_eq!(
                detect_in(&scratch.dir("ws"), &NoGo),
                Vec::new(),
                "a link out"
            );
        }
    }

    #[test]
    fn a_cut_text_leaves_out_the_character_it_split() {
        let text = "a€".as_bytes();
        assert_eq!(text_of(&text[..2], true), "a");
        assert_eq!(text_of(&text[..2], false), "a\u{fffd}");
        assert_eq!(text_of(b"a\xffb", true), "a\u{fffd}b");
    }
}
