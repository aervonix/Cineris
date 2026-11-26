// A `tool.before` hook on the shell tool. It refuses two commands that
// cannot be taken back, a recursive and forced `rm` and a forced
// `git push`, in the forms its README lists, and has no opinion on any
// other: what it lets through is decided by your permission level and
// rules, as it would be without it. It is an example of a hook, not a
// safety boundary: a command can always be written in words it does not
// read.

cineris.on("tool.before", { tools: ["shell"] }, (event) => {
  const line = String(event.arguments.command ?? "");
  for (const words of commandsRun(line)) {
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

// The simple commands of a line, and of each line it hands a shell to read
// again: the argument of `sh -c` and `bash -c`, and the words after
// `eval`. Each line read again is a part of the one before, so this ends.
function commandsRun(line) {
  const all = [];
  const lines = [line];
  while (lines.length > 0) {
    for (const words of commandsIn(lines.pop())) {
      all.push(words);
      lines.push(...readAgain(words));
    }
  }
  return all;
}

// The lines a command hands a shell to read again: what `sh` or `bash` is
// given after `-c` (or after a run of short options with `c` among them,
// as in `-lc`), and the words after `eval`, joined as `eval` joins them.
function readAgain(words) {
  const again = [];
  words.forEach((word, at) => {
    const name = program(word);
    if (name === "eval") {
      again.push(words.slice(at + 1).join(" "));
    } else if (name === "sh" || name === "bash") {
      const flag = words.findIndex(
        (next, index) => index > at && /^-[A-Za-z]*c[A-Za-z]*$/.test(next),
      );
      if (flag > at && flag + 1 < words.length) {
        again.push(words[flag + 1]);
      }
    }
  });
  return again;
}

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

// `rm` given a recursive option and a forced one, found anywhere in the
// command, so that `sudo rm -rf` and `xargs rm -rf` are read too. Each is
// a short option, alone or among others (`-rf`, `-fr`, `-r -f`, `-Rf`), or
// a long one, whole or shortened as GNU `rm` takes it (`--recursive`,
// `--rec`; `--force`, `--f`). Case is not looked at, so that PowerShell's
// `rm -Recurse -Force` and `rm -r -Force`, where `rm` is `Remove-Item`, are
// read as well.
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
    const option = word.toLowerCase();
    if (option.startsWith("--")) {
      const name = option.slice(2);
      recursive ||= name !== "" && "recursive".startsWith(name);
      force ||= name !== "" && "force".startsWith(name);
    } else if (/^-[a-z]+$/.test(option)) {
      recursive ||= option.includes("r");
      force ||= option.includes("f");
    }
  }
  return recursive && force;
}

// git's own options that take the next word as their value.
const GIT_VALUED = [
  "-C",
  "-c",
  "--git-dir",
  "--work-tree",
  "--namespace",
  "--config-env",
  "--attr-source",
];

// `git push` with `--force`, `--mirror` (which overwrites every branch of
// the remote), `-f` (alone or among other short options, as in `-uf`), or a
// refspec that begins with `+`, which forces that one branch; or a push
// whose `-c` sets a `push` refspec that begins with `+`.
// `--force-with-lease` and `--force-if-includes` are let through. It is
// read from each word that names `git`, so that `sudo git push --force` is
// read too, and `sudo -u git git push --force`, where the first is sudo's
// user.
function pushesForcefully(words) {
  return words.some((word, at) => program(word) === "git" && pushesForcefullyAt(words, at));
}

// Whether the `git` at `at` in `words` is a forced push, as above.
function pushesForcefullyAt(words, at) {
  // git's own options come before its subcommand, each of `GIT_VALUED`
  // with its value after it.
  let forcedByConfiguration = false;
  let next = at + 1;
  while (next < words.length && words[next].startsWith("-")) {
    if (GIT_VALUED.includes(words[next])) {
      if (words[next] === "-c") {
        forcedByConfiguration ||= /\.push=\+/i.test(words[next + 1] ?? "");
      }
      next += 2;
    } else {
      next += 1;
    }
  }
  if (words[next] !== "push") {
    return false;
  }
  return (
    forcedByConfiguration ||
    words
      .slice(next + 1)
      .some(
        (word) =>
          word === "--force" ||
          word === "--mirror" ||
          (/^-[A-Za-z]+$/.test(word) && word.includes("f")) ||
          word.startsWith("+"),
      )
  );
}
