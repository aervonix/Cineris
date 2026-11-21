//! The help, written by hand (decision 0011) and printed in the name the
//! program was started with: `cineris` or `cin` (decision 0038).

use std::fmt::Write as _;

use crate::args::{FLAGS, Topic};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The help of `topic`, for a program started as `name`.
pub(crate) fn help(topic: Topic, name: &str) -> String {
    match topic {
        Topic::Main => main(name),
        Topic::Sessions => format!(
            "\
Usage: {name} sessions list [--all] [--limit N] [--json]

List the stored sessions of this workspace, newest first.

Options:
  --all        Every workspace's sessions
  --limit N    At most N sessions
  --json       One JSON object per session, a line each

Continue one with `{name} -p \"…\" --resume ID`.
"
        ),
        Topic::Config => format!(
            "\
Usage: {name} config <COMMAND>

Commands:
  show [--origin] [--layer NAME]  Print the configuration in effect here
  check                           Check every layer; exit 2 on any problem
  defaults                        Print the built-in defaults

Options:
  --origin        With show: where each value comes from
  --layer NAME    With show: one layer as it was read: default, managed,
                  user, project, project-local, profile, environment or
                  command-line
  --profile NAME  Load with the profile NAME
  -c KEY=VALUE    Set a key, as a run would
"
        ),
        Topic::Permissions => format!(
            "\
Usage: {name} permissions <COMMAND>

Commands:
  list         This project's trust, and the approvals stored for it
  revoke ID…   Remove stored approvals, by the ids that list shows
  trust        Trust this project, or approve the change of its rules;
               it shows the project's extensions and hooks, if it has
               any, and asks whether to approve them too
  untrust      Take this project's trust away
  forget       Forget this project's trust, or that you said never to
               trust it: the next session there asks again

Options:
  --with-extensions  With trust, where there is no terminal to ask at:
                     approve the extensions and hooks it shows. At a
                     terminal the question decides, and this flag is
                     ignored

Trusting a project lets its configuration choose a model, allow what its
rules allow, and run what it defines. A change to its permission rules sets
them aside again until it is approved with `{name} permissions trust`. A
project you said never to trust is not asked about; `trust` trusts it.
"
        ),
        Topic::Attach => format!(
            "\
Usage: {name} attach [--socket PATH] [--continue | --resume ID] [PROMPT]

Run the interactive application on a server that `{name} serve --socket`
started, with no core of its own: the server runs the sessions, and they go
on when you leave, for you to attach to again.

A new session for this folder, or with --continue its latest, or with
--resume a session by its id. A session running another client's turn is
watched as it runs; a prompt sent then steers it.

The server is reached only once it is shown to be yours: on Unix a socket of
yours in a folder others cannot write in, and a server running as you; on
Windows a pipe you own, at an integrity level of medium or above.

Options:
  --socket PATH        Where the server listens (default: its own default
                       place, as `{name} serve --socket` chooses it)
  --continue           Open this folder's latest session
  --resume ID          Open the session ID
  --log-level LEVEL    What the log file takes
  -h, --help           Print this help
"
        ),
        Topic::Serve => format!(
            "\
Usage: {name} serve --stdio [OPTIONS]
       {name} serve --socket [PATH] [OPTIONS]

Serve the RPC protocol: JSON-RPC 2.0, one message a line.

With --stdio, to one client, the process that started it, on standard input
and output. Standard output carries nothing else; the log goes to standard
error and the log file. The server ends when standard input ends.

With --socket, to several clients at once, over a local socket that only
you can open (a named pipe on Windows), by default in your runtime
directory. A session goes on when its client leaves, for `{name} attach` or
another client to take up.

Either ends when a client asks it to (`shutdown`) or at a signal; a second
signal ends it at once.

Options:
  --stdio              Serve over standard input and output
  --socket [PATH]      Serve over a local socket, at PATH or the default
  --profile NAME       Load configuration with the profile NAME
  -c KEY=VALUE         Set a configuration key, as a run would
  --model MODEL        The main model of the sessions it starts
  --effort LEVEL       Their reasoning level
  --speed TIER         Their speed tier
  --permission-mode MODE
                       Their permission mode
  --allow RULE         Allow what a rule matches, in every session
  --deny RULE          Deny what a rule matches, in every session
  --trust-project      Treat the projects of its sessions as trusted
  --system-append TEXT Add text to the end of the system prompt
  --log-level LEVEL    What the log file takes
  -q, --quiet          Write nothing to standard error
  -v, --verbose        Write more to standard error

The protocol is in docs/architecture/11-rpc-protocol.md; its schema and
TypeScript declarations are in docs/protocol/.
"
        ),
        Topic::Project => format!(
            "\
Usage: {name} project [COMMAND]

What the harness knows about the project of this directory: its build,
test and check commands, its conventions, its flaky tests and facts.

Commands:
  list           The entries (the default). Options: --all adds the ones
                 that are turned off, --kind KIND keeps one kind, --json
                 prints the same choice as one line of JSON
  accept ID…     Put a proposal to use, or turn an entry on again. For a
                 command it is also what lets it run without asking, once
                 the project is trusted
  accept --shared
                 Accept every entry of the repository's file that waits
  disable ID…    Take an entry out of use, and keep it
  delete ID…     Remove an entry of your own store
  promote ID…    Move an entry of yours to the repository's file
  edit ID TEXT   Replace the text of an entry of your own store
  add KIND TEXT  Write an entry: KIND is command, convention, flaky or
                 fact. A command takes --purpose build|test|check|format|run
                 and --command LINE, and may take --cwd DIR and --read-only;
                 a flaky test may name its command the same way
  path           Where the two files are

An ID is given as the listing shows it. One of the repository's file,
which is long, may be cut short to a start that only one of them has.

A command the harness saw succeed twice is listed as \"active, asks\": the
model is told of it, and it still asks before it runs. `{name} project
accept` is what lets it run without asking, in a project you trust, and
only a line that is one plain command: the line as it is written, wherever
in the workspace it runs. An entry of the repository's file,
.cineris/profile.toml, is a proposal until you accept it, whoever wrote it;
a change to it is a proposal again.
"
        ),
        Topic::Usage => format!(
            "\
Usage: {name} usage [--range R] [--by G] [--project KEY] [--json] [--refresh]

What was used and what it cost, from the statistics on this machine, with
where the turns' time went and the latest limits each provider reported.
Nothing is sent anywhere, unless --refresh is given.

Options:
  --range R      today, week, month (the default) or all
  --by G         Group by provider, model, project, source or speed: once
                 for each, or several joined by commas, outermost first
                 (provider,model by default)
  --project KEY  One project's usage
  --json         The report as one JSON object
  --refresh      First ask each provider in `usage.refresh` (your own
                 configuration) for its figures, at its documented usage
                 endpoint, with its own key. Off when the list is empty.
"
        ),
        Topic::Stats => format!(
            "\
Usage: {name} stats [show [--json] | reset]

The local statistics: what is stored, where, and how large.

Commands:
  show    What is stored (the default)
  reset   Delete every statistics file

Statistics never leave this machine. Set `stats.enabled = false` to stop
recording them; every feature then works on its defaults.
"
        ),
        Topic::Models => format!(
            "\
Usage: {name} models

List the models of the catalog, the built-in one and the user's own, with
the provider that serves each and its context window.
"
        ),
        Topic::Auth => format!(
            "\
Usage: {name} auth <COMMAND>

Commands:
  status            Where each provider's key comes from (never the key)
  login PROVIDER    Store a key, read from standard input
  logout PROVIDER   Remove a stored key

Example:
  {name} auth login anthropic < ~/anthropic-key.txt
"
        ),
    }
}

