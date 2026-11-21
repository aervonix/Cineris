# Step 28, its parts: the hooks' seam and hook commands

The first part of step 28's hooks ([answered.md](../answered.md), "The design, before building", points 3 and 6): one seam in the loop, `cineris_agent::Hooks`, at the seven points, and hooks that are command lines, the user's own and a trusted project's. Script hooks, an extension's functions on the same seam, are the next part.

## What was built

- **The seam** (`cineris-agent`'s `hooks`, the round's `tool.before` and `tool.after`, the turn's `prompt.submit` and `turn.end`, the epochs' `compaction.before`).
  With no hook whose filter matches, every point is passed on one synchronous check, with no await. A hook only narrows (`deny` over `ask` over `allow`, and `allow` never lifts a question); a replacement of the arguments, from exactly one hook, is schema-checked, planned, decided again, held to the protected paths, and then always put to a person, allow once or deny; a failure is `deny` where a hook gates and adds nothing where it only adds; every text a hook gives is quoted under its name, escaped and cut to 8 KiB ([03-agent-loop.md](../../../architecture/03-agent-loop.md), "Hooks").
- **The protocol's additions:** `approval/request`'s `hooks` and `replaced`, `tool_result`'s `replaced`, the notice `prompt_returned`, the notice source `hook`, and the errors `prompt_denied` and `invalid_prompt` ([11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md)). The interactive application's dialog names each hook that asked and shows a replacement's two sets of arguments; a refused prompt comes back into the editor with why.
- **The configuration:** `[hooks.<name>]`, enforced, in a new trust class, *kept apart*: the user's layers merge, a project's hooks are read apart, wait for trust and are pinned (kind `hook`) by the hash of what the question shows ([12-extensions.md](../../../architecture/12-extensions.md), "As built in step 28: hook commands").
- **Hook commands** (`cineris_ext::hook`; the core's `ext::hooks`, `ext::hook_command` and `ext::hook_points`): started by `cineris.exec`'s own code, its lookup, Windows quoting, launcher, environment, runner and look at the protected paths, with the put-back security fix 3 gave it; each start the permission engine's decision, by `hook_may_run`; the event on standard input; the answer read strictly; the trust question's entries and `cineris permissions trust`.
- **The breaks:** the parts' lists (`step-28-hookseam`, `step-28-hookcmd`) are put together in the step's, [mutations/step-28.breaks](../../../../mutations/step-28.breaks).

**Settled during the build:**

1. **A hook's start** (the owner's answer of 2026-11-21, [answered.md](../answered.md), "The owner's answers"): a hook of the user's own configuration runs where the engine would ask, a project's never does, and none runs where a managed `ask` rule is among the reasons, all in `hook_may_run`. Point 6's sentence that a hook which would need a question has failed is amended to it.
2. **A replaced call is answered only by a person** (a `user` client), never by a supervisor's grant, and is denied where no user is attached (the maintainers, after the seam's early review).
3. **The seam fails closed:** `wants` is false only when no hook in any state would match, and a hook that cannot run is returned as one whose run fails; the core's snapshot is settled as the session opens.
4. **Hooks are bounded:** observers run at a call's first decision only, four runs of one at a time, under the session's token; a replacement over 256 KiB is refused; a returned prompt goes whole only to the client that sent it; `turn/start` takes text and image parts only.
5. **The look at protected paths is the shell's, reached through `cineris.exec`'s own start,** never a copy, so the put-back reaches hooks; a hook's protected change, on any event and from an observer, is told at once and holds back the next prompt.
6. **The environment** (the maintainers, as the design settled): a hook command is given the shell's environment, less the providers' keys and `shell.env_remove`, **tokens in it included**, as the project's checks are; the trust question says so of a project's hook before the yes, and `security.md` says it plainly.
7. **`{path}`** fills only files within the workspace; **an entry that uses it does not run for a call with no such file** and counts as no opinion, and a gating entry that uses it with no `tools` filter is warned of as the session opens (the maintainers' answer to `find {path} -delete`).
8. **`--with-extensions` stays,** meaning the project's extensions and hooks, and its help says so.
9. Hooks with a write root run one at a time, and at `read-only` a hook has none.

## The early reviews

**The seam's permission path** (a fresh reviewer, before the core implemented it): no high. Every path to a replacement's run goes through the question, the protected check comes before it on the plan that runs, a hook's `allow` never acts, and a stop stands. **Medium:** a supervisor's grant could answer a replaced call (settled point 2); `wants` and `matching` had to fail closed (point 3); hook names and context reached the harness's own words unescaped, so a context could forge a notice's lead-in (now quoted and escaped); observers and decisions again were unbounded (point 4); a client that does not read `replaced` saw a replaced call as the model's (the dialog, and `reasons` lines for any client). Lows, each fixed with a test: the interrupt and thrown-away paths, a note that said the user was asked when nobody was, caps, a hook's `ask` with nobody to answer, the steering message's wording.

**The command hooks' OS layer:** no high. `hook_may_run` matches every row of the owner's rule; the line decided is the line started; placeholders are whole and absolute; the Windows quoting and refused characters are `exec`'s; the pin covers everything shown. **Medium:** a protected path just changed counted as the harness's own until the next look, so a second change went unreported (true of the look before security fix 3; on its look the case is caught, held by a test and two breaks, and what the look could not put back is in [the-put-backs-remaining-edges](../../../outstanding/the-put-backs-remaining-edges.md)); a hook's stop outside a running turn stopped nothing (point 5); the pages described a narrower environment and sandbox than the design's (point 6). Lows, each fixed: escaping and caps of an answer's failure and `reason`, a present `null`, one outcome of the engine a start, `{tool}`'s shape, `{path}` outside the workspace (point 7), `read-only`'s write root, wording, the missing tests and breaks.

## What was measured

| What | Measured | Where |
|---|---|---|
| With no hook | One `wants` call at each point (a planned call, each call that ran, a prompt or steering message, a turn's end, a compaction), with no allocation and no await; every existing test unchanged | `cineris-agent::it hooks::with_no_hook_that_matches_every_point_is_passed_and_nothing_changes` |
| The tiers | [ci.md](../ci.md) once the pull request runs | |

## What is left open, and where

- **Script hooks** (an extension's functions on the seam), with `Ready`'s failed state, a changed extension's pinned registration kept for its gating hooks, the test that the policy's stop answers a hook's host call after a stop (which turns `a-stopped-host-call-leaves-the-turn-going` and `exec-protected-change-does-not-stop-the-turn` back to caught), hot-reload of hooks, and the engine's flaky test: the next part.
- **A path the look could not put back** and that changes again before the next look: [the-put-backs-remaining-edges](../../../outstanding/the-put-backs-remaining-edges.md), step 32.
- **Repository configuration that points git at a workspace file** (`include.path`, a relative filter program, `core.hooksPath`): as for extensions' writes, under security fix 3's watch of what git's configuration names.
