//! Custom commands from files: a Markdown file is a slash command
//! (`docs/architecture/12-extensions.md`, "Tier 1: custom commands"; since
//! step 28).
//!
//! A command is `<root>/<name>.md`, in the user's `<config dir>/commands/`
//! or the workspace's `.cineris/commands/`. Its name is the file's, by the
//! registry's rules ([`check_name`]). Its front matter, read strictly, may
//! give a `description`, the `arguments` it names (each with a default if
//! it has one) and a settings `profile`; its body is the text sent as the
//! user's prompt, with `{{name}}`, `{{name | "default"}}` and `{{args}}`
//! (everything typed after the command) put in ([`CommandFile::expand`]).
//! Nothing in a command runs and nothing is included: it is text for the
//! model, and needs no trust.
//!
//! **Which is used.** The user's command of a name is used over a
//! project's, names compared with ASCII's case folded: its body is sent as
//! the user's prompt, so a repository must not be able to make the user's
//! own `/review` send text it chose. A project's command whose name a file
//! of the user's takes is left out, saying why; a file of the user's takes
//! its name whether it can be used or not (a link, one that is not UTF-8).
//! When a folder of the user's could not be read whole (it could not be
//! read, it is not a folder, it holds more than [`MAX_ENTRIES`] entries, an
//! entry of it could not be read, or its path is not Unicode), no project's
//! command is used, each said: its name could not be checked against the
//! user's. A folder of the user's that is not there has no names. Within
//! one folder, files whose names differ only in case are all left out, on
//! every system: on Windows they are one name.
//!
//! **What a project's command sends is seen in its file.** Its body, and
//! its arguments' defaults, may hold no character that acts unseen (a
//! control character other than a line break or a tab, a format character
//! such as a direction override or a tag character), as a script's
//! description may not ([`check_description`]); one that does is left out,
//! the character named as an escape. The user's own are sent as they wrote
//! them.
//!
//! **Bounds**, as the skills' are: at most [`MAX_ENTRIES`] entries looked
//! at in a root, every entry counting; a file of at most
//! [`MAX_FILE_BYTES`], one over it refused by its length, unread; and no
//! link followed inside a folder of commands. A root may be a link the user
//! made, as a folder of skills may; a project's must lead to a place inside
//! the workspace. What is left out is returned with its reason, to be said
//! once.
//!
//! Finding commands blocks: it reads folders and files. The caller runs
//! [`find`] on the blocking pool.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "finding commands reads folders and files; callers run it on the blocking pool"
)]

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::Path;

use cineris_base::{
    AbsPath, OpenError, PathResolver, ResolvedPath, location_within, open_regular,
    strip_location_prefix, visible, visible::is_selector, visible_keeping_after,
};

use crate::registration::{MAX_ARGUMENTS, check_description, check_name};
use crate::skills::FrontMatter;

/// The folder commands are in, under the configuration directory or the
/// workspace's `.cineris`.
pub const FOLDER: &str = "commands";
/// The project's folder of commands, relative to the workspace.
pub const PROJECT_FOLDER: &str = ".cineris/commands";
/// The most entries looked at in one folder of commands. Every entry
/// counts; past it the rest is not read, and that is said.
pub const MAX_ENTRIES: usize = 100;
/// The largest command file read. A larger one is left out, by its length,
/// unread.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// The most text typed after a command's name.
pub const MAX_TYPED_BYTES: usize = 16 * 1024;
/// The most text a command's body expands to.
pub const MAX_EXPANDED_BYTES: usize = 256 * 1024;
/// The longest name of a profile a command names.
pub const MAX_PROFILE_NAME: usize = 64;
/// The placeholder for everything typed after the command's name.
pub const ARGS: &str = "args";
/// The keys a command's front matter may give.
pub const KEYS: [&str; 3] = ["description", "arguments", "profile"];

/// Whose a folder of commands is. The order is the order a name is taken
/// in: the user's over a project's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Owner {
    /// The user's, in `<config dir>/commands/`.
    User,
    /// The workspace's, in `.cineris/commands/`.
    Project,
}

/// A folder to look for commands in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// Whose it is.
    pub owner: Owner,
    /// Where it is, as the session's resolver takes a path.
    pub path: String,
    /// Whether `path` lost part of the real path, which was not Unicode:
    /// the resolver takes text, and the text would name another place. Such
    /// a root is not looked in, and is said to be unreadable; for the
    /// user's, no project's command is used.
    pub lossy: bool,
}

