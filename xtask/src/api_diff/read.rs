//! Reading rustdoc's JSON output as a list of public items
//! (`docs/testing-and-ci.md`, "The public API between two trees").
//!
//! The list has one entry for each way a user of the crate can name an item:
//! its path, and a signature written as Rust is written. Two lists of the
//! same crate, from two trees, are compared by [`super::diff`].
//!
//! **Only the JSON format this code was written against is read**
//! ([`SUPPORTED_FORMATS`]): rustdoc's format has no stability promise, and
//! its field names move from one toolchain to the next. A file of another
//! version is refused with the versions named, which is a bump of the
//! toolchain's cue to read the new format and extend the list.
//!
//! **What is in an entry.** An item is reached by walking the modules from
//! the crate root through public items only, following `pub use` (a glob
//! too). An item reached by one path is an entry there, with its members:
//! fields, variants, a trait's items, the methods of its inherent `impl`s,
//! and the `impl`s of traits for it. An item reached by several paths is a
//! full entry at the first of them (a path where it is defined, else the
//! shortest) and a `use` entry at each of the others. What an entry says
//! about itself is its **signature**: generics, bounds, arguments, return
//! type, a struct's shape, an enum variant's fields, a trait's supertraits.
//! Not its documentation and not its body (a constant's value, a method's
//! code). What rustdoc does not write in its JSON (private and hidden items,
//! what a `cfg` leaves out) is not here.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::Result;

/// The formats of rustdoc's JSON this code reads: the one the pinned
/// toolchain writes. A toolchain bump that moves it makes `api-diff` refuse
/// until the new format has been looked at, and this list says it has.
pub(crate) const SUPPORTED_FORMATS: &[u64] = &[61];

/// The namespace a name lives in: a struct and a function may share one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Space {
    Type,
    Value,
    Macro,
    Impl,
    Field,
}

/// Where an entry is: a path and the namespace of the name.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    pub(crate) path: String,
    pub(crate) space: Space,
}

/// One public item, member or impl.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    /// `fn`, `struct`, `variant`, `impl`, `use`...
    pub(crate) kind: String,
    /// The signature as a person reads it.
    pub(crate) shown: String,
    /// What a change is judged on: `shown`, but for an argument's name,
    /// which a caller does not see.
    pub(crate) identity: String,
    /// For an `impl` and what comes with one: the path of the local type
    /// or trait it is for, so that an `impl` of a type that was added in
    /// the same change is not reported beside it.
    pub(crate) owner: Option<String>,
}

/// The public items of one crate.
pub(crate) type Api = BTreeMap<Key, Entry>;

static NULL: Value = Value::Null;

