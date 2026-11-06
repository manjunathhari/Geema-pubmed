//! Copied from Incisive at 54433fc, `agent/llm.rs` and `gate.rs`. See `.10x/decisions/sde/stage0-writer-test.md`.

use regex::Regex;
use std::sync::LazyLock;

/// Replacement for a neutralised tag opener: U+2039 SINGLE LEFT-POINTING ANGLE
/// QUOTATION MARK. `</source>` becomes `‹/source>`, `&lt;source` becomes
/// `‹source`. Whitespace and zero-width characters between the opener and the
/// name are dropped; the slash is kept.
const NEUTRAL_OPEN: char = '\u{2039}';

/// An opener (`<`, `&lt;`, fullwidth `＜`, or a numeric entity for `<`), then
/// optional blanks, an optional `/`, optional blanks, then `source`.
static SOURCE_TAG: LazyLock<Regex> = LazyLock::new(|| {
    const GAP: &str = r"[\s\u{200b}-\u{200d}\u{2060}\u{feff}]*";
    Regex::new(&format!(
        r"(?i)(?:<|&lt;|\u{{ff1c}}|&#0*60;|&#x0*3c;){GAP}(/?){GAP}source"
    ))
    .expect("source tag pattern is valid")
});

/// Break every opening or closing `source` tag in `text` (any case, spaced,
/// entity-encoded or fullwidth) by replacing its opener with `‹`. Deterministic
/// and idempotent. Apply it identically wherever text is shown to a model, so
/// verbatim quotes taken from the wrapped text still match.
pub fn neutralise_source_tags(text: &str) -> String {
    SOURCE_TAG
        .replace_all(text, |c: &regex::Captures| {
            format!("{NEUTRAL_OPEN}{}source", &c[1])
        })
        .into_owned()
}

/// Longest `id` [`wrap_source`] accepts.
pub const MAX_SOURCE_ID_LEN: usize = 64;

fn valid_source_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_SOURCE_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Wrap fetched text as `<source id="…">…</source>`, with every source tag
/// inside the text neutralised by [`neutralise_source_tags`].
///
/// `id` must be 1 to 64 characters of `[A-Za-z0-9_-]` (callers pass `ref-N`).
/// A bad id is a programming error: it panics in debug builds; in release it
/// is cleaned (invalid characters dropped, cut to 64, `x` if nothing is left)
/// so the wrapper can never be broken out of.
pub fn wrap_source(id: &str, text: &str) -> String {
    debug_assert!(valid_source_id(id), "invalid source id");
    let mut id: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .take(MAX_SOURCE_ID_LEN)
        .collect();
    if id.is_empty() {
        id.push('x');
    }
    format!(
        "<source id=\"{id}\">\n{}\n</source>",
        neutralise_source_tags(text)
    )
}

/// Stop the question from opening or closing our `<question>` wrapper.
pub(crate) fn neutralise_close_tag(question: &str) -> String {
    let once = replace_ci(question, "</question>", "< /question>");
    replace_ci(&once, "<question>", "< question>")
}

/// ASCII case-insensitive replace of every occurrence of `tag`.
fn replace_ci(text: &str, tag: &str, with: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (i, _) in lower.match_indices(tag) {
        out.push_str(&text[last..i]);
        out.push_str(with);
        last = i + tag.len();
    }
    out.push_str(&text[last..]);
    out
}
