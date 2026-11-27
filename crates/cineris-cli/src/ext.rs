//! `cineris ext …`: the user's extensions and the project's (since step
//! 28's third pull request; `docs/architecture/12-extensions.md`, "As built
//! in step 28: managing extensions").
//!
//! The core decides; this prints and asks. A question is put only at a
//! terminal, as `cineris permissions trust` puts its own.
//!
//! - **What is signed** (since the signed approvals step): `add`, and
//!   `remove` of a pinned extension, change the person's pins, which are
//!   signed with the key their password makes. After what it approves is
//!   shown, the password is asked for at the terminal, with lines that
//!   name it (`crate::approvals::key_at_terminal`): `Enter` is the yes,
//!   `Esc` the no, and with no key set up the yes sets one up. With no
//!   terminal there is no approval: `--hash` stays a check of what is
//!   shown, and is not enough on its own (the owner accepted that scripted
//!   approval stops working without a person).
//! - **What is not signed:** `disable`, which writes the user's
//!   configuration, and `remove` of a folder nothing pins. A yes (`y` or
//!   `yes`) at a terminal does it, anything else, an empty line included,
//!   does not; with no terminal to ask at, only `--hash H`, the hash of
//!   what was shown (the owner's answer to the early review of step 28's
//!   third pull request, which took `--yes` away).
//! - **A `--hash` given** must be the hash of what is shown wherever it is
//!   given: otherwise nothing is asked or done.
//!
//! **Under a session's command** each of `add`, `remove` and `disable` is
//! refused before anything is read (`cineris_sandbox::session`).
//!
//! **In a build without the script engine** (this crate's `scripts`
//! feature off; decision 0005), `add` says so and changes nothing: an
//! extension is approved only as it was evaluated, and there is nothing to
//! evaluate it with.

use std::io::{self, IsTerminal};
#[cfg(feature = "scripts")]
use std::path::Path;
use std::sync::Arc;

use cineris_base::in_code;
use cineris_config::Env;
#[cfg(feature = "scripts")]
use cineris_core::{Added, ToAdd};
use cineris_core::{
    Core, CoreOptions, DisableOutcome, ExtensionListed, ExtensionSource, ExtensionsListing, Gates,
    OpError, Project, Removal, Removed, TokioTimer,
};
use cineris_protocol::ErrorKind;

use crate::args::ExtCommand;
use crate::exit;

/// A finished subcommand: its exit code.
type Done = u8;

fn say(name: &str, text: &str) {
    crate::print_err(&format!("{name}: {}\n", crate::visible(text)));
}

fn out(text: &str) {
    crate::print_out(&crate::visible(text));
}

fn fail(name: &str, error: &OpError) -> Done {
    say(name, &error.message);
    match error.kind {
        ErrorKind::Usage | ErrorKind::Config => exit::USAGE,
        _ => exit::INTERNAL,
    }
}

/// A core as the other subcommands start one, with the launcher a session
/// is given, so that what an extension's programs are said to be able to do
/// is what a session would let them do. With `asks`, at a terminal, the
/// terminal for the approvals' password too, taken before the core starts
/// (and before anything else slow), with the `[terminal]` settings of the
/// configuration the core is given: a password typed early is not echoed
/// (`crate::approvals::Taken`).
fn core(name: &str, asks: bool) -> Result<(Core, Option<crate::approvals::Taken>), Done> {
    let env = Env::from_process();
    let loaded = cineris_config::load(&cineris_config::LoadOptions::new(env.clone())).map_err(
        |failure| {
            say(name, &failure.render(&env));
            exit::USAGE
        },
    )?;
    let taken = if asks {
        crate::approvals::Taken::take(crate::interactive::overrides(&loaded.settings.terminal))
    } else {
        None
    };
    let mut options = CoreOptions::new(env, Arc::new(TokioTimer));
    options.loaded = Some(loaded);
    options.launcher = std::env::current_exe()
        .ok()
        .map(|exe| (exe, vec![std::ffi::OsString::from(crate::args::SANDBOX)]));
    cineris_base::hook::hold_core();
    match Core::start(options) {
        Ok(core) => Ok((core, taken)),
        Err(error) => {
            // Put back first, so that what is said is said as usual.
            drop(taken);
            Err(fail(name, &error))
        }
    }
}

