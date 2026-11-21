//! The command line, read into what to do (decision 0011: `lexopt`, and the
//! matching, the help and the errors written here).
//!
//! The grammar is small. A first word that names a subcommand
//! (`sessions`, `config`, `permissions`, `auth`) chooses it, and the rest is
//! that subcommand's. Anything else is a run: flags, and words that are the
//! prompt. A run is headless with `-p`, or when a prompt is given and
//! standard output is not a terminal; otherwise it is the interactive
//! application, which is not built yet (`docs/architecture/10-headless.md`,
//! "Invocation").
//!
//! [`FLAGS`] lists every flag of a run once. The help is printed from it,
//! and a test parses every entry, so the two cannot disagree.

use std::ffi::OsString;

use lexopt::{Arg, Parser, ValueExt};

/// The hidden subcommand whose body is the guard of process trees
/// (`cineris_sandbox::tree::run_guard`). The core starts the harness's own
/// executable with it. It is not in the help.
pub(crate) const GUARD: &str = "__tree-guard";

/// The hidden subcommand whose body is the sandbox launcher
/// (`cineris_sandbox::launch::run`). The core starts the harness's own
/// executable with it, the policy on its standard input and the command
/// after `--`. It is not in the help.
pub(crate) const SANDBOX: &str = "__sandbox";

/// A flag of a run, as the help shows it.
pub(crate) struct Flag {
    /// The names: `-p, --print`.
    pub(crate) names: &'static str,
    /// What it takes, if anything: `MODEL`.
    pub(crate) value: Option<&'static str>,
    /// One line of help.
    pub(crate) help: &'static str,
}

const fn flag(names: &'static str, value: Option<&'static str>, help: &'static str) -> Flag {
    Flag { names, value, help }
}

/// Every flag a run takes, in the order the help shows them.
pub(crate) const FLAGS: &[Flag] = &[
    flag(
        "-p, --print",
        None,
        "Run headless: the prompt is the words given, standard input, or both",
    ),
    flag(
        "--output",
        Some("FORMAT"),
        "text (the default), json, or jsonl: a line per event",
    ),
    flag(
        "--include-deltas",
        None,
        "With jsonl: also write streaming deltas",
    ),
    flag(
        "--continue",
        None,
        "Continue the latest session of this workspace",
    ),
    flag("--resume", Some("ID"), "Continue the session ID"),
    flag(
        "--fork",
        Some("ID"),
        "Start a new session from a copy of the session ID",
    ),
    flag("--no-session", None, "Keep the session in memory: no log"),
    flag(
        "--fullscreen",
        None,
        "Draw the interface on the whole window, for this run",
    ),
    flag("--model", Some("MODEL"), "The main model, for this run"),
    flag(
        "--effort",
        Some("LEVEL"),
        "The reasoning level: minimal, low, medium, high, max, or the model's own",
    ),
    flag(
        "--speed",
        Some("TIER"),
        "The speed tier: standard, or fast where the model offers it",
    ),
    flag(
        "--permission-mode",
        Some("MODE"),
        "read-only, supervised, autonomous or unrestricted",
    ),
    flag(
        "--allow",
        Some("RULE"),
        "Allow what a rule matches, for this run: 'exec:cargo test *'",
    ),
    flag(
        "--deny",
        Some("RULE"),
        "Deny what a rule matches, for this run: 'write src/generated/**'",
    ),
    flag(
        "--trust-project",
        None,
        "Treat the project as trusted, for this run",
    ),
    flag(
        "--fail-on-denial",
        None,
        "End the run at the first call nobody can approve (exit code 6)",
    ),
    flag(
        "--max-requests",
        Some("N"),
        "Stop after N requests to a model (exit code 7)",
    ),
    flag(
        "--max-cost",
        Some("USD"),
        "Stop when the run has cost this many US dollars (exit code 7)",
    ),
    flag(
        "--max-time",
        Some("DURATION"),
        "Stop after this long, as in 90s or 10m (exit code 7)",
    ),
    flag(
        "--system-append",
        Some("TEXT"),
        "Add text to the end of the system prompt",
    ),
    flag(
        "--profile",
        Some("NAME"),
        "Load configuration with the profile NAME",
    ),
    flag(
        "-c",
        Some("KEY=VALUE"),
        "Set a configuration key for this run: -c retry.attempts=3",
    ),
    flag(
        "--log-level",
        Some("LEVEL"),
        "What the log file takes: error, warn, info, debug, trace",
    ),
    flag("-q, --quiet", None, "Write nothing to standard error"),
    flag(
        "-v, --verbose",
        None,
        "Write more to standard error: what each step does",
    ),
    flag("-h, --help", None, "Print this help"),
    flag("-V, --version", None, "Print the version"),
];

/// What the command line asks for.
#[derive(Debug, PartialEq)]
pub(crate) enum Command {
    /// The help of the program, or of a subcommand.
    Help(Topic),
    /// The version.
    Version,
    /// A run: headless, or the interactive application.
    Run(Box<Run>),
    /// `cineris sessions …`
    Sessions(SessionsCommand),
    /// `cineris config …`
    Config(ConfigCommand),
    /// `cineris permissions …`
    Permissions(PermissionsCommand),
    /// `cineris auth …`
    Auth(AuthCommand),
    /// `cineris models`
    Models,
    /// `cineris usage …`
    Usage(UsageCommand),
    /// `cineris stats …`
    Stats(StatsCommand),
    /// `cineris project …`
    Project(ProjectCommand),
    /// `cineris serve …`: the RPC server (step 27).
    Serve(Box<Serve>),
    /// `cineris attach …`: the interactive application on a running server
    /// (step 27).
    Attach(Box<Attach>),
}

/// `cineris attach [--socket PATH] [--continue | --resume ID] [PROMPT]`: the
/// interactive application on a running `serve --socket`, with no core of
/// its own (`docs/steps/27/answered.md`, point 5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Attach {
    /// Where the server listens: its default place when not given.
    pub(crate) socket: Option<String>,
    /// The session to open: a new one, the workspace's latest, or one by
    /// its id.
    pub(crate) choice: Choice,
    /// A prompt to send once the session is open.
    pub(crate) prompt: Option<String>,
    /// `--log-level`: what the log file takes.
    pub(crate) log_level: Option<String>,
}

