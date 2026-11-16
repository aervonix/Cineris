//! The Unix half of [`super`]: a socket file in a folder no one else can
//! reach, and a check of each peer's credentials.
//!
//! - **The folder is secured before the socket exists.** The default one is
//!   created owner-only (`0700`), and an existing one must be a directory,
//!   not a link, owned by the user, with nothing for group or others. A
//!   folder the user names must be the user's with no write for group or
//!   others, or root's with the sticky bit (`/tmp`).
//! - **A lock beside the socket for the server's life** (`rpc.sock.lock`,
//!   owner-only, opened without following a link, the user's): another
//!   server holding it refuses the start. With it taken, a socket at the
//!   path is removed only when a `connect` to it is refused, so a lock file
//!   a cleaner of `/tmp` removed under a live server cannot lead a second
//!   server to remove the live socket. Anything there that is not a socket
//!   is never removed.
//! - **The socket is `0600`,** set right after `bind`; the window between
//!   is covered by the folder and by the peer check.
//! - **Every peer's credentials** (`SO_PEERCRED`, held at `connect`) must
//!   name the server's own effective uid.
//! - **The server's own commands are refused** (the design note's point 1,
//!   "The harness's own commands"): a peer whose line of parents reaches the
//!   server's process is one of the commands its sessions started, which
//!   could otherwise answer its own session's approvals. A peer whose line
//!   cannot be read is refused too: its process, or a parent of it, is gone
//!   (a command can connect, hand the socket to a child and exit at once,
//!   the independent review of step 27's second part), or it is in another
//!   pid namespace. A command that detaches itself from that line escapes
//!   this, and so could one whose process id was taken by another process in
//!   between, so it is defence in depth beside the sandbox, which keeps a
//!   confined command from any Unix socket.

#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a listener is made once, before the server serves anything, with a few calls on \
              one local folder; and its socket file is removed when it is dropped, at the end"
)]

use std::fs::File;
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use super::{Accepted, ListenOptions, Place};

/// The user the server runs as: the owner of `/proc/self`, which is the
/// process's effective uid, read with no `unsafe`.
pub(super) fn effective_uid() -> io::Result<u32> {
    Ok(std::fs::metadata("/proc/self")?.uid())
}

pub(super) fn default_place(runtime: Option<&Path>) -> io::Result<Place> {
    let folder = match runtime.filter(|runtime| runtime.is_absolute()) {
        Some(runtime) => runtime.join("cortex-harness"),
        None => PathBuf::from(format!("/tmp/cortex-harness-{}", effective_uid()?)),
    };
    Ok(Place::Path(folder.join("rpc.sock")))
}

/// Whether the folder the server made or keeps is the user's alone.
fn check_own_folder(folder: &Path, uid: u32) -> io::Result<()> {
    let found = std::fs::symlink_metadata(folder)?;
    if !found.file_type().is_dir() {
        return Err(refused(format!(
            "{} is not a directory (a link is not followed): the server does not listen there",
            folder.display()
        )));
    }
    if found.uid() != uid {
        return Err(refused(format!(
            "{} belongs to another user: the server does not listen there",
            folder.display()
        )));
    }
    if found.mode() & 0o077 != 0 {
        return Err(refused(format!(
            "{} can be reached by others (mode {:o}): make it 0700, or remove it and the server \
             makes it so",
            folder.display(),
            found.mode() & 0o7777
        )));
    }
    Ok(())
}

/// Whether a folder the user named is safe to put a socket in: the user's
/// with no write for others, or root's with the sticky bit.
fn check_named_folder(folder: &Path, uid: u32) -> io::Result<()> {
    let found = std::fs::metadata(folder)?;
    let mode = found.mode();
    let theirs = found.uid() == uid && mode & 0o022 == 0;
    let sticky_root = found.uid() == 0 && mode & 0o1000 != 0;
    if found.is_dir() && (theirs || sticky_root) {
        Ok(())
    } else {
        Err(refused(format!(
            "{} is a folder others can write in (mode {:o}): put the socket in a folder of your \
             own",
            folder.display(),
            mode & 0o7777
        )))
    }
}