impl Root {
    /// The workspace's `.cineris/commands/`.
    #[must_use]
    pub fn project() -> Self {
        Self {
            owner: Owner::Project,
            path: PROJECT_FOLDER.to_owned(),
            lossy: false,
        }
    }

    /// The user's `<config dir>/commands/`, [`lossy`](Self::lossy) when
    /// its path is not Unicode.
    #[must_use]
    pub fn user(config_dir: &Path) -> Self {
        let path = config_dir.join(FOLDER);
        Self {
            owner: Owner::User,
            lossy: path.to_str().is_none(),
            path: path.to_string_lossy().into_owned(),
        }
    }
}

/// An argument a command names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argument {
    /// Its name, by the registry's rules.
    pub name: String,
    /// What it is when it is not given.
    pub default: Option<String>,
}

/// One piece of a command's body.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    /// Text, sent as it is.
    Text(String),
    /// A placeholder: an argument's name, or [`ARGS`], and its default here.
    Put {
        name: String,
        default: Option<String>,
    },
}

/// A command read from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandFile {
    /// Its name: the file's, without `.md`.
    pub name: String,
    /// Whose it is.
    pub owner: Owner,
    /// Its file, relative to the workspace for a project's and under `~`
    /// for the user's, with every character that acts unseen escaped.
    pub path: String,
    /// Its description, as the front matter gives it; empty with none.
    pub description: String,
    /// The arguments its front matter names, in order.
    pub arguments: Vec<Argument>,
    /// The settings profile its front matter names, if any.
    pub profile: Option<String>,
    body: Vec<Piece>,
}

/// Something looked at and not used, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeftOut {
    /// The command's name, when the file has one; empty for a folder.
    pub name: String,
    /// Whose folder it is in.
    pub owner: Owner,
    /// The file or the folder, as [`CommandFile::path`] shows one.
    pub path: String,
    /// Why it is not used.
    pub reason: Reason,
}

/// Why a folder or a file of commands was left out.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Reason {
    /// It could not be read.
    #[error("it could not be read: {0}")]
    Unreadable(String),
    /// A project's folder that leads out of the workspace.
    #[error("it leads out of the workspace, and a project's commands are read only from inside it")]
    OutsideWorkspace,
    /// A root that is not a folder.
    #[error("it is not a folder")]
    NotAFolder,
    /// A folder with more than [`MAX_ENTRIES`] entries.
    #[error("it holds more than {MAX_ENTRIES} entries, and the rest were not looked at")]
    TooManyEntries,
    /// A name that is not valid Unicode.
    #[error("its name is not valid Unicode")]
    NotUnicode,
    /// A link: none is followed inside a folder of commands.
    #[error("it is a link, and no link is followed inside a folder of commands")]
    Link,
    /// Not a regular file: a FIFO, a device.
    #[error("it is not a regular file")]
    NotAFile,
    /// Larger than [`MAX_FILE_BYTES`].
    #[error("it is larger than 64 KiB")]
    TooLarge,
    /// Not UTF-8.
    #[error("it is not UTF-8 text")]
    NotText,
    /// Its file's name is no command's name.
    #[error("{0}")]
    BadName(String),
    /// Files of this folder whose names differ from its only in case.
    #[error(
        "the commands {0} have names that differ only in case, which on Windows are one name, so none of them is used"
    )]
    CaseClash(String),
    /// What is wrong with what it holds.
    #[error("{0}")]
    Content(String),
    /// A project's command whose name one of the user's takes.
    #[error(
        "the user has a command of this name (`{0}`), which is used: a command's text is sent as your own prompt, so a project's command never takes the name of one of yours"
    )]
    TakenByUser(String),
    /// A project's command, when a folder of the user's could not be read
    /// whole: its name could not be checked against the user's.
    #[error(
        "the user's folder of commands could not be read whole, so no project's command is used: its name could not be checked against the user's, and a project's command never takes the name of one of yours"
    )]
    UsersUnread,
}

/// What [`find`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The commands that may be used, by name.
    pub commands: Vec<CommandFile>,
    /// What was left out, and why.
    pub left_out: Vec<LeftOut>,
}

impl Found {
    /// The command called `name`, exactly.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CommandFile> {
        self.commands.iter().find(|command| command.name == name)
    }
}

