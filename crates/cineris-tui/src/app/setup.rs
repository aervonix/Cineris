//! What the first run asks of the program (since step 25;
//! `docs/architecture/08-configuration.md`, "First run";
//! `docs/architecture/09-tui.md`, "The first run").
//!
//! The steps that write the person's own files (the key, the configuration
//! file) cannot be operations of the core's protocol without letting any
//! client write them, so the program gives the application a [`Setup`],
//! as it gives a `Restyle`: the providers and where each one's key would
//! come from, storing a key, what other tools left, reading one file the
//! person picked, writing the configuration. `cineris attach` gives none,
//! and never runs the first run: the server it attaches to has its
//! configuration.
//!
//! **None of it runs on the interface's thread.** Each call goes to a
//! [`Worker`] of its own, which wakes the loop with the terminal's waker
//! when it answers; a step shows `reading…` until its answer comes.
//!
//! **What comes back holds no secret.** A key in the environment is given
//! by its variable's name, an MCP server's environment and headers by
//! their names, and a server's command with every part that looked like a
//! credential masked already, the server refused: the program's reader
//! does that before anything reaches this crate.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};

use cineris_base::Secret;

/// The providers the core knows, for the first step.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Providers {
    /// Each provider, in the core's order.
    pub providers: Vec<Provider>,
    /// Where a key typed here is kept.
    pub store: KeyStore,
    /// Whether a session started now would be confined: the sandbox
    /// enforces on this machine and the configuration in force leaves it
    /// on (since the step 25 fixes). `autonomous` is offered only then.
    pub sandbox: bool,
}

/// One provider, and where its key would come from now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    /// Its name.
    pub name: String,
    /// The environment variable that holds a key for it, when one is set:
    /// its name, never its value.
    pub variable: Option<String>,
    /// Whether the credential store holds a key for it already.
    pub stored: bool,
    /// The model written as `model.main` when it is chosen: the one the
    /// model catalog marks as its default. `None` when it marks none (since
    /// the full review of step 25's third pull request).
    pub model: Option<String>,
    /// Whether another provider has a model of [`model`](Self::model)'s id
    /// too: the provider is then named beside it as the main role's,
    /// `model.roles.main.provider`, and otherwise not (since the review of
    /// the fixes of step 25's third pull request: named always, it held a
    /// later `model.main` to this provider's models).
    pub model_shared: bool,
}

/// Where a key typed at the first run is kept: the credential store
/// `cineris auth login` uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyStore {
    /// The owner-only file, as the person reads its path.
    File(String),
    /// Windows Credential Manager, for this machine only.
    CredentialManager,
    /// No store can be used here: why.
    Unavailable(String),
}

impl Default for KeyStore {
    fn default() -> Self {
        Self::Unavailable("no credential store was named".to_owned())
    }
}

impl KeyStore {
    /// Where a key is kept, as the end of a sentence (`It is stored …`).
    #[must_use]
    pub fn kept(&self) -> String {
        match self {
            Self::File(path) => format!(
                "in {}, which only you can read",
                crate::view::one_line(path)
            ),
            Self::CredentialManager => {
                "in Windows Credential Manager, for this machine only".to_owned()
            }
            Self::Unavailable(_) => "nowhere: no credential store can be used here".to_owned(),
        }
    }

    /// Whether the store keeps only printable ASCII.
    #[must_use]
    pub fn ascii_only(&self) -> bool {
        matches!(self, Self::CredentialManager)
    }
}

/// What other tools left, as discovery reports it: nothing was opened.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The user's MCP files of other clients, each read only when the
    /// person asks to see inside it.
    pub user_mcp: Vec<McpFile>,
    /// The project's own MCP files, shown and never imported.
    pub project_mcp: Vec<String>,
}

/// Another client's MCP file of the user's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpFile {
    /// Its path, as the person reads it (`~/.claude.json`): what
    /// [`Setup::read_servers`] is given back.
    pub path: String,
    /// The tool it is whose (`Claude Code`).
    pub from: String,
}

/// One server of another client's file, as the program's reader gave it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    /// Its name in that file.
    pub name: String,
    /// Its command line, arguments and all, or its address: whole, each
    /// argument's bounds shown, with every part that looked like a
    /// credential masked.
    pub shown: String,
    /// Whether `shown` masks a part that is imported as it is in its file
    /// (a value whose name says it is a credential): the row says so (since
    /// the full review of step 25's third pull request).
    pub hidden: bool,
    /// The names of the environment variables it is given, each taken from
    /// the variable of the same name once imported.
    pub env: Vec<String>,
    /// Each header it sends, by name, and the variable its value is taken
    /// from once imported.
    pub headers: Vec<(String, String)>,
    /// Why it cannot be imported, as words to end a row: `holds a
    /// credential in its command`.
    pub refused: Option<String>,
}

