//! Budgeting (`docs/architecture/07-sessions-and-context.md`, "Budgeting").
//!
//! Before each request the loop estimates its size: what the provider
//! counted for the last request, plus what was added to the history since,
//! at a ratio of bytes to tokens measured from the session's own requests.
//! There is no tokenizer. The estimate is held against a limit, which is
//! the model's context window less room for the answer and a margin.

/// Bytes per token before the provider has counted a request. Low on
/// purpose: an estimate that is too high compacts a little early, one that
/// is too low sends a request that does not fit.
pub const DEFAULT_RATIO: f64 = 3.0;

/// The most room kept for the answer, in tokens, however much more the
/// model may write.
pub const OUTPUT_RESERVE_CAP: u64 = 32_768;

/// The most a request of a model may hold, in tokens: its context window,
/// less the room kept for the answer (the most the role may write, capped
/// at [`OUTPUT_RESERVE_CAP`] and at a quarter of the window), less a margin
/// of a fiftieth of the window.
#[must_use]
pub fn limit_of(window: u64, max_output: u64) -> u64 {
    let reserve = max_output.min(OUTPUT_RESERVE_CAP).min(window / 4);
    let margin = window / 50;
    window.saturating_sub(reserve).saturating_sub(margin)
}

/// What the last request that ended was counted as.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Counted {
    /// Its input, every class, and its output.
    tokens: u64,
    /// The size of the request's content once its answer was recorded.
    bytes_after: u64,
}

/// The session's estimate of a request's size.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Budget {
    counted: Option<Counted>,
    /// Bytes per token, from the last request the provider counted.
    ratio: Option<f64>,
    /// A provider refused a request as too long since it last counted one:
    /// the estimate is known to be low, by an amount the refusal does not
    /// say. A summary request is then cut to half the limit.
    refused: bool,
}

impl Budget {
    /// Bytes per token.
    pub(crate) fn ratio(&self) -> f64 {
        self.ratio.unwrap_or(DEFAULT_RATIO)
    }

    /// Whether a provider refused a request as too long since it last
    /// counted one.
    pub(crate) fn refused(&self) -> bool {
        self.refused
    }

    /// Tokens for so many bytes.
    pub(crate) fn tokens(&self, bytes: u64) -> u64 {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "an estimate: a request is far below 2^52 bytes, and the ratio is positive"
        )]
        let tokens = (bytes as f64 / self.ratio()).ceil() as u64;
        tokens
    }

    /// The estimate for a request whose content is `bytes` now.
    pub(crate) fn estimate(&self, bytes: u64) -> u64 {
        match self.counted {
            Some(counted) => {
                counted.tokens + self.tokens(bytes.saturating_sub(counted.bytes_after))
            }
            None => self.tokens(bytes),
        }
    }

    /// A request whose content was `sent_bytes` was counted as `input`
    /// tokens (every class) and answered with `output`; with its answer the
    /// content is `bytes_after`.
    pub(crate) fn counted(&mut self, sent_bytes: u64, input: u64, output: u64, bytes_after: u64) {
        if input == 0 {
            // A provider that counted nothing says nothing about the ratio.
            return;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "a ratio: the counts are far below 2^52"
        )]
        let ratio = (sent_bytes as f64 / input as f64).clamp(1.0, 8.0);
        self.ratio = Some(ratio);
        self.refused = false;
        self.counted = Some(Counted {
            tokens: input + output,
            bytes_after,
        });
    }

    /// A request whose content was `sent_bytes` did not fit a window of
    /// `window` tokens: it held more tokens to the byte than the ratio said.
    /// The ratio is brought down to what that shows at most, and the count
    /// is dropped, so that the same content is estimated at the window or
    /// more. The true ratio may be lower still: what is cut to fit after an
    /// overflow keeps a margin of its own, until a request is counted again.
    ///
    /// Any request's refusal teaches it, a summary's as well as the turn's
    /// own (the review of step 15 found a refused summary request taught
    /// nothing, and was sent whole again at every turn).
    pub(crate) fn overflowed(&mut self, sent_bytes: u64, window: u64) {
        if window == 0 {
            return;
        }
        self.refused = true;
        #[expect(
            clippy::cast_precision_loss,
            reason = "a ratio: the counts are far below 2^52"
        )]
        let shown = (sent_bytes as f64 / window as f64).clamp(1.0, 8.0);
        self.ratio = Some(self.ratio().min(shown));
        self.counted = None;
    }

    /// A new epoch began: the provider's count is of the old one. The ratio
    /// stays.
    pub(crate) fn new_epoch(&mut self) {
        self.counted = None;
    }
}

