//! Tool surface. Schemas are the contract any harness can discover; the
//! job engine is what enforces them.

use anyhow::{Context, Result};
use ndstrl_core::{Job, Outcome, JobSpec};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn definitions() -> Value {
    let ro = json!({ "readOnlyHint": true, "openWorldHint": false });
    let rw = json!({ "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false });
    json!([
        {
            "name": "job_open",
            "title": "Open the job",
            "description": "Start or resume the job. Give the user's goal and limits to start one, or only the repo to resume the job in .ndstrl/job.toml (for example after a handoff). Returns the editable paths, the test command, the budget, and a size estimate. Call this first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "repo": { "type": "string", "description": "Repo path. Defaults to the git root of the server's working directory." },
                    "goal": { "type": "string", "description": "What done means, in the user's words." },
                    "allow": { "type": "array", "items": { "type": "string" }, "description": "Globs that may be edited, e.g. [\"src/invoice\", \"tests/**\"]. Empty means the whole repo." },
                    "deny": { "type": "array", "items": { "type": "string" }, "description": "Globs that must not be edited, e.g. [\"src/billing\"]." },
                    "test": { "type": "string", "description": "Check command; exit code 0 means the job is done, e.g. \"cargo test invoice\"." },
                    "budget_usd": { "type": "number", "description": "Dollar cap for this run." },
                    "budget_tokens": { "type": "integer", "description": "Token cap, for agents whose cost cannot be read." },
                    "budget_plan_5h_pct": { "type": "number", "description": "Cap in percentage points of the subscription's 5-hour window, e.g. 10." },
                    "budget_plan_7d_pct": { "type": "number", "description": "Cap in percentage points of the subscription's weekly window, e.g. 2." },
                    "guard": { "type": "string", "description": "Broader command (usually the full test suite) that must also pass before the job is done, so a fix here cannot break things elsewhere." },
                    "max_files": { "type": "integer", "description": "Ask before the job changes more than this many files. A small UI fix might be 3." },
                    "max_changed_lines": { "type": "integer", "description": "Ask before the job's total changed lines pass this." },
                    "max_minutes": { "type": "integer", "description": "Ask before the job runs longer than this." },
                    "max_context_tokens": { "type": "integer", "description": "Hand off to a fresh session past this context size. Default 160000." }
                },
                "additionalProperties": false
            },
            "annotations": rw
        },
        {
            "name": "job_status",
            "title": "Job status",
            "description": "The job spec, budget gauges, files touched, open questions, and the last check run.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": ro
        },
        {
            "name": "list_files",
            "title": "List files",
            "description": "List repo files (gitignore respected), optionally filtered by a glob like \"src/**/*.rs\".",
            "inputSchema": { "type": "object", "properties": { "glob": { "type": "string" } }, "additionalProperties": false },
            "annotations": ro
        },
        {
            "name": "search",
            "title": "Search code",
            "description": "Regex search across the repo. Returns path:line: text hits, capped. Use this before reading.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Rust regex syntax." },
                    "glob": { "type": "string", "description": "Only files matching this glob." },
                    "ignore_case": { "type": "boolean" }
                },
                "required": ["pattern"],
                "additionalProperties": false
            },
            "annotations": ro
        },
        {
            "name": "read",
            "title": "Read an excerpt",
            "description": "Numbered lines from one file, at most 250 per call. Ask for the range you need.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "end_line": { "type": "integer", "minimum": 1 }
                },
                "required": ["path"],
                "additionalProperties": false
            },
            "annotations": ro
        },
        {
            "name": "edit",
            "title": "Apply an edit",
            "description": "Replace an exact string in a file. old_string must be unique unless replace_all. Refused outside the job's scope (that becomes a QUESTION), after checks pass, or at the budget cap.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_string": { "type": "string" },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" }
                },
                "required": ["path", "old_string", "new_string"],
                "additionalProperties": false
            },
            "annotations": rw
        },
        {
            "name": "write",
            "title": "Write a file",
            "description": "Create a file or replace it whole. Same scope, done, and budget rules as edit.",
            "inputSchema": {
                "type": "object",
                "properties": { "path": { "type": "string" }, "content": { "type": "string" } },
                "required": ["path", "content"],
                "additionalProperties": false
            },
            "annotations": rw
        },
        {
            "name": "run_checks",
            "title": "Run checks",
            "description": "Run the job's test command on this computer and return pass/fail with a short failure excerpt. Uses no model tokens beyond the excerpt. A pass marks the job done and closes edits.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false }
        },
        {
            "name": "ask",
            "title": "Ask the user",
            "description": "Pause and ask the user instead of guessing: picking a design, going outside the job's scope, or anything expensive or irreversible.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "question": { "type": "string" },
                    "options": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["question"],
                "additionalProperties": false
            },
            "annotations": ro
        },
        {
            "name": "answer",
            "title": "Record the user's answer",
            "description": "Record what the USER said to an open QUESTION. Only call this with the user's own reply.",
            "inputSchema": {
                "type": "object",
                "properties": { "question_id": { "type": "integer" }, "answer": { "type": "string" } },
                "required": ["question_id", "answer"],
                "additionalProperties": false
            },
            "annotations": rw
        },
        {
            "name": "receipt",
            "title": "Write the receipt",
            "description": "Write the run's receipt (spend, files, check runs, questions, refusals) to .ndstrl/receipts and return its path.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": rw
        }
    ])
}

