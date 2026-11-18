//! The scripted provider, run by hand: the test kit's scripted SSE server
//! ([`SseServer`]) as a model that streams long answers to every request, on
//! the loopback only, for the Windows Terminal checklist of step 25
//! (`docs/steps/25/parts/pr3-wt-checklist.md`).
//!
//! ```text
//! cargo run -p cineris-testkit --example scripted-provider --features scripted-provider -- [--port N] [--rate WORDS] [--paragraphs N] [--log FILE]
//! ```
//!
//! It speaks Anthropic's Messages protocol, the one the system tests point
//! the real binary at, on `http://127.0.0.1:<port>` (41917 unless `--port`
//! says otherwise, `0` for any free port). It reads no key and checks none,
//! prints no header, and sends nothing anywhere: it only answers. It runs
//! until `Ctrl+C`.
//!
//! **What it answers.** A turn goes as a model's does:
//!
//! - **A prompt** is answered with the long answer: numbered paragraphs, a
//!   list, a block of code with a line too wide for any window, a line of
//!   wide and accented text, then a call to `list` of the workspace, which
//!   asks nothing.
//! - **That call's result** is answered with three short paragraphs that end
//!   the turn; or, when the prompt had the word `dialog` in it, with a
//!   `write` into the project's `.cineris/` folder, which asks first at every
//!   mode below `unrestricted`, and after its result with the three short
//!   paragraphs.
//!
//! Each piece is a few words, let go on a clock: `--rate` words a second, 50
//! unless it says otherwise, about as fast as a model streams.
//!
//! **Why each reply is chosen one request early.** The server takes its
//! reply from a queue as a request arrives, before anyone can read it, so the
//! reply to request `n + 1` is put in the queue when request `n` arrives. A
//! turn stopped part-way leaves the next prompt the three short paragraphs,
//! once; the log says so (`out of step`), and the turn after it is whole
//! again.
//!
//! **Why a relay.** The server listens on a port the system picks, so that
//! tests never collide. The checklist's configuration names a port once, so
//! this program listens on that port and passes each connection through to
//! the server, byte for byte, both ways.
//!
//! **The log.** One line for each request: its number, what it was (a
//! prompt, with its lines and characters and its first words; a call's
//! result), and what it is answered with; and one when each answer ends,
//! whole or cut off by the client. On standard output, and with `--log` in
//! that file too.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "a program of its own, run by hand: it prints its log, writes it to a file, and lets \
              its answers go on a real clock, as a model's come"
)]

use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cineris_testkit::conformance::anthropic::body;
use cineris_testkit::{Gate, ReceivedRequest, ServerReply, SseServer};
use serde_json::{Value, json};

/// The port the checklist's configuration names.
const DEFAULT_PORT: u16 = 41_917;
/// The model the replies say they are: the one the checklist's
/// configuration names, of the built-in catalog.
const MODEL: &str = "claude-haiku-5-5";
/// The words of one piece of an answer.
const WORDS_PER_PIECE: usize = 2;
/// The file the `dialog` turn's `write` asks to make.
const NOTE: &str = ".cineris/checklist-note.md";

const USAGE: &str =
    "usage: scripted-provider [--port N] [--rate WORDS] [--paragraphs N] [--log FILE]

  --port N         the loopback port to answer on (41917; 0 for any free one)
  --rate WORDS     words a second each answer streams at (50)
  --paragraphs N   numbered paragraphs in the long answer (18)
  --log FILE       append each line of the log to FILE too";

fn main() -> ExitCode {
    let options = match Options::parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("scripted-provider: {error}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("scripted-provider: {error}");
            ExitCode::FAILURE
        }
    }
}

/// What the command line asked for.
struct Options {
    port: u16,
    rate: u32,
    paragraphs: usize,
    log: Option<PathBuf>,
}

