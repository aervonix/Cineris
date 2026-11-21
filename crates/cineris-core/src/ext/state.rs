//! What an extension keeps: `cineris.state` (`docs/architecture/12-extensions.md`,
//! "Host functions"; `docs/steps/28/answered.md`, points 2 and 4).
//!
//! - **Whose values.** An extension's values are keyed by where it comes
//!   from and its name ([`Owner`]), so a project's `tracker` never reads or
//!   writes the user's `tracker`'s values, in either scope.
//! - **`session` values** are kept in the session's log: each change is one
//!   `ext` record, `{"ext": "user/tracker", "data": {"state": {"key": …,
//!   "value": …}}}`, a `null` value removing the key. They are read back
//!   from the log the first time an extension of the session uses them. A
//!   record belongs to the session, not to a branch: a resume has the values
//!   as the session left them, a rewind leaves them, and a fork copies the
//!   records written before its point (`cineris_session::Log::fork`), so it
//!   has them as they were there. A session with no log keeps them in
//!   memory.
//! - **`project` values** are kept in the state directory, in one file an
//!   extension and project: `projects/<project key>/extensions/<source>-<name>.json`,
//!   `{"version": 1, "values": {…}}`. It is read strictly: a file this build
//!   would not have written (an unknown field, another version, bad JSON) is
//!   refused with the reason and left as it is, and the call is refused, so
//!   nothing is read from it or written over it. It is changed under its
//!   lock, from before the read until after the write, and written whole,
//!   owner-only and atomically.
//! - **The caps.** 64 KiB of values an extension a scope
//!   ([`STATE_MAX_BYTES`]): a set that would take them past it, and grow
//!   them, is refused with the reason, and nothing is written. A set that
//!   changes nothing writes nothing. **And 16 MiB of `session` records an
//!   extension a session** ([`STATE_MAX_SESSION_LOG`]): a key set again
//!   adds a line each time, so the cap on the values alone does not bound
//!   the log. Each record counts as its `ext` and `data` in compact JSON
//!   and [`LINE_ALLOWANCE`] for the rest of its line, counted from the
//!   records already in the log when the values are first read, so a
//!   resume does not start it again; a set past it, a removal included, is
//!   refused with the reason, and nothing is written. A session with no
//!   log writes nothing, and has none.
//!
//! No call here is a permission question: the design gives `state` no
//! capability and no effect, since it touches only the extension's own
//! values.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use cineris_base::{AtomicWriteOptions, CancelToken, SharedClock, write_atomic};
use cineris_config::StateDir;
use cineris_ext::host::{HostError, STATE_MAX_BYTES, STATE_MAX_KEY, STATE_MAX_SESSION_LOG, Scope};
use cineris_ext::source::Source;
use cineris_protocol::{ProjectKey, TurnId};
use cineris_session::Body;
use cineris_session::entry::{ExtRecord, Kind};
use cineris_tools::run_blocking;
use serde_json::{Map, Value as Json, json};

use crate::store::SharedLog;

/// The version of a project's file this build writes, and the only one it
/// reads.
pub(crate) const FILE_VERSION: u64 = 1;

/// The most of a project's file that is read. A file this build wrote is
/// far smaller: its values are at most [`STATE_MAX_BYTES`].
const FILE_MAX_BYTES: u64 = 1024 * 1024;

/// What a record of a `session` value counts for its line's envelope
/// (`v`, `id`, `parent`, `ts`, `turn`, `kind`, and `redacted` when it is
/// there), besides its own `ext` and `data`: about 90 bytes in a session's
/// first ten thousand entries, and more as the ids and the turns grow.
pub(crate) const LINE_ALLOWANCE: usize = 128;

/// One extension's values, by key.
type Values = Map<String, Json>;

/// A session's values, and what each extension's records of them hold in
/// its log.
#[derive(Debug, Default)]
struct SessionValues {
    /// The values, by [`Owner::id`].
    values: HashMap<String, Values>,
    /// The bytes of each extension's records in the log, by [`Owner::id`],
    /// as [`record_bytes`] counts them.
    written: HashMap<String, usize>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Each change behind these locks is whole before the lock is let go.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn failed(message: impl Into<String>) -> HostError {
    HostError::Failed(message.into())
}

/// Whose values: where an extension comes from, and its name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Owner {
    source: Source,
    name: String,
}

impl Owner {
    pub(crate) fn new(source: Source, name: &str) -> Self {
        Self {
            source,
            name: name.to_owned(),
        }
    }

