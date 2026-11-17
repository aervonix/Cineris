# Step 25, the design: the screens composed

The screens of step 25 that no reference screen draws yet, composed as text in the look of [decision 0033](../../../decisions/0033-tui-look.md) and [tui-look/](../../../architecture/tui-look/README.md), for the coordinator to judge against Roomy 2 and the golden screens before they are built widely; the owner sees them afterwards, for information. They are drafts: once the coordinator has answered, the ones kept become golden screens in `tui-look/` (step 25's third pull request), drawn by the real views and compared cell by cell. Until then the layout they show is kept easy to change. A script wrote them, so that every row is the width the window allows, one column less than the window's.

- **The first run** (F1a to F4) is drawn inline, as the rest of the application is: each step takes the input box's place, as a dialog does, and once it is answered one dim line saying what it did is committed to the scrollback, so the steps already taken stay above the one being asked. Step 3's trust question is the trust dialog of screen 2c, as any session asks it; F3 is the step after it.
- **The full-screen presenter** (F5, F6) draws the same views. F5 is a window of 150 columns, where the panels sit in a column of their own beside the transcript; below the width at which they move there ([answered.md](../answered.md), design point 2), they are stacked above the input box as inline. F6 is search.
- **What is illustration.** The providers, the files found, the servers and the commands are examples. The usage panel in F5 is open because a limit is near, as in screen 4.

### F1a. The first run, step 1: a key found in the environment

100 columns, 20 rows.

```text
╭──────────────────────────────────────────────────────────────╮
│ ◆ Cortex-Harness 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped.  │
│   ~/src/parser                                               │
╰──────────────────────────────────────────────────────────────╯

◇ Provider  1 of 4
  A key for anthropic is in the environment, in ANTHROPIC_API_KEY. Use it from
  there, or store a key: it is kept in ~/.config/cortex-harness/credentials.toml,
  which only you can read, and never in a file of settings.

  ◆ anthropic    its key from ANTHROPIC_API_KEY, as it is set
    anthropic    store a key
    openai       store a key
    openrouter   store a key
    deepseek     store a key
    venice       store a key

  enter chooses · s skips this step · ctrl+c leaves, and asks again next time              1 of 4  
```

### F1b. The first run, step 1: a key typed, nothing of it drawn

100 columns, 16 rows.

```text
╭──────────────────────────────────────────────────────────────╮
│ ◆ Cortex-Harness 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped.  │
│   ~/src/parser                                               │
╰──────────────────────────────────────────────────────────────╯

◇ Provider  1 of 4
  Paste or type the key for anthropic. Nothing of it is shown. It is stored in
  ~/.config/cortex-harness/credentials.toml, which only you can read, and is
  never written to a file of settings, the log or the prompt history.

╭─────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ◆ key  108 characters, hidden                                                                   │
╰─────────────────────────────────────────────────────────────────────────────────────────────────╯
  enter stores it · ctrl+u clears it · esc goes back                                       1 of 4  
```

### F2a. The first run, step 2: what other tools left, nothing read yet

100 columns, 24 rows.

```text
╭──────────────────────────────────────────────────────────────╮
│ ◆ Cortex-Harness 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped.  │
│   ~/src/parser                                               │
╰──────────────────────────────────────────────────────────────╯

· Provider: anthropic, its key from ANTHROPIC_API_KEY.

◇ Imports  2 of 4
  Other tools left these. Nothing is read, started or put in the prompt
  unless you pick it here.

    Instructions, this project's
    ☑ AGENTS.md, CLAUDE.md          read as its instructions in every project
    Skills, read from step 28 on
    ☐ ~/.claude/skills              Claude Code, 4 skills
    ☐ .agents/skills                this project's, 2 skills
    MCP servers, started from step 30 on
  ◆ ☐ ~/.claude.json                Claude Code: space reads it and lists its servers
    ☐ ~/.cursor/mcp.json            Cursor: space reads it and lists its servers
      .mcp.json                     this project's: offered in each session, after trust

  space picks · enter brings in what is picked · s skips this step                         2 of 4  
```

### F2b. The first run, step 2: one MCP file read, each server's command whole

100 columns, 21 rows.

```text
╭──────────────────────────────────────────────────────────────╮
│ ◆ Cortex-Harness 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped.  │
│   ~/src/parser                                               │
╰──────────────────────────────────────────────────────────────╯

· Provider: anthropic, its key from ANTHROPIC_API_KEY.

◇ Imports  2 of 4
  ~/.claude.json holds 3 MCP servers. Each is written to your settings only if
  you pick it, with its command as it is here; none starts before step 30's
  client, and then in the sandbox.

  ◆ ☑ docs       npx -y @example/docs-mcp --root ~/src/docs
    ☐ tracker    https://tracker.example.com/mcp
    ☐ github     docker run -i --rm -e GITHUB_TOKEN ghcr.io/example/github-mcp:1.4
                 its GITHUB_TOKEN is a value in the file: it is brought in as the
                 variable's name, and the value is not copied

  space picks · enter goes back to the list · esc drops these picks                        2 of 4  
```

### F3. The first run, step 3: the project's commands, after the trust question (screen 2c)

100 columns, 21 rows.

```text
╭──────────────────────────────────────────────────────────────╮
│ ◆ Cortex-Harness 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped.  │
│   ~/src/parser                                               │
╰──────────────────────────────────────────────────────────────╯

· Provider: anthropic, its key from ANTHROPIC_API_KEY.
· Imports: AGENTS.md and CLAUDE.md read; the MCP server docs.
· Project: ~/src/parser trusted.

◇ Commands  3 of 4
  Found in Cargo.toml. Each one picked joins the project profile as an entry you
  wrote: it runs without asking, and the checks at a turn's end use it.

  ◆ ☑ build    cargo build --workspace
    ☑ test     cargo test --workspace
    ☑ check    cargo clippy --workspace --all-targets -- -D warnings
    ☐ format   cargo fmt --all -- --check

  space picks · e edits · enter keeps what is picked · s skips this step                   3 of 4  
```

### F4. The first run, step 4: the mode, supervised chosen

100 columns, 22 rows.

```text
╭──────────────────────────────────────────────────────────────╮
│ ◆ Cortex-Harness 0.1.0                                       │
│                                                              │
│   Setting up, in four short steps. Each one can be skipped.  │
│   ~/src/parser                                               │
╰──────────────────────────────────────────────────────────────╯

· Provider: anthropic, its key from ANTHROPIC_API_KEY.
· Imports: AGENTS.md and CLAUDE.md read; the MCP server docs.
· Project: ~/src/parser trusted.
· Commands: build, test and check, in the project profile.

◇ Mode  4 of 4
  How much the agent does before it asks you. /mode changes it at any time.

  ◆ supervised     reads and edits files in this project without asking; asks
                   before a command that is not known to be safe
    read-only      reads, and asks before every command; never writes
    autonomous     runs commands in the sandbox without asking
    unrestricted   never asks, and nothing is confined: your full rights

  enter chooses · s keeps supervised                                                       4 of 4  
```

### F5. The full-screen presenter at 150 columns: panels beside the transcript

150 columns, 35 rows.

```text
◆ 1_000 lexes as 1. Fix scan_number so digit separators work, and add a test.                            ◇ Plan  1 of 4 done
                                                                                                           ☑ allow _ between digits
∴ Reasoned for 6s                                                                                          ◧ reject a trailing _
  The loop breaks on any non-digit, so _ ends the literal. Check whether a                                 ☐ add a test for 1_000
  trailing _ is meant to be an error first.                                                                ☐ run clippy

◇ scan_number stops at the first character that is not a digit, so 1_000 ends                            ◇ Changed 1 file  +5 −1
  after the 1. I'll read the scanner and allow _ between digits.                                           crates/parser/src/lex.rs  +5 −1

╭─────────────────────────────────────────────────────────────────────────────────────────────────────╮  ◇ Usage  this session
│ ✓ Read lex.rs  lines 120–178                                                                        │    opus-5.5   in 41k  out 3.2k   $0.38
│ ✓ Edit lex.rs  +5 −1                                                                                │    cache hit 92%
│ ✗ Run cargo test -p parser  exit 101 · 3.4s                                                         │    anthropic fast tier: 18% of the output
╰──────────────────────────────────────────────────────────────────────────── click or enter expands ─╯    token limit left, resets 15:00

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

╭─────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ⣾ Run cargo clippy -p parser -- -D warnings  6.2s                                                   │
╰──────────────────────────────────────────────────────────────────────────────────── ctrl+t expands ─╯

╭───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ◆ also run clippy when that's done                                                                                                                │
╰───────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
  ⣾ 14s  esc stops · enter steers this turn · tab queues it instead                                                            opus-5.5 high · fast  
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
╰──────────────────────────────────────────────────── opened by the search · enter keeps it open ─╯

◇ One test fails: 1_ must be an error, but the scanner now stops before the _
  and returns 1. A separator has to be followed by a digit, so the other case
  becomes an error:

╭─────────────────────────────────────────────────────────────────────────────────────────────────╮
│ ◆ also run clippy when that's done                                                              │
╰─────────────────────────────────────────────────────────────────────────────────────────────────╯
  search  trailing_separator  2 of 3 · enter or up older · down newer · esc ends    opus-5.5 high  
```
