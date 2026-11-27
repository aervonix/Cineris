//! What a Linux harness tells everything it starts, in a form the kernel
//! vouches for: its stores, by a socket only it could have made.
//!
//! Since the harness closes its own process ([`crate::harden`]), its
//! `/proc/<pid>/environ` and `exe` are root's to read, and a nested run can
//! no longer compare its stores with the session's environment there, nor
//! tell its harness by the very file it runs. So [`give`], called by
//! [`super::mark`] before the harness starts any process, makes a pair of
//! sequenced-packet Unix sockets, writes one record into the end it keeps
//! (the tag, when the harness was started, and each variable of
//! [`super::sys`]'s list that it has, `NAME=value`), closes the other end,
//! and lets the kept end pass to every process it starts. What it says can
//! then be neither changed nor added to: the end that wrote it is closed,
//! and a packet sent into the kept end goes to the closed one.
//!
//! [`held`] reads, of every descriptor this process holds, those that are
//! such a socket: its peer's credentials (`SO_PEERCRED`) are the process
//! that made the pair, which the kernel set then and no process can choose,
//! and its record is read without being taken (`MSG_PEEK`). A command can
//! close the descriptor, or take the record out of it, and then holds
//! nothing: a nested run refuses, and the line of parents is read as
//! before. It cannot hold one that names its harness and says something
//! else. It can make a word of its own, naming itself: that counts only
//! beside its harness's, since every process of the line that runs
//! sessions must have given a word that agrees, and so must every word held
//! whose maker still runs (the review of the harness's word, W1). A word
//! whose maker still runs counts for the approving subcommands wherever it
//! is held, on the line of parents or not (W3).
//!
//! Cineris makes sequenced-packet sockets nowhere else, and connects to
//! none, so no socket whose peer is a harness carries a record it did not
//! write for this.

#![allow(
    unsafe_code,
    reason = "decision 0030: a socket pair, its peer's credentials, a look at a packet without \
              taking it, a descriptor's flags and a signal of nothing are only reachable \
              through the C library; cineris-sandbox is where such calls live"
)]
#![allow(
    clippy::disallowed_methods,
    reason = "a few small reads of `/proc/self`, once, before a process starts or a nested run \
              decides"
)]

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::OnceLock;

/// What a record begins with.
const TAG: &[u8] = b"cineris-session-stores 1";

/// The longest record read or written.
const MOST: usize = 64 * 1024;

/// The end a harness keeps, for its life: what it starts inherits it.
static GIVEN: OnceLock<OwnedFd> = OnceLock::new();

/// One harness's word, as this process holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Channel {
    /// The process that made it, as the kernel says.
    pub(super) pid: u32,
    /// When that process started, as it said: `/proc/<pid>/stat`'s 22nd
    /// field, in clock ticks since the system started.
    pub(super) start: u64,
    /// Each variable it had, with its value's bytes.
    entries: Vec<(String, Vec<u8>)>,
    /// The descriptor it was read from.
    pub(super) fd: RawFd,
}

impl Channel {
    /// The value it gave `name`, `None` where it had none.
    #[cfg(test)]
    pub(super) fn value(&self, name: &str) -> Option<&[u8]> {
        self.entries
            .iter()
            .find(|(had, _)| had == name)
            .map(|(_, value)| value.as_slice())
    }

    /// What it says, as the entries of an environment, in its order, for
    /// [`super::moved_from`]: an entry whose value is not Unicode is left
    /// out, as Cineris's own reading of its environment leaves it out
    /// ([`super::entries_of_this_process`]).
    pub(super) fn entries(&self) -> Vec<(String, String)> {
        self.entries
            .iter()
            .filter_map(|(name, value)| {
                Some((name.clone(), String::from_utf8(value.clone()).ok()?))
            })
            .collect()
    }

    /// A word as a test would have been given it.
    #[cfg(test)]
    pub(super) fn given(pid: u32, start: u64, entries: &[(&str, &str)]) -> Self {
        Self {
            pid,
            start,
            entries: entries
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.as_bytes().to_vec()))
                .collect(),
            fd: -1,
        }
    }
}

