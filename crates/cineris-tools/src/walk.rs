//! The directory walk that `list` and `search` share.
//!
//! It is the walker of the ripgrep libraries
//! (`docs/decisions/0014-search-in-process.md`), so ignore files and hidden
//! files are treated the way people expect, and it runs on several threads.
//! Around it, this module adds what the harness requires of every path:
//!
//! - **Each entry is named through `cineris-base`.** A name the resolver's
//!   rules refuse (one that is not valid Unicode, and on Windows a device
//!   name or a name that ends with a dot) is stepped past and counted, with
//!   the resolver's reason. The walk never stops for one bad name.
//! - **Each entry is put to the [`ReadGate`]**, so that a rule can protect
//!   one file inside a tree whose walk was allowed.
//! - **Links are not followed.** The walk stays under the directory the plan
//!   declared. A link is reported as a link, and the caller decides.
//! - **It stops when cancelled.**

use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use cineris_base::{AbsPath, CancelToken, ResolvedPath};
use globset::{GlobBuilder, GlobMatcher};
use ignore::{WalkBuilder, WalkState};

use crate::context::ReadGate;
use crate::output::count;
use crate::result::ToolError;

/// What to walk, and how.
pub(crate) struct WalkSpec<'a> {
    /// The directory to walk: an existing one, resolved.
    pub(crate) root: &'a ResolvedPath,
    /// How deep to go. 1 is the directory's own entries.
    pub(crate) max_depth: Option<usize>,
    /// Whether to include what ignore files and the hidden-file rule leave
    /// out.
    pub(crate) include_ignored: bool,
    /// Only entries that match this.
    pub(crate) glob: Option<&'a GlobFilter>,
    /// How many threads walk. 0 lets the walker choose.
    pub(crate) threads: usize,
}

/// A glob over the paths of a walk, relative to the directory walked.
///
/// A pattern with no `/` in it matches a name at any depth (`*.rs`). A
/// pattern with one matches the whole relative path (`src/**/*.rs`). `*`
/// never crosses a `/`; `**` does.
///
/// A pattern that holds a name as the model was shown it, escaped
/// (`a\u{200c}b.md`), matches that name too: in a glob `\u` is a `u` and
/// `{200c}` a choice of one, so the pattern is also read with the escapes
/// read back ([`cineris_base::unescape`]), and an entry matches either (the
/// independent review of step 18's fixes).
#[derive(Debug, Clone)]
pub(crate) struct GlobFilter {
    matcher: GlobMatcher,
    /// The pattern with its escapes read back, when that differs, and
    /// whether that one matches a name: on Windows the pattern as written
    /// may have a `/` (made from the `\` of an escape) that it has not.
    unescaped: Option<(GlobMatcher, bool)>,
    name_only: bool,
}

impl GlobFilter {
    /// Reads a glob. A pattern that is not a glob is refused, with the
    /// reason.
    pub(crate) fn new(pattern: &str) -> Result<Self, ToolError> {
        // On Windows a model may write the separator either way. The
        // escapes are read back first, while each `\` is still one.
        let normal = |pattern: &str| {
            let pattern = if cfg!(windows) {
                pattern.replace('\\', "/")
            } else {
                pattern.to_owned()
            };
            match pattern.strip_prefix("./") {
                Some(rest) => rest.to_owned(),
                None => pattern,
            }
        };
        let build = |pattern: &str| {
            GlobBuilder::new(pattern)
                .literal_separator(true)
                .case_insensitive(cfg!(windows))
                .build()
        };
        let unescaped = match cineris_base::unescape(pattern) {
            std::borrow::Cow::Owned(unescaped) => {
                let unescaped = normal(&unescaped);
                build(&unescaped)
                    .ok()
                    .map(|glob| (glob.compile_matcher(), !unescaped.contains('/')))
            }
            std::borrow::Cow::Borrowed(_) => None,
        };
        let pattern = normal(pattern);
        if pattern.is_empty() {
            return Err(ToolError::invalid("`glob` must not be empty."));
        }
        let glob = build(&pattern).map_err(|error| {
            ToolError::invalid(format!(
                "`glob` is not a valid glob: {}. Use `*` for part of a name, `**` for any \
                 directories, `{{a,b}}` for alternatives.",
                error.kind()
            ))
        })?;
        Ok(Self {
            matcher: glob.compile_matcher(),
            unescaped,
            name_only: !pattern.contains('/'),
        })
    }