/// `cineris serve --stdio [FLAGS]`: the RPC server, and the flags of a run
/// that set the core up rather than one run (`docs/steps/27/answered.md`,
/// point 4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Serve {
    /// `--stdio`: one client, the process that started the server, over
    /// standard input and output.
    pub(crate) stdio: bool,
    /// `--socket [PATH]`: several clients, over a local socket (a named
    /// pipe on Windows); `Some(None)` at the default place.
    pub(crate) socket: Option<Option<String>>,
    /// The core's flags, read as a run reads them. The ones only a run
    /// takes are refused by name ([`Serve::run_only`]).
    pub(crate) run: Run,
}

impl Serve {
    /// The first flag given that only a run takes: a server has no prompt,
    /// no one session and no one turn.
    pub(crate) fn run_only(&self) -> Option<&'static str> {
        let run = &self.run;
        let limit = |key: &str| {
            run.common
                .settings
                .iter()
                .find(|(set, _, flag)| set == key && flag.is_some())
                .is_some()
        };
        if run.print {
            Some("-p")
        } else if run.prompt.is_some() {
            Some("a prompt")
        } else if run.format != Format::Text {
            Some("--output")
        } else if run.include_deltas {
            Some("--include-deltas")
        } else if run.choice != Choice::New {
            Some(match run.choice {
                Choice::Continue => "--continue",
                Choice::Resume(_) => "--resume",
                _ => "--fork",
            })
        } else if run.no_session {
            Some("--no-session")
        } else if run.fail_on_denial {
            Some("--fail-on-denial")
        } else if limit("limits.turn.requests") {
            Some("--max-requests")
        } else if limit("limits.turn.cost") {
            Some("--max-cost")
        } else if limit("limits.turn.time") {
            Some("--max-time")
        } else {
            None
        }
    }
}

/// `cineris project …`: what the harness knows about this project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectCommand {
    /// `list [--all] [--kind KIND] [--json]`, or nothing.
    List {
        /// `--all`: the entries that are turned off too.
        all: bool,
        /// `--kind`: `command`, `convention`, `flaky` or `fact`.
        kind: Option<String>,
        /// `--json`
        json: bool,
    },
    /// `accept ID…`, `accept --shared`, `disable ID…`, `delete ID…`,
    /// `promote ID…`.
    Change {
        /// `accept`, `disable`, `delete` or `promote`.
        action: String,
        /// The ids, or the starts of ids.
        ids: Vec<String>,
        /// `accept --shared`: every entry of the repository's file that
        /// waits for this user.
        shared: bool,
    },
    /// `edit ID TEXT`
    Edit { id: String, text: String },
    /// `add KIND TEXT [--purpose P --command LINE [--cwd DIR] [--read-only]]`
    Add {
        kind: String,
        text: String,
        purpose: Option<String>,
        command: Option<String>,
        cwd: Option<String>,
        read_only: bool,
    },
    /// `path`: where the two files are.
    Path,
}

/// `cineris usage [--range R] [--by G]… [--project KEY] [--json] [--refresh]`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsageCommand {
    /// `--range`: `today`, `week`, `month` (the default) or `all`.
    pub(crate) range: String,
    /// `--by`, outermost first: `provider`, `model`, `project`, `source`,
    /// `speed`.
    pub(crate) by: Vec<String>,
    /// `--project`: one project's key.
    pub(crate) project: Option<String>,
    /// `--json`
    pub(crate) json: bool,
    /// `--refresh`: ask the providers in `usage.refresh` first.
    pub(crate) refresh: bool,
}

/// `cineris stats …`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StatsCommand {
    /// `show [--json]`, or nothing.
    Show { json: bool },
    /// `reset`
    Reset,
}

/// Whose help to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Topic {
    Main,
    Sessions,
    Config,
    Permissions,
    Auth,
    Models,
    Usage,
    Stats,
    Project,
    Serve,
    Attach,
}

/// What a headless run writes on standard output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Format {
    /// The assistant's last message, as plain text.
    #[default]
    Text,
    /// One JSON object when the run ends.
    Json,
    /// A JSON object per line, as things happen.
    Jsonl,
}

/// Which session a run is in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Choice {
    /// A new one.
    #[default]
    New,
    /// The latest of this workspace.
    Continue,
    /// This one.
    Resume(String),
    /// A copy of this one.
    Fork(String),
}

/// Options every command that loads configuration takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Common {
    /// `--profile`
    pub(crate) profile: Option<String>,
    /// `-c key=value`, and the dedicated flags, in order, as
    /// `(key, value, flag)`.
    pub(crate) settings: Vec<(String, String, Option<String>)>,
}

/// A run, headless or not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Run {
    /// `-p`
    pub(crate) print: bool,
    /// The words given as the prompt, joined by spaces.
    pub(crate) prompt: Option<String>,
    pub(crate) format: Format,
    pub(crate) include_deltas: bool,
    pub(crate) choice: Choice,
    pub(crate) no_session: bool,
    pub(crate) allow: Vec<String>,
    pub(crate) deny: Vec<String>,
    pub(crate) trust_project: bool,
    pub(crate) fail_on_denial: bool,
    pub(crate) system_append: Option<String>,
    pub(crate) log_level: Option<String>,
    pub(crate) quiet: bool,
    pub(crate) verbose: bool,
    pub(crate) common: Common,
}

/// `cineris sessions …`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionsCommand {
    /// `list [--all] [--limit N] [--json]`
    List {
        all: bool,
        limit: Option<u32>,
        json: bool,
    },
}

/// `cineris config …`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConfigCommand {
    /// `show [--origin] [--layer NAME]`
    Show {
        origin: bool,
        layer: Option<String>,
        common: Common,
    },
    /// `check`
    Check { common: Common },
    /// `defaults`
    Defaults,
}

