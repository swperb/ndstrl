# How NDSTRL works

NDSTRL is a local MCP server. Your coding agent (Claude Code in the terminal or in Claude Desktop, Codex, anything that speaks MCP) connects to it, and from then on the agent does its work through NDSTRL's tools instead of its own. That one position, between the agent and your repo, is what lets NDSTRL see and steer everything the agent does.

```
 you ──goal + limits──▶ agent (Claude Code, Codex, …)
                          │  MCP tools: job_open, search, read, edit, run_checks, delegate, …
                          ▼
                       NDSTRL  ── enforces the job spec, meters spend, writes receipts
                     ╱    │    ╲
              your repo  checks  helpers on other plans (Haiku, Sonnet, Codex)
```

Nothing here is a prompt the model can forget. Every rule lives in the server and is applied to each tool call.

## 1. The job

Every piece of work is a **job**. Its spec lives at `.ndstrl/job.toml` in your repo:

```toml
goal = "On the settings page, the Save button stays disabled after an edit. Fix it."
allow = ["src/settings"]              # paths the agent may edit
deny = ["src/billing", "src/ui"]      # paths it may never edit
test = "cargo test -q"                # exit 0 means the job is done
guard = "cargo test -q --workspace"   # must also pass: no breaking things elsewhere
budget_usd = 2.0                      # at API prices
max_files = 3
max_context_tokens = 160000
```

The agent creates the job by calling `job_open` with what you told it ("Only touch src/settings. Stop at $2."). In **auto mode**, you give only a goal and NDSTRL drafts the rest (section 7).

The spec is plain TOML. Commit it and the same job can be picked up by a different agent on a different machine; that is the point of keeping it outside any one tool.

## 2. Scope: the agent cannot quietly leave the job

`edit` and `write` check every path against `allow` and `deny`:

- Inside scope: applied, and the change is recorded (+/− lines).
- Outside scope: not applied. It becomes a **question** for you: "The agent wants to edit `src/billing/mod.rs`, which this job keeps off limits. Allow it for this job?"
- Secrets (`.env`, keys) and `.git` are never readable or editable; `.ndstrl/` is never editable by the agent.

While a question is open, every paid step is blocked. Reads still work so the agent can explain itself.

**Who answers.** If the client supports MCP elicitation, NDSTRL asks *you* directly, bypassing the model, so the model cannot invent your answer. Otherwise the agent is told to stop and relay the question, and the receipt records which path each answer took. You can also answer from the NDSTRL window.

## 3. Budget: real usage, not a guess

NDSTRL meters the job from the agent's own records:

- **Claude Code** writes a transcript (JSONL) per session and tells the MCP servers it starts which session they belong to (`CLAUDE_CODE_SESSION_ID`). NDSTRL reads the real token counts from it (input, output, cache writes, cache reads; one count per API response) and prices them per model at Anthropic's list prices.
- **Helpers** report their own usage when they finish.
- **Anything else** falls back to NDSTRL's own estimate of what its tools returned.

On a subscription you do not pay per token, so the dollar figure is labelled "at API prices". It is a consistent unit for budgets, not a bill.

The budget gate runs before every paid step:

- Below 25% of the budget left (configurable), the agent must ask before continuing.
- At the cap, every paid step is blocked and a receipt is written.

## 4. Your plan windows: 5-hour and weekly

Claude Code passes its status-line command your plan usage (`rate_limits.five_hour` and `seven_day`, each with a used percentage and a reset time), computed from response headers it already receives. `ndstrl install` makes `ndstrl statusline` your status line (it never replaces one you already have), so every reading is logged to `~/.ndstrl/` at no cost: no extra requests, no credentials.

From those readings NDSTRL shows, on every tool result and in the window:

- how full each window is and when it resets,
- how many points of each window *this job* has used,
- a projection between readings, from a learned "plan points per API dollar" rate (marked `~`).

A job can be capped in plan terms (`budget_plan_5h_pct`, `budget_plan_7d_pct`): "stop this job at 5% of my 5-hour window".

For Codex, NDSTRL reads the same kind of numbers (`used_percent`, `resets_at`) from Codex's own session logs.

`ndstrl report` rolls finished jobs up by project: what each cost, how many points of your windows they used, and which jobs were the most expensive, so you can see which workflows eat your week.

## 5. Context rot

Long sessions drift: the model re-reads more, follows stale assumptions, and fixes things that were not broken. NDSTRL watches the agent's live context size (input + cache read + cache write on its latest response, from the transcript).

Past `max_context_tokens` (default 160k), NDSTRL:

1. writes `.ndstrl/handoff.md`: the goal, the limits, what changed so far, where the checks stand, and every decision you made;
2. blocks the rest of that session;
3. tells you to start a fresh session and say "resume the ndstrl job".

