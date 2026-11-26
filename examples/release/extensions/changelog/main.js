// One command, `/changelog:draft [since]`: it reads the commits since a tag
// (or any revision) with `git log`, and sends a prompt that asks for a
// changelog entry made from them. The prompt is the command's result: the
// model sees it under a line that names the command and its extension.

// The most commits one entry is drafted from.
const MOST = 200;

cineris.command({
  name: "draft",
  description:
    "Draft a changelog entry from the commits since a tag or other revision, or from the " +
    "whole history when none is given",
  arguments: ["since"],
  async run({ arguments: { since } }) {
    // A word that begins with `-` looks like an option, and is not a
    // revision: it is refused here, before git is started.
    if (since !== undefined && since.startsWith("-")) {
      throw new Error(`\`${since}\` is not a revision: give a tag, a branch or a commit`);
    }
    const range = since === undefined ? "HEAD" : `${since}..HEAD`;
    // Each commit on a line, its short hash and its subject. `--oneline`
    // rather than a `--format` of its own: on Windows a `%` in an argument
    // keeps any rule from allowing the line. `--end-of-options` comes last
    // of the options: git reads no word after it as one, so a rule that
    // allows this line up to it allows no other option (the README's).
    const log = await cineris.exec("git", [
      "log",
      "--no-merges",
      `--max-count=${MOST}`,
      "--oneline",
      "--no-decorate",
      "--end-of-options",
      range,
    ]);
    if (log.code !== 0) {
      throw new Error(`git log failed: ${log.stderr.trim()}`);
    }
    const commits = log.stdout.trim();
    if (commits === "") {
      throw new Error(`there are no commits since ${since ?? "the start"}`);
    }
    const what = since === undefined ? "this project's history" : `the changes since ${since}`;
    return [
      `Draft the changelog entry for ${what}. These are its commits, newest first, each its short hash and its subject:`,
      "",
      commits,
      "",
      "Group them under Added, Changed, Fixed and Removed, and leave out a group with nothing " +
        "in it. Write each change as one line a user of the project would understand, and " +
        "join the commits that make one change. Leave out commits that change nothing a user " +
        "sees, such as tests and refactoring. Show me the entry; do not write it to a file " +
        "until I ask.",
    ].join("\n");
  },
});
