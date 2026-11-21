//! The schema: every key a configuration file may hold, what it takes, who
//! may set it and when a change takes effect.
//!
//! **A key is declared once.** The [`settings!`] macro turns one declaration
//! into the field of a settings struct, its default, its place in the schema
//! tree ([`Node`]), its reader and its writer. Validation, the trust rules,
//! the reload notices, the JSON Schema and the reference page are all walks
//! over that one tree, so none of them can describe a different
//! configuration from the one the code reads.
//!
//! ```ignore
//! settings! {
//!     /// How requests are retried.
//!     pub struct RetrySettings {
//!         /// The most attempts for one request, the first included.
//!         [Preference, Now, 1..=20]
//!         pub attempts: u32 = 5,
//!     }
//! }
//! ```
//!
//! The bracket before a field says what kind of entry it is:
//!
//! | Bracket | Meaning |
//! |---|---|
//! | `[<trust>, <effect>]` | A setting: its [`Trust`] rule and its [`Effect`]. |
//! | `[<trust>, <effect>, low..=high]` | The same, with the range a number must be in. A length of time's range is written in its units, as the reference writes it (`100ms..=1m`), and any other number's with none (`1..=20`); the other way round is an error of the build. |
//! | `[<trust>, <effect>, joined]` | The same, for a list that every layer adds to. |
//! | `[nested]` | A table of settings, or a named collection of such tables. |
//! | `[enforced]` | A table of settings the harness enforces: an unknown key in it is an error. |
//! | `[reserved <trust>]` | A section whose keys the step that builds it will declare. Anything in it is accepted, unread. |

mod json;
pub(crate) mod redact;
mod reference;
mod sections;
mod types;

use std::collections::BTreeMap;
use std::sync::OnceLock;

use toml::Value;

pub use json::json_schema;
pub(crate) use redact::redact;
pub use reference::reference_page;
pub use sections::{
    AdaptSettings, CompatSettings, CredentialSettings, HookSettings, KeySettings, LimitSet,
    LimitSettings, LogSettings, ManagedSettings, ModelSettings, PermissionSettings,
    ProviderSettings, RetrySettings, RoleSettings, RpcSettings, SandboxSettings, SessionSettings,
    Settings, StatsSettings, TerminalSettings, TuiSettings, UsageSettings, VariantSettings,
    VerifySettings,
};
pub use types::{
    BackspaceChoice, Bindings, ColorChoice, DensityChoice, EnvText, GlyphChoice, Headers,
    HookEventName, PresenterChoice, Profiles, ReasoningShown, ReasoningVisibility, Reserved,
    SessionDurability, StoreChoice, ToolOutputChoice, UsageRanges, VerifyMode, Weekday,
};
pub(crate) use types::{magnitude, milliseconds_as_text};

use crate::key::KeyPath;

/// The version of the configuration schema this build reads and writes:
/// the `config_version` key.
pub const CONFIG_VERSION: u32 = 1;

