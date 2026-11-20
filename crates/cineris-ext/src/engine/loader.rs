//! Modules, served only from the extension's snapshot.
//!
//! An `import` of `./lib.js` from `main.js` is resolved by its text against
//! the folder of the module that imports it, and loaded from the snapshot
//! the runtime was made from, which was checked against the pin. Nothing
//! is ever read from the disk here, so a file changed or added after the
//! snapshot is not seen, and no name can reach outside the folder: a bare
//! name (`fs`, `node:fs`), an absolute path, a URL, or a relative path that
//! climbs out of the folder is refused.

use std::sync::Arc;

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Error, Module, Result};

use crate::folder::Snapshot;

/// Resolves an import against the importing module's folder, within the
/// snapshot.
pub(crate) struct SnapshotResolver {
    pub(crate) snapshot: Arc<Snapshot>,
}

impl Resolver for SnapshotResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<String> {
        let resolved = resolve(base, name).ok_or_else(|| refused(name))?;
        if self.snapshot.file(&resolved).is_some() {
            Ok(resolved)
        } else {
            Err(Error::new_resolving_message(
                base,
                name,
                "the extension's folder has no such file",
            ))
        }
    }
}

/// Loads a module's source from the snapshot.
pub(crate) struct SnapshotLoader {
    pub(crate) snapshot: Arc<Snapshot>,
}

impl Loader for SnapshotLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> Result<Module<'js, Declared>> {
        let source = self.snapshot.file(name).ok_or_else(|| {
            Error::new_loading_message(name, "the extension's folder has no such file")
        })?;
        Module::declare(ctx.clone(), name, source.to_vec())
    }
}

fn refused(name: &str) -> Error {
    Error::new_resolving_message(
        "",
        name,
        "an import names a file in the extension's own folder, as `./file.js` or `../file.js`",
    )
}

/// The path, relative to the folder, that `name` imported from the module
/// `base` leads to: `None` for anything but a relative path that stays in
/// the folder.
pub(crate) fn resolve(base: &str, name: &str) -> Option<String> {
    if !(name.starts_with("./") || name.starts_with("../")) || name.contains('\\') {
        return None;
    }
    // The importing module's folder; a script that is not a module of the
    // folder (`eval`'s input) imports from the top.
    let mut names: Vec<&str> = if base.contains('/') {
        base.rsplit_once('/')
            .map(|(folder, _)| folder.split('/').collect())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    for part in name.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                names.pop()?;
            }
            other => names.push(other),
        }
    }
    (!names.is_empty()).then(|| names.join("/"))
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn imports_resolve_by_their_text_within_the_folder() {
        assert_eq!(resolve("main.js", "./lib.js").as_deref(), Some("lib.js"));
        assert_eq!(resolve("lib/a.js", "./b.js").as_deref(), Some("lib/b.js"));
        assert_eq!(resolve("lib/a.js", "../c.js").as_deref(), Some("c.js"));
        assert_eq!(
            resolve("lib/a.js", "./x/../d.js").as_deref(),
            Some("lib/d.js")
        );
        for outside in [
            ("main.js", "../escape.js"),
            ("lib/a.js", "../../escape.js"),
            ("main.js", "fs"),
            ("main.js", "node:fs"),
            ("main.js", "/etc/passwd"),
            ("main.js", "file:///etc/passwd"),
            ("main.js", "https://example.com/x.js"),
            ("main.js", ".\\lib.js"),
            ("main.js", "C:/x.js"),
        ] {
            assert_eq!(resolve(outside.0, outside.1), None, "{outside:?}");
        }
    }
}
