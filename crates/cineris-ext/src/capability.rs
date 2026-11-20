//! The manifest's capabilities, compiled for one workspace, and the check
//! every host call meets first (`docs/steps/28/answered.md`, "The host
//! functions").
//!
//! A host call is checked here before anything else: refused here, it
//! never reaches the permission engine. Allowed here, it is then decided by
//! the engine as the same effect a built-in tool would declare, so the
//! manifest can only narrow what the session's policy allows, never widen
//! it.
//!
//! Paths are checked **resolved**: the caller resolves what a script gave
//! (`~`, a relative path, `..`, a link followed to where it leads, case on
//! Windows) with the session's path resolver, and the globs are matched
//! against where it leads. A glob is matched as a permission rule that
//! allows is: within the workspace, and in the spelling the file system
//! stores (`cineris_policy::glob`).

use cineris_base::path::{AbsPath, ResolvedPath};
use cineris_policy::glob::{Anchors, Case, PathGlob};

use crate::event::Event;
use crate::manifest::{Declared, HostPattern, Program};

/// Why the manifest refused a host call. The text says which capability,
/// so that a script's author and the user see what to change.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the extension's manifest does not allow this: {detail} (`capabilities.{capability}`)")]
pub struct Refusal {
    /// The capability that does not allow it: `exec`, `read`, `write`,
    /// `net` or `hooks`.
    pub capability: &'static str,
    /// What was asked for.
    pub detail: String,
}

/// Why the capabilities could not be compiled for a workspace.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`capabilities.{capability}` has `{glob}`, which is not a path glob: {reason}")]
pub struct CompileError {
    /// `read` or `write`.
    pub capability: &'static str,
    /// The glob as written.
    pub glob: String,
    /// What is wrong with it.
    pub reason: String,
}

/// The capabilities of one extension, ready to check host calls in one
/// workspace.
#[derive(Debug, Clone)]
pub struct Capabilities {
    declared: Declared,
    read: Vec<PathGlob>,
    write: Vec<PathGlob>,
    workspace: AbsPath,
}

