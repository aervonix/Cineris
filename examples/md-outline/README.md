# md-outline: a tool that reads files through its manifest

An extension that gives the model one tool, `md-outline/outline`: the headings of a Markdown file in the workspace, each with its level and its line number. With it the model can find the part of a long document it needs and read only those lines.

## What it shows

- **A tool written in JavaScript** (`main.js`), registered with `cineris.tool`. The model finds it through `tool_search` and calls it through `tool_call`, as it does every extension's tool.
- **A file read through `cineris.fs.read`,** the one way a script reads a file. The path is resolved as the built-in `read` tool resolves it, checked against the manifest's `read` globs, and then decided by your permission level and rules, as a read by the `read` tool would be.
- **A read outside the manifest refused.** Ask it for `src/main.rs`, or for a Markdown file outside the workspace, and the call fails with the reason, naming `capabilities.read`. Your rules are not asked: the manifest refuses first.

## What its manifest asks for, and why

| Capability | Value | Why |
|---|---|---|
| `read` | `["**/*.md"]` | It outlines Markdown files, so it may read Markdown files in the workspace, and nothing else. |

Nothing else: it runs no program, writes no file, reaches no network and hooks no event. Each capability left out is empty.

## Install it

```
cineris ext add examples/md-outline
```

Cineris reads the folder, shows what the extension may do and the tool it registers, and asks you to approve it. A yes copies the folder into your configuration directory's `extensions/md-outline/` (`~/.config/cineris/` on Linux, `%APPDATA%\cineris\` on Windows) and pins it by its hash: if a file in it changes, it is not loaded until you approve it again. The yes is your approvals password, typed at the terminal; with no terminal nothing is approved.

Cineris asks before each call of the tool, as it does for every extension's tool, unless a rule allows it or refuses it, or the session runs at the `unrestricted` level, where an extension's tool that no rule names runs unasked. To allow it without asking at the other levels, add a rule to your configuration:

```toml
[[permissions.rules]]
effect = "ext"
match = "md-outline/*"
action = "allow"
```

## Try it

Ask for the outline of a document: "Outline `guide.md`, then read me the part about installing." The model is given the tool's result as JSON, under a line that says it comes from the extension:

```json
{"headings":[{"level":1,"line":1,"text":"Guide"},{"level":2,"line":5,"text":"Install"}],"path":"guide.md"}
```

Headings written with `#` count; those underlined with `===` or `---` do not, nor a `#` line inside a fenced code block.

This example is run as a test on Linux and Windows: `crates/cineris-cli/tests/system/examples.rs`.