/// Looks for commands in `roots` through the session's `resolver`, which
/// takes `~`, a relative path, `..` and links. The user's are taken over a
/// project's of the same name (ASCII case folded); when a folder of the
/// user's could not be read whole, no project's command is used.
///
/// It reads the folders and the `.md` files in them, and nothing else. It
/// blocks: call it on the blocking pool.
#[must_use]
pub fn find(resolver: &PathResolver, roots: &[Root]) -> Found {
    let workspace = resolver.root().to_fs_path();
    let home = resolver
        .resolve("~")
        .ok()
        .filter(ResolvedPath::exists)
        .map(|home| home.to_fs_path());
    let show = |owner: Owner, path: &Path| shown(owner, path, &workspace, home.as_deref());
    let mut commands = Vec::new();
    let mut left_out = Vec::new();
    // Every name a file of the user's takes, folded, whether it could be
    // used or not; and whether every folder of the user's was read whole,
    // without which a project's command's name cannot be checked against
    // them (L1 of the early review of step 28's commands).
    let mut users: Vec<(String, String)> = Vec::new();
    let mut users_whole = true;
    let mut roots = roots.to_vec();
    roots.sort_by_key(|root| root.owner);
    for root in &roots {
        let real = match open_root(resolver, root, &workspace, &show, &mut left_out) {
            Opened::Folder(real) => real,
            Opened::Absent => continue,
            Opened::Refused => {
                users_whole &= root.owner != Owner::User;
                continue;
            }
        };
        let mut found = look_in(root.owner, &real, &show, &mut left_out);
        if root.owner == Owner::User {
            users_whole &= found.whole;
            users.extend(
                found
                    .names
                    .iter()
                    .map(|name| (name.to_ascii_lowercase(), name.clone())),
            );
        } else {
            found.commands.retain(|command| {
                let reason = if users_whole {
                    let folded = command.name.to_ascii_lowercase();
                    match users.iter().find(|(user, _)| *user == folded) {
                        Some((_, user)) => Reason::TakenByUser(user.clone()),
                        None => return true,
                    }
                } else {
                    Reason::UsersUnread
                };
                left_out.push(LeftOut {
                    name: command.name.clone(),
                    owner: command.owner,
                    path: command.path.clone(),
                    reason,
                });
                false
            });
        }
        commands.extend(found.commands);
    }
    commands.sort_by(|one: &CommandFile, other: &CommandFile| {
        one.name
            .cmp(&other.name)
            .then_with(|| one.owner.cmp(&other.owner))
    });
    Found { commands, left_out }
}

/// `path` as it is shown: relative to the workspace for a project's, under
/// `~` for the user's, escaped.
fn shown(owner: Owner, path: &Path, workspace: &Path, home: Option<&Path>) -> String {
    let below = |base: &Path| {
        strip_location_prefix(path, base).map(|rest| {
            rest.components()
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/")
        })
    };
    let text = match owner {
        Owner::Project => below(workspace),
        Owner::User => home.and_then(below).map(|rest| format!("~/{rest}")),
    };
    visible(&text.unwrap_or_else(|| path.to_string_lossy().into_owned())).into_owned()
}

/// A root resolved and judged.
enum Opened {
    /// A folder to look in, where it really is.
    Folder(AbsPath),
    /// Nothing there: no folder, and so no names.
    Absent,
    /// Not looked in, and said why: what it holds is not known.
    Refused,
}

/// A root resolved and judged: where it really is, nothing there, or
/// refused (and said).
fn open_root(
    resolver: &PathResolver,
    root: &Root,
    workspace: &Path,
    show: &dyn Fn(Owner, &Path) -> String,
    left_out: &mut Vec<LeftOut>,
) -> Opened {
    let left = |left_out: &mut Vec<LeftOut>, path: String, reason: Reason| {
        left_out.push(LeftOut {
            name: String::new(),
            owner: root.owner,
            path,
            reason,
        });
    };
    if root.lossy {
        left(
            left_out,
            visible(&root.path).into_owned(),
            Reason::Unreadable("its path is not valid Unicode".to_owned()),
        );
        return Opened::Refused;
    }
    let resolved = match resolver.resolve(&root.path) {
        Ok(resolved) => resolved,
        Err(cineris_base::PathError::NoHome { .. }) => return Opened::Absent,
        Err(error) => {
            left(
                left_out,
                visible(&root.path).into_owned(),
                Reason::Unreadable(error.to_string()),
            );
            return Opened::Refused;
        }
    };
    if !resolved.exists() {
        return Opened::Absent;
    }
    let real = resolved.to_fs_path();
    if root.owner == Owner::Project && !location_within(&real, workspace) {
        left(
            left_out,
            visible(&root.path).into_owned(),
            Reason::OutsideWorkspace,
        );
        return Opened::Refused;
    }
    match std::fs::metadata(&real) {
        Ok(metadata) if metadata.is_dir() => Opened::Folder(resolved.abs().clone()),
        Ok(_) => {
            left(left_out, show(root.owner, &real), Reason::NotAFolder);
            Opened::Refused
        }
        Err(error) => {
            left(
                left_out,
                show(root.owner, &real),
                Reason::Unreadable(error.to_string()),
            );
            Opened::Refused
        }
    }
}

