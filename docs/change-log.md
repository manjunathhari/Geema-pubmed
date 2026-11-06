# Change log
## 2026-11-06
- Added project instructions, agents, records and logs. No product code.
- Added Stage 0 design and ADR-001. Wider ignore rules for output files. No product code.
- Added `stage0/`: the `stage0-runner` test program, copied Incisive writer code at 54433fc, mock-server tests, a Dockerfile and a compose example with placeholders. Not merged.
- Stage 0 prompt now has Incisive's source rule. Jobs take optional `searches` and `seed`, and results record them. Results also keep the cleaned answer, untraced spans and build commit. Runaways with rising numbers are labelled as runaways, and unclear limit hits as `NUM_PREDICT_NEEDS_READING`. Studies PubMed doesn't return are left out. `print` reads earlier results. Not merged.
- Recorded the provisional Stage 0 result. No code change.
