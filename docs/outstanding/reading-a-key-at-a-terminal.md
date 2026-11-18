`cineris auth login` refuses a terminal because reading a key without echo needs the terminal layer.

- Area: cineris-cli
- Owner: step 25

| Item | Who | When |
|---|---|---|
| **`cineris auth login` at a terminal.** [08-configuration.md](../architecture/08-configuration.md) says it reads a key without echo. Step 13b reads it from standard input when that is not a terminal, and refuses a terminal with how to give it a file: turning echo off needs the terminal layer (`cineris-term`), which `cineris-cli` reaches only through `cineris-tui`, or `unsafe` code of its own, which it does not allow. The onboarding flow asks for a key in the TUI anyway ("First run"). **Step 23 left it** (its design had it "if the time allows", and it did not): step 25's first-run flow, which asks for the provider and its key, is where it is needed first. | Step 25 (onboarding: the provider and its key), moved from step 23 | Before the first release |
