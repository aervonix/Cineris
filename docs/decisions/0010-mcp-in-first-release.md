# 0010: MCP client scope in the first release

- **Status:** Accepted by the owner on 2026-11-12, as written. It is no longer provisional.
- **Date:** 2026-11-12

## Context

MCP is how most third-party tools reach coding agents. The vision requires that people switching from another agent keep their MCP servers, and that existing MCP configurations are read.

The protocol changed fundamentally in its July 2026 revision: no handshake, no sessions, server requests replaced by retried results, several features deprecated. Most servers in use still speak the older, handshake-based revisions, so a client must handle both. The official Rust SDK does. Details are in the research notes and must be re-read when the step starts.

## Options considered

**Scope.**

| Option | Assessment |
|---|---|
| Tools only, standard I/O and HTTP with static credentials | Covers the great majority of developer use. Small enough for one step. |
| Add OAuth for remote servers | Needs a browser hand-off, a loopback redirect, a hosted client identity document, token storage per issuer, and a paste-the-URL fallback. A step of its own. |
| Add prompts, resources, server-to-user questions | Useful, optional, and each is more surface. |

**Implementation.**

| Option | Clean release build (4 cores) | Size added | Crates | Assessment |
|---|---|---|---|---|
| Official SDK, standard I/O only | 27.6 s | 1.8 MB | 60 | Handles both protocol eras. |
| Official SDK, standard I/O and HTTP with authentication support | 45.3 s | 3.6 MB | 128 | The same, for remote servers. Shares the HTTP stack the harness already has, so the marginal cost in the full binary is lower than the figure here. |
| Hand-written client | none beyond what exists | small | 0 | Small for standard I/O with one protocol era. Tracking two eras and a moving specification by hand is the real cost. |

**How tools reach the model.** Declaring every tool (one reference harness) costs context and makes the tool list change mid-session. Hiding tools behind a scripting sandbox plus a search tool (the other) keeps the prompt stable at the price of a large subsystem. A fixed search-and-call pair keeps the prompt stable with no extra subsystem.

## Decision

- Tools only. Standard I/O, and HTTP with static credentials. Both protocol eras.
- The official SDK, client features only, confined to `cineris-mcp` behind our own transport trait and a cargo feature. *Amended in step 30: a client of our own, behind the same interface ("Amended in step 30" below).*
- Tools are reached through `tool_search` and `tool_call`, shared with extension tools, with opt-in direct exposure per server.
- Existing configurations are read: the project's `.mcp.json` live, and other clients' per-user files by import.
- Every tool definition is hashed at approval. A change needs approval again.

## Why

- The SDK's cost is real (the largest single addition after the HTTP stack) and is paid for something that would otherwise be ours to track through a specification that is still moving.
- Confinement to one crate behind a trait keeps the option of replacing it.
- A stable tool list is required by the context design ([0016](0016-context-management.md)), and the search-and-call pair gives it on every provider.

## Amended in step 30

**The client is our own, and no MCP SDK is linked** (the owner, 2026-11-27, on the maintainers' recommendation; [steps/30/answered.md](../steps/30/answered.md), point 1). Measured on hosted runners, `rmcp` 3.12.0 as the step would have linked it added 1,718,016 bytes to the shipped Linux build and 2,171,904 to the Windows one, and 19 packages; a client of our own was priced at a tenth of that or less. Most of a client was ours either way: the process, HTTP through `cineris-llm`, the policy around the protocol (time limits, caps, the probe of a server that exits on it) and the fake servers the tests need. Decision 0039's first size reserve is spent here.

- `cineris-mcp` speaks both eras itself: the JSON-RPC messages, a transport (standard I/O, and Streamable HTTP), a session over any transport, the probe and the fallback ([architecture/13-mcp.md](../architecture/13-mcp.md), "Implementation").
- Everything else is written against `cineris_mcp::Client`, so an SDK could be put back behind it, as this record first had it the other way round.
- **Against it:** interoperability is tested against servers we wrote from the specification's text. `rmcp`'s server is run against our client in the nightly tier, in both eras, from a package outside the workspace with its own lock file, kept out of the shipped binary and the blocking checks (the owner's yes of 2026-11-27). And each new revision of the specification is ours to read, twice a year.

## Revisit when

- Tracking the specification by hand costs more than an SDK would, with the size budgets met: put one back behind `Client`.
- Binary size or CI time is over budget: the `mcp` feature can be measured in isolation and trimmed.
- Users need a remote server that requires OAuth: that step moves up.