/// What one folder held.
struct InFolder {
    commands: Vec<CommandFile>,
    /// The name of every `.md` file in it, used or not.
    names: Vec<String>,
    /// Whether every entry of it was looked at: false when it could not be
    /// read, held more than [`MAX_ENTRIES`], or an entry could not be read.
    whole: bool,
}

/// The commands of one folder, by name.
fn look_in(
    owner: Owner,
    real: &AbsPath,
    show: &dyn Fn(Owner, &Path) -> String,
    left_out: &mut Vec<LeftOut>,
) -> InFolder {
    let folder = real.to_fs_path();
    let mut out = InFolder {
        commands: Vec::new(),
        names: Vec::new(),
        whole: true,
    };
    let left = |left_out: &mut Vec<LeftOut>, name: &str, path: String, reason: Reason| {
        left_out.push(LeftOut {
            name: name.to_owned(),
            owner,
            path,
            reason,
        });
    };
    let entries = match std::fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) => {
            left(
                left_out,
                "",
                show(owner, &folder),
                Reason::Unreadable(error.to_string()),
            );
            out.whole = false;
            return out;
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        if names.len() == MAX_ENTRIES {
            left(left_out, "", show(owner, &folder), Reason::TooManyEntries);
            out.whole = false;
            break;
        }
        match entry.and_then(|entry| Ok((entry.file_name(), entry.file_type()?))) {
            Ok(named) => names.push(named),
            Err(error) => {
                left(
                    left_out,
                    "",
                    show(owner, &folder),
                    Reason::Unreadable(error.to_string()),
                );
                out.whole = false;
                break;
            }
        }
    }
    names.sort_by(|one, other| one.0.cmp(&other.0));
    let mut read: Vec<CommandFile> = Vec::new();
    for (file_name, kind) in names {
        let path = folder.join(&file_name);
        let Some(text_name) = file_name.to_str() else {
            if Path::new(&file_name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
            {
                left(left_out, "", show(owner, &path), Reason::NotUnicode);
            }
            continue;
        };
        let Some(name) = command_name(text_name) else {
            continue;
        };
        if kind.is_dir() {
            continue;
        }
        // Taken whether the file can be used or not: a user's link, or a
        // file that is not UTF-8, still keeps a project's command from the
        // name.
        out.names.push(name.to_owned());
        let shown_path = show(owner, &path);
        if kind.is_symlink() {
            left(left_out, name, shown_path, Reason::Link);
            continue;
        }
        if let Err(why) = check_name(name) {
            left(left_out, name, shown_path, Reason::BadName(why));
            continue;
        }
        match read_file(&path)
            .and_then(|text| parse(name, owner, shown_path.clone(), &text).map_err(Reason::Content))
        {
            Ok(command) => read.push(command),
            Err(reason) => left(left_out, name, shown_path, reason),
        }
    }
    // Names that differ only in case: none of them is used.
    let clashes = case_clashes(&out.names);
    for command in read {
        if let Some(all) = clashes.get(&command.name.to_ascii_lowercase()) {
            let all: Vec<String> = all
                .iter()
                .map(|other| format!("`{}`", visible(other)))
                .collect();
            left(
                left_out,
                &command.name,
                command.path.clone(),
                Reason::CaseClash(all.join(", ")),
            );
        } else {
            out.commands.push(command);
        }
    }
    out
}

/// Of a folder's command names, as its listing gives them, those that
/// differ only in the case of their ASCII letters (`Deploy` and `deploy`),
/// by that name in lowercase, each with every name of it in the order
/// given. A name that no other differs from only in case is not among them.
fn case_clashes(names: &[String]) -> BTreeMap<String, Vec<&str>> {
    let mut by_fold: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for name in names {
        by_fold
            .entry(name.to_ascii_lowercase())
            .or_default()
            .push(name);
    }
    by_fold.retain(|_, same| same.len() > 1);
    by_fold
}

