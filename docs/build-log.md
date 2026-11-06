# Build log
## 2026-11-06
- Project started under the 10x process.
- 12 agents written in `.claude/agents/`.
- Reference code read at Incisive 54433fc, unchanged.
- Owner chose Rust for the app and the Stage 0 test script.
- No product code yet.
- Owner chose Stage 0 first, his own runs only, all automatic fails, and the pass mark. SciSpace left out. Repo stays public.
- Read-only server check passed through Hostinger, then removed.
- Stage 0 design and ADR-001 written for approval.
- Owner approved the design. Stage 0 runner built on branch `stage0/runner`, in `stage0/`. Not run on the server yet. Details in `.10x/decisions/sde/stage0-writer-test.md`.
- Departures from the build brief: search log also says "not available"; Ollama call uses `std::net`, not `ureq`, so a timer can close the socket; a small XML reader instead of `quick-xml`.
- Code review of `8197988` fixed on `stage0/runner`: source rule, search log per job, seed per job, runaway label, cleaned answer saved. M1 in part, M3 and M4 too. M2, the rest of M1 and the Low findings left. Not merged. Details in `.10x/decisions/sde/stage0-writer-test.md`.
- Departures: `NUM_PREDICT_LIMIT` replaced by `NUM_PREDICT_NEEDS_READING`; `searches: []` refused; a loop at `num_ctx` also labelled runaway.
- Stage 0 runs finished. Provisional result, scored by Claude at the owner's request: the model did not pass the automatic-fail rules on any of the 3 questions. Results stay outside this repo.
