//! Copied from Incisive at 54433fc, `agent/extract.rs`. See `.10x/decisions/sde/stage0-writer-test.md`.

use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

enum Fold {
    /// Dropped: soft hyphen and zero-width characters.
    Skip,
    Space,
    Char(char),
}

/// Middle dots used as decimal points (Lancet style): U+00B7, U+2219, U+22C5.
const MIDDLE_DOTS: &[char] = &['\u{b7}', '\u{2219}', '\u{22c5}'];

/// The folded form of `c`, whose neighbours in the source are `prev` and
/// `next`. A middle dot between two ASCII digits is a decimal point (Ruling
/// R-11e); elsewhere it is left alone.
fn fold(c: char, prev: Option<char>, next: Option<char>) -> Fold {
    let digit = |c: Option<char>| c.is_some_and(|c| c.is_ascii_digit());
    match c {
        c if MIDDLE_DOTS.contains(&c) && digit(prev) && digit(next) => Fold::Char('.'),
        '\u{ad}' | '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}' => Fold::Skip,
        c if c.is_whitespace() => Fold::Space,
        '\u{2018}' | '\u{2019}' => Fold::Char('\''),
        '\u{201c}' | '\u{201d}' => Fold::Char('"'),
        // Hyphen, non-breaking hyphen, figure dash, en dash, em dash,
        // horizontal bar, minus sign.
        '\u{2010}'..='\u{2015}' | '\u{2212}' => Fold::Char('-'),
        c => Fold::Char(c),
    }
}

/// A normalised text with the source byte span of every output byte and the
/// output positions of spaces that stand for a paragraph break.
struct Mapped {
    text: String,
    map: Vec<(usize, usize)>,
    paragraphs: HashSet<usize>,
}

fn normalise_mapped(s: &str) -> Mapped {
    let mut text = String::with_capacity(s.len());
    let mut map = Vec::with_capacity(s.len());
    let mut paragraphs = HashSet::new();
    // (start, end, newlines) of the pending whitespace run.
    let mut gap: Option<(usize, usize, usize)> = None;
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let end = i + c.len_utf8();
        let prev = k.checked_sub(1).map(|j| chars[j].1);
        let next = chars.get(k + 1).map(|&(_, c)| c);
        match fold(c, prev, next) {
            Fold::Skip => {}
            Fold::Space => {
                let nl = usize::from(c == '\n');
                gap = Some(gap.map_or((i, end, nl), |(start, _, n)| (start, end, n + nl)));
            }
            Fold::Char(m) => {
                if let Some((start, gend, newlines)) = gap.take() {
                    // Leading whitespace is dropped; trailing never gets flushed.
                    if !text.is_empty() {
                        if newlines >= 2 {
                            paragraphs.insert(text.len());
                        }
                        text.push(' ');
                        map.push((start, gend));
                    }
                }
                text.push(m);
                map.extend(std::iter::repeat_n((i, end), m.len_utf8()));
            }
        }
    }
    Mapped {
        text,
        map,
        paragraphs,
    }
}

/// The form both sides are compared in: every run of Unicode whitespace
/// becomes one space, the ends are trimmed, soft hyphens and zero-width
/// characters (U+00AD, U+200B to U+200D, U+2060, U+FEFF) are removed, ‘’
/// become `'`, “” become `"`, hyphen, dash and minus characters (U+2010 to
/// U+2015, U+2212) become `-`, and a middle dot (U+00B7, U+2219, U+22C5)
/// between two ASCII digits becomes `.`. Case is kept.
pub fn normalise(s: &str) -> String {
    normalise_mapped(s).text
}

/// One number: digits with an optional decimal point (no decimal comma:
/// "3,4" is two numbers, Stage 3 re-review 4; grouped thousands are read
/// whole first, [`THOUSANDS`]).
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-?(?:\d+(?:\.\d+)?|\.\d+)%?").expect("number pattern is valid"));

/// A run of digits with `.` / `,` separators (and an optional sign and `%`),
/// read whole first so a thousands-grouped number is one token.
static NUMBER_RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-?(?:\d+(?:[.,]\d+)*|\.\d+)%?").expect("number run pattern is valid")
});

/// Thousands grouping: `1,234`, `1,136,000`, `12,345.6` (Stage 3 review 7).
static THOUSANDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^-?\d{1,3}(?:,\d{3})+(?:\.\d+)?%?$").expect("thousands pattern is valid")
});

/// Stage 4 follow-up: where a *spaced* minus sign before byte `start` of `s`
/// begins. A dash (`-`, `−`, `–`) followed by exactly one space before a
/// number is a sign when the character before the dash (spaces skipped)
/// opens a value: `,` `(` `:` `;` or `[` ("95% CI, - 0.34" is −0.34, a
/// PDF-extraction artefact). After a digit it stays a range or a
/// subtraction ("12 - 2.3" is 12 and 2.3). Shared by quotes ([`numbers`])
/// and free text (the extractor's `signed_start`), so both read alike.
pub(crate) fn spaced_minus(s: &str, start: usize) -> Option<usize> {
    let before = s.get(..start)?;
    let rest = before.strip_suffix(' ')?;
    let dash = rest.chars().next_back()?;
    if !matches!(dash, '-' | '\u{2212}' | '\u{2013}') {
        return None;
    }
    let at = rest.len() - dash.len_utf8();
    let opener = rest[..at].trim_end_matches(' ').chars().next_back()?;
    matches!(opener, ',' | '(' | ':' | ';' | '[').then_some(at)
}

/// The number tokens of `s`, read from its normalised (folded) form, so a
/// middle-dot decimal is one number. A `-` right after a digit or `%` is a
/// range dash, not a sign, and right after a letter a hyphen ("COVID-19"
/// is 19, not -19); a thousands-grouped number (`1,136,000`) is one number
/// without its commas; a leading `.` reads as `0.` (".52" is 0.52).
pub(crate) fn numbers(s: &str) -> Vec<String> {
    let s = normalise(s);
    let mut out = Vec::new();
    for run in NUMBER_RUN.find_iter(&s) {
        // Whole runs that are thousands-grouped are one token; anything
        // else is read as before, piece by piece.
        let pieces: Vec<(usize, String)> = if THOUSANDS.is_match(run.as_str()) {
            vec![(run.start(), run.as_str().replace(',', ""))]
        } else {
            NUMBER
                .find_iter(run.as_str())
                .map(|m| (run.start() + m.start(), m.as_str().to_string()))
                .collect()
        };
        for (start, t) in pieces {
            let mut t = t.as_str();
            if t.starts_with('-')
                && s[..start]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_digit() || c == '%' || c.is_alphabetic())
            {
                t = &t[1..];
            }
            let spaced;
            if !t.starts_with('-') && spaced_minus(&s, start).is_some() {
                spaced = format!("-{t}");
                t = &spaced;
            }
            out.push(match t.strip_prefix('-') {
                Some(r) if r.starts_with('.') => format!("-0{r}"),
                _ if t.starts_with('.') => format!("0{t}"),
                _ => t.to_string(),
            });
        }
    }
    out
}
