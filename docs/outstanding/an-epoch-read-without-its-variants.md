An epoch from a log before step 16 has no variants and follows the live model's; reload must fix them.

- Area: cortex-agent
- Owner: step 24

| Item | Who | When |
|---|---|---|
| **An epoch read from a log written before step 16 has no variants, and follows the live model's.** An `epoch` entry without `variants` (a log from before step 16, or one with a choice this build does not know) is read as `variants: None`. `epoch_variants()` then falls back to `self.model.variants`, the model's as they are now, and a model-change epoch begun from such an epoch takes `None` too (`turn/epochs.rs`, `variants = epoch.variants.clone().map(…)`). Today nothing replaces the main model without beginning an epoch (`session/set` of the model or the effort begins one), so the variants cannot change under such an epoch, and this is latent. Found by the independent review of step 16 as a note; confirmed by reading. **What reload must do** when it rebuilds the role models of a running session (step 24): fix an epoch that has `variants: None` to the variants in force when it was resumed (or when reload began), record them in the session's state so that the next request and the next `epoch` entry carry them, and begin a new epoch, as a change of variants does, when the rebuilt model's variants differ from those. | Step 24 (hot reload of the role models) | When reload rebuilds role models |
