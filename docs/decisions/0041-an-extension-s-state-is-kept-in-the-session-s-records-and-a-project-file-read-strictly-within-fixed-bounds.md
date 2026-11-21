# 0041: An extension's state is kept in the session's records and a project file read strictly, within fixed bounds

- **Status:** Accepted by the maintainers during step 28's build.
- **Date:** 2026-11-21

## Context

`cineris.state` is an extension's own small store of JSON values, in two scopes: `session` and `project` ([architecture/12-extensions.md](../architecture/12-extensions.md), "Tier 4: script extensions"). The design settled before step 28's code said where each is kept: `session` in the session log's `ext` entries, so that a resume or a fork has the values as they were there, and `project` in a file in the state directory, with a version and a cap of 64 KiB an extension a scope (the maintainers' change to the design, [steps/28/answered.md](../steps/28/answered.md), point 2).

Building it raised what the design did not say: what a rewind does to a `session` value, what a fork copies, what is done with a project's file that this build did not write, and what bounds the session's log, since each change of a value is a line of it. [Decision 0040](0040-extensions-run-in-a-runtime-each-checked-by-their-manifest-and-then-the-permission-engine.md) is about the runtime and the order a host call is checked in, not this.

## Options

- **`session` values as entries of the conversation's tree, or as records.** An entry of the tree is on a branch: a rewind to before it takes it off, and a fork copies it with the branch. A record belongs to the session, not to a branch: a rewind leaves it, and a fork copies only the records it is told to. The session format has had `ext` as a kind of record from its first design ([architecture/07-sessions-and-context.md](../architecture/07-sessions-and-context.md), "The log").
- **A project's file this build did not write: set aside, or refused.** The registration cache is set aside, moved to `registered.json.set-aside` and begun again, because what it held can be had again by evaluating the approved extensions. An extension's values cannot be had again.
- **What bounds the log.** The 64 KiB cap bounds what an extension holds, not what it writes: a set appends a record even when it sets a key again, and the session log has no total limit of its own, only a line's 64 MiB. A cap on the sets in one call, a total for the session, or both.

## Decision

1. **`session` values are the session's `ext` records,** one a change, `{"ext": "user/tracker", "data": {"state": {"key": …, "value": …}}}`, a `null` value removing the key, read back in the order they were written the first time an extension of the session uses them. A resume has them as the session left them; a rewind leaves them; a fork copies the records written before the next entry of the conversation after its point (all of them when none came after it), so it has the values as they were there. A session with no log keeps them in memory.
2. **`project` values are one file an extension and project,** `projects/<project key>/extensions/<source>-<name>.json` in the state directory, `{"version": 1, "values": {…}}`, read strictly: a file this build did not write (a field it does not know, another version, bad JSON, a key it would not take, a `null` value, more than 1 MiB) **refuses the call, with the reason, and is left as it is**, never set aside and never written over. It is changed under its lock, from before the read until after the write, and written whole, owner-only and atomically.
3. **Both are keyed by where the extension comes from and its name** (`user/tracker`, `project/tracker`).
4. **64 KiB of values an extension a scope,** counted as the compact JSON of them all, and **a key of 1 to 256 bytes.** A set that would take the values past it, and grow them, is refused, and nothing is kept; one that does not grow them is always taken.
5. **At most a hundred `cineris.state.set` calls in one call of the extension.**
6. **16 MiB of `session` records an extension a session,** each record counted as its `ext` and `data` in compact JSON and 128 bytes for the rest of its line, the same when it is written and when a resume reads it back, and counted from the records already in the log when the values are first read, so a resume does not start it again. Past it a set, a removal included, is refused with the reason, and nothing is written; a new session starts again.

## Why

1. **Refusing a project's file, rather than setting it aside, loses nothing.** A file set aside is one the next write starts again from nothing: the extension would carry on without its values and without being told. Refused, the file stays for the user to put right or delete, as the refusal says, and a newer build's file is never written over by an older build. It is [decision 0028](0028-enforced-values-fail-closed.md)'s rule: a file a build cannot read in full is not read in part.
2. **A hundred sets a call** (the first decision line). Each `session` set is a line of the log of up to 64 KiB of values; a hundred bound one call of the extension to about 6.4 MiB, as the counts of notices, questions and lines of the log bound a call's other host calls (a hundred lines of the log, ten notices, five questions).
3. **16 MiB a session** (the second decision line). The 64 KiB and the hundred a call bound a call; nothing else bounds a session, since a set appends a record even when it sets a key again, and the log has no total limit of its own (a line's 64 MiB only). 16 MiB is the session store's other bound on one thing it holds (the most a file may hold for a checkpoint to keep its content), and of the order of the 10,000-entry, 12.1 MB session that the budget for opening a session is measured on ([performance-budgets.md](../performance-budgets.md)). It is counted the same when a record is written and when a resume reads it back, and from the records at the first read, so neither a resume nor a fork resets it.
4. **Records are the format's own kind for state outside the conversation.** `ext` has been a record since the session format was designed, beside the base state and the leases, which belong to the session and not to a branch. Keeping the values there needs no new kind of entry, and a fork copies them as they were at its point, which is what the design asked of a fork.
5. **64 KiB and 256 bytes are the design's** (point 2, settled on 2026-11-20): a small store, read whole at its first use, each of whose records holds at most one value of it.

## What is given up

- **A rewind does not take back a value** that a turn it undoes set: the values go on as the session left them, and an extension cannot tell from them that those turns were undone.
- **A session that reaches the 16 MiB cannot change its values,** not even remove one, until a new session.
- **A project's file broken by hand stops the extension's `project` calls** until it is put right or deleted.
- **Opening a session whose log holds the whole 16 MiB** in the smallest records reads about 88,000 more lines: about 100 ms by the rate step 8 measured, against the 50 ms budget for a session of 10,000 entries. Reaching it takes about 880 calls of the extension, each setting a hundred values.

## Revisit when

- Extensions need values that follow the branch, so that a rewind takes them back: they would be entries of the tree, of a kind of their own.
- The session log gains a total bound of its own: the 16 MiB would then be measured against it.
- An extension in real use reaches the hundred sets in one call or the 16 MiB in one session.