/// `cineris permissions …`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PermissionsCommand {
    /// `list`: this project's stored approvals, and its trust.
    List,
    /// `revoke ID…`
    Revoke { ids: Vec<String> },
    /// `trust [--with-extensions]`: trust this project, or approve the
    /// change of its rules; and its extensions, shown, on a yes at a
    /// terminal, or with `--with-extensions` where there is none to ask at
    /// (since step 28).
    Trust {
        /// `--with-extensions`: approve the extensions shown with no
        /// terminal to ask at.
        with_extensions: bool,
    },
    /// `untrust`
    Untrust,
    /// `forget`: forget trust and never alike, so the next start asks
    /// (since step 23).
    Forget,
}

/// `cineris auth …`
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthCommand {
    /// `status`
    Status,
    /// `login PROVIDER`: the key is read from standard input.
    Login { provider: String },
    /// `logout PROVIDER`
    Logout { provider: String },
}

/// A mistake on the command line, said to the person who typed it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct UsageError(pub(crate) String);

impl From<lexopt::Error> for UsageError {
    fn from(error: lexopt::Error) -> Self {
        Self(match error {
            lexopt::Error::MissingValue {
                option: Some(option),
            } => format!("'{option}' needs a value"),
            lexopt::Error::UnexpectedOption(option) => format!("unexpected argument '{option}'"),
            lexopt::Error::UnexpectedArgument(argument) => {
                format!("unexpected argument '{}'", argument.to_string_lossy())
            }
            lexopt::Error::NonUnicodeValue(value) => {
                format!("'{}' is not valid text", value.to_string_lossy())
            }
            other => other.to_string(),
        })
    }
}

fn unexpected(argument: &Arg<'_>) -> UsageError {
    UsageError(format!("unexpected argument '{}'", shown(argument)))
}

fn shown(argument: &Arg<'_>) -> String {
    match argument {
        Arg::Short(short) => format!("-{short}"),
        Arg::Long(long) => format!("--{long}"),
        Arg::Value(value) => value.to_string_lossy().into_owned(),
    }
}

fn text(parser: &mut Parser) -> Result<String, UsageError> {
    Ok(parser.value()?.string()?)
}

/// Reads the arguments after the program's name.
pub(crate) fn parse(arguments: Vec<OsString>) -> Result<Command, UsageError> {
    let mut parser = Parser::from_args(arguments.clone());
    if let Some(first) = arguments.first().and_then(|first| first.to_str()) {
        let topic = match first {
            "sessions" => Some(Topic::Sessions),
            "config" => Some(Topic::Config),
            "permissions" => Some(Topic::Permissions),
            "auth" => Some(Topic::Auth),
            "models" => Some(Topic::Models),
            "usage" => Some(Topic::Usage),
            "stats" => Some(Topic::Stats),
            "project" => Some(Topic::Project),
            "serve" => Some(Topic::Serve),
            "attach" => Some(Topic::Attach),
            _ => None,
        };
        if let Some(topic) = topic {
            // Past the subcommand's own name.
            let _ = parser.next()?;
            return subcommand(topic, &mut parser);
        }
    }
    run(&mut parser)
}

fn run(parser: &mut Parser) -> Result<Command, UsageError> {
    let mut run = Run::default();
    let mut words: Vec<String> = Vec::new();
    let mut choices = 0;
    while let Some(argument) = parser.next()? {
        let argument = Owned::from(argument);
        if let Some(command) = run_flag(parser, &mut run, &mut words, &mut choices, argument)? {
            return Ok(command);
        }
    }
    finish_run(run, &words, choices).map(|run| Command::Run(Box::new(run)))
}

/// `cineris serve`: `--stdio`, and a run's flags.
fn serve(parser: &mut Parser) -> Result<Command, UsageError> {
    let mut serve = Serve::default();
    let mut words: Vec<String> = Vec::new();
    let mut choices = 0;
    // `--socket` may take the word after it as its place.
    let mut place_may_follow = false;
    while let Some(argument) = parser.next()? {
        let follows = std::mem::take(&mut place_may_follow);
        match argument {
            Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(Topic::Serve)),
            Arg::Long("stdio") => serve.stdio = true,
            Arg::Long("socket") => {
                let attached = parser
                    .optional_value()
                    .map(|value| value.string())
                    .transpose()?;
                place_may_follow = attached.is_none();
                serve.socket = Some(attached);
            }
            Arg::Value(value) if follows => {
                serve.socket = Some(Some(value.string()?));
            }
            argument => {
                let argument = Owned::from(argument);
                if let Some(command) =
                    run_flag(parser, &mut serve.run, &mut words, &mut choices, argument)?
                {
                    return Ok(command);
                }
            }
        }
    }
    serve.run = finish_run(serve.run, &words, choices)?;
    match (serve.stdio, &serve.socket) {
        (false, None) => {
            return Err(UsageError(
                "serve needs --stdio (one client, the process that started it) or --socket \
                 [PATH] (several, over a local socket)"
                    .to_owned(),
            ));
        }
        (true, Some(_)) => {
            return Err(UsageError(
                "serve takes --stdio or --socket, not both".to_owned(),
            ));
        }
        _ => {}
    }
    Ok(Command::Serve(Box::new(serve)))
}

/// `cineris attach`: where the server is, the session to open, and a prompt.
fn attach(parser: &mut Parser) -> Result<Command, UsageError> {
    let mut attach = Attach::default();
    let mut words: Vec<String> = Vec::new();
    let mut choices = 0;
    while let Some(argument) = parser.next()? {
        match argument {
            Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(Topic::Attach)),
            Arg::Long("socket") => attach.socket = Some(text(parser)?),
            Arg::Long("continue") => {
                choices += 1;
                attach.choice = Choice::Continue;
            }
            Arg::Long("resume") => {
                choices += 1;
                attach.choice = Choice::Resume(text(parser)?);
            }
            Arg::Long("log-level") => attach.log_level = Some(text(parser)?),
            Arg::Value(value) => words.push(value.string()?),
            other => return Err(unexpected(&other)),
        }
    }
    if choices > 1 {
        return Err(UsageError(
            "attach takes --continue or --resume ID, not both".to_owned(),
        ));
    }
    if !words.is_empty() {
        attach.prompt = Some(words.join(" "));
    }
    Ok(Command::Attach(Box::new(attach)))
}

/// An argument as lexopt reads it, holding its own text, so that the parser
/// can be asked for a flag's value while the flag is looked at.
enum Owned {
    Short(char),
    Long(String),
    Value(OsString),
}

