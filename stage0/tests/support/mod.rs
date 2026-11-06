//! A small mock HTTP server for Ollama and NCBI. All test data is made up.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One request the mock received.
#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub target: String,
    pub body: String,
    pub at: Instant,
}

pub struct Mock {
    pub url: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    closed: Arc<AtomicUsize>,
}

impl Mock {
    /// Requests so far.
    pub fn requests(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// Waits until the client has closed `n` connections; false on timeout.
    pub fn saw_closed(&self, n: usize) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if self.closed.load(Ordering::SeqCst) >= n {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }
}

fn read_request(s: &mut TcpStream) -> Option<Seen> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 8192];
    let head_end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        let n = s.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let len = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    while buf.len() < head_end + len {
        let n = s.read(&mut tmp).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&tmp[..n]);
    }
    let mut first = head.lines().next()?.split(' ');
    Some(Seen {
        method: first.next()?.to_string(),
        target: first.next()?.to_string(),
        body: String::from_utf8_lossy(&buf[head_end..head_end + len]).to_string(),
        at: Instant::now(),
    })
}

/// Starts a mock. `handler` answers each request on its own thread. When it
/// returns, the mock waits for the client to close, then counts the close.
pub fn serve(handler: impl Fn(&Seen, &mut TcpStream) + Send + Sync + 'static) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicUsize::new(0));
    let handler = Arc::new(handler);
    let (seen2, closed2) = (seen.clone(), closed.clone());
    std::thread::spawn(move || {
        for s in listener.incoming() {
            let Ok(mut s) = s else { continue };
            let (seen, closed, handler) = (seen2.clone(), closed2.clone(), handler.clone());
            std::thread::spawn(move || {
                if let Some(r) = read_request(&mut s) {
                    seen.lock().unwrap().push(r.clone());
                    handler(&r, &mut s);
                }
                // Blocks until the client shuts the connection.
                let mut sink = [0u8; 256];
                while matches!(s.read(&mut sink), Ok(n) if n > 0) {}
                closed.fetch_add(1, Ordering::SeqCst);
            });
        }
    });
    Mock { url, seen, closed }
}

/// A whole response with a `Content-Length`, then the mock closes its write side.
pub fn respond(s: &mut TcpStream, status: u16, body: &str) {
    let head = format!(
        "HTTP/1.1 {status} Mock\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body.as_bytes());
    let _ = s.shutdown(std::net::Shutdown::Write);
}

/// Response headers for a chunked NDJSON stream.
pub fn start_stream(s: &mut TcpStream) {
    let _ = s.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\n\r\n",
    );
    let _ = s.flush();
}

/// One chunk.
pub fn chunk(s: &mut TcpStream, bytes: &[u8]) {
    let _ = s.write_all(format!("{:x}\r\n", bytes.len()).as_bytes());
    let _ = s.write_all(bytes);
    let _ = s.write_all(b"\r\n");
    let _ = s.flush();
}

/// The last chunk, then the mock closes its write side.
pub fn end_stream(s: &mut TcpStream) {
    let _ = s.write_all(b"0\r\n\r\n");
    let _ = s.shutdown(std::net::Shutdown::Write);
}

/// A token line of Ollama's stream.
pub fn token(text: &str) -> String {
    let m = serde_json::json!({"model": "m", "message": {"role": "assistant", "content": text}, "done": false});
    format!("{m}\n")
}

/// A done line with counters.
pub fn done_line(prompt: u64, eval: u64, reason: &str) -> String {
    let m = serde_json::json!({
        "model": "m", "message": {"role": "assistant", "content": ""}, "done": true,
        "done_reason": reason, "total_duration": 9_000_000_000u64, "load_duration": 1_000_000_000u64,
        "prompt_eval_count": prompt, "prompt_eval_duration": 5_000_000_000u64,
        "eval_count": eval, "eval_duration": 3_000_000_000u64
    });
    format!("{m}\n")
}

/// A full stream of one reply.
pub fn stream_reply(s: &mut TcpStream, reply: &str, prompt: u64, eval: u64, reason: &str) {
    start_stream(s);
    chunk(s, token(reply).as_bytes());
    chunk(s, done_line(prompt, eval, reason).as_bytes());
    end_stream(s);
}