/// Reads the public items out of a rustdoc JSON document.
pub(crate) fn read(json: &Value) -> Result<Api> {
    let version = json
        .get("format_version")
        .and_then(Value::as_u64)
        .ok_or("not a rustdoc JSON document: it has no `format_version`")?;
    if !SUPPORTED_FORMATS.contains(&version) {
        return Err(format!(
            "rustdoc wrote JSON format {version}; this xtask reads format {} only, the one \
             the pinned toolchain wrote when it was written. Look at the new format (the \
             fields in `xtask/src/api_diff/read.rs`), extend the reader and SUPPORTED_FORMATS, \
             and run the ignored test `sys_the_real_rustdoc_...` of `api_diff`.",
            SUPPORTED_FORMATS
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if json.get("includes_private").and_then(Value::as_bool) == Some(true) {
        return Err("the JSON includes private items: it was not made as api-diff makes it".into());
    }
    let index = json
        .get("index")
        .and_then(Value::as_object)
        .ok_or("the JSON has no `index`")?;
    let root = json
        .get("root")
        .and_then(Value::as_u64)
        .ok_or("the JSON has no `root`")?;
    Reader { index }.api(root)
}

/// One path that reaches an item.
struct Exposure {
    path: String,
    id: u64,
    /// The item is defined in the module the path goes through, and not
    /// brought there by a `use`.
    direct: bool,
}

/// What a walk of the modules found.
#[derive(Default)]
struct Found {
    exposures: Vec<Exposure>,
    /// `pub use` of what is not in this crate: a path and what was written.
    externals: Vec<(String, String)>,
}

/// The entries so far, before two with one key are told apart.
#[derive(Default)]
struct Sink {
    rows: Vec<Row>,
}

struct Row {
    key: Key,
    entry: Entry,
    /// What tells two entries of one path apart: the `impl` header.
    context: Option<String>,
}

impl Sink {
    fn push(&mut self, path: String, space: Space, entry: Entry, context: Option<String>) {
        self.rows.push(Row {
            key: Key { path, space },
            entry,
            context,
        });
    }

    /// Entries with one key (`A::x` in `impl A<u8>` and in `impl A<u16>`)
    /// are told apart by their `impl` header.
    fn finish(self) -> Api {
        let mut counts: BTreeMap<Key, usize> = BTreeMap::new();
        for row in &self.rows {
            *counts.entry(row.key.clone()).or_default() += 1;
        }
        let mut api = Api::new();
        for row in self.rows {
            let mut key = row.key;
            if counts.get(&key).copied().unwrap_or(0) > 1 {
                let context = row.context.unwrap_or_else(|| "again".to_owned());
                key.path = format!("{} [{context}]", key.path);
            }
            while api.contains_key(&key) {
                key.path.push('+');
            }
            api.insert(key, row.entry);
        }
        api
    }
}

struct Reader<'a> {
    index: &'a Map<String, Value>,
}

/// The item's kind (the one key of `inner`) and what is under it.
fn kind_of(item: &Value) -> (&str, &Value) {
    match item["inner"]
        .as_object()
        .and_then(|inner| inner.iter().next())
    {
        Some((kind, value)) => (kind.as_str(), value),
        None => ("", &NULL),
    }
}

/// The only key of an object and its value: how rustdoc writes an enum.
fn single(value: &Value) -> Option<(&str, &Value)> {
    let object = value.as_object()?;
    if object.len() == 1 {
        object
            .iter()
            .next()
            .map(|(key, value)| (key.as_str(), value))
    } else {
        None
    }
}

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

fn text(value: &Value) -> String {
    value.as_str().map_or_else(|| compact(value), str::to_owned)
}

/// What cannot be read as Rust is kept as JSON, so that a change in it is
/// still a change.
fn compact(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn flag(value: &Value) -> bool {
    value.as_bool() == Some(true)
}

fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

fn space_of(kind: &str) -> Space {
    match kind {
        "function" | "constant" | "static" | "assoc_const" => Space::Value,
        "struct_field" => Space::Field,
        "macro" | "proc_macro" => Space::Macro,
        _ => Space::Type,
    }
}

impl Reader<'_> {
    /// An item of this crate by its id.
    fn item(&self, id: u64) -> Option<&Value> {
        self.index
            .get(&id.to_string())
            .filter(|item| item["crate_id"].as_u64() == Some(0))
    }

    fn api(&self, root: u64) -> Result<Api> {
        let root_item = self.item(root).ok_or("the JSON has no root module")?;
        let crate_name = text(&root_item["name"]);
        let mut found = Found::default();
        let mut stack = vec![root];
        self.walk(
            kind_of(root_item).1,
            &crate_name,
            &mut stack,
            true,
            &mut found,
        );

        let mut by_id: BTreeMap<u64, Vec<Exposure>> = BTreeMap::new();
        for exposure in found.exposures {
            by_id.entry(exposure.id).or_default().push(exposure);
        }
        for list in by_id.values_mut() {
            list.sort_by_key(|exposure| {
                (
                    !exposure.direct,
                    exposure.path.matches("::").count(),
                    exposure.path.clone(),
                )
            });
        }
        let homes: BTreeMap<u64, String> = by_id
            .iter()
            .filter_map(|(id, list)| list.first().map(|first| (*id, first.path.clone())))
            .collect();

        let mut sink = Sink::default();
        for (id, list) in &by_id {
            let Some(home) = homes.get(id) else { continue };
            self.item_rows(*id, home, &mut sink);
            for other in list.iter().skip(1) {
                let kind = self.item(*id).map_or("", |item| kind_of(item).0);
                let shown = format!("pub use {home}");
                sink.push(
                    other.path.clone(),
                    space_of(kind),
                    Entry {
                        kind: "use".to_owned(),
                        identity: shown.clone(),
                        shown,
                        owner: None,
                    },
                    None,
                );
            }
        }
        for (path, source) in found.externals {
            let shown = format!("pub use {source}");
            sink.push(
                path,
                Space::Type,
                Entry {
                    kind: "use".to_owned(),
                    identity: shown.clone(),
                    shown,
                    owner: None,
                },
                None,
            );
        }
        self.impl_rows(&homes, &mut sink);
        Ok(sink.finish())
    }

    /// The public paths through a module's items.
    fn walk(
        &self,
        module: &Value,
        prefix: &str,
        stack: &mut Vec<u64>,
        direct: bool,
        found: &mut Found,
    ) {
        for id in items(&module["items"]).filter_map(Value::as_u64) {
            let Some(child) = self.item(id) else { continue };
            if child["visibility"] != "public" {
                continue;
            }
            let (kind, inner) = kind_of(child);
            match kind {
                "use" => self.walk_use(inner, prefix, stack, found),
                "impl" => {}
                _ => {
                    let path = format!("{prefix}::{}", text(&child["name"]));
                    found.exposures.push(Exposure {
                        path: path.clone(),
                        id,
                        direct,
                    });
                    if kind == "module" && !stack.contains(&id) {
                        stack.push(id);
                        self.walk(inner, &path, stack, direct, found);
                        stack.pop();
                    }
                }
            }
        }
    }

    fn walk_use(&self, used: &Value, prefix: &str, stack: &mut Vec<u64>, found: &mut Found) {
        let name = text(&used["name"]);
        let glob = flag(&used["is_glob"]);
        let source = text(&used["source"]);
        let target = used["id"]
            .as_u64()
            .and_then(|id| self.item(id).map(|item| (id, item)));
        let Some((id, item)) = target else {
            let name = if glob { "*" } else { &name };
            found.externals.push((format!("{prefix}::{name}"), source));
            return;
        };
        let (kind, inner) = kind_of(item);
        if glob {
            if kind != "module" {
                found.externals.push((format!("{prefix}::*"), source));
            } else if !stack.contains(&id) {
                stack.push(id);
                self.walk(inner, prefix, stack, false, found);
                stack.pop();
            }
            return;
        }
        let path = format!("{prefix}::{name}");
        found.exposures.push(Exposure {
            path: path.clone(),
            id,
            direct: false,
        });
        if kind == "module" && !stack.contains(&id) {
            stack.push(id);
            self.walk(inner, &path, stack, false, found);
            stack.pop();
        }
    }

    /// The entries for an item at its home path, and for its members.
    fn item_rows(&self, id: u64, home: &str, sink: &mut Sink) {
        let Some(item) = self.item(id) else { return };
        let (kind, inner) = kind_of(item);
        let name = last_segment(home);
        let attrs = attr_prefix(item);
        let space = space_of(kind);
        let both = |make: &dyn Fn(Style) -> String| both_styles(make);
        match kind {
            "module" => put(
                sink,
                home,
                space,
                "mod",
                (format!("mod {name}"), format!("mod {name}")),
            ),
            "function" => put(sink, home, space, "fn", both(&|s| s.function(name, inner))),
            "constant" => put(
                sink,
                home,
                space,
                "const",
                both(&|s| format!("const {name}: {}", s.ty(&inner["type"]))),
            ),
            "static" => put(
                sink,
                home,
                space,
                "static",
                both(&|s| {
                    format!(
                        "static {}{name}: {}",
                        if flag(&inner["is_mutable"]) {
                            "mut "
                        } else {
                            ""
                        },
                        s.ty(&inner["type"])
                    )
                }),
            ),
            "type_alias" => put(
                sink,
                home,
                space,
                "type",
                both(&|s| {
                    format!(
                        "type {name}{} = {}{}",
                        s.params(&inner["generics"]),
                        s.ty(&inner["type"]),
                        s.where_clause(&inner["generics"])
                    )
                }),
            ),
            "trait_alias" => put(
                sink,
                home,
                space,
                "trait alias",
                both(&|s| {
                    format!(
                        "trait {name}{} = {}{}",
                        s.params(&inner["generics"]),
                        s.bounds(&inner["params"]),
                        s.where_clause(&inner["generics"])
                    )
                }),
            ),
            "macro" => put(
                sink,
                home,
                space,
                "macro",
                (format!("macro {name}!"), format!("macro {name}!")),
            ),
            "proc_macro" => {
                let shown = match inner["kind"].as_str() {
                    Some("attr") => format!("#[{name}]"),
                    Some("derive") => format!("#[derive({name})]"),
                    _ => format!("proc macro {name}!"),
                };
                put(sink, home, space, "macro", (shown.clone(), shown));
            }
            "struct" => {
                let shape = &inner["kind"];
                let mut field_ids: Vec<Option<u64>> = Vec::new();
                let marker = if shape == "unit" {
                    ";".to_owned()
                } else if let Some(fields) = shape["tuple"].as_array() {
                    field_ids = fields.iter().map(Value::as_u64).collect();
                    let hidden = fields.iter().any(Value::is_null);
                    format!("(..){}", if hidden { " /* private fields */" } else { "" })
                } else {
                    let plain = &shape["plain"];
                    field_ids = items(&plain["fields"]).map(Value::as_u64).collect();
                    if flag(&plain["has_stripped_fields"]) {
                        " { /* private fields */ }".to_owned()
                    } else {
                        String::new()
                    }
                };
                put(
                    sink,
                    home,
                    space,
                    "struct",
                    both(&|s| {
                        format!(
                            "{attrs}struct {name}{}{marker}{}",
                            s.params(&inner["generics"]),
                            s.where_clause(&inner["generics"])
                        )
                    }),
                );
                self.field_rows(&field_ids, home, sink);
            }
            "union" => {
                let marker = if flag(&inner["has_stripped_fields"]) {
                    " { /* private fields */ }"
                } else {
                    ""
                };
                put(
                    sink,
                    home,
                    space,
                    "union",
                    both(&|s| {
                        format!(
                            "{attrs}union {name}{}{marker}{}",
                            s.params(&inner["generics"]),
                            s.where_clause(&inner["generics"])
                        )
                    }),
                );
                let ids: Vec<Option<u64>> = items(&inner["fields"]).map(Value::as_u64).collect();
                self.field_rows(&ids, home, sink);
            }
            "enum" => {
                let marker = if flag(&inner["has_stripped_variants"]) {
                    " /* hidden variants */"
                } else {
                    ""
                };
                put(
                    sink,
                    home,
                    space,
                    "enum",
                    both(&|s| {
                        format!(
                            "{attrs}enum {name}{}{marker}{}",
                            s.params(&inner["generics"]),
                            s.where_clause(&inner["generics"])
                        )
                    }),
                );
                for variant in items(&inner["variants"])
                    .filter_map(Value::as_u64)
                    .filter_map(|id| self.item(id))
                {
                    let variant_name = text(&variant["name"]);
                    let made =
                        both(&|s| self.variant(s, &variant_name, &kind_of(variant).1.clone()));
                    sink.push(
                        format!("{home}::{variant_name}"),
                        Space::Type,
                        Entry {
                            kind: "variant".to_owned(),
                            shown: made.0,
                            identity: made.1,
                            owner: None,
                        },
                        None,
                    );
                }
            }
            "trait" => {
                put(
                    sink,
                    home,
                    space,
                    "trait",
                    both(&|s| {
                        let supers = s.bounds(&inner["bounds"]);
                        format!(
                            "{}{}trait {name}{}{}{}",
                            if flag(&inner["is_unsafe"]) {
                                "unsafe "
                            } else {
                                ""
                            },
                            if flag(&inner["is_auto"]) { "auto " } else { "" },
                            s.params(&inner["generics"]),
                            if supers.is_empty() {
                                String::new()
                            } else {
                                format!(": {supers}")
                            },
                            s.where_clause(&inner["generics"])
                        )
                    }),
                );
                for member in items(&inner["items"])
                    .filter_map(Value::as_u64)
                    .filter_map(|id| self.item(id))
                {
                    self.member_row(member, home, None, true, sink);
                }
            }
            "primitive" | "extern_crate" => {}
            other => put(
                sink,
                home,
                space,
                other,
                (format!("{other} {name}"), format!("{other} {name}")),
            ),
        }
    }

    /// The fields of a struct or union, by id; a stripped one is `None`.
    fn field_rows(&self, ids: &[Option<u64>], home: &str, sink: &mut Sink) {
        for field in ids.iter().flatten().filter_map(|id| self.item(*id)) {
            let name = text(&field["name"]);
            let made = both_styles(|s| format!("{name}: {}", s.ty(kind_of(field).1)));
            sink.push(
                format!("{home}::{name}"),
                Space::Field,
                Entry {
                    kind: "field".to_owned(),
                    shown: made.0,
                    identity: made.1,
                    owner: None,
                },
                None,
            );
        }
    }

    /// A variant as written: `A`, `B(u8, String)`, `C { x: Vec<u8> }`, with
    /// its discriminant if it has one.
    fn variant(&self, style: Style, name: &str, variant: &Value) -> String {
        let fields = |ids: &[Option<u64>]| -> Vec<(String, String)> {
            ids.iter()
                .map(|id| match id.and_then(|id| self.item(id)) {
                    Some(field) => (text(&field["name"]), style.ty(kind_of(field).1)),
                    None => ("..".to_owned(), "..".to_owned()),
                })
                .collect()
        };
        let mut out = name.to_owned();
        let shape = &variant["kind"];
        if let Some(tuple) = shape["tuple"].as_array() {
            let ids: Vec<Option<u64>> = tuple.iter().map(Value::as_u64).collect();
            let types: Vec<String> = fields(&ids).into_iter().map(|(_, ty)| ty).collect();
            out.push_str(&format!("({})", types.join(", ")));
        } else if shape["struct"].is_object() {
            let ids: Vec<Option<u64>> = items(&shape["struct"]["fields"])
                .map(Value::as_u64)
                .collect();
            let named: Vec<String> = fields(&ids)
                .into_iter()
                .map(|(field, ty)| format!("{field}: {ty}"))
                .collect();
            let hidden = flag(&shape["struct"]["has_stripped_fields"]);
            out.push_str(&format!(
                " {{ {}{} }}",
                named.join(", "),
                if hidden { ", .." } else { "" }
            ));
        }
        if let Some(expr) = variant["discriminant"]["expr"].as_str() {
            out.push_str(&format!(" = {expr}"));
        }
        out
    }

    /// An associated function, constant or type: of a trait (`provided`
    /// says a default is written), of an inherent `impl` (the entry says
    /// which one when it is not just the type), or of a trait's `impl`.
    fn member_row(
        &self,
        member: &Value,
        home: &str,
        context: Option<&str>,
        in_trait: bool,
        sink: &mut Sink,
    ) {
        let name = text(&member["name"]);
        let (kind, inner) = kind_of(member);
        let (label, space, made) = match kind {
            "function" => (
                "fn",
                Space::Value,
                both_styles(|s| {
                    let mut text = s.function(&name, inner);
                    if in_trait && flag(&inner["has_body"]) {
                        text.push_str(" { .. }");
                    }
                    text
                }),
            ),
            "assoc_const" => (
                "const",
                Space::Value,
                both_styles(|s| {
                    format!(
                        "const {name}: {}{}",
                        s.ty(&inner["type"]),
                        if in_trait && !inner["value"].is_null() {
                            " = .."
                        } else {
                            ""
                        }
                    )
                }),
            ),
            "assoc_type" => (
                "type",
                Space::Type,
                both_styles(|s| {
                    let bounds = s.bounds(&inner["bounds"]);
                    format!(
                        "type {name}{}{}{}{}",
                        s.params(&inner["generics"]),
                        if bounds.is_empty() {
                            String::new()
                        } else {
                            format!(": {bounds}")
                        },
                        if inner["type"].is_null() {
                            String::new()
                        } else {
                            format!(" = {}", s.ty(&inner["type"]))
                        },
                        s.where_clause(&inner["generics"])
                    )
                }),
            ),
            _ => return,
        };
        let (mut shown, mut identity) = made;
        if let Some(context) = context {
            shown.push_str(&format!(" (in {context})"));
            identity.push_str(&format!(" (in {context})"));
        }
        sink.push(
            format!("{home}::{name}"),
            space,
            Entry {
                kind: label.to_owned(),
                shown,
                identity,
                owner: None,
            },
            context.map(str::to_owned),
        );
    }

    /// The `impl`s: the methods of the inherent ones at their type's path,
    /// and the trait ones as entries of their own.
    fn impl_rows(&self, homes: &BTreeMap<u64, String>, sink: &mut Sink) {
        for item in self.index.values() {
            if item["crate_id"].as_u64() != Some(0) {
                continue;
            }
            let (kind, imp) = kind_of(item);
            if kind != "impl" {
                continue;
            }
            // What rustdoc adds to every type: `Send`, `Sync`, `From<T> for
            // T`. A type that is not `Send` is an `impl !Send`, and stays.
            if !imp["blanket_impl"].is_null()
                || (flag(&imp["is_synthetic"]) && !flag(&imp["is_negative"]))
            {
                continue;
            }
            let for_type = &imp["for"];
            let self_id = for_type["resolved_path"]["id"].as_u64();
            let self_local = self_id.is_some_and(|id| self.item(id).is_some());
            let self_home = self_id.and_then(|id| homes.get(&id));
            if self_local && self_home.is_none() {
                continue;
            }
            let shown = Style { names: true };
            if imp["trait"].is_null() {
                let Some(owner) = self_home else { continue };
                let header = format!(
                    "impl{} {}{}",
                    shown.params(&imp["generics"]),
                    shown.ty(for_type),
                    shown.where_clause(&imp["generics"])
                );
                let plain = imp["generics"]["params"]
                    .as_array()
                    .is_none_or(|params| params.iter().all(is_synthetic))
                    && items(&imp["generics"]["where_predicates"]).next().is_none()
                    && for_type["resolved_path"]["args"].is_null();
                for member in items(&imp["items"])
                    .filter_map(Value::as_u64)
                    .filter_map(|id| self.item(id))
                    .filter(|member| member["visibility"] == "public")
                {
                    let context = if plain { None } else { Some(header.as_str()) };
                    self.member_row(member, owner, context, false, sink);
                }
                continue;
            }
            let trait_ = &imp["trait"];
            let trait_id = trait_["id"].as_u64();
            let trait_home = trait_id.and_then(|id| homes.get(&id));
            if trait_id.is_some_and(|id| self.item(id).is_some()) && trait_home.is_none() {
                continue;
            }
            let owner = self_home.or(trait_home).cloned();
            let negative = if flag(&imp["is_negative"]) { "!" } else { "" };
            let unsafe_ = if flag(&imp["is_unsafe"]) {
                "unsafe "
            } else {
                ""
            };
            let trait_text = shown.path(trait_);
            let self_text = shown.ty(for_type);
            // Named in full where it is ours: two types of one name in two
            // modules are two.
            let long_trait = match trait_home {
                Some(home) => format!("{home}{}", shown.args(&trait_["args"])),
                None => trait_text.clone(),
            };
            let long_self = match self_home {
                Some(home) => format!("{home}{}", shown.args(&for_type["resolved_path"]["args"])),
                None => self_text.clone(),
            };
            let path = format!("impl {negative}{long_trait} for {long_self}");
            let header = format!(
                "{unsafe_}impl{} {negative}{trait_text} for {self_text}{}",
                shown.params(&imp["generics"]),
                shown.where_clause(&imp["generics"])
            );
            sink.push(
                path,
                Space::Impl,
                Entry {
                    kind: "impl".to_owned(),
                    identity: header.clone(),
                    shown: header,
                    owner: owner.clone(),
                },
                None,
            );
            // What the `impl` fixes of the trait: an associated type or
            // constant's type. Its methods are the trait's.
            let base = format!("<{long_self} as {long_trait}>");
            for member in items(&imp["items"])
                .filter_map(Value::as_u64)
                .filter_map(|id| self.item(id))
                .filter(|member| matches!(kind_of(member).0, "assoc_type" | "assoc_const"))
            {
                let before = sink.rows.len();
                self.member_row(member, &base, None, false, sink);
                for row in &mut sink.rows[before..] {
                    row.entry.owner.clone_from(&owner);
                }
            }
        }
    }
}

/// The entry of an item at its own path.
fn put(sink: &mut Sink, home: &str, space: Space, label: &str, made: (String, String)) {
    sink.push(
        home.to_owned(),
        space,
        Entry {
            kind: label.to_owned(),
            shown: made.0,
            identity: made.1,
            owner: None,
        },
        None,
    );
}

fn is_synthetic(param: &Value) -> bool {
    flag(&param["kind"]["type"]["is_synthetic"])
}

/// `make` in both styles: for people, and for comparing.
fn both_styles(make: impl Fn(Style) -> String) -> (String, String) {
    (make(Style { names: true }), make(Style { names: false }))
}

/// The attributes of an item that a user depends on: it cannot be built
/// by hand, and how it is laid out.
fn attr_prefix(item: &Value) -> String {
    let mut out = String::new();
    for attr in items(&item["attrs"]) {
        if attr == "non_exhaustive" {
            out.push_str("#[non_exhaustive] ");
        } else if let Some(repr) = attr.get("repr")
            && let Some(text) = repr_text(repr)
        {
            out.push_str(&format!("#[repr({text})] "));
        }
    }
    out
}

fn repr_text(repr: &Value) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    match repr["kind"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("c") => parts.push("C".to_owned()),
        Some("transparent") => parts.push("transparent".to_owned()),
        Some("simd") => parts.push("simd".to_owned()),
        _ => {}
    }
    if let Some(int) = repr["int"].as_str() {
        parts.push(int.to_owned());
    }
    match repr["packed"].as_u64() {
        Some(1) => parts.push("packed".to_owned()),
        Some(n) => parts.push(format!("packed({n})")),
        None => {}
    }
    if let Some(n) = repr["align"].as_u64() {
        parts.push(format!("align({n})"));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// How types, bounds and signatures are written.
#[derive(Clone, Copy)]
struct Style {
    /// Arguments carry their names. Off, for comparing: a caller does not
    /// see them.
    names: bool,
}

impl Style {
    fn ty(self, value: &Value) -> String {
        if value == "infer" {
            return "_".to_owned();
        }
        let Some((tag, v)) = single(value) else {
            return compact(value);
        };
        match tag {
            "primitive" if v == "never" => "!".to_owned(),
            "primitive" | "generic" => text(v),
            "resolved_path" => self.path(v),
            "tuple" => {
                let parts: Vec<String> = items(v).map(|part| self.ty(part)).collect();
                if parts.len() == 1 {
                    format!("({},)", parts[0])
                } else {
                    format!("({})", parts.join(", "))
                }
            }
            "slice" => format!("[{}]", self.ty(v)),
            "array" => format!("[{}; {}]", self.ty(&v["type"]), text(&v["len"])),
            "pat" => self.ty(&v["type"]),
            "borrowed_ref" => format!(
                "&{}{}{}",
                v["lifetime"]
                    .as_str()
                    .map_or(String::new(), |l| format!("{l} ")),
                if flag(&v["is_mutable"]) { "mut " } else { "" },
                self.ty(&v["type"])
            ),
            "raw_pointer" => format!(
                "*{} {}",
                if flag(&v["is_mutable"]) {
                    "mut"
                } else {
                    "const"
                },
                self.ty(&v["type"])
            ),
            "function_pointer" => self.fn_pointer(v),
            "impl_trait" => format!("impl {}", self.bounds(v)),
            "dyn_trait" => {
                let mut parts: Vec<String> = items(&v["traits"])
                    .map(|poly| {
                        format!(
                            "{}{}",
                            self.hrtb(&poly["generic_params"]),
                            self.path(&poly["trait"])
                        )
                    })
                    .collect();
                if let Some(lifetime) = v["lifetime"].as_str() {
                    parts.push(lifetime.to_owned());
                }
                format!("dyn {}", parts.join(" + "))
            }
            "qualified_path" => {
                let name = text(&v["name"]);
                let args = self.args(&v["args"]);
                let self_type = self.ty(&v["self_type"]);
                let trait_ = &v["trait"];
                if trait_.is_null() || trait_["path"] == "" {
                    format!("{self_type}::{name}{args}")
                } else {
                    format!("<{self_type} as {}>::{name}{args}", self.path(trait_))
                }
            }
            _ => compact(value),
        }
    }

    fn fn_pointer(self, v: &Value) -> String {
        let sig = &v["sig"];
        let mut inputs: Vec<String> = items(&sig["inputs"])
            .map(|pair| self.ty(&pair[1]))
            .collect();
        if flag(&sig["is_c_variadic"]) {
            inputs.push("...".to_owned());
        }
        format!(
            "{}{}fn({}){}",
            self.hrtb(&v["generic_params"]),
            header_words(&v["header"], false),
            inputs.join(", "),
            self.output(&sig["output"])
        )
    }

    fn output(self, output: &Value) -> String {
        if output.is_null() {
            String::new()
        } else {
            format!(" -> {}", self.ty(output))
        }
    }

    /// A path as written, with its arguments.
    fn path(self, path: &Value) -> String {
        format!("{}{}", text(&path["path"]), self.args(&path["args"]))
    }

    fn args(self, args: &Value) -> String {
        if args.is_null() {
            return String::new();
        }
        if let Some(angle) = args.get("angle_bracketed") {
            let mut parts: Vec<String> = items(&angle["args"])
                .map(|arg| self.generic_arg(arg))
                .collect();
            for constraint in items(&angle["constraints"]) {
                let name = format!(
                    "{}{}",
                    text(&constraint["name"]),
                    self.args(&constraint["args"])
                );
                let binding = &constraint["binding"];
                if let Some(term) = binding.get("equality") {
                    parts.push(format!("{name} = {}", self.term(term)));
                } else if let Some(bounds) = binding.get("constraint") {
                    parts.push(format!("{name}: {}", self.bounds(bounds)));
                } else {
                    parts.push(format!("{name} {}", compact(binding)));
                }
            }
            return if parts.is_empty() {
                String::new()
            } else {
                format!("<{}>", parts.join(", "))
            };
        }
        if let Some(parenthesized) = args.get("parenthesized") {
            let inputs: Vec<String> = items(&parenthesized["inputs"])
                .map(|input| self.ty(input))
                .collect();
            return format!(
                "({}){}",
                inputs.join(", "),
                self.output(&parenthesized["output"])
            );
        }
        compact(args)
    }

    fn generic_arg(self, arg: &Value) -> String {
        if arg == "infer" {
            return "_".to_owned();
        }
        match single(arg) {
            Some(("lifetime", lifetime)) => text(lifetime),
            Some(("type", ty)) => self.ty(ty),
            Some(("const", constant)) => text(&constant["expr"]),
            _ => compact(arg),
        }
    }

    fn term(self, term: &Value) -> String {
        match single(term) {
            Some(("type", ty)) => self.ty(ty),
            Some(("constant", constant)) => text(&constant["expr"]),
            _ => compact(term),
        }
    }

    /// Bounds joined as written: `Clone + Send + 'static`.
    fn bounds(self, bounds: &Value) -> String {
        items(bounds)
            .map(|bound| self.bound(bound))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    fn bound(self, bound: &Value) -> String {
        match single(bound) {
            Some(("trait_bound", v)) => {
                let modifier = match v["modifier"].as_str() {
                    Some("none") | None => String::new(),
                    Some("maybe") => "?".to_owned(),
                    Some("maybe_const") => "~const ".to_owned(),
                    Some(other) => format!("{other} "),
                };
                format!(
                    "{modifier}{}{}",
                    self.hrtb(&v["generic_params"]),
                    self.path(&v["trait"])
                )
            }
            Some(("outlives", lifetime)) => text(lifetime),
            Some(("use", captured)) => {
                let names: Vec<String> = items(captured).map(text).collect();
                format!("use<{}>", names.join(", "))
            }
            _ => compact(bound),
        }
    }

    /// `for<'a> ` before what it binds, or nothing.
    fn hrtb(self, params: &Value) -> String {
        let list: Vec<String> = items(params).map(|param| self.param(param)).collect();
        if list.is_empty() {
            String::new()
        } else {
            format!("for<{}> ", list.join(", "))
        }
    }

    fn param(self, param: &Value) -> String {
        let name = text(&param["name"]);
        match single(&param["kind"]) {
            Some(("lifetime", v)) => {
                let outlives: Vec<String> = items(&v["outlives"]).map(text).collect();
                if outlives.is_empty() {
                    name
                } else {
                    format!("{name}: {}", outlives.join(" + "))
                }
            }
            Some(("type", v)) => {
                let bounds = self.bounds(&v["bounds"]);
                let mut out = name;
                if !bounds.is_empty() {
                    out.push_str(&format!(": {bounds}"));
                }
                if !v["default"].is_null() {
                    out.push_str(&format!(" = {}", self.ty(&v["default"])));
                }
                out
            }
            Some(("const", v)) => {
                let mut out = format!("const {name}: {}", self.ty(&v["type"]));
                if let Some(default) = v["default"].as_str() {
                    out.push_str(&format!(" = {default}"));
                }
                out
            }
            _ => compact(param),
        }
    }

    /// `<'a, T: Clone, const N: usize>`, without what `impl Trait` in an
    /// argument makes up.
    fn params(self, generics: &Value) -> String {
        let list: Vec<String> = items(&generics["params"])
            .filter(|param| !is_synthetic(param))
            .map(|param| self.param(param))
            .collect();
        if list.is_empty() {
            String::new()
        } else {
            format!("<{}>", list.join(", "))
        }
    }

    fn where_clause(self, generics: &Value) -> String {
        let list: Vec<String> = items(&generics["where_predicates"])
            .map(|predicate| self.predicate(predicate))
            .collect();
        if list.is_empty() {
            String::new()
        } else {
            format!(" where {}", list.join(", "))
        }
    }

    fn predicate(self, predicate: &Value) -> String {
        match single(predicate) {
            Some(("bound_predicate", v)) => format!(
                "{}{}: {}",
                self.hrtb(&v["generic_params"]),
                self.ty(&v["type"]),
                self.bounds(&v["bounds"])
            ),
            Some(("lifetime_predicate", v)) => {
                let outlives: Vec<String> = items(&v["outlives"]).map(text).collect();
                format!("{}: {}", text(&v["lifetime"]), outlives.join(" + "))
            }
            Some(("eq_predicate", v)) => {
                format!("{} == {}", self.ty(&v["lhs"]), self.term(&v["rhs"]))
            }
            _ => compact(predicate),
        }
    }

    /// A function's signature: `const unsafe fn name<T>(a: T) -> U where ...`.
    fn function(self, name: &str, function: &Value) -> String {
        let sig = &function["sig"];
        let mut inputs: Vec<String> = items(&sig["inputs"])
            .map(|pair| self.input(&text(&pair[0]), &pair[1]))
            .collect();
        if flag(&sig["is_c_variadic"]) {
            inputs.push("...".to_owned());
        }
        format!(
            "{}fn {name}{}({}){}{}",
            header_words(&function["header"], true),
            self.params(&function["generics"]),
            inputs.join(", "),
            self.output(&sig["output"]),
            self.where_clause(&function["generics"])
        )
    }

    fn input(self, name: &str, ty: &Value) -> String {
        if name == "self" {
            let is_self = |value: &Value| single(value) == Some(("generic", &Value::from("Self")));
            if is_self(ty) {
                return "self".to_owned();
            }
            if let Some(("borrowed_ref", v)) = single(ty)
                && is_self(&v["type"])
            {
                return format!(
                    "&{}{}self",
                    v["lifetime"]
                        .as_str()
                        .map_or(String::new(), |l| format!("{l} ")),
                    if flag(&v["is_mutable"]) { "mut " } else { "" }
                );
            }
            return format!("self: {}", self.ty(ty));
        }
        if self.names {
            format!("{name}: {}", self.ty(ty))
        } else {
            self.ty(ty)
        }
    }
}

/// `const async unsafe extern "C" ` before `fn`.
fn header_words(header: &Value, with_const_and_async: bool) -> String {
    let mut out = String::new();
    if with_const_and_async {
        if flag(&header["is_const"]) {
            out.push_str("const ");
        }
        if flag(&header["is_async"]) {
            out.push_str("async ");
        }
    }
    if flag(&header["is_unsafe"]) {
        out.push_str("unsafe ");
    }
    let abi = match &header["abi"] {
        Value::String(name) if name == "Rust" => None,
        Value::String(name) => Some(name.clone()),
        Value::Object(object) => object
            .iter()
            .next()
            .map(|(name, detail)| match detail.as_str() {
                Some(other) => other.to_owned(),
                None if flag(&detail["unwind"]) => format!("{name}-unwind"),
                None => name.clone(),
            }),
        _ => None,
    };
    if let Some(abi) = abi {
        out.push_str(&format!("extern \"{abi}\" "));
    }
    out
}

#[cfg(test)]
mod tests;