/// The git root above `start`, or `start` itself.
pub fn default_repo(start: &Path) -> PathBuf {
    let mut p = start.to_path_buf();
    loop {
        if p.join(".git").exists() || p.join(".ndstrl").exists() {
            return p;
        }
        if !p.pop() {
            return start.to_path_buf();
        }
    }
}

fn s<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(|v| v.as_str())
}

fn strings(a: &Value, k: &str) -> Vec<String> {
    a.get(k)
        .and_then(|v| v.as_array())
        .map(|xs| xs.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn open(a: &Value, agent: &str) -> Result<(Job, String)> {
    let repo = match s(a, "repo") {
        Some(r) => PathBuf::from(r),
        None => default_repo(&std::env::current_dir()?),
    };
    let fresh = match s(a, "goal") {
        Some(goal) => {
            // Starting a new part keeps any limits already on the spec
            // unless the caller overrides them.
            let base = JobSpec::load(&repo).ok().flatten().unwrap_or_default();
            let mut t = JobSpec { id: String::new(), goal: goal.to_string(), ..base };
            if a.get("allow").is_some() {
                t.allow = strings(a, "allow");
            }
            if a.get("deny").is_some() {
                t.deny = strings(a, "deny");
            }
            if let Some(x) = s(a, "test") {
                t.test = Some(x.to_string());
            }
            if let Some(x) = a.get("budget_usd").and_then(|v| v.as_f64()) {
                t.budget_usd = Some(x);
            }
            if let Some(x) = a.get("budget_tokens").and_then(|v| v.as_u64()) {
                t.budget_tokens = Some(x);
            }
            if let Some(x) = a.get("budget_plan_5h_pct").and_then(|v| v.as_f64()) {
                t.budget_plan_5h_pct = Some(x);
            }
            if let Some(x) = a.get("budget_plan_7d_pct").and_then(|v| v.as_f64()) {
                t.budget_plan_7d_pct = Some(x);
            }
            if let Some(x) = s(a, "guard") {
                t.guard = Some(x.to_string());
            }
            if let Some(x) = a.get("max_files").and_then(|v| v.as_u64()) {
                t.max_files = Some(x as usize);
            }
            if let Some(x) = a.get("max_changed_lines").and_then(|v| v.as_u64()) {
                t.max_changed_lines = Some(x as usize);
            }
            if let Some(x) = a.get("max_minutes").and_then(|v| v.as_u64()) {
                t.max_minutes = Some(x);
            }
            if let Some(x) = a.get("max_context_tokens").and_then(|v| v.as_u64()) {
                t.max_context_tokens = Some(x);
            }
            Some(t)
        }
        None => None,
    };
    let job = Job::open(&repo, fresh, agent).context("opening the job")?;
    let mut text = describe(&job);
    // A fresh session picking up a handed-off job starts from the note,
    // not from re-exploring the repo.
    let handed_off_elsewhere = job.state.handoff.is_some() && job.state.handoff_transcript != job.state.transcript;
    if handed_off_elsewhere {
        if let Ok(note) = std::fs::read_to_string(job.repo.join(".ndstrl/handoff.md")) {
            text.push_str("\n\nHANDOFF FROM THE PREVIOUS SESSION (start here):\n");
            text.push_str(&note);
        }
    }
    Ok((job, text))
}

fn describe(job: &Job) -> String {
    let t = &job.spec;
    let (files, bytes) = size_of_scope(job);
    let est = bytes / 4;
    format!(
        "Job {} open in {}\nGoal: {}\nMay edit: {}\nMay not edit: {}\nChecks: {}\nBudget: {}{}{}\nMeter: {}\nSize: {} editable text files, about {} tokens if read whole. Read through search and excerpts instead.\nDone already: {}",
        t.id,
        job.repo.display(),
        t.goal,
        if t.allow.is_empty() { "whole repo".into() } else { t.allow.join(", ") },
        if t.deny.is_empty() { "nothing denied".into() } else { t.deny.join(", ") },
        t.test.as_deref().unwrap_or("none set (NDSTRL cannot tell when the job is done; ask the user)"),
        t.budget_usd.map(|b| format!("${b:.2}")).unwrap_or_else(|| "no dollar cap".into()),
        t.budget_tokens.map(|b| format!(", {b} tokens")).unwrap_or_default(),
        [t.budget_plan_5h_pct.map(|p| format!(", {p}% of the 5h window")), t.budget_plan_7d_pct.map(|p| format!(", {p}% of the week"))]
            .into_iter()
            .flatten()
            .collect::<String>(),
        job.state.transcript.as_deref().map(|p| format!("real usage from {p}")).unwrap_or_else(|| "NDSTRL estimate only (no transcript found)".into()),
        files,
        est,
        if job.state.done { "yes" } else { "no" },
    )
}

fn size_of_scope(job: &Job) -> (usize, u64) {
    let scope = job.scope();
    let mut files = 0;
    let mut bytes = 0;
    for e in ignore::WalkBuilder::new(scope.root()).build().flatten() {
        if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Ok(rel) = e.path().strip_prefix(scope.root()) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if scope.can_edit(&rel) == ndstrl_core::scope::Verdict::Allowed {
            files += 1;
            bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    (files, bytes)
}

pub fn dispatch(job: &mut Job, name: &str, a: &Value) -> Outcome {
    let need = |k: &str| -> Result<String, Outcome> {
        s(a, k).map(String::from).ok_or_else(|| Outcome::Denied(format!("missing `{k}`")))
    };
    let usize_of = |k: &str| a.get(k).and_then(|v| v.as_u64()).map(|n| n as usize);
    let r: Result<Outcome, Outcome> = (|| {
        Ok(match name {
            "job_status" => {
                job.refresh();
                let st = serde_json::to_string_pretty(&json!({
                    "spec": job.spec,
                    "touched": job.state.touched,
                    "open_questions": job.state.questions.iter().filter(|q| q.answer.is_none()).collect::<Vec<_>>(),
                    "denials": job.state.denials.len(),
                    "last_check_run": job.state.check_runs.last(),
                    "usage": job.state.usage,
                    "factory_tokens": job.state.factory_tokens,
                }))
                .unwrap_or_default();
                Outcome::Ok(st)
            }
            "list_files" => job.list(s(a, "glob")),
            "search" => job.search(&need("pattern")?, s(a, "glob"), a.get("ignore_case").and_then(|v| v.as_bool()).unwrap_or(false)),
            "read" => job.read(&need("path")?, usize_of("start_line"), usize_of("end_line")),
            "edit" => job.edit(
                &need("path")?,
                &need("old_string")?,
                &need("new_string")?,
                a.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false),
            ),
            "write" => job.write(&need("path")?, &need("content")?),
            "run_checks" | "inspect" => job.inspect(),
            "ask" => job.ask(&need("question")?, strings(a, "options")),
            "answer" => {
                let id = a.get("question_id").and_then(|v| v.as_u64()).ok_or_else(|| Outcome::Denied("missing `question_id`".into()))?;
                job.answer(id as u32, &need("answer")?, "agent")
            }
            "receipt" => match job.write_receipt() {
                Ok(p) => Outcome::Ok(format!("receipt written to {p}")),
                Err(e) => Outcome::Denied(format!("{e:#}")),
            },
            other => Outcome::Denied(format!("unknown tool {other}")),
        })
    })();
    r.unwrap_or_else(|o| o)
}
