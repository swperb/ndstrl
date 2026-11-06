//! Auto mode: from a goal alone, draft the job (scope, checks, route,
//! cost range) and the ways widening it could help, for the person to
//! approve before anything runs. Drafting costs no model tokens.

use crate::spec::JobSpec;
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// One way to widen the job, offered with what it buys and what it costs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Widen {
    /// What the approval option says, e.g. "Approve + web research".
    pub option: String,
    pub why: String,
    pub cost: String,
    pub apply: Apply,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Apply {
    Web,
    Paths(Vec<String>),
    Frontier,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Proposal {
    pub spec: JobSpec,
    pub files: Vec<String>,
    pub low_usd: f64,
    pub high_usd: f64,
    pub basis: String,
    pub widen: Vec<Widen>,
}

const STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "make", "when", "then", "than", "should", "would", "could",
    "button", "page", "work", "works", "working", "broken", "fix", "fixes", "add", "adds", "change", "update", "remove",
    "don't", "dont", "touch", "only", "stop", "after", "before", "have", "doesn't", "isn't", "about", "there", "their",
    "test", "tests", "failing", "make", "sure", "also", "just", "some", "more", "less", "every", "each", "using",
];

/// Words from the goal worth searching for: identifiers, paths, and
/// meaningful words, lowercased.
fn terms(goal: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in goal.split(|c: char| c.is_whitespace() || ",;:()[]{}\"'`".contains(c)) {
        let w = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '/' && c != '.' && c != '-');
        if w.len() < 4 {
            continue;
        }
        let lw = w.to_lowercase();
        if STOP.contains(&lw.as_str()) || lw.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '$') {
            continue;
        }
        if !out.contains(&lw) {
            out.push(lw);
        }
    }
    out
}

/// Rank repo files by how strongly they match the goal's terms: path hits
/// count more than content hits.
fn candidates(repo: &Path, terms: &[String]) -> (Vec<(String, u64)>, HashMap<String, usize>) {
    let mut scored = Vec::new();
    let mut per_dir: HashMap<String, usize> = HashMap::new();
    for e in WalkBuilder::new(repo).build().flatten() {
        if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Ok(rel) = e.path().strip_prefix(repo) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if rel.starts_with(".ndstrl/") || rel.starts_with(".git/") || rel.ends_with(".lock") {
            continue;
        }
        let dir = rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        *per_dir.entry(dir).or_default() += 1;
        let lrel = rel.to_lowercase();
        let mut score = 0u64;
        for t in terms {
            if lrel.contains(t.as_str()) {
                score += 10;
            }
        }
        if e.metadata().map(|m| m.len() < 512 * 1024).unwrap_or(false) {
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                let lt = text.to_lowercase();
                for t in terms {
                    score += (lt.matches(t.as_str()).count() as u64).min(5);
                }
            }
        }
        if score > 0 {
            scored.push((rel, score));
        }
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    (scored, per_dir)
}

pub fn guess_test(repo: &Path) -> Option<String> {
    let has = |f: &str| repo.join(f).exists();
    if has("Cargo.toml") {
        Some("cargo test -q".into())
    } else if has("package.json") {
        Some("npm test --silent".into())
    } else if has("pyproject.toml") || has("pytest.ini") || has("setup.py") {
        Some("pytest -q".into())
    } else if has("go.mod") {
        Some("go test ./...".into())
    } else {
        None
    }
}

/// Dependency names declared by the project, for spotting goals that hinge
/// on a library's behavior.
fn dependencies(repo: &Path) -> Vec<String> {
    let mut deps = Vec::new();
    if let Ok(t) = std::fs::read_to_string(repo.join("Cargo.toml")) {
        if let Ok(v) = t.parse::<toml::Table>() {
            for k in ["dependencies", "dev-dependencies"] {
                if let Some(d) = v.get(k).and_then(|d| d.as_table()) {
                    deps.extend(d.keys().cloned());
                }
            }
        }
    }
    if let Ok(t) = std::fs::read_to_string(repo.join("package.json")) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&t) {
            for k in ["dependencies", "devDependencies"] {
                if let Some(d) = v.get(k).and_then(|d| d.as_object()) {
                    deps.extend(d.keys().cloned());
                }
            }
        }
    }
    deps
}