/// When a process started, from its `stat`: the 22nd field, the 20th after
/// the name, which ends at the last `)`.
pub(super) fn start_in(stat: &str) -> Option<u64> {
    let close = stat.rfind(')')?;
    stat.get(close + 1..)?
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

/// The record a harness gives: the tag, its start, and `NAME=value` for
/// each of `names` it has, each ended by a nul, each read as Cineris reads
/// its own environment ([`super::value_in`]: the last entry of a name, an
/// empty value as none, an entry that is not Unicode left out), so that
/// the word says the stores the harness itself uses.
fn record(start: u64, names: &[&str]) -> Vec<u8> {
    let mut record = TAG.to_vec();
    record.push(0);
    record.extend_from_slice(start.to_string().as_bytes());
    record.push(0);
    let own = super::entries_of_this_process();
    for name in names {
        if let Some(value) = super::value_in(&own, name, false) {
            record.extend_from_slice(name.as_bytes());
            record.push(b'=');
            record.extend_from_slice(value.as_bytes());
            record.push(0);
        }
    }
    record
}

/// A record read back: `None` for anything else.
fn parse(bytes: &[u8], pid: u32, fd: RawFd) -> Option<Channel> {
    let mut parts = bytes.split(|byte| *byte == 0);
    if parts.next()? != TAG {
        return None;
    }
    let start = std::str::from_utf8(parts.next()?).ok()?.parse().ok()?;
    let mut entries = Vec::new();
    for part in parts {
        if part.is_empty() {
            continue;
        }
        let at = part.iter().position(|byte| *byte == b'=')?;
        let name = std::str::from_utf8(&part[..at]).ok()?.to_owned();
        entries.push((name, part[at + 1..].to_vec()));
    }
    Some(Channel {
        pid,
        start,
        entries,
        fd,
    })
}

/// Makes this process's word and keeps it where what it starts inherits
/// it: once a process, before it starts anything.
///
/// # Errors
///
/// When this process's start cannot be read, the pair cannot be made, or
/// the record cannot be written whole: what it starts then holds nothing,
/// and a nested run under it refuses.
pub(super) fn give(names: &[&str]) -> io::Result<()> {
    if GIVEN.get().is_some() {
        return Ok(());
    }
    let start = std::fs::read_to_string("/proc/self/stat")
        .ok()
        .and_then(|stat| start_in(&stat))
        .ok_or_else(|| io::Error::other("this process's start could not be read"))?;
    let kept = made(&record(start, names))?;
    // SAFETY: `kept` is open; the call sets its flags to none, so that it
    // is inherited, and touches no memory.
    if unsafe { libc::fcntl(kept.as_raw_fd(), libc::F_SETFD, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let _ = GIVEN.set(kept);
    Ok(())
}

/// A pair of sequenced-packet Unix sockets with `record` written into the
/// end returned, and the other end closed. The end returned is still
/// closed on `execve`: [`give`] clears that.
fn made(record: &[u8]) -> io::Result<OwnedFd> {
    if record.len() > MOST {
        return Err(io::Error::other("the stores' record is too long"));
    }
    let mut pair = [0 as RawFd; 2];
    // SAFETY: `pair` has room for the two descriptors the call writes.
    if unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
            0,
            pair.as_mut_ptr(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both were just made by the call and nothing else owns them.
    let (kept, writer) = unsafe { (OwnedFd::from_raw_fd(pair[0]), OwnedFd::from_raw_fd(pair[1])) };
    // SAFETY: `writer` is open, and the record lives through the call,
    // which reads `record.len()` bytes of it.
    let sent = unsafe {
        libc::send(
            writer.as_raw_fd(),
            record.as_ptr().cast(),
            record.len(),
            libc::MSG_NOSIGNAL,
        )
    };
    if usize::try_from(sent).ok() != Some(record.len()) {
        return Err(io::Error::last_os_error());
    }
    // Nothing can be written to the kept end once this one is closed.
    drop(writer);
    Ok(kept)
}

/// One integer option of a socket: `None` where `fd` is not a socket or
/// the option cannot be read.
fn option(fd: RawFd, name: libc::c_int) -> Option<libc::c_int> {
    let mut value: libc::c_int = 0;
    let mut length = libc::socklen_t::try_from(size_of::<libc::c_int>()).ok()?;
    // SAFETY: `value` and `length` are live, and `length` says how large
    // `value` is; a descriptor that is not a socket fails the call.
    let asked = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            name,
            (&raw mut value).cast(),
            &raw mut length,
        )
    };
    (asked == 0).then_some(value)
}

/// The harness's word on `fd`, where it is one: a Unix socket of
/// sequenced packets, whose peer is a process of this user, holding a
/// record.
fn read_one(fd: RawFd, uid: u32) -> Option<Channel> {
    if option(fd, libc::SO_TYPE)? != libc::SOCK_SEQPACKET
        || option(fd, libc::SO_DOMAIN)? != libc::AF_UNIX
    {
        return None;
    }
    // SAFETY: plain data, for which all zeroes is valid.
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = libc::socklen_t::try_from(size_of::<libc::ucred>()).ok()?;
    // SAFETY: `peer` and `length` are live, and `length` says how large
    // `peer` is.
    let asked = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut peer).cast(),
            &raw mut length,
        )
    };
    if asked != 0 || peer.uid != uid {
        return None;
    }
    let pid = u32::try_from(peer.pid).ok().filter(|pid| *pid > 0)?;
    let mut buffer = vec![0_u8; MOST];
    // SAFETY: the buffer is as large as said and lives through the call;
    // the packet is looked at, not taken, without waiting, and its whole
    // length is returned.
    let read = unsafe {
        libc::recv(
            fd,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            libc::MSG_PEEK | libc::MSG_DONTWAIT | libc::MSG_TRUNC,
        )
    };
    let read = usize::try_from(read).ok().filter(|read| *read <= MOST)?;
    parse(&buffer[..read], pid, fd)
}