/// The project of the working directory, as a session's would be.
fn project(name: &str, core: &Core) -> Result<Project, Done> {
    let here = crate::workspace::Here::now().map_err(|error| {
        say(
            name,
            &format!("the working directory could not be read: {error}"),
        );
        exit::USAGE
    })?;
    for said in here.passed_over_said() {
        say(name, &said);
    }
    core.project(&here.root).map_err(|error| fail(name, &error))
}

/// What the person answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Answer {
    Yes,
    No,
    /// A `--hash` was given that is not the hash of what was shown, or
    /// where there is none to name it by: nothing is asked or done.
    HashDiffers,
}

/// Whether the person approved what was shown with the hash `shown`: a
/// `--hash` given must be it, wherever it is given; then at a terminal only
/// the question decides, and with no terminal to ask at the hash given is
/// the approval, nothing read.
fn approved(
    at_terminal: bool,
    given: Option<&str>,
    shown: Option<&str>,
    answer: impl FnOnce() -> io::Result<String>,
) -> Answer {
    match (given, shown) {
        (Some(given), Some(shown)) if !given.trim().eq_ignore_ascii_case(shown) => {
            Answer::HashDiffers
        }
        (Some(_), None) => Answer::HashDiffers,
        _ if at_terminal => {
            if answer()
                .is_ok_and(|line| matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
            {
                Answer::Yes
            } else {
                Answer::No
            }
        }
        (Some(_), Some(_)) => Answer::Yes,
        (None, _) => Answer::No,
    }
}

/// Asks `question` at the terminal, on the one taken for the approvals'
/// password when there is one (raw, so asked by keys), or takes the hash
/// given where there is none; and whether there was a terminal.
fn asked(
    question: &str,
    given: Option<&str>,
    shown: Option<&str>,
    taken: Option<&mut crate::approvals::Taken>,
) -> (Answer, bool) {
    let at_terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    let answer = approved(at_terminal, given, shown, || match taken {
        Some(taken) => Ok(if taken.confirm(question) { "y" } else { "n" }.to_owned()),
        None => {
            out(&format!("{question} [y/N] "));
            let mut line = String::new();
            io::stdin().read_line(&mut line).map(|_| line)
        }
    });
    (answer, at_terminal)
}

/// What is said when the answer was not a yes: `what` is what was not
/// done, `how` the flag that does it with no terminal.
fn not_done(
    answer: Answer,
    at_terminal: bool,
    given: Option<&str>,
    shown: Option<&str>,
    what: &str,
) -> String {
    match (answer, shown) {
        (Answer::HashDiffers, Some(shown)) => format!(
            "{what}: --hash {} is not the hash of what is shown ({shown})",
            crate::visible(given.unwrap_or_default())
        ),
        (Answer::HashDiffers, None) => {
            format!(
                "{what}: it has no hash to name it by here, so only a yes at a terminal does it"
            )
        }
        _ if at_terminal => what.to_owned(),
        (_, Some(shown)) => format!(
            "{what}: with no terminal to ask at, `--hash {shown}` does it, naming exactly what is \
             shown"
        ),
        (_, None) => format!("{what}: with no terminal to ask at, nothing can say yes to it"),
    }
}

/// Whether a signed change of what was shown with the hash `shown` is
/// asked for: a `--hash` given must be that hash, wherever it is given;
/// then the password is asked for at a terminal, and with none there is no
/// approval, whatever hash was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signing {
    /// Ask for the password.
    Ask,
    /// The `--hash` given is not the hash of what is shown, or there is
    /// none to name it by: nothing is asked or done.
    HashDiffers,
    /// No terminal to ask at: nothing is approved.
    NoTerminal,
}

