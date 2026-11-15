//! The index holds nothing in memory between uses
//! (`docs/performance-budgets.md`, "Memory": the repository index, 0 held).
//!
//! Each test runs in a process of its own under nextest, so the process's
//! resident memory is this test's. After a warm-up (the walker's statics,
//! the allocator's arenas for the pass's threads), more passes and maps
//! must not make it grow: what each one used is given back. Measured where
//! the system says it (Linux); counting the heap byte for byte would need
//! an allocator of the test's own, which is `unsafe` code, and the
//! workspace allows that in three named modules only.

use cortex_index::MapRequest;

use super::index::Tree;

#[test]
fn passes_and_maps_leave_nothing_behind() {
    let tree = Tree::new(cortex_testkit::scratch!(
        "passes_and_maps_leave_nothing_behind"
    ));
    for n in 0..400 {
        tree.write(
            &format!("src/m{:02}/f{n}.rs", n % 17),
            &format!(
                "pub struct S{n};\n\nimpl S{n} {{\n    pub fn go(&self) {{}}\n}}\n{}",
                "// padding\n".repeat(40)
            ),
        );
    }
    let index = tree.index().with_threads(2);
    let round = |n: usize| {
        tree.write(
            &format!("src/m00/f{}.rs", n * 17),
            &format!("pub fn changed_{n}() {{}}\n"),
        );
        let report = tree.pass(&index);
        assert_eq!(report.read, 1, "{report:?}");
        let map = index.map(&MapRequest::new()).expect("a map");
        assert!(!map.is_empty());
    };
    // Warm-up: a cold pass, then a few warm ones.
    tree.pass(&index);
    for n in 0..3 {
        round(n);
    }
    let Some(before) = super::resident_kib("VmRSS:") else {
        // No such figure on this system: nothing to measure. Linux has it.
        if cfg!(target_os = "linux") {
            panic!("Linux says its resident memory");
        }
        return;
    };
    for n in 3..23 {
        round(n);
    }
    let after = super::resident_kib("VmRSS:").expect("the figure again");
    assert!(
        after <= before + 2048,
        "twenty more passes and maps left {} KiB more resident ({before} KiB, then {after} KiB)",
        after.saturating_sub(before)
    );
}