/// The parent of `pid`, from `/proc/<pid>/stat`: the field after the
/// command's name, which is in parentheses and may hold anything, so it is
/// read from the last `)`.
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after = &stat[stat.rfind(')')? + 1..];
    after.split_whitespace().nth(1)?.parse().ok()
}

/// Where a peer's line of parents leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// To the server's process: the peer is one of its commands.
    Server,
    /// To the system's first process without passing the server's.
    Elsewhere,
    /// Not known: the peer, or one of its parents, could not be read.
    Unknown,
}

/// Where the line of parents of `pid` leads, `server` being the server's
/// process (not `pid` itself: the server may connect to itself), with
/// `parent_of` reading a process's parent.
///
/// Only the peer's own process must be readable: one that is gone is
/// `Unknown`. A parent that cannot be read ends the line `Elsewhere`: it is
/// another user's process that `/proc` hides (`hidepid`, a unit's
/// `ProtectProc`: an SSH session's `sshd`, a `sudo`), never one of the
/// server's commands, which are the server's user's and visible to it; or
/// it went in the moment between two reads, as a client's launcher may
/// (the second review of step 27's second part).
fn line_of(pid: u32, server: u32, parent_of: impl Fn(u32) -> Option<u32>) -> Line {
    let mut at = pid;
    // A line of parents is short; the bound only stops a loop the table
    // could show while it changes.
    for _ in 0..4096 {
        match parent_of(at) {
            Some(parent) if parent == server => return Line::Server,
            Some(parent) if parent <= 1 || parent == at => return Line::Elsewhere,
            Some(parent) => at = parent,
            None if at == pid => return Line::Unknown,
            None => return Line::Elsewhere,
        }
    }
    Line::Unknown
}

fn refused(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

/// Takes the lock beside the socket without waiting. `None` when another
/// process holds it.
fn take_lock(socket: &Path, uid: u32) -> io::Result<Option<File>> {
    let path = cortex_base::private::lock_path(socket);
    let opened = match cortex_base::open_regular(&path) {
        Ok(opened) => opened,
        Err(cortex_base::OpenError::NotFound) => {
            match cortex_base::private::create_new(&path) {
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            cortex_base::open_regular(&path)
                .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?
        }
        Err(error) => {
            return Err(io::Error::other(format!("{}: {error}", path.display())));
        }
    };
    let (lock, found) = opened;
    if found.uid() != uid {
        return Err(refused(format!(
            "{} belongs to another user: the server does not listen beside it",
            path.display()
        )));
    }
    match lock.try_lock() {
        Ok(()) => Ok(Some(lock)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error),
    }
}

/// Clears the socket's path for a new one: nothing there, or a socket no
/// server answers on.
fn clear(socket: &Path) -> io::Result<()> {
    let found = match std::fs::symlink_metadata(socket) {
        Ok(found) => found,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !found.file_type().is_socket() {
        return Err(refused(format!(
            "{} is there and is not a socket: the server leaves it alone and does not start",
            socket.display()
        )));
    }
    match std::os::unix::net::UnixStream::connect(socket) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            format!("a server already listens at {}", socket.display()),
        )),
        Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
            std::fs::remove_file(socket)
        }
        Err(error) => Err(error),
    }
}

#[derive(Debug)]
pub(super) struct Listener {
    place: Place,
    path: PathBuf,
    /// The bound socket, until the first `accept` takes it into the
    /// runtime it runs in: binding needs no runtime.
    bound: Option<std::os::unix::net::UnixListener>,
    listener: Option<tokio::net::UnixListener>,
    uid: u32,
    /// Held for the listener's life; let go after the socket is removed.
    lock: Option<File>,
}

