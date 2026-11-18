//! The processes each test started, as the test kit wrote them down
//! (`docs/testing-and-ci.md`, "What a runner would say, from a desk").
//!
//! With `CINERIS_TESTKIT_STARTS` naming a directory, every place in the
//! workspace that starts a process writes a line to a file of its process's
//! own there, `<binary id>\t<test name>\t<program>`, under nextest
//! (`cineris_base::hook::started`). A process started by a test's process
//! (the harness's binary, a test binary in a role) is given the variables
//! and writes for the same test. This reads the directory back: each test's
//! starts, by program and by [`Kind`].
//!
//! A start is what a hosted Windows runner charges most for and a desk
//! cannot see: PowerShell 7 starts in about 0.44 s there, Windows PowerShell
//! in 0.31 s, git in 0.05 s, where bash at a Linux desk takes a millisecond
//! or two.

use std::collections::BTreeMap;
use std::path::Path;

use crate::Result;

/// The variable that names the directory the starts are written to:
/// `cineris_base::hook::STARTS`.
pub(crate) const VARIABLE: &str = "CINERIS_TESTKIT_STARTS";

/// What a line of the launcher becoming a program starts with:
/// `cineris_base::hook::LAUNCHED`.
const LAUNCHED: &str = "launcher>";

/// What a line of a start that asks a program its version starts with:
/// `cineris_base::hook::PROBED`.
const PROBED: &str = "probe>";

/// What kind of process a start was, by what it costs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Kind {
    /// PowerShell 7, `pwsh`.
    Pwsh,
    /// Windows PowerShell 5.1, `powershell`.
    WindowsPowerShell,
    /// A POSIX shell: `bash`, `sh`, `dash`, `zsh`, `ksh`.
    Posix,
    /// `cmd.exe`.
    Cmd,
    /// git.
    Git,
    /// The harness's binary, `cineris` or `cin`.
    Binary,
    /// Anything else: a test binary in a role, a helper, an editor.
    Other,
    /// A shell asked its version (`probe>NAME`), whatever the shell.
    Probe,
}

impl Kind {
    /// Every kind, in the order a report lists them.
    pub(crate) const ALL: [Kind; 8] = [
        Kind::Pwsh,
        Kind::WindowsPowerShell,
        Kind::Posix,
        Kind::Cmd,
        Kind::Git,
        Kind::Binary,
        Kind::Other,
        Kind::Probe,
    ];

    /// The kind of a program, by the name a start was written down with
    /// (its file name without an extension, in lower case).
    pub(crate) fn of(program: &str) -> Kind {
        if program.starts_with(PROBED) {
            return Kind::Probe;
        }
        match program {
            "pwsh" => Kind::Pwsh,
            "powershell" => Kind::WindowsPowerShell,
            "bash" | "sh" | "dash" | "zsh" | "ksh" => Kind::Posix,
            "cmd" => Kind::Cmd,
            "git" => Kind::Git,
            "cineris" | "cin" => Kind::Binary,
            _ => Kind::Other,
        }
    }

    /// How a report names the kind.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Kind::Pwsh => "pwsh",
            Kind::WindowsPowerShell => "powershell",
            Kind::Posix => "posix shell",
            Kind::Cmd => "cmd",
            Kind::Git => "git",
            Kind::Binary => "cineris",
            Kind::Other => "other",
            Kind::Probe => "version probe",
        }
    }
}

/// One test's starts, by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Counts(pub(crate) BTreeMap<Kind, usize>);

impl Counts {
    /// The starts of `kind`.
    pub(crate) fn of(&self, kind: Kind) -> usize {
        self.0.get(&kind).copied().unwrap_or(0)
    }

    /// Every start.
    pub(crate) fn total(&self) -> usize {
        self.0.values().sum()
    }

    pub(crate) fn add(&mut self, other: &Counts) {
        for (kind, count) in &other.0 {
            *self.0.entry(*kind).or_default() += count;
        }
    }