/// The descriptors this process holds, as `/proc/self/fd` lists them (the
/// listing's own among them, which is no socket).
fn descriptors() -> Vec<RawFd> {
    std::fs::read_dir("/proc/self/fd").map_or_else(
        |_| Vec::new(),
        |listing| {
            listing
                .flatten()
                .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
                .collect()
        },
    )
}

/// Every harness's word this process holds.
pub(super) fn held() -> Vec<Channel> {
    // SAFETY: takes nothing and cannot fail.
    let uid = unsafe { libc::geteuid() };
    descriptors()
        .into_iter()
        .filter_map(|fd| read_one(fd, uid))
        .collect()
}

/// The descriptors of the harnesses' words this process holds: what the
/// sandbox launcher keeps open for the command, when it marks every other
/// to be closed.
pub(crate) fn to_keep() -> Vec<RawFd> {
    held().into_iter().map(|channel| channel.fd).collect()
}

/// Lets `fd` pass to the program this process becomes: what the launcher
/// does for each of [`to_keep`] after marking every descriptor.
pub(crate) fn inherit(fd: RawFd) {
    // SAFETY: the call sets a descriptor's flags to none; it touches no
    // memory, and fails harmlessly for a descriptor that is not open.
    let _ = unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
}

/// Whose a process that `/proc` does not show is, as a signal of nothing
/// sent to it tells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Whose {
    /// There, and one this process may signal: of this user.
    Ours,
    /// There, and another user's (`EPERM`).
    Others,
    /// Gone (`ESRCH`).
    Gone,
    /// The call failed otherwise, as where a security module (SELinux,
    /// AppArmor) refuses the signal with `EACCES`: what it is is not known
    /// (the review of the harness's word, W4).
    NotKnown,
}

/// Whose the process `pid`, which `/proc` does not show, is: a signal of
/// nothing is asked of it (`kill(pid, 0)`).
pub(super) fn ours(pid: u32) -> Whose {
    // An id of 0 or below would ask of a group of processes, not of one.
    let Some(pid) = libc::pid_t::try_from(pid).ok().filter(|pid| *pid > 0) else {
        return Whose::NotKnown;
    };
    // SAFETY: signal 0 sends nothing: the call only asks whether a signal
    // could be sent.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return whose(Ok(()));
    }
    whose(Err(io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or_default()))
}

