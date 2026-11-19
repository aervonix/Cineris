//! `cargo xtask mutate --budget TIME`: the most relevant breaks first, and
//! no new break started once the time is spent.
//!
//! The chosen breaks are put in four classes, and run class by class, each
//! in the order of its lists:
//!
//! 1. **touched:** the diff touches the break's `file`, or the break itself
//!    is new or changed in its list;
//! 2. **by a test:** a test its `caught-by` names was touched;
//! 3. **by its crate:** a file of a crate it runs was touched (a test name
//!    that could not be placed, a file that is not Rust, the crate's
//!    manifest or build script, or a crate it is built on), or a data file
//!    a crate it runs reads, or one of theirs;
//! 4. **the rest:** chosen for another reason (the build's settings, a file
//!    no crate names), or with no `--changed` at all.
//!
//! Each class is run as a run of its own (the tests with nothing broken for
//! the crates its breaks run, then each break), so that a budget spent on
//! the first class has not also run the baselines of the last. No new piece
//! of work is started once the budget is spent; the pieces under way finish
//! and are kept. What a break found is kept as in any run, so the same
//! command run again carries on from where this one stopped.

use std::fmt::Write as _;
use std::time::Duration;

use super::drive::Finished;
use super::list::Break;
use crate::Result;

/// How relevant a chosen break is to the diff: the first runs first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    /// Its file, or the break itself, was touched.
    Touched,
    /// A test its `caught-by` names was touched.
    Test,
    /// A file of a crate it runs, or a data file one of them reads.
    Crate,
    /// Anything else.
    Rest,
}

impl Class {
    pub(crate) const ALL: [Class; 4] = [Class::Touched, Class::Test, Class::Crate, Class::Rest];

    /// What a class is called in what is printed.
    pub(crate) fn said(self) -> &'static str {
        match self {
            Class::Touched => "touched (its file, or the break itself)",
            Class::Test => "by a test it names",
            Class::Crate => "by its crate (a file of it, or a data file it reads)",
            Class::Rest => "the rest",
        }
    }
}

/// A length of time as `--budget` takes it: a number and its unit, `s`,
/// `m` or `h`, and any of them together (`1h30m`, `90s`). A number alone
/// is refused: its unit is not to be guessed. So is no time at all.
pub(crate) fn parse_time(text: &str) -> Result<Duration> {
    let refused = || {
        format!(
            "`--budget {text}`: a length of time with its unit, such as 90s, 30m, 1h or \
             1h30m"
        )
    };
    let mut seconds = 0u64;
    let mut number = String::new();
    let mut units = 0;
    for character in text.trim().chars() {
        if character.is_ascii_digit() {
            number.push(character);
            continue;
        }
        let unit = match character {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            _ => return Err(refused()),
        };
        let value: u64 = number.parse().map_err(|_| refused())?;
        seconds = value
            .checked_mul(unit)
            .and_then(|more| seconds.checked_add(more))
            .ok_or_else(refused)?;
        number.clear();
        units += 1;
    }
    if !number.is_empty() || units == 0 {
        return Err(refused());
    }
    if seconds == 0 {
        return Err(format!(
            "`--budget {text}` is no time: leave it out for no limit (or `--minutes 0`)"
        ));
    }
    Ok(Duration::from_secs(seconds))
}

/// Whether a budget is spent: no new piece of work is started after.
pub(crate) fn spent(elapsed: Duration, budget: Duration) -> bool {
    elapsed >= budget
}

/// `breaks` in the order a budget runs them: by class, and in the order
/// given within one. `class_of` says a break's class.
pub(crate) fn ordered(
    breaks: Vec<Break>,
    class_of: impl Fn(&Break) -> Class,
) -> Vec<(Break, Class)> {
    let mut classed: Vec<(Break, Class)> = breaks
        .into_iter()
        .map(|one| {
            let class = class_of(&one);
            (one, class)
        })
        .collect();
    // Stable: the order of the lists within a class.
    classed.sort_by_key(|(_, class)| *class);
    classed
}

/// How many breaks of each class there are, in a line.
pub(crate) fn order_text(ordered: &[(Break, Class)], budget: &str) -> String {
    let counts: Vec<String> = Class::ALL
        .iter()
        .filter_map(|class| {
            let count = ordered.iter().filter(|(_, of)| of == class).count();
            (count > 0).then(|| format!("{count} {}", class.said()))
        })
        .collect();
    format!(
        "--budget {budget}: {} break(s), run in this order: {}.",
        ordered.len(),
        counts.join(", then ")
    )
}

