use super::super::read::{Api, Entry, Key, Space};
use super::*;

fn entry(kind: &str, shown: &str) -> Entry {
    Entry {
        kind: kind.to_owned(),
        shown: shown.to_owned(),
        identity: shown.to_owned(),
        owner: None,
    }
}

fn api(entries: &[(&str, Space, Entry)]) -> Api {
    entries
        .iter()
        .map(|(path, space, entry)| {
            (
                Key {
                    path: (*path).to_owned(),
                    space: *space,
                },
                entry.clone(),
            )
        })
        .collect()
}

fn summary(deltas: &[Delta]) -> Vec<String> {
    deltas
        .iter()
        .map(|delta| format!("{:?} {}", delta.change, delta.key.path))
        .collect()
}

#[test]
fn items_added_removed_and_changed_come_out_in_the_order_of_their_paths() {
    let before = api(&[
        ("c::keep", Space::Value, entry("fn", "fn keep()")),
        ("c::gone", Space::Value, entry("fn", "fn gone()")),
        ("c::go", Space::Value, entry("fn", "fn go(a: u8)")),
    ]);
    let after = api(&[
        ("c::keep", Space::Value, entry("fn", "fn keep()")),
        ("c::go", Space::Value, entry("fn", "fn go(a: u8, b: u8)")),
        ("c::new", Space::Value, entry("fn", "fn new()")),
        ("c::a_first", Space::Type, entry("struct", "struct A")),
    ]);
    let deltas = compare(&before, &after);
    assert_eq!(
        summary(&deltas),
        [
            "Added c::a_first",
            "Changed c::go",
            "Removed c::gone",
            "Added c::new"
        ]
    );
    let changed = &deltas[1];
    assert_eq!(changed.before.as_ref().unwrap().shown, "fn go(a: u8)");
    assert_eq!(changed.after.as_ref().unwrap().shown, "fn go(a: u8, b: u8)");
}

#[test]
fn a_change_is_judged_on_the_identity_and_not_on_what_is_shown() {
    let mut renamed = entry("fn", "fn f(b: u8)");
    renamed.identity = "fn f(u8)".to_owned();
    let mut original = entry("fn", "fn f(a: u8)");
    original.identity = "fn f(u8)".to_owned();
    let before = api(&[("c::f", Space::Value, original)]);
    let after = api(&[("c::f", Space::Value, renamed)]);
    assert!(compare(&before, &after).is_empty());
}

#[test]
fn an_item_that_became_another_kind_is_changed_even_with_one_signature() {
    let before = api(&[("c::x", Space::Type, entry("struct", "x"))]);
    let after = api(&[("c::x", Space::Type, entry("enum", "x"))]);
    assert_eq!(summary(&compare(&before, &after)), ["Changed c::x"]);
}

#[test]
fn the_same_path_in_two_namespaces_is_two_items() {
    let before = api(&[("c::x", Space::Type, entry("struct", "struct x"))]);
    let after = api(&[
        ("c::x", Space::Type, entry("struct", "struct x")),
        ("c::x", Space::Value, entry("fn", "fn x()")),
    ]);
    let deltas = compare(&before, &after);
    assert_eq!(summary(&deltas), ["Added c::x"]);
    assert_eq!(deltas[0].key.space, Space::Value);
}

#[test]
fn the_impls_of_a_type_added_or_removed_with_it_are_not_listed_beside_it() {
    let mut derived = entry("impl", "impl Clone for c::T");
    derived.owner = Some("c::T".to_owned());
    let ty = entry("struct", "struct T");
    // An impl of a type that was there before is listed.
    let mut for_old = entry("impl", "impl Clone for c::Old");
    for_old.owner = Some("c::Old".to_owned());
    let old = entry("struct", "struct Old");

    let none = api(&[("c::Old", Space::Type, old.clone())]);
    let both = api(&[
        ("c::Old", Space::Type, old),
        ("c::T", Space::Type, ty.clone()),
        ("impl Clone for c::T", Space::Impl, derived.clone()),
        ("impl Clone for c::Old", Space::Impl, for_old),
    ]);
    assert_eq!(
        summary(&compare(&none, &both)),
        ["Added c::T", "Added impl Clone for c::Old"]
    );
    // And the same when the type goes.
    assert_eq!(
        summary(&compare(&both, &none)),
        ["Removed c::T", "Removed impl Clone for c::Old"]
    );
}

#[test]
fn an_impl_changed_is_listed_whatever_its_owner() {
    let mut one = entry("impl", "impl<T: Clone> Tr for c::T");
    one.owner = Some("c::T".to_owned());
    let mut two = entry("impl", "impl<T: Copy> Tr for c::T");
    two.owner = Some("c::T".to_owned());
    let before = api(&[("impl Tr for c::T", Space::Impl, one)]);
    let after = api(&[("impl Tr for c::T", Space::Impl, two)]);
    assert_eq!(
        summary(&compare(&before, &after)),
        ["Changed impl Tr for c::T"]
    );
}

fn two_crates() -> Vec<CrateDeltas> {
    let before = api(&[
        ("c::go", Space::Value, entry("fn", "fn go(a: u8)")),
        (
            "c::gone",
            Space::Value,
            entry("fn", "fn gone() -> Option<u8 | u16>"),
        ),
    ]);
    let after = api(&[
        ("c::go", Space::Value, entry("fn", "fn go(a: u8, b: u8)")),
        ("c::new", Space::Value, entry("fn", "fn new()")),
    ]);
    vec![
        CrateDeltas {
            name: "c".to_owned(),
            note: None,
            deltas: compare(&before, &after),
        },
        CrateDeltas {
            name: "d".to_owned(),
            note: Some("a crate the ref does not have".to_owned()),
            deltas: Vec::new(),
        },
        CrateDeltas {
            name: "e".to_owned(),
            note: None,
            deltas: Vec::new(),
        },
    ]
}

#[test]
fn the_report_as_text_has_a_line_for_an_addition_and_three_for_a_change() {
    let expected = "\
Public API, `main` (abc) against the working tree
c: 1 added, 1 removed, 1 changed
  ~ c::go
      was  fn go(a: u8)
      now  fn go(a: u8, b: u8)
  - c::gone  fn gone() -> Option<u8 | u16>
  + c::new  fn new()
no public API changes: d, e
";
    assert_eq!(
        text(
            "Public API, `main` (abc) against the working tree",
            &two_crates()
        ),
        expected
    );
}

#[test]
fn the_report_as_markdown_is_a_table_for_each_crate_with_changes() {
    let expected = "\
Public API, `main` (abc) against the working tree

**c**: 1 added, 1 removed, 1 changed

| | Item | Before | After |
|---|---|---|---|
| changed | `c::go` | `fn go(a: u8)` | `fn go(a: u8, b: u8)` |
| removed | `c::gone` | `fn gone() -> Option<u8 \\| u16>` |  |
| added | `c::new` |  | `fn new()` |

No public API changes: d, e.
";
    assert_eq!(
        markdown(
            "Public API, `main` (abc) against the working tree",
            &two_crates()
        ),
        expected
    );
}

#[test]
fn a_crate_in_one_tree_only_says_so() {
    let mut krate = two_crates().remove(0);
    krate.note = Some("a crate the working tree no longer has".to_owned());
    let out = text("h", &[krate]);
    assert!(
        out.contains("c: 1 added, 1 removed, 1 changed (a crate the working tree no longer has)"),
        "{out}"
    );
}