    /// Whether an entry matches: `relative` is its path under the directory
    /// walked, with `/` between names.
    pub(crate) fn matches(&self, relative: &str) -> bool {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        let matches = |matcher: &GlobMatcher, name_only: bool| {
            matcher.is_match(if name_only { name } else { relative })
        };
        matches(&self.matcher, self.name_only)
            || self
                .unescaped
                .as_ref()
                .is_some_and(|(matcher, name_only)| matches(matcher, *name_only))
    }
}

/// What an entry of a walk is. A link is not looked through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    File,
    Dir,
    Link,
}

/// One entry of a walk, handed to the caller.
pub(crate) struct Found<'a> {
    /// Its path under the directory walked, with `/` between names.
    pub(crate) relative: &'a str,
    /// Its path for the file functions.
    pub(crate) fs_path: &'a Path,
    /// What it is.
    pub(crate) kind: EntryKind,
}

/// What a walk stepped past, for the result to say.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Skipped {
    /// Entries whose names the path rules refuse.
    pub(crate) refused: usize,
    /// Why one of them was refused: the reason that sorts first, so that
    /// the result is the same whatever order the walk's threads met them
    /// in.
    pub(crate) first_refusal: Option<String>,
    /// Entries the permission rules do not let the call read.
    pub(crate) denied: usize,
    /// Entries the file system would not give.
    pub(crate) unreadable: usize,
    /// What it said about one of them: the message that sorts first.
    pub(crate) first_error: Option<String>,
    /// Links the caller did not follow.
    pub(crate) links: usize,
}

impl Skipped {
    /// One line for each kind of thing stepped past, for the end of a
    /// result. Nothing when nothing was.
    pub(crate) fn notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        let entries = |number: usize| if number == 1 { "entry" } else { "entries" };
        if self.refused > 0 {
            notes.push(format!(
                "[Skipped {} {} whose path cannot be used. For example: {}]",
                count(self.refused),
                entries(self.refused),
                self.first_refusal.as_deref().unwrap_or("no reason given")
            ));
        }
        if self.denied > 0 {
            notes.push(format!(
                "[Skipped {} {} that the permission rules do not let this call read.]",
                count(self.denied),
                entries(self.denied)
            ));
        }
        if self.unreadable > 0 {
            notes.push(format!(
                "[Skipped {} {} that could not be read. For example: {}]",
                count(self.unreadable),
                entries(self.unreadable),
                self.first_error.as_deref().unwrap_or("no reason given")
            ));
        }
        if self.links > 0 {
            notes.push(format!(
                "[Skipped {} symbolic {}: a walk does not follow links. Name one as `path` to \
                 use what it points to.]",
                count(self.links),
                if self.links == 1 { "link" } else { "links" }
            ));
        }
        notes
    }

    /// Counts an entry whose name was refused, with the reason. A reason
    /// names the entry, and goes into a result: what would act unseen in it
    /// is an escape.
    pub(crate) fn refuse(&mut self, reason: String) {
        self.refused += 1;
        keep_smallest(
            &mut self.first_refusal,
            cineris_base::visible(&reason).into_owned(),
        );
    }

    /// Counts an entry that could not be read, with what was said of it,
    /// which may name it (the walker's errors do): what would act unseen in
    /// it is an escape.
    pub(crate) fn unreadable(&mut self, reason: &str) {
        self.unreadable += 1;
        // Escaped first, so that a name's line break does not cut the
        // reason short; then one line, and not a long one: it goes into a
        // result.
        let escaped = cineris_base::visible(reason);
        let line: String = escaped.chars().take(200).collect();
        keep_smallest(&mut self.first_error, line);
    }

    /// Adds what another count holds to this one.
    pub(crate) fn absorb(&mut self, other: Self) {
        self.refused += other.refused;
        self.denied += other.denied;
        self.unreadable += other.unreadable;
        self.links += other.links;
        if let Some(reason) = other.first_refusal {
            keep_smallest(&mut self.first_refusal, reason);
        }
        if let Some(reason) = other.first_error {
            keep_smallest(&mut self.first_error, reason);
        }
    }
}

