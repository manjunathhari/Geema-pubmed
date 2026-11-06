//! After each run: labels the failure types and runs Incisive's Quick checks.
//! Logs get only the codes and counts. The cleaned answer goes to the result file.

use crate::codes::Code;
use crate::copied::answer::{IntegrityV3, Span, WrittenAnswer};
use crate::copied::writer::{
    StudyInput, WireAnswer, allowed_numbers_quick, check_citations, trace_numbers,
};
use crate::ollama::{ChatOutcome, NUM_CTX, NUM_PREDICT};
use serde::{Deserialize, Serialize};

/// Counts from the Quick checks, on a valid answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCounts {
    pub outcomes: u32,
    pub studies: u32,
    pub numbers: u32,
    pub outcomes_without_numbers: u32,
    pub labels_defaulted: u32,
    pub removed_citations: u32,
    pub dropped_numbers: u32,
    pub dropped_studies: u32,
    pub numbers_checked: u32,
    pub numbers_untraced: u32,
}

/// A valid answer after the Quick checks, as Incisive stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Checked {
    pub counts: CheckCounts,
    /// The answer with outside citations and rows removed.
    pub answer: WrittenAnswer,
    /// Numbers no source holds, as places in `answer`.
    pub untraced: Vec<Span>,
}

/// Bytes a token, measured on real abstracts (model-prompt report section 2).
const BYTES_PER_TOKEN: f64 = 3.85;
/// A prompt count further than one part in this from the estimate is flagged.
const OFF_BY_ONE_IN: u64 = 4;
/// A runaway: the end of the reply is one pattern, end to end, at least this
/// many times over and at least this long. Numbers don't count as a difference,
/// because a runaway often counts upwards. A real answer's rows share their field
/// names, but not their wording, so they don't match.
const LOOP_REPEATS: usize = 5;
const LOOP_MIN_BYTES: usize = 1_024;
const LOOP_MAX_PATTERN_BYTES: usize = 4_096;

/// Estimated prompt tokens. No tokenizer is bundled.
pub fn estimate_tokens(prompt_bytes: usize) -> u64 {
    (prompt_bytes as f64 / BYTES_PER_TOKEN).round() as u64
}

/// The bytes with each run of digits as one `#`, so 99 and 100 read the same.
fn digits_as_one(reply: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(reply.len());
    for &b in reply.as_bytes() {
        if !b.is_ascii_digit() {
            out.push(b);
        } else if out.last() != Some(&b'#') {
            out.push(b'#');
        }
    }
    out
}

fn looks_like_a_loop(reply: &str) -> bool {
    let b = digits_as_one(reply);
    (1..=LOOP_MAX_PATTERN_BYTES).any(|pattern| {
        let span = (pattern * LOOP_REPEATS).max(LOOP_MIN_BYTES);
        span <= b.len() && b[b.len() - span..b.len() - pattern] == b[b.len() - span + pattern..]
    })
}

/// Incisive's Quick checks, in the order `team.rs` runs them. The empty-list
/// flags read the answer as written, before the citation check drops rows.
fn quick_checks(a: WireAnswer, studies: &[StudyInput]) -> (Vec<Code>, Checked) {
    let selected: Vec<String> = studies.iter().map(|s| s.pmid.clone()).collect();
    let (mut written, labels_defaulted) = a.into_answer();
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let mut c = CheckCounts {
        outcomes: count(written.outcomes.len()),
        studies: count(written.studies.len()),
        numbers: count(written.outcomes.iter().map(|o| o.numbers.len()).sum()),
        outcomes_without_numbers: count(
            written
                .outcomes
                .iter()
                .filter(|o| o.numbers.is_empty())
                .count(),
        ),
        labels_defaulted,
        ..CheckCounts::default()
    };
    let mut codes = Vec::new();
    if c.outcomes == 0 {
        codes.push(Code::EmptyOutcomes);
    }
    if c.studies == 0 {
        codes.push(Code::EmptyStudies);
    }
    if c.numbers == 0 {
        codes.push(Code::EmptyNumbers);
    }
    let mut integrity = IntegrityV3::default();
    check_citations(&mut written, &selected, &mut integrity);
    c.removed_citations = integrity.removed_citations;
    c.dropped_numbers = integrity.dropped_numbers;
    c.dropped_studies = integrity.dropped_studies;
    let allowed = allowed_numbers_quick(studies);
    let (untraced, checked) = trace_numbers(
        &written,
        &allowed,
        |p| selected.iter().any(|s| s == p),
        false,
    );
    c.numbers_checked = checked;
    c.numbers_untraced = count(untraced.len());
    let checked = Checked {
        counts: c,
        answer: written,
        untraced,
    };
    (codes, checked)
}