impl Capabilities {
    /// Compiles `declared` for `workspace`, the session's resolved root.
    ///
    /// # Errors
    ///
    /// When a glob does not compile.
    pub fn compile(declared: &Declared, workspace: &AbsPath) -> Result<Self, CompileError> {
        let compile = |capability: &'static str, globs: &[String]| {
            globs
                .iter()
                .map(|glob| {
                    PathGlob::compile(
                        glob,
                        Anchors {
                            workspace,
                            home: None,
                        },
                        Case::Exact,
                    )
                    .map(|compiled| compiled.kept_within(workspace))
                    .map_err(|error| CompileError {
                        capability,
                        glob: glob.clone(),
                        reason: error.to_string(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        };
        Ok(Self {
            read: compile("read", &declared.read)?,
            write: compile("write", &declared.write)?,
            declared: declared.clone(),
            workspace: workspace.clone(),
        })
    }

    /// What the manifest declared.
    #[must_use]
    pub fn declared(&self) -> &Declared {
        &self.declared
    }

    /// The program a script names, if the manifest lists it exactly as
    /// named: `gh` is not `/usr/bin/gh`, and `gh.exe` is not `gh`.
    ///
    /// # Errors
    ///
    /// When it is not listed.
    pub fn exec(&self, program: &str) -> Result<&Program, Refusal> {
        self.declared
            .exec
            .iter()
            .find(|listed| listed.as_str() == program)
            .ok_or_else(|| Refusal {
                capability: "exec",
                detail: format!("running `{program}`"),
            })
    }

    /// Whether the extension may read where `path` leads.
    ///
    /// # Errors
    ///
    /// When it leads outside the workspace, or to a path no `read` glob
    /// names.
    pub fn read(&self, path: &ResolvedPath) -> Result<(), Refusal> {
        self.check(path, &self.read, "read", "reading")
    }

    /// Whether the extension may write where `path` leads.
    ///
    /// # Errors
    ///
    /// When it leads outside the workspace, or to a path no `write` glob
    /// names.
    pub fn write(&self, path: &ResolvedPath) -> Result<(), Refusal> {
        self.check(path, &self.write, "write", "writing")
    }

    fn check(
        &self,
        path: &ResolvedPath,
        globs: &[PathGlob],
        capability: &'static str,
        doing: &str,
    ) -> Result<(), Refusal> {
        let inside = path.is_inside_workspace() && path.abs().starts_with(&self.workspace);
        if inside && globs.iter().any(|glob| glob.matches(path.abs())) {
            Ok(())
        } else {
            Err(Refusal {
                capability,
                detail: format!("{doing} `{}`", path.shown()),
            })
        }
    }

    /// Whether the manifest lets the extension read `path`, a place a walk
    /// reached that follows no link: an entry of a listing or a search.
    #[must_use]
    pub fn reads(&self, path: &AbsPath) -> bool {
        path.starts_with(&self.workspace) && self.read.iter().any(|glob| glob.matches(path))
    }

    /// The pattern that lets the extension contact `host`, given in lower
    /// case.
    ///
    /// # Errors
    ///
    /// When no `net` pattern names it.
    pub fn host(&self, host: &str) -> Result<&HostPattern, Refusal> {
        self.declared
            .net
            .iter()
            .find(|pattern| pattern.matches(host))
            .ok_or_else(|| Refusal {
                capability: "net",
                detail: format!("contacting `{host}`"),
            })
    }

    /// Whether the manifest names `host` literally, not through a `*`: what
    /// a private or loopback address needs.
    #[must_use]
    pub fn names_host_literally(&self, host: &str) -> bool {
        self.declared
            .net
            .iter()
            .any(|pattern| pattern.names_literally(host))
    }

    /// Whether the script may hook `event`.
    ///
    /// # Errors
    ///
    /// When the manifest does not list it.
    pub fn hook(&self, event: Event) -> Result<(), Refusal> {
        if self.declared.hooks.contains(&event) {
            Ok(())
        } else {
            Err(Refusal {
                capability: "hooks",
                detail: format!(
                    "running on `{}`, which lets it see {}",
                    event.as_str(),
                    event.sees()
                ),
            })
        }
    }

    /// The folders a program the extension runs may write, where the
    /// sandbox confines it: for each `write` glob, the folder before its
    /// first wildcard (`out/**` gives `out`; `**/*.rs`, and a glob with no
    /// `/`, which matches at any depth, give the workspace; a glob that
    /// names one file gives its folder). None when the manifest has no
    /// `write`. A program may then write anything inside these folders,
    /// not only the files the globs name: the sandbox confines by folder.
    #[must_use]
    pub fn write_roots(&self) -> Vec<AbsPath> {
        let mut roots: Vec<AbsPath> = Vec::new();
        for glob in &self.declared.write {
            let root = write_root(&self.workspace, glob);
            if roots.iter().any(|known| root.starts_with(known)) {
                continue;
            }
            roots.retain(|known| !known.starts_with(&root));
            roots.push(root);
        }
        roots
    }
}

/// The folder a write glob's literal start names, within `workspace`.
fn write_root(workspace: &AbsPath, glob: &str) -> AbsPath {
    let names: Vec<&str> = glob
        .split(['/', '\\'])
        .filter(|name| !name.is_empty())
        .collect();
    // A glob with no separator matches that name at any depth.
    if names.len() < 2 {
        return workspace.clone();
    }
    let wild = |name: &str| name.contains(['*', '?', '[', '{']);
    let literal: Vec<&str> = names
        .iter()
        .copied()
        .take_while(|name| !wild(name))
        .collect();
    // All of it literal: one file, written in its folder.
    let literal = if literal.len() == names.len() {
        &literal[..literal.len() - 1]
    } else {
        &literal[..]
    };
    if literal.is_empty() {
        return workspace.clone();
    }
    workspace
        .join(&literal.join("/"))
        .unwrap_or_else(|_| workspace.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cineris_base::path::PathStyle;

    fn workspace() -> AbsPath {
        AbsPath::parse("/work/app", PathStyle::Posix).unwrap()
    }

    fn roots(globs: &[&str]) -> Vec<String> {
        let declared = Declared {
            write: globs.iter().map(|glob| (*glob).to_owned()).collect(),
            ..Declared::default()
        };
        Capabilities::compile(&declared, &workspace())
            .unwrap()
            .write_roots()
            .iter()
            .map(|root| root.as_str().to_owned())
            .collect()
    }

    #[test]
    fn write_roots_are_the_folders_before_each_globs_first_wildcard() {
        assert_eq!(roots(&["out/**"]), ["/work/app/out"]);
        assert_eq!(roots(&["src/gen/*.rs"]), ["/work/app/src/gen"]);
        assert_eq!(roots(&["notes/today.md"]), ["/work/app/notes"]);
        assert_eq!(roots(&["**/*.rs"]), ["/work/app"]);
        assert_eq!(roots(&["*.lock"]), ["/work/app"]);
        // A root inside another is the other's.
        assert_eq!(roots(&["out/a/**", "out/**"]), ["/work/app/out"]);
        assert!(roots(&[]).is_empty());
    }
}