    /// Its id in the session's log: `user/tracker`, `project/tracker`.
    pub(crate) fn id(&self) -> String {
        format!("{}/{}", self.source.as_str(), self.name)
    }

    /// The name of its file in a project's folder: `user-tracker.json`.
    /// The source in front keeps a name such as `con` from being one that
    /// Windows keeps for a device. A name is ASCII letters, digits, `_`,
    /// `-` and `.`, not starting with `.` (the manifest's rules), so it is
    /// one file name; one that is not is refused here as well.
    fn file_name(&self) -> Result<String, HostError> {
        let plain = !self.name.is_empty()
            && !self.name.starts_with('.')
            && self
                .name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-.".contains(character));
        if plain {
            Ok(format!("{}-{}.json", self.source.as_str(), self.name))
        } else {
            Err(failed(
                "the extension's name cannot be a file's: its project values cannot be kept",
            ))
        }
    }
}

/// Where a project's values are kept: its folder in the state directory.
#[derive(Debug, Clone)]
pub(crate) struct ProjectStore {
    /// The folder, `<state dir>/projects/<project key>/extensions`.
    dir: PathBuf,
    /// The folder as the script and the user are told it: relative to the
    /// state directory, with `/`.
    shown: String,
}

impl ProjectStore {
    /// The project's folder in `state`. `None` when the project's key
    /// cannot be a directory's name.
    pub(crate) fn new(state: &StateDir, project: &ProjectKey) -> Option<Self> {
        let dir = state.project_dir(project)?.join("extensions");
        let shown = dir.strip_prefix(state.root()).map_or_else(
            |_| "extensions".to_owned(),
            |relative| relative.to_string_lossy().replace('\\', "/"),
        );
        Some(Self { dir, shown })
    }

    /// The file of `owner`'s values, and how it is named to the script.
    fn file(&self, owner: &Owner) -> Result<(PathBuf, String), HostError> {
        let name = owner.file_name()?;
        Ok((self.dir.join(&name), format!("{}/{name}", self.shown)))
    }
}

/// The values of a session's extensions, and where they are kept.
pub(crate) struct ExtState {
    /// The session's log, given once it is made, which is after the
    /// extensions' tools are registered.
    log: OnceLock<SharedLog>,
    /// The `session` values and what their records hold in the log: `None`
    /// until they are read from the log, at their first use.
    session: Mutex<Option<SessionValues>>,
    /// The most bytes an extension's `session` records may hold in the log
    /// ([`STATE_MAX_SESSION_LOG`], or less).
    session_budget: usize,
    /// Where the project's values are kept: `None` with no state directory.
    project: Option<ProjectStore>,
    clock: SharedClock,
}

impl std::fmt::Debug for ExtState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtState")
            .field("project", &self.project)
            .field("session_budget", &self.session_budget)
            .finish_non_exhaustive()
    }
}

impl ExtState {
    /// The values of a session about to be put together: nothing is read
    /// until an extension asks. `session_budget` is what an extension's
    /// `session` records may hold in the log, never more than
    /// [`STATE_MAX_SESSION_LOG`]: `None` is that.
    pub(crate) fn new(
        project: Option<ProjectStore>,
        clock: SharedClock,
        session_budget: Option<usize>,
    ) -> Self {
        Self {
            log: OnceLock::new(),
            session: Mutex::new(None),
            session_budget: session_budget.map_or(STATE_MAX_SESSION_LOG, |budget| {
                budget.min(STATE_MAX_SESSION_LOG)
            }),
            project,
            clock,
        }
    }

    /// Gives the session's log, where the `session` values are kept. Only
    /// the first one given is kept.
    pub(crate) fn attach(&self, log: SharedLog) {
        let _ = self.log.set(log);
    }

    /// `owner`'s value of `key`: `None` when it has none.
    pub(crate) async fn get(
        self: &Arc<Self>,
        owner: Owner,
        key: String,
        scope: Scope,
    ) -> Result<Option<Json>, HostError> {
        let state = Arc::clone(self);
        blocking(move || match scope {
            Scope::Session => state.session_get(&owner, &key),
            Scope::Project => state.project_get(&owner, &key),
        })
        .await
    }

    /// Sets `owner`'s value of `key` to `value`, or removes it with `None`.
    /// A `session` change is recorded in `turn`.
    pub(crate) async fn set(
        self: &Arc<Self>,
        owner: Owner,
        key: String,
        value: Option<Json>,
        scope: Scope,
        turn: Option<TurnId>,
    ) -> Result<(), HostError> {
        let state = Arc::clone(self);
        blocking(move || match scope {
            Scope::Session => state.session_set(&owner, key, value, turn.as_ref()),
            Scope::Project => state.project_set(&owner, &key, value),
        })
        .await
    }

