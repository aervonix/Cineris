//! The local socket: where `cortex serve --socket` listens, who may connect,
//! and each connection's bytes as a [`Transport`]
//! (`docs/architecture/11-rpc-protocol.md`, "Transports";
//! `docs/steps/27/answered.md`, "The design, before building", point 1).
//!
//! **Unix:** a socket file owner-only (`0600`) in a folder no one else can
//! reach, a lock beside it for the server's life, and a check of every
//! peer's credentials before a byte is read. **Windows:** a named pipe
//! whose access list admits the user alone at the server's integrity, and a
//! check of each connection's own token, made on its first data, since
//! Windows gives a pipe's server the client's token only once it has read
//! something the client wrote. A connection that fails its check is closed
//! with nothing served or written to it.

use std::io;
use std::path::{Path, PathBuf};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::transport::Transport;

#[cfg(unix)]
#[path = "socket/unix.rs"]
mod sys;

#[cfg(windows)]
#[path = "socket/windows.rs"]
mod sys;

/// A peer of the user was taken and its line of parents is not read yet
/// (Unix): an awaited point (`cortex_base::hook::await_point!`), on the
/// server's task, for a test to make a peer's process go first.
pub const BEFORE_A_PEERS_PARENTS_ARE_READ: &str =
    "socket: a peer of the user was taken, and its line of parents is not read yet";

/// How much is read from a connection at a time.
const CHUNK: usize = 64 * 1024;

/// How many chunks may wait for the connection before reading stops.
const CHUNKS: usize = 4;

/// Where a server listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// A socket file (Unix).
    Path(PathBuf),
    /// A named pipe, by its whole name (`\\.\pipe\…`, Windows).
    Pipe(String),
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(path) => write!(f, "{}", path.display()),
            Self::Pipe(name) => f.write_str(name),
        }
    }
}

impl Place {
    /// The place a user names: a path on Unix, a pipe's short name or
    /// whole name on Windows.
    #[must_use]
    pub fn named(name: &str) -> Self {
        #[cfg(windows)]
        {
            if name.starts_with(r"\\.\pipe\") {
                Self::Pipe(name.to_owned())
            } else {
                Self::Pipe(format!(r"\\.\pipe\{name}"))
            }
        }
        #[cfg(not(windows))]
        {
            Self::Path(PathBuf::from(name))
        }
    }

    /// Where a server listens when it is given no place: on Unix the
    /// runtime directory's `cortex-harness/rpc.sock` (`runtime` is
    /// `XDG_RUNTIME_DIR`), or `/tmp/cortex-harness-<uid>/rpc.sock` with
    /// none; on Windows `\\.\pipe\cortex-harness-<the user's SID>`.
    ///
    /// # Errors
    ///
    /// When the user cannot be told (Windows's SID, Unix's uid).
    pub fn default_for(runtime: Option<&Path>) -> io::Result<Self> {
        sys::default_place(runtime)
    }
}

/// How a listener is made.
#[derive(Debug, Clone, Default)]
pub struct ListenOptions {
    /// The place is the server's own default: on Unix its folder is made
    /// owner-only when it is missing and held to that, where a folder the
    /// user named is only checked not to be writable by others.
    pub own_folder: bool,
    /// Test only: the pipe's access list also admits everyone, and its
    /// label is lowered to low integrity, so that a test can show the check
    /// of a connection's token refusing another user, or a lower integrity,
    /// by itself. Off in every server.
    #[cfg(feature = "test-hooks")]
    pub widen_access_for_tests: bool,
    /// Test only: as many of the listener's next `accept`s as this counts
    /// fail at once as if the process were out of descriptors, so that a
    /// test shows the server trying again. None in every server.
    #[cfg(feature = "test-hooks")]
    pub failing_accepts: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}

impl ListenOptions {
    /// A listener's options: at the server's own default place when
    /// `own_folder`, at one the user named otherwise.
    #[must_use]
    pub fn new(own_folder: bool) -> Self {
        Self {
            own_folder,
            #[cfg(feature = "test-hooks")]
            widen_access_for_tests: false,
            #[cfg(feature = "test-hooks")]
            failing_accepts: None,
        }
    }
}

/// What a listener's next connection came to.
#[derive(Debug)]
pub enum Accepted {
    /// A client the checks admitted, and its bytes.
    Client {
        /// The connection's bytes.
        transport: Transport,
        /// Who it is, for the log.
        peer: String,
    },
    /// A connection the checks refused, with why: closed with nothing
    /// served or written (on Windows its first data was read for the check,
    /// and is dropped with it). The listener goes on.
    Refused(String),
}

