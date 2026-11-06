//! The call to Ollama, over a plain TCP socket.
//!
//! This uses `std::net`, not `ureq`, for one reason: when a timer fires the
//! socket must really close, or Ollama keeps writing on a shared server. A
//! blocking `ureq` call can't be stopped from outside and has one read timeout
//! per request. Plain HTTP only, because this runs on the internal Docker
//! network.

use crate::codes::Code;
use crate::prompt::quick_schema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

pub const MODEL: &str = "gemma3:4b";
pub const NUM_CTX: u64 = 40_960;
pub const NUM_PREDICT: u64 = 12_288;
pub const SEED: u64 = 11;
const KEEP_ALIVE: &str = "30m";
/// The most an error body or response head is read.
const MAX_SMALL: usize = 64 * 1024;
/// The unload call gets this long after connecting.
const UNLOAD_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub connect: Duration,
    pub first_token: Duration,
    pub gap: Duration,
    pub whole: Duration,
}

impl Limits {
    pub const REAL: Limits = Limits {
        connect: Duration::from_secs(30),
        first_token: Duration::from_secs(25 * 60),
        gap: Duration::from_secs(5 * 60),
        whole: Duration::from_secs(120 * 60),
    };
}

/// What Ollama's done line reports. `None`: it didn't say.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub prompt_eval_count: Option<u64>,
    pub eval_count: Option<u64>,
    pub total_duration: Option<u64>,
    pub load_duration: Option<u64>,
    pub prompt_eval_duration: Option<u64>,
    pub eval_duration: Option<u64>,
    pub done_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ChatOutcome {
    /// The reply so far, whole or partial.
    pub reply: String,
    pub counters: Counters,
    /// `None`: a done line arrived.
    pub failure: Option<Code>,
    pub http_status: Option<u16>,
    pub first_token_ms: Option<u64>,
    pub wall_ms: u64,
}

/// A bad `OLLAMA_URL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UrlError;

#[derive(Debug, Clone)]
pub struct OllamaUrl {
    authority: String,
    prefix: String,
}