/// Who may set a key from a project's files, as declared for the key.
///
/// A repository must not be able to take over the harness by containing a
/// configuration file (`docs/architecture/08-configuration.md`, "Trust and
/// what a project may set"). Most keys have one class. A few depend on the
/// value: lowering a limit is harmless and raising it is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// A harmless preference. A project's value applies always.
    Preference,
    /// It can run code. A project's value applies only after the user has
    /// trusted the project.
    RunsCode,
    /// It can widen access. A project's value applies only after trust.
    WidensAccess,
    /// It can raise what the user spends, whatever its value: the model, a
    /// reasoning level, the most output, the number of attempts. Whether
    /// one value costs more than another depends on the model catalog,
    /// which this crate does not have. A project's value applies only
    /// after trust; until then the user's own applies (the owner's
    /// decision, 2026-11-13).
    RaisesCost,
    /// It can keep every command from working: the variables a command is
    /// not given (`shell.env_remove = ["*"]` takes `PATH` away). A project's
    /// value applies only after trust (the owner's decision of 2026-11-16,
    /// after the review of step 23).
    StopsCommands,
    /// The user's alone. A project's value is ignored, with a warning.
    UserOnly,
    /// Kept apart (since step 28, for hooks): the user's layers merge as
    /// any setting's do, and a project's value is never merged with them.
    /// A project's entries are read on their own ([`crate::ProjectHook`]),
    /// each waiting for trust and its own approval, so that no project file
    /// can name, change or turn off one of the user's.
    KeptApart,
    /// A permission rule: `deny` and `ask` are preferences, `allow` widens
    /// access.
    ByRuleAction,
    /// A permission mode: at or below the mode already in effect it is a
    /// preference; above that, up to `supervised`, it widens access; above
    /// `supervised` it is the user's alone.
    ByLevel,
    /// The sandbox's network setting: `off` is a preference, `full` widens
    /// access.
    ByNetwork,
    /// A speed tier: `standard` is a preference, and any other tier raises
    /// cost. A fast tier costs about twice as much per token, and an
    /// untrusted repository must not be able to raise what its user spends.
    BySpeed,
    /// Whether the project's checks run (`verify.mode`, since step 19): a
    /// value at or below the one in effect, in the order `off`, `ask`,
    /// `auto`, is a preference, since it runs fewer commands; a higher one
    /// is the user's alone.
    ByVerifyMode,
    /// A limit: a value at or below the one already in effect is a
    /// preference. The first limit where there was none applies only once
    /// the user trusts the project, since a limit of nothing (`time =
    /// "0s"`, `requests = 1`) would stop every turn (the owner's decision of
    /// 2026-11-16). A higher one is the user's alone.
    TightenOnly,
}

/// What a project's value for a key is, once the value is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrustClass {
    /// Applied always.
    Preference,
    /// Applied after trust: a first limit where there was none, which can
    /// stop every turn (since step 23).
    FirstLimit,
    /// Applied after trust: it can run code.
    RunsCode,
    /// Applied after trust: it can widen access.
    WidensAccess,
    /// Applied after trust: it can raise what the user spends.
    RaisesCost,
    /// Applied after trust: it can keep every command from working.
    StopsCommands,
    /// Never applied from a project.
    UserOnly,
    /// Never merged from a project: its entries are read apart, each
    /// approved on its own (since step 28).
    KeptApart,
}

impl TrustClass {
    /// The class as the reference and the messages name it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::FirstLimit => "first limit",
            Self::RunsCode => "runs code",
            Self::WidensAccess => "widens access",
            Self::RaisesCost => "raises cost",
            Self::StopsCommands => "stops commands",
            Self::UserOnly => "user only",
            Self::KeptApart => "kept apart",
        }
    }

    /// Whether a project's value of this class waits for the user to trust
    /// the project.
    #[must_use]
    pub fn needs_trust(self) -> bool {
        matches!(
            self,
            Self::FirstLimit
                | Self::RunsCode
                | Self::WidensAccess
                | Self::RaisesCost
                | Self::StopsCommands
        )
    }
}

impl Trust {
    /// The class of `value` for a key with this rule. `current` is the value
    /// in effect from the layers below (defaults included), for the rules
    /// that compare.
    #[must_use]
    pub fn class(self, value: &Value, current: Option<&Value>) -> TrustClass {
        match self {
            Self::Preference => TrustClass::Preference,
            Self::RunsCode => TrustClass::RunsCode,
            Self::WidensAccess => TrustClass::WidensAccess,
            Self::RaisesCost => TrustClass::RaisesCost,
            Self::StopsCommands => TrustClass::StopsCommands,
            Self::UserOnly => TrustClass::UserOnly,
            Self::KeptApart => TrustClass::KeptApart,
            Self::ByRuleAction => {
                // Anything but a plain `deny` or `ask` is treated as an
                // allow: a rule that cannot be read is not a restriction.
                match value.get("action").and_then(Value::as_str) {
                    Some("deny" | "ask") => TrustClass::Preference,
                    _ => TrustClass::WidensAccess,
                }
            }
            Self::ByLevel => {
                let rank = |value: &Value| {
                    value.as_str().and_then(|name| {
                        cineris_protocol::Level::KNOWN
                            .iter()
                            .position(|known| *known == name)
                    })
                };
                let supervised = cineris_protocol::Level::KNOWN
                    .iter()
                    .position(|known| *known == cineris_protocol::Level::Supervised.as_str());
                match (rank(value), current.and_then(rank)) {
                    (Some(wanted), _) if Some(wanted) > supervised => TrustClass::UserOnly,
                    (Some(wanted), Some(now)) if wanted <= now => TrustClass::Preference,
                    (Some(_), _) => TrustClass::WidensAccess,
                    (None, _) => TrustClass::UserOnly,
                }
            }
            Self::ByNetwork => match value.as_str() {
                Some("off") => TrustClass::Preference,
                _ => TrustClass::WidensAccess,
            },
            Self::BySpeed => match value.as_str() {
                Some("standard") => TrustClass::Preference,
                _ => TrustClass::RaisesCost,
            },
            Self::ByVerifyMode => {
                let rank = |value: &Value| value.as_str().and_then(types::VerifyMode::rank_of);
                match (rank(value), current.and_then(rank)) {
                    (Some(wanted), Some(now)) if wanted <= now => TrustClass::Preference,
                    _ => TrustClass::UserOnly,
                }
            }
            Self::TightenOnly => {
                match (types::magnitude(value), current.and_then(types::magnitude)) {
                    (Some(_), None) => TrustClass::FirstLimit,
                    (Some(wanted), Some(now)) if wanted <= now => TrustClass::Preference,
                    _ => TrustClass::UserOnly,
                }
            }
        }
    }

