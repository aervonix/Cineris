// One tool, `outline`: the headings of a Markdown file in the workspace,
// each with its level and the line it is on, so that the model can find its
// way round a long document before it reads any of it.
//
// The file is read with `cineris.fs.read`, which checks the path against the
// manifest's `read` globs before anything else: a path that is not a
// Markdown file in the workspace is refused there, and the script's call
// fails with the reason.

cineris.tool({
  name: "outline",
  description:
    "List the headings of a Markdown file in the workspace: each heading's level, its text " +
    "and its line number. Use it to find the part of a long document you need, then read " +
    "only those lines.",
  input: {
    type: "object",
    properties: {
      path: { type: "string", description: "The Markdown file, relative to the workspace." },
    },
    required: ["path"],
    additionalProperties: false,
  },
  readOnly: true,
  async run({ path }) {
    const text = await cineris.fs.read(path);
    return { path, headings: headings(text) };
  },
});

// The headings written with `#` (`## Install`), outside fenced code blocks,
// whose lines are code: a `# comment` in a shell example is not a heading.
function headings(text) {
  const found = [];
  let fence = null;
  text.split("\n").forEach((raw, index) => {
    const line = raw.trimEnd();
    const marker = /^ {0,3}(`{3,}|~{3,})/.exec(line);
    if (marker !== null) {
      // A fence ends at a line that opens with the same character, at least
      // as many times.
      if (fence === null) {
        fence = marker[1];
      } else if (marker[1][0] === fence[0] && marker[1].length >= fence.length) {
        fence = null;
      }
      return;
    }
    if (fence !== null) {
      return;
    }
    const heading = /^ {0,3}(#{1,6})(?:\s+(.*?))?(?:\s+#+)?$/.exec(line);
    if (heading !== null) {
      found.push({ level: heading[1].length, text: heading[2] ?? "", line: index + 1 });
    }
  });
  return found;
}