    fn session_get(&self, owner: &Owner, key: &str) -> Result<Option<Json>, HostError> {
        let mut slot = lock(&self.session);
        let all = self.session_values(&mut slot)?;
        Ok(all
            .values
            .get(&owner.id())
            .and_then(|values| values.get(key))
            .cloned())
    }

    fn session_set(
        &self,
        owner: &Owner,
        key: String,
        value: Option<Json>,
        turn: Option<&TurnId>,
    ) -> Result<(), HostError> {
        let mut slot = lock(&self.session);
        let all = self.session_values(&mut slot)?;
        let id = owner.id();
        let empty = Values::new();
        let current = all.values.get(&id).unwrap_or(&empty);
        let Some(after) = apply(current, &key, value.as_ref())? else {
            return Ok(());
        };
        let log = self
            .log
            .get()
            .ok_or_else(|| failed("the session's log is not open yet"))?;
        if let Some(disk) = lock(log).disk() {
            let data = json!({"state": {"key": key, "value": value}});
            let bytes = record_bytes(&id, &data);
            let written = all.written.get(&id).copied().unwrap_or(0);
            within_budget(written, bytes, self.session_budget)?;
            let record = Body::Ext(ExtRecord {
                ext: id.clone(),
                data,
            });
            disk.session
                .append(turn, record)
                .map_err(|error| unseen_path("the session's log could not be written", &error))?;
            all.written
                .insert(id.clone(), written.saturating_add(bytes));
        }
        // In memory only once the log has it: what is read is what a resume
        // would read.
        all.values.insert(id, after);
        Ok(())
    }

    /// The `session` values, read from the log the first time.
    fn session_values<'a>(
        &self,
        slot: &'a mut Option<SessionValues>,
    ) -> Result<&'a mut SessionValues, HostError> {
        if slot.is_none() {
            *slot = Some(self.read_session()?);
        }
        Ok(slot.get_or_insert_with(SessionValues::default))
    }

    /// Every extension's `session` values, as the log's records leave
    /// them, in the order they were written, and what those records hold.
    fn read_session(&self) -> Result<SessionValues, HostError> {
        let log = self
            .log
            .get()
            .ok_or_else(|| failed("the session's log is not open yet"))?;
        let mut kept = lock(log);
        let Some(disk) = kept.disk() else {
            return Ok(SessionValues::default());
        };
        let records = disk
            .session
            .records(&Kind::Ext)
            .map_err(|error| unseen_path("the session's log could not be read", &error))?;
        let mut all = SessionValues::default();
        for entry in records {
            let Body::Ext(record) = entry.body else {
                continue;
            };
            let Some((key, value)) = change_of(&record) else {
                continue;
            };
            // What the log already holds counts against the budget: a
            // resume does not start it again.
            let written = all.written.entry(record.ext.clone()).or_default();
            *written = written.saturating_add(record_bytes(&record.ext, &record.data));
            let values = all.values.entry(record.ext.clone()).or_default();
            match value {
                Some(value) => {
                    values.insert(key.to_owned(), value.clone());
                }
                None => {
                    values.remove(key);
                }
            }
        }
        Ok(all)
    }

    fn project_get(&self, owner: &Owner, key: &str) -> Result<Option<Json>, HostError> {
        let (path, shown) = self.project_file(owner)?;
        Ok(read_file(&path, &shown)?.remove(key))
    }

    fn project_set(&self, owner: &Owner, key: &str, value: Option<Json>) -> Result<(), HostError> {
        let (path, shown) = self.project_file(owner)?;
        // From before the read until after the write: another session of
        // the project changing the file meanwhile would otherwise have its
        // change written over (it makes the folder, owner-only).
        let _held = cineris_base::private::hold_lock(&path).map_err(|error| {
            unseen_path(
                &format!("`{shown}` in the state directory cannot be locked"),
                &error,
            )
        })?;
        let before = read_file(&path, &shown)?;
        let Some(after) = apply(&before, key, value.as_ref())? else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(&json!({"version": FILE_VERSION, "values": after}))
            .map_err(|error| failed(format!("the values cannot be written: {error}")))?;
        let options = AtomicWriteOptions {
            owner_only: true,
            follow_links: false,
            ..AtomicWriteOptions::default()
        };
        write_atomic(&path, &bytes, &options, self.clock.as_ref()).map_err(|error| {
            unseen_path(
                &format!("`{shown}` in the state directory could not be written"),
                &error,
            )
        })
    }

    fn project_file(&self, owner: &Owner) -> Result<(PathBuf, String), HostError> {
        self.project
            .as_ref()
            .ok_or_else(|| {
                failed(
                    "there is no state directory, so an extension's `project` values cannot be \
                     kept",
                )
            })?
            .file(owner)
    }
}

