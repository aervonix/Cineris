//! `cineris ext …`: the user's extensions and the project's (since step
//! 28's third pull request; `docs/architecture/12-extensions.md`, "As built
//! in step 28: managing extensions").
//!
//! The core decides; this prints and asks. A question is put only at a
//! terminal, as `cineris permissions trust` puts its own: a yes (`y` or
//! `yes`) approves, anything else, an empty line included, does not, and
//! `--yes` is ignored there; with no terminal to ask at, only `--yes`
//! approves.

use std::io::{self, IsTerminal};
use std::path::Path;
use std::sync::Arc;

use cineris_base::in_code;
use cineris_config::Env;
use cineris_core::{
    Added, Core, CoreOptions, DisableOutcome, ExtensionListed, ExtensionSource, ExtensionsListing,
    OpError, Project, Removal, Removed, ToAdd, TokioTimer,
};
use cineris_protocol::ErrorKind;

use crate::args::ExtCommand;
use crate::exit;

/// A finished subcommand: its exit code.
type Done = u8;

fn say(name: &str, text: &str) {
    eprintln!("{name}: {}", crate::visible(text));
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
/// is what a session would let them do.
fn core(name: &str) -> Result<Core, Done> {
    let mut options = CoreOptions::new(Env::from_process(), Arc::new(TokioTimer));
    options.launcher = std::env::current_exe()
        .ok()
        .map(|exe| (exe, vec![std::ffi::OsString::from(crate::args::SANDBOX)]));
    Core::start(options).map_err(|error| fail(name, &error))
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

/// Whether the person said yes: at a terminal, only to the question, `--yes`
/// ignored; with no terminal to ask at, only with `--yes`, nothing read.
fn approved(at_terminal: bool, yes: bool, answer: impl FnOnce() -> io::Result<String>) -> bool {
    if at_terminal {
        answer().is_ok_and(|line| matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
    } else {
        yes
    }
}

/// Asks `question` at the terminal, or takes `yes` where there is none.
fn asked(question: &str, yes: bool) -> (bool, bool) {
    let at_terminal = io::stdin().is_terminal() && io::stdout().is_terminal();
    let answer = approved(at_terminal, yes, || {
        out(&format!("{question} [y/N] "));
        let mut line = String::new();
        io::stdin().read_line(&mut line).map(|_| line)
    });
    (answer, at_terminal)
}

/// `cineris ext …`
pub(crate) fn ext(command: ExtCommand, name: &'static str) -> Done {
    let run = || -> Result<Done, Done> {
        let core = core(name)?;
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
            ExtCommand::Disable { name: wanted } => {
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
            ExtCommand::Remove { name: wanted, yes } => {
                let removal = core.extension_removal(&project, &wanted).map_err(fail)?;
                if removal.project_only {
                    // The core refuses it, naming the commands for a
                    // project's extensions.
                    return match core.remove_extension(&removal, false) {
                        Err(error) => Err(fail(error)),
                        Ok(_) => Err(exit::USAGE),
                    };
                }
                out(&removal_text(&removal));
                let mut delete = false;
                let mut at_terminal = true;
                if let Some((folder, link)) = &removal.folder {
                    (delete, at_terminal) = asked(&removal_question(folder, *link), yes);
                }
                let removed = core.remove_extension(&removal, delete).map_err(fail)?;
                out(&removed_text(&removal, removed, at_terminal, name));
                Ok(exit::COMPLETED)
            }
            ExtCommand::Add { path, yes } => {
                let to_add = core
                    .extension_to_add(&project, Path::new(&path))
                    .map_err(fail)?;
                out(&to_add_text(&to_add));
                if to_add.pinned && to_add.unchanged {
                    out(&format!(
                        "It is approved as it is already, at `{}`: nothing to do.\n",
                        to_add.destination
                    ));
                    return Ok(exit::COMPLETED);
                }
                let (answer, at_terminal) = asked(&add_question(&to_add), yes);
                if !answer {
                    say(
                        name,
                        &format!(
                            "`{}` was not added: nothing was written or pinned{}",
                            in_code(&to_add.name),
                            if at_terminal {
                                ""
                            } else {
                                "; with no terminal to ask at, --yes approves it"
                            }
                        ),
                    );
                    return Err(exit::USAGE);
                }
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .map_err(|error| {
                        say(name, &format!("the runtime did not start: {error}"));
                        exit::INTERNAL
                    })?;
                let added = runtime.block_on(core.add_extension(to_add)).map_err(fail)?;
                out(&added_text(&added));
                Ok(exit::COMPLETED)
            }
        }
    };
    match run() {
        Ok(code) | Err(code) => code,
    }
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

/// `ext add`'s question, naming what a yes replaces.
fn add_question(to_add: &ToAdd) -> String {
    let code = in_code(&to_add.name);
    match &to_add.replaces {
        Some(what) => format!("Approve `{code}` as shown, and replace {what} with it?"),
        None if to_add.in_place => {
            format!("Approve `{code}` as shown, in `{}`?", to_add.destination)
        }
        None => format!(
            "Approve `{code}` as shown, written to `{}`?",
            to_add.destination
        ),
    }
}

/// What `ext add` says once it is approved.
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

/// What `ext remove` takes away, said before its question.
fn removal_text(removal: &Removal) -> String {
    let code = in_code(&removal.name);
    let pin = removal.pin.as_ref().map_or_else(
        || "it is not pinned".to_owned(),
        |(hash, at)| format!("its pin ({hash}, {at}) is taken away"),
    );
    let folder = match &removal.folder {
        None => "its folder is gone already".to_owned(),
        Some((folder, false)) => format!("its folder `{folder}` is deleted on a yes"),
        Some((folder, true)) => format!(
            "`{folder}`, a link, is unlinked on a yes, and what it leads to is left as it is"
        ),
    };
    format!("Removing your extension `{code}`: {pin}, and {folder}.\n")
}

/// `ext remove`'s question, naming the folder under `~`.
fn removal_question(folder: &str, link: bool) -> String {
    if link {
        format!("Unlink `{folder}`, which is a link, leaving what it leads to?")
    } else {
        format!("Delete `{folder}` and everything in it?")
    }
}

/// What `ext remove` says it did.
fn removed_text(removal: &Removal, removed: Removed, at_terminal: bool, name: &str) -> String {
    let code = in_code(&removal.name);
    let mut text = if removed.unpinned {
        format!("Unpinned `{code}`: no session loads it, and its gates no longer hold.\n")
    } else {
        format!("`{code}` was not pinned.\n")
    };
    match (&removal.folder, removed.deleted) {
        (Some((folder, _)), true) => text.push_str(&format!("Deleted `{folder}`.\n")),
        (Some((folder, _)), false) => text.push_str(&format!(
            "`{folder}` is left as it is{}: not approved, it is not loaded, and `{name} ext add \
             {folder}` approves it again.\n",
            if at_terminal {
                ""
            } else {
                " (with no terminal to ask at, --yes deletes it)"
            }
        )),
        (None, _) => {}
    }
    text
}

#[cfg(test)]
mod tests;