impl Listener {
    pub(super) fn bind(place: &Place, options: &ListenOptions) -> io::Result<Self> {
        let Place::Path(path) = place else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a Unix server listens at a path",
            ));
        };
        let uid = effective_uid()?;
        let folder = path
            .parent()
            .filter(|folder| !folder.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        if options.own_folder {
            cortex_base::private::create_dir_all(folder)?;
            check_own_folder(folder, uid)?;
        } else {
            check_named_folder(folder, uid)?;
        }
        let Some(lock) = take_lock(path, uid)? else {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("a server already listens at {}", path.display()),
            ));
        };
        clear(path)?;
        let listener = std::os::unix::net::UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            place: place.clone(),
            path: path.clone(),
            bound: Some(listener),
            listener: None,
            uid,
            lock: Some(lock),
        })
    }

    pub(super) fn place(&self) -> &Place {
        &self.place
    }

    pub(super) async fn accept(&mut self) -> io::Result<Accepted> {
        if let Some(bound) = self.bound.take() {
            self.listener = Some(tokio::net::UnixListener::from_std(bound)?);
        }
        let Some(listener) = &self.listener else {
            return Err(io::Error::other("the listener is gone"));
        };
        let (stream, _) = listener.accept().await?;
        let peer = match stream.peer_cred() {
            Ok(peer) => peer,
            Err(error) => return Ok(Accepted::Refused(format!("no credentials: {error}"))),
        };
        if peer.uid() != self.uid {
            return Ok(Accepted::Refused(format!(
                "a peer of another user (uid {}, pid {})",
                peer.uid(),
                peer.pid()
                    .map_or_else(|| "unknown".to_owned(), |pid| pid.to_string())
            )));
        }
        cortex_base::hook::await_point!(super::BEFORE_A_PEERS_PARENTS_ARE_READ);
        let Some(pid) = peer
            .pid()
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid != 0)
        else {
            return Ok(Accepted::Refused(
                "a peer whose process is not known (in another pid namespace, or none given)"
                    .to_owned(),
            ));
        };
        match line_of(pid, std::process::id(), parent_of) {
            Line::Server => {
                return Ok(Accepted::Refused(format!(
                    "one of the server's own commands (pid {pid})"
                )));
            }
            Line::Unknown => {
                return Ok(Accepted::Refused(format!(
                    "a peer whose line of parents could not be read (pid {pid}): its process, \
                     or a parent of it, is gone"
                )));
            }
            Line::Elsewhere => {}
        }
        let described = format!(
            "pid {}",
            peer.pid()
                .map_or_else(|| "unknown".to_owned(), |pid| pid.to_string())
        );
        let (read, write) = stream.into_split();
        Ok(Accepted::Client {
            transport: super::transport_of(read, write),
            peer: described,
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // The socket goes before the lock, so that no second server finds
        // the lock free and this socket still there.
        let _ = std::fs::remove_file(&self.path);
        drop(self.lock.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A table of parents, as `/proc` would give it: `None` for a process
    /// that cannot be read.
    fn table(parents: &[(u32, Option<u32>)]) -> impl Fn(u32) -> Option<u32> + '_ {
        move |pid| {
            parents
                .iter()
                .find(|(child, _)| *child == pid)
                .and_then(|(_, parent)| *parent)
        }
    }

    /// The server's own command, near or far, is the server's; a line that
    /// reaches the first process is not; a peer that cannot be read is not
    /// known; a parent that cannot be read (another user's, hidden) ends
    /// the line elsewhere.
    #[test]
    fn a_line_of_parents_is_the_servers_elsewhere_or_unknown() {
        const SERVER: u32 = 100;
        let parents = [
            (300, Some(200)),
            (200, Some(SERVER)),
            (SERVER, Some(50)),
            (50, Some(1)),
            (400, Some(350)),
            (350, Some(1)),
            (500, Some(450)),
            (450, None),
        ];
        let read = table(&parents);
        assert_eq!(line_of(300, SERVER, &read), Line::Server);
        assert_eq!(line_of(200, SERVER, &read), Line::Server);
        assert_eq!(line_of(SERVER, SERVER, &read), Line::Elsewhere);
        assert_eq!(line_of(400, SERVER, &read), Line::Elsewhere);
        assert_eq!(line_of(500, SERVER, &read), Line::Elsewhere);
        assert_eq!(line_of(999, SERVER, &read), Line::Unknown);
    }
}