impl Options {
    /// `None` for `--help`.
    fn parse(mut arguments: impl Iterator<Item = String>) -> Result<Option<Self>, String> {
        let mut options = Self {
            port: DEFAULT_PORT,
            rate: 50,
            paragraphs: 18,
            log: None,
        };
        while let Some(argument) = arguments.next() {
            let mut value = |flag: &str| {
                arguments
                    .next()
                    .ok_or_else(|| format!("`{flag}` takes a value"))
            };
            match argument.as_str() {
                "--port" => {
                    options.port = value("--port")?
                        .parse()
                        .map_err(|_| "`--port` takes a port, 0 to 65535".to_owned())?;
                }
                "--rate" => {
                    options.rate = value("--rate")?
                        .parse()
                        .ok()
                        .filter(|rate| (1..=10_000).contains(rate))
                        .ok_or("`--rate` takes a number of words, 1 to 10000")?;
                }
                "--paragraphs" => {
                    options.paragraphs = value("--paragraphs")?
                        .parse()
                        .ok()
                        .filter(|count| (1..=1_000).contains(count))
                        .ok_or("`--paragraphs` takes a number, 1 to 1000")?;
                }
                "--log" => options.log = Some(PathBuf::from(value("--log")?)),
                "--help" | "-h" => return Ok(None),
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        Ok(Some(options))
    }

    /// The time between two pieces of an answer.
    fn interval(&self) -> Duration {
        Duration::from_secs_f64(WORDS_PER_PIECE as f64 / f64::from(self.rate))
    }
}

fn run(options: &Options) -> Result<(), String> {
    let file = match &options.log {
        Some(path) => Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|error| format!("{} could not be opened: {error}", path.display()))?,
        ),
        None => None,
    };
    let log = Arc::new(Log {
        file: Mutex::new(file),
    });
    let listener = TcpListener::bind(("127.0.0.1", options.port)).map_err(|error| {
        format!(
            "could not listen on 127.0.0.1:{}: {error}. Another port, with `--port`, and the \
             same port in the configuration's `base_url`",
            options.port
        )
    })?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("the listener has no address: {error}"))?;
    let server = SseServer::start();
    let behind = server
        .base_url()
        .trim_start_matches("http://")
        .parse::<SocketAddr>()
        .map_err(|error| format!("the server's address cannot be read: {error}"))?;
    thread::Builder::new()
        .name("relay".to_owned())
        .spawn(move || relay(&listener, behind))
        .map_err(|error| format!("the relay could not start: {error}"))?;

    log.line(&format!("base_url = \"http://{address}\""));
    log.line(&format!(
        "ready: answering on http://{address}, {} words a second, {} paragraphs a long answer; \
         Ctrl+C ends it",
        options.rate, options.paragraphs
    ));

    let mut queued = Planned::new(Kind::Long, 1, options);
    server.push(queued.reply.take().unwrap_or_else(ServerReply::sse));
    loop {
        // An hour at a time: nothing but the next request opens it.
        while !queued.arrived.wait(Duration::from_secs(3_600)) {}
        let requests = server.requests();
        let number = requests.len();
        let seen = requests
            .last()
            .map_or(Seen::Other("nothing".to_owned()), Seen::of);
        let mut line = format!("request {number}: {}", seen.describe());
        if number > queued.number {
            line.push_str(&format!(
                " (requests {} to {} found no reply waiting, and were answered 500)",
                queued.number,
                number - 1
            ));
        }
        line.push_str(&format!(" -> {}", queued.describe(options)));
        if let Some(why) = queued.out_of_step(&seen) {
            line.push_str(&format!(" [out of step: {why}]"));
        }
        log.line(&line);

        let next = match queued.kind {
            Kind::Long if seen.asks_for_a_dialog() => Kind::Ask,
            Kind::Long | Kind::Ask => Kind::Closing,
            Kind::Closing => Kind::Long,
        };
        let mut planned = Planned::new(next, number + 1, options);
        server.push(planned.reply.take().unwrap_or_else(ServerReply::sse));
        let playing = std::mem::replace(&mut queued, planned);
        let log = Arc::clone(&log);
        let interval = options.interval();
        thread::Builder::new()
            .name(format!("pacer-{number}"))
            .spawn(move || playing.pace(number, interval, &log))
            .map_err(|error| format!("a pacer could not start: {error}"))?;
    }
}

