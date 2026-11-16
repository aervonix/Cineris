A refused host outside ASCII is told the `xn--` form by an example, not its own; a percent-encoded one gets the URL library's message.

- Area: cortex-llm
- Owner: a later step on providers, or step 33

| Item | Who | When |
|---|---|---|
| **The message for a host outside ASCII names an example, not the host's own `xn--` form, and misses two spellings**, found by the independent review of step 33's size fix ([decision 0036](../decisions/0036-the-shipped-binary-is-optimised-for-size.md)). **(1)** `cortex_llm::http::host_outside_ascii_hint` says to write the host in its `xn--` form and shows `bücher.example` as `xn--bcher-kva.example`; it could give the refused host's own form, since Punycode needs no Unicode tables (`idna::punycode` is in the tree), though only for a host already in lower case and normalised, and naming `idna` directly needs a row in [dependencies.md](../dependencies.md). **(2)** A host written percent-encoded (`https://b%C3%BCcher.example`) is ASCII text, so the harness passes it, and the URL library refuses it with its own error, which does not say what to do. **(3)** A port written in full-width digits is reported as part of the host. Each is refused; only the message is worse. | A later step on providers, or step 33 | When a user meets one, or step 33 |