/// What a signal of nothing's outcome says: sent, or refused with an error
/// number. Only `ESRCH` is gone, and only `EPERM` another user's.
fn whose(asked: Result<(), i32>) -> Whose {
    match asked {
        Ok(()) => Whose::Ours,
        Err(libc::EPERM) => Whose::Others,
        Err(libc::ESRCH) => Whose::Gone,
        Err(_) => Whose::NotKnown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record is read back as it was written; anything without the tag,
    /// or with an entry that has no `=`, is not one.
    #[test]
    fn a_record_is_read_back_as_written() {
        let mut bytes = TAG.to_vec();
        bytes.extend_from_slice(b"\x00123\x00HOME=/h/a=b\x00XDG_CONFIG_HOME=\x00");
        let channel = parse(&bytes, 7, 3).unwrap();
        assert_eq!((channel.pid, channel.start), (7, 123));
        assert_eq!(channel.value("HOME"), Some(&b"/h/a=b"[..]));
        assert_eq!(channel.value("XDG_CONFIG_HOME"), Some(&b""[..]));
        assert_eq!(channel.value("CINERIS_CONFIG"), None);
        assert!(parse(b"something else\x00123\x00", 7, 3).is_none());
        let mut no_value = TAG.to_vec();
        no_value.extend_from_slice(b"\x00123\x00HOME\x00");
        assert!(parse(&no_value, 7, 3).is_none());
        let mut no_start = TAG.to_vec();
        no_start.extend_from_slice(b"\x00x\x00");
        assert!(parse(&no_start, 7, 3).is_none());
    }

    /// The start is the 22nd field, read past a name that holds spaces and
    /// parentheses.
    #[test]
    fn a_start_is_read_past_any_name() {
        let stat = "42 (a) b (c) S 1 42 42 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 98765 1 2 3\n";
        assert_eq!(start_in(stat), Some(98765));
        assert_eq!(start_in("42 (a) S 1"), None);
        let own = std::fs::read_to_string("/proc/self/stat").unwrap();
        assert!(start_in(&own).is_some(), "{own}");
    }

    /// A process of this user is told from one that is gone, and from
    /// another user's: the system's first process, root's on every system
    /// the tests run on, unless they run as root or in a container whose
    /// first process is their user's. Only `ESRCH` is gone and only `EPERM`
    /// another user's: any other refusal, as a security module's `EACCES`,
    /// is not known (the review of the harness's word, W4 and W5).
    #[test]
    fn whose_a_process_is_is_told_by_a_signal_of_nothing() {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(ours(std::process::id()), Whose::Ours);
        // The largest id the kernel can give, which no process has.
        assert_eq!(ours(4_194_304), Whose::Gone);
        assert_eq!(ours(0), Whose::NotKnown);
        // SAFETY: takes nothing and cannot fail.
        let uid = unsafe { libc::geteuid() };
        let first = std::fs::metadata("/proc/1").map(|first| first.uid());
        if uid != 0 && first.is_ok_and(|owner| owner != uid) {
            assert_eq!(ours(1), Whose::Others);
        }
        assert_eq!(whose(Ok(())), Whose::Ours);
        assert_eq!(whose(Err(libc::EPERM)), Whose::Others);
        assert_eq!(whose(Err(libc::ESRCH)), Whose::Gone);
        assert_eq!(whose(Err(libc::EACCES)), Whose::NotKnown);
        assert_eq!(whose(Err(libc::EINVAL)), Whose::NotKnown);
    }

    /// A word is held only where the process that made it is of this user,
    /// as the kernel gives it (`SO_PEERCRED`): the same socket, read as a
    /// process of another user would read it, is none. It is looked at and
    /// not taken: read twice, it says the same.
    #[test]
    fn a_word_counts_only_where_its_maker_is_of_this_user() {
        let start = start_in(&std::fs::read_to_string("/proc/self/stat").unwrap()).unwrap();
        let mut bytes = TAG.to_vec();
        bytes.extend_from_slice(format!("\0{start}\0HOME=/h\0").as_bytes());
        let kept = made(&bytes).unwrap();
        // SAFETY: takes nothing and cannot fail.
        let uid = unsafe { libc::geteuid() };
        for _ in 0..2 {
            let word = read_one(kept.as_raw_fd(), uid).unwrap();
            assert_eq!((word.pid, word.start), (std::process::id(), start));
            assert_eq!(word.value("HOME"), Some(&b"/h"[..]));
        }
        assert!(read_one(kept.as_raw_fd(), uid.wrapping_add(1)).is_none());
    }
}
