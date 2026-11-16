//! The Windows half of [`super`]: a named pipe whose access list admits the
//! user alone, and a check of each connection's own token.
//!
//! Not built yet: listening refuses until the pipe's security is in place,
//! so that no build ever listens on a pipe others could open.

use std::io;
use std::path::Path;

use super::{Accepted, ListenOptions, Place};

pub(super) fn default_place(_runtime: Option<&Path>) -> io::Result<Place> {
    Err(unsupported())
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "serving on a named pipe is not in this build yet",
    )
}

#[derive(Debug)]
pub(super) struct Listener {
    place: Place,
}

impl Listener {
    pub(super) fn bind(_place: &Place, _options: &ListenOptions) -> io::Result<Self> {
        Err(unsupported())
    }

    pub(super) fn place(&self) -> &Place {
        &self.place
    }

    #[expect(
        clippy::unused_async,
        reason = "the Unix half's accept awaits; the two halves share one signature"
    )]
    pub(super) async fn accept(&mut self) -> io::Result<Accepted> {
        Err(unsupported())
    }
}
