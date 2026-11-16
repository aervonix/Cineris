A project's `model.speed_cooldown_min = "0s"` can make a fast tier the user chose change speed every half minute, each change a possible rewrite of the prompt cache.

- Area: cortex-config
- Owner: the owner

| Item | Who | When |
|---|---|---|
| **A project's shortest cooldown.** `model.speed_cooldown_min` is a preference, from `"0s"` to an hour (a fix to step 9, 2026-11-15: [steps/09/answered.md](../steps/09/answered.md)), so a project's file may set it to `"0s"`, which takes a provider's time as given. [Decision 0027](../decisions/0027-speed-tiers.md), "A shortest cooldown", says what that does to a fast tier the user chose: a provider that keeps answering "in 25 seconds" gives 25-second cooldowns, and the session changes speed every half minute, each change a possible rewrite of the prompt cache. That raises what the user spends, which the trust rules keep from an untrusted repository, as well as slowing the session. `"0s"` is documented for the user, so the floor was kept and the class left as the owner set it. **Options:** a floor above zero (a lower value means changing speed more often than a provider's window, which the decision says is the waste); or a class the trust rules do not have yet, "raise only", in which a project's value applies only when it is at or above the one in effect. | The owner | Before the first usable release |