/// Past receipts of finished jobs, for estimates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryRow {
    pub files: usize,
    pub lines: usize,
    pub usd: f64,
    pub done: bool,
}

pub fn history() -> Vec<HistoryRow> {
    let Some(dir) = crate::plan::home() else { return vec![] };
    std::fs::read_to_string(dir.join("history.jsonl"))
        .map(|t| t.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
        .unwrap_or_default()
}

pub fn record_history(row: &HistoryRow) {
    use std::io::Write;
    if let Some(dir) = crate::plan::home() {
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("history.jsonl")) {
            let _ = writeln!(f, "{}", serde_json::to_string(row).unwrap_or_default());
        }
    }
}

fn round_up(x: f64) -> f64 {
    if x < 1.0 { (x * 20.0).ceil() / 20.0 } else { (x * 4.0).ceil() / 4.0 }
}

/// A cost range for a job touching about `files` files. From history when
/// there is enough of it, otherwise a rough rule from the scope's size.
fn estimate(files: usize, scope_tokens: u64, hist: &[HistoryRow]) -> (f64, f64, String) {
    let mut similar: Vec<f64> = hist.iter().filter(|h| h.done && h.files.abs_diff(files) <= 2).map(|h| h.usd).collect();
    if similar.len() >= 3 {
        similar.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |p: f64| similar[((similar.len() - 1) as f64 * p).round() as usize];
        return (round_up(q(0.25)), round_up(q(0.9)), format!("from {} past jobs of similar size", similar.len()));
    }
    // A main agent at Opus prices re-reads its context each turn; scope size
    // is the best cheap proxy for how much it will read.
    let low = 0.10 + scope_tokens as f64 * 4e-6 + files as f64 * 0.03;
    (round_up(low), round_up(low * 3.0), "rough: not enough past jobs yet".into())
}

pub fn propose(repo: &Path, goal: &str) -> Proposal {
    let t = terms(goal);
    let (scored, per_dir) = candidates(repo, &t);
    let files: Vec<String> = scored.iter().take(6).filter(|(_, s)| *s >= scored.first().map(|f| f.1 / 4).unwrap_or(0)).map(|(f, _)| f.clone()).collect();
    let scope_bytes: u64 = files.iter().filter_map(|f| std::fs::metadata(repo.join(f)).ok()).map(|m| m.len()).sum();
    let (low, high, basis) = estimate(files.len().max(1), scope_bytes / 4, &history());

    let mut spec = JobSpec { goal: goal.to_string(), ..Default::default() };
    spec.allow = files.clone();
    spec.test = guess_test(repo);
    spec.budget_usd = Some(high.max(0.25));
    spec.max_files = Some((files.len() + 1).max(3));
    spec.max_changed_lines = Some(200);

    let mut widen = Vec::new();
    let lg = goal.to_lowercase();
    let deps = dependencies(repo);
    let named: Vec<&String> = deps.iter().filter(|d| d.len() >= 3 && lg.contains(&d.to_lowercase())).collect();
    let docsy = ["docs", "api", "upgrade", "migrate", "version", "deprecat", "breaking", "sdk"].iter().any(|k| lg.contains(k));
    if !named.is_empty() || docsy {
        let what = if named.is_empty() { "the relevant docs".to_string() } else { format!("the {} docs", named.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" and ")) };
        widen.push(Widen {
            option: "Approve + web research".into(),
            why: format!("Let helpers read {what} online. More robust when the fix depends on a library's current behavior."),
            cost: "about $0.01 to $0.05 per lookup on Haiku".into(),
            apply: Apply::Web,
        });
    }
    let mut dirs: Vec<(String, usize)> = files
        .iter()
        .filter_map(|f| f.rsplit_once('/').map(|(d, _)| d.to_string()))
        .filter_map(|d| per_dir.get(&d).map(|n| (d, *n)))
        .filter(|(_, n)| *n > 1)
        .collect();
    dirs.sort();
    dirs.dedup();
    if !dirs.is_empty() {
        let total: usize = dirs.iter().map(|(_, n)| n).sum();
        let (wl, wh, _) = estimate(total, scope_bytes / 4 * 2, &history());
        widen.push(Widen {
            option: "Approve + wider scope".into(),
            why: format!(
                "Allow all of {} ({total} files). Helps if the fix spans sibling files; the draft only includes the closest matches.",
                dirs.iter().map(|(d, _)| format!("{d}/")).collect::<Vec<_>>().join(", ")
            ),
            cost: format!("estimate rises to ${wl:.2} to ${wh:.2}"),
            apply: Apply::Paths(dirs.iter().map(|(d, _)| d.clone()).collect()),
        });
    }
    let hard = ["race", "concurren", "deadlock", "flaky", "intermittent", "architecture", "redesign", "performance", "memory leak", "security"]
        .iter()
        .any(|k| lg.contains(k));
    if hard {
        widen.push(Widen {
            option: "Approve + Fable for the hard part".into(),
            why: "Allow the frontier model for the hard step only. Better odds on gnarly bugs.".into(),
            cost: "one long Fable turn can use several percent of your week".into(),
            apply: Apply::Frontier,
        });
    }
    Proposal { spec, files, low_usd: low, high_usd: high, basis, widen }
}

