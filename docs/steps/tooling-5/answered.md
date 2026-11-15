# Answered in the fifth tooling step, on 2026-11-15

This step is not a numbered step of the build plan: it built the fifth round of developer tooling, from what wave 8's four build sessions asked for, and carried out one decision of the owner's on the default theme's colours. Its records are here, in a folder named for the job.

## The coordinator's answers and additions

| Question or addition | Answer |
|---|---|
| Measuring on the runners before the step branch's first push. | A scratch branch, `worktree-step-02-tooling-5-try`, for measurement only: a `try` input for the test kit's profile (an allow-list, set through `env:`), the profile written as a file a test, and three throwaway probes, each deleted from it after its run. Every number quoted from it names its commit, whose only difference from `main` is the measurement change. Nothing of the probes is in the step branch. |
| How to read item 1's target, given that `main`'s tier 2 is 27 s on a fast Windows runner and 38 s on a slow one. | About a quarter of tier 2 cut, shown as a share of the tier in the A/B, so that a slow runner (38 s) lands near 29 s: the room is for the slow runners CI draws and for steps 16 and 18. |
| Owner-only files on Windows, if they were the cost of a session's open. | A change to `cortex_base::private` is security-sensitive and is asked about first. The probes showed it is not the cost: no product change was made there. |
| The lease's forced flush at a session's birth. | Not changed: it is behaviour around an enforced value. It is in [performance-budgets.md](../../performance-budgets.md) and an open item. |
| Running the system tests after tiers 1 and 2. | An experiment on the scratch branch only, with the process group at 3 and at 4, against the job as it is, on both OSes. Adopting it is the owner's call, because it changes how the tier budgets are measured. |
| The colour decision's reach: code's strings and numbers, and a changed line drawn in its colour where there is no tint. | Built, behind one flag of the palette (`warm_bold_code_and_lines`), while the owner is asked whether those go bold too. |

## The owner's colour decision

Decision 0033 is amended ("Amendment, 2026-11-15"); no new decision number. On a light background, text in red, green or yellow is bold; on a dark one, and when none is reported, keywords in code are bright blue; muted stays as it is.

| Colour | One Half Light (`#FAFAFA`) | Campbell (`#0C0C0C`) |
|---|---|---|
| Red, ANSI 1 | 3.5 to 1: now bold | 3.2 (bright red, 5.1, is used) |
| Green, ANSI 2 | 3.1 to 1: now bold | (bright green is used) |
| Yellow, ANSI 3 | 3.1 to 1: now bold | (bright yellow is used) |
| Keywords | magenta, 5.9 to 1, unchanged | bright blue, 5.0 to 1, was bright magenta, 3.2 |
| Muted, bright black | | 4.3 to 1, unchanged, by the owner's choice |

The reference screens were rewritten by `cargo xtask tui-screens --write`, and every difference was of style alone: 728 cells of 18 screens.

| Screen | Cells changed |
|---|---|
| 1-dark, 6-dark, 7a-dark, 7c-dark | 4 each (keywords) |
| 8a-dark, 8b-dark | 18 each (keywords) |
| 1-light | 28 |
| 1c-light | 12 |
| 1d-light | 6 |
| 2a-light | 1 |
| 2b-light | 21 |
| 4-light | 25 |
| 5-light | 67 |
| 6-light | 28 |
| 7a-light, 7c-light | 164 each |
| 8a-light | 62 |
| 8b-light | 68 |

The view snapshots that changed: `blocks`, `header-and-hint`, `round`, `rounded` and `theme`.
