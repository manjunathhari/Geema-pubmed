//! Copied from Incisive at 54433fc, `agent/answer_v3.rs`. See `.10x/decisions/sde/stage0-writer-test.md`.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// GRADE-style certainty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Certainty {
    High,
    Moderate,
    Low,
    #[serde(rename = "Very low")]
    VeryLow,
}

impl Certainty {
    /// The four values, as the schema's enum lists them.
    pub const ALL: [&'static str; 4] = ["High", "Moderate", "Low", "Very low"];

    pub fn as_str(self) -> &'static str {
        match self {
            Certainty::High => "High",
            Certainty::Moderate => "Moderate",
            Certainty::Low => "Low",
            Certainty::VeryLow => "Very low",
        }
    }

    /// CSS class for the badge (existing badge colours).
    pub fn class(self) -> &'static str {
        match self {
            Certainty::High => "badge done",
            Certainty::Moderate => "badge mode",
            Certainty::Low => "badge partial",
            Certainty::VeryLow => "badge failed",
        }
    }
}

/// 2–3 sentences and the overall certainty with its reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BottomLine {
    pub text: String,
    pub certainty: Certainty,
    pub certainty_reason: String,
}

/// One number, with its source study.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Number {
    pub label: String,
    pub value: f64,
    pub unit: Option<String>,
    /// The arm or group it belongs to, if any.
    pub arm: Option<String>,
    pub ci_low: Option<f64>,
    pub ci_high: Option<f64>,
    /// As written, e.g. "0.431" or "<0.001".
    pub p: Option<String>,
    pub pmid: String,
}

/// What kind of evidence an outcome row rests on (Stage 4 item 6): a label
/// only; it blocks nothing. Since R-S4a it is derived in code from the
/// cited studies' designs (`writer_v3::evidence_types`); `Unknown` (the
/// default, and answers stored before Stage 4) draws no label.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceType {
    Rct,
    RctMetaAnalysis,
    NetworkMetaAnalysis,
    /// Meta-analyses (what they pooled is in the skeptic's notes).
    MetaAnalysis,
    /// RCTs and meta-analyses together (Stage 4 final review).
    RctsAndMetaAnalyses,
    /// Studies the researcher labelled `other`: not randomised.
    Other,
    Observational,
    Mixed,
    #[default]
    Unknown,
}

/// One row of the outcome table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub outcome: String,
    pub rct_evidence: String,
    pub effect_text: String,
    pub numbers: Vec<Number>,
    pub certainty: Certainty,
    pub why: String,
    pub pmids: Vec<String>,
    /// Stage 4 (additive; old answers: mixed).
    #[serde(default)]
    pub evidence_type: EvidenceType,
}

/// A guideline or consensus statement (Stage 4 item 1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuidelineV3 {
    pub pmid: Option<String>,
    pub body: String,
    pub year: Option<i32>,
    pub statement: String,
    /// Stage 4 review 2: the writer cited a study outside the selection and
    /// the citation check removed it. Not in the writer's schema. With no
    /// PMID either way, the page says "no source among the studies; not
    /// verified".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pmid_removed: bool,
}

/// One key number of the at-a-glance box.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyNumber {
    pub label: String,
    pub value_text: String,
    pub pmid: String,
}

/// The at-a-glance box (Stage 4 item 8): about five lines at the top.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AtAGlance {
    pub verdict: String,
    pub key_numbers: Vec<KeyNumber>,
    pub main_caveat: String,
}

/// What the evidence means for clinical practice (Stage 4 item 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClinicalPractice {
    pub default_choice: String,
    pub who_may_benefit: String,
    pub counselling: String,
    /// A per-case range derived from verified trial numbers, or "not known".
    pub cost_per_case: String,
    /// Stage 4 schema fix (additive): the writer's text that sat under no
    /// label (the flat schema gives one text with labelled lines), drawn as
    /// written.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
}

/// A notable finding outside the outcome tables (Stage 4 item 12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotableFinding {
    pub text: String,
    pub pmids: Vec<String>,
    /// Code-built items only: numbers in `text` (field `text`) that the
    /// skeptic's sources do not have, carried over from its review. Not in
    /// the writer's schema (its items are traced in `AnswerV3.untraced`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<Span>,
}

/// A marked run of stored text (Stage 4 items 5 and 7): `len` bytes from
/// byte `start` of the text at `field` (a path such as
/// `outcomes.2.effect_text`). The text itself is never changed; the page
/// draws a small mark after the run. A span with `len` 0 marks the whole
/// field (a structured number).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub field: String,
    pub start: u32,
    pub len: u32,
}

impl Span {
    pub fn new(field: impl Into<String>, start: usize, end: usize) -> Span {
        Span {
            field: field.into(),
            start: u32::try_from(start).unwrap_or(u32::MAX),
            len: u32::try_from(end.saturating_sub(start)).unwrap_or(u32::MAX),
        }
    }
}

