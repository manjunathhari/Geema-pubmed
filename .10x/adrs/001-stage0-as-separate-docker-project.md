# ADR-001: Run Stage 0 as a separate Docker project

**Status:** Accepted
**Date:** 2026-11-06
**Feature:** stage0-writer-test
**Author:** 10x-Team (Architect + Staff Engineer)

## Context
Claude's workspace can't reach the Gemma address directly. The server's web terminal mangles long pastes. The owner asked Claude to connect through Hostinger. A small read-only check confirmed that a separate Docker project can reach Ollama on the internal network, and that its logs can be read through the connector.

## Decision
Build and run the Rust test as its own Docker project, created through the Hostinger connector, from a pinned commit of this repo. Never edit or restart the Gemma or proxy projects. Delete the project and its data after the test.

## Alternatives Considered
| Alternative | Pros | Cons | Why Not |
|---|---|---|---|
| Owner runs terminal commands | No write access for Claude | Slow, needs the owner, pastes break | The owner asked for a direct connection |
| Binary from a repo branch | No build on the server | Can't be audited, stays in public history | Source build is cleaner |
| GitHub release | Tidy artifact | Most steps; the owner has to create the release | Too heavy for a throwaway test |
| Run elsewhere with the password | Keeps work off the server | Needs a reachable machine and the password in use | Internal route needs no password |

## Consequences

### Positive
- The owner needs no terminal steps. The password isn't used.

### Negative
- Claude can write to a live shared server.
- Results must come back through the last 300 log entries, so large results need compressing and paging.

### Risks
- Creating a project with an existing name replaces it. Use only names starting `gemma-lab-`.
- A build beside a loaded model could use up memory, and the server has no swap. Build before the model loads, with limits.
- Incisive chat may fail during long Gemma calls.

## Dependencies
- Owner approval of the Stage 0 design.
- Does not constrain the Stage 3 hosting decision.
