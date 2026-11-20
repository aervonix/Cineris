# 0040: Extensions run in a runtime each, checked by their manifest and then the permission engine

- **Status:** Accepted by the maintainers, 2026-11-20 (step 28)
- **Date:** 2026-11-20

## Context

[Decision 0005](0005-extension-mechanism.md) chose JavaScript on an embedded QuickJS engine, with Lua as the fallback, and left the shape of what runs it to step 28: the format of the manifest users keep, how a script's request reaches the session, which decisions the permission engine makes about it, how a script's work is held to its limits, and how an extension's tools are scheduled. Steps 29 (self-written extensions) and 30 (the MCP client) build on all of it. The step's design and threat note settled each point with the maintainers before code, and an independent review of the note changed several ([steps/28/answered.md](../steps/28/answered.md)).

## Options

- **Where the manifest is checked.** In each host function's implementation on the session's side, or once, before the session's side is asked. Once, in code that every call passes, makes "the manifest first" a property of the code rather than of each implementation.
- **What a host call's question may keep.** The engine's ordinary choices ("for the session", "in this project"), or "allow once" alone. A rule kept from a host call's question reaches past the extension: a program allowed the network names no host, so its rule would be `network *` for everything after it.
- **How runtimes are threaded.** One thread for every runtime, as the architecture page first said, or one each. With one thread, a script that computes until its time is up holds up every other extension's hook.
- **How a runtime runs calls.** Several at once, interleaved at their awaits, or one at a time. The engine's interrupt check belongs to the runtime and cannot tell which call a piece of work is for.
- **How an extension's tools are scheduled.** As their definitions say, or always alone. A tool that declares itself read-only and makes a link would run beside a later call that was decided before the link was there.

## Decision

1. **The manifest, `extension.toml`, is `api = 1`, read strictly:** an unknown key, a wrong type or another `api` refuses the extension. Its capabilities are `exec` (bare names or absolute paths), `exec_network`, `read` and `write` (workspace globs), `net` (hosts) and `hooks` (the events a script may hook, shown in words when approved), and `[limits]` within hard caps.
2. **An extension's folder is read whole into a snapshot** whose hash is what is pinned; the engine evaluates only a snapshot with the pinned hash, and serves its imports from it. A link anywhere in the folder refuses it.
3. **Every host call meets the manifest first, in `cineris_ext::host::serve`,** and only then the session's side, which has it decided by the permission engine as a call of the extension's tool, with the built-in tools' default and an `ext` effect beside its own. A question it needs offers only "allow once" and "deny", and no answer to it keeps a rule.
4. **One runtime an extension, on a thread of its own, running one call at a time;** a call's time starts when it starts running. A limit tears that runtime down and nothing else; a thread that does not come back within a grace after its deadline is abandoned.
5. **An extension's tool always runs alone,** whatever its definition says.

## Why

1. Each point narrows what an extension can make the harness do, or makes a failure close rather than open, and none costs a feature a user needs.
2. The order "manifest, then policy, then sandbox" is the one the user reads at approval; holding it in one function keeps it true however many host functions are added.
3. Steps 29 and 30 inherit a contract (`Host`, `serve`, the pins) rather than conventions.

## What is given up

- **Convenience at questions.** Each host call that needs an answer is asked about once; a user who wants it to stop asking writes a rule.
- **Parallelism inside one extension.** A hook of an extension waits for that extension's running tool. The tool runs alone in any case, so the call the hook is about cannot start before it.
- **A thread per extension in use,** each with its runtime's memory (32 MiB by default).

## Revisit when

- A host call's question needs a narrower kept answer: a rule that names the extension as well as the effect would need the permission engine's rules to change, which step 28 may not do.
- Extensions with tools that must run beside other calls are wanted, with a way to show the harness they make no link.