fn signing(at_terminal: bool, given: Option<&str>, shown: Option<&str>) -> Signing {
    match (given, shown) {
        (Some(given), Some(shown)) if !given.trim().eq_ignore_ascii_case(shown) => {
            Signing::HashDiffers
        }
        (Some(_), None) => Signing::HashDiffers,
        _ if at_terminal => Signing::Ask,
        _ => Signing::NoTerminal,
    }
}

/// What a signed change is of: the hash a `--hash` given must be, the
/// lines that name it under the prompt's sentence, what is said when it is
/// not done, and the command line to run at a terminal.
struct ToSign<'a> {
    given: Option<&'a str>,
    shown: Option<&'a str>,
    lines: &'a [String],
    not_done: &'a str,
    command: &'a str,
}

/// The person's key for a signed change, after what it approves was shown.
/// `Err` with the exit code, said.
fn signed(
    name: &str,
    core: &Core,
    taken: Option<&mut crate::approvals::Taken>,
    change: &ToSign<'_>,
) -> Result<cineris_core::signed::PersonKey, Done> {
    use crate::approvals::NotSigned;
    let what = change.not_done;
    match signing(crate::approvals::at_terminal(), change.given, change.shown) {
        Signing::HashDiffers => {
            say(
                name,
                &not_done(Answer::HashDiffers, true, change.given, change.shown, what),
            );
            Err(exit::USAGE)
        }
        Signing::NoTerminal => {
            say(
                name,
                &format!(
                    "{what}: {}",
                    crate::approvals::no_terminal(name, change.command)
                ),
            );
            Err(exit::USAGE)
        }
        Signing::Ask => match crate::approvals::key_at_terminal(core, taken, change.lines) {
            Ok(key) => Ok(key),
            Err(NotSigned::Declined) => {
                say(name, what);
                Err(exit::USAGE)
            }
            Err(NotSigned::Refused(why)) => {
                say(name, &format!("{what}: {why}"));
                Err(exit::USAGE)
            }
            Err(NotSigned::Signal(code)) => Err(code),
        },
    }
}

/// A current-thread runtime, for what the core evaluates and writes.
#[cfg(feature = "scripts")]
fn runtime(name: &str) -> Result<tokio::runtime::Runtime, Done> {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(|error| {
            say(name, &format!("the runtime did not start: {error}"));
            exit::INTERNAL
        })
}

/// `path` with a leading `~` (alone, or before a `/`, or a `\` on Windows)
/// as the home directory: a shell expands it, PowerShell and `cmd` do not
/// for a program's argument (the early review of step 28's third pull
/// request).
#[cfg(feature = "scripts")]
fn expanded(path: &str, home: Option<&Path>) -> std::path::PathBuf {
    let rest = path.strip_prefix('~').filter(|rest| {
        rest.is_empty() || rest.starts_with('/') || (cfg!(windows) && rest.starts_with('\\'))
    });
    match (rest, home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches(['/', '\\'])),
        _ => std::path::PathBuf::from(path),
    }
}