/// The command's name of a file's name: its stem, when it ends in `.md`
/// (in any case).
fn command_name(file_name: &str) -> Option<&str> {
    let dot = file_name.rfind('.')?;
    let (stem, extension) = (&file_name[..dot], &file_name[dot + 1..]);
    (extension.eq_ignore_ascii_case("md") && !stem.is_empty()).then_some(stem)
}

/// Reads a command file, opened so that nothing at its name but a regular
/// file is read: one over [`MAX_FILE_BYTES`] by its length is refused
/// unread.
fn read_file(path: &Path) -> Result<String, Reason> {
    let (file, metadata) = match open_regular(path) {
        Ok(opened) => opened,
        Err(OpenError::Link) => return Err(Reason::Link),
        Err(OpenError::Directory | OpenError::NotAFile) => return Err(Reason::NotAFile),
        Err(error) => return Err(Reason::Unreadable(error.to_string())),
    };
    if metadata.len() > MAX_FILE_BYTES {
        return Err(Reason::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Reason::Unreadable(error.to_string()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(Reason::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| Reason::NotText)
}

/// The front matter's text and the body after it: `None` for the front
/// matter when the file does not begin with a `---` line; an error when
/// the block is never closed.
fn split(text: &str) -> Result<(Option<&str>, &str), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let first_end = text.find('\n').map_or(text.len(), |at| at + 1);
    if text[..first_end].trim_end() != "---" {
        return Ok((None, text));
    }
    let mut at = first_end;
    while at < text.len() {
        let end = text[at..]
            .find('\n')
            .map_or(text.len(), |found| at + found + 1);
        if matches!(text[at..end].trim_end(), "---" | "...") {
            return Ok((Some(&text[..end]), &text[end..]));
        }
        at = end;
    }
    Err("its front matter is never closed: a `---` line must end it".to_owned())
}

/// A command from its file's text.
///
/// # Errors
///
/// Why the file is not a command, in a clause.
fn parse(name: &str, owner: Owner, path: String, text: &str) -> Result<CommandFile, String> {
    let (front, body) = split(text)?;
    let mut description = String::new();
    let mut arguments = Vec::new();
    let mut profile = None;
    if let Some(front) = front {
        let front = FrontMatter::read(front)
            .ok_or_else(|| "its front matter could not be read".to_owned())?;
        if let Some(key) = front.repeated().first() {
            return Err(format!("its front matter gives `{}` twice", visible(key)));
        }
        if let Some(line) = front.unread().first() {
            return Err(format!(
                "its front matter has a line that is no `key: value` field: `{}`",
                visible(line)
            ));
        }
        for (key, value) in front.fields() {
            match key {
                "description" => {
                    check_description(value).map_err(|why| format!("its `description`: {why}"))?;
                    value.clone_into(&mut description);
                }
                "arguments" => arguments = read_arguments(value)?,
                "profile" => profile = Some(read_profile(value)?),
                other => {
                    return Err(format!(
                        "its front matter has the key `{}`, which a command does not take: only \
                         `description`, `arguments` and `profile`",
                        visible(other)
                    ));
                }
            }
        }
    }
    // A project's command is sent as the user's own prompt: its variation
    // selectors are taken out of its body and its arguments' defaults, an
    // emoji's too, since one after each character can carry a byte unseen
    // and a run any bytes (the fixes to step 28). The user's own are sent
    // as they wrote them.
    let without_selectors;
    let body = if owner == Owner::Project && body.chars().any(is_selector) {
        without_selectors = body
            .chars()
            .filter(|character| !is_selector(*character))
            .collect::<String>();
        without_selectors.as_str()
    } else {
        body
    };
    if owner == Owner::Project {
        for default in arguments
            .iter_mut()
            .filter_map(|argument: &mut Argument| argument.default.as_mut())
        {
            default.retain(|character| !is_selector(character));
        }
        // What else it sends is seen in its file (L4 of the early review of
        // step 28's commands).
        if let Some(unseen) = first_unseen(body) {
            return Err(format!(
                "its body holds `{unseen}`, {UNSEEN}: a project's command is sent as your own \
                 prompt, so it may hold nothing you would not see"
            ));
        }
        let hidden = arguments.iter().find_map(|argument: &Argument| {
            Some((&argument.name, first_unseen(argument.default.as_deref()?)?))
        });
        if let Some((name, unseen)) = hidden {
            return Err(format!(
                "its `arguments` give `{name}` a default that holds `{unseen}`, {UNSEEN}: a \
                 project's command is sent as your own prompt, so it may hold nothing you would \
                 not see"
            ));
        }
    }
    let body = template(body)?;
    for piece in &body {
        if let Piece::Put { name: used, .. } = piece
            && used != ARGS
            && !arguments
                .iter()
                .any(|argument: &Argument| argument.name == *used)
        {
            return Err(format!(
                "its body puts in `{{{{{used}}}}}`, which its front matter's `arguments` do not \
                 name"
            ));
        }
    }
    let empty = body.iter().all(|piece| match piece {
        Piece::Text(text) => text.trim().is_empty(),
        Piece::Put { .. } => false,
    });
    if empty {
        return Err("its body is empty: there is nothing to send".to_owned());
    }
    Ok(CommandFile {
        name: name.to_owned(),
        owner,
        path,
        description,
        arguments,
        profile,
        body,
    })
}

/// What a character that acts unseen is, in a reason.
const UNSEEN: &str = "a character that acts unseen (a control or a format character)";

/// The first character of `text` that acts unseen, written as an escape
/// ([`visible`]): what [`check_description`] refuses in a description, a
/// line break and a tab apart, where a line break is `\n` or `\r\n` (a
/// checkout on Windows may end its lines so). A `\r` alone is one.
fn first_unseen(text: &str) -> Option<String> {
    let keep = ['\n', '\t'];
    // The character before a line is the line break's `\n`, as in the text.
    let mut before = None;
    for line in text.split("\r\n") {
        let shown = visible_keeping_after(before, line, &keep);
        if shown != line {
            // Everything before the first escape is the text as it is.
            let at = line
                .bytes()
                .zip(shown.bytes())
                .take_while(|(one, other)| one == other)
                .count();
            let unseen = line[at..].chars().next()?;
            return Some(visible(unseen.encode_utf8(&mut [0; 4])).into_owned());
        }
        before = Some('\n');
    }
    None
}

/// The `arguments` field: a list of names, each with a default in double
/// quotes if it has one, in brackets or not: `[focus, depth = "shallow"]`.
fn read_arguments(value: &str) -> Result<Vec<Argument>, String> {
    let value = value.trim();
    let inner = match value.strip_prefix('[') {
        Some(rest) => rest
            .strip_suffix(']')
            .ok_or_else(|| "its `arguments` open a `[` and never close it".to_owned())?,
        None => value,
    };
    let mut arguments: Vec<Argument> = Vec::new();
    for item in split_items(inner)? {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let (name, default) = match item.split_once('=') {
            Some((name, default)) => (name.trim(), Some(quoted_default(default.trim())?)),
            None => (item, None),
        };
        check_name(name).map_err(|why| format!("its `arguments`: {why}"))?;
        if name == ARGS {
            return Err(format!(
                "its `arguments` name `{ARGS}`, which is everything typed after the command"
            ));
        }
        if arguments.iter().any(|earlier| earlier.name == name) {
            return Err(format!("its `arguments` name `{name}` twice"));
        }
        arguments.push(Argument {
            name: name.to_owned(),
            default,
        });
    }
    if arguments.len() > MAX_ARGUMENTS {
        return Err(format!("its `arguments` name more than {MAX_ARGUMENTS}"));
    }
    Ok(arguments)
}

/// The items of a list, split at the commas outside double quotes.
fn split_items(inner: &str) -> Result<Vec<String>, String> {
    let mut items = Vec::new();
    let mut item = String::new();
    let mut quoted = false;
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' if quoted => {
                item.push('\\');
                if let Some(next) = chars.next() {
                    item.push(next);
                }
            }
            '"' => {
                quoted = !quoted;
                item.push('"');
            }
            ',' if !quoted => items.push(std::mem::take(&mut item)),
            other => item.push(other),
        }
    }
    if quoted {
        return Err("its `arguments` open a quote and never close it".to_owned());
    }
    items.push(item);
    Ok(items)
}