/// The codes for one finished call, and the checked answer if it is valid.
pub fn evaluate(
    o: &ChatOutcome,
    prompt_bytes: usize,
    studies: &[StudyInput],
) -> (Vec<Code>, Option<Checked>) {
    if let Some(f) = o.failure {
        return (vec![f], None);
    }
    let mut codes = Vec::new();
    let k = &o.counters;
    if k.prompt_eval_count.is_none() || k.eval_count.is_none() {
        codes.push(Code::CountsMissing);
    }
    if let Some(p) = k.prompt_eval_count {
        let est = estimate_tokens(prompt_bytes);
        if p.abs_diff(est) * OFF_BY_ONE_IN > est {
            codes.push(Code::PromptCountOff);
        }
    }
    let context_used_up = match (k.prompt_eval_count, k.eval_count) {
        (Some(p), Some(e)) => p + e >= NUM_CTX,
        _ => false,
    };
    let at_predict_limit = k.eval_count.is_some_and(|e| e >= NUM_PREDICT);
    let reason = k.done_reason.as_deref();
    match reason {
        Some("stop") if context_used_up => codes.push(Code::PromptCut),
        Some("stop") => {}
        Some("length") if context_used_up || at_predict_limit => {
            codes.push(if looks_like_a_loop(&o.reply) {
                Code::RunawaySuspected
            } else if context_used_up {
                Code::ContextFull
            } else {
                Code::NumPredictNeedsReading
            });
        }
        _ => codes.push(Code::DoneReasonOther),
    }
    match serde_json::from_str::<WireAnswer>(&o.reply) {
        Ok(a) => {
            let (empty, checked) = quick_checks(a, studies);
            codes.extend(empty);
            (codes, Some(checked))
        }
        Err(_) => {
            if reason == Some("stop") {
                codes.push(Code::InvalidJsonCleanFinish);
            }
            (codes, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ollama::Counters;
    use serde_json::json;

    const PMID: &str = "11111111";

    fn studies() -> Vec<StudyInput> {
        vec![StudyInput {
            pmid: PMID.into(),
            title: "A made-up trial".into(),
            journal: None,
            year: Some(2021),
            pub_types: vec![],
            doi: None,
            retracted: false,
            abstract_text: Some("Pain 4.5 vs 5.5.".into()),
            design: "not available".into(),
            why: "not available".into(),
        }]
    }

    fn answer() -> serde_json::Value {
        json!({
            "applicability": {"case_mix": "c", "india_cost_context": "i", "training": "t"},
            "at_a_glance": "Made-up verdict.\nMain caveat: small.",
            "bottom_line": {"text": "Pain 4.5 vs 5.5 [11111111]. Also 9.9 [99999999].",
                            "certainty": "Low", "certainty_reason": "small"},
            "clinical_practice": "Default choice: either.",
            "conflicts": [],
            "notable_findings": "None noted [11111111].",
            "outcomes": [{
                "outcome": "Pain", "rct_evidence": "one trial", "effect_text": "4.5 vs 5.5 [11111111]",
                "numbers": [
                    {"label": "pain", "value": 4.5, "unit": null, "arm": "A", "ci_low": null,
                     "ci_high": null, "p": null, "pmid": PMID},
                    {"label": "pain", "value": 7.7, "unit": null, "arm": "B", "ci_low": null,
                     "ci_high": null, "p": null, "pmid": PMID},
                    {"label": "other", "value": 5.5, "unit": null, "arm": "B", "ci_low": null,
                     "ci_high": null, "p": null, "pmid": "99999999"}
                ],
                "certainty": "not a word", "why": "w", "pmids": [PMID]
            }],
            "studies": [{"pmid": PMID, "label": "L", "design": "RCT", "n": null, "setting": "s",
                         "comparison": "c", "key_results": "k", "appraisal": "a"}],
            "would_change": []
        })
    }

    /// A prompt size whose estimate is exactly `tokens`.
    fn bytes_for(tokens: u64) -> usize {
        (tokens as f64 * 3.85).round() as usize
    }

    fn outcome(reply: &str, prompt: u64, eval: u64, reason: &str) -> ChatOutcome {
        ChatOutcome {
            reply: reply.into(),
            counters: Counters {
                prompt_eval_count: Some(prompt),
                eval_count: Some(eval),
                done_reason: Some(reason.into()),
                ..Counters::default()
            },
            ..ChatOutcome::default()
        }
    }

    #[test]
    fn a_clean_valid_answer_has_no_codes_and_real_check_counts() {
        let o = outcome(&answer().to_string(), 10_000, 500, "stop");
        let (codes, c) = evaluate(&o, bytes_for(10_000), &studies());
        assert_eq!(codes, []);
        let checked = c.unwrap();
        assert!(
            !checked.answer.bottom_line.text.contains("99999999"),
            "the outside citation is removed"
        );
        assert_eq!(checked.answer.outcomes[0].numbers.len(), 2);
        assert_eq!(checked.untraced.len(), 2);
        let c = checked.counts;
        assert_eq!((c.outcomes, c.studies, c.numbers), (1, 1, 3));
        assert_eq!(c.outcomes_without_numbers, 0);
        assert_eq!(c.labels_defaulted, 1, "the unknown certainty word");
        assert_eq!(c.removed_citations, 1, "[99999999]");
        assert_eq!(c.dropped_numbers, 1, "the number citing 99999999");
        assert_eq!(c.dropped_studies, 0);
        assert!(c.numbers_checked > 0);
        // 9.9 in the bottom line and the structured 7.7.
        assert_eq!(c.numbers_untraced, 2);
    }

    #[test]
    fn bad_json_after_a_clean_finish_is_labelled() {
        for reply in ["{ not json", "[1,2,3]", r#"{"bottom_line":{}}"#, ""] {
            let o = outcome(reply, 10_000, 500, "stop");
            let (codes, c) = evaluate(&o, bytes_for(10_000), &studies());
            assert_eq!(codes, [Code::InvalidJsonCleanFinish], "{reply}");
            assert_eq!(c, None);
        }
    }

    #[test]
    fn done_reason_length_at_num_predict_needs_reading() {
        let o = outcome("{\"cut", 10_000, NUM_PREDICT, "length");
        let (codes, c) = evaluate(&o, bytes_for(10_000), &studies());
        assert_eq!(codes, [Code::NumPredictNeedsReading]);
        assert_eq!(c, None);
    }

    #[test]
    fn done_reason_length_with_a_full_context_is_context_full() {
        let o = outcome("{\"cut", NUM_CTX - 100, 100, "length");
        let (codes, _) = evaluate(&o, bytes_for(NUM_CTX - 100), &studies());
        assert_eq!(codes, [Code::ContextFull]);
    }

    #[test]
    fn prompt_plus_output_at_num_ctx_with_a_clean_stop_is_a_prompt_cut() {
        let o = outcome(&answer().to_string(), NUM_CTX - 100, 100, "stop");
        let (codes, c) = evaluate(&o, bytes_for(NUM_CTX - 100), &studies());
        assert_eq!(codes, [Code::PromptCut]);
        assert!(c.is_some(), "still valid, so still counted");
        // One token below the limit passes.
        let o = outcome(&answer().to_string(), NUM_CTX - 100, 99, "stop");
        let (codes, _) = evaluate(&o, bytes_for(NUM_CTX - 100), &studies());
        assert_eq!(codes, []);
    }

    #[test]
    fn a_prompt_count_more_than_25_percent_off_the_estimate_is_flagged() {
        let bytes = bytes_for(10_000);
        for (count, flagged) in [
            (10_000, false),
            (12_500, false),
            (7_500, false),
            (12_501, true),
            (7_499, true),
        ] {
            let o = outcome(&answer().to_string(), count, 500, "stop");
            let (codes, _) = evaluate(&o, bytes, &studies());
            assert_eq!(codes.contains(&Code::PromptCountOff), flagged, "{count}");
        }
    }

    #[test]
    fn a_done_line_without_counts_can_not_pass() {
        let mut o = outcome(&answer().to_string(), 1, 1, "stop");
        o.counters.prompt_eval_count = None;
        assert_eq!(evaluate(&o, 40, &studies()).0, [Code::CountsMissing]);
        let mut o = outcome(&answer().to_string(), 10_000, 1, "stop");
        o.counters.eval_count = None;
        assert_eq!(
            evaluate(&o, bytes_for(10_000), &studies()).0,
            [Code::CountsMissing]
        );
    }

    #[test]
    fn an_unknown_or_absent_done_reason_is_flagged() {
        let o = outcome(&answer().to_string(), 10_000, 500, "load");
        assert_eq!(
            evaluate(&o, bytes_for(10_000), &studies()).0,
            [Code::DoneReasonOther]
        );
        let mut o = outcome(&answer().to_string(), 10_000, 500, "stop");
        o.counters.done_reason = None;
        assert_eq!(
            evaluate(&o, bytes_for(10_000), &studies()).0,
            [Code::DoneReasonOther]
        );
        // "length" with neither limit reached is not explained.
        let o = outcome("{", 10_000, 500, "length");
        assert_eq!(
            evaluate(&o, bytes_for(10_000), &studies()).0,
            [Code::DoneReasonOther]
        );
    }

    fn at_predict_limit(reply: &str) -> Vec<Code> {
        let o = outcome(reply, 10_000, NUM_PREDICT, "length");
        evaluate(&o, bytes_for(10_000), &studies()).0
    }

    /// The first ~45 KB of a made-up answer, then `row(i)` until the limit.
    fn runaway(row: impl Fn(u32) -> String) -> String {
        let mut s = format!("{{\"at_a_glance\":\"{}", distinct_words(2_000));
        let mut i = 95;
        while s.len() < 45_000 {
            s.push_str(&row(i));
            i += 1;
        }
        s
    }

    /// Words that never settle into a pattern.
    fn distinct_words(bytes: usize) -> String {
        const WORDS: [&str; 8] = [
            "mesh",
            "pain",
            "trial",
            "recurrence",
            "cost",
            "operative",
            "groin",
            "follow-up",
        ];
        let (mut x, mut s) = (0x1234_5678_9abc_def1u64, String::new());
        while s.len() < bytes {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            s.push_str(WORDS[(x % 8) as usize]);
            s.push(' ');
        }
        s
    }

    #[test]
    fn a_repeating_reply_at_the_limit_is_a_runaway_and_not_setup() {
        assert_eq!(
            at_predict_limit(&"loop text, ".repeat(100)),
            [Code::RunawaySuspected]
        );
    }

    #[test]
    fn a_runaway_with_rising_numbers_is_not_labelled_setup() {
        // The review's two cases: repeating rows, and one long string.
        let rows = runaway(|i| {
            format!(
                "{{\"label\":\"row {i}\",\"value\":{i}.5,\"unit\":null,\"arm\":\"A\",\"pmid\":\"{}\"}},",
                11_111_111 + i
            )
        });
        assert_eq!(at_predict_limit(&rows), [Code::RunawaySuspected]);
        let string = runaway(|i| format!("and in patient {i} the pain score was {} ", i * 3));
        assert_eq!(at_predict_limit(&string), [Code::RunawaySuspected]);
    }

    #[test]
    fn a_runaway_that_fills_the_context_is_not_labelled_setup() {
        let string = runaway(|i| format!("and in patient {i} the pain score was {} ", i * 3));
        let o = outcome(&string, NUM_CTX - 100, 100, "length");
        assert_eq!(
            evaluate(&o, bytes_for(NUM_CTX - 100), &studies()).0,
            [Code::RunawaySuspected]
        );
    }

    #[test]
    fn a_long_reply_with_no_clear_loop_at_the_limit_needs_reading() {
        // Plain long text.
        assert_eq!(
            at_predict_limit(&distinct_words(45_000)),
            [Code::NumPredictNeedsReading]
        );
        // Honest rows share their field names, so the last 64 bytes recur. Not a loop.
        let words = distinct_words(45_000);
        let rows: String = words
            .split(' ')
            .enumerate()
            .map(|(i, w)| {
                format!(
                    "{{\"label\":\"{w}\",\"value\":{i},\"unit\":null,\"arm\":\"A\",\"ci_low\":null,\"ci_high\":null,\"p\":null,\"pmid\":\"11111111\"}},"
                )
            })
            .collect();
        assert_eq!(at_predict_limit(&rows), [Code::NumPredictNeedsReading]);
        // A counter in words can't be told from writing, so it isn't guessed at.
        let spelled = [
            "one", "two", "three", "four", "five", "six", "seven", "eight",
        ];
        let counted = runaway(|i| {
            let word = (u64::from(i).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 61) as usize;
            format!("row {} ", spelled[word])
        });
        assert_eq!(at_predict_limit(&counted), [Code::NumPredictNeedsReading]);
        // A short repeat at the end is too little to call.
        let short = format!("{}{}", distinct_words(45_000), "row 1, ".repeat(20));
        assert_eq!(at_predict_limit(&short), [Code::NumPredictNeedsReading]);
    }

    #[test]
    fn empty_lists_are_flagged() {
        let mut a = answer();
        a["outcomes"] = json!([]);
        a["studies"] = json!([]);
        let (codes, c) = evaluate(
            &outcome(&a.to_string(), 10_000, 500, "stop"),
            bytes_for(10_000),
            &studies(),
        );
        assert_eq!(
            codes,
            [Code::EmptyOutcomes, Code::EmptyStudies, Code::EmptyNumbers]
        );
        assert_eq!(c.unwrap().counts.outcomes, 0);

        let mut a = answer();
        a["outcomes"][0]["numbers"] = json!([]);
        let (codes, c) = evaluate(
            &outcome(&a.to_string(), 10_000, 500, "stop"),
            bytes_for(10_000),
            &studies(),
        );
        assert_eq!(codes, [Code::EmptyNumbers]);
        assert_eq!(c.unwrap().counts.outcomes_without_numbers, 1);
    }

    #[test]
    fn a_failed_call_keeps_its_code_and_runs_no_checks() {
        let o = ChatOutcome {
            reply: answer().to_string(),
            failure: Some(Code::TimeGap),
            ..ChatOutcome::default()
        };
        assert_eq!(evaluate(&o, 40, &studies()), (vec![Code::TimeGap], None));
    }
}