/// One row of the trials table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Study {
    pub pmid: String,
    /// Short name, e.g. "ROGER (2025, Br J Surg)".
    pub label: String,
    pub design: String,
    pub n: Option<i64>,
    pub setting: String,
    pub comparison: String,
    pub key_results: String,
    pub appraisal: String,
}

/// Conflicting evidence and why it differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub text: String,
    pub pmids: Vec<String>,
}

/// Who and where the evidence applies to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Applicability {
    pub case_mix: String,
    pub india_cost_context: String,
    pub training: String,
}

/// The writer's answer (its structured output, after the citation check).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WrittenAnswer {
    pub bottom_line: BottomLine,
    pub outcomes: Vec<Outcome>,
    pub studies: Vec<Study>,
    pub conflicts: Vec<Conflict>,
    pub applicability: Applicability,
    pub would_change: Vec<String>,
    /// Stage 4 (additive, default empty or none: old answers and replies
    /// that omit them still load).
    #[serde(default)]
    pub at_a_glance: Option<AtAGlance>,
    #[serde(default)]
    pub clinical_practice: Option<ClinicalPractice>,
    #[serde(default)]
    pub guidelines: Vec<GuidelineV3>,
    #[serde(default)]
    pub notable_findings: Vec<NotableFinding>,
}

/// One search the researcher ran, as its tool returned it (recorded by
/// code, not reported by the model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchEntry {
    pub query: String,
    /// "PubMed", "Europe PMC" or "PubMed related".
    pub source: String,
    pub hits: u64,
}

/// What the code checks removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrityV3 {
    /// Studies the researcher selected that no tool returned in this run.
    pub dropped_unseen: u32,
    /// Selected studies whose record could not be fetched.
    pub dropped_unfetched: u32,
    /// `[PMID]` citations of a study outside the selection, removed.
    pub removed_citations: u32,
    /// Number entries whose source study is outside the selection.
    pub dropped_numbers: u32,
    /// Trial rows whose study is outside the selection.
    pub dropped_studies: u32,
    /// Stage 4 item 5 (additive): numbers written in the answer's text,
    /// and how many of them are not traced to a source (marked).
    #[serde(default)]
    pub numbers_checked: u32,
    #[serde(default)]
    pub numbers_untraced: u32,
    /// Stage 4 item 7 (additive): extractor numbers and outcomes kept but
    /// not verified.
    #[serde(default)]
    pub numbers_unverified: u32,
    #[serde(default)]
    pub outcomes_unverified: u32,
    /// Stage 4 schema fix (additive): certainty or evidence-type labels the
    /// writer gave that were not recognised, set to Very low / mixed.
    #[serde(default)]
    pub labels_defaulted: u32,
}

/// A citation marker: `[` PMIDs separated by `;` or `,` `]`, each optionally
/// prefixed `PMID`/`PMID:`. Digits only (1–9 each).
static CITE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\s*(?:PMIDs?:?\s*)?\d{1,9}(?:\s*[;,]\s*(?:PMIDs?:?\s*)?\d{1,9})*\s*\]")
        .expect("citation pattern is valid")
});

static DIGITS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{1,9}").expect("digits pattern is valid"));

/// A run of answer text: plain, or a citation of these PMIDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    Text(String),
    Cite(Vec<String>),
}

/// Shortest id that counts as a PMID on its own in a bracket.
pub const MIN_PMID_DIGITS: usize = 5;

/// `text` cut into plain runs and citation runs, in order. A bracket of
/// numbers is a citation only if one of its ids is `known` (a selected
/// study) or any id has at least [`MIN_PMID_DIGITS`] digits; otherwise
/// (e.g. "LOS 2 [1, 3] days") it stays text.
pub fn split_citations(text: &str, known: impl Fn(&str) -> bool) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut plain = String::new();
    let mut last = 0;
    for m in CITE.find_iter(text) {
        let ids: Vec<String> = DIGITS
            .find_iter(m.as_str())
            .map(|d| d.as_str().to_string())
            .collect();
        let citation =
            ids.iter().any(|i| known(i)) || ids.iter().any(|i| i.len() >= MIN_PMID_DIGITS);
        plain.push_str(&text[last..m.start()]);
        if citation {
            if !plain.is_empty() {
                out.push(Segment::Text(std::mem::take(&mut plain)));
            }
            out.push(Segment::Cite(ids));
        } else {
            plain.push_str(m.as_str());
        }
        last = m.end();
    }
    plain.push_str(&text[last..]);
    if !plain.is_empty() {
        out.push(Segment::Text(plain));
    }
    out
}

/// The byte ranges of the citation markers in `text` (as
/// [`split_citations`] decides them): numbers inside them are PMIDs, not
/// findings.
pub fn citation_ranges(text: &str, known: impl Fn(&str) -> bool) -> Vec<(usize, usize)> {
    CITE.find_iter(text)
        .filter(|m| {
            let ids: Vec<&str> = DIGITS.find_iter(m.as_str()).map(|d| d.as_str()).collect();
            ids.iter().any(|i| known(i)) || ids.iter().any(|i| i.len() >= MIN_PMID_DIGITS)
        })
        .map(|m| (m.start(), m.end()))
        .collect()
}

