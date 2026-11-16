# The tenth tooling step: the owner's decisions of 2026-11-16, put into the repository

A pull request of its own, after the step's tools, made at the coordinator's request. One sentence a line. Each change is a commit of its own.

## A managed `ask` holds back the user's own check

- **The owner's choice** (the item `a-managed-ask-and-the-users-own-check`, now settled): an `ask` rule of the managed configuration holds back a line of the user's own `verify.commands`, as a managed `deny` does; a user's own `ask` rule, or any other `ask`, still lets the user's line run, as step 19 built it.
- **How:** the engine marks an `ask` that an `ask` rule decided, with whether the rule is the managed configuration's (`Finding::asked_by`), and the question carries it (`Question::managed`: whether a managed `ask` rule is among what asks).
  The verifier asks the engine for the question only when the decision is `ask`, and refuses the user's line when it is managed, saying that an administrator's rule asks and a check never asks.
  The asks a managed `deny` makes for a path it protects (a pattern that may expand to one, a search through one) are not `ask` rules, and are left as they were.
- **Tested:** `cortex-core::it verify::a_users_check_a_managed_ask_rule_asks_about_does_not_run_and_a_users_own_ask_does` (the managed file's `ask` holds the line back with its note; the same rule in the user's file lets it run; nothing is asked). Breaks: `mutations/tooling-10-decisions.breaks`, `managed-ask-*` and `any-ask-holds-the-users-line-back`.
- **Documented** in 15-efficiency ("Automatic verification"), `security.md`, and step 19's record.