/// What the steps before the session chose, for the configuration file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Chosen {
    /// The provider whose key was stored or found; `None`, skipped.
    pub provider: Option<String>,
    /// Its model, written as `model.main` ([`Provider::model`]).
    pub model: Option<String>,
    /// Whether another provider has a model of its model's id too
    /// ([`Provider::model_shared`]): the provider is then named beside it.
    pub model_shared: bool,
    /// What the imports picked; `None`, the step was skipped.
    pub imports: Option<Imported>,
    /// `permissions.mode`; `None`, skipped (supervised holds).
    pub mode: Option<String>,
    /// `permissions.read_skill_folders`: whether the user's own skill
    /// folders are read without asking. `false` unless the person said yes:
    /// a skip is no (since step 28's third pull request, the owner's
    /// answer).
    pub read_skill_folders: bool,
}

/// The question of whether the user's own skill folders are read without
/// asking, as the first run and `cineris setup skill-folders` put it (the
/// owner's answer, `docs/steps/28/answered.md`: no is the default, and it
/// says plainly what a yes sends).
pub const SKILL_FOLDERS_QUESTION: &str = "May the agent read your own skill folders, outside the \
     project, without asking you first? With yes, any file in them, secrets kept there included, \
     is sent to the model's provider without a question, and some providers keep what they are \
     sent. `cineris setup skill-folders` changes it later.";

/// The answers to [`SKILL_FOLDERS_QUESTION`], no first: each with what it
/// means.
pub const SKILL_FOLDERS_ANSWERS: [(&str, &str); 2] = [
    (
        "no",
        "a read of a file in your own skill folders is asked about, as any read outside the \
         project is",
    ),
    (
        "yes",
        "your own skill folders are read without asking, and what is read, secrets included, \
         goes to the model's provider",
    ),
];

/// What the imports step picked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    /// `compat.instructions`.
    pub instructions: bool,
    /// `compat.skills`.
    pub skills: bool,
    /// The servers picked: each one's file, as [`McpFile::path`], and its
    /// name.
    pub servers: Vec<(String, String)>,
}

/// What the program gives the application for the first run. Every call
/// may block on the disk: they run on the [`Worker`].
pub trait Setup: Send + Sync {
    /// The providers, where each one's key would come from, where a key is
    /// kept, and whether the sandbox enforces.
    ///
    /// # Errors
    ///
    /// A sentence, when the core cannot say.
    fn providers(&self) -> Result<Providers, String>;

    /// Keeps `key` for `provider` in the credential store.
    ///
    /// # Errors
    ///
    /// A sentence, when the store refuses.
    fn store_key(&self, provider: &str, key: Secret) -> Result<(), String>;

    /// What other tools left: discovery, which opens nothing.
    fn discover(&self) -> Found;

    /// The servers of the file at `path` (an [`McpFile::path`]), read now
    /// because the person asked to see inside it.
    ///
    /// # Errors
    ///
    /// A sentence, when it cannot be read or is not understood.
    fn read_servers(&self, path: &str) -> Result<Vec<McpServer>, String>;

    /// Writes the user's configuration file, new, with what was chosen.
    /// Returns its path as the person reads it.
    ///
    /// # Errors
    ///
    /// [`NotWritten`]: a file is there already, which was left as it is,
    /// or it could not be written.
    fn write_config(&self, chosen: &Chosen) -> Result<String, NotWritten>;
}

/// Why the configuration file was not written (since the review of the
/// fixes of step 25's third pull request: a file found at its name was
/// said to be nothing saved, and the first run to run again).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotWritten {
    /// A file was at its name when it came to be written, made meanwhile:
    /// it was left as it is, and it is the configuration from the next
    /// start. Its path, as the person reads it.
    Found(String),
    /// It could not be written: a sentence saying why.
    Failed(String),
}

/// A call for the worker.
pub(crate) enum Job {
    Providers,
    StoreKey(String, Secret),
    Discover,
    Read(String),
    Write(Chosen),
}

/// The worker's answer to a [`Job`].
#[derive(Debug)]
pub(crate) enum Answer {
    Providers(Result<Providers, String>),
    Stored(String, Result<(), String>),
    Found(Found),
    Servers(String, Result<Vec<McpServer>, String>),
    Written(Result<String, NotWritten>),
}

