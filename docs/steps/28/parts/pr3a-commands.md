# Step 28, its parts: custom commands

The first part of step 28's third pull request ([answered.md](../answered.md), "The design, before building", point 10, item 3): custom commands from files and from extensions' scripts, `commands/list` beside the lane, a prompt that names a command in the lane, expanded before `prompt.submit`, and the interface's menu ([12-extensions.md](../../../architecture/12-extensions.md), "As built in step 28: custom commands").

## What was built

- **Commands from files** (`cineris_ext::commands`): `<config dir>/commands/*.md` and `<workspace>/.cineris/commands/*.md`, read each time they are listed or used, strictly, within the skills' bounds; `{{name}}`, `{{name | "default"}}` and `{{args}}` put in once.
- **Commands from extensions' scripts,** `<extension>:<command>`, run as the extension's `Target::Command`, their text sent under a line naming its author.
- **The protocol's additions:** `commands/list`; `turn/start`'s `command`; `user_message`'s, `prompt_returned`'s and `turn/interrupt`'s result's command; the codes `unknown_command`, `command_arguments`, `command_refused` and `command_failed` (of kind `interrupted` for one stopped) ([11-rpc-protocol.md](../../../architecture/11-rpc-protocol.md)).
- **The core expands a command in the lane** (the core's `commands`), once the session's extensions are settled, before `prompt.submit`; an interrupt stops a command queued or running, through the lane's one exception (`Dispatcher::start_with`'s `early` hook, `Core::early`).
- **The interface:** commands in the `/` menu and `/help`, a project's labelled, the commands not used said once and listed; a command's prompt under a line that names it; a prompt given back as `/name args`. Headless mode sends `/name …` as a command.
- **The breaks:** the part's list, `step-28-commands`, 90 breaks, put together in the step's ([mutations/step-28.breaks](../../../../mutations/step-28.breaks)).

**Settled during the build:**

1. **The user's command wins over a project's of the same name** (the maintainers, with the early reviewer's agreement): a command's body is sent as the user's own prompt, so a repository could otherwise make the user's `/review` send text it chose; the page said the reverse, and is changed. Every project command is labelled; none is used when the user's folder cannot be read whole; a built-in wins, ignoring case.
2. **A command's settings profile is judged and not sent** (the maintainers): no mechanism applies a profile for one turn, and sending the command without it could run it wider than its author meant ([a-commands-settings-profile-for-its-turn](../../../outstanding/a-commands-settings-profile-for-its-turn.md)). The judgment refuses more than the maintainers' list: `allow` rules, `verify.commands`, MCP servers, raised limits, a faster speed, `shell.*`, `compat.*`, `read_skill_folders`, `[extensions]` and `sandbox.enabled`.
3. **The command's origin is carried through the loop's data** (`SessionHandle::prompt_from`, `Prompt`, `Pending::Steer`, to `UserMessageItem.command`), as the first hooks part's ticket was (the maintainers): no branch of the loop reads it. It is the step's one change to the loop outside its hook points, a change of data, not of behaviour.
4. **An interrupt stops a running script command** through its own token (the maintainers), from any client that may drive the session, queued or running, its questions withdrawn and its programs killed at a second interrupt; a stopped command starts no turn and records nothing. Its questions name the run, `cmd_<n>`, never a turn.
5. **A prompt given back carries its command** (the maintainers), so the editor gets `/name args`, not a project's text as if the user had typed it.
6. **A command has 30 s from when it is asked, at most 120 s,** whatever the manifest's `time` (the maintainers, after the early review): it runs while the user waits at the prompt, inside `turn/start`'s step.
7. **A command's programs have no write root,** as a hook's (the maintainers, after the early review): it runs outside any round, so nothing they wrote would be recorded. This reverses the second hooks part's point 13, which left commands' write roots to this part.
8. **A command does not run while a stop waits, a gate is held on `prompt.submit`, or the running turn is stopped,** and a stop of the running turn made while its script runs refuses its later host calls (the maintainers, after the re-check).
9. **A typed `/name` waits at most 3 s for the first command list, on the application's clock,** then goes as a command, never as text, when the list failed or did not come (the maintainers, after the re-check); any `/a:b` goes as a command at once.
10. **Linked command files stay refused,** as skills' and extensions' links are.

## The reviews

**The early review of the prompt path** (a fresh reviewer, of the first round): no high; it agreed that the user's command wins, asking that the user's names fail closed when their folder is not read whole, that case be folded against built-ins, and that a project's command be labelled everywhere. **Medium:** a refused command (one naming a profile, a broken file) was sent to the model as plain text, since the front ends ignored `not_used`; a command's expanded text came back into the editor as the user's own; a script command could hold the client and the session for its time and a wait behind its tool; a script command's programs wrote outside any round, its stops went to turn 0, and it ran before the gates that would refuse its prompt. Each fixed, with the maintainers' two calls (points 6 and 7).

**The narrow re-check** (of those fixes): the give-back and the time fixed, the turn-0 stops gone, the lane's exception sound. **Medium:** an interrupt missed a command whose `turn/start` was still queued in the lane (now marked as it is sent). M1's remainders (a `/name` typed before the first list, an unloaded extension's `/a:b`) and three lows fixed. **Left as they are,** each a narrow window or a gap of display: the instant between the last look and the prompt's hand-off, which another client's interrupt could miss; a command whose question is open when the turn stops, not looked at again after the answer; another client's interrupt giving back a command without a word; the runtime's start not raced against the cancel; and the rewind picker and the export showing a command's text ([a-commands-text-in-the-rewind-picker-and-the-export](../../../outstanding/a-commands-text-in-the-rewind-picker-and-the-export.md)).

**A flaky test of the second hooks part's** (`extensions_hooks::an_extensions_hooks_are_on_their_events_and_the_loop_reads_them`, its part `a_hooks_time_at_its_point_runs_from_when_it_was_asked`) failed on the records branch under load, stressed alone 60 times with two cores loaded, and passed every time on this part's tree, whose command time from when it is asked changed the path it waits on.

## What was measured

| What | Measured | Where |
|---|---|---|
| Windows' tier 2, both trees on one runner (medians of 5) | `cineris-core` 15.9 s against 15.6 before the last round's fixes, +0.2 s; tiers 1 and 3 +0.0 s; the new tests 0.02 to 0.06 s each | `try` 45737378589 |
| A session with no command | Nothing read until commands are listed or one is sent; the open path untouched | |

## What is left open, and where

- **A command's settings profile for its turn:** [a-commands-settings-profile-for-its-turn](../../../outstanding/a-commands-settings-profile-for-its-turn.md).
- **A command's text in the rewind picker and the export:** [a-commands-text-in-the-rewind-picker-and-the-export](../../../outstanding/a-commands-text-in-the-rewind-picker-and-the-export.md), step 32.
- The management of extensions, the examples, conformance and the step's close: the second part of the third pull request.