/// Runs `ordered` a class at a time, in order: `drive` runs one class's
/// breaks, and `out_of_time` says whether the budget is spent, which is
/// asked before each class after the first (a class is not begun once it
/// is). A class that stops for the time ends the run. What it comes to:
/// complete, with every class's flagged breaks counted, or unfinished.
pub(crate) fn run_in_order(
    ordered: &[(Break, Class)],
    out_of_time: &mut dyn FnMut() -> bool,
    drive: &mut dyn FnMut(Class, &[Break]) -> Result<Finished>,
) -> Result<Finished> {
    let mut flagged = 0;
    let mut started = false;
    for class in Class::ALL {
        let breaks: Vec<Break> = ordered
            .iter()
            .filter(|(_, of)| *of == class)
            .map(|(one, _)| one.clone())
            .collect();
        if breaks.is_empty() {
            continue;
        }
        if started && out_of_time() {
            let left = ordered.iter().filter(|(_, of)| *of >= class).count();
            return Ok(Finished::Unfinished { left });
        }
        started = true;
        match drive(class, &breaks)? {
            Finished::Complete { flagged: more } => flagged += more,
            unfinished @ Finished::Unfinished { .. } => return Ok(unfinished),
        }
    }
    Ok(Finished::Complete { flagged })
}