impl From<Arg<'_>> for Owned {
    fn from(argument: Arg<'_>) -> Self {
        match argument {
            Arg::Short(short) => Self::Short(short),
            Arg::Long(long) => Self::Long(long.to_owned()),
            Arg::Value(value) => Self::Value(value),
        }
    }
}

/// One argument of a run. A command when the argument is one by itself
/// (`--help`, `--version`).
fn run_flag(
    parser: &mut Parser,
    run: &mut Run,
    words: &mut Vec<String>,
    choices: &mut usize,
    argument: Owned,
) -> Result<Option<Command>, UsageError> {
    let long;
    let argument = match argument {
        Owned::Short(short) => Arg::Short(short),
        Owned::Long(text) => {
            long = text;
            Arg::Long(&long)
        }
        Owned::Value(value) => Arg::Value(value),
    };
    let set = |run: &mut Run, key: &str, value: String, flag: &str| {
        run.common
            .settings
            .push((key.to_owned(), value, Some(flag.to_owned())));
    };
    match argument {
        Arg::Short('h') | Arg::Long("help") => return Ok(Some(Command::Help(Topic::Main))),
        Arg::Short('V') | Arg::Long("version") => return Ok(Some(Command::Version)),
        Arg::Short('p') | Arg::Long("print") => {
            run.print = true;
            // `--print=TEXT`: the prompt attached to the flag.
            if let Some(attached) = parser.optional_value() {
                words.push(attached.string()?);
            }
        }
        Arg::Long("output") => {
            run.format = match text(parser)?.as_str() {
                "text" => Format::Text,
                "json" => Format::Json,
                "jsonl" => Format::Jsonl,
                other => {
                    return Err(UsageError(format!(
                        "'{other}' is not an output format: use text, json or jsonl"
                    )));
                }
            };
        }
        Arg::Long("include-deltas") => run.include_deltas = true,
        Arg::Long("continue") => {
            *choices += 1;
            run.choice = Choice::Continue;
        }
        Arg::Long("resume") => {
            *choices += 1;
            run.choice = Choice::Resume(text(parser)?);
        }
        Arg::Long("fork") => {
            *choices += 1;
            run.choice = Choice::Fork(text(parser)?);
        }
        Arg::Long("no-session") => run.no_session = true,
        Arg::Long("fullscreen") => set(
            run,
            "tui.presenter",
            "fullscreen".to_owned(),
            "--fullscreen",
        ),
        Arg::Long("model") => set(run, "model.main", text(parser)?, "--model"),
        Arg::Long("effort") => set(run, "model.effort", text(parser)?, "--effort"),
        Arg::Long("speed") => set(run, "model.speed", text(parser)?, "--speed"),
        Arg::Long("permission-mode") => {
            let mode = text(parser)?;
            set(run, "permissions.mode", mode, "--permission-mode");
        }
        Arg::Long("allow") => run.allow.push(text(parser)?),
        Arg::Long("deny") => run.deny.push(text(parser)?),
        Arg::Long("trust-project") => run.trust_project = true,
        Arg::Long("fail-on-denial") => run.fail_on_denial = true,
        Arg::Long("max-requests") => {
            let value = text(parser)?;
            set(run, "limits.turn.requests", value, "--max-requests");
        }
        Arg::Long("max-cost") => {
            let value = text(parser)?;
            set(run, "limits.turn.cost", value, "--max-cost");
        }
        Arg::Long("max-time") => {
            let value = text(parser)?;
            set(run, "limits.turn.time", value, "--max-time");
        }
        Arg::Long("system-append") => run.system_append = Some(text(parser)?),
        Arg::Long("profile") => run.common.profile = Some(text(parser)?),
        Arg::Short('c') => {
            let given = text(parser)?;
            let Some((key, value)) = given.split_once('=') else {
                return Err(UsageError(
                    "-c takes KEY=VALUE, as in -c retry.attempts=3".to_owned(),
                ));
            };
            run.common
                .settings
                .push((key.trim().to_owned(), value.to_owned(), None));
        }
        Arg::Long("log-level") => run.log_level = Some(text(parser)?),
        Arg::Short('q') | Arg::Long("quiet") => run.quiet = true,
        Arg::Short('v') | Arg::Long("verbose") => run.verbose = true,
        Arg::Value(word) => words.push(word.string()?),
        other => return Err(unexpected(&other)),
    }
    Ok(None)
}

/// The checks of a run as a whole, once every argument is read.
fn finish_run(mut run: Run, words: &[String], choices: usize) -> Result<Run, UsageError> {
    if choices > 1 {
        return Err(UsageError(
            "choose one of --continue, --resume and --fork".to_owned(),
        ));
    }
    if run.no_session && run.choice != Choice::New {
        return Err(UsageError(
            "--no-session starts a new session; it cannot be used with --continue, --resume \
             or --fork"
                .to_owned(),
        ));
    }
    if !words.is_empty() {
        run.prompt = Some(words.join(" "));
    }
    Ok(run)
}

