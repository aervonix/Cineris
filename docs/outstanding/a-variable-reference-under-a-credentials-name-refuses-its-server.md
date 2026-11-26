The first run's import refuses a server whose credential's argument is already a `${VAR}` reference, telling the person to move it into a variable.

- Area: cineris-config
- Owner: step 30

| Item | Who | When |
|---|---|---|
| **A reference is taken for a literal.** The first run's import of another tool's MCP servers (`crates/cineris-config/src/import.rs`) refuses a server with `["--api-key", "${SEARCH_API_KEY}"]` and says "move it into a variable", though it already is one. It fails on the safe side: the server is left out. Found by the review of step 25 and confirmed by a scratch test it ran against `cineris-config` (`review_a_variable_reference_under_a_credentials_name_refuses_its_server`, kept with the review, not in the repository). **Recommendation:** show such a reference whole and import it as it is, saying what the server will be passed (the variable's value when the server starts). Step 30, which declares `[mcp]` and resolves its references, settles how a `${VAR}` in `args` is read; the import's other names left for it are in [the-first-runs-import-and-writer-corners](the-first-runs-import-and-writer-corners.md). | Step 30 (the MCP client) | With step 30 |
