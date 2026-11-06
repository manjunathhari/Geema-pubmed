# Gemma Quick

Evidence answers in the style of Incisive Quick, written by an open Gemma model, using open literature APIs.

## How the owner wants work done
- Plain words, short replies, phone-friendly.
- Change only what the owner names, in small batches: one or two changes at a time, with the code shown first.
- Mockup first for any UI, frozen on his phone.
- Brief, then builder subagent, then independent reviewer subagent, then fixes.
- Keep `docs/build-log.md` and `docs/change-log.md` current.
- Keep the project board current, so the owner can check progress without stopping the build.
- Merge only on his "merge it".
- For server work, one one-line command at a time.
- Discuss any engine change first, however small. Writing it into a plan doesn't count as asking.
- Flag, never delete. Don't pile on guardrails.
- Evidence language: never overstate. Keep estimates labelled, and say what wasn't checked.

## This repo is public
Never commit exported runs, questions, replies, secrets, server names or account ids. Those stay in the Project Brief, outside the repo. Stage files by explicit path, never `git add -A`.

## Process
Follows the 10x process in `.10x/`. No product code before the owner approves the design. Agents are in `.claude/agents/`.
