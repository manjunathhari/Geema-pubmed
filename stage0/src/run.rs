//! The program's steps, in order. Logs carry labels, counts, codes and times:
//! never a question, an abstract or a reply.

use crate::checks::{CheckCounts, estimate_tokens, evaluate};
use crate::codes::Code;
use crate::copied::answer::{Span, WrittenAnswer};
use crate::copied::writer::{STUDY_CAPS, StudyInput};
use crate::job::{ConfigError, Job, parse_jobs, prepare_out_dir};
use crate::ollama::{
    ChatOutcome, Counters, Limits, MODEL, NUM_CTX, NUM_PREDICT, OllamaUrl, chat, chat_request,
    unload,
};
use crate::prompt::build_prompt;
use crate::pubmed::{EUTILS, Fetched, NcbiClient};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Config {
    pub ollama: OllamaUrl,
    pub out: PathBuf,
    /// NCBI's address, with its trailing slash.
    pub ncbi_base: String,
}

impl ConfigError {
    /// The fixed word for logs.
    pub fn word(self) -> &'static str {
        match self {
            ConfigError::MissingEnv => "MISSING_ENV",
            ConfigError::BadOllamaUrl => "BAD_OLLAMA_URL",
            ConfigError::JobUnreadable => "JOB_UNREADABLE",
            ConfigError::BadJob => "BAD_JOB",
            ConfigError::OutInsideRepo => "OUT_INSIDE_REPO",
            ConfigError::OutUnusable => "OUT_UNUSABLE",
        }
    }
}

