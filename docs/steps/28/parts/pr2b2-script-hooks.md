# Step 28, its parts: script hooks

The second part of step 28's hooks ([answered.md](../answered.md), "The design, before building", points 5, 6 and 8): an extension's registered hooks, functions of its script, on the seam the first part built ([pr2b1-hooks.md](pr2b1-hooks.md)), beside the hook commands.

## What was built

- **Script hooks on the seam** (the core's `ext::script_hooks`, read by `ext::hook_points` beside the hook commands): each hook a loaded extension registered with `cineris.on`, on an event its manifest lists, named `<ext>/hook.<event>[.N]`, from when the extension is loaded until it is withdrawn; a project's only while the project is trusted and the extension pinned as it is ([12-extensions.md](../../../architecture/12-extensions.md), "As built in step 28: script hooks").
- **A run is a call of the extension's runtime** (`LoadedExtension::call_hook`, `Target::Hook`), one of its queue, its time from when it is asked (`cineris-ext`'s `CallContext::asked`): queued past its time it is withdrawn before it starts, running past it it is stopped. A run is a task the session keeps, under a token of its extension's, and its future stops it when dropped, polled or not.
- **In a hook, a question is a refusal** (`CallHost`'s `Phase::Hook`, `for_a_hook`): a host call the engine would ask about is refused, naming the rule that would allow it, so a hook runs only what the engine allows outright; `cineris.ask` and `cineris.fs.write` are refused, and a hook's program has no write root.
- **Its gates fail closed:** a changed, unreadable, removed or unloadable extension that stands approved keeps its `tool.before` and `prompt.submit` gates on their events, failing, from the registration approved; one never registered holds one on each gating event its manifest lists.
- **A session whose finding died starts no turn:** `turn/start` and `session/compact` are refused with `extensions_not_settled` ([11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md)).
- **The breaks:** the part's list (`step-28-scripthooks`) is put together in the step's, [mutations/step-28.breaks](../../../../mutations/step-28.breaks).

**Settled during the build:**

1. **A stopped turn's script hooks do not start,** the maintainers' rule for hook commands ([pr2b1-hooks.md](pr2b1-hooks.md), point 11) carried over; a hook's host call in a stopped turn is refused with the stop and is no stop of its own.
2. **A hook's time starts when it is asked** (`CallContext::asked`, an addition to the engine; a tool's call passes none and is unchanged). Point 8's "costs nothing in practice" no longer holds: a gating hook queued behind its extension's long tool can fail for the wait alone, and its gate then refuses the call; point 8 says so.
3. **No write root in a hook** (the maintainers, after the early review): `serve` gives the manifest's write roots only to a tool's call, pinned at `supervised` and at `read-only`; a hook that needs to write is a hook command with `write = true`.
4. **Gates never lapse** (the early review's merge-blocking finding, point 5): an extension's hooks leave their events only when the user turns it off, the project's trust goes, or its pin is taken away. Where no registration was ever known, a failing gate on each gating event the manifest lists (the maintainers). Each refusal names the extension, why, and the way out, written in one place (`script_hooks::way_out`) for the third pull request's `cineris ext` to change; the user is told once a session; the core's own operations are no hook points, so turning the extension off stays open, pinned by `a_changed_extensions_gates_hold_and_the_way_out_stays_open`.
5. **`session.start` and `session.end` script hooks** are built in this part, as the hook commands' are.
6. **A finding that dies fails the session's turns** (`extensions_not_settled`), rather than letting a first turn wait for ever or start without a gate.
7. **A hook's `cineris.state` is its extension's,** keyed by its source and name, as its tool's is (the independent review's "`state` keyed by name alone", answered for hooks too).
8. **`hook.` and `command.` names are the harness's:** no tool or command of an extension may take one.

**The engine's flaky test is settled** (its item, `an-engine-test-that-advances-its-clock-before-the-script-spins`, is deleted): the test moves its clock only once the module has registered its tool, just before the loop that never looks at the interrupt. It counts the engine thread's arrivals at the point `A_DEFINITION_IS_REGISTERED`, which a test can stop at and this one never holds. The item's recommendation, a host call just before the loop, could not be the signal, because no host call can be answered during a first evaluation.

## The early review

**The script hooks' permission path** (a fresh reviewer, of the first round): no high. A script hook could put no question to anyone and get none of its own host calls decided as a tool's. **Medium:** a gating hook vanished at the next turn when its extension changed on disk, became unreadable or failed to load (one byte appended to `main.js` dropped a `tool.before` guard), now held failing (point 4); and a hook's programs were given the manifest's write roots, unrecorded (point 3). **Lows,** each fixed with a test and a break: a hook's settled promises ran in the next tool call's phase; a call's leftover host calls could stop the runtime and end the extension's next call; the test of a hook that finds its turn stopped by another call; `asked` tested where the loop's own wait does not cover it; the close's `session.end` runs stopped together, and a runtime's start raced against the run's cancel; withdrawal stopping the runs going; the policy keeping the latest sixteen stopped turns, not one; the reserved names; a test that held one wording of two that fire at once; Windows' refusal of a program read as only reading.

## What was measured

| What | Measured | Where |
|---|---|---|
| With no extension, or none with hooks | Nothing more on a session's open; a point with no hook is passed as before | |

## What is left open, and where

- **`cineris ext disable`, `remove` and `add`** (the third pull request): the way out a held gate names becomes the command.
- **Older breaks whose lists no longer say how they are caught:** [breaks-whose-lists-no-longer-say-how-they-are-caught](../../../outstanding/breaks-whose-lists-no-longer-say-how-they-are-caught.md).
- **The policy's guards of a stop from a run,** three breaks of steps 12 and 26: [the-policys-guards-of-a-stop-from-a-run-have-no-test](../../../outstanding/the-policys-guards-of-a-stop-from-a-run-have-no-test.md).
