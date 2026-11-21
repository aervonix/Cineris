# 0042: A sandbox.write entry that holds the harness's own files is left out, and the sandbox stays on

- **Status:** Accepted by the owner on 2026-11-21, in the third security fix. Supersedes the paragraph "A write root that holds the harness's own files is not confined" of [decision 0032](0032-what-the-linux-sandbox-needs.md), for the entries of `sandbox.write`; for the workspace and its temporary folder that paragraph stands.
- **Date:** 2026-11-21

## Context

Landlock cannot keep a directory inside a write root from being written. So since step 26, where a write root holds the harness's own files the session reports the sandbox off (`NoSandbox::HarnessFilesWritable`) and `autonomous` is refused. Those files are the configuration, state or data directory, the user's configuration file (as named or where a link leads), or the program started as the launcher. Otherwise a confined command could change what it is allowed, or put a program that confines nothing where the next launcher is started from ([0032](0032-what-the-linux-sandbox-needs.md)).

A write root is the workspace, its temporary folder, or an entry of `sandbox.write`. A trusted project's own entry counts, since `sandbox.write` only widens. Step 28's review found what that allows: a project's file, once trusted, could switch the Linux sandbox off for its sessions by naming `~/.cargo` with the harness in `~/.cargo/bin`. That turned it off for the shell's commands and for its extensions' programs alike. Step 28 made the trust question warn of it; the fix was left as an open item, `a-projects-write-root-holding-the-harness-files-turns-the-sandbox-off`, which this decision settles ([steps/security-fix-3/answered.md](../steps/security-fix-3/answered.md)).

## Options

| Option | Assessment |
|---|---|
| Keep the session unconfined, as 0032 says | A file of the repository's, trusted once, takes the sandbox away from every later session of the project, for an entry that adds one folder. |
| Refuse the session | No session at all for a configuration that only widened a root. A user with `~/.cargo` among their own entries could not work there. |
| **Leave the entry out, keep enforcing, and say so** | The entry is the only part that cannot be granted, and the rest of the grant stands. A command that needed to write there fails inside the sandbox, as any write outside the roots does, and the notice says which folder to name instead. |
| Leave out a project's entry, keep 0032 for the user's own | Two rules for one setting. A user's entry means "let commands write there", not "turn the sandbox off". |

## Decision

**An entry of `sandbox.write`, the project's or the user's own, that holds the harness's own files is left out of the sandbox's grant.** It is judged on its own, as a session's start judges its roots: it holds the configuration, state or data directory, or lies inside one, or holds the user's configuration file (as named or where a link leads) or the program started as the launcher. The session keeps the sandbox enforcing, and is told, naming the entry under `~`: "`sandbox.write` names `~/.cargo`, which holds Cineris's own files (its configuration, its state or data, or its program): a command allowed to write there could change what it is allowed. It is left out, so commands do not write there, and the sandbox stays on. To let commands write in a folder there, name one that holds none of them instead." (`assemble::left_out_writes`, `assemble::left_out_notice`).

- **A reload** that adds such an entry leaves it out the same way and says so, and applies the rest. Before, it was refused whole (`harness_files_writable`).
- **The trust question,** and `cineris permissions trust`, say of such an entry of a project's that it "is left out, so commands do not write there, and its sessions keep the sandbox on" (`assemble::harness_write_warnings`).
- **The workspace and its temporary folder cannot be left out.** Where either holds the harness's files, the session reports the sandbox off, as 0032 says.
- **An extension's write root** that holds the harness's files is still refused: no program of it runs.
- **Only where the sandbox would enforce.** Elsewhere (Windows, no Landlock, the sandbox turned off) nothing is confined for an entry to undo, so nothing is left out or said, as before.

## Why

- **The entry is the part that cannot be confined, and only it.** The rest of the grant, the network setting and the protected paths are as safe without it as with it.
- **A project's file must not be able to switch confinement off.** Trust is given for what the question showed: a write root, not the sandbox.
- **One rule for the user's entry and the project's.** Leaving the user's entry out keeps more confined than turning the sandbox off did, and the notice names the narrower folder to write instead.

## What is given up

- **A command that needed the entry fails inside the sandbox.** A `cargo build` that writes `~/.cargo/registry` with `~/.cargo` named fails there until `~/.cargo/registry` is named instead. That is the same failure as for any folder not granted, and the notice says what to name.
- **The roots a front end is told of** (`SessionInfo::write_roots`) still list the entry. A front end keeps the programs it runs out of them, and keeping out of one more folder only narrows.

## Revisit when

- A backend can protect a directory inside a write root (the namespace backend, or a Landlock that can deny inside an allowed tree). Such an entry could then be granted with the harness's files held back inside it.
