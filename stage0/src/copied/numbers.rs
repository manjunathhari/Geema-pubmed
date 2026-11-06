//! Copied from Incisive at 54433fc, `agent/extractor.rs`. See `.10x/decisions/sde/stage0-writer-test.md`.

use super::tokens::numbers;
use std::sync::LazyLock;

/// The numeric values of `quote`'s number tokens (the Phase 2 tokens:
/// folded dashes and middle dots, range dashes, thousands separators,
/// leading-dot decimals; a trailing `%` dropped), plus the numbers it
/// writes as words ([`word_values`]: "Sixty patients" is 60).
pub(crate) fn quote_values(quote: &str) -> Vec<f64> {
    let mut v: Vec<f64> = numbers(quote)
        .iter()
        .filter_map(|t| t.trim_end_matches('%').parse::<f64>().ok())
        .collect();
    v.extend(word_values(quote));
    v
}

const UNIT_WORDS: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];

const TENS_WORDS: [&str; 8] = [
    "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

fn unit_word(w: &str) -> Option<u32> {
    UNIT_WORDS.iter().position(|u| *u == w).map(|i| i as u32)
}

fn tens_word(w: &str) -> Option<u32> {
    TENS_WORDS
        .iter()
        .position(|t| *t == w)
        .map(|i| 20 + 10 * i as u32)
}

/// 0–99 starting at `words[i]` ("sixty", "sixty-two" or "sixty two",
/// "seven"), and the index after it.
fn below_hundred(words: &[String], i: usize) -> Option<(u32, usize)> {
    let w = words.get(i)?;
    if let Some(u) = unit_word(w) {
        return Some((u, i + 1));
    }
    let t = tens_word(w)?;
    match words.get(i + 1).and_then(|w| unit_word(w)) {
        Some(u) if (1..=9).contains(&u) => Some((t + u, i + 2)),
        _ => Some((t, i + 1)),
    }
}

/// Numbers written as words in `text` (Stage 4 item 10): "zero" to
/// "ninety-nine", compounds with a hyphen or a space ("Sixty-two"),
/// "hundred" (100) and "N hundred [and] M" (e.g. "one hundred and five").
/// Case does not matter. Used only to widen what a source is allowed to
/// support (its quotes and its text), never to read the model's text.
pub(crate) fn word_values(text: &str) -> Vec<f64> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        if words[i] == "hundred" {
            out.push(100.0);
            i += 1;
            continue;
        }
        let Some((mut v, mut j)) = below_hundred(&words, i) else {
            i += 1;
            continue;
        };
        if v > 0 && v < 10 && words.get(j).is_some_and(|w| w == "hundred") {
            v *= 100;
            j += 1;
            let k = if words.get(j).is_some_and(|w| w == "and") {
                j + 1
            } else {
                j
            };
            if let Some((rest, after)) = below_hundred(&words, k) {
                v += rest;
                j = after;
            }
        }
        out.push(f64::from(v));
        i = j;
    }
    out
}

fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// The digits of a number written in free text (a digit run with
/// separators and an optional `%`), without any sign: whether a dash in
/// front is a minus is decided from its context ([`signed_start`]).
static WRITTEN_NUMBER: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\d(?:[\d.,\u{b7}]*\d)?%?").expect("written number pattern is valid")
});

/// Where a number written at `start` in `text` begins once its sign is
/// taken into account: one character earlier when a dash (`-`, `−`, `–`)
/// right before it is a minus sign. The dash is **not** a sign (it is a
/// range dash or a hyphen) when the character before it is a digit, `%` or
/// a letter (`0.48-1.13`, `12%-15%`, `COVID-19`). Exactly the rule
/// `extract::numbers` applies to quotes, so free text and quotes tokenise
/// alike: after a space the dash is a sign in both ("12 −2.3" is 12 and
/// -2.3; a spaced range "2019 – 2021" reads as 2019 and -2021 in both).
fn signed_start(text: &str, start: usize) -> usize {
    // A spaced minus after a value opener ("CI, - 0.34"), as in quotes.
    if let Some(at) = super::tokens::spaced_minus(text, start) {
        return at;
    }
    let mut back = text[..start].chars().rev();
    let Some(dash) = back.next() else {
        return start;
    };
    if !matches!(dash, '-' | '\u{2212}' | '\u{2013}') {
        return start;
    }
    let range = match back.next() {
        None => false,
        Some(c) if c.is_ascii_digit() || c == '%' || c.is_alphabetic() => true,
        Some(_) => false,
    };
    if range {
        start
    } else {
        start - dash.len_utf8()
    }
}

/// Each number written in `text` (in digits) as its byte range, sign
/// included ([`signed_start`]), with its values.
pub(crate) fn written_numbers(text: &str) -> Vec<(usize, usize, Vec<f64>)> {
    let mut out = Vec::new();
    let mut last = 0;
    for m in WRITTEN_NUMBER.find_iter(text) {
        let start = signed_start(text, m.start()).max(last);
        // A spaced sign ("CI, - 0.34") is read with the number it signs.
        let written = if start < m.start() && text[start..m.start()].ends_with(' ') {
            format!("-{}", &text[m.start()..m.end()])
        } else {
            text[start..m.end()].to_string()
        };
        let values: Vec<f64> = numbers(&written)
            .iter()
            .filter_map(|t| t.trim_end_matches('%').parse::<f64>().ok())
            .collect();
        out.push((start, m.end(), values));
        last = m.end();
    }
    out
}

/// Whether every one of `values` is among `allowed`.
pub(crate) fn all_allowed(values: &[f64], allowed: &[f64]) -> bool {
    values.iter().all(|v| allowed.iter().any(|a| same(*a, *v)))
}

/// Whether `a` and `b` are the same number up to its sign (Stage 4b item
/// 1): prose often drops a sign the source writes ("13.4 min shorter" for
/// "−13.4"). U+2212 and the hyphen-minus are already one sign in the
/// tokeniser ([`super::tokens::numbers`] folds them).
pub(crate) fn same_magnitude(a: f64, b: f64) -> bool {
    same(a.abs(), b.abs())
}

/// Whether the number written at byte `start` of `text` (as
/// [`written_numbers`] gives it, its minus included) carries a sign of its
/// own: a minus (`-`, `−`, `–`) or a `+` right before it. Review S1: only a
/// number with no sign of its own may match its source up to the sign.
pub(crate) fn has_own_sign(text: &str, start: usize) -> bool {
    text.get(start..)
        .is_some_and(|t| t.starts_with(['-', '\u{2212}', '\u{2013}', '+']))
        || text.get(..start).is_some_and(|t| t.ends_with('+'))
}

/// Review S1: whether a number written in prose traces to `allowed`. Exact
/// values always do. With `loose` (Full and the coverage check, not Quick),
/// a number written with no sign of its own also matches its source's
/// magnitude ("13.4 min shorter" for "−13.4"); one with its own sign must
/// match exactly (a written "+2.3" against a source "−2.3" stays marked).
pub(crate) fn written_traced(values: &[f64], own_sign: bool, allowed: &[f64], loose: bool) -> bool {
    all_allowed(values, allowed) || (loose && !own_sign && all_allowed_unsigned(values, allowed))
}

/// [`all_allowed`] up to the sign (Stage 4b item 1): for checks of the
/// answer against its sources (the writer's "not traced" marks and the
/// coverage check), where a sign difference is wording, not a different
/// finding. The extractor's own quote checks stay exact.
pub(crate) fn all_allowed_unsigned(values: &[f64], allowed: &[f64]) -> bool {
    values
        .iter()
        .all(|v| allowed.iter().any(|a| same_magnitude(*a, *v)))
}