/// The failure `what`, for the script, with `error` in the harness's log
/// alone: its text names where the user's files are (the session's log,
/// the state directory, under their home), which is not the script's to
/// learn or to hand to the model.
fn unseen_path(what: &str, error: &dyn std::fmt::Display) -> HostError {
    tracing::warn!(%error, "{what}");
    failed(format!(
        "the value could not be kept or read: {what} (the harness's log says why)"
    ))
}

/// Runs `work` on the blocking pool, with a token of its own that is never
/// cancelled: a change is made whole or not at all, and the script is told
/// which, whatever becomes of its call meanwhile.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, HostError> + Send + 'static,
) -> Result<T, HostError> {
    run_blocking(&CancelToken::new(), move |_| work())
        .await
        .map_err(|error| failed(error.to_string()))?
}

/// A record's change of a `session` value: its key, and its value, `None`
/// where it removes the key. `None` for a record that is not one of
/// `cineris.state`'s.
fn change_of(record: &ExtRecord) -> Option<(&str, Option<&Json>)> {
    let state = record.data.get("state")?;
    let key = state.get("key")?.as_str()?;
    Some((key, state.get("value").filter(|value| !value.is_null())))
}

/// The values after `key` is set to `value` (`None` removes it): `None`
/// when that changes nothing.
///
/// # Errors
///
/// When they would be over [`STATE_MAX_BYTES`] and larger than before:
/// a change that does not grow them is always taken.
fn apply(before: &Values, key: &str, value: Option<&Json>) -> Result<Option<Values>, HostError> {
    if before.get(key) == value {
        return Ok(None);
    }
    let mut after = before.clone();
    match value {
        Some(value) => {
            after.insert(key.to_owned(), value.clone());
        }
        None => {
            after.remove(key);
        }
    }
    let (was, would_be) = (bytes_of(before), bytes_of(&after));
    if would_be > STATE_MAX_BYTES && would_be > was {
        return Err(failed(format!(
            "the value is not kept: the extension's values in this scope would be {would_be} \
             bytes of JSON, more than the {} KiB it may keep",
            STATE_MAX_BYTES / 1024
        )));
    }
    Ok(Some(after))
}

/// The bytes of `values` as the cap counts them: their compact JSON.
fn bytes_of(values: &Values) -> usize {
    serde_json::to_vec(values).map_or(usize::MAX, |bytes| bytes.len())
}

/// What a `session` record of the extension `ext` holding `data` counts
/// against its budget: its `ext` and `data` as compact JSON, and
/// [`LINE_ALLOWANCE`] for the rest of its line. Counted the same from what
/// is about to be written and from what a resume reads back.
pub(crate) fn record_bytes(ext: &str, data: &Json) -> usize {
    serde_json::to_vec(&json!({"ext": ext, "data": data}))
        .map_or(usize::MAX, |bytes| bytes.len())
        .saturating_add(LINE_ALLOWANCE)
}

/// Refuses a record of `bytes` that would take an extension's `session`
/// records past `budget`, `written` being what they hold already.
fn within_budget(written: usize, bytes: usize, budget: usize) -> Result<(), HostError> {
    let would_be = written.saturating_add(bytes);
    if would_be > budget {
        let most = if budget.is_multiple_of(1024 * 1024) {
            format!("{} MiB", budget / (1024 * 1024))
        } else {
            format!("{budget} bytes")
        };
        return Err(failed(format!(
            "the value is not kept: the extension's records in this session's log would be \
             {would_be} bytes, more than the {most} it may write to one session (each change is \
             a record of its own); a new session starts again"
        )));
    }
    Ok(())
}

