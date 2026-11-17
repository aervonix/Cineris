//! `cortex attach`: the interactive application on a running
//! `serve --socket`, with no core of its own (`docs/steps/27/answered.md`,
//! point 5): the same application as `cortex`, through a second
//! [`Link`], over the RPC connection.
//!
//! What the in-process link does in memory crosses the wire here:
//!
//! - **An operation** is a JSON-RPC request; the server's lane for this
//!   connection orders it as the in-process lane does, and its response is
//!   matched back to the operation's ticket by the request's id.
//! - **Events and requests** (`session/event` notifications, the server's
//!   requests) are put by the link's thread into an [`EventQueue`] of its
//!   own, the core's type, so that deltas merge and give way to an
//!   `item.resync` for an interface that is behind (an editor open while a
//!   turn streams) as they do in process, and the server's rule for a
//!   client that takes nothing never meets an `attach` whose person is away.
//!   A `session/read`'s result fences the queue at its `seq`, as the core
//!   fences the in-process queue, so that no delta after the read is merged
//!   into one before it.
//! - **A result** is put with the others in the order it arrived, and the
//!   interface takes it in [`Order`], after the events that came before it.
//! - **An answer** to the server's request is a JSON-RPC response; one the
//!   server refuses (the request was settled meanwhile) it drops.
//! - **The end:** when the connection closes, every operation still asked is
//!   answered with an error, and the interface is told once. `attach`
//!   going away is a disconnect: the server keeps the session.
//!
//! The link's reading and writing run on a thread of its own with a
//! current-thread runtime (a named pipe's I/O is overlapped, so a blocking
//! read and write on two threads would wait on each other), and wake the
//! terminal's waker when something arrives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use cortex_base::SecretFilter;
use cortex_config::{Dirs, Env, LoadOptions, StateDir};
use cortex_core::EventQueue;
use cortex_protocol::jsonrpc::{ErrorObject, Message, Request, RequestId, Response};
use cortex_protocol::methods::{
    ClientCapabilities, ClientRole, InitializeParams, Op, OpResult, PROTOCOL_VERSION, PeerInfo,
    ServerNotification, ServerRequest, ServerResponse,
};
use cortex_protocol::session::SessionSettings;
use cortex_protocol::{ErrorData, ErrorKind, SessionId};
use cortex_rpc::order::Order;
use cortex_rpc::{Place, Transport};
use cortex_tui::app::{self, Exit, Inbound, Link, Open, Start, Ticket};
use cortex_tui::highlight::Highlighter;
use tokio::sync::mpsc;

use crate::args::{Attach, Choice, Run};
use crate::exit;
use crate::interactive::{self, Done, Slot};

/// The longest line the server may send before the connection is taken
/// for broken. A page of a history read is far under it.
const LONGEST_LINE: usize = 64 * 1024 * 1024;

/// How long the log is given to reach its file at the end.
const LOG_END: Duration = Duration::from_millis(300);

