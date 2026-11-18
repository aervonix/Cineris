//! A module doc, which belongs to no definition.

use std::fmt;

/// The largest size a frame may have.
pub const MAX_FRAME: usize = 64 * 1024;

static GREETING: &str = "fn not_a_function() { }";

/// A parser of frames.
#[derive(Debug, Clone)]
pub struct Parser<'a> {
    input: &'a str,
    at: usize,
}

/// What went wrong.
pub enum Error {
    /// The input ended.
    Eof,
    Bad { at: usize },
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl<'a> Parser<'a> {
    /// Makes a parser over `input`.
    pub fn new(input: &'a str) -> Self {
        let braces = "{{{";
        Self { input, at: 0 }
    }

    /// Reads the next frame.
    pub fn next_frame(
        &mut self,
        limit: usize,
    ) -> Result<&'a str> {
        fn helper() {}
        let c = '{';
        Ok(&self.input[self.at..limit])
    }

    const fn width(&self) -> usize { 1 }
}

impl fmt::Display for Parser<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "parser at {}", self.at)
    }
}

pub trait Source {
    /// Reads bytes.
    fn read(&mut self, into: &mut [u8]) -> usize;

    fn name(&self) -> &str {
        "unnamed"
    }
}

impl<T> Source for Vec<T>
where
    T: Copy,
{
    fn read(&mut self, _into: &mut [u8]) -> usize {
        0
    }
}

macro_rules! frame {
    ($x:expr) => { $x };
}

pub mod wire {
    pub struct Header;

    pub async unsafe fn decode(raw: &[u8]) {}
}

extern "C" {
    fn c_side(x: i32) -> i32;
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {}
}