    /// The rule as the reference page states it.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::RunsCode => "runs code",
            Self::WidensAccess => "widens access",
            Self::RaisesCost => "raises cost",
            Self::StopsCommands => "stops commands",
            Self::UserOnly => "user only",
            Self::KeptApart => {
                "kept apart: a project's own are read on their own, never merged with yours, each \
                 waiting for trust and its approval"
            }
            Self::ByRuleAction => {
                "by action: `deny` and `ask` are preferences, `allow` widens access"
            }
            Self::ByLevel => {
                "by level: not above the mode in effect is a preference, up to `supervised` \
                 widens access, above it user only"
            }
            Self::ByNetwork => "by value: `off` is a preference, `full` widens access",
            Self::BySpeed => "by value: `standard` is a preference, any other tier raises cost",
            Self::ByVerifyMode => {
                "lower only: at or below the mode in effect (`off` < `ask` < `auto`) is a \
                 preference, a higher one is user only"
            }
            Self::TightenOnly => {
                "tighten only: a lower limit is a preference, a first limit where there was \
                 none waits for trust, a higher one is user only"
            }
        }
    }
}

/// When a change to a key takes effect in a running harness
/// (`docs/architecture/08-configuration.md`, "Reloading without a restart").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effect {
    /// At once: on the next frame, tool call or request.
    Now,
    /// At the session's next context epoch (a compaction, or the session
    /// opened again), because it shapes the system prompt, the tool list or
    /// the model. A notice says what is pending.
    NextEpoch,
    /// In sessions opened from then on: what a session is made with once,
    /// as it opens (its shell, its sandbox's switch and protected paths),
    /// which no epoch of a running session can take. Since step 24.
    NextSession,
    /// When the harness is next started.
    Restart,
}

impl Effect {
    /// The effect as the reference and the notices name it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Now => "at once",
            Self::NextEpoch => "next epoch",
            Self::NextSession => "next session",
            Self::Restart => "restart",
        }
    }
}

/// What a setting's value is, for the generated schema and the reference.
#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// `true` or `false`.
    Bool,
    /// A whole number.
    Integer,
    /// A number, whole or not.
    Number,
    /// Text.
    Text,
    /// One of a fixed set of words.
    Choice(&'static [&'static str]),
    /// A length of time, written as text: `"20s"`.
    Duration,
    /// Text, or `{ env = "NAME" }` to read an environment variable when the
    /// value is used.
    EnvText,
    /// A list.
    List(Box<Kind>),
    /// A table from names to values.
    Map(Box<Kind>),
    /// A permission rule: `effect`, `match` and `action`, and nothing else.
    Rule,
}

/// What kind of entry a child of a table is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Meta {
    /// A setting.
    Leaf {
        /// Who may set it from a project.
        trust: Trust,
        /// When a change takes effect.
        effect: Effect,
        /// The range a number must be in, ends included.
        range: Option<(f64, f64)>,
        /// Whether the value is a list that each layer adds to, instead of
        /// replacing it.
        joined: bool,
    },
    /// A table of settings, or a named collection.
    Nested {
        /// Whether the harness enforces what is in it: an unknown key is
        /// then an error, not a warning.
        enforced: bool,
    },
    /// A section whose keys are not declared yet.
    Reserved {
        /// What everything in it counts as, from a project.
        trust: Trust,
    },
}

