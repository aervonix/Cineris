# Answered in CI budgets II, on 2026-11-26

A fix outside the numbered steps, given by the maintainers: on a slow hosted Windows runner `main`'s own tests were close to failing pull requests that did not cause it. On a runner at 8 ms a process start `main` at `42c90dcc` read Windows' integration tier at 55.2 s of its 40 (a failure past 60) and the system tier at 91.6 s of its 90 (`try` 45695681459), and on a usual runner (6 ms) `main`'s own CI read the integration tier at 43.8 s (run 45721643017). This record says where a Windows session's time goes, what was changed at its cause, and what that did to the tiers and to the test job. The CI times are in [ci.md](ci.md).

## Where a Windows session's time goes

**How it was measured.** Timing points along the core's path (a session's open, its turns, the log's birth, the rig's world), on an experiment branch that is not merged, read by an ignored test that runs a child process forty times over, as every test of CI is a process of its own: each child times a world, a core, a session's open, two turns, the close and the drops, then the same in a second world of the same process, then a second session of that world's core, and says each stretch between two points. The table gives medians of the forty, on a hosted runner of each system, in a release build (`try` 45725495734; the same run's CI build is `try` 45725475934). The parent test, with the points, was also run on Linux under `strace -f` with each point marked by a failing look at `/zz-lap/<name>`, which counts the system calls of each stretch. The cost of each call on its own was measured by a throwaway test that times each one 400 times in a directory as deep as a core test's workspace (`try` 45726487886, release). Everything compared below is one system against the other, each from its own runner; a claim that a change saved time rests on a one-runner A/B, in the next sections.

| Part, a core's second session (release) | Windows | Linux | Calls on Linux (`strace`) |
|---|---|---|---|
| **The whole open** | **8.91 ms** | **0.98 ms** | 302 `statx` and `readlink` |
| The configuration loaded for the trust look (`LiveConfig::start`: the files' stamps, then the files read, parsed and merged) | 2.33 ms | 0.17 ms | 74 (six walks of the workspace) |
| The rest of the trust look (a load as trusted, the project's extensions folder, `.git`) | 1.34 ms | 0.12 ms | 44 (three walks) |
| The permission engine's parts (`engine_parts`: the protected folders, the sandbox's grant, the write roots, each folder resolved) | 2.18 ms | 0.16 ms | 106 (twelve `resolve_dir`) |
| The workspace and the home folder placed, for the look and again for the assembly | 0.84 ms | 0.06 ms | 42 (four walks) |
| The session's private temporary folder (1.79 ms for a process's first) | 0.38 ms | 0.04 ms | 9 |
| The tools put together (no file is touched) | 0.73 ms | 0.21 ms | 25 `mprotect` |
| The rest | 1.11 ms | 0.22 ms | |
| **The first turn** | **8.59 ms** | **2.65 ms** | 293 |
| Its look at the configuration's files (`at_turn_start`, every turn) | 0.98 ms | 0.05 ms | 29 |
| The log's birth (its private folder, the lock, the owner-only atomic write, the session index) | 3.04 ms | 1.02 ms | 57 |
| A second turn | 3.35 ms | 1.00 ms | 145 |
| A world made, and dropped (the core rig's directories, removed when the test ends) | 1.26 + 3.20 ms | 0.23 + 1.34 ms | |

**The cause is the path resolver's walk.** A session's open walks 29 paths ([cineris-base's `PathResolver`](../../../crates/cineris-base/src/path/resolve.rs)): the workspace and the home folder in each place, each project file of each load of the configuration, and each folder the engine's parts resolve. A walk looks at every name of a path from the drive's root, without following it, so that no link is followed that the rules have not seen ([security.md](../../security.md), section 7), and then asks the system for the path's own spelling (`canonicalize`). On a hosted runner a test's workspace is nine names deep (`D:\a\_temp\tmp\cineris-core-it-N\<test>\work\app`), and each name was a `symlink_metadata`, which on Windows opens a handle, reads two of the file's records and closes it. Timed in place, on a Windows runner in a release build (`try` 45727050579), the 29 walks of an open took **2.67 ms in their names and 1.02 ms in their `canonicalize`: 3.69 ms of an open of 5.97 ms** on that runner, against about 0.2 ms on Linux. The configuration's loads, the trust look, the engine's parts and each turn's look at the files are each mostly walks; the rest of the difference is CPU (the tools, about 2.5 times slower there) and private files (below).

| One call, in a release build | Windows runner | Linux runner |
|---|---|---|
| `symlink_metadata` of a directory | 9.8 µs | 1.46 µs |
| `GetFileAttributesW` of a directory, which opens nothing | 3.1 µs | |
| `canonicalize` of the workspace / of a file in it | 26.1 / 30.5 µs | 4.5 / 6.7 µs |
| A resolver made for the workspace (a walk of nine names on Windows, six on Linux) | 114.8 µs | 14.6 µs |
| A project file resolved through it | 137.2 µs | 19.2 µs |
| A file's 20 bytes read | 18.3 µs | 5.4 µs |
| A private directory made (`private::create_dir_all`, one level) | 79.6 µs | 25.9 µs |
| An owner-only atomic write (`write_atomic`, no flush) | 167.6 µs | 56.4 µs |
| A small tree like a world's made and removed (6 folders, 2 files) | 1.29 ms | 0.46 ms |

**What else was checked, and is not the cause.** The candidates the maintainers named, each against the numbers above:

- **Thread or runtime start-up per session:** a session's loop is a task on the core's runtime; the open starts no thread. The store's writer is a thread of the session's log, started at the first turn (`birth`), not in the open.
- **The state directory's locks:** none is taken in the open; the log's lock is taken at its birth, in the first turn (0.14 ms of the birth on Windows).
- **The registry and the environment:** the open reads neither; the environment was read when the core started.
- **The extensions' finding with no extension:** off the path that opens the session (step 28), and step 28 measured it at 4.6 to 10.4 µs on the reference machine.
- **Defender scanning a new file:** the open writes no file; it makes one folder, the session's temporary one (0.38 ms, 1.79 ms for a process's first, which includes the first look-up of the user's security identifier). The first turn writes the log and the index, the birth's 3.04 ms on Windows against 1.02 ms on Linux, which is what an owner-only file costs there (the table above).
- **A process:** the open starts none (the rig hands its core a shell; the product asks its version off the open path).
