//! What carries a connection's bytes: two channels, whatever is at their
//! other ends (`docs/architecture/11-rpc-protocol.md`, "Transports").
//!
//! A connection reads chunks of bytes from one channel and writes whole
//! lines to the other. Behind them is standard I/O (two threads of their
//! own, [`Transport::blocking`]), a local socket, or a test's own ends
//! ([`Transport::pair`]). Both channels are bounded, so a client that
//! sends faster than the server reads, or reads slower than it writes,
//! waits in its own stream and holds nothing more in the server.
//!
//! The threads of a blocking stream are not joined: a read on a pipe that
//! never closes would otherwise hold the end of the process
//! (`docs/architecture/02-runtime.md`, "Async and concurrency"). They end
//! by themselves when the stream ends or the connection lets go of its
//! channel, and with the process.

use std::io::{self, Read, Write};

use tokio::sync::mpsc;

/// How much is read from a blocking stream at a time.
const CHUNK: usize = 64 * 1024;

/// How many chunks may wait for the connection before the reading side
/// stops reading.
const CHUNKS: usize = 4;

/// A connection's two channels.
#[derive(Debug)]
pub struct Transport {
    /// Bytes from the client, in chunks of any size. The channel's end is
    /// the end of the client's stream; an error ends it too.
    pub input: mpsc::Receiver<io::Result<Vec<u8>>>,
    /// Lines to the client, each a whole message with its `\n`. A send that
    /// waits is a client that is not reading; a send that fails is a
    /// stream that has closed.
    pub output: mpsc::Sender<Vec<u8>>,
}

/// The client's ends of a [`Transport::pair`].
#[derive(Debug)]
pub struct Ends {
    /// Bytes to the server.
    pub input: mpsc::Sender<io::Result<Vec<u8>>>,
    /// Lines from the server, one message each.
    pub output: mpsc::Receiver<Vec<u8>>,
}

impl Transport {
    /// A transport whose other ends are channels of the caller's: for a
    /// client in the same process (a test, or a supervisor that drives a
    /// session in process as it would over a pipe). `lines` is how many
    /// lines may wait for the client before the server's writer waits.
    #[must_use]
    pub fn pair(lines: usize) -> (Self, Ends) {
        let (to_server, input) = mpsc::channel(CHUNKS);
        let (output, from_server) = mpsc::channel(lines.max(1));
        (
            Self { input, output },
            Ends {
                input: to_server,
                output: from_server,
            },
        )
    }

    /// A transport over two blocking streams, each served by a thread of its
    /// own that is not joined: standard input and output for `serve
    /// --stdio`. With it, a way to wait until everything sent has been
    /// written ([`Written`]): the process must not end with its last line
    /// still in the writing thread's hands.
    ///
    /// # Errors
    ///
    /// When a thread cannot be started.
    pub fn blocking(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> io::Result<(Self, Written)> {
        let (chunks, input) = mpsc::channel(CHUNKS);
        // One line waits while the thread writes the one before it: the
        // connection's writer knows a client that stopped reading within a
        // line of it.
        let (output, lines) = mpsc::channel::<Vec<u8>>(1);
        std::thread::Builder::new()
            .name("cineris-rpc-in".to_owned())
            .spawn(move || read_into(reader, &chunks))?;
        let (finished, written) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("cineris-rpc-out".to_owned())
            .spawn(move || {
                write_from(writer, lines);
                // Dropped when the last line is written: the waiter is told.
                drop(finished);
            })?;
        Ok((Self { input, output }, Written(written)))
    }
}

/// Says when a blocking stream's writing thread has written everything it
/// was sent, which is once the transport's output is dropped and the
/// lines before it are out.
#[derive(Debug)]
pub struct Written(std::sync::mpsc::Receiver<()>);

impl Written {
    /// Waits at most `limit` for the last line to be written. False when
    /// the time ran out first: a reader that stopped reading holds it.
    #[must_use]
    pub fn wait(&self, limit: std::time::Duration) -> bool {
        matches!(
            self.0.recv_timeout(limit),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) | Ok(())
        )
    }
}

/// Reads `reader` into `chunks` until it ends, fails, or nobody takes the
/// chunks any more.
fn read_into(mut reader: impl Read, chunks: &mpsc::Sender<io::Result<Vec<u8>>>) {
    let mut buffer = vec![0; CHUNK];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return,
            Ok(read) => {
                if chunks.blocking_send(Ok(buffer[..read].to_vec())).is_err() {
                    return;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                let _ = chunks.blocking_send(Err(error));
                return;
            }
        }
    }
}

/// Writes each line to `writer`, flushed, until the lines end or a write
/// fails: then the receiver is dropped, and the connection's next send
/// fails, which is how it learns the stream has closed.
fn write_from(mut writer: impl Write, mut lines: mpsc::Receiver<Vec<u8>>) {
    while let Some(line) = lines.blocking_recv() {
        if writer
            .write_all(&line)
            .and_then(|()| writer.flush())
            .is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that gives its bytes in the pieces it is given.
    struct Pieces(std::collections::VecDeque<Vec<u8>>);

    impl Read for Pieces {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let Some(piece) = self.0.pop_front() else {
                return Ok(0);
            };
            buffer[..piece.len()].copy_from_slice(&piece);
            Ok(piece.len())
        }
    }

    /// A writer that hands each write to the test.
    struct Writes(std::sync::mpsc::Sender<Vec<u8>>);

    impl Write for Writes {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let _ = self.0.send(bytes.to_vec());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_blocking_stream_is_read_to_its_end_and_lines_are_written_whole() {
        let (writes, written) = std::sync::mpsc::channel();
        let reader = Pieces(vec![b"ab".to_vec(), b"c\n".to_vec()].into());
        let (mut transport, done) = Transport::blocking(reader, Writes(writes)).unwrap();
        let mut read = Vec::new();
        while let Some(chunk) = transport.input.blocking_recv() {
            read.extend(chunk.unwrap());
        }
        assert_eq!(read, b"abc\n");
        transport.output.blocking_send(b"{}\n".to_vec()).unwrap();
        assert_eq!(written.recv().unwrap(), b"{}\n");
        // Once the output is let go, the thread is done with every line.
        drop(transport.output);
        assert!(done.wait(std::time::Duration::from_secs(60)));
    }
}