/// A default in double quotes, its `\"` and `\\` read.
fn quoted_default(text: &str) -> Result<String, String> {
    let inner = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| {
            format!(
                "its `arguments` give the default `{}`, which is not in double quotes",
                visible(text)
            )
        })?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        if character == '\\' {
            match chars.next() {
                Some(escaped @ ('"' | '\\')) => out.push(escaped),
                _ => return Err("a default's `\\` comes before `\"` or `\\` alone".to_owned()),
            }
        } else {
            out.push(character);
        }
    }
    Ok(out)
}

/// The `profile` field: a profile's name, by the registry's name rules.
fn read_profile(value: &str) -> Result<String, String> {
    let name = value.trim();
    let fine = !name.is_empty()
        && name.len() <= MAX_PROFILE_NAME
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        });
    if fine {
        Ok(name.to_owned())
    } else {
        Err(format!(
            "its `profile` is `{}`, which is no profile's name: 1 to 64 of ASCII letters, \
             digits, `_`, `-` and `.`",
            visible(name)
        ))
    }
}

/// A body read into its pieces: `{{name}}`, `{{name | "default"}}` and
/// `{{args}}` are placeholders, `\{{` is a `{{` of the text, and any other
/// `{{` refuses the file.
fn template(body: &str) -> Result<Vec<Piece>, String> {
    let mut pieces = Vec::new();
    let mut text = String::new();
    let mut rest = body;
    while let Some(at) = rest.find("{{") {
        if rest[..at].ends_with('\\') {
            text.push_str(&rest[..at - 1]);
            text.push_str("{{");
            rest = &rest[at + 2..];
            continue;
        }
        text.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let (put, used) = placeholder(after).ok_or_else(|| {
            let line = body[..body.len() - rest.len() + at].matches('\n').count() + 1;
            let shown: String = rest[at..]
                .chars()
                .take(40)
                .take_while(|c| *c != '\n')
                .collect();
            format!(
                "its body has `{}` on line {line}, which is no placeholder: write \
                 `{{{{name}}}}`, `{{{{name | \"default\"}}}}` or `{{{{args}}}}`, or `\\{{{{` for \
                 a `{{{{` of the text",
                visible(&shown)
            )
        })?;
        if !text.is_empty() {
            pieces.push(Piece::Text(std::mem::take(&mut text)));
        }
        pieces.push(put);
        rest = &after[used..];
    }
    text.push_str(rest);
    if !text.is_empty() {
        pieces.push(Piece::Text(text));
    }
    Ok(pieces)
}