/// The thread [`Setup`]'s calls run on, one at a time in the order asked.
pub(crate) struct Worker {
    jobs: Sender<Job>,
    answers: Receiver<Answer>,
    waker: Arc<Mutex<Option<cineris_term::Waker>>>,
    /// How many jobs were asked, and how many answers taken: the worker
    /// owes the difference.
    asked: std::sync::atomic::AtomicUsize,
    taken: std::sync::atomic::AtomicUsize,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker").finish_non_exhaustive()
    }
}

/// What a call of [`Setup`]'s that panicked answers: the step that waits
/// for it goes on, and says so (since the full review of step 25's third
/// pull request: `storing…` stayed for good).
pub(crate) const PANICKED: &str = "it failed inside the program; the log says how";

/// `call`, or [`PANICKED`] when it panics: a panic on the worker is the
/// program's to report (its hook logs it), not a reason for a step to wait
/// for good.
fn guarded<T>(call: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).map_err(|_| PANICKED.to_owned())
}

#[cfg(test)]
thread_local! {
    /// The worker's thread is refused on this thread: for the test of a
    /// first run whose worker cannot start.
    pub(crate) static REFUSE_THREAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

impl Worker {
    /// Starts the worker on `setup`.
    ///
    /// # Errors
    ///
    /// Why the thread could not be started: the first run is then not run,
    /// said.
    pub(crate) fn start(setup: Arc<dyn Setup>) -> std::io::Result<Self> {
        #[cfg(test)]
        if REFUSE_THREAD.with(std::cell::Cell::get) {
            return Err(std::io::Error::other("no thread could be made"));
        }
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (outbox, answers) = mpsc::channel();
        let waker: Arc<Mutex<Option<cineris_term::Waker>>> = Arc::default();
        let wake = Arc::clone(&waker);
        std::thread::Builder::new()
            .name("cineris-setup".to_owned())
            .spawn(move || {
                while let Ok(job) = inbox.recv() {
                    let answer = match job {
                        Job::Providers => {
                            Answer::Providers(guarded(|| setup.providers()).and_then(|got| got))
                        }
                        Job::StoreKey(provider, key) => {
                            let stored =
                                guarded(|| setup.store_key(&provider, key)).and_then(|got| got);
                            Answer::Stored(provider, stored)
                        }
                        // Discovery that failed found nothing.
                        Job::Discover => {
                            Answer::Found(guarded(|| setup.discover()).unwrap_or_default())
                        }
                        Job::Read(path) => {
                            let read = guarded(|| setup.read_servers(&path)).and_then(|got| got);
                            Answer::Servers(path, read)
                        }
                        Job::Write(chosen) => Answer::Written(
                            guarded(|| setup.write_config(&chosen))
                                .map_err(NotWritten::Failed)
                                .and_then(|got| got),
                        ),
                    };
                    if outbox.send(answer).is_err() {
                        return;
                    }
                    if let Some(waker) =
                        wake.lock().unwrap_or_else(PoisonError::into_inner).as_ref()
                    {
                        waker.wake();
                    }
                }
            })?;
        Ok(Self {
            jobs,
            answers,
            waker,
            asked: std::sync::atomic::AtomicUsize::new(0),
            taken: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// The terminal's waker, which the worker wakes the loop with.
    pub(crate) fn set_waker(&self, waker: cineris_term::Waker) {
        *self.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// Hands `job` to the worker.
    pub(crate) fn ask(&self, job: Job) {
        if self.jobs.send(job).is_ok() {
            self.asked
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// An answer that has come, if one has.
    pub(crate) fn answer(&self) -> Option<Answer> {
        let answer = self.answers.try_recv().ok();
        if answer.is_some() {
            self.taken
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        answer
    }

    /// Waits at most `limit` for an answer the worker owes: for tests,
    /// which have no loop to wake. `None` when it owes none.
    #[cfg(any(test, feature = "test-kit"))]
    pub(crate) fn wait(&self, limit: std::time::Duration) -> Option<Answer> {
        use std::sync::atomic::Ordering::Relaxed;
        if self.taken.load(Relaxed) >= self.asked.load(Relaxed) {
            return None;
        }
        let answer = self.answers.recv_timeout(limit).ok();
        if answer.is_some() {
            self.taken.fetch_add(1, Relaxed);
        }
        answer
    }

    /// Whether every job asked has been answered and taken.
    #[cfg(any(test, feature = "test-kit"))]
    pub(crate) fn settled(&self) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        self.taken.load(Relaxed) >= self.asked.load(Relaxed)
    }
}