/// The values in a project's file at `path`, which the script is told of
/// as `shown`: none when there is no file.
///
/// # Errors
///
/// When there is a file and it is not one this build wrote, read strictly
/// ([`parse_file`]): the call is refused with the reason, and the file is
/// left as it is.
#[allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "reading the file blocks; it runs on the blocking pool"
)]
fn read_file(path: &Path, shown: &str) -> Result<Values, HostError> {
    use std::io::Read as _;
    let refused = |why: String| {
        tracing::warn!(path = %path.display(), "an extension's project values are refused: {why}");
        failed(format!(
            "the extension's values for this project cannot be read, so none was read or \
             kept: `{shown}` in the state directory {why}. It is left as it is: put it right, \
             or delete it to start again"
        ))
    };
    let (file, _) = match cineris_base::open_regular(path) {
        Ok(found) => found,
        Err(cineris_base::OpenError::NotFound) => return Ok(Values::new()),
        Err(error) => return Err(refused(format!("cannot be opened: {error}"))),
    };
    let mut bytes = Vec::new();
    file.take(FILE_MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| refused(format!("cannot be read: {error}")))?;
    if bytes.len() as u64 > FILE_MAX_BYTES {
        return Err(refused(format!(
            "is larger than the {} KiB this build reads",
            FILE_MAX_BYTES / 1024
        )));
    }
    parse_file(&bytes).map_err(refused)
}