/// What a budgeted run reached and what it left: for each class how many of
/// its breaks have a result, then each break left, by name, and how to run
/// them. `reached[i]` is whether `ordered[i]` has a result now. Empty when
/// every break was reached.
pub(crate) fn reached_text(ordered: &[(Break, Class)], reached: &[bool], command: &str) -> String {
    let left: Vec<&(Break, Class)> = ordered
        .iter()
        .zip(reached)
        .filter(|(_, reached)| !**reached)
        .map(|(one, _)| one)
        .collect();
    if left.is_empty() {
        return String::new();
    }
    let mut text = format!(
        "\nThe budget was spent: {} of {} break(s) reached.\n",
        ordered.len() - left.len(),
        ordered.len()
    );
    for class in Class::ALL {
        let of: Vec<bool> = ordered
            .iter()
            .zip(reached)
            .filter(|((_, each), _)| *each == class)
            .map(|(_, reached)| *reached)
            .collect();
        if !of.is_empty() {
            let _ = writeln!(
                text,
                "  {}: {} of {} reached",
                class.said(),
                of.iter().filter(|reached| **reached).count(),
                of.len()
            );
        }
    }
    let _ = writeln!(text, "Left ({}):", left.len());
    for (one, class) in &left {
        let _ = writeln!(text, "  {}  ({})", one.id, class.said());
    }
    let names: Vec<&str> = left.iter().map(|(one, _)| one.id.as_str()).collect();
    let _ = write!(
        text,
        "Run the same command again to carry on from here (what was found is kept):\n  \
         {command}\nor the breaks left alone:\n  cargo xtask mutate {}",
        names.join(" ")
    );
    text
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        Class, Finished, order_text, ordered, parse_time, reached_text, run_in_order, spent,
    };
    use crate::mutate::list::{self, Break};

    fn breaks(names: &[&str]) -> Vec<Break> {
        let text: String = names
            .iter()
            .map(|name| {
                format!(
                    "[break] {name}\nrule: r\nfile: crates/demo/src/{name}.rs\nfind: x\nreplace: y\n\n"
                )
            })
            .collect();
        list::parse("mutations/demo.breaks", &text).unwrap()
    }

    #[test]
    fn a_time_needs_its_unit_and_may_have_several() {
        assert_eq!(parse_time("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_time("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_time("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_time("1h30m").unwrap(), Duration::from_secs(5400));
        for bad in ["30", "", "m", "1d", "1.5h", "-5m", "30m5"] {
            let error = parse_time(bad).unwrap_err();
            assert!(error.contains("with its unit"), "{bad}: {error}");
        }
        assert!(parse_time("0m").unwrap_err().contains("no time"));
        assert!(parse_time("99999999999999999999h").is_err());
    }

    #[test]
    fn the_budget_is_spent_at_its_length_and_not_before() {
        let budget = Duration::from_secs(60);
        assert!(!spent(Duration::from_secs(59), budget));
        assert!(spent(Duration::from_secs(60), budget));
        assert!(spent(Duration::from_secs(61), budget));
    }

    #[test]
    fn the_breaks_are_ordered_by_class_and_by_their_lists_within_one() {
        let classes = |one: &Break| match one.id.as_str() {
            "demo/a" | "demo/e" => Class::Rest,
            "demo/b" => Class::Crate,
            "demo/c" => Class::Touched,
            _ => Class::Test,
        };

        let got = ordered(breaks(&["a", "b", "c", "d", "e", "f"]), classes);

        assert_eq!(
            got.iter()
                .map(|(one, class)| (one.id.as_str(), *class))
                .collect::<Vec<_>>(),
            [
                ("demo/c", Class::Touched),
                ("demo/d", Class::Test),
                ("demo/f", Class::Test),
                ("demo/b", Class::Crate),
                ("demo/a", Class::Rest),
                ("demo/e", Class::Rest),
            ]
        );
        assert_eq!(
            order_text(&got, "30m"),
            "--budget 30m: 6 break(s), run in this order: 1 touched (its file, or the break \
             itself), then 2 by a test it names, then 1 by its crate (a file of it, or a data \
             file it reads), then 2 the rest."
        );
    }

    #[test]
    fn each_class_is_run_in_turn_and_none_is_begun_once_the_time_is_spent() {
        let classed = ordered(breaks(&["a", "b", "c", "d"]), |one| match one.id.as_str() {
            "demo/a" => Class::Touched,
            "demo/b" | "demo/c" => Class::Crate,
            _ => Class::Rest,
        });
        // Every class run, each with its own breaks, the flagged added up.
        let mut runs = Vec::new();
        let got = run_in_order(&classed, &mut || false, &mut |class, breaks| {
            runs.push((class, breaks.len()));
            Ok(Finished::Complete { flagged: 1 })
        })
        .unwrap();
        assert_eq!(got, Finished::Complete { flagged: 3 });
        assert_eq!(
            runs,
            [(Class::Touched, 1), (Class::Crate, 2), (Class::Rest, 1)]
        );

        // The time is spent after the first class: the second is not begun,
        // and the breaks of it and after it are left.
        let mut runs = Vec::new();
        let mut asked = 0;
        let got = run_in_order(
            &classed,
            &mut || {
                asked += 1;
                true
            },
            &mut |class, _| {
                runs.push(class);
                Ok(Finished::Complete { flagged: 0 })
            },
        )
        .unwrap();
        assert_eq!(got, Finished::Unfinished { left: 3 });
        assert_eq!(runs, [Class::Touched]);
        assert_eq!(asked, 1, "the first class is begun without asking");

        // A class that stops for the time ends the run as it ended.
        let mut runs = Vec::new();
        let got = run_in_order(&classed, &mut || false, &mut |class, _| {
            runs.push(class);
            Ok(if class == Class::Crate {
                Finished::Unfinished { left: 2 }
            } else {
                Finished::Complete { flagged: 0 }
            })
        })
        .unwrap();
        assert_eq!(got, Finished::Unfinished { left: 2 });
        assert_eq!(runs, [Class::Touched, Class::Crate]);
    }

    #[test]
    fn what_was_reached_is_counted_by_class_and_what_was_left_is_named() {
        let classed = ordered(breaks(&["a", "b", "c"]), |one| match one.id.as_str() {
            "demo/a" => Class::Touched,
            _ => Class::Rest,
        });
        let command = "cargo xtask mutate --changed main --budget 10m";

        let said = reached_text(&classed, &[true, true, false], command);

        assert!(said.contains("2 of 3 break(s) reached"), "{said}");
        assert!(
            said.contains("touched (its file, or the break itself): 1 of 1 reached"),
            "{said}"
        );
        assert!(said.contains("the rest: 1 of 2 reached"), "{said}");
        assert!(said.contains("Left (1):\n  demo/c  (the rest)"), "{said}");
        assert!(said.contains(command), "{said}");
        assert!(said.ends_with("cargo xtask mutate demo/c"), "{said}");
        // All reached: nothing to say.
        assert_eq!(reached_text(&classed, &[true; 3], command), "");
    }
}