/// Keeps whichever of two texts sorts first.
fn keep_smallest(kept: &mut Option<String>, candidate: String) {
    if kept.as_ref().is_none_or(|kept| candidate < *kept) {
        *kept = Some(candidate);
    }
}

/// The directory a thread's last entry was in, so that the entries of one
/// directory, which arrive together, build its path once.
struct Parent {
    fs_path: PathBuf,
    abs: AbsPath,
    relative: String,
}

/// What a walk calls for each entry on one of its threads.
pub(crate) type Visitor<'v> = Box<dyn FnMut(&Found<'_>) + Send + 'v>;

/// Walks a directory on several threads. Each thread gets a visitor of its
/// own from `visitor`, and calls it for each entry that may be used, in no
/// particular order.
///
/// Returns what was stepped past. When `cancel` fires the walk stops, and
/// what was visited so far is all there is: the caller checks the token.
pub(crate) fn walk<'v>(
    spec: &WalkSpec<'_>,
    gate: &dyn ReadGate,
    cancel: &CancelToken,
    visitor: &mut dyn FnMut() -> Visitor<'v>,
) -> Skipped {
    let root_fs = spec.root.to_fs_path();
    let root_abs = spec.root.abs();
    let skipped = Mutex::new(Skipped::default());
    let note = |change: &dyn Fn(&mut Skipped)| {
        change(&mut skipped.lock().unwrap_or_else(PoisonError::into_inner));
    };

    let standard = !spec.include_ignored;
    let mut builder = WalkBuilder::new(&root_fs);
    builder
        .follow_links(false)
        .max_depth(spec.max_depth)
        .threads(spec.threads)
        .hidden(standard)
        .ignore(standard)
        .git_ignore(standard)
        .git_exclude(standard)
        .parents(standard)
        // A `.gitignore` is honoured whether or not the tree is a git
        // repository: an unpacked project has one and no `.git`.
        .require_git(false)
        // The user's global git ignore file is theirs, not the project's,
        // and would make a search depend on the machine it runs on.
        .git_global(false);

    builder.build_parallel().run(|| {
        let mut visit = visitor();
        let mut parent: Option<Parent> = None;
        let root_fs = &root_fs;
        let note = &note;
        Box::new(move |entry| {
            if cancel.is_cancelled() {
                return WalkState::Quit;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    note(&|skipped| skipped.unreadable(&error.to_string()));
                    return WalkState::Continue;
                }
            };
            if entry.depth() == 0 {
                return WalkState::Continue;
            }
            let Some(file_type) = entry.file_type() else {
                return WalkState::Continue;
            };
            let is_dir = file_type.is_dir();
            // Past a directory that cannot be used, nothing is used.
            let step_past = if is_dir {
                WalkState::Skip
            } else {
                WalkState::Continue
            };

            let Some(fs_parent) = entry.path().parent() else {
                return step_past;
            };
            if parent
                .as_ref()
                .is_none_or(|known| known.fs_path != fs_parent)
            {
                match parent_of(root_fs, root_abs, fs_parent) {
                    Ok(found) => parent = Some(found),
                    Err(reason) => {
                        note(&|skipped| skipped.refuse(reason.clone()));
                        return step_past;
                    }
                }
            }
            let Some(known) = parent.as_ref() else {
                return step_past;
            };
            let abs = match known.abs.child(entry.file_name()) {
                Ok(abs) => abs,
                Err(error) => {
                    note(&|skipped| skipped.refuse(error.to_string()));
                    return step_past;
                }
            };
            if !gate.may_read(&abs) {
                note(&|skipped| skipped.denied += 1);
                return step_past;
            }

            let Some(name) = abs.file_name() else {
                return step_past;
            };
            let relative = if known.relative.is_empty() {
                name.to_owned()
            } else {
                format!("{}/{name}", known.relative)
            };
            if spec.glob.is_none_or(|glob| glob.matches(&relative)) {
                let kind = if is_dir {
                    EntryKind::Dir
                } else if file_type.is_symlink() {
                    EntryKind::Link
                } else {
                    EntryKind::File
                };
                visit(&Found {
                    relative: &relative,
                    fs_path: entry.path(),
                    kind,
                });
            }
            WalkState::Continue
        })
    });

    skipped.into_inner().unwrap_or_else(PoisonError::into_inner)
}