/// One entry of a table in the schema.
#[derive(Debug, Clone)]
pub struct Child {
    /// The key's name.
    pub name: &'static str,
    /// Its documentation, a line per element, as written on the field.
    pub doc: &'static [&'static str],
    /// What kind of entry it is.
    pub meta: Meta,
    /// The default, when there is one.
    pub default: Option<Value>,
    /// What it holds.
    pub node: Node,
    /// Reads a value of the field's type, to say whether it is one.
    pub(crate) check: fn(&Value) -> Result<(), String>,
}

impl Child {
    /// The documentation as one paragraph.
    #[must_use]
    pub fn doc_text(&self) -> String {
        self.doc
            .iter()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_owned()
    }
}

/// A place in the schema.
#[derive(Debug, Clone)]
pub enum Node {
    /// A setting of this kind.
    Leaf(Kind),
    /// A table with these keys.
    Table(Vec<Child>),
    /// A named collection: any name, each holding the same thing.
    Map(Box<Node>),
    /// A reserved section: anything is accepted.
    Open,
    /// The profiles: any name, each holding the keys of the file itself.
    Profiles,
}

impl Node {
    /// The entry called `name`, when this is a table that has it.
    #[must_use]
    pub fn child(&self, name: &str) -> Option<&Child> {
        match self {
            Self::Table(children) => children.iter().find(|child| child.name == name),
            _ => None,
        }
    }
}

/// A type that can be a setting: it has a place in the schema, and is read
/// from and written to a TOML value.
pub trait Setting: Sized {
    /// What the schema says about a value of this type.
    fn node() -> Node;

    /// Reads a value.
    ///
    /// # Errors
    ///
    /// With a phrase that says what was expected, without the key or the
    /// file: the caller knows those.
    fn read(value: &Value) -> Result<Self, String>;

    /// The value as TOML, or `None` for one that is not set.
    fn write(&self) -> Option<Value>;
}

/// Reads the section `name` of a configuration as `config/get` answers it:
/// each key dotted, each value as JSON. A key it does not hold has its
/// default. For a front end that reads part of the configuration again
/// while it runs (step 24: the interface's look and key map, after a
/// reload applied a key of them).
///
/// # Errors
///
/// When a value does not read as its setting: the key and what was
/// expected.
pub fn read_section<T: Setting>(
    entries: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<T, String> {
    let mut table = toml::Table::new();
    for (key, value) in entries {
        // Read as the key it is: a part a name with a dot in it is quoted.
        let Ok(path) = KeyPath::parse(key) else {
            continue;
        };
        let Some((first, rest)) = path.parts().split_first() else {
            continue;
        };
        if first != name {
            continue;
        }
        // JSON's null has no TOML value: a key not set.
        if value.is_null() {
            continue;
        }
        let value: Value =
            serde_json::from_value(value.clone()).map_err(|error| format!("{key}: {error}"))?;
        let Some((last, path)) = rest.split_last() else {
            continue;
        };
        let mut at = &mut table;
        for part in path {
            let next = at
                .entry(part.clone())
                .or_insert_with(|| Value::Table(toml::Table::new()));
            let Value::Table(next) = next else {
                return Err(format!("{key}: a table and a value at once"));
            };
            at = next;
        }
        at.insert(last.clone(), value);
    }
    T::read(&Value::Table(table)).map_err(|error| format!("{name}.{error}"))
}

/// The schema of a whole configuration file.
#[must_use]
pub fn root() -> &'static Node {
    static ROOT: OnceLock<Node> = OnceLock::new();
    ROOT.get_or_init(Settings::node)
}

