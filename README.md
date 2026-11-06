# NDSTRL

**See what an agent job will cost before it runs, steer it while it runs, and stop one task from eating your week.**

NDSTRL is a local MCP server that sits between your coding agent (Claude Code in the terminal or Claude Desktop, Codex, anything that speaks MCP) and your repo. It holds the job you gave the agent (the goal, the files it may touch, the tests that define done, the budget) and enforces it from outside the model, while metering real usage against your 5-hour and weekly plan windows.

One Rust binary. No prompt packs, no skills to maintain, no new editor.

## Why this exists

> I'm tired of realizing halfway through an Opus 5.5 task that it's using far too many tokens for the theoretical complexity of the task, and that it's going to take far too long. I want a way to steer the agents, see the expected usage, and better monitor and report my 5-hour and weekly usage based on my workflows.
>
> — Jay ([@swperb](https://github.com/swperb)), who built NDSTRL

Agents are good at code now. What hurts is not being in control of the run: a small fix that turns into an hour of re-reading files, a session that quietly wanders outside the task, one question to the frontier model that takes a visible bite out of the week, and no way to know any of this until it has already happened.

NDSTRL's answer is to make the run visible and bounded:

| Problem | What NDSTRL does |
| --- | --- |
| You can't tell what a task will cost | **Auto mode** drafts the job from your goal (likely files, checks, route) with a cost range from your own past jobs, and asks you to approve it, with what widening the scope would buy and cost |
| You notice the overspend halfway through | Every tool result carries the live spend, plan windows, and context size. The agent must **ask before spending into the last 25%** and is **stopped at the cap** |
| You can't steer without stopping it | The **NDSTRL window** shows what the agent is doing and lets you answer its questions, skip a step, or send it a message that arrives with its next tool result |
| Small fixes sprawl | Edits outside the job's paths become questions. Optional caps on files, changed lines and minutes |
| Long sessions rot | Past a context ceiling (160k by default), NDSTRL writes a handoff note and stops the session; a fresh session resumes from the note with spend carried over |
| "Fixed it" but broke something else | Checks run on your machine; a `guard` suite must pass too; a pass **closes edits** so the agent can't polish forever |
| The expensive model does chores | `delegate` hands look-ups, docs research and decided edits to a helper on a cheaper model, across your subscriptions (Claude, ChatGPT/Codex), picking whichever plan has the most room |
| You don't know which workflows eat your week | Every job writes a receipt; `ndstrl report` rolls them up by project with points of your 5-hour and weekly windows used |

## Install

Requires Rust and Claude Code (Codex optional).

```bash
git clone https://github.com/swperb/ndstrl && cd ndstrl
cargo build --release
./target/release/ndstrl install          # Claude Code + Claude Desktop's Code tab, and the status line
./target/release/ndstrl install codex    # optional: Codex CLI
```

Then start a new session and give it a goal:

> Fix the save button on the settings page.

NDSTRL drafts the job and asks you to approve it:

```
Draft job: Fix the save button on the settings page.
Scope: src/settings/save.rs, src/settings/form.rs, src/settings/mod.rs, src/settings/store.rs, src/settings/validate.rs, src/ui/layout.rs
Checks: cargo test -q
Route: look-ups to Haiku, edits to Sonnet, hard problems to Opus, each sent to whichever of your plans is least used; Fable off.
Estimate: $0.30 to $0.90 at API prices (rough: not enough past jobs yet). Cap set at $0.90. Up to 7 files, 200 changed lines.
Widening could help:
- Approve + wider scope: Allow all of src/settings/, src/ui/ (9 files). Helps if the fix spans sibling files; the draft only includes the closest matches. (estimate rises to $0.40 to $1.25)
- Approve + allow test edits: Let the agent change tests/settings.rs, tests/billing.rs, tests/ui.rs too. Only if the tests themselves are wrong; editing tests to make them pass hides bugs. (no extra cost)

[Approve]  [Approve + wider scope]  [Approve + allow test edits]  [Cancel]
```

(That is real output for the bench's save-button project. Drafting reads the repo locally and uses no model tokens.)

Or state the limits yourself and it starts right away:

> Fix the save button on the settings page. Only touch src/settings. Run the full test suite before you call it done. Stop at $2.

Open the window to watch and steer:

```bash
cargo run --release -p ndstrl-desk -- path/to/your/repo
```

See where your usage goes:

```bash
ndstrl plans     # each plan, how full, where each kind of work goes right now
ndstrl report    # last 7 days: cost and plan-window points by project, costliest jobs
```

## How it works, briefly

1. **The job** lives in `.ndstrl/job.toml`: goal, editable paths, check command, budget, caps. The agent works through NDSTRL's tools (`search`, `read`, `edit`, `run_checks`, `delegate`, `ask`, …), and every rule is enforced on every call. Nothing is a prompt the model can forget.
2. **Metering** reads the agent's real token usage from the Claude Code session transcript and prices it per model at API list prices. **Plan windows** (5-hour and weekly) come free from Claude Code's status line and from Codex's own session logs.
3. **Questions** (out-of-scope edits, budget, size, web access, the auto-mode draft) go to you directly when the client supports MCP elicitation, or through the agent otherwise, or from the window. Every answer is recorded with who gave it.
4. **Helpers** run headless on the cheapest plan with room. Their edits are checked with git; anything outside the job's scope is put back.
5. **Receipts** record the cost, the files, the checks, and every decision. They feed `ndstrl report` and auto mode's estimates.

Details: [docs/how-it-works.md](docs/how-it-works.md). Every command, tool and spec field: [docs/reference.md](docs/reference.md).

## Does it actually help? The bench

NDSTRL measures itself. `ndstrl bench` runs fixed jobs through real headless Claude Code sessions (the same engine as Claude Desktop's Code tab), once with no steering and once through NDSTRL, and scores both: did it pass, was the fix faithful to the codebase (no forked logic, no edited tests, nothing outside scope), what it cost, how many turns.

The full method, every run, and what the numbers do and do not show: **[docs/benchmarks.md](docs/benchmarks.md)**.

First results (12 real sessions, 3 jobs, 2 trials each):

| | Faithful result | Cost per job | Turns |
| --- | --- | ---: | ---: |
| Plain Claude Code | 5 of 6 | $0.118 | 5.0 |
| Through NDSTRL | **6 of 6** | $0.167 | 8.5 |

- **Trust:** in one trial, plain Claude Code "fixed" the trap job by quietly forking the tax rate to get around an off-limits module, and reported success. Through NDSTRL the agent asked instead, both times.
- **Cost:** on these small, well-specified jobs NDSTRL is overhead, about +$0.05 per job, mostly from extra tool round trips. Cutting that overhead, and benching the long tasks NDSTRL is meant for, are the next steps.

## Status

Working and used daily by its author, on macOS. Claude Code is the main agent it is tested with; Codex works as a helper. Windows is untested in this version. Cursor helpers are not wired up yet.

## Layout

```
crates/core     the rules: job spec, scope, checks, metering, plan windows, routing, auto mode
crates/ndstrl   the binary: MCP server, CLI, status line, bench
crates/desk     the window (Dioxus)
bench/parts     bench jobs: small repos with a known bug, a spec, and a faithfulness check
bench/results   every bench run, as run
docs/           how it works, reference, benchmarks
```

## License

Apache-2.0