fn subcommand(topic: Topic, parser: &mut Parser) -> Result<Command, UsageError> {
    if topic == Topic::Serve {
        return serve(parser);
    }
    if topic == Topic::Attach {
        return attach(parser);
    }
    if topic == Topic::Models {
        return match parser.next()? {
            None => Ok(Command::Models),
            Some(Arg::Short('h') | Arg::Long("help")) => Ok(Command::Help(topic)),
            Some(other) => Err(unexpected(&other)),
        };
    }
    if topic == Topic::Usage {
        return usage(parser);
    }
    if topic == Topic::Stats {
        let mut action = None;
        let mut json = false;
        while let Some(argument) = parser.next()? {
            match argument {
                Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(topic)),
                Arg::Long("json") => json = true,
                Arg::Value(value) if action.is_none() => action = Some(value.string()?),
                other => return Err(unexpected(&other)),
            }
        }
        return match action.as_deref() {
            None | Some("show") => Ok(Command::Stats(StatsCommand::Show { json })),
            Some("reset") if !json => Ok(Command::Stats(StatsCommand::Reset)),
            Some(other) => Err(UsageError(format!(
                "'{other}' is not a subcommand of 'stats'"
            ))),
        };
    }
    if topic == Topic::Project {
        return project(parser);
    }
    let Some(first) = parser.next()? else {
        return Ok(Command::Help(topic));
    };
    let action = match first {
        Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(topic)),
        Arg::Value(action) => action.string()?,
        other => return Err(unexpected(&other)),
    };
    let wrong = |action: &str| {
        UsageError(format!(
            "'{action}' is not a subcommand of '{}'",
            match topic {
                Topic::Sessions => "sessions",
                Topic::Config => "config",
                Topic::Permissions => "permissions",
                Topic::Auth => "auth",
                Topic::Models => "models",
                Topic::Usage => "usage",
                Topic::Stats => "stats",
                Topic::Project => "project",
                Topic::Serve => "serve",
                Topic::Attach => "attach",
                Topic::Main => "",
            }
        ))
    };
    match topic {
        Topic::Sessions => match action.as_str() {
            "list" => {
                let (mut all, mut limit, mut json) = (false, None, false);
                while let Some(argument) = parser.next()? {
                    match argument {
                        Arg::Long("all") => all = true,
                        Arg::Long("json") => json = true,
                        Arg::Long("limit") => {
                            let value = text(parser)?;
                            limit = Some(value.parse().map_err(|_| {
                                UsageError(format!("'{value}' is not a whole number"))
                            })?);
                        }
                        Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(topic)),
                        other => return Err(unexpected(&other)),
                    }
                }
                Ok(Command::Sessions(SessionsCommand::List {
                    all,
                    limit,
                    json,
                }))
            }
            other => Err(wrong(other)),
        },
        Topic::Config => {
            let (mut origin, mut layer, mut common) = (false, None, Common::default());
            while let Some(argument) = parser.next()? {
                match argument {
                    Arg::Long("origin") if action == "show" => origin = true,
                    Arg::Long("layer") if action == "show" => layer = Some(text(parser)?),
                    Arg::Long("profile") if action != "defaults" => {
                        common.profile = Some(text(parser)?);
                    }
                    Arg::Short('c') if action != "defaults" => {
                        let given = text(parser)?;
                        let Some((key, value)) = given.split_once('=') else {
                            return Err(UsageError("-c takes KEY=VALUE".to_owned()));
                        };
                        common
                            .settings
                            .push((key.trim().to_owned(), value.to_owned(), None));
                    }
                    Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(topic)),
                    other => return Err(unexpected(&other)),
                }
            }
            match action.as_str() {
                "show" => Ok(Command::Config(ConfigCommand::Show {
                    origin,
                    layer,
                    common,
                })),
                "check" => Ok(Command::Config(ConfigCommand::Check { common })),
                "defaults" => Ok(Command::Config(ConfigCommand::Defaults)),
                other => Err(wrong(other)),
            }
        }
        Topic::Permissions => {
            let mut ids = Vec::new();
            let mut with_extensions = false;
            while let Some(argument) = parser.next()? {
                match argument {
                    Arg::Value(id) if action == "revoke" => ids.push(id.string()?),
                    Arg::Long("with-extensions") if action == "trust" => with_extensions = true,
                    Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(topic)),
                    other => return Err(unexpected(&other)),
                }
            }
            match action.as_str() {
                "list" => Ok(Command::Permissions(PermissionsCommand::List)),
                "revoke" if ids.is_empty() => Err(UsageError(
                    "'permissions revoke' takes the ids that 'permissions list' shows".to_owned(),
                )),
                "revoke" => Ok(Command::Permissions(PermissionsCommand::Revoke { ids })),
                "trust" => Ok(Command::Permissions(PermissionsCommand::Trust {
                    with_extensions,
                })),
                "untrust" => Ok(Command::Permissions(PermissionsCommand::Untrust)),
                "forget" => Ok(Command::Permissions(PermissionsCommand::Forget)),
                other => Err(wrong(other)),
            }
        }
        Topic::Auth => {
            let mut provider = None;
            while let Some(argument) = parser.next()? {
                match argument {
                    Arg::Value(name) if provider.is_none() && action != "status" => {
                        provider = Some(name.string()?);
                    }
                    Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(topic)),
                    other => return Err(unexpected(&other)),
                }
            }
            let named = |provider: Option<String>| {
                provider
                    .ok_or_else(|| UsageError(format!("'auth {action}' takes the provider's name")))
            };
            match action.as_str() {
                "status" => Ok(Command::Auth(AuthCommand::Status)),
                "login" => Ok(Command::Auth(AuthCommand::Login {
                    provider: named(provider)?,
                })),
                "logout" => Ok(Command::Auth(AuthCommand::Logout {
                    provider: named(provider)?,
                })),
                other => Err(wrong(other)),
            }
        }
        Topic::Main
        | Topic::Models
        | Topic::Usage
        | Topic::Stats
        | Topic::Project
        | Topic::Serve
        | Topic::Attach => Err(wrong(&action)),
    }
}