    /// `pwsh 6, git 27`: the kinds there are, in [`Kind::ALL`]'s order.
    pub(crate) fn brief(&self) -> String {
        Kind::ALL
            .iter()
            .filter(|kind| self.of(**kind) > 0)
            .map(|kind| format!("{} {}", kind.name(), self.of(*kind)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Every test's starts, by its binary and name as nextest's report names
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Starts {
    pub(crate) by_test: BTreeMap<(String, String), Counts>,
}

impl Starts {
    /// The starts of one test: none when it wrote nothing down.
    pub(crate) fn of(&self, binary: &str, name: &str) -> Counts {
        self.by_test
            .get(&(binary.to_owned(), name.to_owned()))
            .cloned()
            .unwrap_or_default()
    }

    /// Reads the lines the processes wrote. A line that is not three
    /// fields is passed over: a process killed half-way through its write.
    pub(crate) fn parse(lines: &str) -> Self {
        let mut starts = Starts::default();
        // The launcher's own start, counted as its own program's by whoever
        // started it (the harness's binary, or a test binary in that role),
        // is the program it became: `launcher>LAUNCHER>PROGRAM`. Only as
        // many are moved as the launcher was started: a command wrapped and
        // never started (a refusal after the wrap, a test of the wrap) is
        // no start.
        let mut launched: BTreeMap<(String, String, Kind, Kind), usize> = BTreeMap::new();
        for line in lines.lines() {
            let mut fields = line.split('\t');
            let (Some(binary), Some(name), Some(program), None) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if let Some((launcher, program)) = program
                .strip_prefix(LAUNCHED)
                .and_then(|rest| rest.split_once('>'))
            {
                *launched
                    .entry((
                        binary.to_owned(),
                        name.to_owned(),
                        Kind::of(launcher),
                        Kind::of(program),
                    ))
                    .or_default() += 1;
                continue;
            }
            *starts
                .by_test
                .entry((binary.to_owned(), name.to_owned()))
                .or_default()
                .0
                .entry(Kind::of(program))
                .or_default() += 1;
        }
        for ((binary, name, launcher, program), count) in launched {
            let Some(counts) = starts.by_test.get_mut(&(binary, name)) else {
                continue;
            };
            let started = counts.of(launcher);
            let moved = count.min(started);
            if moved == 0 {
                continue;
            }
            counts.0.insert(launcher, started - moved);
            *counts.0.entry(program).or_default() += moved;
        }
        starts
    }

    /// Reads every file in `directory`. A directory that is not there is
    /// no starts at all.
    pub(crate) fn read(directory: &Path) -> Result<Self> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Ok(Starts::default());
        };
        let mut all = String::new();
        for entry in entries {
            let path = entry
                .map_err(|error| format!("{}: {error}", directory.display()))?
                .path();
            let text = std::fs::read(&path)
                .map_err(|error| format!("could not read {}: {error}", path.display()))?;
            all.push_str(&String::from_utf8_lossy(&text));
            if !all.ends_with('\n') {
                all.push('\n');
            }
        }
        Ok(Self::parse(&all))
    }

    /// Whether anything was written down at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.by_test.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tests_starts_are_counted_by_kind() {
        let starts = Starts::parse(
            "cineris-core::it\ttrust::sys_a\tpwsh\n\
             cineris-core::it\ttrust::sys_a\tgit\n\
             cineris-core::it\ttrust::sys_a\tgit\n\
             cineris-core::it\ttrust::sys_b\tpowershell\n\
             cineris-cli::system\truns::sys_c\tcineris\n\
             cineris-cli::system\truns::sys_c\tcx\n\
             cineris-cli::system\truns::sys_c\tsystem-0123abcd\n\
             a line cut short\n",
        );
        let a = starts.of("cineris-core::it", "trust::sys_a");
        assert_eq!((a.of(Kind::Pwsh), a.of(Kind::Git), a.total()), (1, 2, 3));
        assert_eq!(a.brief(), "pwsh 1, git 2");
        let c = starts.of("cineris-cli::system", "runs::sys_c");
        assert_eq!((c.of(Kind::Binary), c.of(Kind::Other)), (2, 1));
        assert_eq!(starts.of("cineris-cli::system", "missing").total(), 0);
        assert_eq!(starts.by_test.len(), 3, "the short line is passed over");
    }

    #[test]
    fn the_launcher_becoming_a_shell_is_the_shells_start_and_not_the_binarys() {
        // The harness's binary as the launcher, beside a start of its own.
        let starts = Starts::parse(
            "cineris-cli::system\truns::sys_x\tcineris\n\
             cineris-cli::system\truns::sys_x\tlauncher>cineris>sh\n\
             cineris-cli::system\truns::sys_x\tcineris\n",
        );
        let x = starts.of("cineris-cli::system", "runs::sys_x");
        assert_eq!((x.of(Kind::Posix), x.of(Kind::Binary)), (1, 1));
        // A test binary in the launcher's role: its start is taken off the
        // other starts, not the binary's.
        let starts = Starts::parse(
            "cineris-core::it\tsandbox::sys_x\tit-0123abcd\n\
             cineris-core::it\tsandbox::sys_x\tlauncher>it-0123abcd>bash\n\
             cineris-core::it\tsandbox::sys_x\tcineris\n",
        );
        let x = starts.of("cineris-core::it", "sandbox::sys_x");
        assert_eq!(
            (x.of(Kind::Posix), x.of(Kind::Other), x.of(Kind::Binary)),
            (1, 0, 1)
        );
        // A command wrapped and never started is no start of anything.
        let starts = Starts::parse(
            "cineris-sandbox\tlaunch::tests::wrap\tlauncher>cineris>bash\n\
             cineris-sandbox\tlaunch::tests::wrap\tgit\n",
        );
        let w = starts.of("cineris-sandbox", "launch::tests::wrap");
        assert_eq!(
            (w.of(Kind::Posix), w.of(Kind::Binary), w.total()),
            (0, 0, 1)
        );
    }

    #[test]
    fn programs_are_told_apart_by_their_names() {
        for (program, kind) in [
            ("pwsh", Kind::Pwsh),
            ("powershell", Kind::WindowsPowerShell),
            ("bash", Kind::Posix),
            ("sh", Kind::Posix),
            ("cmd", Kind::Cmd),
            ("git", Kind::Git),
            ("cineris", Kind::Binary),
            ("cin", Kind::Binary),
            ("it-0590a6a4d850de8a", Kind::Other),
            ("probe>bash", Kind::Probe),
            ("probe>pwsh", Kind::Probe),
        ] {
            assert_eq!(Kind::of(program), kind, "{program}");
        }
    }

    #[test]
    fn a_directory_of_files_is_read_together_and_a_missing_one_is_empty() {
        let dir = std::env::temp_dir().join(format!("xtask-starts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1.starts"), "b\tt\tgit\n").unwrap();
        // A file whose last line has no end, beside another.
        std::fs::write(dir.join("2.starts"), "b\tt\tpwsh").unwrap();
        std::fs::write(dir.join("3.starts"), "b\tt\tgit\n").unwrap();
        let starts = Starts::read(&dir).unwrap();
        let t = starts.of("b", "t");
        assert_eq!((t.of(Kind::Git), t.of(Kind::Pwsh)), (2, 1));
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(Starts::read(&dir).unwrap().is_empty());
    }

    /// The texts `cineris-base` writes with are the ones read here: a name
    /// changed on one side only would count nothing, silently.
    #[test]
    fn the_variable_and_the_marks_are_cineris_bases() {
        let hook = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../crates/cineris-base/src/hook.rs"),
        )
        .unwrap();
        for line in [
            format!("pub const STARTS: &str = \"{VARIABLE}\";"),
            format!("pub const LAUNCHED: &str = \"{LAUNCHED}\";"),
            format!("pub const PROBED: &str = \"{PROBED}\";"),
        ] {
            assert!(
                hook.contains(&line),
                "cineris-base's hook.rs has no `{line}`"
            );
        }
    }
}
