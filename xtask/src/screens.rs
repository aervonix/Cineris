//! `cargo xtask tui-screens [--write]`: the golden screens.
//!
//! The reference screens of the chosen look (`docs/architecture/tui-look/`)
//! are drawn again by `cortex-tui`'s views through the inline presenter into
//! the test kit's virtual terminal, and compared cell by cell with the
//! reference files. This runs that comparison, which is an ordinary test of
//! `cortex-tui`'s integration binary (tier 2, so every pull request runs it
//! too), and shows its report: each screen's rows compared, the rows other
//! steps draw that are not built yet, and every cell that differs.
//!
//! With `--write`, the rows that differ are written into the reference files
//! from the real views, and the report lists every cell changed: the only
//! way a reference screen changes (`docs/testing-and-ci.md`, "The golden
//! screens").

use crate::Result;
use crate::outside;

/// The test that compares the screens.
const TEST: &str = "screens::the_views_draw_the_reference_screens";

fn parse(arguments: &[String]) -> Result<bool> {
    match arguments {
        [] => Ok(false),
        [flag] if flag == "--write" => Ok(true),
        _ => Err("usage: cargo xtask tui-screens [--write]".to_owned()),
    }
}

/// `cargo xtask tui-screens [--write]`.
pub(crate) fn run(arguments: &[String]) -> Result<()> {
    let write = parse(arguments)?;
    let variables: &[(&str, &str)] = if write {
        &[("CORTEX_UPDATE_SCREENS", "1")]
    } else {
        &[]
    };
    let ran = outside::run_test(
        &["--package", "cortex-tui", "--test", "it"],
        TEST,
        variables,
        false,
    );
    if write && ran.is_err() {
        // The comparison fails on the run that rewrites what differed: the
        // files now hold the new rows, and a run that writes nothing must
        // pass. If it does not, something else failed (the build, or a count
        // of rows compared), and that is the result.
        println!("The rows that differed are rewritten; the cells changed are listed above.");
        println!("Comparing again, writing nothing:");
        return outside::run_test(
            &["--package", "cortex-tui", "--test", "it"],
            TEST,
            &[],
            false,
        );
    }
    ran
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        assert_eq!(parse(&[]), Ok(false));
        assert_eq!(parse(&["--write".to_owned()]), Ok(true));
        assert!(parse(&["--check".to_owned()]).is_err());
    }
}
