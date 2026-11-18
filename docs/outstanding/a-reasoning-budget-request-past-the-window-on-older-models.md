A main request with a reasoning budget can pass the window with its `max_tokens` on Anthropic's models before Claude 4.5, failing every turn alike.

- Area: cineris-agent
- Owner: the step that adds a budget model to the catalog

| Item | Who | When |
|---|---|---|
| **A main request with a reasoning budget can pass the window with its `max_tokens` on Anthropic's models before Claude 4.5.** Those models refuse a request with thinking whose input and `max_tokens` together pass the context window, with a validation error that is not an overflow (`anthropic::classify` reads it as a provider's error), so the budget learns nothing and every turn fails the same way. The loop's limit keeps at most 32,768 tokens for the answer (`limit_of`), and a budget model's role writes up to its `max_output`, 64,000 on many. Claude 4.5 and later take such a request and stop with `model_context_window_exceeded`, which the loop answers. Since the review of the fixes after step 15's review, a summary request with a budget keeps its whole output in reserve; the turn's own request does not, because that would bring the trigger forward for every budget model, the later ones included, and no built-in model takes a budget today. To settle: keep the whole output in reserve for a budget request on the models that need it (a catalog flag), or classify that refusal as an overflow so the fallback answers it. | A later step that adds a budget model to the catalog, or the owner if a user's entry meets it | Before a model with a token budget is in the built-in catalog |
