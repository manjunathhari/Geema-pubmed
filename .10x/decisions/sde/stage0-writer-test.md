# Stage 0 writer test: what was built

Branch `stage0/runner`. Crate `stage0/`, program `stage0-runner`. Throwaway test, not the app.

## Copied code
Fact: `stage0/src/copied/` is copied from Incisive at 54433fc, with no Claude types.
- `writer.rs`: from `agent/writer_v3.rs`, plus `Contradiction` and its limit from `researcher.rs`.
- `answer.rs`: from `agent/answer_v3.rs`.
- `numbers.rs`: from `agent/extractor.rs`. `tokens.rs`: from `agent/extract.rs`.
- `wrap.rs`: from `agent/llm.rs` and `gate.rs`.
- `writer_tests.rs`: 13 tests and 2 helpers from `writer_v3.rs` `mod tests`.

The first commit on the branch is the plain copy, with module paths changed. Later edits to copied files:
1. `notes_tail`: "not available" in place of "(none)", twice.
2. `DESIGN_BYTES` 13, not 10. At 10, "not available" would print as "not availa". A test covers this.
3. New `allowed_numbers_quick`: the Quick path of `allowed_numbers`, written from its `full = None` path.
4. `#[allow(dead_code)]` on `Mapped`, whose other fields Incisive reads elsewhere.

## Prompt
One user message: SYSTEM, blank line, `writer_user`, blank line, compact schema. Field order is as the schema gives it (sorted by `serde_json`). A test holds the schema at 2,330 bytes and SYSTEM at 4,361 bytes.

## Departures and additions, for the owner to check
1. **Search log also says "not available".** The brief names reason, design label and contradiction notes. The job file has no search log, and "(none)" is ruled out. Stored answers do hold `searches`, so the log could be added to the job later if the owner wants it.
2. **The Ollama call uses `std::net`, not `ureq`.** A blocking `ureq` call can't be closed from outside and has one read timeout per request. Closing the socket is what stops Ollama writing on a shared server. `ureq` is used for NCBI only. So `OLLAMA_URL` must be plain `http://`.
3. **XML is read by a small parser in `xml.rs`.** Incisive uses `quick-xml`, which isn't on the approved dependency list.
4. **Title, journal, year and DOI come from efetch**, as the brief says. Incisive takes them from esummary, so wording may differ slightly.
5. **Optional `STAGE0_NCBI_URL`**, so tests can point at a mock. Leave it unset on the server.
6. **A label whose studies have no abstract at all is not sent to Gemma** (`NCBI_NO_ABSTRACTS`). Writing from nothing would hold a shared server for no result.
7. **A repeat run keeps the earlier result** as `<label>.prev-<time>.json`.
8. **Failed fetches retry** twice more, 2 seconds apart, staying under 3 requests a second.

## Limits to know
- Seed is fixed at 11, as the brief says. Repeats on the same prompt with one seed may add little (Estimate). Varying it needs a change.
- Fact, same as Incisive: the number check allows numbers from the whole abstract, including any part past the 5,000-byte cap that Gemma didn't see.
- The first-token clock starts when the request has been sent. Only a line with non-empty content counts as a token.

## Failure codes
Setup, re-run and not held against Gemma:
| Code | Means |
|---|---|
| `PROMPT_CUT` | Context-size error, or prompt + output reached `num_ctx` on a clean stop |
| `PROMPT_COUNT_OFF` | `prompt_eval_count` more than 25% from the byte estimate (3.85 bytes a token, Estimate) |
| `COUNTS_MISSING` | Done line without `prompt_eval_count` or `eval_count` |
| `CONTEXT_FULL` | `done_reason` length, prompt + output reached `num_ctx` |
| `NUM_PREDICT_LIMIT` | `done_reason` length, `eval_count` reached `num_predict` |
| `TIME_CONNECT`, `TIME_FIRST_TOKEN`, `TIME_GAP`, `TIME_WHOLE` | Each timer |
| `CONNECT_FAILED` | Connection refused or address not found |
| `PROXY_CUT` | HTTP 502 or 504, broken framing, or the stream ended with no done line |
| `SERVER_BUSY` | HTTP 503 |
| `ACCESS` | HTTP 401, 403 or 404 |
| `OUT_OF_MEMORY_OR_CRASH` | HTTP 500, or a runner or memory error in the stream |
| `SCHEMA_REJECTED` | HTTP 400 or a stream error naming the format |
| `INVALID_JSON_CLEAN_FINISH` | `done_reason` stop, but the reply doesn't read as the answer |
| `STREAM_BAD_LINE`, `STREAM_ERROR_OTHER`, `HTTP_OTHER`, `DONE_REASON_OTHER` | Anything else unexpected |
| `NCBI_FAILED`, `NCBI_NO_ABSTRACTS` | Studies not fetched; Gemma not called |
| `UNLOAD_FAILED` | Logged at the end, not per run |

Input limit, stated with the result: `THIN_INPUT`. Set on every run, because no stored run holds reasons or notes. Counts of missing abstracts and records are in the result.

Against Gemma, after review: `RUNAWAY_SUSPECTED` (the last 64 bytes repeat 3 or more times at the limit), `EMPTY_OUTCOMES`, `EMPTY_STUDIES`, `EMPTY_NUMBERS` (no numbers in any outcome; a count of outcomes without numbers is also saved).

Report labels the program can't set: fallback in use and invalid JSON with no grammar (no fallback is built), contaminated run, scorecard items, and `truncating input prompt` in the server log. Those need a person or the server log.

## Reading a result back
`stage0-runner print <label>` prints `S0P <label> <n>/<total> <base64>` lines and a last line `S0END <label> lines= b64_chars= crc32= gz_bytes= result_bytes=`. Join the base64 in order, check the CRC-32 of that text, base64-decode, gunzip. The result is JSON: counters, codes, check counts and the raw reply. It holds no question or abstract. It does hold PMIDs of missing records.

## Tests
Failing tests came first for the prompt, XML, efetch reading, Ollama call, checks, job, print and run steps. The NCBI client's tests were written together with its code.