/// The log: standard output, and a file when one was given.
struct Log {
    file: Mutex<Option<File>>,
}

impl Log {
    fn line(&self, text: &str) {
        let line = format!("{}  {text}", time_of_day());
        println!("{line}");
        let mut file = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(file) = file.as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }
}

/// The time of day in UTC, to the millisecond: `09:42:07.118Z`.
fn time_of_day() -> String {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = since.as_secs() % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03}Z",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60,
        since.subsec_millis()
    )
}

/// Passes every connection on `listener` through to the server at
/// `behind`, both ways.
fn relay(listener: &TcpListener, behind: SocketAddr) {
    for client in listener.incoming() {
        let Ok(client) = client else { continue };
        let _ = thread::Builder::new()
            .name("relay-connection".to_owned())
            .spawn(move || {
                let _ = forward(client, behind);
            });
    }
}

/// One connection: the request on to the server, the answer back, and a
/// client that hangs up closes the server's side too, so that the server
/// stops writing to it, as it would to the client itself.
fn forward(client: TcpStream, behind: SocketAddr) -> io::Result<()> {
    let server = TcpStream::connect(behind)?;
    client.set_nodelay(true)?;
    server.set_nodelay(true)?;
    let mut from_client = client.try_clone()?;
    let mut to_server = server.try_clone()?;
    let upstream = thread::Builder::new()
        .name("relay-upstream".to_owned())
        .spawn(move || {
            // The end of the client's request: an HTTP client writes nothing
            // more, and its side's end means it is done or gone.
            let _ = io::copy(&mut from_client, &mut to_server);
            let _ = to_server.shutdown(Shutdown::Write);
        })?;
    let (mut from_server, mut to_client) = (server, client);
    let _ = io::copy(&mut from_server, &mut to_client);
    // The answer ended, or the client stopped taking it: either way both
    // connections are done.
    let _ = to_client.shutdown(Shutdown::Both);
    let _ = from_server.shutdown(Shutdown::Both);
    let _ = upstream.join();
    Ok(())
}

/// What a reply is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The long answer and a call to `list`: for a prompt.
    Long,
    /// A `write` that asks first: for `list`'s result, after a prompt with
    /// the word `dialog`.
    Ask,
    /// Three short paragraphs that end the turn: for a call's result.
    Closing,
}

/// A reply put in the queue, and what lets its pieces go.
struct Planned {
    kind: Kind,
    /// The request it is meant for, counted from 1.
    number: usize,
    /// Taken when the reply is put in the queue.
    reply: Option<ServerReply>,
    /// Opened by the server when the reply begins.
    arrived: Gate,
    /// The pieces' gates, one for each piece after the first, and the gates
    /// the server opens once it has written each piece.
    pieces: Vec<(Gate, Gate)>,
    /// Opened by the server once it has written the reply's end.
    finished: Gate,
    words: usize,
}

