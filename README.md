# NDSTRL

One job contract for every coding agent you use. Stop burning a week of plan usage on one question.

You tell your agent what you want, the limits, and a budget. NDSTRL is an MCP server that holds that job and enforces it from outside the model, so it holds whether the agent is Claude Code today or Codex tomorrow:

- **Scope:** edits outside the job's paths turn into a question, never a silent change.
- **Budget:** real spend is read from the session transcript and priced per model. Plan usage (5-hour and weekly windows) comes from Claude Code's status line at no cost. The agent is asked before it spends into the last 25%, and stopped at the cap.
- **Context rot:** past 160k tokens of context (configurable), NDSTRL writes a handoff note and stops the session. A fresh session resumes the job from the note, spend carried over.
- **Blast radius:** optional caps on files and changed lines, so a small fix stays small.
- **Done means done:** checks run on your machine, not in the model. A passing run closes edits. An optional `guard` command (your full suite) must pass too, so a fix here cannot break something there.
- **Cheap reads:** search and read return short excerpts, never whole trees. Secrets and `.git` are unreadable.
- **Helpers across your subscriptions:** the agent hands look-ups, docs research, and already-decided edits to a helper on a cheaper model with `delegate`. Each piece goes to whichever of your plans (Claude, ChatGPT/Codex, Cursor) is least used, so none runs out while another sits idle. Fable is off unless you allow it. Helper edits are checked with git; anything outside the job's scope is put back.
- **Auto mode:** give only a goal and NDSTRL drafts the job (likely files, checks, route, a cost range from your past jobs) plus what widening it would buy, for example web research on a library's docs, with the extra cost. You approve in the chat before anything runs.
- **Receipts:** every job writes what it cost, what it touched, and what you decided.

One Rust binary. No skills, no prompt packs, no new editor.

## Install

```bash
cargo build --release
./target/release/ndstrl install          # Claude Code (CLI and the Desktop Code tab) + status line
./target/release/ndstrl install codex    # Codex CLI
```

Then start a session and say what you want. With just a goal, NDSTRL drafts the job and asks you to approve it:

> Fix the save button on the settings page.

Or spell out the limits yourself:

> Fix the save button on the settings page. Only touch src/settings. Max 3 files. Run the full test suite before you call it done. Stop at $2.

## The job spec

Lives at `.ndstrl/job.toml`. Commit it if you want the job to move between machines or agents.

| Field | Meaning |
| --- | --- |
| `goal` | What done means, in your words |
| `allow` / `deny` | Paths the agent may and may not edit |
| `test` | The check command; exit 0 means done |
| `guard` | A broader command that must also pass |
| `budget_usd` | Cap at API prices (a usage proxy on subscription plans) |
| `budget_plan_5h_pct` / `budget_plan_7d_pct` | Cap in points of your plan's windows |
| `max_files` / `max_changed_lines` / `max_minutes` | Ask before the job grows past these |
| `max_context_tokens` | Hand off to a fresh session past this (default 160000) |
| `tools` | Extra tools, e.g. `["web"]` for docs research |
| `allow_frontier` | Let hard problems use Fable (off by default) |

## The window

`ndstrl-desk [repo]` is one screen: what the agent is doing, whether it needs your answer (answer right there), what it has cost, which files changed, and how much of your plan is left.

```bash
cargo run --release -p ndstrl-desk -- path/to/repo
```

## Bench

`ndstrl bench` runs fixed jobs in `bench/parts/` through real headless Claude Code sessions, once plain and once through NDSTRL, and scores them: good (green and faithful to the codebase, or correctly stopped), tokens, cost, turns, files out of scope, questions asked.

## Not promised

NDSTRL cannot stop an agent from using its own built-in edit tools; it is the contract the agent is told to work through. Codex and Cursor helpers need their CLIs installed and signed in; until then routing uses Claude only. Windows gets no OS sandbox; the scope is enforced by NDSTRL's tools. Costs are only as precise as the usage the agent's harness writes down.

License: Apache-2.0
