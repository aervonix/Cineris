use serde_json::{Value, json};

use super::*;

/// A crate of two modules, written as the pinned toolchain's rustdoc
/// writes it (`xtask/fixtures/api-diff/toy.rs.txt`, with what the reader
/// does not read taken out): the `use`s of a private module, a glob, tuple
/// and plain structs, an enum with a `repr`, a trait with defaults, `impl`s
/// written and derived, `unsafe extern "C"` and every shape of type.
fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/api-diff/toy.json")).unwrap()
}

fn listing(api: &Api) -> String {
    api.iter()
        .map(|(key, entry)| format!("{} | {} | {}\n", key.path, entry.kind, entry.shown))
        .collect()
}

#[test]
fn a_real_crate_is_read_as_the_signatures_its_source_has() {
    let api = read(&fixture()).unwrap();
    let expected = "\
<toy2::P as toy2::Tr<T>>::Out | type | type Out = IntoIter<u8>
impl Clone for toy2::P | impl | impl Clone for P
impl Debug for toy2::P | impl | impl Debug for P
impl From<T> for toy2::Tup | impl | impl<T: Clone> From<T> for Tup
impl PartialEq for toy2::P | impl | impl PartialEq for P
impl StructuralPartialEq for toy2::P | impl | impl StructuralPartialEq for P
impl toy2::Tr<T> for toy2::P | impl | impl<T: Copy> Tr<T> for P
toy2::Alias | type | type Alias<T> = Result<T, Box<dyn std::error::Error + Send + 'static>>
toy2::E | enum | #[non_exhaustive] #[repr(u8)] enum E
toy2::E::A | variant | A = 1
toy2::E::B | variant | B(u8, String)
toy2::E::C | variant | C { x: Vec<u8> }
toy2::K | const | const K: usize
toy2::Only | struct | struct Only(..)
toy2::Only::0 | field | 0: u8
toy2::P | struct | #[non_exhaustive] struct P { /* private fields */ }
toy2::P::a | fn | async fn a(&self)
toy2::P::a | field | a: u8
toy2::P::c | fn | const fn c(self) -> u8
toy2::P::new | fn | fn new() -> Self
toy2::P::u | fn | unsafe fn u(&self)
toy2::S | static | static S: &str
toy2::SM | static | static mut SM: u8
toy2::Tr | trait | trait Tr<T = u8>: Send where T: Copy
toy2::Tr::N | const | const N: usize = ..
toy2::Tr::Out | type | type Out: Iterator<Item = u8>
toy2::Tr::d | fn | fn d(&mut self) { .. }
toy2::Tr::m | fn | fn m(&self) -> Self::Out where Self: Sized
toy2::Tup | struct | struct Tup(..) /* private fields */
toy2::Tup::0 | field | 0: u8
toy2::U | union | union U
toy2::U::a | field | a: u8
toy2::U::b | field | b: u16
toy2::Un | trait | unsafe trait Un
toy2::Unit | struct | struct Unit;
toy2::cabi | fn | unsafe extern \"C\" fn cabi(x: u8, ...)
toy2::genr | fn | fn genr<'a: 'b, 'b, const N: usize>(a: [u8; N], b: &'a mut u8) -> <P as Tr>::Out
toy2::glob | mod | mod glob
toy2::glob::D | const | const D: u8
toy2::glob::Local | struct | struct Local;
toy2::helper | fn | fn helper()
toy2::hr | fn | fn hr<F>(f: F) -> impl Fn(u8) -> u8 where F: for<'a> Fn(&'a str) -> &'a str
toy2::mm | macro | macro mm!
toy2::old | fn | fn old()
toy2::ptr | fn | fn ptr(p: *const u8, q: *mut [u8; 4], r: fn(u8) -> u8, d: &dyn Fn(u8) -> u8, t: (u8, i8), s: &'static str) -> !
toy2::shallow | mod | mod shallow
toy2::shallow::D | use | pub use toy2::glob::D
";
    assert_eq!(listing(&api), expected);
}

/// What a name lives in: a field and a method of one name are two entries.
#[test]
fn a_field_and_a_method_of_one_name_are_not_one_entry() {
    let api = read(&fixture()).unwrap();
    let spaces: Vec<Space> = api
        .keys()
        .filter(|key| key.path == "toy2::P::a")
        .map(|key| key.space)
        .collect();
    assert_eq!(spaces, [Space::Value, Space::Field]);
}

