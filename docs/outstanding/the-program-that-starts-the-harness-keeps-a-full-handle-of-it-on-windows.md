On Windows the program that starts the harness keeps the full handle `CreateProcess` gave it, which the harness's own access list does not take back, so that program, or a command that copies the handle out of it, can read and write the harness's memory.

- Area: cineris-sandbox
- Owner: step 32 (hardening pass), the maintainers

| Item | Who | When |
|---|---|---|
| **What is open.** Since the signed approvals step the harness closes its own process to the user's other programs (`cineris_sandbox::harden`; [security.md](../security.md), section 3): on Windows an access list on the process and its threads leaves the user only `SYNCHRONIZE`, `PROCESS_QUERY_LIMITED_INFORMATION` and `PROCESS_TERMINATE`. A handle opened before the list was set keeps its rights: above all the one the starting program holds from `CreateProcess` (a shell, a terminal), which a command of the user's can copy out of that program. Closing it needs the harness to be started by a program that keeps no full handle of it: a first process that starts the real harness with its lists given at creation, and keeps only a limited handle, at the cost of a process start on every run. **Recommendation (the maintainers, 2026-11-27):** measure the relaunch's cost on a Windows runner (`try … startup -f profile=dist -f against=main`); take it if it costs about 2 ms of start-up or less, and otherwise leave it as stated for step 32. | Step 32 (hardening pass), the maintainers | Before the first release |