/// What the schema says about a key path.
#[derive(Debug, Clone)]
pub enum Lookup {
    /// It names a setting.
    Setting {
        /// The setting's entry.
        child: &'static Child,
        /// Whether it is inside a table the harness enforces.
        enforced: bool,
    },
    /// It names something inside a setting whose value is itself a table
    /// (a header in a provider's `headers`). Such a setting is set whole.
    InsideSetting {
        /// The setting it is inside.
        child: &'static Child,
    },
    /// It names a table of settings, a named collection or one of its
    /// entries, or a profile.
    Table {
        /// Whether it is inside a table the harness enforces.
        enforced: bool,
    },
    /// It is a reserved section, or inside one.
    Reserved {
        /// What everything in the section counts as, from a project.
        trust: Trust,
    },
    /// No such key is declared.
    Unknown {
        /// The names that are declared where the path went wrong, for a
        /// suggestion.
        siblings: Vec<&'static str>,
        /// Whether that place is inside a table the harness enforces.
        enforced: bool,
    },
}

/// Looks a key path up in the schema. A path into a profile is looked up as
/// the key it sets.
#[must_use]
pub fn lookup(path: &KeyPath) -> Lookup {
    let mut node = root();
    let mut enforced = false;
    for (index, part) in path.parts().iter().enumerate() {
        match node {
            Node::Table(_) => {
                let Some(child) = node.child(part) else {
                    return Lookup::Unknown {
                        siblings: names(node),
                        enforced,
                    };
                };
                match child.meta {
                    Meta::Leaf { .. } => {
                        return if index + 1 == path.len() {
                            Lookup::Setting { child, enforced }
                        } else {
                            Lookup::InsideSetting { child }
                        };
                    }
                    Meta::Reserved { trust } => return Lookup::Reserved { trust },
                    Meta::Nested { enforced: here } => {
                        enforced = enforced || here;
                        node = &child.node;
                    }
                }
            }
            // Any name: the schema goes on below it.
            Node::Map(inner) => node = inner,
            // Any name: the keys of a profile are the keys of the file.
            Node::Profiles => {
                node = root();
                enforced = false;
            }
            Node::Leaf(_) | Node::Open => break,
        }
    }
    Lookup::Table { enforced }
}

/// One declared setting, with its full name. A name inside a named
/// collection has `*` where the name goes: `providers.*.base_url`.
#[derive(Debug, Clone)]
pub struct KeyInfo {
    /// The key, with `*` for a name the user chooses.
    pub path: String,
    /// What it holds. `None` for a reserved section.
    pub kind: Option<Kind>,
    /// Who may set it from a project.
    pub trust: Trust,
    /// When a change takes effect. `None` for a reserved section.
    pub effect: Option<Effect>,
    /// Whether the harness enforces it.
    pub enforced: bool,
    /// Whether each layer adds to the list instead of replacing it.
    pub joined: bool,
    /// The range a number must be in.
    pub range: Option<(f64, f64)>,
    /// The default.
    pub default: Option<Value>,
    /// The documentation.
    pub doc: String,
}

/// Every declared setting and reserved section, in the order of the schema.
/// The keys of a profile are the same keys again, and are not repeated.
#[must_use]
pub fn keys() -> Vec<KeyInfo> {
    let mut keys = Vec::new();
    collect(root(), "", false, &mut keys);
    keys
}

fn collect(node: &Node, prefix: &str, enforced: bool, keys: &mut Vec<KeyInfo>) {
    let Node::Table(children) = node else {
        return;
    };
    for child in children {
        let path = if prefix.is_empty() {
            child.name.to_owned()
        } else {
            format!("{prefix}.{}", child.name)
        };
        match child.meta {
            Meta::Leaf {
                trust,
                effect,
                range,
                joined,
            } => keys.push(KeyInfo {
                kind: match &child.node {
                    Node::Leaf(kind) => Some(kind.clone()),
                    _ => None,
                },
                path,
                trust,
                effect: Some(effect),
                enforced,
                joined,
                range,
                default: child.default.clone(),
                doc: child.doc_text(),
            }),
            Meta::Reserved { trust } => keys.push(KeyInfo {
                path: format!("{path}.*"),
                kind: None,
                trust,
                effect: None,
                enforced,
                joined: false,
                range: None,
                default: None,
                doc: child.doc_text(),
            }),
            Meta::Nested { enforced: here } => {
                let mut node = &child.node;
                let mut path = path;
                while let Node::Map(inner) = node {
                    node = inner;
                    path.push_str(".*");
                }
                collect(node, &path, enforced || here, keys);
            }
        }
    }
}

/// The names of a table's entries, for a "did you mean" suggestion.
pub(crate) fn names(node: &Node) -> Vec<&'static str> {
    match node {
        Node::Table(children) => children.iter().map(|child| child.name).collect(),
        _ => Vec::new(),
    }
}