#[test]
fn an_item_in_a_private_module_is_at_the_path_that_reexports_it() {
    let api = read(&fixture()).unwrap();
    let only = api
        .get(&Key {
            path: "toy2::Only".to_owned(),
            space: Space::Type,
        })
        .unwrap();
    assert_eq!(only.kind, "struct");
    // The module's other path is a `use`: a second path to the one item.
    let shallow = api
        .get(&Key {
            path: "toy2::shallow::D".to_owned(),
            space: Space::Value,
        })
        .unwrap();
    assert_eq!(
        (shallow.kind.as_str(), shallow.shown.as_str()),
        ("use", "pub use toy2::glob::D")
    );
    // Where an item is defined in a public module, that is its path, and
    // the shorter path that reexports it is the `use`.
    assert!(api.keys().all(|key| !key.path.contains("hidden")));
}

#[test]
fn a_format_the_reader_was_not_written_for_is_refused_with_both_numbers() {
    let mut json = fixture();
    json["format_version"] = json!(62);
    let error = read(&json).unwrap_err();
    assert!(error.contains("format 62"), "{error}");
    assert!(error.contains("reads format 61 only"), "{error}");
    let error = read(&json!({"root": 0, "index": {}})).unwrap_err();
    assert!(error.contains("not a rustdoc JSON document"), "{error}");
}

#[test]
fn json_with_private_items_is_refused() {
    let mut json = fixture();
    json["includes_private"] = json!(true);
    let error = read(&json).unwrap_err();
    assert!(error.contains("private items"), "{error}");
}

/// A document of a crate whose root module holds `items`, and `others`
/// that are only in the index.
fn document(root_items: &[u64], items: &[Value]) -> Value {
    let mut index = serde_json::Map::new();
    index.insert(
        "0".to_owned(),
        json!({
            "id": 0, "crate_id": 0, "name": "demo", "visibility": "public",
            "attrs": [],
            "inner": {"module": {"is_crate": true, "items": root_items, "is_stripped": false}},
        }),
    );
    for item in items {
        index.insert(item["id"].to_string(), item.clone());
    }
    json!({
        "root": 0, "format_version": 61, "includes_private": false,
        "index": index, "paths": {}, "external_crates": {},
    })
}

fn item(id: u64, name: &str, visibility: &str, inner: Value) -> Value {
    json!({
        "id": id, "crate_id": 0, "name": name, "visibility": visibility,
        "attrs": [], "inner": inner,
    })
}

fn function(id: u64, name: &str, visibility: &str, inputs: Value) -> Value {
    item(
        id,
        name,
        visibility,
        json!({"function": {
            "sig": {"inputs": inputs, "output": null, "is_c_variadic": false},
            "generics": {"params": [], "where_predicates": []},
            "header": {"is_const": false, "is_unsafe": false, "is_async": false, "abi": "Rust"},
            "has_body": true,
        }}),
    )
}

fn paths(api: &Api) -> Vec<&str> {
    api.keys().map(|key| key.path.as_str()).collect()
}

#[test]
fn only_what_is_public_is_listed() {
    let json = document(
        &[1, 2, 3],
        &[
            function(1, "open", "public", json!([])),
            function(2, "hidden", "crate", json!([])),
            function(3, "restricted", "default", json!([])),
        ],
    );
    assert_eq!(paths(&read(&json).unwrap()), ["demo::open"]);
}

#[test]
fn the_names_of_arguments_are_shown_and_do_not_count() {
    let pair = |name: &str| json!([[name, {"primitive": "u8"}], ["_", {"primitive": "bool"}]]);
    let one = read(&document(&[1], &[function(1, "f", "public", pair("a"))])).unwrap();
    let other = read(&document(&[1], &[function(1, "f", "public", pair("b"))])).unwrap();
    let key = Key {
        path: "demo::f".to_owned(),
        space: Space::Value,
    };
    assert_eq!(one[&key].shown, "fn f(a: u8, _: bool)");
    assert_eq!(other[&key].shown, "fn f(b: u8, _: bool)");
    assert_eq!(one[&key].identity, "fn f(u8, bool)");
    assert_eq!(one[&key].identity, other[&key].identity);
}

