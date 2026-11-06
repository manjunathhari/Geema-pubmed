//! The Ollama call, against a mock server.

mod support;

use stage0_runner::codes::Code;
use stage0_runner::ollama::{ChatOutcome, Limits, OllamaUrl, chat, chat_request, unload};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::*;

const SHORT: Limits = Limits {
    connect: Duration::from_millis(500),
    first_token: Duration::from_millis(400),
    gap: Duration::from_millis(400),
    whole: Duration::from_secs(20),
};

fn call(m: &Mock, limits: &Limits) -> ChatOutcome {
    let url = OllamaUrl::parse(&m.url).unwrap();
    chat(&url, &chat_request("made-up prompt", 11), limits)
}

#[test]
fn the_request_carries_every_setting() {
    let m = serve(|_, s| stream_reply(s, "{}", 10, 2, "stop"));
    let o = call(&m, &SHORT);
    assert_eq!(o.failure, None);
    let r = &m.requests()[0];
    assert_eq!(r.method, "POST");
    assert_eq!(r.target, "/api/chat");
    let b: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(b["model"], "gemma3:4b");
    assert_eq!(b["stream"], true);
    assert_eq!(b["truncate"], false);
    assert_eq!(b["shift"], false);
    assert_eq!(b["keep_alive"], "30m");
    assert_eq!(b["format"], stage0_runner::prompt::quick_schema());
    let o = &b["options"];
    assert_eq!(o["num_ctx"], 40960);
    assert_eq!(o["num_predict"], 12288);
    assert_eq!(o["temperature"], 0.2);
    assert_eq!(o["seed"], 11);
    assert_eq!(o["top_k"], 64);
    assert_eq!(o["top_p"], 0.95);
    assert_eq!(o["repeat_penalty"], 1.0);
    assert_eq!(o.as_object().unwrap().len(), 7);
    let msgs = b["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1, "one user message");
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"], "made-up prompt");
    assert!(
        r.body.find("\"applicability\"") < r.body.find("\"outcomes\""),
        "the schema keeps its field order on the wire"
    );
}

#[test]
fn the_real_timers_are_the_ones_in_the_brief() {
    assert_eq!(Limits::REAL.connect, Duration::from_secs(30));
    assert_eq!(Limits::REAL.first_token, Duration::from_secs(25 * 60));
    assert_eq!(Limits::REAL.gap, Duration::from_secs(5 * 60));
    assert_eq!(Limits::REAL.whole, Duration::from_secs(120 * 60));
}

#[test]
fn a_stream_is_joined_across_awkward_chunks_and_counters_are_kept() {
    let m = serve(|_, s| {
        start_stream(s);
        // A line, and a two-byte character, split across chunks.
        let t = token("caf\u{e9} ").into_bytes();
        let at = t.iter().position(|b| *b == 0xc3).unwrap() + 1;
        chunk(s, &t[..at]);
        chunk(s, &t[at..]);
        // Two lines in one chunk.
        chunk(s, format!("{}{}", token("au"), token(" lait")).as_bytes());
        chunk(s, done_line(1234, 56, "stop").as_bytes());
        end_stream(s);
    });
    let o = call(&m, &SHORT);
    assert_eq!(o.failure, None);
    assert_eq!(o.reply, "caf\u{e9} au lait");
    assert_eq!(o.counters.prompt_eval_count, Some(1234));
    assert_eq!(o.counters.eval_count, Some(56));
    assert_eq!(o.counters.total_duration, Some(9_000_000_000));
    assert_eq!(o.counters.load_duration, Some(1_000_000_000));
    assert_eq!(o.counters.prompt_eval_duration, Some(5_000_000_000));
    assert_eq!(o.counters.eval_duration, Some(3_000_000_000));
    assert_eq!(o.counters.done_reason.as_deref(), Some("stop"));
    assert_eq!(o.http_status, Some(200));
    assert!(o.first_token_ms.is_some());
}

#[test]
fn a_stream_without_chunking_also_works() {
    let m = serve(|_, s| {
        let body = format!("{}{}", token("x"), done_line(1, 1, "stop"));
        respond(s, 200, &body);
    });
    let o = call(&m, &SHORT);
    assert_eq!(o.failure, None);
    assert_eq!(o.reply, "x");
}

#[test]
fn a_stream_that_ends_with_no_done_line_is_a_proxy_cut_and_keeps_the_partial_reply() {
    let m = serve(|_, s| {
        start_stream(s);
        chunk(s, token("partial").as_bytes());
        end_stream(s);
    });
    let o = call(&m, &SHORT);
    assert_eq!(o.failure, Some(Code::ProxyCut));
    assert_eq!(o.reply, "partial");
    assert_eq!(o.counters.done_reason, None);
}

#[test]
fn a_connection_dropped_mid_stream_is_a_proxy_cut() {
    let m = serve(|_, s| {
        start_stream(s);
        chunk(s, token("partial").as_bytes());
        let _ = s.shutdown(std::net::Shutdown::Both);
    });
    assert_eq!(call(&m, &SHORT).failure, Some(Code::ProxyCut));
}

#[test]
fn a_bad_stream_line_has_its_own_code() {
    let m = serve(|_, s| {
        start_stream(s);
        chunk(s, b"not json at all\n");
        end_stream(s);
    });
    assert_eq!(call(&m, &SHORT).failure, Some(Code::StreamBadLine));
}

#[test]
fn first_token_timer_fires_when_nothing_arrives() {
    let m = serve(|_, _| {});
    let t = Instant::now();
    let o = call(&m, &SHORT);
    assert_eq!(o.failure, Some(Code::TimeFirstToken));
    assert!(t.elapsed() >= SHORT.first_token);
    assert!(t.elapsed() < Duration::from_secs(5));
    assert!(
        m.saw_closed(1),
        "the connection is closed, so Ollama stops writing"
    );
}

#[test]
fn first_token_timer_also_covers_headers_with_no_token() {
    let m = serve(|_, s| {
        start_stream(s);
        // Empty content is not a token.
        chunk(s, token("").as_bytes());
    });
    let o = call(&m, &SHORT);
    assert_eq!(o.failure, Some(Code::TimeFirstToken));
    assert_eq!(o.http_status, Some(200));
    assert!(m.saw_closed(1));
}

#[test]
fn gap_timer_fires_after_a_token_and_keeps_the_partial_reply() {
    let m = serve(|_, s| {
        start_stream(s);
        chunk(s, token("one").as_bytes());
    });
    let o = call(&m, &SHORT);
    assert_eq!(o.failure, Some(Code::TimeGap));
    assert_eq!(o.reply, "one");
    assert!(m.saw_closed(1));
}

#[test]
fn only_real_tokens_reset_the_gap_timer() {
    let m = serve(|_, s| {
        start_stream(s);
        chunk(s, token("one").as_bytes());
        for _ in 0..100 {
            chunk(s, token("").as_bytes());
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let t = Instant::now();
    assert_eq!(call(&m, &SHORT).failure, Some(Code::TimeGap));
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

#[test]
fn whole_call_timer_fires_even_while_tokens_flow() {
    let m = serve(|_, s| {
        start_stream(s);
        for _ in 0..200 {
            chunk(s, token("x").as_bytes());
            std::thread::sleep(Duration::from_millis(30));
        }
    });
    let limits = Limits {
        whole: Duration::from_millis(500),
        ..SHORT
    };
    let o = call(&m, &limits);
    assert_eq!(o.failure, Some(Code::TimeWhole));
    assert!(!o.reply.is_empty());
    assert!(m.saw_closed(1));
}

/// A listener whose accept queue is full drops new connection attempts, so a
/// connect hangs without any outside network.
#[test]
fn connect_timer_fires_when_the_connection_hangs() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr: SocketAddr = l.local_addr().unwrap();
    let mut held: Vec<TcpStream> = Vec::new();
    let full =
        (0..10_000).any(
            |_| match TcpStream::connect_timeout(&addr, Duration::from_millis(150)) {
                Ok(s) => {
                    held.push(s);
                    false
                }
                Err(_) => true,
            },
        );
    assert!(full, "the accept queue never filled");
    let url = OllamaUrl::parse(&format!("http://{addr}")).unwrap();
    let t = Instant::now();
    let o = chat(&url, &chat_request("p", 11), &SHORT);
    assert_eq!(o.failure, Some(Code::TimeConnect));
    assert!(t.elapsed() >= SHORT.connect);
    assert_eq!(o.http_status, None);
}

#[test]
fn a_refused_connection_has_its_own_code() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap();
    drop(l);
    let url = OllamaUrl::parse(&format!("http://{addr}")).unwrap();
    assert_eq!(
        chat(&url, &chat_request("p", 11), &SHORT).failure,
        Some(Code::ConnectFailed)
    );
}

#[test]
fn a_context_error_is_a_prompt_cut_whatever_the_status() {
    for status in [400, 500] {
        let m = serve(move |_, s| {
            respond(
                s,
                status,
                r#"{"error":"the input length exceeds the available context size"}"#,
            )
        });
        let o = call(&m, &SHORT);
        assert_eq!(o.failure, Some(Code::PromptCut), "{status}");
        assert_eq!(o.http_status, Some(status));
    }
}

#[test]
fn a_context_error_inside_the_stream_is_a_prompt_cut() {
    let m = serve(|_, s| {
        start_stream(s);
        chunk(
            s,
            b"{\"error\":\"prompt exceeds the available context size\"}\n",
        );
        end_stream(s);
    });
    assert_eq!(call(&m, &SHORT).failure, Some(Code::PromptCut));
}

#[test]
fn http_errors_map_to_fixed_codes() {
    let cases = [
        (
            400,
            r#"{"error":"invalid JSON schema in format"}"#,
            Code::SchemaRejected,
        ),
        (400, r#"{"error":"something else"}"#, Code::HttpOther),
        (401, "", Code::Access),
        (403, "", Code::Access),
        (404, r#"{"error":"model not found"}"#, Code::Access),
        (
            500,
            r#"{"error":"llama runner process has terminated"}"#,
            Code::OutOfMemoryOrCrash,
        ),
        (502, "", Code::ProxyCut),
        (503, r#"{"error":"server busy"}"#, Code::ServerBusy),
        (504, "", Code::ProxyCut),
        (418, "", Code::HttpOther),
    ];
    for (status, body, want) in cases {
        let m = serve(move |_, s| respond(s, status, body));
        assert_eq!(call(&m, &SHORT).failure, Some(want), "{status} {body}");
    }
}

#[test]
fn a_crash_inside_the_stream_is_named() {
    let m = serve(|_, s| {
        start_stream(s);
        chunk(s, token("x").as_bytes());
        chunk(
            s,
            b"{\"error\":\"llama runner process has terminated: signal: killed\"}\n",
        );
        end_stream(s);
    });
    assert_eq!(call(&m, &SHORT).failure, Some(Code::OutOfMemoryOrCrash));
}

#[test]
fn the_url_may_have_a_port_and_a_prefix_but_must_be_plain_http() {
    let u = OllamaUrl::parse("http://ollama:11434/").unwrap();
    assert_eq!(u.authority(), "ollama:11434");
    assert_eq!(u.path("api/chat"), "/api/chat");
    let u = OllamaUrl::parse("http://ollama").unwrap();
    assert_eq!(u.authority(), "ollama:80");
    let u = OllamaUrl::parse("http://ollama:1/base/").unwrap();
    assert_eq!(u.path("api/chat"), "/base/api/chat");
    assert!(OllamaUrl::parse("https://ollama:1").is_err());
    assert!(OllamaUrl::parse("ollama:1").is_err());
    assert!(OllamaUrl::parse("http://").is_err());
}

#[test]
fn unload_asks_for_keep_alive_zero() {
    let m = serve(|_, s| {
        respond(
            s,
            200,
            r#"{"model":"m","done":true,"done_reason":"unload"}"#,
        );
    });
    let url = OllamaUrl::parse(&m.url).unwrap();
    assert_eq!(unload(&url, &SHORT), Ok(()));
    let r = &m.requests()[0];
    assert_eq!(r.target, "/api/chat");
    let b: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(b["model"], "gemma3:4b");
    assert_eq!(b["keep_alive"], 0);
    assert_eq!(b["stream"], false);
    assert_eq!(b["messages"], serde_json::json!([]));
}

#[test]
fn a_failed_unload_is_reported() {
    let m = serve(|_, s| respond(s, 500, "{}"));
    let url = OllamaUrl::parse(&m.url).unwrap();
    assert_eq!(unload(&url, &SHORT), Err(Code::UnloadFailed));
}
