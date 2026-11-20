# Step 28, its parts: the fetch

The second of step 28's pull requests: `cineris.http.fetch`, an extension's HTTP request made by the host. The maintainers settled that it comes in a pull request of its own after the engine's, and that its bar is the resolver: no second lookup, the refused ranges with their IPv6 forms and the written forms of an address, no redirect, and the headers that steer a request.

## What was built

- **The fetch client, in `cineris-llm`** behind `http`, since only that crate links the HTTP stack: `Fetcher`, given a resolver and a timer. A request's body and headers are checked before anything is looked up; the URL is read by the URL standard (`reqwest`'s `Url`), so every written form of an IPv4 address is the address it means; the name is looked up once and every address it has is checked; the HTTP client is made for that one fetch with a resolver that answers the request's host with the checked addresses and refuses every other name, no proxy, no redirect, no retry and no referrer; and the connection's peer must be one of the checked addresses. The ranges refused unless named literally: loopback, unspecified, link-local, private (with IPv6's site-local), shared (carrier-grade NAT), unique-local, multicast, broadcast, and the metadata addresses, each in its IPv6 forms too (IPv4-mapped, translated, IPv4-compatible, NAT64's well-known prefix). `http` goes only to a literal address the caller allows, or to `localhost` when loopback is allowed.
- **The host that was checked is the host connected to.** The extension side reads the URL's host for the manifest's `net` patterns and the permission decision, and refuses a host the URL standard would read otherwise: a `\` (which ends the host there), a tab or a line break (dropped), a `%` (decoded), a host that ends in a number unless it is four decimal numbers (`0x7f.1`, `2130706433`, `127.1`, `0177.0.0.1`), and an IPv6 address with anything but hex digits, colons and dots. The client is given the host its caller checked, and refuses a URL it reads as another before the lookup. Found while the client was carried onto the first part's code: before it, a manifest allowing `*.example.com` let `https://evil.com\.example.com/` reach `evil.com`.
- **The wiring:** `cineris.http.fetch` in the engine's prelude and `host::serve` (the manifest's hosts, its literal entries as the client's allowance, the call's time as the fetch's longest), and the core's host (a `network` effect on the host, decided as every host call is, then the client).

## What was measured

| Budget | Measured | Where |
|---|---|---|
| Binary size | The client adds no crate: `reqwest` and `rustls` were linked for the providers, and `cineris-llm` turns on `tokio`'s `rt`, which the workspace already builds | [dependencies.md](../../../dependencies.md) |
| Start-up | Untouched: the client is made only when a script fetches | `cineris-core::it extensions` |
