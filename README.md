# NDSTRL

A software factory for coding agents. The part stays the same; the machines on the floor can change.

You send the factory a part: a goal, a repo, a scope, the tests that define done, and a budget. The factory holds that contract and enforces it from the server side, so the same job runs on Claude Code today and Codex tomorrow without rewriting skills or prompts.

`ndstrl` is one Rust binary that speaks MCP over stdio. No skills, no prompt packs, no new editor.

## Install

```bash
cargo build --release
./target/release/ndstrl install          # Claude Code + Claude Desktop Code tab (~/.claude.json)
./target/release/ndstrl install codex    # Codex CLI (~/.codex/config.toml)
./target/release/ndstrl install all --dry-run
```

Then start a new session and say what you want, with a budget:

> Fix the failing invoice test. Don't touch billing. Stop at $4.

## What the factory enforces

| Rule | How |
| --- | --- |
| Scope | `edit`/`write` refuse paths outside `allow` or inside `deny`. Leaving the drawing becomes a QUESTION, never a silent edit. |
| Ask, don't guess | Questions go to the human directly over MCP elicitation when the client supports it; otherwise the model must relay them, and the receipt records which. |
| Budget | Real spend is read from the Claude Code transcript (`CLAUDE_CODE_SESSION_ID`), priced per model. Below `ask_below` (25%) of the budget, the next step asks. At the cap, every paid tool blocks and the receipt is written. |
| Cheap reads | `search`/`read`/`list_files` return capped excerpts, never the tree. Secrets and `.git` are unreadable. |
| Local critic | `inspect` runs the test command on the machine and returns a short failure excerpt. |
| Stop at green | A passing inspection marks the part done; edits are refused after that. |
| Receipts | `.ndstrl/receipts/<job>.md` and `.json`: spend, files with +/- lines, inspections, questions, denials. |

The traveler lives at `.ndstrl/job.toml` (commit it, the part travels). Live state and an event ledger sit beside it for the front office to watch.

## Layout

- `crates/core`: traveler, scope, reads, critic, meter, job engine
- `crates/ndstrl`: the binary (MCP server and CLI)

## Not promised

The factory cannot stop a harness from using its own built-in edit tools; it is the contract machines are told to work through. Windows gets no OS sandbox in week one: the allowlist is enforced by the factory tools. A dollar cap is only as precise as the usage the harness writes down.

License: Apache-2.0