/// A server's listening place, secured, until it is dropped: then the
/// socket file is removed and its lock let go (Unix).
#[derive(Debug)]
pub struct Listener {
    inner: sys::Listener,
    #[cfg(feature = "test-hooks")]
    failing: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}

impl Listener {
    /// Listens at `place`, secured before anything can connect.
    ///
    /// # Errors
    ///
    /// When the place is not safe to listen at (a folder others can reach,
    /// a name another process holds), another server listens there, or the
    /// system refuses. The message says which, and what to do.
    pub fn bind(place: &Place, options: &ListenOptions) -> io::Result<Self> {
        Ok(Self {
            inner: sys::Listener::bind(place, options)?,
            #[cfg(feature = "test-hooks")]
            failing: options.failing_accepts.clone(),
        })
    }

    /// Where it listens.
    #[must_use]
    pub fn place(&self) -> &Place {
        self.inner.place()
    }

    /// The next connection, checked.
    ///
    /// # Errors
    ///
    /// When listening fails: for want of a resource for the moment (the
    /// process's descriptors, memory), or for good.
    pub async fn accept(&mut self) -> io::Result<Accepted> {
        #[cfg(feature = "test-hooks")]
        if let Some(failing) = &self.failing
            && failing
                .try_update(
                    std::sync::atomic::Ordering::SeqCst,
                    std::sync::atomic::Ordering::SeqCst,
                    |left| left.checked_sub(1),
                )
                .is_ok()
        {
            // EMFILE, and ERROR_TOO_MANY_OPEN_FILES.
            return Err(io::Error::from_raw_os_error(if cfg!(windows) {
                4
            } else {
                24
            }));
        }
        self.inner.accept().await
    }
}

/// Connects to a server listening at `place`, as a client (`cortex
/// attach`), once the server is shown to be the user's own: a client sends
/// its prompts, and answers its approvals, to whatever answers there.
///
/// - **Unix:** the socket must be the user's, with no write for group or
///   others, in a folder that is the user's with no write for others, or
///   root's with the sticky bit; and the credentials of the process at the
///   other end of the connection (`SO_PEERCRED`) must name the user.
/// - **Windows:** the pipe's owner must be the user, and its integrity
///   label at least medium (asked of the open handle); a pipe whose
///   security cannot be read is refused.
///
/// Runs in the runtime the transport's tasks are to run in.
///
/// # Errors
///
/// When nothing listens there, the server is not shown to be the user's,
/// or the system refuses. The message says which.
pub async fn connect(place: &Place) -> io::Result<Transport> {
    sys::connect(place).await
}

/// A connection's two halves as a [`Transport`], each served by a task of
/// the runtime it is made in: what is read goes to the connection in
/// chunks, and its lines are written in turn. Reading stops when the
/// connection lets go of its input, writing when it lets go of its output.
pub(crate) fn transport_of<R, W>(mut read: R, mut write: W) -> Transport
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (chunks, input) = mpsc::channel(CHUNKS);
    let (output, mut lines) = mpsc::channel::<Vec<u8>>(1);
    tokio::spawn(async move {
        let mut buffer = vec![0; CHUNK];
        loop {
            let read = {
                let mut reading = std::pin::pin!(read.read(&mut buffer));
                let mut gone = std::pin::pin!(chunks.closed());
                std::future::poll_fn(|context| {
                    if let std::task::Poll::Ready(read) = reading.as_mut().poll(context) {
                        return std::task::Poll::Ready(Some(read));
                    }
                    if gone.as_mut().poll(context).is_ready() {
                        return std::task::Poll::Ready(None);
                    }
                    std::task::Poll::Pending
                })
                .await
            };
            match read {
                // The connection let go of its input: nobody reads on.
                None | Some(Ok(0)) => return,
                Some(Ok(count)) => {
                    if chunks.send(Ok(buffer[..count].to_vec())).await.is_err() {
                        return;
                    }
                }
                Some(Err(error)) => {
                    let _ = chunks.send(Err(error)).await;
                    return;
                }
            }
        }
    });
    tokio::spawn(async move {
        while let Some(line) = lines.recv().await {
            if write.write_all(&line).await.is_err() {
                return;
            }
        }
        let _ = write.shutdown().await;
    });
    Transport { input, output }
}
