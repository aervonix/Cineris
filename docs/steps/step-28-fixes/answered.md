# Answered in the fixes to step 28, on 2026-11-27

Work outside the numbered steps. An independent review of step 28 (extensions I), on `main` at `5eda6204`, found 2 high, 8 medium, 3 low-to-medium and 6 low findings and a set of rules no test held. The maintainers chose which to fix before steps 29 (self-written extensions) and 30 (the MCP client) build on the code. The fixes come in three pull requests, each with a record of its own in [parts/](parts/):

1. **Variation selectors** ([parts/selectors.md](parts/selectors.md)): finding 6, first, since step 30's MCP descriptions come through `cineris_base::one_line`.
2. **The guards and the socket** ([parts/guards.md](parts/guards.md)): findings 1, 2, 3, 7, 8 and 17, and the tests of guard B's store-moving variables (19).
3. **The rest:** the gates and hooks (4, 5, 14), the approvals' words and the commands' front matter (11, 12, 15, 16, 18), the engine and `fetch` (9, 10, 13, and 19's untested rules), the build without the engine (19), and the lower-confidence notes (20).

The review's report and its tests are the maintainers'; each finding's test here is the review's test made a real one, or a better one, and fails on `main` at `5eda6204`.

## The findings and what became of each

| # | Finding | Outcome |
|---|---|---|
| 1 | `cineris project`'s changes have no guard B | Fixed, and `auth login`/`logout` and `stats reset` with them, with tests and breaks ([parts/guards.md](parts/guards.md)) |
| 2 | Windows: B's walk for a service's start falls open at its bound | Fixed, and its twin in the line of parents; shown on a Windows runner ([parts/guards.md](parts/guards.md)) |
| 3 | The RPC socket refuses only its own server's commands | Fixed: a peer under any harness is refused, the check passed in by `serve`; shown on both systems ([parts/guards.md](parts/guards.md)) |
| 6 | A run of variation selectors survives `one_line` and `plain` into sections 6 and 7 | Fixed, with tests and breaks ([parts/selectors.md](parts/selectors.md)) |
| 7 | Linux: a harness that is process 1 is never seen | Fixed, and the stores' twin; shown on Linux 22.04 ([parts/guards.md](parts/guards.md)) |
| 8 | A pid namespace of the command's own hides every parent from guard B (Linux) | Not built: settled by the maintainers on the evidence below; stated in 06, security.md §3 and the guards' item ([parts/guards.md](parts/guards.md)) |
| 17 | Windows: `ProgramData` moves the managed configuration | Fixed, in A's and B's lists; shown on a Windows runner ([parts/guards.md](parts/guards.md)) |
| 19 | B's store-moving variables have no test | Tested, each variable on each system ([parts/guards.md](parts/guards.md)) |

## Settled before building

- **Finding 8: a pid or user namespace is not read as "not known".** A rule that refused under a pid namespace other than the system's first would refuse every person's own approvals under WSL 2, whose processes are all in such a namespace (checked on the reference machine: `/proc/self/ns/pid` is not the initial namespace's), and in every Docker or development container; one that refused under a user namespace other than the first would refuse every toolbox and distrobox user (rootless containers that share the system's pids). Nor can a parent of 0 tell the attack from a legitimate top: a namespace's own first process (WSL's `/init`, `bwrap`, a container's entry point) has a parent of 0 just as `unshare -Urpf` gives one. Ubuntu's restriction of unprivileged user namespaces does not stop the attack where `bwrap` is installed, since it has a profile of its own (Flatpak installs it): on the second machine `bwrap --unshare-user --unshare-pid` ran where `unshare -Ur` was refused, and a terminal or editor run under Flatpak is that shape for a person's own Cineris. So the gap is stated (06, [security.md](../../security.md) §3 and the guards' item), approvals signed with a key that needs a person close it by construction, and step 32's channel the kernel names closes the rest.
