An extension's tool runs alone by its spec and by its plan, and no test runs one beside another call, so a break of the plan's alone is caught by nothing.

- Area: cineris-core
- Owner: step 30

| Item | Who | When |
|---|---|---|
| **An extension's tool running alone has no end-to-end test.** It is held twice: by its spec (`ext::tools::spec_of`, `ConcurrencyClass::Exclusive`, which a unit test holds) and by its plan (`ExtensionTool::plan`, `Concurrency::Exclusive`), and `tool_call` honours either, so a break of the plan's alone changes nothing a test can see, and no test runs an extension's tool beside another call of its response (the review of step 28's close). A unit test of the plan needs a loaded extension, whose host parts are a session's. **Recommendation:** with step 30's MCP tools, which must be exclusive in their spec too, a test of the loop that runs a response of an exclusive tool's call and another call, and sees the other start only after the exclusive one ended, for an extension's tool and for an MCP tool. | Step 30 (MCP client) | With the MCP tools |
