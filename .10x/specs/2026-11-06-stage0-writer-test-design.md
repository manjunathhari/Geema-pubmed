# Stage 0 writer test: design for approval

Status: awaiting the owner's approval. Throwaway test, not the app.

## Question
Can Gemma 3 4B, given the studies Opus used, write an answer as refined as Incisive Quick?

## Owner decisions so far
- Stage 0 runs before any app is built.
- Rust for the test program and the app.
- Only the owner's own stored runs: the hernia run if its format fits, plus 2 topics he picks.
- All 11 automatic fails apply. One bad repeat fails that question.
- Pass mark: average 4 of 5, none below 3, within 1 point of the Claude answer on each question.
- SciSpace is left out for now. Repo stays public, so nothing private enters it.

## How it runs
1. Runs as a separate Docker project on the Gemma server, started through the Hostinger connector. The Gemma project isn't edited or restarted.
2. It reaches Ollama over the internal Docker network, so it needs no password and doesn't pass through the proxy.
3. Source lives in this repo. The server builds it from a pinned commit. Only source goes in the repo.
4. The server fetches abstracts from PubMed by PMID. The question goes into the project's settings, and that project is deleted after the test.
5. Results are written to a Docker volume, outside the repo. They are read back through project logs, up to 300 entries at a time, compressed. The volume is deleted after scoring.
6. The owner runs one read-only query and attaches the result. That gives the question, PMIDs and Opus answer. These stay off the server, apart from the question and PMIDs.

## Probes first
Made-up text only. They settle these before any real question is sent:
- The full answer format works.
- A prompt that is too long gives an error instead of a silent cut.
- The memory fit at `num_ctx` 40960. Estimate: about 4.5 GB in use.
- Real reading and writing speed on a long prompt.

If any fails, stop and report. Incisive chat may fail for about 10 minutes during these (estimate).

## Gemma settings
`num_ctx` 40960, `num_predict` 12288, `truncate` false, `shift` false, `temperature` 0.2, `top_k` 64, `top_p` 0.95, `repeat_penalty` 1.0, a fixed seed per repeat, streaming on.

## Prompt
Same writer rules and study layout as Incisive at 54433fc. Differences, all stated in the results:
- One user message, because this model has no real system role.
- The answer format is also written into the prompt, because Ollama doesn't show it to the model.
- Researcher reasons, design labels and contradiction notes aren't stored. The prompt says "not available", not "(none)".
- Abstracts are fetched fresh and may differ slightly from those Opus saw.

Field order stays the same as the Opus format, even though the verdict comes before the evidence rows.

## Scoring
Code checks: citations, numbers, valid answer. Claude's Clinical Evidence Agent does a first check, which is a declared conflict of interest. The owner rules blind on each serious fault. Failures of setup (cut prompt, time limit, memory) are re-run and counted separately, not held against Gemma.

## What it can't show
- Three questions are a small sample. Zero fails still fits a true failure rate of about 63%.
- A pass means worth testing further, not as good as Claude.
- It says nothing about finding studies. That is Stage 1.

## Not checked
Everything on the probe list, Quick answer length, and the stored runs themselves.
