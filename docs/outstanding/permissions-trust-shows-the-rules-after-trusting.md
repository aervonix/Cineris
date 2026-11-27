`cineris permissions trust` trusts a project's rules, settings and git before it shows them, where the application's trust question shows them first: a person running it learns what they trusted after they trusted it.

- Area: cineris-cli
- Owner: the maintainers

| Item | Who | When |
|---|---|---|
| **What was seen.** The guards' area of the review of step 28 noted, at lower confidence and as predating step 28, that `cineris permissions trust` trusts the project's rules, settings and git and then lists them, where the application's trust question lists what the project would be trusted with before the person's yes. Its extensions and hooks are shown before they are approved, each by its hash (step 28); the rest is not. Nothing a session's command can reach: the subcommand refuses under one (B). **Recommendation:** show what trusting would take (the rules, the settings, git, the sandbox's warnings) before trusting, and at a terminal ask, as the trust question does; with no terminal, keep trusting as the person asked, and print the same list first. | The maintainers | Before the first release |