/// `cineris project …`
fn project(parser: &mut Parser) -> Result<Command, UsageError> {
    let help = Ok(Command::Help(Topic::Project));
    let action = match parser.next()? {
        // With nothing after it, the listing: what one most often wants.
        None => {
            return Ok(Command::Project(ProjectCommand::List {
                all: false,
                kind: None,
                json: false,
            }));
        }
        Some(Arg::Short('h') | Arg::Long("help")) => return help,
        Some(Arg::Value(action)) => action.string()?,
        Some(other) => return Err(unexpected(&other)),
    };
    let (mut all, mut json, mut shared, mut read_only) = (false, false, false, false);
    let (mut kind, mut purpose, mut command, mut cwd) = (None, None, None, None);
    let mut words: Vec<String> = Vec::new();
    while let Some(argument) = parser.next()? {
        match argument {
            Arg::Short('h') | Arg::Long("help") => return help,
            Arg::Long("all") if action == "list" => all = true,
            Arg::Long("json") if action == "list" => json = true,
            Arg::Long("kind") if action == "list" => kind = Some(text(parser)?),
            Arg::Long("shared") if action == "accept" => shared = true,
            Arg::Long("purpose") if action == "add" => purpose = Some(text(parser)?),
            Arg::Long("command") if action == "add" => command = Some(text(parser)?),
            Arg::Long("cwd") if action == "add" => cwd = Some(text(parser)?),
            Arg::Long("read-only") if action == "add" => read_only = true,
            Arg::Value(word) => words.push(word.string()?),
            other => return Err(unexpected(&other)),
        }
    }
    let takes = |what: &str| UsageError(format!("'project {action}' takes {what}"));
    let kinds = ["command", "convention", "flaky", "fact"];
    let parsed = match action.as_str() {
        "list" => {
            if let Some(word) = words.first() {
                return Err(UsageError(format!("unexpected argument '{word}'")));
            }
            if let Some(kind) = &kind
                && !kinds.contains(&kind.as_str())
            {
                return Err(UsageError(format!(
                    "'{kind}' is not a kind of entry: use command, convention, flaky or fact"
                )));
            }
            ProjectCommand::List { all, kind, json }
        }
        "accept" | "disable" | "delete" | "promote" => {
            if shared != words.is_empty() {
                return Err(takes(if action == "accept" {
                    "the ids that 'project list' shows, or --shared for every entry of the \
                     repository's file"
                } else {
                    "the ids that 'project list' shows"
                }));
            }
            ProjectCommand::Change {
                action,
                ids: words,
                shared,
            }
        }
        "edit" => {
            let [id, text] = <[String; 2]>::try_from(words)
                .map_err(|_| takes("an id and the new text, as one argument"))?;
            ProjectCommand::Edit { id, text }
        }
        "add" => {
            let [kind, text] = <[String; 2]>::try_from(words)
                .map_err(|_| takes("a kind and the entry's text, as one argument"))?;
            if !kinds.contains(&kind.as_str()) {
                return Err(UsageError(format!(
                    "'{kind}' is not a kind of entry: use command, convention, flaky or fact"
                )));
            }
            match (kind.as_str(), command.is_some()) {
                ("command", false) => {
                    return Err(UsageError(
                        "a command entry takes --purpose and --command".to_owned(),
                    ));
                }
                ("convention" | "fact", true) => {
                    return Err(UsageError(format!(
                        "--command is for a command or a flaky entry, not a {kind}"
                    )));
                }
                _ => {}
            }
            if (purpose.is_some() || cwd.is_some() || read_only) && command.is_none() {
                return Err(UsageError(
                    "--purpose, --cwd and --read-only go with --command".to_owned(),
                ));
            }
            if command.is_some() {
                match purpose.as_deref() {
                    Some("build" | "test" | "check" | "format" | "run") => {}
                    Some(other) => {
                        return Err(UsageError(format!(
                            "'{other}' is not a purpose: use build, test, check, format or run"
                        )));
                    }
                    None => {
                        return Err(UsageError(
                            "--command takes --purpose: build, test, check, format or run"
                                .to_owned(),
                        ));
                    }
                }
            }
            ProjectCommand::Add {
                kind,
                text,
                purpose,
                command,
                cwd,
                read_only,
            }
        }
        "path" if words.is_empty() => ProjectCommand::Path,
        other => {
            return Err(UsageError(format!(
                "'{other}' is not a subcommand of 'project'"
            )));
        }
    };
    Ok(Command::Project(parsed))
}