impl OllamaUrl {
    /// `http://host[:port][/prefix]`.
    pub fn parse(s: &str) -> Result<OllamaUrl, UrlError> {
        let rest = s.strip_prefix("http://").ok_or(UrlError)?;
        let (host, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        if host.is_empty() {
            return Err(UrlError);
        }
        let has_port = host
            .rsplit_once(':')
            .is_some_and(|(_, p)| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
        let authority = if has_port {
            host.to_string()
        } else {
            format!("{host}:80")
        };
        let prefix = prefix.trim_matches('/');
        Ok(OllamaUrl {
            authority,
            prefix: if prefix.is_empty() {
                String::new()
            } else {
                format!("/{prefix}")
            },
        })
    }

    pub fn authority(&self) -> String {
        self.authority.clone()
    }

    pub fn path(&self, p: &str) -> String {
        format!("{}/{p}", self.prefix)
    }
}

/// The chat request body for `prompt`.
pub fn chat_request(prompt: &str) -> Vec<u8> {
    json!({
        "model": MODEL,
        "stream": true,
        "truncate": false,
        "shift": false,
        "keep_alive": KEEP_ALIVE,
        "format": quick_schema(),
        "options": {
            "num_ctx": NUM_CTX,
            "num_predict": NUM_PREDICT,
            "temperature": 0.2,
            "seed": SEED,
            "top_k": 64,
            "top_p": 0.95,
            "repeat_penalty": 1.0
        },
        "messages": [{"role": "user", "content": prompt}]
    })
    .to_string()
    .into_bytes()
}

// ---------- timers ----------

struct Timers {
    start: Instant,
    sent: Instant,
    last_token: Option<Instant>,
    first_token: Duration,
    gap: Duration,
    whole: Duration,
}

impl Timers {
    /// The next deadline, and the code it fires with.
    fn next(&self) -> (Instant, Code) {
        let token = match self.last_token {
            None => (self.sent + self.first_token, Code::TimeFirstToken),
            Some(t) => (t + self.gap, Code::TimeGap),
        };
        let whole = (self.start + self.whole, Code::TimeWhole);
        if whole.0 <= token.0 { whole } else { token }
    }
}

enum Stop {
    Timer(Code),
    Broken,
}

/// One read, bounded by the timers. `Ok(0)` is the end of the stream.
fn read_some(s: &mut TcpStream, timers: &Timers, tmp: &mut [u8]) -> Result<usize, Stop> {
    loop {
        let (deadline, code) = timers.next();
        let now = Instant::now();
        if now >= deadline {
            return Err(Stop::Timer(code));
        }
        s.set_read_timeout(Some(deadline - now))
            .map_err(|_| Stop::Broken)?;
        match s.read(tmp) {
            Ok(n) => return Ok(n),
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(_) => return Err(Stop::Broken),
        }
    }
}

// ---------- HTTP framing ----------

enum Framing {
    Chunked(Chunks),
    Length(u64),
    UntilClose,
}

enum Chunks {
    Size(Vec<u8>),
    Data(u64),
    DataEnd(u8),
    Trailer(Vec<u8>),
    Finished,
}

/// A chunk header or framing error.
struct BadFrame;

impl Framing {
    /// Moves body bytes from `input` to `out`. True when the body is complete.
    fn feed(&mut self, input: &[u8], out: &mut Vec<u8>) -> Result<bool, BadFrame> {
        match self {
            Framing::UntilClose => {
                out.extend_from_slice(input);
                Ok(false)
            }
            Framing::Length(left) => {
                let take = input
                    .len()
                    .min(usize::try_from(*left).unwrap_or(usize::MAX));
                out.extend_from_slice(&input[..take]);
                *left -= take as u64;
                Ok(*left == 0)
            }
            Framing::Chunked(state) => {
                let mut i = 0;
                while i < input.len() {
                    match state {
                        Chunks::Size(line) => {
                            let b = input[i];
                            i += 1;
                            if b != b'\n' {
                                line.push(b);
                                if line.len() > 64 {
                                    return Err(BadFrame);
                                }
                                continue;
                            }
                            let text = std::str::from_utf8(line).map_err(|_| BadFrame)?;
                            let hex = text.split(';').next().unwrap_or("").trim();
                            let n = u64::from_str_radix(hex, 16).map_err(|_| BadFrame)?;
                            *state = if n == 0 {
                                Chunks::Trailer(Vec::new())
                            } else {
                                Chunks::Data(n)
                            };
                        }
                        Chunks::Data(left) => {
                            let take =
                                (input.len() - i).min(usize::try_from(*left).unwrap_or(usize::MAX));
                            out.extend_from_slice(&input[i..i + take]);
                            i += take;
                            *left -= take as u64;
                            if *left == 0 {
                                *state = Chunks::DataEnd(2);
                            }
                        }
                        Chunks::DataEnd(left) => {
                            let want = if *left == 2 { b'\r' } else { b'\n' };
                            if input[i] != want {
                                return Err(BadFrame);
                            }
                            i += 1;
                            *left -= 1;
                            if *left == 0 {
                                *state = Chunks::Size(Vec::new());
                            }
                        }
                        Chunks::Trailer(line) => {
                            let b = input[i];
                            i += 1;
                            if b != b'\n' {
                                line.push(b);
                            } else if line.iter().all(|c| *c == b'\r') {
                                *state = Chunks::Finished;
                            } else {
                                line.clear();
                            }
                        }
                        Chunks::Finished => return Ok(true),
                    }
                }
                Ok(matches!(state, Chunks::Finished))
            }
        }
    }
}

/// The response head: status, framing, and any body bytes read with it.
fn read_head(s: &mut TcpStream, timers: &Timers) -> Result<(u16, Framing, Vec<u8>), Stop> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    let end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if buf.len() > MAX_SMALL {
            return Err(Stop::Broken);
        }
        let n = read_some(s, timers, &mut tmp)?;
        if n == 0 {
            return Err(Stop::Broken);
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or(Stop::Broken)?;
    let mut framing = Framing::UntilClose;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        if k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked")
        {
            framing = Framing::Chunked(Chunks::Size(Vec::new()));
            break;
        }
        if k.eq_ignore_ascii_case("content-length")
            && let Ok(n) = v.trim().parse::<u64>()
        {
            framing = Framing::Length(n);
        }
    }
    Ok((status, framing, buf[end..].to_vec()))
}

/// Reads a short body as far as the timers allow. What it got is what counts.
fn read_small_body(
    s: &mut TcpStream,
    timers: &Timers,
    mut framing: Framing,
    first: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();
    let mut tmp = [0u8; 4096];
    let mut input = first.to_vec();
    loop {
        match framing.feed(&input, &mut body) {
            Ok(false) if body.len() < MAX_SMALL => {}
            _ => return body,
        }
        match read_some(s, timers, &mut tmp) {
            Ok(n) if n > 0 => input = tmp[..n].to_vec(),
            _ => return body,
        }
    }
}

// ---------- errors ----------

fn error_text(body: &[u8]) -> String {
    let raw = String::from_utf8_lossy(body);
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| raw.into_owned())
        .to_lowercase()
}