/// How full a request is, against the thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pressure {
    /// Below the threshold for a summary in the background.
    Fine,
    /// Past it: a summary is prepared while the session goes on.
    Prepare,
    /// Past the trigger: the epoch is compacted before the request.
    Compact,
}

/// Where `estimate` stands against the trigger and the threshold for a
/// summary in the background, all in tokens.
pub(crate) fn pressure(estimate: u64, prepare: u64, trigger: u64) -> Pressure {
    if estimate > trigger {
        Pressure::Compact
    } else if estimate > prepare {
        Pressure::Prepare
    } else {
        Pressure::Fine
    }
}

/// So many hundredths, or another fraction, of `limit`, in whole tokens.
pub(crate) fn fraction_of(fraction: f64, limit: u64) -> u64 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a threshold: a fraction of a limit far below 2^52, and not negative"
    )]
    let tokens = (fraction.max(0.0) * limit as f64).floor() as u64;
    tokens
}

/// The room kept for a summary when the trigger is placed: 8,192 tokens,
/// or an eighth of the limit on a small window. A summary under the six
/// headings is rarely past 2,000 tokens; the carried state adds the plan,
/// the files and the user's last message.
pub(crate) fn summary_room(limit: u64) -> u64 {
    (limit / 8).min(8_192)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overflow_brings_the_ratio_down_to_what_it_shows() {
        let mut budget = Budget::default();
        // Measured at four bytes a token, so 600,000 bytes looked like
        // 150,000 tokens, and the provider said they do not fit 200,000.
        budget.counted(4_000, 1_000, 0, 4_000);
        assert_eq!(budget.estimate(600_000), 1_000 + 149_000);
        budget.overflowed(600_000, 200_000);
        assert_eq!(budget.estimate(600_000), 200_000);
        // A ratio already lower than what the overflow shows stays.
        budget.overflowed(900_000, 200_000);
        assert_eq!(budget.estimate(600_000), 200_000);
        // Until the provider counts a request again, the estimate is known
        // to be low.
        assert!(budget.refused());
        budget.new_epoch();
        assert!(budget.refused(), "a new epoch is no count");
        budget.counted(3_000, 1_000, 0, 3_000);
        assert!(!budget.refused());
    }

    #[test]
    fn the_limit_keeps_room_for_the_answer_and_a_margin() {
        // A million tokens, an answer of up to 128,000: the room is capped.
        assert_eq!(limit_of(1_000_000, 128_000), 1_000_000 - 32_768 - 20_000);
        // A small local model: a quarter of the window at most for the answer.
        assert_eq!(limit_of(4_096, 2_048), 4_096 - 1_024 - 81);
        assert_eq!(limit_of(200_000, 8_000), 200_000 - 8_000 - 4_000);
        assert_eq!(limit_of(0, 100), 0);
    }

    #[test]
    fn the_estimate_is_what_was_counted_and_what_came_after() {
        let mut budget = Budget::default();
        assert_eq!(budget.estimate(3_000), 1_000, "the default ratio");
        budget.counted(8_000, 2_000, 100, 9_000);
        assert!((budget.ratio() - 4.0).abs() < f64::EPSILON);
        assert_eq!(budget.estimate(9_000), 2_100);
        assert_eq!(budget.estimate(13_000), 2_100 + 1_000);
        budget.new_epoch();
        assert_eq!(budget.estimate(4_000), 1_000, "the ratio stays");
    }

    #[test]
    fn the_thresholds_are_the_limits_fractions() {
        let (prepare, trigger) = (fraction_of(0.7, 100), fraction_of(0.85, 100));
        assert_eq!((prepare, trigger), (70, 85));
        assert_eq!(pressure(69, prepare, trigger), Pressure::Fine);
        assert_eq!(pressure(71, prepare, trigger), Pressure::Prepare);
        assert_eq!(pressure(86, prepare, trigger), Pressure::Compact);
        assert_eq!(fraction_of(-1.0, 100), 0);
        assert_eq!(summary_room(200_000), 8_192);
        assert_eq!(summary_room(3_650), 456);
    }
}
