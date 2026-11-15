//! The map's tree draws a directory's subdirectories under it, whatever
//! characters the names of its neighbours have (the review of step 18:
//! `-` and `.` sort before `/` as bytes).

use cortex_index::MapRequest;

use super::index::Tree;

#[test]
fn subdirectories_are_drawn_under_their_own_directory() {
    let tree = Tree::new(cortex_testkit::scratch!("subdirectories_are_drawn_under"));
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