/// `cineris ext …`
pub(crate) fn ext(command: ExtCommand, name: &'static str) -> Done {
    let run = || -> Result<Done, Done> {
        match &command {
            ExtCommand::Add { .. } => crate::not_under_a_session(name, "ext add")?,
            ExtCommand::Remove { .. } => crate::not_under_a_session(name, "ext remove")?,
            ExtCommand::Disable { .. } => crate::not_under_a_session(name, "ext disable")?,
            ExtCommand::List | ExtCommand::Inspect { .. } => {}
        }
        // `add` and `remove` may ask for the approvals' password: the
        // terminal is taken before the core starts.
        let asks = matches!(command, ExtCommand::Add { .. } | ExtCommand::Remove { .. });
        let (core, mut taken) = core(name, asks)?;
        let project = project(name, &core)?;
        let fail = |error: OpError| fail(name, &error);
        match command {
            ExtCommand::List => {
                let listing = core.extensions(&project).map_err(fail)?;
                out(&listing_text(&listing, &project.shown));
                Ok(exit::COMPLETED)
            }
            ExtCommand::Inspect { name: wanted } => {
                let listing = core.extensions(&project).map_err(fail)?;
                let matching: Vec<&ExtensionListed> = listing
                    .extensions
                    .iter()
                    .filter(|one| one.name == wanted)
                    .collect();
                if matching.is_empty() {
                    say(
                        name,
                        &format!(
                            "no extension is named `{}`: `{name} ext list` lists them",
                            in_code(&wanted)
                        ),
                    );
                    return Err(exit::USAGE);
                }
                let text: Vec<String> = matching.into_iter().map(inspected_text).collect();
                out(&text.join("\n"));
                Ok(exit::COMPLETED)
            }
            ExtCommand::Disable { name: wanted, hash } => {
                // One that holds gates is turned off only on a yes naming
                // them (the early review's M1).
                let gates = core.extension_gates(&project, &wanted).map_err(fail)?;
                if !gates.held.is_empty() || gates.unknown {
                    out(&gates_text(&wanted, &gates));
                    let shown = gates.hash.map(|hash| hash.to_string());
                    let (answer, at_terminal) = asked(
                        &format!("Turn `{}` off, letting its gates go?", in_code(&wanted)),
                        hash.as_deref(),
                        shown.as_deref(),
                        None,
                    );
                    if answer != Answer::Yes {
                        say(
                            name,
                            &not_done(
                                answer,
                                at_terminal,
                                hash.as_deref(),
                                shown.as_deref(),
                                &format!("`{}` was not turned off", in_code(&wanted)),
                            ),
                        );
                        return Err(exit::USAGE);
                    }
                }
                let outcome = core.disable_extension(&project, &wanted).map_err(fail)?;
                let (text, done) = disabled_text(&outcome, &wanted, name);
                if done {
                    out(&text);
                    Ok(exit::COMPLETED)
                } else {
                    say(name, &text);
                    Err(exit::USAGE)
                }
            }
            ExtCommand::Remove { name: wanted, hash } => {
                let removal = core.extension_removal(&project, &wanted).map_err(fail)?;
                if removal.project_only {
                    // The core refuses it, naming the commands for a
                    // project's extensions.
                    return match core.remove_extension(&removal, false, None) {
                        Err(error) => Err(fail(error)),
                        Ok(_) => Err(exit::USAGE),
                    };
                }
                out(&removal_text(&removal));
                // Its pin and its folder go on one yes that names both (the
                // early review's M1), and nothing goes without it.
                let shown = removal.hash.map(|hash| hash.to_string());
                let not_removed = format!(
                    "`{}` was not removed, and nothing was changed",
                    in_code(&removal.name)
                );
                // Taking a pin away is signed with the person's key: the
                // password is the yes. A folder with no pin is deleted on a
                // plain yes.
                let key = if removal.pin.is_some() {
                    Some(signed(
                        name,
                        &core,
                        taken.as_mut(),
                        &ToSign {
                            given: hash.as_deref(),
                            shown: shown.as_deref(),
                            lines: &[removal_signed(&removal)],
                            not_done: &not_removed,
                            command: &format!("ext remove {wanted}"),
                        },
                    )?)
                } else {
                    let (answer, at_terminal) = asked(
                        &removal_question(&removal),
                        hash.as_deref(),
                        shown.as_deref(),
                        taken.as_mut(),
                    );
                    if answer != Answer::Yes {
                        say(
                            name,
                            &not_done(
                                answer,
                                at_terminal,
                                hash.as_deref(),
                                shown.as_deref(),
                                &not_removed,
                            ),
                        );
                        return Err(exit::USAGE);
                    }
                    None
                };
                // The key goes when this arm ends, once it has signed.
                let removed =
                    core.remove_extension(&removal, removal.folder.is_some(), key.as_ref());
                out(&removed_text(&removal, removed.map_err(fail)?));
                Ok(exit::COMPLETED)
            }
            // Without the script engine nothing can be evaluated, and an
            // extension is approved only as it was: nothing is read from
            // where it is, written or pinned.
            #[cfg(not(feature = "scripts"))]
            ExtCommand::Add { .. } => {
                say(name, &no_script_engine(name));
                Err(exit::USAGE)
            }
            #[cfg(feature = "scripts")]
            ExtCommand::Add { path, hash } => {
                let home = Env::from_process().home();
                let mut to_add = core
                    .extension_to_add(&project, &expanded(&path, home.as_deref()))
                    .map_err(fail)?;
                // Evaluated before the question, so that what it registers
                // is shown with the rest (the early review's M2): one that
                // cannot be is not asked about.
                let runtime = runtime(name)?;
                runtime
                    .block_on(core.evaluate_extension(&mut to_add))
                    .map_err(fail)?;
                out(&to_add_text(&to_add));
                if to_add.pinned && to_add.unchanged {
                    out(&format!(
                        "It is approved as it is already, at `{}`: nothing to do.\n",
                        to_add.destination
                    ));
                    return Ok(exit::COMPLETED);
                }
                // The password is the yes: the pin is signed with the key it
                // makes (the signed approvals step).
                let shown = to_add.hash.to_string();
                let key = signed(
                    name,
                    &core,
                    taken.as_mut(),
                    &ToSign {
                        given: hash.as_deref(),
                        shown: Some(&shown),
                        lines: &add_signed(&to_add),
                        not_done: &format!(
                            "`{}` was not added, and nothing was written or pinned",
                            in_code(&to_add.name)
                        ),
                        command: &format!("ext add {path}"),
                    },
                )?;
                // The key goes when this arm ends, once it has signed.
                let added = runtime.block_on(core.add_extension(to_add, &key));
                out(&added_text(&added.map_err(fail)?));
                Ok(exit::COMPLETED)
            }
        }
    };
    match run() {
        Ok(code) | Err(code) => code,
    }
}