#[test]
fn what_rustdoc_adds_to_every_type_is_left_out_and_a_type_that_is_not_send_is_not() {
    let plain = |id: u64, trait_name: &str, synthetic: bool, negative: bool, blanket: Value| {
        item(
            id,
            "",
            "default",
            json!({"impl": {
                "is_unsafe": false,
                "generics": {"params": [], "where_predicates": []},
                "trait": {"path": trait_name, "id": 90 + id, "args": null},
                "for": {"resolved_path": {"path": "T", "id": 1, "args": null}},
                "items": [], "is_negative": negative, "is_synthetic": synthetic,
                "blanket_impl": blanket,
            }}),
        )
    };
    let json = document(
        &[1],
        &[
            item(
                1,
                "T",
                "public",
                json!({"struct": {"kind": "unit", "generics": {"params": [], "where_predicates": []}}}),
            ),
            plain(10, "Sync", true, false, Value::Null),
            plain(11, "Send", true, true, Value::Null),
            plain(12, "From", false, false, json!({"generic": "T"})),
            plain(13, "Default", false, false, Value::Null),
        ],
    );
    let api = read(&json).unwrap();
    assert_eq!(
        paths(&api),
        [
            "demo::T",
            "impl !Send for demo::T",
            "impl Default for demo::T"
        ]
    );
}

#[test]
fn methods_of_one_name_in_two_impls_are_told_apart_by_the_impl() {
    let impl_for = |id: u64, arg: &str, method: u64| {
        item(
            id,
            "",
            "default",
            json!({"impl": {
                "is_unsafe": false,
                "generics": {"params": [], "where_predicates": []},
                "trait": null,
                "for": {"resolved_path": {"path": "A", "id": 1, "args": {"angle_bracketed": {
                    "args": [{"type": {"primitive": arg}}], "constraints": []}}}},
                "items": [method], "is_negative": false, "is_synthetic": false,
                "blanket_impl": null,
            }}),
        )
    };
    let json = document(
        &[1],
        &[
            item(
                1,
                "A",
                "public",
                json!({"struct": {"kind": "unit", "generics": {
                    "params": [{"name": "T", "kind": {"type": {"bounds": [], "default": null, "is_synthetic": false}}}],
                    "where_predicates": []}}}),
            ),
            impl_for(10, "u8", 11),
            function(11, "x", "public", json!([])),
            impl_for(12, "u16", 13),
            function(13, "x", "public", json!([["n", {"primitive": "u16"}]])),
        ],
    );
    let api = read(&json).unwrap();
    let methods: Vec<(&str, &str)> = api
        .iter()
        .filter(|(_, entry)| entry.kind == "fn")
        .map(|(key, entry)| (key.path.as_str(), entry.shown.as_str()))
        .collect();
    assert_eq!(
        methods,
        [
            ("demo::A::x [impl A<u16>]", "fn x(n: u16) (in impl A<u16>)"),
            ("demo::A::x [impl A<u8>]", "fn x() (in impl A<u8>)"),
        ]
    );
}

#[test]
fn a_type_the_reader_does_not_know_is_kept_as_it_is_so_a_change_in_it_shows() {
    let odd = |shape: &str| {
        document(
            &[1],
            &[item(
                1,
                "k",
                "public",
                json!({"constant": {"type": {"unheard_of": {"shape": shape}},
                    "const": {"expr": "1", "value": "1", "is_literal": true}}}),
            )],
        )
    };
    let key = Key {
        path: "demo::k".to_owned(),
        space: Space::Value,
    };
    let one = read(&odd("one")).unwrap();
    let two = read(&odd("two")).unwrap();
    assert_eq!(
        one[&key].shown,
        r#"const k: {"unheard_of":{"shape":"one"}}"#
    );
    assert_ne!(one[&key].identity, two[&key].identity);
}

#[test]
fn a_trait_method_with_a_default_says_so_and_a_required_one_does_not() {
    let api = read(&fixture()).unwrap();
    let shown = |path: &str| {
        api.iter()
            .find(|(key, _)| key.path == path && key.space == Space::Value)
            .map(|(_, entry)| entry.shown.clone())
            .unwrap()
    };
    assert_eq!(shown("toy2::Tr::d"), "fn d(&mut self) { .. }");
    assert_eq!(
        shown("toy2::Tr::m"),
        "fn m(&self) -> Self::Out where Self: Sized"
    );
    // An associated constant with a default, and without.
    assert_eq!(shown("toy2::Tr::N"), "const N: usize = ..");
}

