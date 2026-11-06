# Reference

## CLI

| Command | What it does |
| --- | --- |
| `ndstrl` | Serve MCP on stdio. This is what your agent launches. |
| `ndstrl install [claude\|codex\|all] [--dry-run]` | Register NDSTRL with Claude Code (CLI and the Desktop Code tab, via `~/.claude.json`) and set up the status line; or with Codex (`~/.codex/config.toml`). Backs up anything it edits and never replaces an existing status line. |
| `ndstrl plans` | Your AI plans, how full each is, its size weight, and where each kind of work would go right now. |
| `ndstrl report [--days N]` | Finished jobs from the last N days (default 7): cost at API prices, points of your plan windows used, by project, and the costliest jobs. |
| `ndstrl status` | The current repo's job, one line. |
| `ndstrl init "<goal>"` | Write a starter `.ndstrl/job.toml` you can edit by hand. |
| `ndstrl statusline` | The Claude Code status line command. Logs plan usage to `~/.ndstrl/`, prints your windows and the job's gauges. |
| `ndstrl bench [job..] [--mode plain\|ndstrl\|both] [--trials N] [--model M]` | Run bench jobs through real headless Claude Code sessions, plain and through NDSTRL, and score them. |
| `ndstrl-desk [repo]` | The NDSTRL window. |

## MCP tools

Every result ends with a status line: spend against the cap, plan windows, context size, files touched, check status, done, and any open question.

| Tool | Purpose |
| --- | --- |
| `job_open` | Start or resume the job. Goal only: auto mode drafts it for approval. Goal plus limits: starts it directly. Nothing: resumes the job in `.ndstrl/job.toml` (and hands over a handoff note if there is one). |
| `job_status` | The spec, gauges, files touched, open questions, last check run. |
| `list_files`, `search`, `read` | Look at code. Short excerpts only (search caps at 60 hits, read at 250 lines). |
| `edit`, `write` | Change code. Refused outside scope (becomes a question), after checks pass, or at the cap. |
| `run_checks` | Run the job's test command (and `guard`, if set) on your machine. A pass closes edits. |
| `delegate` | Hand a `lookup`, `research`, `edit`, or `hard` task to a helper on the plan with the most room. |
| `ask` | Ask you something instead of guessing. |
| `answer` | Record your answer to an open question (only with your words). |
| `receipt` | Write the job's receipt. |

## Job spec (`.ndstrl/job.toml`)

| Field | Default | Meaning |
| --- | --- | --- |
| `goal` | required | What done means, in your words |
| `allow` | whole repo | Paths the agent may edit (a bare directory means everything under it) |
| `deny` | none | Paths it may never edit, even inside `allow` |
| `test` | none | Check command; exit 0 means done |
| `guard` | none | Broader command that must also pass for done |
| `budget_usd` | none | Cap at API prices |
| `budget_tokens` | none | Token cap, for agents whose cost cannot be read |
| `budget_plan_5h_pct`, `budget_plan_7d_pct` | none | Cap in points of your plan's windows |
| `ask_below` | 0.25 | Ask before continuing once less than this fraction of the budget is left |
| `max_files`, `max_changed_lines` | none | Ask before the job grows past these |
| `max_minutes` | none | Ask before the job runs longer |
| `max_context_tokens` | 160000 | Hand off to a fresh session past this context size |
| `tools` | `[]` | Extra tools, e.g. `["web"]` for helper research |
| `allow_frontier` | false | Let `hard` helpers use Fable |
| `check_timeout_secs` | 600 | Kill a check run after this long |

## Files NDSTRL writes

In your repo, under `.ndstrl/` (state, ledger and inbox are git-ignored automatically):

| File | Contents |
| --- | --- |
| `job.toml` | The job spec |
| `state.json` | Live state: usage, files touched, check runs, questions, notes, helper runs |
| `ledger.jsonl` | Every event, in order |
| `inbox.jsonl` | Answers and messages from the window, waiting for the server |
| `handoff.md` | Written when a session is handed off for context size |
| `receipts/<job>.md`, `.json` | One per finished job |

In `~/.ndstrl/` (override with `NDSTRL_HOME`):

| File | Contents |
| --- | --- |
| `plan.json`, `plan.jsonl` | Latest Claude plan reading and its history (from the status line) |
| `history.jsonl` | One line per finished job, for estimates and `ndstrl report` |
| `plans.toml` | Optional size overrides, e.g. `claude = 5.0`, `codex = 1.0` |

## What NDSTRL reads, and what it never does

- **Reads:** your repo (through its tools, excluding secrets and `.git`); the Claude Code transcript of the session it runs in (token counts and model names); `~/.claude.json` for your plan tier only; Codex's session logs for its rate-limit numbers; the status-line data Claude Code hands it.
- **Never:** reads or sends credentials; makes network requests of its own; sends anything anywhere. Helpers are run through the CLIs you already have signed in.

## Environment variables

| Variable | Effect |
| --- | --- |
| `NDSTRL_HOME` | Where plan readings, history and overrides live (default `~/.ndstrl`) |
| `NDSTRL_TRANSCRIPT` | Meter this transcript file instead of discovering one |
| `NDSTRL_CODEX` | Path to the `codex` binary if it is not on PATH or in `ChatGPT.app` |
| `CLAUDE_CONFIG_DIR`, `CODEX_HOME` | Respected the same way Claude Code and Codex do |