/// A project's file, read strictly: exactly `version`, which is
/// [`FILE_VERSION`], and `values`, an object whose keys are 1 to
/// [`STATE_MAX_KEY`] bytes and whose values are not `null`. Anything else
/// is a file this build did not write, and the reason is given.
pub(crate) fn parse_file(bytes: &[u8]) -> Result<Values, String> {
    let shown = |text: &str| {
        cineris_base::visible(cineris_base::text::truncate_bytes(text, 80)).into_owned()
    };
    let json: Json =
        serde_json::from_slice(bytes).map_err(|error| format!("is not JSON ({error})"))?;
    let Json::Object(mut fields) = json else {
        return Err("is not a JSON object".to_owned());
    };
    // The version first: a newer one may have fields this build does not
    // know, and is said to be newer.
    match fields.get("version") {
        None => return Err("has no `version`".to_owned()),
        Some(version) => match version.as_u64() {
            Some(FILE_VERSION) => {}
            Some(newer) if newer > FILE_VERSION => {
                return Err(format!(
                    "was written by a newer version of Cineris (its version is {newer}, and this \
                     build reads {FILE_VERSION})"
                ));
            }
            _ => {
                return Err(format!(
                    "has a `version` this build does not read ({})",
                    shown(&version.to_string())
                ));
            }
        },
    }
    if let Some(unknown) = fields
        .keys()
        .find(|name| !matches!(name.as_str(), "version" | "values"))
    {
        return Err(format!(
            "has a field this build does not know, `{}`",
            shown(unknown)
        ));
    }
    let Some(Json::Object(values)) = fields.remove("values") else {
        return Err("has no `values` object".to_owned());
    };
    if let Some(key) = values
        .keys()
        .find(|key| key.is_empty() || key.len() > STATE_MAX_KEY)
    {
        return Err(format!(
            "has a key of {} bytes, where a key is 1 to {STATE_MAX_KEY}",
            key.len()
        ));
    }
    if let Some((key, _)) = values.iter().find(|(_, value)| value.is_null()) {
        return Err(format!(
            "has `null` for `{}`, which a set never writes: `null` removes a key",
            shown(key)
        ));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_extensions_values_are_keyed_by_where_it_comes_from_and_its_name() {
        let user = Owner::new(Source::User, "tracker");
        let project = Owner::new(Source::Project, "tracker");
        assert_eq!(user.id(), "user/tracker");
        assert_eq!(project.id(), "project/tracker");
        assert_eq!(user.file_name().unwrap(), "user-tracker.json");
        assert_eq!(project.file_name().unwrap(), "project-tracker.json");
        // A name Windows keeps for a device is not the file's whole name.
        assert_eq!(
            Owner::new(Source::User, "con").file_name().unwrap(),
            "user-con.json"
        );
        for name in ["", ".hidden", "a/b", "a\\b", "..", "a b"] {
            assert!(
                Owner::new(Source::User, name).file_name().is_err(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_file_this_build_wrote_is_read() {
        let values = parse_file(br#"{"version":1,"values":{"a":1,"b":{"c":[true]}}}"#).unwrap();
        assert_eq!(values["a"], json!(1));
        assert_eq!(values["b"], json!({"c": [true]}));
        assert!(
            parse_file(br#"{"values":{},"version":1}"#)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_file_this_build_did_not_write_is_refused_with_the_reason() {
        let long = format!(r#"{{"version":1,"values":{{"{}":1}}}}"#, "k".repeat(257));
        for (file, reason) in [
            ("{", "is not JSON"),
            ("[]", "is not a JSON object"),
            (r#"{"values":{}}"#, "has no `version`"),
            (r#"{"version":2,"values":{}}"#, "newer version"),
            (r#"{"version":2,"values":{},"more":1}"#, "newer version"),
            (r#"{"version":0,"values":{}}"#, "does not read (0)"),
            (r#"{"version":"1","values":{}}"#, "does not read"),
            (
                r#"{"version":1,"values":{},"more":1}"#,
                "does not know, `more`",
            ),
            (r#"{"version":1}"#, "has no `values`"),
            (r#"{"version":1,"values":[]}"#, "has no `values`"),
            (r#"{"version":1,"values":{"":1}}"#, "a key of 0 bytes"),
            (long.as_str(), "a key of 257 bytes"),
            (r#"{"version":1,"values":{"a":null}}"#, "`null` for `a`"),
        ] {
            let refused = parse_file(file.as_bytes()).unwrap_err();
            assert!(refused.contains(reason), "{file}: {refused}");
        }
    }

    /// The cap is on the compact JSON of all the values: at it is kept, one
    /// byte over is not, and a change that does not grow them is taken.
    #[test]
    fn the_values_are_kept_up_to_the_cap_and_no_further() {
        let edge = STATE_MAX_BYTES - r#"{"k":""}"#.len();
        let full = apply(&Values::new(), "k", Some(&json!("x".repeat(edge))))
            .unwrap()
            .unwrap();
        assert_eq!(bytes_of(&full), STATE_MAX_BYTES);
        assert!(apply(&Values::new(), "k", Some(&json!("x".repeat(edge + 1)))).is_err());
        let refused = apply(&full, "n", Some(&json!(1))).unwrap_err();
        assert!(refused.to_string().contains("64 KiB"), "{refused}");
        // Over the cap already (a file put right by hand): what shrinks
        // them is taken, and what grows them is not.
        let mut over = full.clone();
        over.insert("n".to_owned(), json!(12345));
        assert!(apply(&over, "n", None).unwrap().is_some());
        assert!(apply(&over, "n", Some(&json!(1))).unwrap().is_some());
        assert!(apply(&over, "n", Some(&json!(123_456))).is_err());
    }

    /// A record counts its `ext` and `data` and the allowance for its line;
    /// at the budget it is written, one byte past it it is not.
    #[test]
    fn the_session_records_are_held_to_their_budget() {
        let data = json!({"state": {"key": "k", "value": 1}});
        let own = serde_json::to_vec(&json!({"ext": "user/probe", "data": data}))
            .unwrap()
            .len();
        assert_eq!(record_bytes("user/probe", &data), own + LINE_ALLOWANCE);
        assert!(within_budget(0, 1000, 1000).is_ok());
        assert!(within_budget(400, 600, 1000).is_ok());
        let refused = within_budget(401, 600, 1000).unwrap_err().to_string();
        assert!(
            refused.contains("1001 bytes, more than the 1000 bytes"),
            "{refused}"
        );
        let refused = within_budget(STATE_MAX_SESSION_LOG, 1, STATE_MAX_SESSION_LOG)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("the 16 MiB"), "{refused}");
    }

    #[test]
    fn a_set_that_changes_nothing_writes_nothing() {
        let mut values = Values::new();
        values.insert("a".to_owned(), json!(1));
        assert!(apply(&values, "a", Some(&json!(1))).unwrap().is_none());
        assert!(apply(&values, "b", None).unwrap().is_none());
        assert!(apply(&values, "a", Some(&json!(2))).unwrap().is_some());
        assert!(apply(&values, "a", None).unwrap().is_some());
    }

    #[test]
    fn only_a_records_state_is_a_change_of_a_value() {
        let record = |data: Json| ExtRecord {
            ext: "user/probe".to_owned(),
            data,
        };
        let set = record(json!({"state": {"key": "a", "value": 1}}));
        assert_eq!(change_of(&set), Some(("a", Some(&json!(1)))));
        let removed = record(json!({"state": {"key": "a", "value": null}}));
        assert_eq!(change_of(&removed), Some(("a", None)));
        let without = record(json!({"state": {"key": "a"}}));
        assert_eq!(change_of(&without), Some(("a", None)));
        for other in [
            json!(null),
            json!({"other": 1}),
            json!({"state": {"key": 1}}),
        ] {
            assert_eq!(change_of(&record(other)), None);
        }
    }
}