/// What `ext add` says in a build without the script engine (decision
/// 0005), run as `name`.
#[cfg(any(test, not(feature = "scripts")))]
fn no_script_engine(name: &str) -> String {
    format!(
        "This build of Cineris has no script engine, so `{name} ext add` cannot evaluate or \
         approve an extension."
    )
}

fn whose(source: ExtensionSource) -> &'static str {
    match source {
        ExtensionSource::User => "yours",
        ExtensionSource::Project => "this project's",
    }
}

/// What a registration holds, on one line: its tools, commands and hooks.
fn registered_line(registered: &cineris_core::Registered) -> String {
    let list = |items: &[String]| {
        if items.is_empty() {
            "none".to_owned()
        } else {
            items
                .iter()
                .map(|item| format!("`{}`", in_code(item)))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    format!(
        "tools {}; commands {}; hooks {}",
        list(&registered.tools),
        list(&registered.commands),
        list(&registered.hooks)
    )
}

/// What `ext list` prints.
fn listing_text(listing: &ExtensionsListing, project: &str) -> String {
    let mut text = String::new();
    for (source, heading) in [
        (ExtensionSource::User, "Your extensions:".to_owned()),
        (
            ExtensionSource::Project,
            format!(
                "This project's extensions ({project}, {}):",
                if listing.project_trusted {
                    "trusted"
                } else {
                    "not trusted"
                }
            ),
        ),
    ] {
        let ones: Vec<&ExtensionListed> = listing
            .extensions
            .iter()
            .filter(|one| one.source == source)
            .collect();
        text.push_str(&heading);
        text.push('\n');
        if ones.is_empty() {
            text.push_str("  none\n");
        }
        for one in ones {
            text.push_str(&format!(
                "  `{}`{}: {}\n",
                in_code(&one.name),
                one.folder
                    .as_deref()
                    .map(|folder| format!(" (`{folder}`)"))
                    .unwrap_or_default(),
                one.standing.words()
            ));
            if let Some(registered) = &one.registered {
                text.push_str(&format!("      {}\n", registered_line(registered)));
            }
        }
    }
    if !listing.problems.is_empty() {
        text.push_str("Could not be read:\n");
        for problem in &listing.problems {
            text.push_str(&format!("  {problem}\n"));
        }
    }
    text.push_str("Skills:\n");
    if listing.skills.is_empty() {
        text.push_str("  none found\n");
    }
    for (skill, path) in &listing.skills {
        text.push_str(&format!("  {skill}  ({path})\n"));
    }
    if !listing.skills_left_out.is_empty() {
        text.push_str("Skills left out:\n");
        for (path, why) in &listing.skills_left_out {
            text.push_str(&format!("  {path}: {why}\n"));
        }
    }
    text
}

/// What `ext inspect` prints of one extension.
fn inspected_text(one: &ExtensionListed) -> String {
    let mut text = format!("`{}`, {}\n", in_code(&one.name), whose(one.source));
    text.push_str(&format!(
        "  folder:   {}\n",
        one.folder
            .as_deref()
            .map_or_else(|| "gone".to_owned(), |folder| format!("`{folder}`"))
    ));
    text.push_str(&format!(
        "  hash:     {}\n",
        one.hash
            .map_or_else(|| "not read".to_owned(), |hash| hash.to_string())
    ));
    text.push_str(&format!(
        "  pin:      {}\n",
        one.pin
            .as_ref()
            .map_or_else(|| "none".to_owned(), |(hash, at)| format!("{hash}, {at}"))
    ));
    text.push_str(&format!("  standing: {}\n", one.standing.words()));
    text.push_str(&format!(
        "  registered: {}\n",
        one.registered.as_ref().map_or_else(
            || "not in the registration cache".to_owned(),
            registered_line
        )
    ));
    if let Some(words) = &one.words {
        text.push_str("  What it may do:\n");
        for line in words.lines() {
            text.push_str(&format!("    {line}\n"));
        }
    }
    text
}

/// What `ext add` shows before its question: the snapshot's manifest in
/// words, and every file with its size.
#[cfg(feature = "scripts")]
fn to_add_text(to_add: &ToAdd) -> String {
    let mut text = format!(
        "The extension `{}`, read from `{}` (its hash is {}):\n",
        in_code(&to_add.name),
        to_add.source,
        to_add.hash
    );
    for line in to_add.words.lines() {
        text.push_str(&format!("  {line}\n"));
    }
    for (heading, items) in [
        ("Its tools:", &to_add.tools),
        ("Its commands:", &to_add.commands),
    ] {
        if items.is_empty() {
            continue;
        }
        text.push_str(&format!("{heading}\n"));
        for (item, description) in items {
            text.push_str(&format!("  `{item}`, which its author describes as:\n"));
            for line in description.lines() {
                text.push_str(&format!("    {line}\n"));
            }
        }
    }
    if let Some(prompt) = &to_add.prompt {
        text.push_str("What it adds to the system prompt, whole, in its author's words:\n");
        for line in prompt.lines() {
            text.push_str(&format!("  {line}\n"));
        }
    }
    let total: u64 = to_add.files.iter().map(|(_, size)| size).sum();
    text.push_str(&format!(
        "Its files, {} in all, {total} bytes:\n",
        to_add.files.len()
    ));
    for (path, size) in &to_add.files {
        text.push_str(&format!("  `{path}`  {size} bytes\n"));
    }
    if to_add.in_place {
        text.push_str(&format!(
            "It is approved where it is kept, `{}`, as it was read: anything changed there since \
             is put back as shown.\n",
            to_add.destination
        ));
    } else if to_add.unchanged {
        text.push_str(&format!(
            "`{}` holds it as shown already.\n",
            to_add.destination
        ));
    } else {
        text.push_str(&format!(
            "It is written to `{}`{}.\n",
            to_add.destination,
            to_add
                .replaces
                .as_deref()
                .map(|what| format!(", replacing {what}"))
                .unwrap_or_default()
        ));
    }
    if let Some((hash, at)) = &to_add.pinned_before {
        text.push_str(&format!(
            "You approved another version of it ({hash}, {at}): this approval takes its place.\n"
        ));
    }
    for note in &to_add.notes {
        text.push_str(note);
        text.push('\n');
    }
    text
}

/// What the password approves at `ext add`, under the prompt's sentence:
/// the extension as shown, where it goes (naming what it replaces), and
/// its hash.
#[cfg(feature = "scripts")]
fn add_signed(to_add: &ToAdd) -> Vec<String> {
    let code = in_code(&to_add.name);
    let first = match &to_add.replaces {
        Some(what) => format!("`{code}` as shown, replacing {what}"),
        None if to_add.in_place => format!("`{code}` as shown, in `{}`", to_add.destination),
        None => format!("`{code}` as shown, written to `{}`", to_add.destination),
    };
    vec![first, format!("its hash {}", to_add.hash)]
}

/// What `ext add` says once it is approved.
#[cfg(feature = "scripts")]
fn added_text(added: &Added) -> String {
    let mut text = format!(
        "Approved `{}`{}, and pinned it: sessions opened from now on load it, and a session \
         running now loads it at its next turn.\n",
        in_code(&added.name),
        if added.written {
            format!(", written to `{}`", added.destination)
        } else {
            format!(" in `{}`", added.destination)
        },
    );
    text.push_str(&format!(
        "It registered {}.\n",
        registered_line(&added.registered)
    ));
    if let Some(why) = &added.left_aside {
        text.push_str(&format!("It is in place; {why}.\n"));
    }
    if let Some(why) = &added.not_cached {
        text.push_str(&format!(
            "What it registered was not kept in the registration cache ({why}): the next session \
             registers it again.\n"
        ));
    }
    text
}

/// `name` as a key of a TOML table's header: bare when it can be, quoted
/// otherwise.
fn table(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        format!("[extensions.{name}]")
    } else {
        format!(
            "[extensions.\"{}\"]",
            name.replace('\\', "\\\\").replace('"', "\\\"")
        )
    }
}

/// What `ext disable` says, and whether it was done.
fn disabled_text(outcome: &DisableOutcome, wanted: &str, name: &str) -> (String, bool) {
    let code = in_code(wanted);
    match outcome {
        DisableOutcome::Project => (
            format!(
                "`{code}` is this project's extension, which your configuration does not turn \
                 off: a project's extension is kept out by its trust. `{name} permissions trust` \
                 shows the project's extensions and approves them, and `{name} permissions \
                 untrust` takes the project's trust away"
            ),
            false,
        ),
        DisableOutcome::Done {
            file,
            written,
            made,
            effect,
            in_force,
        } => {
            let key = format!("`{} enabled = false`", table(wanted));
            let mut text = if *written {
                format!("Turned `{code}` off: {key} in `{file}`.\n")
            } else {
                format!(
                    "Your configuration turns `{code}` off already ({key} in `{file}`): nothing was written.\n"
                )
            };
            if *made {
                text.push_str(&format!(
                    "`{file}` was not there, and was made with this alone, so Cineris's first \
                     run, which starts only while that file is not there, does not start.\n"
                ));
            }
            text.push_str(&match effect {
                cineris_config::Effect::NextSession | cineris_config::Effect::Restart => format!(
                    "It takes effect {}: sessions running now keep the extension until their \
                     next session, and `config/reload` (`/reload`) does not apply it to them, as \
                     the key takes effect {}.\n",
                    effect.as_str(),
                    effect.as_str()
                ),
                other => format!(
                    "It takes effect {}: `config/reload` (`/reload`) applies it to sessions \
                     running now.\n",
                    other.as_str()
                ),
            });
            if !in_force {
                text.push_str(
                    "But the configuration in force here does not turn it off: a managed file \
                     holds it on, and the managed file's word stands.\n",
                );
            }
            (text, true)
        }
    }
}

/// What `ext remove` takes away, said before its question, with the hash
/// that `--hash` names it by.
fn removal_text(removal: &Removal) -> String {
    let code = in_code(&removal.name);
    let pin = removal.pin.as_ref().map_or_else(
        || "it is not pinned".to_owned(),
        |(hash, at)| {
            format!(
                "its pin ({hash}, {at}) is taken away, so no session loads it and its gates no \
                 longer hold"
            )
        },
    );
    let folder = match &removal.folder {
        None => "its folder is gone already".to_owned(),
        Some((folder, false)) => format!("its folder `{folder}` is deleted"),
        Some((folder, true)) => {
            format!("`{folder}`, a link, is unlinked, and what it leads to is left as it is")
        }
    };
    let hash = match (&removal.pin, removal.hash) {
        (Some(_), _) => String::new(),
        (None, Some(hash)) => format!(" Its folder's hash is {hash}."),
        (None, None) => String::new(),
    };
    format!("Removing your extension `{code}`: {pin}, and {folder}.{hash}\n")
}

/// `ext remove`'s one question, naming the pin and the folder under `~`.
fn removal_question(removal: &Removal) -> String {
    let code = in_code(&removal.name);
    let folder = match &removal.folder {
        None => None,
        Some((folder, true)) => Some(format!(
            "unlink `{folder}`, which is a link, leaving what it leads to"
        )),
        Some((folder, false)) => Some(format!("delete `{folder}` and everything in it")),
    };
    match (removal.pin.is_some(), folder) {
        (true, Some(folder)) => format!("Unpin `{code}` and {folder}?"),
        (true, None) => format!("Unpin `{code}`?"),
        (false, Some(folder)) => {
            let mut chars = folder.chars();
            let first = chars
                .next()
                .map(|c| c.to_ascii_uppercase())
                .unwrap_or_default();
            format!("{first}{}?", chars.as_str())
        }
        (false, None) => format!("Remove `{code}`?"),
    }
}

/// What the password approves at `ext remove` of a pinned extension, under
/// the prompt's sentence.
fn removal_signed(removal: &Removal) -> String {
    let code = in_code(&removal.name);
    match &removal.folder {
        None => format!("taking away your approval of `{code}`"),
        Some((folder, true)) => format!(
            "taking away your approval of `{code}`, and unlinking `{folder}`, which is a link, \
             leaving what it leads to"
        ),
        Some((folder, false)) => format!(
            "taking away your approval of `{code}`, and deleting `{folder}` and everything in it"
        ),
    }
}

/// What `ext remove` says it did.
fn removed_text(removal: &Removal, removed: Removed) -> String {
    let code = in_code(&removal.name);
    let mut text = if removed.unpinned {
        format!("Unpinned `{code}`: no session loads it, and its gates no longer hold.\n")
    } else {
        format!("`{code}` was not pinned.\n")
    };
    if let (Some((folder, link)), true) = (&removal.folder, removed.deleted) {
        text.push_str(&if *link {
            format!("Unlinked `{folder}`.\n")
        } else {
            format!("Deleted `{folder}`.\n")
        });
    }
    text
}

/// What `ext disable` says of the gates it would let go, before asking.
fn gates_text(wanted: &str, gates: &Gates) -> String {
    let code = in_code(wanted);
    let hash = gates
        .hash
        .map(|hash| format!(" (its approved version's hash is {hash})"))
        .unwrap_or_default();
    if gates.held.is_empty() {
        format!(
            "What `{code}`{hash} registered is not known, and it may hold gates: hooks that can \
             refuse a prompt or a tool call. Turned off, none of them holds.\n"
        )
    } else {
        format!(
            "`{code}`{hash} holds gates, hooks that can refuse what they are on: {}. Turned off, \
             they no longer hold, and what they would refuse goes ahead.\n",
            gates.held.join(", ")
        )
    }
}

#[cfg(test)]
mod tests;
