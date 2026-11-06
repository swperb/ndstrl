//! Read-only view of one repo's floor. The desk never writes factory
//! state; it reads the same files the server writes and computes gauges
//! with the same core functions.

use ndstrl_core::job::{gauges_of, remaining_fraction_of, State};
use ndstrl_core::{Gauges, Traveler};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub at: String,
    pub kind: String,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReceiptRow {
    pub id: String,
    pub goal: String,
    pub result: String,
    pub files: usize,
    pub usd: Option<f64>,
    pub closed: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub repo: PathBuf,
    pub repo_name: String,
    pub traveler: Option<Traveler>,
    pub state: Option<State>,
    pub gauges: Option<Gauges>,
    pub remaining: Option<f64>,
    pub plan: Option<ndstrl_core::plan::Snapshot>,
    pub events: Vec<Event>,
    pub receipts: Vec<ReceiptRow>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Idle,
    Running,
    Asking,
    Done,
    Capped,
}

impl View {
    pub fn status(&self) -> Status {
        match &self.state {
            None => Status::Idle,
            Some(s) if s.capped => Status::Capped,
            Some(s) if s.done => Status::Done,
            Some(s) if s.questions.iter().any(|q| q.answer.is_none()) => Status::Asking,
            Some(_) => Status::Running,
        }
    }
}

pub fn default_repo(start: &Path) -> PathBuf {
    let mut p = start.to_path_buf();
    loop {
        if p.join(".ndstrl").exists() || p.join(".git").exists() {
            return p;
        }
        if !p.pop() {
            return start.to_path_buf();
        }
    }
}

pub fn load(repo: &Path) -> View {
    let dir = repo.join(".ndstrl");
    let traveler = Traveler::load(repo).ok().flatten();
    let state: Option<State> = std::fs::read_to_string(dir.join("state.json")).ok().and_then(|s| serde_json::from_str(&s).ok());
    // A state left over from an earlier traveler is not this job.
    let state = match (&traveler, state) {
        (Some(t), Some(s)) if s.job_id == t.id => Some(s),
        _ => None,
    };
    let gauges = match (&traveler, &state) {
        (Some(t), Some(s)) => Some(gauges_of(t, s)),
        _ => None,
    };
    let remaining = match (&traveler, &gauges) {
        (Some(t), Some(g)) => remaining_fraction_of(t, g),
        _ => None,
    };
    let job_id = traveler.as_ref().map(|t| t.id.clone()).unwrap_or_default();
    View {
        repo_name: repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        repo: repo.to_path_buf(),
        events: events(&dir.join("ledger.jsonl"), &job_id),
        receipts: receipts(&dir.join("receipts")),
        plan: ndstrl_core::plan::latest(),
        traveler,
        state,
        gauges,
        remaining,
    }
}

fn s(v: &serde_json::Value, k: &str) -> String {
    v.get(k).map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).unwrap_or_default()
}

/// Newest first, this job only, capped.
fn events(path: &Path, job: &str) -> Vec<Event> {
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    let mut out: Vec<Event> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|v| v["job"].as_str() == Some(job))
        .filter_map(|v| {
            let d = &v["data"];
            let kind = v["kind"].as_str()?.to_string();
            let (title, detail) = match kind.as_str() {
                "open" => (
                    if d["resumed"].as_bool() == Some(true) { "Job resumed".into() } else { "Job opened".into() },
                    format!("on {}", s(d, "machine")),
                ),
                "edit" => (format!("Edited {}", s(d, "path")), format!("+{} −{}", d["added"], d["removed"])),
                "deny" => (format!("Refused {} on {}", s(d, "tool"), s(d, "path")), s(d, "reason")),
                "ask" => (format!("Asked: question {}", d["id"]), s(d, "text")),
                "answer" => (
                    format!("Answered question {}", d["id"]),
                    format!("“{}” via {}", s(d, "answer"), if s(d, "via") == "human" { "you, directly" } else { "the machine" }),
                ),
                "inspect_start" => ("Inspection started".into(), s(d, "command")),
                "inspect" => (
                    if d["passed"].as_bool() == Some(true) { "Inspection passed".into() } else { "Inspection failed".into() },
                    format!("{:.1}s", d["secs"].as_f64().unwrap_or(0.0)),
                ),
                "cap" => ("Budget cap reached".into(), format!("blocked {}", s(d, "tool"))),
                "receipt" => ("Receipt written".into(), s(d, "path")),
                _ => return None,
            };
            let kind = if kind == "inspect" {
                if d["passed"].as_bool() == Some(true) { "pass".into() } else { "fail".into() }
            } else {
                kind
            };
            Some(Event { at: s(&v, "at"), kind, title, detail })
        })
        .collect();
    out.reverse();
    out.truncate(200);
    out
}

fn receipts(dir: &Path) -> Vec<ReceiptRow> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut rows: Vec<ReceiptRow> = rd
        .flatten()
        .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
        .filter_map(|e| {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            let st = &v["state"];
            let result = if st["done"].as_bool() == Some(true) {
                "passed"
            } else if st["capped"].as_bool() == Some(true) {
                "capped"
            } else {
                "open"
            };
            Some(ReceiptRow {
                id: s(&v["traveler"], "id"),
                goal: s(&v["traveler"], "goal"),
                result: result.into(),
                files: st["touched"].as_object().map(|o| o.len()).unwrap_or(0),
                usd: v["gauges"]["spent_usd"].as_f64(),
                closed: s(st, "opened_at"),
            })
        })
        .collect();
    rows.sort_by(|a, b| b.id.cmp(&a.id));
    rows
}