fn says_context(t: &str) -> bool {
    t.contains("context")
        && (t.contains("exceed") || t.contains("too long") || t.contains("too large"))
}

fn says_schema(t: &str) -> bool {
    t.contains("format") || t.contains("schema") || t.contains("grammar")
}

/// The code for an HTTP error. The text is read, never kept.
fn classify_status(status: u16, body: &[u8]) -> Code {
    let t = error_text(body);
    if says_context(&t) {
        return Code::PromptCut;
    }
    match status {
        400 if says_schema(&t) => Code::SchemaRejected,
        401 | 403 | 404 => Code::Access,
        500 => Code::OutOfMemoryOrCrash,
        502 | 504 => Code::ProxyCut,
        503 => Code::ServerBusy,
        _ => Code::HttpOther,
    }
}

/// The code for an `error` line inside a 200 stream.
fn classify_stream_error(text: &str) -> Code {
    let t = text.to_lowercase();
    if says_context(&t) {
        Code::PromptCut
    } else if ["memory", "runner", "terminated", "killed", "crash"]
        .iter()
        .any(|w| t.contains(w))
    {
        Code::OutOfMemoryOrCrash
    } else if says_schema(&t) {
        Code::SchemaRejected
    } else {
        Code::StreamErrorOther
    }
}

// ---------- the call ----------

fn connect(url: &OllamaUrl, limit: Duration) -> Result<TcpStream, Code> {
    let start = Instant::now();
    let addrs = url
        .authority
        .to_socket_addrs()
        .map_err(|_| Code::ConnectFailed)?;
    let mut failure = Code::ConnectFailed;
    for addr in addrs {
        let left = limit.saturating_sub(start.elapsed());
        if left.is_zero() {
            return Err(Code::TimeConnect);
        }
        match TcpStream::connect_timeout(&addr, left) {
            Ok(s) => return Ok(s),
            Err(e) if e.kind() == ErrorKind::TimedOut => failure = Code::TimeConnect,
            Err(_) => {}
        }
    }
    Err(failure)
}