/// Builds the path of a directory inside the walk from the root's, one name
/// at a time, each checked as any name is.
fn parent_of(root_fs: &Path, root_abs: &AbsPath, fs_parent: &Path) -> Result<Parent, String> {
    let below = fs_parent.strip_prefix(root_fs).map_err(|_| {
        format!(
            "`{}` is not under the directory walked",
            fs_parent.display()
        )
    })?;
    let mut abs = root_abs.clone();
    let mut relative = String::new();
    for component in below.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        abs = abs.child(name).map_err(|error| error.to_string())?;
        if let Some(name) = abs.file_name() {
            if !relative.is_empty() {
                relative.push('/');
            }
            relative.push_str(name);
        }
    }
    Ok(Parent {
        fs_path: fs_parent.to_owned(),
        abs,
        relative,
    })
}

/// A path under a walked directory as data, under the directory's own:
/// for a program to use, never for text a model or a client reads.
pub(crate) fn joined_path(root: &str, relative: &str) -> String {
    if root == "." {
        relative.to_owned()
    } else if root.ends_with('/') {
        format!("{root}{relative}")
    } else {
        format!("{root}/{relative}")
    }
}

/// How a path under a walked directory is shown to the model: under the
/// directory's own shown path, with every character that would act unseen
/// written as an escape.
pub(crate) fn shown_path(root_shown: &str, relative: &str) -> String {
    let joined = joined_path(root_shown, relative);
    // A name from the walk is the workspace's: a line break in it must
    // not start what looks like another entry (the independent review
    // of step 18). Escaping what is escaped already changes nothing.
    match cineris_base::visible(&joined) {
        std::borrow::Cow::Borrowed(_) => joined,
        std::borrow::Cow::Owned(escaped) => escaped,
    }
}

/// How many files [`workspace_files`] meets, at most, for each it keeps.
const SEEN_PER_KEPT: usize = 4;

/// The files of a workspace, for a front end's file completion
/// (`workspace/files`, since step 23): their paths under `root` with `/`
/// between names, the most recently changed first, as the ignore rules, the
/// hidden-file rule and `gate` leave them, at most `limit`. Says whether
/// there were more. Blocks: it walks the tree on several threads, as
/// `search` and `list` do. The walk stops once it has met four times
/// `limit` files, so that a tree of millions (a home directory) costs a
/// bounded time and memory: the newest are then the newest of those.
#[must_use]
pub fn workspace_files(
    root: &ResolvedPath,
    gate: &dyn ReadGate,
    cancel: &CancelToken,
    limit: usize,
) -> (Vec<String>, bool) {
    let most = limit.saturating_mul(SEEN_PER_KEPT).max(1);
    files_met(root, gate, cancel, limit, most)
}