/// Every text field of the writer's answer with its span path (Stage 4
/// item 5), in page order. Structured numbers are not text; see
/// `writer_v3::trace_numbers`.
pub fn text_fields(a: &WrittenAnswer) -> Vec<(String, &str)> {
    let mut v: Vec<(String, &str)> = Vec::new();
    if let Some(g) = &a.at_a_glance {
        v.push(("at_a_glance.verdict".into(), &g.verdict));
        for (i, k) in g.key_numbers.iter().enumerate() {
            v.push((format!("at_a_glance.key_numbers.{i}.label"), &k.label));
            v.push((
                format!("at_a_glance.key_numbers.{i}.value_text"),
                &k.value_text,
            ));
        }
        v.push(("at_a_glance.main_caveat".into(), &g.main_caveat));
    }
    v.push(("bottom_line.text".into(), &a.bottom_line.text));
    v.push((
        "bottom_line.certainty_reason".into(),
        &a.bottom_line.certainty_reason,
    ));
    if let Some(c) = &a.clinical_practice {
        v.push(("clinical_practice.default_choice".into(), &c.default_choice));
        v.push((
            "clinical_practice.who_may_benefit".into(),
            &c.who_may_benefit,
        ));
        v.push(("clinical_practice.counselling".into(), &c.counselling));
        v.push(("clinical_practice.cost_per_case".into(), &c.cost_per_case));
        v.push(("clinical_practice.text".into(), &c.text));
    }
    for (i, o) in a.outcomes.iter().enumerate() {
        v.push((format!("outcomes.{i}.outcome"), &o.outcome));
        v.push((format!("outcomes.{i}.rct_evidence"), &o.rct_evidence));
        v.push((format!("outcomes.{i}.effect_text"), &o.effect_text));
        v.push((format!("outcomes.{i}.why"), &o.why));
    }
    for (i, s) in a.studies.iter().enumerate() {
        v.push((format!("studies.{i}.label"), &s.label));
        v.push((format!("studies.{i}.design"), &s.design));
        v.push((format!("studies.{i}.setting"), &s.setting));
        v.push((format!("studies.{i}.comparison"), &s.comparison));
        v.push((format!("studies.{i}.key_results"), &s.key_results));
        v.push((format!("studies.{i}.appraisal"), &s.appraisal));
    }
    for (i, c) in a.conflicts.iter().enumerate() {
        v.push((format!("conflicts.{i}.text"), &c.text));
    }
    for (i, g) in a.guidelines.iter().enumerate() {
        v.push((format!("guidelines.{i}.body"), &g.body));
        v.push((format!("guidelines.{i}.statement"), &g.statement));
    }
    v.push(("applicability.case_mix".into(), &a.applicability.case_mix));
    v.push((
        "applicability.india_cost_context".into(),
        &a.applicability.india_cost_context,
    ));
    v.push(("applicability.training".into(), &a.applicability.training));
    for (i, w) in a.would_change.iter().enumerate() {
        v.push((format!("would_change.{i}"), w));
    }
    for (i, n) in a.notable_findings.iter().enumerate() {
        v.push((format!("notable_findings.{i}.text"), &n.text));
    }
    v
}

/// `text` with every citation marker ([`split_citations`]) rewritten to keep only PMIDs for which
/// `known` is true (de-duplicated, in order, as `[a; b]`); a marker left
/// empty is removed with the blank before it. Returns the text and how many
/// PMID citations were removed.
pub fn clean_citations(text: &str, known: impl Fn(&str) -> bool) -> (String, u32) {
    let mut removed = 0u32;
    let mut out = String::with_capacity(text.len());
    for seg in split_citations(text, &known) {
        match seg {
            Segment::Text(t) => out.push_str(&t),
            Segment::Cite(ids) if !ids.iter().any(|i| known(i)) => {
                // No selected study in it: unknown PMIDs (5+ digits) are
                // removed and counted; short numbers stay as text.
                let mut short: Vec<String> = Vec::new();
                for id in ids {
                    if id.len() >= MIN_PMID_DIGITS {
                        removed += 1;
                    } else {
                        short.push(id);
                    }
                }
                if short.is_empty() {
                    let trimmed = out.trim_end_matches([' ', '\t']).len();
                    out.truncate(trimmed);
                } else {
                    out.push('[');
                    out.push_str(&short.join("; "));
                    out.push(']');
                }
            }
            Segment::Cite(ids) => {
                let mut keep: Vec<String> = Vec::new();
                for id in ids {
                    if !known(&id) {
                        removed += 1;
                    } else if !keep.contains(&id) {
                        keep.push(id);
                    }
                }
                if keep.is_empty() {
                    let trimmed = out.trim_end_matches([' ', '\t']).len();
                    out.truncate(trimmed);
                } else {
                    out.push('[');
                    out.push_str(&keep.join("; "));
                    out.push(']');
                }
            }
        }
    }
    (out, removed)
}