impl Planned {
    fn new(kind: Kind, number: usize, options: &Options) -> Self {
        let text = match kind {
            Kind::Long => long_answer(number, options.paragraphs),
            Kind::Ask => ask_text(number),
            Kind::Closing => closing_text(number),
        };
        let pieces = pieces(&text);
        let words = text.split_whitespace().count();
        let arrived = Gate::new();
        let finished = Gate::new();
        let mut gates = Vec::new();
        let mut reply = ServerReply::sse().mark(&arrived).send(body(&[
            json!({"type": "message_start", "message": {"model": MODEL,
                "id": format!("msg_scripted_{number}"), "type": "message",
                "role": "assistant", "content": [], "stop_reason": null,
                "usage": {"input_tokens": 1_200, "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0, "output_tokens": 1}}}),
            json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": ""}}),
        ]));
        for (index, piece) in pieces.iter().enumerate() {
            if index > 0 {
                let go = Gate::new();
                reply = reply.wait(&go);
                let written = Gate::new();
                gates.push((go, written));
            }
            reply = reply.send(body(&[json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": piece}})]));
            if let Some((_, written)) = gates.last() {
                reply = reply.mark(written);
            }
        }
        let mut end = vec![json!({"type": "content_block_stop", "index": 0})];
        let call = match kind {
            Kind::Long => Some(("list", json!({"path": "."}))),
            Kind::Ask => Some((
                "write",
                json!({"path": NOTE, "content": "Written by the scripted provider, after the dialog was answered.\n"}),
            )),
            Kind::Closing => None,
        };
        let output_tokens = words * 4 / 3;
        if let Some((tool, input)) = call {
            end.extend([
                json!({"type": "content_block_start", "index": 1,
                    "content_block": {"type": "tool_use", "id": format!("toolu_scripted_{number}"),
                        "name": tool, "input": {}}}),
                json!({"type": "content_block_delta", "index": 1,
                    "delta": {"type": "input_json_delta", "partial_json": input.to_string()}}),
                json!({"type": "content_block_stop", "index": 1}),
                json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
                    "usage": {"output_tokens": output_tokens}}),
            ]);
        } else {
            end.push(
                json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
                "usage": {"output_tokens": output_tokens}}),
            );
        }
        end.push(json!({"type": "message_stop"}));
        let reply = reply.send(body(&end)).mark(&finished);
        Self {
            kind,
            number,
            reply: Some(reply),
            arrived,
            pieces: gates,
            finished,
            words,
        }
    }

    /// What the reply is, for the log.
    fn describe(&self, options: &Options) -> String {
        let seconds = self
            .words
            .div_ceil(usize::try_from(options.rate).unwrap_or(1));
        match self.kind {
            Kind::Long => format!(
                "the long answer ({} words, about {seconds} s), then a call to `list`",
                self.words
            ),
            Kind::Ask => format!(
                "a short answer ({} words), then a `write` to {NOTE}, which asks first",
                self.words
            ),
            Kind::Closing => format!(
                "the closing paragraphs ({} words, about {seconds} s), which end the turn",
                self.words
            ),
        }
    }

    /// Why the request is not the one the reply was meant for, if it is not.
    fn out_of_step(&self, seen: &Seen) -> Option<&'static str> {
        match (self.kind, seen) {
            (Kind::Long, Seen::Result { .. }) => {
                Some("a call's result took the answer meant for a prompt")
            }
            (Kind::Ask | Kind::Closing, Seen::Prompt { .. }) => Some(
                "a prompt took the answer meant for a call's result: the turn before did not \
                 finish its call; the next prompt is answered whole",
            ),
            (_, Seen::Other(_)) => Some("not a model's request"),
            _ => None,
        }
    }

    /// Lets the pieces go on the clock, one each `interval`, and says how the
    /// reply ended.
    fn pace(self, number: usize, interval: Duration, log: &Log) {
        let started = Instant::now();
        let mut due = started;
        for (index, (go, _)) in self.pieces.iter().enumerate() {
            due += interval;
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                thread::sleep(wait);
            }
            // The piece before this one was written, or the server stopped:
            // the client hung up, and nothing more of this reply goes.
            let before = index.checked_sub(1).map(|at| &self.pieces[at].1);
            if before.is_some_and(|written| !written.wait(Duration::from_secs(2))) {
                log.line(&format!(
                    "request {number}: cut off by the client after {} s, at piece {} of {}",
                    started.elapsed().as_secs(),
                    index,
                    self.pieces.len() + 1
                ));
                return;
            }
            go.open();
        }
        if self.finished.wait(Duration::from_secs(5)) {
            log.line(&format!(
                "request {number}: answered whole in {:.1} s",
                started.elapsed().as_secs_f64()
            ));
        } else {
            log.line(&format!(
                "request {number}: cut off by the client at its end, after {} s",
                started.elapsed().as_secs()
            ));
        }
    }
}

/// What a request was.
enum Seen {
    /// The person's prompt: each text block's lines and characters, and the
    /// first words of the last.
    Prompt {
        blocks: Vec<(usize, usize)>,
        start: String,
        dialog: bool,
    },
    /// A call's result.
    Result { id: String },
    /// Not a model's request, or not one this can read.
    Other(String),
}

impl Seen {
    fn of(request: &ReceivedRequest) -> Self {
        if request.method != "POST" || !request.path.ends_with("/v1/messages") {
            return Self::Other(format!("{} {}", request.method, request.path));
        }
        let Ok(body) = serde_json::from_slice::<Value>(&request.body) else {
            return Self::Other("a body that is not JSON".to_owned());
        };
        let Some(last) = body["messages"].as_array().and_then(|all| all.last()) else {
            return Self::Other("no messages".to_owned());
        };
        let texts: Vec<&str> = match &last["content"] {
            Value::String(text) => vec![text.as_str()],
            Value::Array(blocks) => {
                if let Some(result) = blocks.iter().find(|block| block["type"] == "tool_result") {
                    return Self::Result {
                        id: result["tool_use_id"].as_str().unwrap_or("?").to_owned(),
                    };
                }
                blocks
                    .iter()
                    .filter(|block| block["type"] == "text")
                    .filter_map(|block| block["text"].as_str())
                    .collect()
            }
            _ => Vec::new(),
        };
        let start: String = texts
            .last()
            .map(|text| {
                let first: String = text.chars().take(40).collect();
                let more = if text.chars().count() > 40 { "…" } else { "" };
                format!("{}{more}", first.escape_debug())
            })
            .unwrap_or_default();
        Self::Prompt {
            blocks: texts
                .iter()
                .map(|text| (text.lines().count(), text.chars().count()))
                .collect(),
            start,
            dialog: texts
                .iter()
                .any(|text| text.to_lowercase().contains("dialog")),
        }
    }