/// Runs the interactive application on the server. Returns the exit code.
pub(crate) fn run(attach: Attach, name: &'static str) -> u8 {
    let env = Env::from_process();
    let here = match crate::workspace::Here::now() {
        Ok(here) => here,
        Err(error) => {
            eprintln!("{name}: the working directory could not be read: {error}");
            return exit::USAGE;
        }
    };
    let workspace = here.root.clone();
    // The user's configuration, for the interface's own settings and the
    // log: the server reads its own for the sessions.
    let loaded = match cortex_config::load(&LoadOptions::new(env.clone())) {
        Ok(loaded) => loaded,
        Err(failure) => {
            eprintln!("{name}: {}", crate::visible(&failure.render(&env)));
            return exit::USAGE;
        }
    };
    let secrets = Arc::new(SecretFilter::new());
    let logging = Run {
        log_level: attach.log_level.clone(),
        ..Run::default()
    };
    let log = interactive::start_log(&logging, &env, &loaded, &secrets);
    let logs = StateDir::from_dirs(&Dirs::from_env(&env)).map(|state| state.logs_dir());
    interactive::install_panic_hook(logs, secrets);

    let place = match &attach.socket {
        Some(named) => Place::named(named),
        None => match Place::default_for(env.get("XDG_RUNTIME_DIR").map(Path::new)) {
            Ok(place) => place,
            Err(error) => {
                eprintln!("{name}: the server's default place is not known: {error}");
                return exit::USAGE;
            }
        },
    };
    let link = match RpcLink::start(&place) {
        Ok(link) => link,
        Err(said) => {
            eprintln!("{name}: {}", crate::visible(&said));
            return exit::USAGE;
        }
    };

    let mut notices = Vec::new();
    let settings = interactive::tui_settings(&loaded.settings.tui, &mut notices);
    let (keymap, errors) = interactive::keymap(&loaded.settings.keys);
    notices.extend(errors);
    notices.extend(here.passed_over_said());
    let overrides = interactive::overrides(&loaded.settings.terminal);
    let folder = interactive::shown_folder(&workspace, env.home().as_deref());
    let open = open(&attach.choice, &workspace);
    let root = workspace.to_string_lossy().into_owned();
    let history = interactive::history_for(&env, &workspace);
    let prompt = attach.prompt.clone();

    let ended = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        app::interactive(overrides, move |waker, _caps| {
            link.slot.set(waker.clone());
            let wake = waker.clone();
            let start = Start {
                version: crate::VERSION.to_owned(),
                folder,
                workspace: root,
                cwd: here
                    .cwd
                    .as_ref()
                    .and_then(|cwd| cwd.to_str().map(str::to_owned)),
                open,
                prompt,
                settings,
                keymap,
                notices,
                history,
                highlighter: Highlighter::new(move || wake.wake()),
                // The server's `config/get` reads the same user's
                // configuration as the in-process core does.
                restyle: Some(Box::new(interactive::restyle)),
            };
            (Box::new(link) as Box<dyn Link>, start)
        })
    }));
    if let Some(log) = log {
        log.finish_within(LOG_END);
    }
    let ended = match (ended, app::signalled()) {
        (Ok(Ok(Exit::Quit | Exit::Gone(_))), Some(code)) => Ok(Ok(Exit::Signal(code))),
        (ended, _) => ended,
    };
    let (code, said) = interactive::ending(
        ended,
        interactive::CRASH.get().map(PathBuf::as_path),
        name,
        interactive::cortex_tui_terminal,
    );
    if let Some(said) = said {
        eprintln!("{said}");
    }
    cortex_tui::app::signal_ended();
    code
}

/// The session `attach` opens: a new one for the workspace, its latest, or
/// one by its id.
fn open(choice: &Choice, workspace: &Path) -> Open {
    let workspace = workspace.to_string_lossy().into_owned();
    match choice {
        Choice::Continue => Open::Continue {
            workspace,
            settings: SessionSettings::default(),
            ephemeral: false,
        },
        Choice::Resume(id) => Open::Resume(SessionId::from(id.as_str())),
        Choice::Fork(id) => Open::Fork(SessionId::from(id.as_str())),
        Choice::New => Open::New {
            workspace,
            settings: SessionSettings::default(),
            ephemeral: false,
        },
    }
}

/// What the interface sends: an operation, or an answer to the server.
enum Out {
    Op(Ticket, Op),
    Answer(RequestId, ServerResponse),
}

/// What a request asked is waiting for, by its id on the wire.
struct Asked {
    ticket: Ticket,
    method: &'static str,
    /// The session whose queue the result fences (a history read) or
    /// opens again (an open, a close).
    session: Option<SessionId>,
}

type AskedMap = Arc<Mutex<HashMap<i64, Asked>>>;

/// The RPC link. See the [module documentation](self).
pub(crate) struct RpcLink {
    out: mpsc::UnboundedSender<Out>,
    queue: Arc<EventQueue>,
    done: Done,
    pub(crate) slot: Slot,
    next: u64,
    order: Order,
    gone_said: bool,
}

