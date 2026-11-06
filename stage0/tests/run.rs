//! The whole program against mock servers. All data is made up.

mod support;

use base64::Engine;
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use support::*;

const QUESTION: &str = "QUESTIONZEBRA is it made up?";
const ABSTRACT: &str = "ABSTRACTYAK pain 4.5 vs 5.5.";
const TITLE: &str = "TITLEOKAPI trial";
const REPLY_WORD: &str = "REPLYIBIS";
const NCBI_ERROR: &str = "NCBIERRORMOLE";
const OLLAMA_ERROR: &str = "OLLAMAERRORSHREW";
const SEARCH_TERM: &str = "SEARCHTERMTAPIR";
const MARKERS: [&str; 7] = [
    SEARCH_TERM,
    "QUESTIONZEBRA",
    "ABSTRACTYAK",
    "TITLEOKAPI",
    REPLY_WORD,
    NCBI_ERROR,
    OLLAMA_ERROR,
];

fn temp(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("stage0-run-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn answer() -> String {
    json!({
        "applicability": {"case_mix": "c", "india_cost_context": "i", "training": "t"},
        "at_a_glance": format!("{REPLY_WORD} verdict [11111111]."),
        "bottom_line": {"text": "Pain 4.5 vs 5.5 [11111111].", "certainty": "Low", "certainty_reason": "small"},
        "clinical_practice": "Default choice: either.",
        "conflicts": [],
        "notable_findings": "None [11111111].",
        "outcomes": [{"outcome": "Pain", "rct_evidence": "one", "effect_text": "4.5 vs 5.5 [11111111]",
            "numbers": [{"label": "p", "value": 4.5, "unit": null, "arm": "A", "ci_low": null, "ci_high": null, "p": null, "pmid": "11111111"}],
            "certainty": "Low", "why": "w", "pmids": ["11111111"]}],
        "studies": [{"pmid": "11111111", "label": "L", "design": "RCT", "n": null, "setting": "s", "comparison": "c", "key_results": "k", "appraisal": "a"}],
        "would_change": []
    })
    .to_string()
}

fn record(pmid: &str) -> String {
    format!(
        "<PubmedArticle><MedlineCitation><PMID>{pmid}</PMID><Article><ArticleTitle>{TITLE}</ArticleTitle>\
         <Abstract><AbstractText>{ABSTRACT}</AbstractText></Abstract></Article></MedlineCitation></PubmedArticle>"
    )
}

/// Answers every PMID but 3xxxxxxx (an error) and 4xxxxxxx (no record).
fn ncbi() -> Mock {
    serve(|r, s| {
        if r.target.contains("id=3") {
            return respond(s, 500, NCBI_ERROR);
        }
        let id = r
            .target
            .split("id=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        let records: String = id
            .split(',')
            .filter(|p| !p.starts_with('4'))
            .map(record)
            .collect();
        respond(
            s,
            200,
            &format!("<PubmedArticleSet>{records}</PubmedArticleSet>"),
        );
    })
}

/// Ollama: the reply depends on a word in the question.
fn ollama() -> Mock {
    serve(|r, s| {
        let b: Value = serde_json::from_str(&r.body).unwrap();
        if b["keep_alive"] == 0 {
            return respond(s, 200, r#"{"done":true,"done_reason":"unload"}"#);
        }
        let prompt = b["messages"][0]["content"].as_str().unwrap();
        let tokens = (prompt.len() as f64 / 3.85) as u64;
        if prompt.contains("ASKBADJSON") {
            stream_reply(s, &format!("{{ {REPLY_WORD} not json"), tokens, 10, "stop");
        } else if prompt.contains("ASKCONTEXT") {
            respond(
                s,
                400,
                &format!(r#"{{"error":"{OLLAMA_ERROR} exceeds the available context size"}}"#),
            );
        } else if prompt.contains("ASKLENGTH") {
            stream_reply(s, &format!("{{ {REPLY_WORD} cut"), tokens, 12288, "length");
        } else if prompt.contains("ASKNODONE") {
            start_stream(s);
            chunk(s, token(REPLY_WORD).as_bytes());
            end_stream(s);
        } else {
            stream_reply(s, &answer(), tokens, 500, "stop");
        }
    })
}

fn run(job: &Path, out: &Path, nc: &Mock, ol: &Mock, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_stage0-runner"))
        .args(args)
        .env("STAGE0_JOB", job)
        .env("STAGE0_OUT", out)
        .env("OLLAMA_URL", &ol.url)
        .env("STAGE0_NCBI_URL", format!("{}/", nc.url))
        .output()
        .unwrap()
}

fn all_output(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn assert_no_input_text(log: &str) {
    for m in MARKERS {
        assert!(!log.contains(m), "{m} leaked into: {log}");
    }
}

fn write_job(dir: &Path, jobs: Value) -> PathBuf {
    let p = dir.join("job");
    std::fs::write(&p, jobs.to_string()).unwrap();
    p
}

fn result(out: &Path, label: &str) -> Value {
    serde_json::from_slice(&std::fs::read(out.join(format!("{label}.json"))).unwrap()).unwrap()
}

fn codes(v: &Value) -> Vec<&str> {
    v["codes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect()
}

#[test]
fn a_full_run_sends_the_right_things_and_saves_the_result() {
    let (nc, ol, dir) = (ncbi(), ollama(), temp("full"));
    let out = dir.join("out");
    let job = write_job(
        &dir,
        json!([{"label": "a-1", "question": QUESTION, "pmids": ["11111111"]}]),
    );
    let o = run(&job, &out, &nc, &ol, &[]);
    assert!(o.status.success(), "{}", all_output(&o));
    assert_no_input_text(&all_output(&o));

    // NCBI saw the PMID and nothing else.
    let n = nc.requests();
    assert_eq!(n.len(), 1);
    assert!(n[0].target.contains("id=11111111&"));
    assert!(!n[0].target.contains("QUESTIONZEBRA"));

    // Ollama: one chat, then one unload.
    let c = ol.requests();
    assert_eq!(c.len(), 2);
    let chat: Value = serde_json::from_str(&c[0].body).unwrap();
    let prompt = chat["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains(QUESTION) && prompt.contains(ABSTRACT) && prompt.contains(TITLE));
    assert!(prompt.contains("Researcher's design label: not available"));
    let last: Value = serde_json::from_str(&c[1].body).unwrap();
    assert_eq!(last["keep_alive"], 0);

    // The saved result.
    let r = result(&out, "a-1");
    assert_eq!(codes(&r), ["THIN_INPUT"]);
    assert_eq!(r["raw_reply"].as_str().unwrap(), answer());
    assert_eq!(r["counters"]["eval_count"], 500);
    assert_eq!(r["counters"]["done_reason"], "stop");
    assert_eq!(r["counters"]["prompt_eval_duration"], 5_000_000_000u64);
    assert_eq!(r["checks"]["outcomes"], 1);
    assert_eq!(r["checks"]["numbers_untraced"], 0);
    assert_eq!(r["input"]["studies"], 1);
    assert_eq!(r["input"]["prompt_bytes"], prompt.len());
    assert_eq!(r["settings"]["seed"], 11);
    let raw = std::fs::read_to_string(out.join("a-1.json")).unwrap();
    assert!(
        !raw.contains("QUESTIONZEBRA"),
        "the question isn't stored in the result"
    );
    assert!(!raw.contains("ABSTRACTYAK"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn each_failure_is_labelled_the_run_goes_on_and_logs_stay_clean() {
    let (nc, ol, dir) = (ncbi(), ollama(), temp("mix"));
    let out = dir.join("out");
    let job = write_job(
        &dir,
        json!([
            {"label": "bad", "question": format!("{QUESTION} ASKBADJSON"), "pmids": ["11111111"]},
            {"label": "ctx", "question": format!("{QUESTION} ASKCONTEXT"), "pmids": ["11111111"]},
            {"label": "len", "question": format!("{QUESTION} ASKLENGTH"), "pmids": ["11111111"]},
            {"label": "nod", "question": format!("{QUESTION} ASKNODONE"), "pmids": ["11111111"]},
            {"label": "miss", "question": QUESTION, "pmids": ["11111111", "44444444"]},
            {"label": "ncbi", "question": QUESTION, "pmids": ["33333333"]},
            {"label": "gone", "question": QUESTION, "pmids": ["44444444"]}
        ]),
    );
    let o = run(&job, &out, &nc, &ol, &[]);
    let log = all_output(&o);
    assert!(o.status.success(), "{log}");
    assert_no_input_text(&log);

    assert_eq!(
        codes(&result(&out, "bad")),
        ["THIN_INPUT", "INVALID_JSON_CLEAN_FINISH"]
    );
    assert_eq!(codes(&result(&out, "ctx")), ["THIN_INPUT", "PROMPT_CUT"]);
    assert_eq!(
        codes(&result(&out, "len")),
        ["THIN_INPUT", "NUM_PREDICT_LIMIT"]
    );
    assert_eq!(codes(&result(&out, "nod")), ["THIN_INPUT", "PROXY_CUT"]);
    assert_eq!(
        result(&out, "nod")["raw_reply"],
        REPLY_WORD,
        "the partial reply is kept"
    );
    assert_eq!(codes(&result(&out, "miss")), ["THIN_INPUT"]);
    let m = result(&out, "miss");
    assert_eq!(m["input"]["studies"], 2, "the study stays");
    assert_eq!(m["input"]["missing_abstracts"], json!(["44444444"]));
    assert_eq!(m["input"]["missing_records"], json!(["44444444"]));
    assert_eq!(codes(&result(&out, "ncbi")), ["THIN_INPUT", "NCBI_FAILED"]);
    assert_eq!(
        codes(&result(&out, "gone")),
        ["THIN_INPUT", "NCBI_NO_ABSTRACTS"]
    );
    assert!(log.contains("label=bad") && log.contains("INVALID_JSON_CLEAN_FINISH"));

    // Five chats (no call for the two with no abstracts) and one unload, last.
    let c = ol.requests();
    assert_eq!(c.len(), 6);
    let unloads = c
        .iter()
        .filter(|r| r.body.contains("\"keep_alive\":0"))
        .count();
    assert_eq!(unloads, 1);
    assert!(c[5].body.contains("\"keep_alive\":0"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_search_log_reaches_the_prompt_is_recorded_and_its_terms_are_not_kept() {
    let (nc, ol, dir) = (ncbi(), ollama(), temp("searches"));
    let out = dir.join("out");
    let job = write_job(
        &dir,
        json!([
            {"label": "with", "question": QUESTION, "pmids": ["11111111"],
             "searches": [{"source": "PubMed", "query": SEARCH_TERM, "hits": 327}]},
            {"label": "without", "question": QUESTION, "pmids": ["11111111"]},
            {"label": "failed", "question": QUESTION, "pmids": ["33333333"],
             "searches": [{"source": "PubMed", "query": SEARCH_TERM, "hits": 327}]}
        ]),
    );
    let o = run(&job, &out, &nc, &ol, &[]);
    assert!(o.status.success(), "{}", all_output(&o));
    assert_no_input_text(&all_output(&o));

    let c = ol.requests();
    let body = |i: usize| -> Value { serde_json::from_str(&c[i].body).unwrap() };
    let with = body(0);
    let prompt = with["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.contains(&format!("- PubMed: {SEARCH_TERM} (327 hits)")));
    let without = body(1);
    let prompt = without["messages"][0]["content"].as_str().unwrap();
    assert!(
        prompt.contains("RESEARCHER'S SEARCH LOG (as the databases returned it):\nnot available\n")
    );

    let r = result(&out, "with");
    assert_eq!(r["input"]["searches_present"], true);
    assert_eq!(r["input"]["searches_given"], 1);
    let r = result(&out, "without");
    assert_eq!(r["input"]["searches_present"], false);
    assert_eq!(r["input"]["searches_given"], 0);
    // Recorded even when the fetch fails.
    let r = result(&out, "failed");
    assert_eq!(codes(&r), ["THIN_INPUT", "NCBI_FAILED"]);
    assert_eq!(r["input"]["searches_present"], true);
    for label in ["with", "without", "failed"] {
        let raw = std::fs::read_to_string(out.join(format!("{label}.json"))).unwrap();
        assert!(!raw.contains(SEARCH_TERM), "the terms aren't stored");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn each_job_sends_and_records_its_own_seed() {
    let (nc, ol, dir) = (ncbi(), ollama(), temp("seed"));
    let out = dir.join("out");
    let job = write_job(
        &dir,
        json!([
            {"label": "plain", "question": QUESTION, "pmids": ["11111111"]},
            {"label": "other", "question": QUESTION, "pmids": ["11111111"], "seed": 22}
        ]),
    );
    assert!(run(&job, &out, &nc, &ol, &[]).status.success());
    let c = ol.requests();
    let seed = |i: usize| -> Value {
        let b: Value = serde_json::from_str(&c[i].body).unwrap();
        b["options"]["seed"].clone()
    };
    assert_eq!((seed(0), seed(1)), (json!(11), json!(22)));
    assert_eq!(result(&out, "plain")["settings"]["seed"], 11);
    assert_eq!(result(&out, "other")["settings"]["seed"], 22);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn print_mode_gives_back_the_saved_result() {
    let (nc, ol, dir) = (ncbi(), ollama(), temp("print"));
    let out = dir.join("out");
    let job = write_job(
        &dir,
        json!([{"label": "p-1", "question": QUESTION, "pmids": ["11111111"]}]),
    );
    assert!(run(&job, &out, &nc, &ol, &[]).status.success());
    let before = ol.requests().len();

    let o = run(&job, &out, &nc, &ol, &["print", "p-1"]);
    assert!(o.status.success());
    let log = all_output(&o);
    assert_no_input_text(&log);
    assert_eq!(ol.requests().len(), before, "print mode calls nothing");
    let b64: String = log
        .lines()
        .filter(|l| l.starts_with("S0P p-1 "))
        .map(|l| l.split(' ').nth(3).unwrap())
        .collect();
    let gz = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    let mut text = Vec::new();
    flate2::read::GzDecoder::new(&gz[..])
        .read_to_end(&mut text)
        .unwrap();
    assert_eq!(text, std::fs::read(out.join("p-1.json")).unwrap());
    assert!(log.lines().last().unwrap().starts_with("S0END p-1 lines="));

    let o = run(&job, &out, &nc, &ol, &["print", "nothing"]);
    assert!(!o.status.success());
    let o = run(&job, &out, &nc, &ol, &["print", "../up"]);
    assert!(!o.status.success());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_repeat_run_keeps_the_earlier_result() {
    let (nc, ol, dir) = (ncbi(), ollama(), temp("repeat"));
    let out = dir.join("out");
    let job = write_job(
        &dir,
        json!([{"label": "r", "question": QUESTION, "pmids": ["11111111"]}]),
    );
    assert!(run(&job, &out, &nc, &ol, &[]).status.success());
    assert!(run(&job, &out, &nc, &ol, &[]).status.success());
    let names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"r.json".to_string()));
    assert!(names.iter().any(|n| n.starts_with("r.prev-")));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn bad_settings_stop_before_any_call_and_log_no_text() {
    let (nc, ol, dir) = (ncbi(), ollama(), temp("bad"));
    let out = dir.join("out");

    // A broken job file: the parser's message must not reach the log.
    let job = dir.join("job");
    std::fs::write(
        &job,
        format!("[{{\"label\":\"x\",\"question\":\"{QUESTION}\",\"QUESTIONZEBRA\":1}}]"),
    )
    .unwrap();
    let o = run(&job, &out, &nc, &ol, &[]);
    assert_eq!(o.status.code(), Some(2));
    assert_no_input_text(&all_output(&o));
    assert!(all_output(&o).contains("BAD_JOB"));

    // No job file.
    let o = run(&dir.join("absent"), &out, &nc, &ol, &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(all_output(&o).contains("JOB_UNREADABLE"));

    // Output folder inside a repo.
    let repo = dir.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let good = write_job(
        &dir,
        json!([{"label": "x", "question": QUESTION, "pmids": ["11111111"]}]),
    );
    let o = run(&good, &repo.join("out"), &nc, &ol, &[]);
    assert_eq!(o.status.code(), Some(2));
    assert!(all_output(&o).contains("OUT_INSIDE_REPO"));

    // Missing settings and a wrong command.
    let o = Command::new(env!("CARGO_BIN_EXE_stage0-runner"))
        .env_remove("STAGE0_JOB")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(all_output(&o).contains("MISSING_ENV"));
    let o = run(&good, &out, &nc, &ol, &["sideways"]);
    assert_eq!(o.status.code(), Some(2));

    // An https Ollama address is refused.
    let o = Command::new(env!("CARGO_BIN_EXE_stage0-runner"))
        .env("STAGE0_JOB", &good)
        .env("STAGE0_OUT", &out)
        .env("OLLAMA_URL", "https://example.invalid")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(all_output(&o).contains("BAD_OLLAMA_URL"));

    assert!(nc.requests().is_empty() && ol.requests().is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

/// A timer during a real run: the partial reply is saved and the unload still runs.
#[test]
fn a_timer_in_a_run_saves_the_partial_reply_and_still_unloads() {
    use stage0_runner::job::Job;
    use stage0_runner::ollama::{Limits, OllamaUrl};
    use stage0_runner::run::{Config, run_all};
    use std::time::Duration;

    let nc = ncbi();
    let ol = serve(|r, s| {
        if r.body.contains("\"keep_alive\":0") {
            return respond(s, 200, r#"{"done":true}"#);
        }
        start_stream(s);
        chunk(s, token(REPLY_WORD).as_bytes());
    });
    let dir = temp("timer");
    let cfg = Config {
        ollama: OllamaUrl::parse(&ol.url).unwrap(),
        out: dir.clone(),
        ncbi_base: format!("{}/", nc.url),
    };
    let jobs = vec![Job {
        label: "t".into(),
        question: QUESTION.into(),
        pmids: vec!["11111111".into()],
        searches: None,
        seed: 11,
    }];
    let limits = Limits {
        gap: Duration::from_millis(200),
        ..Limits::REAL
    };
    let mut log = Vec::new();
    run_all(&cfg, &jobs, &limits, &mut log);
    let log = String::from_utf8(log).unwrap();
    assert_no_input_text(&log);
    assert!(log.contains("TIME_GAP"));
    let r = result(&dir, "t");
    assert_eq!(codes(&r), ["THIN_INPUT", "TIME_GAP"]);
    assert_eq!(r["raw_reply"], REPLY_WORD);
    assert!(ol.saw_closed(1));
    assert!(
        ol.requests()
            .last()
            .unwrap()
            .body
            .contains("\"keep_alive\":0")
    );
    std::fs::remove_dir_all(dir).unwrap();
}