The fresh session gets the handoff in its very first tool result, so it starts from the note instead of re-exploring the repo. Spend from the old session carries over, so the budget stays honest.

Related guards keep small jobs small:

- `max_files` and `max_changed_lines`: crossing either becomes a question.
- `max_minutes`: asks before the job runs past its time.
- `guard`: a broader command, usually the full test suite, that must pass before the job counts as done. "Fixed the button, broke checkout" fails.

## 6. Done means done

`run_checks` runs the job's test command on your machine and returns pass/fail with a short excerpt of the failures, so the model does not spend tokens reasoning about what a compiler can tell it. A pass marks the job done and **closes edits**. The agent cannot keep polishing after green; it is told to write the receipt and stop.

If the agent edits after a check run, the result is marked **stale** everywhere, so nobody trusts an old pass.

## 7. Auto mode

Give only a goal ("Fix the save button on the settings page") and `job_open` drafts the job before anything runs, using no model tokens:

- **Scope:** files ranked by how strongly they match the goal's words (path matches count most).
- **Checks:** guessed from the project (`cargo test`, `npm test`, `pytest`, `go test`).
- **Route:** where helper work will go.
- **Estimate:** a cost range. From your past jobs of similar size once there are enough; a rough rule from the scope's size until then. The cap is set at the high end.
- **What widening would buy**, each with its cost, for example:
  - "Approve + web research: let helpers read the serde docs online. More robust when the fix depends on a library's current behavior. About $0.01 to $0.05 per lookup."
  - "Approve + wider scope: allow all of `src/settings/` (5 files). Estimate rises to $0.20 to $0.50."
  - "Approve + Fable for the hard part", only when the goal sounds hard, with a warning about what it costs.

You pick Approve, an approve-with option, or Cancel, in the chat. Edits wait until you answer.

## 8. Helpers across your subscriptions

The agent you are talking to keeps the judgment. Everything else can be handed off with `delegate`:

| kind | what it is for | default model |
| --- | --- | --- |
| `lookup` | find and summarize code, read-only | Claude Haiku |
| `research` | read docs on the web (needs web allowed) | Claude Haiku |
| `edit` | apply a change that is already decided | Claude Sonnet |
| `hard` | debugging or design | Claude Opus (Fable only if you allow it) |

The helper runs headless with a short brief and must answer in a few lines, so a look-up costs the main session a dozen lines of context instead of the files it read.

**Which subscription.** Each plan gets a *room* score: the unused share of its fuller window times a size weight for its tier (Claude Max 5x = 5, ChatGPT Free = 0.2, Plus = 1, Pro = 6; rough defaults you can override in `~/.ndstrl/plans.toml`). Work goes to the plan with the most room, so no plan runs out while another sits idle. Free plans only get look-ups. `ndstrl plans` shows the current picture.

**Helper edits are verified, not trusted.** Before a helper that can change files runs, NDSTRL snapshots the repo with git. Afterwards, changes inside the job's scope are recorded like any other edit; anything outside it is put back exactly as it was (including files you were already editing). Look-up helpers may change nothing.

## 9. Receipts

Every job ends with `.ndstrl/receipts/<job>.md` and `.json`: the goal, the agent, spend (tokens by type and dollars at API prices), every file changed with +/− lines, every check run, every question and how it was answered, and every refusal. The same numbers feed `ndstrl report` and auto mode's estimates.

## 10. The window

`ndstrl-desk` is a small native window (Rust, Dioxus) that reads the same files the server writes:

- what the agent is doing and whether it needs you,
- the open question, with its options and a **Skip this** button (decline it; the agent drops that piece and carries on),
- a **message box**: what you type reaches the agent with its very next tool result ("MESSAGE FROM THE USER: …"), so you can steer, narrow, or redirect it mid-run without switching to the chat. If the agent asked you something, what you type is your answer;
- the budget bar, the files changed, your plan windows,
- the **decisions** made so far, and whether you answered directly or the agent relayed it.

The agent is told to `ask` whenever it is unsure a step fits the job or the budget, so those questions land here too.

It computes every number with the same code as the server, so they cannot disagree, and it never changes the job itself: what you click or type is queued in `.ndstrl/inbox.jsonl` for the server to apply.

## What NDSTRL cannot do

- It cannot stop an agent from using its own built-in edit tools. It is the contract the agent is told to work through, and the bench measures how well that holds.
- Dollar figures are list-price equivalents, as precise as the usage the harness writes down.
- Plan percentages depend on Claude Code's status line (for Claude) and Codex's session logs (for Codex). Without a recent reading, NDSTRL says so.
- Windows has no OS sandbox in this version; scope is enforced by NDSTRL's tools.
- Cursor helpers are not wired up yet.
