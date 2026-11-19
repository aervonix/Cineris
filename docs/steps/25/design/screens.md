# Step 25, the design: the screens composed

The screens of step 25 that no reference screen draws yet, composed as text in the look of [decision 0033](../../../decisions/0033-tui-look.md) and [tui-look/](../../../architecture/tui-look/README.md), for the coordinator to judge against Roomy 2 and the golden screens before they are built widely; the owner sees them afterwards, for information. They are drafts: once the coordinator has answered, the ones kept become golden screens in `tui-look/` (step 25's third pull request), drawn by the real views and compared cell by cell. Until then the layout they show is kept easy to change. A script wrote them, so that every row is the width the window allows, one column less than the window's. They were revised after the design's review ([answered.md](../answered.md), "What the review found").

- **The first run** (F1a to F4) is drawn inline, as the rest of the application is. **Since the third pull request these are the golden screens** (`tui-look/f1a.txt` to `f4.txt`), drawn by the application itself and revised to the maintainers' answers: the imports' rows unpicked with each one's key named, no `unrestricted`, the set-up card never committed, the store named as each system's store is, the levels' sentences as 06's table has them, and a mask written `[redacted]` as the harness writes it everywhere else. The drafts' wording is replaced by the views' own; where a sentence wraps differently, it is the prose width's doing. As drafted: each step takes the input box's place, as a dialog does, under the set-up card, which stays in the live region while the steps before the session run and is never committed (the coordinator's answer, 2026-11-17). Once a step is answered, one dim line saying what it did is committed to the scrollback, so the steps already taken stay above the one being asked. The order is the provider, the imports, the mode, then the project: the configuration file is written after the mode, before the session opens, so that the first session is made with what the person chose. Step 4 comes after the session opened: the session's header card is committed above it, the only card then, and its trust question (screen 2c) came before it when the project has something to trust.
- **The full-screen presenter** (F5, F6) draws the same views. F5 is a window of 150 columns, where the panels sit in a column of their own beside the transcript; below the width at which they move there ([answered.md](../answered.md), design point 2), they are stacked above the input box as inline. As in step 24, a panel without the focus is drawn collapsed and the focused one whole. F6 is search.
- **What is illustration.** The providers, the files found, the servers and the commands are examples. F1a is Linux's wording and F1b Windows'.

### F1a. The first run, step 1: a key found in the environment (Linux)

100 columns, 21 rows. Now golden screen [f1a](../../../architecture/tui-look/f1a.txt).

```text
╭──────────────────────────────────────────────────────────────╮
│ ◆ Cineris 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped,  │
│   and x skips them all.                                      │
╰──────────────────────────────────────────────────────────────╯

◇ Provider  1 of 4
  A key for anthropic is in the environment, in ANTHROPIC_API_KEY. Use it from
  there, or store a key: it is kept in
  ~/.config/cineris/credentials.toml, which only you can read, and never
  in a file of settings.

  ◆ anthropic   its key from ANTHROPIC_API_KEY, as it is set
    anthropic   store a key
    openai      store a key
    openrouter  store a key
    deepseek    store a key
    venice      store a key

  enter chooses · s skips this step · x skips the set-up
```

### F1b. The first run, step 1: a key typed, nothing of it drawn (Windows)

100 columns, 16 rows. Now golden screen [f1b](../../../architecture/tui-look/f1b.txt).

```text
╭──────────────────────────────────────────────────────────────╮
│ ◆ Cineris 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped,  │
│   and x skips them all.                                      │
╰──────────────────────────────────────────────────────────────╯

◇ Provider  1 of 4
  Paste or type the key for anthropic. Nothing of it is shown. It is stored in
  Windows Credential Manager, for this machine only, and is never written to a
  file of settings, the log or the prompt history.

╭─────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ◆ 108 characters, hidden                                                                        │
╰─────────────────────────────────────────────────────────────────────────────────────────────────╯
  enter stores it · ctrl+u clears it · esc goes back
```

### F2a. The first run, step 2: what other tools left, nothing read or picked yet

100 columns, 28 rows. Now golden screen [f2a](../../../architecture/tui-look/f2a.txt).

```text
· Provider: anthropic, its key from ANTHROPIC_API_KEY.

╭──────────────────────────────────────────────────────────────╮
│ ◆ Cineris 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped,  │
│   and x skips them all.                                      │
╰──────────────────────────────────────────────────────────────╯

◇ Imports  2 of 4
  Other tools left these. None is read, started or put in the prompt unless you
  pick it, and what is left unpicked stays off. What you pick holds in every
  project.

    Instructions
  ◆ ☐ AGENTS.md, CLAUDE.md            read as each project's instructions; off
                                      unless picked (compat.instructions)
    Skills
    ☐ .claude/skills, .agents/skills  yours and each project's, once this
                                      version reads skills; off unless picked
                                      (compat.skills)
    MCP servers
    ▸ ~/.claude.json                  Claude Code's: space reads it
    ▸ ~/.cursor/mcp.json              Cursor's: space reads it
      .mcp.json, .vscode/mcp.json     this project's: offered in its sessions,
                                      after trust, never imported

  space picks · enter keeps what is picked · s skips this step · x skips the set-up
```

### F2b. The first run, step 2: one MCP file read, each server's command whole, one holding a credential

100 columns, 34 rows. Now golden screen [f2b](../../../architecture/tui-look/f2b.txt).

```text
· Provider: anthropic, its key from ANTHROPIC_API_KEY.

╭──────────────────────────────────────────────────────────────╮
│ ◆ Cineris 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped,  │
│   and x skips them all.                                      │
╰──────────────────────────────────────────────────────────────╯

◇ Imports  2 of 4
  Other tools left these. None is read, started or put in the prompt unless you
  pick it, and what is left unpicked stays off. What you pick holds in every
  project.

    Instructions
    ☑ AGENTS.md, CLAUDE.md            read as each project's instructions; off
                                      unless picked (compat.instructions)
    Skills
    ☐ .claude/skills, .agents/skills  yours and each project's, once this
                                      version reads skills; off unless picked
                                      (compat.skills)
    MCP servers
    ▾ ~/.claude.json                  Claude Code's
  ◆   ☑ docs     npx -y @example/docs-mcp --root ~/src/docs
      ☐ tracker  https://tracker.example.com/mcp
                 its Authorization header comes from TRACKER_AUTH, which you set
        github   docker run -i --rm -e GITHUB_TOKEN=[redacted]
                 ghcr.io/example/github-mcp:1.4
                 holds a credential in its command: not imported
    ▸ ~/.cursor/mcp.json              Cursor's: space reads it
      .mcp.json, .vscode/mcp.json     this project's: offered in its sessions,
                                      after trust, never imported

  space picks · enter keeps what is picked · s skips this step · x skips the set-up
```

### F3. The first run, step 3: the mode, before the session opens

100 columns, 22 rows. Now golden screen [f3](../../../architecture/tui-look/f3.txt).

```text
· Provider: anthropic, its key from ANTHROPIC_API_KEY.
· Imports: AGENTS.md and CLAUDE.md read; the MCP server docs.

╭──────────────────────────────────────────────────────────────╮
│ ◆ Cineris 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped,  │
│   and x skips them all.                                      │
╰──────────────────────────────────────────────────────────────╯

◇ Mode  3 of 4
  How much the agent does before it asks you. /mode changes it at any time.

    read-only   reads, and runs only commands known to read; asks before any
                other, and never writes
  ◆ supervised  reads and edits files in this project without asking; asks
                before a command that is not on a safe list, and before anything
                outside the project
    autonomous  runs commands in the sandbox without asking; asks before leaving
                it, using the network or writing outside the project

  enter chooses · s keeps supervised · x skips the set-up
```

### F4. The first run, step 4: the project's commands, after the session opened and trust was asked (screen 2c)

100 columns, 22 rows. Now golden screen [f4](../../../architecture/tui-look/f4.txt).

```text
· Provider: anthropic, its key from ANTHROPIC_API_KEY.
· Imports: AGENTS.md and CLAUDE.md read; the MCP server docs.
· Mode: supervised. Written to ~/.config/cineris/config.toml.

╭──────────────────────────────────────────────────────────────╮
│ ◆ Cineris 0.1.0                                       │
│                                                              │
│   claude-opus-5-5 high  ·  ~/src/parser                      │
╰──────────────────────────────────────────────────────────────╯

· Project: ~/src/parser trusted.

◇ Commands  4 of 4
  Found in Cargo.toml. Each one picked joins the project's profile as an entry
  you wrote: it runs without asking, and the checks at a turn's end use it.

  ◆ ☐ build   cargo build --workspace
    ☐ test    cargo test --workspace
    ☐ check   cargo clippy --workspace --all-targets -- -D warnings
    ☐ format  cargo fmt --all -- --check

  space picks · e edits · enter keeps what is picked · s skips this step     opus-5.5 high · fast
```

### F5. The full-screen presenter at 150 columns: panels beside the transcript, the plan focused

150 columns, 38 rows.

```text
◆ 1_000 lexes as 1. Fix scan_number so digit separators work, and add a test.                                ◇ Plan  1 of 4 done
                                                                                                               ☑ allow _ between digits
∴ Reasoned for 6s                                                                                              ◧ reject a trailing _
  The loop breaks on any non-digit, so _ ends the literal. Check whether a                                     ☐ add a test for 1_000
  trailing _ is meant to be an error first.                                                                    ☐ run clippy

◇ scan_number stops at the first character that is not a digit, so 1_000 ends
  after the 1. I'll read the scanner and allow _ between digits.                                               anthropic fast tier: 18% of the output
                                                                                                               token limit left, resets 15:00
╭─────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ✓ Read lex.rs  lines 120–178                                                                            │
│ ✓ Edit lex.rs  +5 −1                                                                                    │
│ ✗ Run cargo test -p parser  exit 101 · 3.4s                                                             │
│   test lex::numbers::trailing_separator ... FAILED                                                      │
│   assertion failed: lex("1_").is_err()                                                                  │
│   test result: FAILED. 47 passed; 1 failed; 0 ignored; finished in 0.02s                                │
╰─────────────────────────────────────────────────────────────────────────────────────────────────────────╯

◇ One test fails: 1_ must be an error, but the scanner now stops before the _
  and returns 1. A separator has to be followed by a digit, so the other case
  becomes an error:

  if c == '_' {
      if !self.peek_next().is_some_and(|d| d.is_ascii_digit()) {
          return Err(self.error("digit separator must be followed by a digit"));
      }
      self.bump();
      continue;
  }

╭─────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ⣾ Run cargo clippy -p parser -- -D warnings  6.2s                                                       │
╰──────────────────────────────────────────────────────────────────────────────────────── ctrl+t expands ─╯

╭───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ◆                                                                                                                                                 │
╰───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
  ⣾ 14s  space skips · e edits · a adds · esc back to the input                                                                opus-5.5 high · fast
```

### F6. Search in the full-screen presenter: a match inside a box it opened

100 columns, 20 rows.

```text
◇ scan_number stops at the first character that is not a digit, so 1_000 ends
  after the 1. I'll read the scanner and allow _ between digits.

╭─────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ✓ Read lex.rs  lines 120–178                                                                    │
│ ✓ Edit lex.rs  +5 −1                                                                            │
│ ✗ Run cargo test -p parser  exit 101 · 3.4s                                                     │
│   test lex::numbers::trailing_separator ... FAILED                                              │
│   assertion failed: lex("1_").is_err()                                                          │
│   test result: FAILED. 47 passed; 1 failed; 0 ignored; finished in 0.02s                        │
╰────────────────────────────────────────────────────────────────────────── opened by the search ─╯

◇ One test fails: 1_ must be an error, but the scanner now stops before the _
  and returns 1. A separator has to be followed by a digit, so the other case
  becomes an error:

╭─────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ◆ also run clippy when that's done                                                              │
╰─────────────────────────────────────────────────────────────────────────────────────────────────╯
  ctrl+f trailing_separator   2 of 3 · up older · down newer · esc ends             opus-5.5 high
```