/// A placeholder after its `{{`: the piece and how much of `after` it
/// used, its `}}` included.
fn placeholder(after: &str) -> Option<(Piece, usize)> {
    let end = after.find("}}")?;
    let inside = &after[..end];
    if inside.contains('\n') {
        return None;
    }
    let (name, default) = match inside.split_once('|') {
        Some((name, default)) => (name.trim(), Some(quoted_default(default.trim()).ok()?)),
        None => (inside.trim(), None),
    };
    check_name(name).ok()?;
    Some((
        Piece::Put {
            name: name.to_owned(),
            default,
        },
        end + 2,
    ))
}

/// Why what was typed after a command does not fit it: a sentence.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ArgumentError(pub String);

/// What was typed after a command, given to its arguments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assigned {
    /// Each argument given, by name, as it was given.
    pub values: Vec<(String, String)>,
    /// The words given by position past the arguments, in order.
    pub rest: Vec<String>,
}

impl Assigned {
    /// The value given for `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(given, _)| given == name)
            .map(|(_, value)| value.as_str())
    }
}

/// Gives what was typed (`typed`) to `arguments`: each word in turn to the
/// next argument not given by name, and `name=value` to the argument of
/// that name (a word whose part before `=` names none is a word like any
/// other). A word may be in double quotes, with `\"` and `\\` read in them.
///
/// # Errors
///
/// When more than [`MAX_TYPED_BYTES`] was typed, a quote is not closed, or
/// an argument is given twice.
pub fn assign(arguments: &[&str], typed: &str) -> Result<Assigned, ArgumentError> {
    if typed.len() > MAX_TYPED_BYTES {
        return Err(ArgumentError(format!(
            "more than {} KiB was typed after the command",
            MAX_TYPED_BYTES / 1024
        )));
    }
    let mut out = Assigned::default();
    let mut positional = Vec::new();
    for (word, named_at) in words(typed)? {
        if let Some(at) = named_at
            && arguments.contains(&&word[..at])
        {
            let name = &word[..at];
            if out.get(name).is_some() {
                return Err(ArgumentError(format!(
                    "the argument `{name}` is given twice"
                )));
            }
            out.values
                .push((name.to_owned(), word[at + 1..].to_owned()));
        } else {
            positional.push(word);
        }
    }
    let mut free = arguments
        .iter()
        .filter(|name| out.get(name).is_none())
        .copied()
        .collect::<Vec<_>>()
        .into_iter();
    for word in positional {
        match free.next() {
            Some(name) => out.values.push((name.to_owned(), word)),
            None => out.rest.push(word),
        }
    }
    Ok(out)
}