#[test]
fn bounds_are_written_as_rust_writes_them() {
    let style = Style { names: true };
    let bound = |modifier: &str, name: &str| {
        json!({"trait_bound": {"trait": {"path": name, "id": 1, "args": null},
            "generic_params": [], "modifier": modifier}})
    };
    assert_eq!(style.bound(&bound("maybe", "Sized")), "?Sized");
    assert_eq!(style.bound(&bound("none", "Clone")), "Clone");
    assert_eq!(style.bound(&json!({"outlives": "'static"})), "'static");
    // An argument of `impl Trait` is a parameter made up: not listed.
    let generics = json!({"params": [
        {"name": "T", "kind": {"type": {"bounds": [bound("none", "Clone")], "default": null, "is_synthetic": false}}},
        {"name": "impl Fn()", "kind": {"type": {"bounds": [], "default": null, "is_synthetic": true}}},
    ], "where_predicates": []});
    assert_eq!(style.params(&generics), "<T: Clone>");
}

fn unit_struct(id: u64, name: &str) -> Value {
    item(
        id,
        name,
        "public",
        json!({"struct": {"kind": "unit", "generics": {"params": [], "where_predicates": []}}}),
    )
}

fn module(id: u64, name: &str, visibility: &str, members: &[u64]) -> Value {
    item(
        id,
        name,
        visibility,
        json!({"module": {"is_crate": false, "items": members, "is_stripped": false}}),
    )
}

fn use_of(id: u64, name: &str, target: Value, source: &str) -> Value {
    item(
        id,
        "",
        "public",
        json!({"use": {"source": source, "name": name, "id": target, "is_glob": false}}),
    )
}

fn entry_at<'a>(api: &'a Api, path: &str) -> &'a Entry {
    api.iter()
        .find(|(key, _)| key.path == path)
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| panic!("no entry at {path}: {:?}", paths(api)))
}

#[test]
fn an_item_defined_in_a_public_module_is_there_and_a_reexport_is_a_use_of_it() {
    let json = document(
        &[1, 3],
        &[
            module(1, "inner", "public", &[2]),
            unit_struct(2, "X"),
            use_of(3, "X", json!(2), "inner::X"),
        ],
    );
    let api = read(&json).unwrap();
    assert_eq!(entry_at(&api, "demo::inner::X").kind, "struct");
    let reexport = entry_at(&api, "demo::X");
    assert_eq!(
        (reexport.kind.as_str(), reexport.shown.as_str()),
        ("use", "pub use demo::inner::X")
    );
}

#[test]
fn an_item_in_a_private_module_is_at_the_shortest_of_the_paths_that_reexport_it() {
    // `hid` is not among any module's items: it is private. `Y` is brought
    // out at the root as `zed` and again, renamed `a` (which sorts first),
    // in the module `m`; `Display` is
    // another crate's.
    let json = document(
        &[3, 4, 5],
        &[
            unit_struct(2, "Y"),
            use_of(3, "zed", json!(2), "hid::Y"),
            module(4, "m", "public", &[6]),
            use_of(6, "a", json!(2), "super::hid::Y"),
            use_of(5, "Display", Value::Null, "std::fmt::Display"),
            module(7, "hid", "default", &[2]),
        ],
    );
    let api = read(&json).unwrap();
    assert_eq!(entry_at(&api, "demo::zed").kind, "struct");
    assert_eq!(entry_at(&api, "demo::m::a").shown, "pub use demo::zed");
    assert_eq!(
        entry_at(&api, "demo::Display").shown,
        "pub use std::fmt::Display"
    );
    assert!(api.keys().all(|key| !key.path.contains("hid")));
}

#[test]
fn an_impl_and_what_it_fixes_belong_to_the_type_they_are_for() {
    let json = document(
        &[1],
        &[
            unit_struct(1, "T"),
            item(
                10,
                "",
                "default",
                json!({"impl": {
                    "is_unsafe": false,
                    "generics": {"params": [], "where_predicates": []},
                    "trait": {"path": "Iterator", "id": 99, "args": null},
                    "for": {"resolved_path": {"path": "T", "id": 1, "args": null}},
                    "items": [11], "is_negative": false, "is_synthetic": false,
                    "blanket_impl": null,
                }}),
            ),
            item(
                11,
                "Item",
                "default",
                json!({"assoc_type": {"generics": {"params": [], "where_predicates": []},
                    "bounds": [], "type": {"primitive": "u8"}}}),
            ),
        ],
    );
    let api = read(&json).unwrap();
    let owner = Some("demo::T".to_owned());
    let impl_ = entry_at(&api, "impl Iterator for demo::T");
    assert_eq!(impl_.owner, owner);
    let fixed = entry_at(&api, "<demo::T as Iterator>::Item");
    assert_eq!(fixed.shown, "type Item = u8");
    assert_eq!(fixed.owner, owner);
    // A type is not its own owner.
    assert_eq!(entry_at(&api, "demo::T").owner, None);
}