/// `cineris usage …`
fn usage(parser: &mut Parser) -> Result<Command, UsageError> {
    let mut command = UsageCommand {
        range: "month".to_owned(),
        by: Vec::new(),
        project: None,
        json: false,
        refresh: false,
    };
    while let Some(argument) = parser.next()? {
        match argument {
            Arg::Short('h') | Arg::Long("help") => return Ok(Command::Help(Topic::Usage)),
            Arg::Long("range") => {
                let range = text(parser)?;
                if !["today", "week", "month", "all"].contains(&range.as_str()) {
                    return Err(UsageError(format!(
                        "'{range}' is not a range: use today, week, month or all (the open \
                         session's own figures are /usage's)"
                    )));
                }
                command.range = range;
            }
            Arg::Long("by") => {
                for group in text(parser)?.split(',') {
                    let group = group.trim();
                    if !["provider", "model", "project", "source", "speed"].contains(&group) {
                        return Err(UsageError(format!(
                            "'{group}' is not a grouping: use provider, model, project, source \
                             or speed"
                        )));
                    }
                    command.by.push(group.to_owned());
                }
            }
            Arg::Long("project") => command.project = Some(text(parser)?),
            Arg::Long("json") => command.json = true,
            Arg::Long("refresh") => command.refresh = true,
            other => return Err(unexpected(&other)),
        }
    }
    Ok(Command::Usage(command))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Command, UsageError> {
        parse(words.iter().map(OsString::from).collect())
    }

    fn run_of(words: &[&str]) -> Run {
        match parse_words(words) {
            Ok(Command::Run(run)) => *run,
            other => panic!("{words:?} is not a run: {other:?}"),
        }
    }

    #[test]
    fn every_flag_in_the_help_is_one_the_parser_takes() {
        for flag in FLAGS {
            for name in flag.names.split(", ") {
                let mut words = vec![name];
                let value = match flag.value {
                    Some("FORMAT") => Some("json"),
                    Some("KEY=VALUE") => Some("retry.attempts=3"),
                    Some("RULE") => Some("exec:cargo test *"),
                    Some(_) => Some("x"),
                    None => None,
                };
                words.extend(value);
                let parsed = parse_words(&words);
                assert!(parsed.is_ok(), "{words:?}: {parsed:?}");
            }
        }
    }

    #[test]
    fn a_print_run_takes_its_prompt_from_the_words_given() {
        let run = run_of(&["-p", "explain", "the", "failure"]);
        assert!(run.print);
        assert_eq!(run.prompt.as_deref(), Some("explain the failure"));
        let run = run_of(&["--print=fix it", "--output", "jsonl"]);
        assert_eq!(run.prompt.as_deref(), Some("fix it"));
        assert_eq!(run.format, Format::Jsonl);
        let run = run_of(&["-p", "--output=json"]);
        assert_eq!(run.prompt, None);
        assert_eq!(run.format, Format::Json);
    }

    #[test]
    fn the_flags_that_choose_a_setting_become_its_key() {
        let run = run_of(&[
            "-p",
            "x",
            "--model",
            "m",
            "--effort",
            "high",
            "--speed",
            "fast",
            "--permission-mode",
            "read-only",
            "--max-requests",
            "3",
            "--max-cost",
            "0.5",
            "--max-time",
            "90s",
            "-c",
            "retry.attempts=2",
        ]);
        let keys: Vec<(&str, &str)> = run
            .common
            .settings
            .iter()
            .map(|(key, value, _)| (key.as_str(), value.as_str()))
            .collect();
        assert_eq!(
            keys,
            [
                ("model.main", "m"),
                ("model.effort", "high"),
                ("model.speed", "fast"),
                ("permissions.mode", "read-only"),
                ("limits.turn.requests", "3"),
                ("limits.turn.cost", "0.5"),
                ("limits.turn.time", "90s"),
                ("retry.attempts", "2"),
            ]
        );
        assert_eq!(run.common.settings[2].2.as_deref(), Some("--speed"));
        assert_eq!(run.common.settings[7].2, None);
    }

    #[test]
    fn mistakes_are_usage_errors() {
        for words in [
            &["--no-such-flag"][..],
            &["--output", "xml"],
            &["--continue", "--resume", "s_1"],
            &["--no-session", "--continue"],
            &["--model"],
            &["-c", "no-equals-sign"],
            &["sessions", "drop"],
            &["auth", "login"],
            &["permissions", "revoke"],
            &["config", "show", "--json"],
        ] {
            assert!(parse_words(words).is_err(), "{words:?}");
        }
        assert_eq!(
            parse_words(&["--no-such-flag"]),
            Err(UsageError(
                "unexpected argument '--no-such-flag'".to_owned()
            ))
        );
    }

    #[test]
    fn project_reads_its_subcommands_and_says_what_each_takes() {
        let list = |all, kind: Option<&str>, json| {
            Ok(Command::Project(ProjectCommand::List {
                all,
                kind: kind.map(str::to_owned),
                json,
            }))
        };
        // With nothing after it, the listing.
        assert_eq!(parse_words(&["project"]), list(false, None, false));
        assert_eq!(parse_words(&["project", "list"]), list(false, None, false));
        assert_eq!(
            parse_words(&["project", "list", "--all", "--kind", "fact", "--json"]),
            list(true, Some("fact"), true)
        );
        for action in ["accept", "disable", "delete", "promote"] {
            assert_eq!(
                parse_words(&["project", action, "k_1", "s_0f"]),
                Ok(Command::Project(ProjectCommand::Change {
                    action: action.to_owned(),
                    ids: vec!["k_1".to_owned(), "s_0f".to_owned()],
                    shared: false,
                }))
            );
        }
        assert_eq!(
            parse_words(&["project", "accept", "--shared"]),
            Ok(Command::Project(ProjectCommand::Change {
                action: "accept".to_owned(),
                ids: Vec::new(),
                shared: true,
            }))
        );
        assert_eq!(
            parse_words(&["project", "edit", "k_2", "Tabs, not spaces."]),
            Ok(Command::Project(ProjectCommand::Edit {
                id: "k_2".to_owned(),
                text: "Tabs, not spaces.".to_owned(),
            }))
        );
        assert_eq!(
            parse_words(&[
                "project",
                "add",
                "command",
                "Runs the tests.",
                "--purpose",
                "test",
                "--command",
                "cargo test --workspace",
                "--cwd",
                "crates/app",
                "--read-only",
            ]),
            Ok(Command::Project(ProjectCommand::Add {
                kind: "command".to_owned(),
                text: "Runs the tests.".to_owned(),
                purpose: Some("test".to_owned()),
                command: Some("cargo test --workspace".to_owned()),
                cwd: Some("crates/app".to_owned()),
                read_only: true,
            }))
        );
        assert_eq!(
            parse_words(&["project", "add", "fact", "The docs are in docs/."]),
            Ok(Command::Project(ProjectCommand::Add {
                kind: "fact".to_owned(),
                text: "The docs are in docs/.".to_owned(),
                purpose: None,
                command: None,
                cwd: None,
                read_only: false,
            }))
        );
        assert_eq!(
            parse_words(&["project", "path"]),
            Ok(Command::Project(ProjectCommand::Path))
        );
        assert_eq!(
            parse_words(&["project", "--help"]),
            Ok(Command::Help(Topic::Project))
        );
        assert_eq!(
            parse_words(&["project", "accept", "-h"]),
            Ok(Command::Help(Topic::Project))
        );

        // What each takes, said to the person who typed it.
        for (words, says) in [
            (&["project", "accept"][..], "takes the ids"),
            (&["project", "accept", "k_1", "--shared"], "takes the ids"),
            (&["project", "disable"], "takes the ids"),
            (
                &["project", "disable", "--shared"],
                "unexpected argument '--shared'",
            ),
            (&["project", "edit", "k_1"], "takes an id and the new text"),
            (
                &["project", "edit", "k_1", "two", "words"],
                "takes an id and the new text",
            ),
            (
                &["project", "add", "fact"],
                "takes a kind and the entry's text",
            ),
            (
                &["project", "add", "rule", "x"],
                "'rule' is not a kind of entry",
            ),
            (
                &["project", "add", "command", "x"],
                "takes --purpose and --command",
            ),
            (
                &["project", "add", "command", "x", "--command", "make"],
                "--command takes --purpose",
            ),
            (
                &[
                    "project",
                    "add",
                    "command",
                    "x",
                    "--command",
                    "make",
                    "--purpose",
                    "ship",
                ],
                "'ship' is not a purpose",
            ),
            (
                &[
                    "project",
                    "add",
                    "fact",
                    "x",
                    "--command",
                    "make",
                    "--purpose",
                    "test",
                ],
                "not a fact",
            ),
            (
                &["project", "add", "fact", "x", "--cwd", "sub"],
                "go with --command",
            ),
            (
                &["project", "list", "--kind", "rule"],
                "'rule' is not a kind of entry",
            ),
            (&["project", "list", "k_1"], "unexpected argument 'k_1'"),
            (
                &["project", "list", "--shared"],
                "unexpected argument '--shared'",
            ),
            (&["project", "path", "x"], "'path' is not a subcommand"),
            (
                &["project", "forget"],
                "'forget' is not a subcommand of 'project'",
            ),
        ] {
            let Err(UsageError(message)) = parse_words(words) else {
                panic!("{words:?} was read");
            };
            assert!(message.contains(says), "{words:?}: {message}");
        }
    }

    #[test]
    fn a_subcommand_is_chosen_by_the_first_word_only() {
        assert_eq!(
            parse_words(&["sessions", "list", "--all"]),
            Ok(Command::Sessions(SessionsCommand::List {
                all: true,
                limit: None,
                json: false
            }))
        );
        assert_eq!(
            parse_words(&["auth", "login", "anthropic"]),
            Ok(Command::Auth(AuthCommand::Login {
                provider: "anthropic".to_owned()
            }))
        );
        assert_eq!(parse_words(&["config"]), Ok(Command::Help(Topic::Config)));
        // After a flag, the same word is part of the prompt.
        assert_eq!(
            run_of(&["-p", "sessions"]).prompt.as_deref(),
            Some("sessions")
        );
    }

    /// `attach` takes where the server is, the session to open, a prompt
    /// and the log's level; one session choice at most; nothing of a run's
    /// core, which is the server's.
    #[test]
    fn attach_takes_its_place_its_session_and_a_prompt() {
        assert_eq!(
            parse_words(&["attach"]).unwrap(),
            Command::Attach(Box::default())
        );
        assert_eq!(
            parse_words(&["attach", "--socket", "here.sock", "--continue", "say", "hi"]).unwrap(),
            Command::Attach(Box::new(Attach {
                socket: Some("here.sock".to_owned()),
                choice: Choice::Continue,
                prompt: Some("say hi".to_owned()),
                log_level: None,
            }))
        );
        assert_eq!(
            parse_words(&["attach", "--resume", "s_1", "--log-level", "debug"]).unwrap(),
            Command::Attach(Box::new(Attach {
                socket: None,
                choice: Choice::Resume("s_1".to_owned()),
                prompt: None,
                log_level: Some("debug".to_owned()),
            }))
        );
        assert_eq!(
            parse_words(&["attach", "--help"]).unwrap(),
            Command::Help(Topic::Attach)
        );
        let both = parse_words(&["attach", "--continue", "--resume", "s_1"]).unwrap_err();
        assert!(both.0.contains("not both"), "{}", both.0);
        assert!(parse_words(&["attach", "--model", "m"]).is_err());
    }

    fn serve_of(words: &[&str]) -> Serve {
        match parse_words(words) {
            Ok(Command::Serve(serve)) => *serve,
            other => panic!("{words:?} is not a server: {other:?}"),
        }
    }

    /// `serve` takes `--stdio` and a run's flags that set the core up, as a
    /// run reads them; a run's own flags are refused by name.
    #[test]
    fn serve_takes_the_cores_flags_and_refuses_a_runs_own() {
        let serve = serve_of(&[
            "serve",
            "--stdio",
            "--profile",
            "review",
            "-c",
            "retry.attempts=3",
            "--model",
            "m",
            "--permission-mode",
            "read-only",
            "--allow",
            "exec:cargo test *",
            "--trust-project",
            "--log-level",
            "debug",
        ]);
        assert!(serve.stdio);
        assert_eq!(serve.run.common.profile.as_deref(), Some("review"));
        assert_eq!(serve.run.allow, ["exec:cargo test *"]);
        assert!(serve.run.trust_project);
        assert_eq!(serve.run.log_level.as_deref(), Some("debug"));
        assert!(
            serve
                .run
                .common
                .settings
                .iter()
                .any(|(key, value, _)| key == "model.main" && value == "m")
        );
        assert_eq!(serve.run_only(), None);

        for (words, flag) in [
            (&["serve", "--stdio", "-p"][..], "-p"),
            (&["serve", "--stdio", "fix", "it"], "a prompt"),
            (&["serve", "--stdio", "--output", "json"], "--output"),
            (
                &["serve", "--stdio", "--include-deltas"],
                "--include-deltas",
            ),
            (&["serve", "--stdio", "--continue"], "--continue"),
            (&["serve", "--stdio", "--resume", "s_1"], "--resume"),
            (&["serve", "--stdio", "--fork", "s_1"], "--fork"),
            (&["serve", "--stdio", "--no-session"], "--no-session"),
            (
                &["serve", "--stdio", "--fail-on-denial"],
                "--fail-on-denial",
            ),
            (
                &["serve", "--stdio", "--max-requests", "3"],
                "--max-requests",
            ),
            (&["serve", "--stdio", "--max-cost", "1"], "--max-cost"),
            (&["serve", "--stdio", "--max-time", "1m"], "--max-time"),
        ] {
            assert_eq!(serve_of(words).run_only(), Some(flag), "{words:?}");
        }
        // The same key set with -c is not the flag.
        assert_eq!(
            serve_of(&["serve", "--stdio", "-c", "limits.turn.requests=3"]).run_only(),
            None
        );
    }

    #[test]
    fn serve_needs_a_transport_and_has_its_help() {
        let Err(UsageError(message)) = parse_words(&["serve"]) else {
            panic!("a server with no transport was read");
        };
        assert!(message.contains("--stdio"), "{message}");
        assert!(message.contains("--socket"), "{message}");
        assert!(parse_words(&["serve", "--stdio", "--socket"]).is_err());
        assert_eq!(serve_of(&["serve", "--socket"]).socket, Some(None));
        assert_eq!(
            serve_of(&["serve", "--socket", "/run/x.sock"]).socket,
            Some(Some("/run/x.sock".to_owned()))
        );
        assert_eq!(
            serve_of(&["serve", "--socket=/run/x.sock", "--trust-project"]).socket,
            Some(Some("/run/x.sock".to_owned()))
        );
        // A word that does not follow `--socket` is a prompt, which a
        // server refuses.
        assert_eq!(
            serve_of(&["serve", "--socket", "--trust-project", "fix", "it"]).run_only(),
            Some("a prompt")
        );
        assert_eq!(
            parse_words(&["serve", "--help"]),
            Ok(Command::Help(Topic::Serve))
        );
        assert_eq!(
            parse_words(&["serve", "--stdio", "-h"]),
            Ok(Command::Help(Topic::Serve))
        );
    }
}