/// The words of `typed`, each with where its first `=` outside quotes is,
/// when it has one before any quote.
fn words(typed: &str) -> Result<Vec<(String, Option<usize>)>, ArgumentError> {
    let mut words = Vec::new();
    let mut chars = typed.chars().peekable();
    loop {
        while chars
            .next_if(|character| character.is_whitespace())
            .is_some()
        {}
        if chars.peek().is_none() {
            return Ok(words);
        }
        let mut word = String::new();
        let mut named_at = None;
        let mut quoted_yet = false;
        while let Some(&character) = chars.peek() {
            if character.is_whitespace() {
                break;
            }
            chars.next();
            match character {
                '"' => {
                    quoted_yet = true;
                    loop {
                        match chars.next() {
                            None => {
                                return Err(ArgumentError(
                                    "a `\"` typed after the command is never closed".to_owned(),
                                ));
                            }
                            Some('"') => break,
                            Some('\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                                word.extend(chars.next());
                            }
                            Some(other) => word.push(other),
                        }
                    }
                }
                '=' if named_at.is_none() && !quoted_yet => {
                    named_at = Some(word.len());
                    word.push('=');
                }
                other => word.push(other),
            }
        }
        words.push((word, named_at));
    }
}

impl CommandFile {
    /// The text the command sends for `typed`, what was typed after its
    /// name: its body with each placeholder put in. `{{args}}` is `typed`,
    /// trimmed; an argument is the value given, or the placeholder's
    /// default, or the front matter's. What is put in is not read again.
    ///
    /// # Errors
    ///
    /// When `typed` does not fit the command's arguments ([`assign`]); when
    /// more words are given than it has arguments and its body does not
    /// put in `{{args}}`; when an argument it puts in without a default is
    /// not given; and when the text would be empty or larger than
    /// [`MAX_EXPANDED_BYTES`].
    pub fn expand(&self, typed: &str) -> Result<String, ArgumentError> {
        let names: Vec<&str> = self.arguments.iter().map(|a| a.name.as_str()).collect();
        let assigned = assign(&names, typed)?;
        let takes_args = self
            .body
            .iter()
            .any(|piece| matches!(piece, Piece::Put { name, .. } if name == ARGS));
        if !assigned.rest.is_empty() && !takes_args {
            return Err(ArgumentError(format!(
                "`/{}` takes {}, and was given {} more: {}",
                self.name,
                match names.len() {
                    0 => "no argument".to_owned(),
                    1 => format!("one argument, `{}`", names[0]),
                    count => format!("{count} arguments"),
                },
                assigned.rest.len(),
                assigned
                    .rest
                    .iter()
                    .map(|word| format!("`{}`", visible(word)))
                    .collect::<Vec<_>>()
                    .join(" ")
            )));
        }
        let mut out = String::new();
        for piece in &self.body {
            let put = match piece {
                Piece::Text(text) => text.as_str(),
                Piece::Put { name, default } if name == ARGS => {
                    let typed = typed.trim();
                    match default {
                        Some(default) if typed.is_empty() => default.as_str(),
                        _ => typed,
                    }
                }
                Piece::Put { name, default } => {
                    let declared = self
                        .arguments
                        .iter()
                        .find(|argument| argument.name == *name)
                        .and_then(|argument| argument.default.as_deref());
                    assigned
                        .get(name)
                        .or(default.as_deref())
                        .or(declared)
                        .ok_or_else(|| {
                            ArgumentError(format!(
                                "`/{}` needs the argument `{name}`: give it after the name, or \
                                 as `{name}=…`",
                                self.name
                            ))
                        })?
                }
            };
            if out.len() + put.len() > MAX_EXPANDED_BYTES {
                return Err(ArgumentError(format!(
                    "`/{}` would send more than {} KiB",
                    self.name,
                    MAX_EXPANDED_BYTES / 1024
                )));
            }
            out.push_str(put);
        }
        if out.trim().is_empty() {
            return Err(ArgumentError(format!(
                "`/{}` gave nothing to send",
                self.name
            )));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