fn send(s: &mut TcpStream, url: &OllamaUrl, body: &[u8], limit: Duration) -> Result<(), Code> {
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: stage0-runner\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        url.path("api/chat"),
        url.authority,
        body.len()
    );
    let to_code = |e: std::io::Error| match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => Code::TimeConnect,
        _ => Code::ConnectFailed,
    };
    s.set_write_timeout(Some(limit)).map_err(to_code)?;
    s.write_all(head.as_bytes()).map_err(to_code)?;
    s.write_all(body).map_err(to_code)?;
    s.flush().map_err(to_code)
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// One stream line. `Some` ends the stream: `None` inside is a clean finish.
fn handle_line(line: &[u8], o: &mut ChatOutcome, timers: &mut Timers) -> Option<Option<Code>> {
    if line.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let Ok(v) = serde_json::from_slice::<Value>(line) else {
        return Some(Some(Code::StreamBadLine));
    };
    if let Some(e) = v.get("error") {
        return Some(Some(classify_stream_error(e.as_str().unwrap_or(""))));
    }
    if let Some(t) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
    {
        o.reply.push_str(t);
        let now = Instant::now();
        timers.last_token = Some(now);
        o.first_token_ms
            .get_or_insert_with(|| ms(now - timers.start));
    }
    if v.get("done").and_then(Value::as_bool) == Some(true) {
        let n = |k: &str| v.get(k).and_then(Value::as_u64);
        o.counters = Counters {
            prompt_eval_count: n("prompt_eval_count"),
            eval_count: n("eval_count"),
            total_duration: n("total_duration"),
            load_duration: n("load_duration"),
            prompt_eval_duration: n("prompt_eval_duration"),
            eval_duration: n("eval_duration"),
            done_reason: v
                .get("done_reason")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        return Some(None);
    }
    None
}

/// Reads the stream up to its end. `None`: a done line arrived.
fn read_stream(s: &mut TcpStream, o: &mut ChatOutcome, timers: &mut Timers) -> Option<Code> {
    let (status, mut framing, first) = match read_head(s, timers) {
        Ok(h) => h,
        Err(Stop::Timer(c)) => return Some(c),
        Err(Stop::Broken) => return Some(Code::ProxyCut),
    };
    o.http_status = Some(status);
    if status != 200 {
        let body = read_small_body(s, timers, framing, &first);
        return Some(classify_status(status, &body));
    }
    let mut pending: Vec<u8> = Vec::new();
    let mut tmp = vec![0u8; 16 * 1024];
    let mut input = first;
    loop {
        let finished = match framing.feed(&input, &mut pending) {
            Ok(f) => f,
            Err(BadFrame) => return Some(Code::ProxyCut),
        };
        while let Some(at) = pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = pending.drain(..=at).collect();
            if let Some(end) = handle_line(&line, o, timers) {
                return end;
            }
        }
        if finished {
            break;
        }
        match read_some(s, timers, &mut tmp) {
            Ok(0) => break,
            Ok(n) => input = tmp[..n].to_vec(),
            Err(Stop::Timer(c)) => return Some(c),
            Err(Stop::Broken) => return Some(Code::ProxyCut),
        }
    }
    // The stream ended. A last line may have had no newline.
    if !pending.is_empty()
        && let Some(end) = handle_line(&pending, o, timers)
    {
        return end;
    }
    Some(Code::ProxyCut)
}

/// One chat call. Never panics on server input, and always closes the socket.
pub fn chat(url: &OllamaUrl, body: &[u8], limits: &Limits) -> ChatOutcome {
    let start = Instant::now();
    let mut o = ChatOutcome::default();
    let mut stream = match connect(url, limits.connect) {
        Ok(s) => s,
        Err(c) => {
            o.failure = Some(c);
            o.wall_ms = ms(start.elapsed());
            return o;
        }
    };
    let _ = stream.set_nodelay(true);
    if let Err(c) = send(&mut stream, url, body, limits.connect) {
        o.failure = Some(c);
        o.wall_ms = ms(start.elapsed());
        let _ = stream.shutdown(Shutdown::Both);
        return o;
    }
    let mut timers = Timers {
        start,
        sent: Instant::now(),
        last_token: None,
        first_token: limits.first_token,
        gap: limits.gap,
        whole: limits.whole,
    };
    o.failure = read_stream(&mut stream, &mut o, &mut timers);
    // Closing is what stops Ollama writing, when a timer fired.
    let _ = stream.shutdown(Shutdown::Both);
    o.wall_ms = ms(start.elapsed());
    o
}

/// Asks Ollama to unload the model now.
pub fn unload(url: &OllamaUrl, limits: &Limits) -> Result<(), Code> {
    let fail = |_| Code::UnloadFailed;
    let body =
        json!({"model": MODEL, "stream": false, "keep_alive": 0, "messages": []}).to_string();
    let mut s = connect(url, limits.connect).map_err(fail)?;
    send(&mut s, url, body.as_bytes(), limits.connect).map_err(fail)?;
    let timers = Timers {
        start: Instant::now(),
        sent: Instant::now(),
        last_token: None,
        first_token: UNLOAD_LIMIT,
        gap: UNLOAD_LIMIT,
        whole: UNLOAD_LIMIT,
    };
    let result = match read_head(&mut s, &timers) {
        Ok((200, _, _)) => Ok(()),
        _ => Err(Code::UnloadFailed),
    };
    let _ = s.shutdown(Shutdown::Both);
    result
}
