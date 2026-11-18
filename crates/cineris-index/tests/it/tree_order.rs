//! The map's tree draws a directory's subdirectories under it, whatever
//! characters the names of its neighbours have (the review of step 18:
//! `-` and `.` sort before `/` as bytes).

use cineris_index::{MapRequest, path_order};
use cineris_testkit::{Rng, SeededRng};

use super::index::Tree;

#[test]
fn subdirectories_are_drawn_under_their_own_directory() {
    let tree = Tree::new(cineris_testkit::scratch!("subdirectories_are_drawn_under"));
    tree.write("a/x/f.rs", "pub fn f() {}\n");
    tree.write("a-b/g.rs", "pub fn g() {}\n");
    tree.write("a.old/h.rs", "pub fn h() {}\n");
    let index = tree.index();
    tree.pass(&index);
    let map = index.map(&MapRequest::new()).expect("a map");
    let tree_part = map
        .split("Directories:\n")
        .nth(1)
        .and_then(|rest| rest.split("\n\n").next())
        .expect("a tree");
    assert_eq!(tree_part, "a/ (1)\n  x/ (1)\na-b/ (1)\na.old/ (1)", "{map}");
}

/// The order compares bytes with `/` as the least of them, which is the
/// order of comparing the names one by one, on any paths (the walk on
/// several threads sorts its listing by it).
#[test]
fn property_the_path_order_is_the_order_name_by_name() {
    const PIECES: &[&str] = &["a", "b", "Z", "0", "-", ".", " ", "é", "~", "/"];
    let mut rng = SeededRng::new(cineris_testkit::seed::for_this_test());
    let path = |rng: &mut SeededRng| {
        let length = 1 + rng.below(8);
        let mut text = String::new();
        for _ in 0..length {
            text.push_str(PIECES[usize::try_from(rng.below(PIECES.len() as u64)).unwrap_or(0)]);
        }
        // No empty name: a listing's paths have none.
        text.split('/')
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join("/")
    };
    for _ in 0..20_000 {
        let one = path(&mut rng);
        let other = path(&mut rng);
        assert_eq!(
            path_order(&one, &other),
            one.split('/').cmp(other.split('/')),
            "{one:?} against {other:?}"
        );
    }
}
