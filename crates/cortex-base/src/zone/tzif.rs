//! A compiled zone file (TZif, RFC 8536): the transitions of a zone's
//! offset, and the POSIX rule for the times after the last one.

use super::posix::PosixRule;

/// What a zone file says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transitions {
    /// The moments the offset changes, in seconds since the Unix epoch,
    /// ascending, each with the offset from then on (seconds east of UTC).
    changes: Vec<(i64, i32)>,
    /// The offset before the first change.
    initial: i32,
    /// The rule after the last change, when the file ends with one.
    tail: Option<PosixRule>,
}

/// Reads `N` bytes as a big-endian number.
fn be<const N: usize>(bytes: &[u8], at: usize) -> Option<[u8; N]> {
    bytes.get(at..at + N)?.try_into().ok()
}

struct Header {
    version: u8,
    isutcnt: usize,
    isstdcnt: usize,
    leapcnt: usize,
    timecnt: usize,
    typecnt: usize,
    charcnt: usize,
}

impl Header {
    const LEN: usize = 44;

    fn read(bytes: &[u8], at: usize) -> Option<Self> {
        if bytes.get(at..at + 4)? != b"TZif" {
            return None;
        }
        let count = |n: usize| -> Option<usize> {
            usize::try_from(u32::from_be_bytes(be::<4>(bytes, at + 20 + n * 4)?)).ok()
        };
        Some(Self {
            version: *bytes.get(at + 4)?,
            isutcnt: count(0)?,
            isstdcnt: count(1)?,
            leapcnt: count(2)?,
            timecnt: count(3)?,
            typecnt: count(4)?,
            charcnt: count(5)?,
        })
    }

    /// The length of the data block after this header, with times of
    /// `time_len` bytes.
    fn data_len(&self, time_len: usize) -> usize {
        self.timecnt * time_len
            + self.timecnt
            + self.typecnt * 6
            + self.charcnt
            + self.leapcnt * (time_len + 4)
            + self.isstdcnt
            + self.isutcnt
    }
}

impl Transitions {
    /// Reads a zone file. `None` when the bytes are not one.
    #[must_use]
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let first = Header::read(bytes, 0)?;
        let (header, start, time_len) = if first.version >= b'2' {
            let second_at = Header::LEN + first.data_len(4);
            (Header::read(bytes, second_at)?, second_at + Header::LEN, 8)
        } else {
            (first, Header::LEN, 4)
        };
        if header.typecnt == 0 {
            return None;
        }
        let times_at = start;
        let indexes_at = times_at + header.timecnt * time_len;
        let types_at = indexes_at + header.timecnt;
        let mut offsets = Vec::with_capacity(header.typecnt);
        for n in 0..header.typecnt {
            offsets.push(i32::from_be_bytes(be::<4>(bytes, types_at + n * 6)?));
        }
        let mut changes = Vec::with_capacity(header.timecnt);
        for n in 0..header.timecnt {
            let at = times_at + n * time_len;
            let time = if time_len == 8 {
                i64::from_be_bytes(be::<8>(bytes, at)?)
            } else {
                i64::from(i32::from_be_bytes(be::<4>(bytes, at)?))
            };
            let index = usize::from(*bytes.get(indexes_at + n)?);
            changes.push((time, *offsets.get(index)?));
        }
        if changes.windows(2).any(|pair| pair[0].0 > pair[1].0) {
            return None;
        }
        let tail = if header.version >= b'2' {
            let footer_at = start + header.data_len(time_len);
            let footer = bytes.get(footer_at..)?;
            let footer = footer.strip_prefix(b"\n")?;
            let end = footer.iter().position(|byte| *byte == b'\n')?;
            let text = std::str::from_utf8(&footer[..end]).ok()?;
            if text.is_empty() {
                None
            } else {
                PosixRule::parse(text)
            }
        } else {
            None
        };
        Some(Self {
            changes,
            initial: offsets[0],
            tail,
        })
    }

    /// The offset at `secs` since the Unix epoch, in seconds east of UTC.
    #[must_use]
    pub fn offset_at(&self, secs: i64) -> i32 {
        match self.changes.last() {
            None => self.tail.as_ref().map_or(self.initial, |rule| rule.offset_at(secs)),
            Some((last, _)) if secs >= *last && self.tail.is_some() => self
                .tail
                .as_ref()
                .map_or(self.initial, |rule| rule.offset_at(secs)),
            Some(_) => {
                let after = self.changes.partition_point(|(time, _)| *time <= secs);
                if after == 0 {
                    self.initial
                } else {
                    self.changes[after - 1].1
                }
            }
        }
    }
}