impl Proposal {
    /// The approval options: approve as drafted, each widening, cancel.
    pub fn options(&self) -> Vec<String> {
        let mut o = vec!["Approve".to_string()];
        o.extend(self.widen.iter().map(|w| w.option.clone()));
        o.push("Cancel".into());
        o
    }

    pub fn summary(&self) -> String {
        let s = &self.spec;
        let mut t = format!("Draft job: {}\n", s.goal);
        t.push_str(&format!(
            "Scope: {}\n",
            if self.files.is_empty() { "no strong matches; the whole repo (consider naming files)".to_string() } else { self.files.join(", ") }
        ));
        t.push_str(&format!("Checks: {}\n", s.test.as_deref().unwrap_or("none found (say what done looks like)")));
        t.push_str("Route: look-ups to Haiku, edits to Sonnet, hard problems to Opus, each sent to whichever of your plans is least used; Fable off.\n");
        if let Some(hint) = crate::route::main_session_hint(&crate::route::pressures()) {
            t.push_str(&format!("{hint}\n"));
        }
        t.push_str(&format!(
            "Estimate: ${:.2} to ${:.2} at API prices ({}). Cap set at ${:.2}. Up to {} files, 200 changed lines.\n",
            self.low_usd,
            self.high_usd,
            self.basis,
            s.budget_usd.unwrap_or(self.high_usd),
            s.max_files.unwrap_or(3)
        ));
        if !self.widen.is_empty() {
            t.push_str("Widening could help:\n");
            for w in &self.widen {
                t.push_str(&format!("- {}: {} ({})\n", w.option, w.why, w.cost));
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drafts_scope_from_the_goal() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        std::fs::create_dir_all(r.join("src/settings")).unwrap();
        std::fs::write(r.join("Cargo.toml"), "[package]\nname='x'\n[dependencies]\nserde = '1'\n").unwrap();
        std::fs::write(r.join("src/settings/save.rs"), "fn save_settings() {}\n").unwrap();
        std::fs::write(r.join("src/settings/form.rs"), "fn form() {}\n").unwrap();
        std::fs::write(r.join("src/other.rs"), "fn other() {}\n").unwrap();
        let p = propose(r, "Fix save_settings so the settings page persists; check the serde docs");
        assert_eq!(p.files.first().map(String::as_str), Some("src/settings/save.rs"));
        assert!(!p.files.contains(&"src/other.rs".to_string()));
        assert_eq!(p.spec.test.as_deref(), Some("cargo test -q"));
        assert!(p.widen.iter().any(|w| w.apply == Apply::Web));
        assert!(p.high_usd >= p.low_usd && p.low_usd > 0.0);
        assert_eq!(p.options().first().map(String::as_str), Some("Approve"));
    }
}