    fn asks_for_a_dialog(&self) -> bool {
        matches!(self, Self::Prompt { dialog: true, .. })
    }

    fn describe(&self) -> String {
        match self {
            Self::Prompt { blocks, start, .. } => {
                let sizes: Vec<String> = blocks
                    .iter()
                    .map(|(lines, chars)| {
                        format!(
                            "{lines} line{}, {chars} characters",
                            if *lines == 1 { "" } else { "s" }
                        )
                    })
                    .collect();
                format!(
                    "a prompt in {} text block{} ({}): \"{start}\"",
                    blocks.len(),
                    if blocks.len() == 1 { "" } else { "s" },
                    sizes.join("; ")
                )
            }
            Self::Result { id } => format!("the result of the call {id}"),
            Self::Other(what) => format!("something else: {what}"),
        }
    }
}

/// `text` in pieces of a few words, each with the spaces and line breaks
/// after it, so that the pieces put together are the text.
fn pieces(text: &str) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut words = 0;
    let mut in_word = false;
    for character in text.chars() {
        let space = character.is_whitespace();
        if !space && !in_word && words == WORDS_PER_PIECE {
            pieces.push(std::mem::take(&mut piece));
            words = 0;
        }
        if !space && !in_word {
            words += 1;
        }
        in_word = !space;
        piece.push(character);
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    pieces
}

/// The paragraphs the long answer goes round.
const SEASONS: [&str; 6] = [
    "The garden begins in early spring, when the ground is still cold and the first green \
     shoots push up through last year's leaves. Nothing is planted yet. The beds are turned over \
     by hand, the stones the frost lifted are carried to the wall, and the winter's compost is \
     spread thin, so that the soil can breathe before anything is asked of it.",
    "By the middle of spring the rows are marked out with string and short wooden pegs. Peas go \
     in first, along the fence where they can climb, then lettuce and radishes in short rows a \
     week apart, so that they do not all come ready at once. Each row is labelled with the day it \
     was sown, in pencil, which the rain does not wash away.",
    "Summer brings the long evenings and most of the work. Water is carried in the cool of the \
     morning, before the sun is high, and the paths between the beds are weeded once a week. The \
     tomatoes are tied to their canes as they grow, a soft loop of twine every hand's width, and \
     their side shoots are pinched out so that the plant puts its strength into the fruit.",
    "The first harvest is small and comes all at once: a bowl of peas, a handful of radishes, \
     more lettuce than anyone can eat. What cannot be eaten is given away over the fence. Seeds \
     are saved from the best plants, dried on paper in the shed, and folded into envelopes with \
     the name and the year written on the front.",
    "Autumn is for clearing. The spent plants are pulled and chopped for the compost, the beds \
     that will rest are covered with straw, and garlic is set in the cold ground to wait for the \
     spring. The tools are cleaned, oiled and hung on their nails, and the notebook is read from \
     its first page, to see what worked and what did not.",
    "Winter is quiet. The garden is planned again at the kitchen table, with the notebook open and \
     the catalogues spread out beside it. A new bed is drawn where the old one flooded, a \
     different bean is chosen because the last one was tough, and the list of seeds to order is \
     written out twice, once to send and once to keep.",
];

/// The block of code in the long answer, with one line wider than any
/// window, which the screen cuts and `/copy` keeps whole.
const CODE: &str = "```rust
/// How many days each crop takes from sowing to harvest.
fn days_to_harvest(crop: &str) -> Option<u32> {
    match crop {
        \"radish\" => Some(28),
        \"lettuce\" => Some(45),
        \"pea\" => Some(60),
        \"tomato\" => Some(80),
        _ => None,
    }
}

fn main() {
    for crop in [\"radish\", \"lettuce\", \"pea\", \"tomato\", \"garlic\"] {
        match days_to_harvest(crop) {
            Some(days) => println!(\"{crop}: ready about {days} days after sowing, if the weather is kind and the slugs are kept off the young leaves\"),
            None => println!(\"{crop}: not in the table\"),
        }
    }
}
```";

/// The long answer to request `number`.
fn long_answer(number: usize, paragraphs: usize) -> String {
    let mut text = format!(
        "This is the answer to request {number}: {paragraphs} numbered paragraphs, a list, a \
         block of code and a line of wide text, then a listing of the workspace.\n\n"
    );
    let middle = paragraphs.div_ceil(2);
    for at in 1..=paragraphs {
        let season = SEASONS[(at - 1) % SEASONS.len()];
        text.push_str(&format!("Paragraph {at} of {paragraphs}. {season}\n\n"));
        if at == middle {
            text.push_str(
                "- Each paragraph is numbered, so that a gap or a repeat in the scrollback is \
                 easy to see.\n\
                 - `inline code`, **bold** and *italic* text are drawn in the answer's own \
                 styles.\n\
                 - This item is long enough to wrap at a narrow width, which is what the rows \
                 that resize the window look at, when it is made narrower and wider again while \
                 the answer streams.\n\n",
            );
            text.push_str(CODE);
            text.push_str("\n\n");
        }
    }
    text.push_str(
        "Wide and accented text, for the glyphs: café, naïve, Ελληνικά, Русский, 日本語の文, \
         한국어, and a seedling 🌱.\n\n",
    );
    text.push_str(&format!(
        "The answer to request {number} ends here, and the listing follows."
    ));
    text
}

/// The text before the `write` that asks first.
fn ask_text(number: usize) -> String {
    format!(
        "This is the answer to request {number}, after the listing. Before the turn ends, a note \
         goes into the project's settings folder, `.cineris/`, which asks first: the dialog is \
         next."
    )
}

/// The three short paragraphs that end a turn.
fn closing_text(number: usize) -> String {
    format!(
        "This is the answer to request {number}, after the call. Three short paragraphs end the \
         turn.\n\n\
         Closing 1 of 3. The listing came back, and nothing in it needs to change.\n\n\
         Closing 2 of 3. The next prompt starts a new turn, with a long answer of its own.\n\n\
         Closing 3 of 3. The turn of request {number} is complete."
    )
}
