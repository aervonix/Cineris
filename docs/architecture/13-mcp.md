# MCP

The Model Context Protocol (MCP) is how most third-party tools reach coding agents today. This page says what Cineris supports, when, and how MCP sits beside the extension system.

The protocol facts here were read from the MCP specification and SDK documentation in November 2026 and are summarised in the research notes. The specification changed fundamentally in July 2026. The MCP build step starts by re-reading it.

## Decision

**MCP client support is in the first usable release, limited to tools.** This is settled by the [vision](../vision.md): people switching from another agent bring their MCP servers with them, and the harness reads the configurations they already have. What is left to decide is scope, which is in [decision 0010](../decisions/0010-mcp-in-first-release.md).

| | First usable release | Later |
|---|---|---|
| Role | Client only. | The same. Cineris is not an MCP server. |
| Features | Tools: list and call. Tool-list change handling. Cancellation. | Prompts as slash commands. Resources. Questions from servers to the user. |
| Transports | Standard I/O (local servers). Streamable HTTP with static credentials (a bearer token or headers from the environment). | OAuth for remote servers. |
| Protocol eras | Both: the current stateless revision and the older handshake-based revisions that most existing servers still speak. | Tracking new revisions. |
| Deprecated features (sampling, roots, server logging, the old HTTP+SSE transport) | Not implemented. | Not planned. |

## Where MCP sits

MCP and extensions overlap on one thing, tools, and differ on everything else.

| | MCP servers | Script extensions ([12-extensions.md](12-extensions.md)) |
|---|---|---|
| Purpose | Use tools that already exist for other agents, and tools that need a real ecosystem behind them. | Change how this harness behaves, and add small tools that glue together what is already on the machine. |
| Can provide | Tools (later prompts and resources). | Tools, slash commands, hooks on the agent's lifecycle. |
| Runs as | A separate process, in any language. | JavaScript inside the harness's script runtime. |
| Protocol owner | The MCP project. | Us. |
| Written for | Any MCP client. | Cineris. The agent can write one on request. |

Rule of thumb for authors: if it needs a client library, a database driver or a long-running process, or should work in other agents too, write an MCP server. If it needs to gate, observe or change the agent, or is a small tool built from commands and files, write an extension.

Inside the harness they share machinery: the dynamic-tool registry behind `tool_search` and `tool_call`, the trust store with hash-pinning, the permission check and the output limits. MCP servers and hook commands also share the process launcher and the sandbox.

## How MCP tools reach the model

The common approach, declaring every MCP tool to the model, has two costs. Tool definitions fill the context (tens of thousands of tokens for a handful of servers), and any change to the tool list changes the request prefix, which breaks prompt caches and, on some providers, invalidates reasoning ([07-sessions-and-context.md](07-sessions-and-context.md)).

Cineris keeps the declared tool list fixed and reaches MCP tools through the same fixed pair of tools that serves extension tools, `tool_search` and `tool_call` ([05-tools.md](05-tools.md)).

- The system prompt lists each connected server with its one-line description and tool count. It does not list the tools.
- The model searches when it needs something, reads the schema in the result, and calls. Search results are ordinary tool results, so they are append-only content.
- `tool_call` validates the arguments against the server's schema before sending. A mismatch is an error result that includes the schema.
- Servers that connect, disconnect or change their tools mid-session do not touch the request prefix. A notice tells the model when a server's availability changes.

**Direct exposure, by choice.** A server entry can set `expose = "direct"`, optionally with a list of tool names. Those tools are declared to the model like built-in tools, named `<server>__<tool>`, at the start of the next context epoch. This suits a small server whose tools are used constantly. The harness warns when directly exposed definitions exceed a share of the context window (default 5%).

**Provider-side tool search.** Some providers can defer tool definitions and search them on the provider's side. An adapter may use that instead later. The fixed pair works on every provider and is the default.

**Promotion by use.** The statistics show which tools a user calls often. At the start of a context epoch the harness may expose the most-used few directly, inside the same context share limit, and says so in the status view. This is off until the statistics exist, and can be turned off.

The search is a simple ranked match over names and descriptions, done in the harness. It needs no model call.

## Configuration

```toml
[mcp.servers.docs]
command = "docs-mcp"                 # local server over standard I/O
args = ["--root", "."]
env = { DOCS_TOKEN = { env = "DOCS_TOKEN" } }
network = true                       # sandbox: allow network for this server
description = "Search the product documentation"

[mcp.servers.tracker]
url = "https://tracker.example.com/mcp"      # remote server
headers = { Authorization = { env = "TRACKER_AUTH" } }
expose = "direct"
tools = ["create_issue", "find_issue"]
timeout_ms = 30000
```