impl RpcLink {
    /// Reaches the server at `place`, on a thread of its own, and says
    /// `initialize`; returns once the server has answered, or with what
    /// went wrong.
    fn start(place: &Place) -> Result<Self, String> {
        let queue = Arc::new(EventQueue::new(cortex_core::QUEUE_CAPACITY, true));
        let done: Done = Arc::default();
        let slot = Slot::default();
        let wake = slot.clone();
        queue.set_wake(Arc::new(move || wake.wake()));
        let (out, outgoing) = mpsc::unbounded_channel();
        let (started, starting) = std::sync::mpsc::channel();
        let thread = (
            place.clone(),
            Arc::clone(&queue),
            Arc::clone(&done),
            slot.clone(),
        );
        std::thread::Builder::new()
            .name("cortex-attach".to_owned())
            .spawn(move || {
                let (place, queue, done, slot) = thread;
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = started.send(Err(format!(
                            "the connection's runtime did not start: {error}"
                        )));
                        return;
                    }
                };
                runtime.block_on(connection(
                    place,
                    outgoing,
                    Shared { queue, done, slot },
                    started,
                ));
            })
            .map_err(|error| format!("the connection's thread could not be started: {error}"))?;
        match starting.recv() {
            Ok(Ok(())) => Ok(Self {
                out,
                queue,
                done,
                slot,
                next: 0,
                order: Order::default(),
                gone_said: false,
            }),
            Ok(Err(said)) => Err(said),
            Err(_) => Err("the connection's thread ended as it started".to_owned()),
        }
    }
}

impl Link for RpcLink {
    fn send(&mut self, op: Op) -> Ticket {
        self.next += 1;
        let ticket = Ticket(self.next);
        if self.out.send(Out::Op(ticket, op)).is_err() {
            interactive::push(&self.done, Inbound::Done(ticket, Err(gone())));
            self.slot.wake();
        }
        ticket
    }

    fn respond(&mut self, request: RequestId, response: ServerResponse) {
        // With the connection gone the request is gone with it.
        let _ = self.out.send(Out::Answer(request, response));
    }

    fn next(&mut self) -> Option<Inbound> {
        interactive::take(
            &self.done,
            &self.queue,
            &mut self.order,
            &mut self.gone_said,
        )
    }
}

/// What the link's thread fills, and how it wakes the interface.
struct Shared {
    queue: Arc<EventQueue>,
    done: Done,
    slot: Slot,
}

fn gone() -> ErrorData {
    ErrorData::new(
        ErrorKind::Internal,
        "server_gone",
        "The server's connection has ended.",
    )
}

/// The client's side of the handshake: a `user` client that answers
/// everything and wants deltas, as the in-process interface is.
fn initialize() -> InitializeParams {
    InitializeParams {
        protocol_version: PROTOCOL_VERSION,
        min_protocol_version: None,
        client: PeerInfo {
            name: "cortex attach".to_owned(),
            version: crate::VERSION.to_owned(),
        },
        role: ClientRole::User,
        capabilities: ClientCapabilities {
            approvals: true,
            questions: true,
            trust: true,
            deltas: true,
            experimental: Vec::new(),
        },
    }
}

