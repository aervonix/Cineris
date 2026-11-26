// A `tool.before` hook on the shell tool. It refuses two commands that
// cannot be taken back, a recursive and forced `rm` and a forced
// `git push`, and has no opinion on any other: what it lets through is
// decided by your permission level and rules, as it would be without it.

cineris.on("tool.before", { tools: ["shell"] }, (event) => {
  const line = String(event.arguments.command ?? "");
  for (const words of commandsIn(line)) {
    if (removesForcefully(words)) {
      return {
        decision: "deny",
        reason:
          "`rm` with `-r` and `-f` deletes a whole tree without asking, and nothing brings it " +
          "back. Remove the files you mean by name, or ask the user to run it.",
      };
    }
    if (pushesForcefully(words)) {
      return {
        decision: "deny",
        reason:
          "A forced push replaces the remote's history with yours, and commits others pushed " +
          "are lost. Use `--force-with-lease`, which refuses when the remote has commits you " +
          "have not fetched, or ask the user to push.",
      };
    }
  }
  return undefined;
});

// The simple commands of a line, each as its words. Quotes keep a word
// whole and are taken off it, a backslash escapes the next character, and
// `;`, `&`, `|`, `(`, `)`, a backtick or a line break ends a command, so
// that `cd out && rm -rf .` and `$(rm -rf .)` are each read as the commands
// they run. It reads a line as a POSIX shell would split it, not a whole
// grammar: enough to see what a command line runs.
function commandsIn(line) {
  const commands = [];
  let words = [];
  let word = null;
  let quote = null;
  const endWord = () => {
    if (word !== null) {
      words.push(word);
      word = null;
    }
  };
  const endCommand = () => {
    endWord();
    if (words.length > 0) {
      commands.push(words);
    }
    words = [];
  };
  for (let at = 0; at < line.length; at += 1) {
    const character = line[at];
    if (quote !== null) {
      if (character === quote) {
        quote = null;
      } else if (character === "\\" && quote === '"' && at + 1 < line.length) {
        at += 1;
        word += line[at];
      } else {
        word += character;
      }
    } else if (character === "'" || character === '"') {
      quote = character;
      word = word ?? "";
    } else if (character === "\\" && at + 1 < line.length) {
      at += 1;
      word = (word ?? "") + line[at];
    } else if (character === "\n" || ";&|()`".includes(character)) {
      endCommand();
    } else if (/\s/.test(character)) {
      endWord();
    } else {
      word = (word ?? "") + character;
    }
  }
  endCommand();
  return commands;
}

// The program a word names: its last part, without `.exe`, in lower case.
function program(word) {
  return word.split(/[\\/]/).pop().toLowerCase().replace(/\.exe$/, "");
}

// `rm` given both `-r` (or `-R`) and `-f`, in any form: `-rf`, `-fr`,
// `-r -f`, `-Rf`, `--recursive --force`. Found anywhere in the command, so
// that `sudo rm -rf` and `xargs rm -rf` are read too.
function removesForcefully(words) {
  const at = words.findIndex((word) => program(word) === "rm");
  if (at < 0) {
    return false;
  }
  let recursive = false;
  let force = false;
  for (const word of words.slice(at + 1)) {
    if (word === "--") {
      break;
    }
    if (word === "--recursive") {
      recursive = true;
    } else if (word === "--force") {
      force = true;
    } else if (/^-[A-Za-z]+$/.test(word)) {
      recursive ||= /[rR]/.test(word);
      force ||= word.includes("f");
    }
  }
  return recursive && force;
}

// `git push` with `--force`, `-f` (alone or among other short options, as
// in `-uf`), or a refspec that begins with `+`, which forces that one
// branch. `--force-with-lease` and `--force-if-includes` are let through.
function pushesForcefully(words) {
  const at = words.findIndex((word) => program(word) === "git");
  if (at < 0) {
    return false;
  }
  // git's own options come before its subcommand; `-C` and `-c` take the
  // next word as their value.
  let next = at + 1;
  while (next < words.length && words[next].startsWith("-")) {
    next += words[next] === "-C" || words[next] === "-c" ? 2 : 1;
  }
  if (words[next] !== "push") {
    return false;
  }
  return words
    .slice(next + 1)
    .some(
      (word) =>
        word === "--force" ||
        (/^-[A-Za-z]+$/.test(word) && word.includes("f")) ||
        word.startsWith("+"),
    );
}