/// [`workspace_files`], stopping once `most` files have been met.
#[expect(
    clippy::disallowed_methods,
    reason = "the walk blocks, and runs on the blocking pool: each file's time is asked as it is met"
)]
fn files_met(
    root: &ResolvedPath,
    gate: &dyn ReadGate,
    cancel: &CancelToken,
    limit: usize,
    most: usize,
) -> (Vec<String>, bool) {
    let found: Mutex<Vec<(std::time::SystemTime, String)>> = Mutex::new(Vec::new());
    let stopped = std::sync::atomic::AtomicBool::new(false);
    let enough = cancel.child();
    let spec = WalkSpec {
        root,
        max_depth: None,
        include_ignored: false,
        glob: None,
        threads: 0,
    };
    let _ = walk(&spec, gate, &enough, &mut || {
        let (found, enough, stopped) = (&found, &enough, &stopped);
        Box::new(move |entry: &Found<'_>| {
            if entry.kind != EntryKind::File {
                return;
            }
            let changed = std::fs::symlink_metadata(entry.fs_path)
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            let mut found = found.lock().unwrap_or_else(PoisonError::into_inner);
            if found.len() >= most {
                stopped.store(true, std::sync::atomic::Ordering::SeqCst);
                enough.cancel();
                return;
            }
            found.push((changed, entry.relative.to_owned()));
        })
    });
    let mut found = found.into_inner().unwrap_or_else(PoisonError::into_inner);
    // Newest first; the path breaks a tie, so that the order is the same
    // whatever order the walk's threads met them in.
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let truncated = found.len() > limit || stopped.load(std::sync::atomic::Ordering::SeqCst);
    found.truncate(limit);
    (found.into_iter().map(|(_, path)| path).collect(), truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The walk for `workspace/files` stops once it has met its bound of
    /// files, and says there were more: a tree of millions costs a bounded
    /// time and memory.
    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "a test makes its tree with blocking calls"
    )]
    fn the_workspace_walk_stops_at_its_bound_and_says_there_were_more() {
        use cineris_base::{PathOptions, PathResolver, Rng as _, SystemRng};
        let dir = std::env::temp_dir().join(format!(
            "cineris-tools-walk-bound-{:016x}",
            SystemRng::new().next_u64()
        ));
        for n in 0..40 {
            let sub = dir.join(format!("d{n:02}"));
            std::fs::create_dir_all(&sub).expect("a directory");
            std::fs::write(sub.join("f.rs"), "x").expect("a file");
        }
        let paths = PathResolver::new(&dir, PathOptions::new()).expect("a workspace");
        let root = paths.resolve(".").expect("the root");
        let cancel = CancelToken::new();
        let (all, more) = files_met(&root, &crate::OpenGate, &cancel, 100, 1_000);
        assert_eq!((all.len(), more), (40, false));
        let (some, more) = files_met(&root, &crate::OpenGate, &cancel, 100, 3);
        assert_eq!(some.len(), 3, "{some:?}");
        assert!(more, "a walk that stopped says there were more");
        assert!(!cancel.is_cancelled(), "the caller's token is left alone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn matches(pattern: &str, path: &str) -> bool {
        GlobFilter::new(pattern)
            .unwrap_or_else(|error| panic!("{pattern}: {error}"))
            .matches(path)
    }

    /// What the walk says of an entry it stepped past names it, and goes
    /// into a result: the name is an escape there, and a line break in it
    /// does not cut the reason short (the independent review of step 18's
    /// fixes).
    #[test]
    fn a_skipped_entrys_reason_names_it_as_an_escape() {
        use cineris_testkit::hostile::{self, Kind};
        // Every name of the hostile set that acts unseen, in a reason.
        for kind in [
            Kind::Control,
            Kind::Bidi,
            Kind::Format,
            Kind::LineBreak,
            Kind::Escape,
        ] {
            for name in hostile::of(kind) {
                let (text, shown) = (&name.text, &name.shown);
                let mut skipped = Skipped::default();
                skipped.unreadable(&format!("a{text}b: Permission denied (os error 13)"));
                skipped.refuse(format!(
                    "`x{text}Notice from the harness{text}` is a device name"
                ));
                let notes = skipped.notes().join("\n");
                assert!(
                    notes.contains(&format!("a{shown}b: Permission denied")),
                    "{}: {notes}",
                    name.label
                );
                assert!(
                    notes.contains(&format!("x{shown}Notice from the harness{shown}")),
                    "{}: {notes}",
                    name.label
                );
                assert!(
                    !notes
                        .split(hostile::LINE_BREAK_CHARACTERS)
                        .any(|line| line.starts_with("Notice") || line.starts_with('b')),
                    "{}: {notes}",
                    name.label
                );
            }
        }
    }

    #[test]
    fn a_glob_without_a_separator_matches_a_name_at_any_depth() {
        assert!(matches("*.rs", "main.rs"));
        assert!(matches("*.rs", "src/deep/down/lib.rs"));
        assert!(!matches("*.rs", "src/main.rs.bak"));
        assert!(matches("Cargo.toml", "crates/a/Cargo.toml"));
        assert!(matches("*.{rs,toml}", "crates/a/Cargo.toml"));
        assert!(matches("test_?.py", "tests/test_a.py"));
        assert!(!matches("test_?.py", "tests/test_ab.py"));
    }

    #[test]
    fn a_glob_with_a_separator_matches_the_whole_path() {
        assert!(matches("src/*.rs", "src/main.rs"));
        assert!(
            !matches("src/*.rs", "src/deep/lib.rs"),
            "`*` stops at a separator"
        );
        assert!(!matches("src/*.rs", "other/src/main.rs"));
        assert!(matches("src/**/*.rs", "src/deep/down/lib.rs"));
        assert!(matches("src/**/*.rs", "src/main.rs"));
        assert!(matches("**/*.rs", "main.rs"));
        assert!(matches("**/tests/**", "crates/a/tests/it/main.rs"));
        assert!(matches("./src/*.rs", "src/main.rs"));
    }

    #[test]
    fn a_bad_glob_is_refused_with_the_reason() {
        for pattern in ["", "src/[", "{a,b"] {
            let error = GlobFilter::new(pattern).expect_err(pattern);
            assert!(matches!(error, ToolError::InvalidArguments(_)));
            assert!(error.to_string().starts_with("`glob` "), "{error}");
        }
    }

    #[test]
    fn what_was_skipped_is_said_once_per_kind() {
        assert!(Skipped::default().notes().is_empty());
        let mut skipped = Skipped::default();
        skipped.refuse("the path `a/\u{fffd}` is not valid Unicode".to_owned());
        skipped.refuse("another".to_owned());
        skipped.denied = 1;
        skipped.unreadable("Permission denied (os error 13)\nsecond line");
        skipped.links = 1200;
        assert_eq!(
            skipped.notes(),
            [
                "[Skipped 2 entries whose path cannot be used. For example: another]",
                "[Skipped 1 entry that the permission rules do not let this call read.]",
                "[Skipped 1 entry that could not be read. For example: Permission denied (os \
                 error 13)\\nsecond line]",
                "[Skipped 1,200 symbolic links: a walk does not follow links. Name one as \
                 `path` to use what it points to.]",
            ]
        );
    }

    #[test]
    fn a_path_is_shown_under_the_directory_walked() {
        assert_eq!(shown_path(".", "src/main.rs"), "src/main.rs");
        assert_eq!(shown_path("src", "main.rs"), "src/main.rs");
        assert_eq!(shown_path("/var/log", "syslog"), "/var/log/syslog");
        assert_eq!(shown_path("/", "etc"), "/etc");
        assert_eq!(shown_path("C:/", "Work"), "C:/Work");
    }
}