/// The connection: reaches the server, says `initialize`, then writes what
/// the interface sends and reads what the server sends until either ends.
async fn connection(
    place: Place,
    outgoing: mpsc::UnboundedReceiver<Out>,
    shared: Shared,
    started: std::sync::mpsc::Sender<Result<(), String>>,
) {
    let Transport { mut input, output } = match cortex_rpc::socket::connect(&place).await {
        Ok(transport) => transport,
        Err(error) => {
            let _ = started.send(Err(format!(
                "the server at {place} is not reached: {error}"
            )));
            return;
        }
    };
    let mut lines = Lines::default();
    let hello = Op::Initialize(initialize());
    let Ok(request) = hello.to_request(RequestId::Number(0)) else {
        let _ = started.send(Err("the handshake could not be written".to_owned()));
        return;
    };
    if write(&output, &Message::Request(request)).await.is_err() {
        let _ = started.send(Err(format!("the server at {place} closed the connection")));
        return;
    }
    // Nothing comes before the handshake's answer.
    loop {
        match read(&mut input, &mut lines).await {
            Some(Message::Response(Response {
                id: Some(RequestId::Number(0)),
                result,
            })) => match result {
                Ok(_) => break,
                Err(error) => {
                    let _ = started.send(Err(format!(
                        "the server at {place} refused the connection: {}",
                        error.message
                    )));
                    return;
                }
            },
            Some(_) => {}
            None => {
                let _ = started.send(Err(format!(
                    "the server at {place} closed the connection: it may have refused it (another \
                     user's, or one of its own commands)"
                )));
                return;
            }
        }
    }
    let _ = started.send(Ok(()));

    let asked: AskedMap = Arc::default();
    // The writer: what the interface sends, in order. Each request is
    // asked before it is written, so that its answer finds it.
    let writer = tokio::spawn(writes(outgoing, output.clone(), Arc::clone(&asked)));
    // The reader, until the server's end.
    while let Some(message) = read(&mut input, &mut lines).await {
        take_in(message, &asked, &shared, &output).await;
    }
    writer.abort();
    // The end: every operation still asked is answered, then the interface
    // is told once, as the queue is closed and empty.
    let left: Vec<Ticket> = asked
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .drain()
        .map(|(_, asked)| asked.ticket)
        .collect();
    for ticket in left {
        interactive::push(&shared.done, Inbound::Done(ticket, Err(gone())));
    }
    shared.queue.close();
    shared.slot.wake();
}

/// Writes what the interface sends until it lets go of the link.
async fn writes(
    mut outgoing: mpsc::UnboundedReceiver<Out>,
    output: mpsc::Sender<Vec<u8>>,
    asked: AskedMap,
) {
    let mut id = 0_i64;
    while let Some(out) = outgoing.recv().await {
        let message = match out {
            Out::Op(ticket, op) => {
                id += 1;
                let session = match &op {
                    Op::SessionRead(params) => Some(params.session_id.clone()),
                    Op::SessionClose(params) => Some(params.session_id.clone()),
                    _ => None,
                };
                let method = op.method();
                let request = match op.to_request(RequestId::Number(id)) {
                    Ok(request) => request,
                    Err(error) => {
                        tracing::warn!(%error, method, "an operation could not be written");
                        continue;
                    }
                };
                asked.lock().unwrap_or_else(PoisonError::into_inner).insert(
                    id,
                    Asked {
                        ticket,
                        method,
                        session,
                    },
                );
                Message::Request(request)
            }
            Out::Answer(request, response) => Message::Response(Response {
                id: Some(request),
                result: Ok(response.result().unwrap_or(serde_json::Value::Null)),
            }),
        };
        if write(&output, &message).await.is_err() {
            return;
        }
    }
}

/// Puts what the server sent where the interface takes it.
async fn take_in(
    message: Message,
    asked: &AskedMap,
    shared: &Shared,
    output: &mpsc::Sender<Vec<u8>>,
) {
    match message {
        Message::Response(Response {
            id: Some(RequestId::Number(id)),
            result,
        }) => {
            let Some(asked) = asked
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id)
            else {
                tracing::debug!(id, "an answer to nothing asked was dropped");
                return;
            };
            let result = match result {
                Ok(value) => OpResult::from_wire(asked.method, value).map_err(|error| {
                    ErrorData::new(ErrorKind::Internal, "unreadable", error.to_string())
                }),
                Err(error) => Err(error.error_data().unwrap_or_else(|| {
                    ErrorData::new(ErrorKind::Internal, "jsonrpc", error.message)
                })),
            };
            // In the order the core keeps for the in-process queue: a read
            // fences the session, an open or a close lifts the fence.
            match (&result, &asked.session) {
                (Ok(OpResult::SessionRead(read)), Some(session)) => {
                    shared.queue.fence(session, read.seq);
                }
                (Ok(_), Some(session)) => shared.queue.unfence(session),
                (
                    Ok(
                        OpResult::SessionNew(opened)
                        | OpResult::SessionOpen(opened)
                        | OpResult::SessionFork(opened),
                    ),
                    None,
                ) => shared.queue.unfence(&opened.session.session_id),
                _ => {}
            }
            interactive::push(&shared.done, Inbound::Done(asked.ticket, result));
            shared.slot.wake();
        }
        Message::Response(_) => {
            tracing::debug!("an answer with no number for an id was dropped");
        }
        Message::Notification(notification) => {
            if let Ok(ServerNotification::Event(event)) =
                ServerNotification::from_wire(notification)
            {
                shared.queue.push(*event);
            }
        }
        Message::Request(Request { id, method, params }) => {
            match ServerRequest::from_wire(&method, params) {
                Ok(request) => shared.queue.push_request(id, request),
                Err(error) => {
                    // A request this build cannot read is declined, so that
                    // it stays open for the server's other clients.
                    let declined = Message::Response(Response {
                        id: Some(id),
                        result: Err(ErrorObject {
                            code: cortex_protocol::jsonrpc::code::METHOD_NOT_FOUND,
                            message: format!("this client cannot read `{method}`: {error}"),
                            data: None,
                        }),
                    });
                    let _ = write(output, &declined).await;
                }
            }
        }
    }
}