/// A TOML table of `entries`.
///
/// Collected through a `Vec`, and never straight into the table: the map's
/// own `FromIterator` sorts what it is given with a closure that names the
/// iterator's type, so each kind of iterator collected into a table builds
/// the standard library's sort again, about 4 KB of the shipped binary
/// each. Twelve such collects in this crate had a sort each
/// (`docs/measurements/binary-size.md`, "The size step").
pub(crate) fn table_of(entries: impl IntoIterator<Item = (String, Value)>) -> toml::Table {
    let entries: Vec<(String, Value)> = entries.into_iter().collect();
    entries.into_iter().collect()
}

/// The default of every setting that has one, as a table.
#[must_use]
pub fn defaults() -> toml::Table {
    match Settings::default().write() {
        Some(Value::Table(table)) => table,
        _ => toml::Table::new(),
    }
}

impl<T: Setting> Setting for Option<T> {
    fn node() -> Node {
        T::node()
    }

    fn read(value: &Value) -> Result<Self, String> {
        T::read(value).map(Some)
    }

    fn write(&self) -> Option<Value> {
        self.as_ref().and_then(Setting::write)
    }
}

impl<T: Setting> Setting for BTreeMap<String, T> {
    fn node() -> Node {
        Node::Map(Box::new(T::node()))
    }

    fn read(value: &Value) -> Result<Self, String> {
        let Value::Table(table) = value else {
            return Err("expected a table of named entries".to_owned());
        };
        table
            .iter()
            .map(|(name, entry)| {
                T::read(entry)
                    .map(|entry| (name.clone(), entry))
                    .map_err(|error| format!("{name}: {error}"))
            })
            .collect()
    }

    fn write(&self) -> Option<Value> {
        if self.is_empty() {
            return None;
        }
        let table = table_of(
            self.iter()
                .filter_map(|(name, entry)| Some((name.clone(), entry.write()?))),
        );
        Some(Value::Table(table))
    }
}

/// Whether a setting's range is a length of time, written with units
/// (`100ms..=1m`), or a number, written with none. Only the types a range
/// is declared on have it, so a range on any other is an error of the build.
pub(crate) trait Ranged {
    /// A length of time.
    const TIMED: bool;
}

impl Ranged for std::time::Duration {
    const TIMED: bool = true;
}

macro_rules! numbers_are_ranged {
    ($($number:ty),*) => {
        $(impl Ranged for $number {
            const TIMED: bool = false;
        })*
    };
}
numbers_are_ranged!(u8, u16, u32, u64, usize, i32, i64, f64);

impl<T: Ranged> Ranged for Option<T> {
    const TIMED: bool = T::TIMED;
}