fn main(name: &str) -> String {
    let mut text = format!(
        "\
Cineris {VERSION}: a terminal agent harness.

Usage: {name} [OPTIONS] [PROMPT]…
       {name} -p [OPTIONS] [PROMPT]…
       {name} <COMMAND>

With -p, or with a prompt when standard output is not a terminal, the
prompt is run to completion headless: the result goes to standard output
and everything else to standard error. Piped input is added before the
prompt, as context.

Commands:
  sessions      List stored sessions
  config        Show or check the configuration
  permissions   Stored approvals and project trust
  auth          Provider keys
  models        The models of the catalog
  usage         What was used, what it cost, and where the time went
  stats         The local statistics: show or reset
  project       What is known about this project: review, accept, turn off
  serve         Serve the RPC protocol to another program
  attach        Run the interactive application on a running server

Options:
"
    );
    let width = FLAGS.iter().map(|flag| left(flag).len()).max().unwrap_or(0);
    for flag in FLAGS {
        let _ = writeln!(text, "  {:<width$}  {}", left(flag), flag.help);
    }
    let _ = write!(
        text,
        "
Exit codes: 0 completed, 1 internal error, 2 usage or configuration,
3 authentication, 4 provider or network, 5 the model stopped, 6 permission
denied, 7 limit reached, 8 context too large, 130 interrupted, 143
terminated.
"
    );
    text
}

fn left(flag: &crate::args::Flag) -> String {
    match flag.value {
        Some(value) => format!("{} {value}", flag.names),
        None => flag.names.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_uses_the_name_it_is_given() {
        for topic in [
            Topic::Main,
            Topic::Sessions,
            Topic::Config,
            Topic::Permissions,
            Topic::Auth,
            Topic::Models,
            Topic::Usage,
            Topic::Stats,
            Topic::Project,
            Topic::Serve,
            Topic::Attach,
        ] {
            let as_cin = help(topic, "cin");
            assert!(as_cin.contains("Usage: cin "), "{topic:?}");
            assert!(!as_cin.contains("cineris "), "{topic:?}: {as_cin}");
            assert!(
                help(topic, "cineris").contains("Usage: cineris "),
                "{topic:?}"
            );
        }
    }

    #[test]
    fn every_flag_is_in_the_help() {
        let text = help(Topic::Main, "cineris");
        for flag in FLAGS {
            assert!(text.contains(flag.names), "{}", flag.names);
        }
        assert!(!text.contains(crate::args::GUARD));
        assert!(!text.contains(crate::args::SANDBOX));
    }
}