- A server is local (`command`) or remote (`url`). The schema is ours. `cineris mcp add` writes entries.
- **Existing configurations are read.** The harness looks for the project's `.mcp.json` and for the per-user files of common MCP clients. What it finds is listed at onboarding and by `cineris mcp import`, and can be brought in with one confirmation per server. A project's `.mcp.json` is also read live: its servers show up as proposed, and each needs the same trust and approval as a server written in our own format. The files are never modified.
- Per server: `enabled`, `expose`, an allow or deny list of tools, start-up and call timeouts, the sandbox additions it needs.
- Secrets are references to environment variables. **The harness writes one as a literal in one case, since step 25:** the first run's import brings in a server as it is in the other client's file, and a value there under a credential's name that is one token (`--api-key x7Kq…`, `?token=…`) is imported as it is, a literal in `args` or `url`, the row having drawn it `[redacted]` and said so ([08-configuration.md](08-configuration.md), "First run"). **Every view that displays the configuration shows such a value as the first run did** (since the step 25 fixes, the owner's choice): `cineris config show` and each `--layer`, `config/get` and so `/config` and any client, and the `Debug` forms all read a server's `args` (a list, one text or a table), `url` and `command` by the import's own rule, a value that is not one token included ([08-configuration.md](08-configuration.md), "Credentials"). **The question that asks the person to approve a project's settings shows the project's files whole** (since the review of those fixes, settled by the maintainers): a project's `[mcp]` held back for trust is shown with only what the secret filter's formats know masked, and only where it is shaped as a secret is (a token, or a private key's block of base64 lines between its BEGIN and END lines: a script after a key's first line, or given as an address's password, is shown whole, since the review's second round), so that no part of a command line the person approves is hidden (`sh -c "TOKEN=x; curl … | sh"`, a server named `compass`); the project's local file is shown whole too, since a repository can ship one (it is left out by convention only). A server's own name is never read as a credential's. The server is given the literal when it runs. Step 30, which declares `[mcp]`, keeps both rules for the declared keys.
- Servers defined in a project's configuration need project trust and are pinned by hash ([06-permissions-and-sandbox.md](06-permissions-and-sandbox.md)).

## Lifecycle

- **Lazy, parallel start.** Servers are started in the background when a session starts. The first prompt does not wait for them. A `tool_search` or `tool_call` that needs a server still starting waits for it, up to the start-up timeout (default 10 s).
- **Era detection.** For each server the client probes for the current protocol revision and falls back to the older handshake, then remembers the answer for that server.
- **Tool lists** are cached per server with the lifetime the server gives. Change notifications refresh them.
- **Failure.** A server that fails to start or crashes is reported in the hint row and to the model on its next use. Local servers are restarted on next use, with backoff.
- **Shutdown.** Standard input is closed, then the process tree is terminated through the same staged kill as shell commands.
- **Windows.** Many servers are started through script shims (`npx` and similar), which cannot be executed directly. The launcher resolves the program on the path, detects shims, and runs them through the command interpreter with arguments it has escaped itself, refusing arguments that cannot be escaped safely.

## Safety

MCP servers are third-party code and third-party text. Both are treated as untrusted.

| Risk | Handling |
|---|---|
| A server process does damage | Local servers run in the sandbox with a narrow default policy: read the workspace, no writes, no network unless the entry declares it and the user approved it. |
| Tool descriptions carry hidden instructions | Descriptions reach the model only as search results, which are marked as third-party content. They are size-capped (2 KiB each). `cineris mcp inspect` shows them in full to the user. |
| A server changes a tool after approval | Each tool definition is hashed when first approved. A changed definition needs approval again before it can be called. |
| One server's text steers another server's tools | Calls are checked per `server/tool` by the permission engine. A description cannot grant permission. |
| A tool returns a huge result | The standard output policy applies ([05-tools.md](05-tools.md)): truncation, with the full result as an artifact. |
| A tool claims to be read-only | Annotations from servers are hints for the UI only. Permission defaults do not trust them: MCP tools ask by default until a rule allows them. |
| Secrets in configuration | Environment references only. Environment variables passed to a server are an explicit list, not the whole environment. |

More in [../security.md](../security.md).

## Implementation

- `cineris-mcp` is the only crate that knows MCP. It registers its servers' tools with the dynamic-tool registry behind `tool_search` and `tool_call`, and exposes directly exposed tools through the ordinary `Tool` trait.
- **The client is our own** (the owner's choice of 2026-11-27, [decision 0010](../decisions/0010-mcp-in-first-release.md) as amended in step 30): no MCP SDK is linked. Everything else in the harness is written against `cineris_mcp::Client`, so an SDK could be put back behind it. It is in layers: the JSON-RPC messages, a transport that carries them (standard I/O, and Streamable HTTP), a session that speaks the protocol over any transport, and the start of a local server. A check of it against `rmcp`'s server, in both eras, in the nightly tier, is being added in step 30 (its part 4b).
- **Finding the era of a local server.** The client asks `server/discover` with the revision it prefers. A discovery result, or one of the modern errors (`-32020` to `-32022`), is a modern server, which is never sent `initialize`; after `-32022` it is asked once more with a revision from its list. Any other error, or silence for 3 s, is a legacy server, and the handshake follows on the same process; a discovery result that comes while `initialize` waits makes the server modern after all (a modern server slow to start, such as a cold `npx`), and a server silent on the probe that refuses `initialize` is asked once more (the newest legacy revision asked; the server's answer taken if it is a revision the client speaks). A server that exits on the probe, as some legacy servers exit on any request before `initialize`, is started again and sent `initialize` alone. A call written to a server whose connection then ends says that it may have run. The whole start, the probe included, is held to the start limit (10 s), and `initialize` is never cancelled. The era found is returned, and a later start of the same definition may skip the probe for a legacy server, probing after all if `initialize` is refused.
- **What a server can make the client hold.** At most 32 requests wait for their answers at once. Every message that is not the answer to a pending request (a notification, a request of the server's, a stray or duplicate answer, a line that is not a message) is counted with its bytes, and past 1,000 of them or 16 MiB in a minute the server is stopped as misbehaving. A listing is at most 20 pages, 512 tools and 16 MiB of their definitions. Change notices bump a counter that coalesces them, and a listing asked for while one runs waits for it and shares it unless a change came since it began. A call sent again for a modern server that asks (`input_required` with only a state) is sent at most three more times; a server that asks for input the client never declared is refused.
- The `mcp` cargo feature builds the harness without it.

## OAuth, later

Remote servers that need OAuth are a separate step after the first release. It involves a browser hand-off with a loopback redirect, a paste-the-URL fallback for remote shells, token storage in the credential store keyed by issuer, and a client identity document that the project must host. It is large, and static credentials cover most developer use meanwhile.