/// Declares a table of settings: the struct, its defaults, its schema, its
/// reader and its writer. The syntax is described on [the module](self).
macro_rules! settings {
    (
        $(#[doc = $struct_doc:literal])*
        pub struct $name:ident {
            $(
                $(#[doc = $doc:literal])*
                [$($meta:tt)*]
                pub $field:ident : $ty:ty $(= $default:expr)?
            ),* $(,)?
        }
    ) => {
        $(#[doc = $struct_doc])*
        #[derive(Clone, PartialEq)]
        pub struct $name {
            $(
                $(#[doc = $doc])*
                pub $field: $ty,
            )*
        }

        /// Every field, with the user part of any web address in it
        /// redacted (`schema::redact`).
        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_struct(stringify!($name))
                    $( .field(stringify!($field), &$crate::schema::redact::Shown(&self.$field)) )*
                    .finish()
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    $( $field: settings!(@default $($default)?), )*
                }
            }
        }

        impl $crate::schema::Setting for $name {
            fn node() -> $crate::schema::Node {
                let defaults = Self::default();
                $crate::schema::Node::Table(vec![
                    $(
                        $crate::schema::Child {
                            name: stringify!($field),
                            doc: &[$($doc),*],
                            meta: settings!(@meta $ty; $($meta)*),
                            default: $crate::schema::Setting::write(&defaults.$field),
                            node: <$ty as $crate::schema::Setting>::node(),
                            check: |value| {
                                <$ty as $crate::schema::Setting>::read(value).map(drop)
                            },
                        },
                    )*
                ])
            }

            fn read(value: &::toml::Value) -> Result<Self, String> {
                let ::toml::Value::Table(table) = value else {
                    return Err("expected a table".to_owned());
                };
                let defaults = Self::default();
                Ok(Self {
                    $(
                        $field: match table.get(stringify!($field)) {
                            Some(value) => <$ty as $crate::schema::Setting>::read(value)
                                .map_err(|error| {
                                    format!("{}: {error}", stringify!($field))
                                })?,
                            None => defaults.$field,
                        },
                    )*
                })
            }

            fn write(&self) -> Option<::toml::Value> {
                let mut table = ::toml::Table::new();
                $(
                    if let Some(value) = $crate::schema::Setting::write(&self.$field) {
                        table.insert(stringify!($field).to_owned(), value);
                    }
                )*
                Some(::toml::Value::Table(table))
            }
        }
    };
    (@default) => { Default::default() };
    (@default $default:expr) => { $default };
    (@meta $_ty:ty; nested) => { $crate::schema::Meta::Nested { enforced: false } };
    (@meta $_ty:ty; enforced) => { $crate::schema::Meta::Nested { enforced: true } };
    (@meta $_ty:ty; reserved $trust:ident) => {
        $crate::schema::Meta::Reserved { trust: $crate::schema::Trust::$trust }
    };
    (@meta $_ty:ty; $trust:ident, $effect:ident) => {
        $crate::schema::Meta::Leaf {
            trust: $crate::schema::Trust::$trust,
            effect: $crate::schema::Effect::$effect,
            range: None,
            joined: false,
        }
    };
    (@meta $_ty:ty; $trust:ident, $effect:ident, joined) => {
        $crate::schema::Meta::Leaf {
            trust: $crate::schema::Trust::$trust,
            effect: $crate::schema::Effect::$effect,
            range: None,
            joined: true,
        }
    };
    (@meta $ty:ty; $trust:ident, $effect:ident, $low:tt ..= $high:tt) => {
        $crate::schema::Meta::Leaf {
            trust: $crate::schema::Trust::$trust,
            effect: $crate::schema::Effect::$effect,
            range: Some({
                // Read when the crate is built, in the setting's unit: a
                // length of time as the reference writes one (`100ms..=1m`),
                // in milliseconds; any other number as it is written.
                const RANGE: (u64, u64) = {
                    let timed = <$ty as $crate::schema::Ranged>::TIMED;
                    let low = $crate::duration::range_bound(stringify!($low), timed);
                    let high = $crate::duration::range_bound(stringify!($high), timed);
                    assert!(low <= high, "a range's low bound is above its high one");
                    (low, high)
                };
                #[expect(clippy::cast_precision_loss, reason = "bounds are far below 2^53")]
                (RANGE.0 as f64, RANGE.1 as f64)
            }),
            joined: false,
        }
    };
}
pub(crate) use settings;

/// Declares a setting that is one of a fixed set of words.
macro_rules! choice {
    (
        $(#[doc = $doc:literal])*
        pub enum $name:ident {
            $(
                $(#[doc = $variant_doc:literal])*
                $variant:ident => $word:literal
            ),+ $(,)?
        }
    ) => {
        $(#[doc = $doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name {
            $(
                $(#[doc = $variant_doc])*
                $variant,
            )+
        }

        impl $name {
            /// Every value, as written in a configuration file.
            pub const WORDS: &'static [&'static str] = &[$($word),+];

            /// The value as written in a configuration file.
            #[must_use]
            pub fn as_str(self) -> &'static str {
                match self {
                    $( Self::$variant => $word, )+
                }
            }
        }

        impl $crate::schema::Setting for $name {
            fn node() -> $crate::schema::Node {
                $crate::schema::Node::Leaf($crate::schema::Kind::Choice(Self::WORDS))
            }

            fn read(value: &::toml::Value) -> Result<Self, String> {
                match value.as_str() {
                    $( Some($word) => Ok(Self::$variant), )+
                    _ => Err($crate::schema::types::one_of(Self::WORDS)),
                }
            }

            fn write(&self) -> Option<::toml::Value> {
                Some(::toml::Value::String(self.as_str().to_owned()))
            }
        }
    };
}
pub(crate) use choice;