/// Writes one message as a line.
async fn write(output: &mpsc::Sender<Vec<u8>>, message: &Message) -> Result<(), ()> {
    let mut line = serde_json::to_vec(message).map_err(|_| ())?;
    line.push(b'\n');
    output.send(line).await.map_err(|_| ())
}

/// The next message the server sent, or `None` at the connection's end (or
/// a line too long to be one). A line that is not a message is passed
/// over. Safe to drop at its await: what was read is kept in `lines`.
async fn read(
    input: &mut mpsc::Receiver<std::io::Result<Vec<u8>>>,
    lines: &mut Lines,
) -> Option<Message> {
    loop {
        match lines.next_line() {
            Err(()) => return None,
            Ok(Some(line)) => match serde_json::from_slice::<Message>(&line) {
                Ok(message) => return Some(message),
                Err(error) => {
                    tracing::warn!(%error, "a line from the server is not a message");
                }
            },
            Ok(None) => match input.recv().await {
                Some(Ok(chunk)) => lines.push(&chunk),
                Some(Err(_)) | None => return None,
            },
        }
    }
}

/// The lines of what was read, split at `\n`.
#[derive(Default)]
struct Lines {
    held: Vec<u8>,
    /// How much of `held` is known to hold no `\n`.
    searched: usize,
}

impl Lines {
    fn push(&mut self, chunk: &[u8]) {
        self.held.extend_from_slice(chunk);
    }

    /// The next whole line, without its `\n`; `Err` when a line has grown
    /// past [`LONGEST_LINE`].
    fn next_line(&mut self) -> Result<Option<Vec<u8>>, ()> {
        let Some(found) = self.held[self.searched..]
            .iter()
            .position(|byte| *byte == b'\n')
        else {
            self.searched = self.held.len();
            return if self.held.len() > LONGEST_LINE {
                Err(())
            } else {
                Ok(None)
            };
        };
        let end = self.searched + found;
        let line = self.held[..end].to_vec();
        self.held.drain(..=end);
        self.searched = 0;
        Ok(Some(line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines are split at `\n` however the chunks fall, and one past the
    /// longest is the connection's end.
    #[test]
    fn lines_are_split_however_the_chunks_fall() {
        let mut lines = Lines::default();
        lines.push(b"{\"a\"");
        assert_eq!(lines.next_line(), Ok(None));
        lines.push(b":1}\n{\"b\":2}\n{\"c\"");
        assert_eq!(lines.next_line(), Ok(Some(b"{\"a\":1}".to_vec())));
        assert_eq!(lines.next_line(), Ok(Some(b"{\"b\":2}".to_vec())));
        assert_eq!(lines.next_line(), Ok(None));
        lines.push(b":3}\n");
        assert_eq!(lines.next_line(), Ok(Some(b"{\"c\":3}".to_vec())));
        assert_eq!(lines.next_line(), Ok(None));
        let mut long = Lines::default();
        long.push(&vec![b'x'; LONGEST_LINE + 1]);
        assert_eq!(long.next_line(), Err(()));
    }
}