/// Reads settings and the job. `get` gives an environment value.
pub fn load(get: impl Fn(&str) -> Option<String>) -> Result<(Config, Vec<Job>), ConfigError> {
    let need = |k: &str| {
        get(k)
            .filter(|v| !v.is_empty())
            .ok_or(ConfigError::MissingEnv)
    };
    let job_path = need("STAGE0_JOB")?;
    let url = need("OLLAMA_URL")?;
    let out = need("STAGE0_OUT")?;
    let ollama = OllamaUrl::parse(&url).map_err(|_| ConfigError::BadOllamaUrl)?;
    let text = std::fs::read(&job_path).map_err(|_| ConfigError::JobUnreadable)?;
    let jobs = parse_jobs(&text)?;
    let out = prepare_out_dir(Path::new(&out))?;
    let ncbi_base = get("STAGE0_NCBI_URL")
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| EUTILS.to_string());
    Ok((
        Config {
            ollama,
            out,
            ncbi_base,
        },
        jobs,
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub model: String,
    pub num_ctx: u64,
    pub num_predict: u64,
    pub seed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputInfo {
    pub studies: usize,
    pub missing_abstracts: Vec<String>,
    pub missing_records: Vec<String>,
    pub abstracts_cut: usize,
    pub prompt_bytes: usize,
    pub prompt_tokens_estimate: u64,
    /// Whether the job gave a search log, and how many entries. Never the terms.
    pub searches_present: bool,
    pub searches_given: usize,
}

/// What is saved for one run. It holds no question, abstract or search terms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    pub label: String,
    /// The commit this program was built from, if the build said. See the Dockerfile.
    pub build_commit: Option<String>,
    pub started_at: String,
    pub finished_at: String,
    pub wall_ms: u64,
    pub first_token_ms: Option<u64>,
    pub http_status: Option<u16>,
    pub settings: Settings,
    pub input: InputInfo,
    pub counters: Counters,
    pub codes: Vec<Code>,
    pub checks: Option<CheckCounts>,
    /// The answer after the Quick checks, as Incisive stores it. `None`: not valid.
    pub cleaned_answer: Option<WrittenAnswer>,
    /// Numbers no source holds, as places in `cleaned_answer`.
    pub untraced: Vec<Span>,
    pub raw_reply: String,
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn say(log: &mut dyn Write, line: &str) {
    let _ = writeln!(log, "stage0 {} {line}", now());
    let _ = log.flush();
}

fn code_list(codes: &[Code]) -> String {
    let words: Vec<String> = codes.iter().map(|c| c.as_str()).collect();
    if words.is_empty() {
        "none".into()
    } else {
        words.join(",")
    }
}

fn opt(v: Option<u64>) -> String {
    v.map_or("na".into(), |n| n.to_string())
}

/// What the job itself says about the input. Counts only, never search terms.
fn input_given(job: &Job) -> InputInfo {
    InputInfo {
        searches_present: job.searches.is_some(),
        searches_given: job.searches.as_ref().map_or(0, Vec::len),
        ..InputInfo::default()
    }
}

fn input_info(f: &Fetched, job: &Job) -> InputInfo {
    InputInfo {
        studies: f.studies.len(),
        missing_abstracts: f.missing_abstracts.clone(),
        missing_records: f.missing_records.clone(),
        abstracts_cut: f
            .studies
            .iter()
            .filter(|s: &&StudyInput| {
                s.abstract_text
                    .as_ref()
                    .is_some_and(|a| a.trim().len() > STUDY_CAPS.abstract_text)
            })
            .count(),
        ..input_given(job)
    }
}

/// Saves one result. An earlier result for the label is kept under another name.
fn save(out: &Path, r: &RunResult) -> std::io::Result<()> {
    let path = out.join(format!("{}.json", r.label));
    if path.exists() {
        let stamp = Utc::now().format("%Y%m%dT%H%M%S%3f");
        std::fs::rename(&path, out.join(format!("{}.prev-{stamp}.json", r.label)))?;
    }
    let tmp = out.join(format!("{}.partial", r.label));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(&serde_json::to_vec(r)?)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

fn run_one(
    cfg: &Config,
    job: &Job,
    ncbi: &mut NcbiClient,
    limits: &Limits,
    log: &mut dyn Write,
) -> RunResult {
    let started_at = now();
    let l = &job.label;
    let mut result = RunResult {
        label: l.clone(),
        build_commit: option_env!("STAGE0_COMMIT")
            .filter(|c| !c.is_empty())
            .map(str::to_string),
        started_at,
        finished_at: String::new(),
        wall_ms: 0,
        first_token_ms: None,
        http_status: None,
        settings: Settings {
            model: MODEL.into(),
            num_ctx: NUM_CTX,
            num_predict: NUM_PREDICT,
            seed: job.seed,
        },
        input: input_given(job),
        counters: Counters::default(),
        // No stored run holds the researcher's reasons or notes.
        codes: vec![Code::ThinInput],
        checks: None,
        cleaned_answer: None,
        untraced: Vec::new(),
        raw_reply: String::new(),
    };
    say(
        log,
        &format!("label={l} step=fetch pmids={}", job.pmids.len()),
    );
    let Ok(fetched) = ncbi.fetch(&job.pmids) else {
        result.codes.push(Code::NcbiFailed);
        result.finished_at = now();
        return result;
    };
    result.input = input_info(&fetched, job);
    say(
        log,
        &format!(
            "label={l} step=fetched studies={} missing_abstracts={} missing_records={} abstracts_cut={}",
            result.input.studies,
            result.input.missing_abstracts.len(),
            result.input.missing_records.len(),
            result.input.abstracts_cut
        ),
    );
    if result.input.missing_abstracts.len() == result.input.studies {
        // Nothing to write from, so don't hold a shared server for it.
        result.codes.push(Code::NcbiNoAbstracts);
        result.finished_at = now();
        return result;
    }
    let prompt = build_prompt(&job.question, &fetched.studies, job.searches.as_deref());
    result.input.prompt_bytes = prompt.len();
    result.input.prompt_tokens_estimate = estimate_tokens(prompt.len());
    say(
        log,
        &format!(
            "label={l} step=chat prompt_bytes={} prompt_tokens_estimate={}",
            result.input.prompt_bytes, result.input.prompt_tokens_estimate
        ),
    );
    let o: ChatOutcome = chat(&cfg.ollama, &chat_request(&prompt, job.seed), limits);
    let (codes, checked) = evaluate(&o, prompt.len(), &fetched.studies);
    result.codes.extend(codes);
    if let Some(c) = checked {
        result.checks = Some(c.counts);
        result.cleaned_answer = Some(c.answer);
        result.untraced = c.untraced;
    }
    result.wall_ms = o.wall_ms;
    result.first_token_ms = o.first_token_ms;
    result.http_status = o.http_status;
    result.counters = o.counters;
    result.raw_reply = o.reply;
    result.finished_at = now();
    result
}

fn log_result(r: &RunResult, log: &mut dyn Write) {
    let k = &r.counters;
    let c = r.checks.unwrap_or_default();
    say(
        log,
        &format!(
            "label={} step=done codes={} wall_ms={} first_token_ms={} http={} prompt_tokens={} output_tokens={} reply_bytes={} valid={} outcomes={} studies={} numbers={} removed_citations={} dropped_numbers={} dropped_studies={} numbers_checked={} numbers_untraced={} labels_defaulted={}",
            r.label,
            code_list(&r.codes),
            r.wall_ms,
            opt(r.first_token_ms),
            opt(r.http_status.map(u64::from)),
            opt(k.prompt_eval_count),
            opt(k.eval_count),
            r.raw_reply.len(),
            r.checks.is_some(),
            c.outcomes,
            c.studies,
            c.numbers,
            c.removed_citations,
            c.dropped_numbers,
            c.dropped_studies,
            c.numbers_checked,
            c.numbers_untraced,
            c.labels_defaulted
        ),
    );
}

/// How the whole job ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    pub runs: usize,
    pub saves_failed: usize,
}

/// Runs every job in order, then one unload.
pub fn run_all(cfg: &Config, jobs: &[Job], limits: &Limits, log: &mut dyn Write) -> Summary {
    let mut ncbi = NcbiClient::new(&cfg.ncbi_base);
    let mut saves_failed = 0;
    say(log, &format!("step=start runs={}", jobs.len()));
    for job in jobs {
        let r = run_one(cfg, job, &mut ncbi, limits, log);
        log_result(&r, log);
        if save(&cfg.out, &r).is_err() {
            saves_failed += 1;
            say(
                log,
                &format!("label={} step=save result=SAVE_FAILED", r.label),
            );
        }
    }
    let unloaded = match unload(&cfg.ollama, limits) {
        Ok(()) => "ok".to_string(),
        Err(c) => c.as_str(),
    };
    say(
        log,
        &format!(
            "step=end runs={} saves_failed={saves_failed} unload={unloaded}",
            jobs.len()
        ),
    );
    Summary {
        runs: jobs.len(),
        saves_failed,
    }
}
