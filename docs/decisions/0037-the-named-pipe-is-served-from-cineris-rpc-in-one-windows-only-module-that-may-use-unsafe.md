# 0037: The named pipe is served from cineris-rpc, in one Windows-only module that may use unsafe

- **Status:** Proposed in step 27 on 2026-11-16, for the owner
- **Date:** 2026-11-16

## Context

`cineris serve --socket` (step 27) serves several clients of the same user over a local socket. On Windows that is a named pipe, and its security cannot come from the standard library or from tokio:

- **The pipe's security descriptor** (owner, access list and mandatory integrity label) must be given to the call that makes each instance, `CreateNamedPipeW`, so that no instance ever exists without it. tokio's `ServerOptions` takes one only through an `unsafe` function, and needs a runtime to make an instance, where binding needs none.
- **The check of each connection's own token** (its user, integrity level, restrictions, AppContainer) takes `ImpersonateNamedPipeClient`, `OpenThreadToken`, `RevertToSelf` and the token queries, all `unsafe` to call.

Without them any process of another user, or a lower-integrity process of the same user (a sandboxed browser, say), could drive a server whose commands run as the user, or elevated.

## Options

| Option | For | Against |
|---|---|---|
| **A helper in `cineris-sandbox`,** which may already use `unsafe`. | No new place for `unsafe`. | `cineris-rpc` may depend only on `cineris-protocol`, `cineris-base` and `cineris-core` ([architecture/01-workspace.md](../architecture/01-workspace.md)); a pipe server has nothing to do with sandboxing. |
| **A helper in `cineris-base`'s `private::windows`,** the module [decision 0029](0029-owner-only-files-on-windows.md) allows `unsafe` in. | Reuses an audited module. | That module makes private files; a server's pipe instances, impersonation and token checks are a second subject, and `cineris-base` would gain tokio's `net` for one consumer. It does gain one small public function from it: the user's SID as text. |
| **Rely on the pipe's default security.** | No `unsafe`. | The default access list admits others (Everyone may read, and the creator's group), and there is no integrity label: the rule in [security.md](../security.md), section 5, would not hold. |
| **One Windows-only module of `cineris-rpc`,** `socket::windows`. | One audited place, beside the Unix half it mirrors; the security is part of the transport and stays with it. | A new place for `unsafe`. |

## Decision

**The named pipe is made and checked in `crates/cineris-rpc/src/socket/windows.rs`, the one module of `cineris-rpc` where `unsafe` is allowed,** with a module-level allow that says why and a `// SAFETY:` comment on every block. It calls `windows-sys`, compiled on Windows only, with the API families it needs and no more. The user's SID comes from `cineris_base::private::current_user_sid`. Nothing else in `cineris-rpc` uses `unsafe`.

## Why

1. The rule that a client is never more than the user at the terminal ([security.md](../security.md), section 5) needs the descriptor at creation and the token check, and both need the Windows API.
2. The crate rules keep `cineris-rpc` off `cineris-sandbox`, and a pipe server is not what `cineris-base`'s private-file module is for.
3. Keeping the Windows half beside the Unix half keeps the two transports' guarantees in one place to read and test.

## What is given up

The pipe's security keeps out other users and lower-integrity processes of the user. It cannot keep out a process of the user at the server's own integrity, which may open the server's process, a client's, or the pipe's name with full rights by default. The refusal of the server's own commands, by the job the server joins, is therefore a thin check on Windows ([security.md](../security.md), section 5, "Residual risk"): the person who approves a command is what keeps it from answering its own session's approvals.

## Revisit when

tokio or the standard library can make a named pipe with a given security descriptor, and query a pipe client's token, without `unsafe`; or a second crate needs to serve a pipe.
