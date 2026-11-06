//! The job engine: one open part on the floor, and every rule the
//! factory enforces while a machine works it.

use crate::critic::{self, CriticRun};
use crate::meter::{estimate_tokens, TranscriptMeter, Usage};
use crate::reads;
use crate::scope::{Scope, Verdict};
use crate::traveler::{Traveler, DIR};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Touch {
    pub added: usize,
    pub removed: usize,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    /// The next edit would leave the drawing.
    Scope { path: String },
    /// The next step would spend into the last part of the budget.
    Budget { remaining_fraction: f64 },
    /// The machine itself asked.
    Machine,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: u32,
    pub kind: QuestionKind,
    pub text: String,
    pub options: Vec<String>,
    pub answer: Option<String>,
    /// "human" when the factory asked the person directly (elicitation),
    /// "machine" when the harness relayed the answer.
    pub answered_via: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Denial {
    pub at: String,
    pub tool: String,
    pub path: String,
    pub reason: String,
}

/// Everything the factory remembers about the open job. Written to
/// `.ndstrl/state.json` after every change so the front office can watch.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub job_id: String,
    pub opened_at: String,
    pub machine: String,
    pub touched: BTreeMap<String, Touch>,
    pub critic_runs: Vec<CriticRun>,
    pub denials: Vec<Denial>,
    pub questions: Vec<Question>,
    /// Paths a human allowed after a scope question.
    pub widened: Vec<String>,
    /// Lowest remaining-budget fraction a human already approved going below.
    pub budget_ok_below: Option<f64>,
    pub factory_tokens: u64,
    pub transcript: Option<String>,
    pub usage: Usage,
    pub done: bool,
    pub capped: bool,
    pub receipt: Option<String>,
    /// Plan snapshot when the job opened, for per-job window deltas.
    #[serde(default)]
    pub plan_at_open: Option<crate::plan::Snapshot>,
    /// (snapshot time, job list-usd when that snapshot was first seen):
    /// the base for projecting plan usage between snapshots.
    #[serde(default)]
    pub plan_anchor: Option<(i64, f64)>,
    /// Edits applied since the last inspection; nonzero means its verdict is stale.
    #[serde(default)]
    pub edits_since_inspect: u32,
}

/// What a tool hands back. Every variant is reported with the gauges.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Ok(String),
    Denied(String),
    Ask(Question),
    Blocked(String),
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Gauges {
    pub spent_usd: Option<f64>,
    pub budget_usd: Option<f64>,
    pub remaining_usd: Option<f64>,
    pub tokens: u64,
    pub budget_tokens: Option<u64>,
    pub meter: &'static str,
    pub files_touched: usize,
    pub critic: &'static str,
    pub done: bool,
    pub open_question: Option<u32>,
    pub plan_5h: Option<crate::plan::Estimate>,
    pub plan_7d: Option<crate::plan::Estimate>,
    /// Percentage points of each window this job has used.
    pub job_5h_pts: Option<f64>,
    pub job_7d_pts: Option<f64>,
}

impl Gauges {
    pub fn line(&self) -> String {
        let money = match (self.spent_usd, self.budget_usd) {
            (Some(s), Some(b)) => format!("${s:.2} of ${b:.2} list-equiv"),
            (Some(s), None) => format!("${s:.2} list-equiv, no cap"),
            (None, Some(b)) => format!("{} tok, cap ${b:.2} (unmetered)", self.tokens),
            (None, None) => format!("{} tok", self.tokens),
        };
        let tok = self.budget_tokens.map(|b| format!(" / {b} tok cap")).unwrap_or_default();
        let q = self.open_question.map(|q| format!(" | waiting on question {q}")).unwrap_or_default();
        let w = |e: &Option<crate::plan::Estimate>, job: Option<f64>, name: &str| -> Option<String> {
            let e = e.as_ref()?;
            let j = job.filter(|j| *j >= 0.05).map(|j| format!(" (job +{j:.1})")).unwrap_or_default();
            Some(format!("{name} {:.0}%{}{j}", e.used_percentage, if e.projected { "~" } else { "" }))
        };
        let plan: Vec<String> = [w(&self.plan_5h, self.job_5h_pts, "5h"), w(&self.plan_7d, self.job_7d_pts, "7d")]
            .into_iter()
            .flatten()
            .collect();
        let plan = if plan.is_empty() { String::new() } else { format!(" | plan {}", plan.join(" ")) };
        format!(
            "[ndstrl] {money}{tok} ({}){plan} | touched {} | critic {} | done {}{q}",
            self.meter,
            self.files_touched,
            self.critic,
            if self.done { "yes" } else { "no" }
        )
    }
}

pub struct Job {
    pub repo: PathBuf,
    pub traveler: Traveler,
    pub state: State,
    scope: Scope,
    meter: Option<TranscriptMeter>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Job {
    /// Open (or resume) a job in `repo`. If the repo has a traveler and
    /// `fresh` is None, it is resumed; otherwise `fresh` becomes the traveler.
    pub fn open(repo: &Path, fresh: Option<Traveler>, machine: &str) -> Result<Self> {
        let repo = repo.canonicalize().with_context(|| format!("no such repo {}", repo.display()))?;
        let existing = Traveler::load(&repo)?;
        let (mut traveler, resumed) = match (fresh, existing) {
            (Some(t), _) => (t, false),
            (None, Some(t)) => (t, true),
            (None, None) => bail!("no traveler at {}; give a goal to start one", Traveler::path(&repo).display()),
        };
        if traveler.id.is_empty() {
            traveler.id = Traveler::new_id(&traveler.goal);
        }
        traveler.save(&repo)?;
        let scope = Scope::new(&repo, &traveler.allow, &traveler.deny)?;

        let state_path = repo.join(DIR).join("state.json");
        let prior: Option<State> = std::fs::read_to_string(&state_path).ok().and_then(|s| serde_json::from_str(&s).ok());
        let prior_transcript = prior.as_ref().and_then(|p| p.transcript.clone());
        let prior_job = prior.as_ref().map(|p| p.job_id.clone()).unwrap_or_default();
        let mut state = match prior {
            Some(s) if resumed && s.job_id == traveler.id => s,
            _ => State { job_id: traveler.id.clone(), opened_at: now(), ..Default::default() },
        };
        state.machine = machine.to_string();
        if state.plan_at_open.is_none() {
            state.plan_at_open = crate::plan::latest();
        }
        // A fresh job meters the whole harness session: "stop at $4" means
        // the session. Only when an earlier job already ran in this same
        // session does the count start at this job's opening.
        let found = TranscriptMeter::discover();
        let earlier_job_here = prior_transcript.is_some()
            && prior_transcript == found.as_ref().map(|p| p.display().to_string())
            && prior_job != traveler.id;
        let since = if earlier_job_here { state.opened_at.clone() } else { String::new() };
        let meter = found.map(|p| TranscriptMeter::new(p, since));
        state.transcript = meter.as_ref().map(|m| m.path().display().to_string());
        let mut job = Job { repo, traveler, state, scope, meter };
        job.rescope()?;
        job.refresh();
        job.log("open", serde_json::json!({ "goal": job.traveler.goal, "resumed": resumed, "machine": machine }));
        job.persist();
        Ok(job)
    }

    fn rescope(&mut self) -> Result<()> {
        self.scope = Scope::with_widened(&self.repo, &self.traveler.allow, &self.traveler.deny, &self.state.widened)?;
        Ok(())
    }

    // ---- meter -----------------------------------------------------------

    pub fn refresh(&mut self) {
        if let Some(m) = &mut self.meter {
            if let Ok(u) = m.poll() {
                self.state.usage = u.clone();
            }
        }
        if let Some(snap) = crate::plan::latest() {
            if self.state.plan_anchor.map(|(at, _)| at) != Some(snap.at) {
                self.state.plan_anchor = Some((snap.at, self.state.usage.usd));
            }
        }
    }

    pub fn gauges(&self) -> Gauges {
        gauges_of(&self.traveler, &self.state)
    }

    pub fn remaining_fraction(&self) -> Option<f64> {
        remaining_fraction_of(&self.traveler, &self.gauges())
    }

    /// Run before every step that spends. Returns a block or a question
    /// if the cell should not proceed.
    fn gate(&mut self, tool: &str) -> Option<Outcome> {
        self.refresh();
        if let Some(q) = self.state.questions.iter().find(|q| q.answer.is_none()) {
            return Some(Outcome::Blocked(format!(
                "question {} is still open: \"{}\". Put it to the user and pass their answer to `answer`.",
                q.id, q.text
            )));
        }
        if self.state.capped {
            return Some(Outcome::Blocked("the budget cap was hit; the receipt is written. Stop and report.".into()));
        }
        if let Some(f) = self.remaining_fraction() {
            if f <= 0.0 {
                self.state.capped = true;
                let path = self.write_receipt().ok();
                self.log("cap", serde_json::json!({ "tool": tool }));
                self.persist();
                return Some(Outcome::Blocked(format!(
                    "budget cap reached. No further paid steps. Receipt: {}. Stop and report to the user.",
                    path.unwrap_or_default()
                )));
            }
            let ok_below = self.state.budget_ok_below.unwrap_or(1.0);
            // Ask once when crossing the ask line, and again at each halving after an approval.
            let line = if self.state.budget_ok_below.is_some() { ok_below / 2.0 } else { self.traveler.ask_below };
            if f < line && tool != "receipt" {
                let pct = (f * 100.0).round();
                let q = self.ask_inner(
                    QuestionKind::Budget { remaining_fraction: f },
                    format!(
                        "Only {pct}% of the budget is left and the part is not done. Keep going, or stop here with a receipt?"
                    ),
                    vec!["continue".into(), "stop".into()],
                );
                return Some(Outcome::Ask(q));
            }
        }
        None
    }

    // ---- questions -------------------------------------------------------

    fn ask_inner(&mut self, kind: QuestionKind, text: String, options: Vec<String>) -> Question {
        let id = self.state.questions.len() as u32 + 1;
        let q = Question { id, kind, text, options, answer: None, answered_via: None };
        self.state.questions.push(q.clone());
        self.log("ask", serde_json::to_value(&q).unwrap_or_default());
        self.persist();
        q
    }

    /// The machine asks the user something. Always allowed, never paid.
    pub fn ask(&mut self, text: &str, options: Vec<String>) -> Outcome {
        Outcome::Ask(self.ask_inner(QuestionKind::Machine, text.to_string(), options))
    }

    pub fn answer(&mut self, id: u32, answer: &str, via: &str) -> Outcome {
        let Some(q) = self.state.questions.iter_mut().find(|q| q.id == id) else {
            return Outcome::Denied(format!("no question {id}"));
        };
        if q.answer.is_some() {
            return Outcome::Denied(format!("question {id} is already answered"));
        }
        q.answer = Some(answer.to_string());
        q.answered_via = Some(via.to_string());
        let kind = q.kind.clone();
        let yes = {
            let a = answer.trim().to_ascii_lowercase();
            a.starts_with("allow") || a.starts_with("yes") || a.starts_with("continue") || a == "y" || a == "ok"
        };
        let msg = match kind {
            QuestionKind::Scope { path } if yes => {
                self.state.widened.push(path.clone());
                let _ = self.rescope();
                format!("{path} is now in scope for this job.")
            }
            QuestionKind::Scope { path } => format!("{path} stays out of scope. Find a way inside the drawing or stop."),
            QuestionKind::Budget { remaining_fraction } if yes => {
                self.state.budget_ok_below = Some(remaining_fraction);
                "Approved to keep spending. You will be asked again if half of what is left goes too.".into()
            }
            QuestionKind::Budget { .. } => {
                self.state.capped = true;
                let r = self.write_receipt().unwrap_or_default();
                format!("The user chose to stop. Receipt: {r}. Report and end the run.")
            }
            QuestionKind::Machine => format!("Recorded. The user said: {answer}"),
        };
        self.log("answer", serde_json::json!({ "id": id, "answer": answer, "via": via }));
        self.persist();
        Outcome::Ok(msg)
    }

    // ---- reads (cheap, factory-metered) -----------------------------------

    fn charge(&mut self, text: &str) {
        self.state.factory_tokens += estimate_tokens(text);
    }

    pub fn list(&mut self, glob: Option<&str>) -> Outcome {
        let r = reads::list(&self.scope, glob, 300);
        self.read_outcome(r)
    }

    pub fn search(&mut self, pattern: &str, glob: Option<&str>, ignore_case: bool) -> Outcome {
        if let Some(b) = self.cap_only() {
            return b;
        }
        let r = reads::search(&self.scope, pattern, glob, ignore_case);
        self.read_outcome(r)
    }

    pub fn read(&mut self, path: &str, start: Option<usize>, end: Option<usize>) -> Outcome {
        if let Some(b) = self.cap_only() {
            return b;
        }
        let r = self.scope.rel(path).and_then(|rel| reads::read(&self.scope, &rel, start, end));
        self.read_outcome(r)
    }

    /// Reads are allowed while a question is open (the machine may need
    /// them to explain itself) but not after the cap.
    fn cap_only(&mut self) -> Option<Outcome> {
        self.refresh();
        self.state.capped.then(|| Outcome::Blocked("the budget cap was hit. Stop and report.".into()))
    }

    fn read_outcome(&mut self, r: Result<String>) -> Outcome {
        match r {
            Ok(t) => {
                self.charge(&t);
                self.persist();
                Outcome::Ok(t)
            }
            Err(e) => Outcome::Denied(e.to_string()),
        }
    }

    // ---- edits -----------------------------------------------------------

    fn edit_gate(&mut self, tool: &str, path: &str) -> Result<String, Outcome> {
        if let Some(b) = self.gate(tool) {
            return Err(b);
        }
        if self.state.done {
            return Err(Outcome::Blocked(
                "inspection already passes, so the part is done. No more edits. Call `receipt` and report.".into(),
            ));
        }
        let rel = self.scope.rel(path).map_err(|e| Outcome::Denied(e.to_string()))?;
        match self.scope.can_edit(&rel) {
            Verdict::Allowed => Ok(rel),
            Verdict::Forbidden(why) => {
                self.deny(tool, &rel, &why);
                Err(Outcome::Denied(why))
            }
            Verdict::OutOfScope(why) => {
                self.deny(tool, &rel, &why);
                let q = self.ask_inner(
                    QuestionKind::Scope { path: rel.clone() },
                    format!("The next edit would touch {rel}, which is outside this job ({why}). Allow it for this job?"),
                    vec![format!("allow {rel}"), "keep it out of scope".into()],
                );
                Err(Outcome::Ask(q))
            }
        }
    }

    fn deny(&mut self, tool: &str, path: &str, reason: &str) {
        self.state.denials.push(Denial { at: now(), tool: tool.into(), path: path.into(), reason: reason.into() });
        self.log("deny", serde_json::json!({ "tool": tool, "path": path, "reason": reason }));
    }

    fn record_touch(&mut self, rel: &str, before: Option<&str>, after: &str) {
        let (mut add, mut rem) = (0, 0);
        let diff = similar::TextDiff::from_lines(before.unwrap_or(""), after);
        for c in diff.iter_all_changes() {
            match c.tag() {
                similar::ChangeTag::Insert => add += 1,
                similar::ChangeTag::Delete => rem += 1,
                _ => {}
            }
        }
        self.state.edits_since_inspect += 1;
        let t = self.state.touched.entry(rel.to_string()).or_default();
        t.added += add;
        t.removed += rem;
        t.created |= before.is_none();
        self.log("edit", serde_json::json!({ "path": rel, "added": add, "removed": rem }));
    }

    /// Exact string replacement, the harness-neutral edit.
    pub fn edit(&mut self, path: &str, old: &str, new: &str, replace_all: bool) -> Outcome {
        let rel = match self.edit_gate("edit", path) {
            Ok(r) => r,
            Err(o) => return o,
        };
        let abs = self.scope.abs(&rel);
        let Ok(before) = std::fs::read_to_string(&abs) else {
            return Outcome::Denied(format!("{rel} does not exist or is not text; use `write` to create it"));
        };
        if old.is_empty() {
            return Outcome::Denied("old_string is empty; use `write` to create or replace a whole file".into());
        }
        let count = before.matches(old).count();
        let after = match (count, replace_all) {
            (0, _) => return Outcome::Denied(format!("old_string not found in {rel}; read the file again")),
            (1, _) | (_, true) => before.replace(old, new),
            (n, false) => {
                return Outcome::Denied(format!(
                    "old_string occurs {n} times in {rel}; add context to make it unique or set replace_all"
                ))
            }
        };
        if let Err(e) = std::fs::write(&abs, &after) {
            return Outcome::Denied(format!("write failed: {e}"));
        }
        self.record_touch(&rel, Some(&before), &after);
        self.persist();
        let t = &self.state.touched[&rel];
        Outcome::Ok(format!("applied to {rel} ({count} replacement(s)); this job so far +{} -{} there", t.added, t.removed))
    }

    pub fn write(&mut self, path: &str, content: &str) -> Outcome {
        let rel = match self.edit_gate("write", path) {
            Ok(r) => r,
            Err(o) => return o,
        };
        let abs = self.scope.abs(&rel);
        let before = std::fs::read_to_string(&abs).ok();
        if let Some(parent) = abs.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&abs, content) {
            return Outcome::Denied(format!("write failed: {e}"));
        }
        self.record_touch(&rel, before.as_deref(), content);
        self.persist();
        Outcome::Ok(format!("{} {rel}", if before.is_some() { "replaced" } else { "created" }))
    }

    // ---- critic ----------------------------------------------------------

    pub fn inspect(&mut self) -> Outcome {
        if let Some(b) = self.gate("inspect") {
            return b;
        }
        let Some(cmd) = self.traveler.test.clone() else {
            return Outcome::Denied("the traveler has no test command, so the factory cannot inspect. Ask the user what done looks like.".into());
        };
        self.log("inspect_start", serde_json::json!({ "command": cmd }));
        self.persist();
        let run = critic::run(&self.repo, &cmd, Duration::from_secs(self.traveler.critic_timeout_secs));
        let passed = run.passed;
        let text = if passed {
            self.state.done = true;
            format!(
                "PASS in {:.1}s: `{}`. The part is done; edit tools are now closed. Call `receipt`.\n{}",
                run.secs, run.command, run.summary
            )
        } else if run.timed_out {
            format!("TIMED OUT after {:.0}s: `{}`\n{}", run.secs, run.command, run.summary)
        } else {
            format!("FAIL (exit {:?}) in {:.1}s: `{}`\n{}", run.exit_code, run.secs, run.command, run.summary)
        };
        self.log("inspect", serde_json::json!({ "passed": passed, "secs": run.secs, "exit": run.exit_code }));
        self.state.critic_runs.push(run);
        self.state.edits_since_inspect = 0;
        self.charge(&text);
        self.persist();
        Outcome::Ok(text)
    }

    // ---- receipt ---------------------------------------------------------

    pub fn write_receipt(&mut self) -> Result<String> {
        self.refresh();
        let g = self.gauges();
        let t = &self.traveler;
        let s = &self.state;
        let dir = self.repo.join(DIR).join("receipts");
        std::fs::create_dir_all(&dir)?;
        let mut md = String::new();
        use std::fmt::Write as _;
        let _ = writeln!(md, "# Receipt {}\n", t.id);
        let _ = writeln!(md, "- Goal: {}", t.goal);
        let _ = writeln!(md, "- Machine: {}", s.machine);
        let _ = writeln!(md, "- Opened: {}  Closed: {}", s.opened_at, now());
        let _ = writeln!(
            md,
            "- Result: {}",
            if s.done { "inspection passed" } else if s.capped { "stopped at the cap" } else { "not done" }
        );
        let _ = writeln!(md, "\n## Spend\n");
        let cap = t.budget_usd.map(|b| format!("${b:.2}")).unwrap_or_else(|| "none".into());
        match g.spent_usd {
            Some(v) => {
                let _ = writeln!(md, "- ${v:.4} at API list price (a usage proxy on subscription plans, not a bill), budget {cap}");
            }
            None => {
                let _ = writeln!(md, "- dollars not metered on this machine; budget {cap}");
            }
        }
        let u = &s.usage;
        let _ = writeln!(
            md,
            "- Model tokens: input {} | output {} | cache write {} | cache read {} | responses {}",
            u.input,
            u.output,
            u.cache_write_5m + u.cache_write_1h,
            u.cache_read,
            u.responses
        );
        let _ = writeln!(md, "- Factory tokens returned to the model (estimate): {}", s.factory_tokens);
        let _ = writeln!(md, "\n## Files\n");
        if s.touched.is_empty() {
            let _ = writeln!(md, "- none");
        }
        for (p, t) in &s.touched {
            let _ = writeln!(md, "- {p}  +{} -{}{}", t.added, t.removed, if t.created { " (new)" } else { "" });
        }
        let _ = writeln!(md, "\n## Inspection\n");
        for r in &s.critic_runs {
            let _ = writeln!(md, "- {} `{}` in {:.1}s", if r.passed { "PASS" } else { "FAIL" }, r.command, r.secs);
        }
        let _ = writeln!(md, "\n## Questions\n");
        for q in &s.questions {
            let _ = writeln!(
                md,
                "- Q{} {} -> {} ({})",
                q.id,
                q.text,
                q.answer.as_deref().unwrap_or("unanswered"),
                q.answered_via.as_deref().unwrap_or("-")
            );
        }
        let _ = writeln!(md, "\n## Denials\n");
        for d in &s.denials {
            let _ = writeln!(md, "- {} {}: {}", d.tool, d.path, d.reason);
        }
        let path = dir.join(format!("{}.md", t.id));
        std::fs::write(&path, md)?;
        std::fs::write(
            dir.join(format!("{}.json", t.id)),
            serde_json::to_string_pretty(&serde_json::json!({ "traveler": t, "state": s, "gauges": g }))?,
        )?;
        let shown = path.strip_prefix(&self.repo).unwrap_or(&path).display().to_string();
        self.state.receipt = Some(shown.clone());
        self.log("receipt", serde_json::json!({ "path": shown }));
        self.persist();
        Ok(shown)
    }

    // ---- persistence -----------------------------------------------------

    fn persist(&self) {
        let p = self.repo.join(DIR).join("state.json");
        if let Ok(s) = serde_json::to_string_pretty(&self.state) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(tmp, p);
            }
        }
    }

    fn log(&self, kind: &str, data: serde_json::Value) {
        let p = self.repo.join(DIR).join("ledger.jsonl");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "{}", serde_json::json!({ "at": now(), "job": self.traveler.id, "kind": kind, "data": data }));
        }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }
}

// ---- pure views, shared by the server and the front office -----------------

fn plan_view(s: &State) -> (Option<crate::plan::Estimate>, Option<crate::plan::Estimate>, Option<f64>, Option<f64>) {
    let Some(snap) = crate::plan::latest() else { return (None, None, None, None) };
    let rates = crate::plan::rates();
    let since = s.plan_anchor.map(|(_, u)| (s.usage.usd - u).max(0.0)).unwrap_or(0.0);
    let e5 = crate::plan::estimate(&snap, "five_hour", since, &rates);
    let e7 = crate::plan::estimate(&snap, "seven_day", since, &rates);
    let delta = |e: &Option<crate::plan::Estimate>, w: Option<crate::plan::Window>| -> Option<f64> {
        let (e, w) = (e.as_ref()?, w?);
        (e.resets_at == w.resets_at).then(|| (e.used_percentage - w.used_percentage).max(0.0))
    };
    let open = s.plan_at_open.as_ref();
    let d5 = delta(&e5, open.and_then(|o| o.five_hour));
    let d7 = delta(&e7, open.and_then(|o| o.seven_day));
    (e5, e7, d5, d7)
}

/// Gauges from the traveler and saved state alone. The server and the
/// front office both call this, so they can never disagree.
pub fn gauges_of(t: &Traveler, s: &State) -> Gauges {
    let has_meter = s.transcript.is_some();
    let spent = (has_meter && s.usage.unpriced_responses == 0).then_some(s.usage.usd);
    let tokens = if has_meter { s.usage.tokens() } else { s.factory_tokens };
    let (p5, p7, d5, d7) = plan_view(s);
    Gauges {
        spent_usd: spent,
        budget_usd: t.budget_usd,
        remaining_usd: match (spent, t.budget_usd) {
            (Some(sp), Some(b)) => Some(b - sp),
            _ => None,
        },
        tokens,
        budget_tokens: t.budget_tokens,
        meter: if has_meter { "transcript" } else { "factory estimate" },
        files_touched: s.touched.len(),
        critic: match s.critic_runs.last() {
            None => "not run",
            Some(_) if s.edits_since_inspect > 0 && !s.done => "stale",
            Some(r) if r.passed => "pass",
            Some(_) => "fail",
        },
        done: s.done,
        open_question: s.questions.iter().find(|q| q.answer.is_none()).map(|q| q.id),
        plan_5h: p5,
        plan_7d: p7,
        job_5h_pts: d5,
        job_7d_pts: d7,
    }
}

/// Fraction of the tightest budget left: list dollars, tokens, or plan
/// window points, whichever is closest to its cap.
pub fn remaining_fraction_of(t: &Traveler, g: &Gauges) -> Option<f64> {
    let mut fracs = Vec::new();
    if let (Some(r), Some(b)) = (g.remaining_usd, g.budget_usd) {
        fracs.push(r / b);
    }
    if let Some(b) = g.budget_tokens {
        fracs.push(1.0 - g.tokens as f64 / b as f64);
    }
    if let (Some(cap), Some(used)) = (t.budget_plan_5h_pct, g.job_5h_pts) {
        fracs.push(1.0 - used / cap);
    }
    if let (Some(cap), Some(used)) = (t.budget_plan_7d_pct, g.job_7d_pts) {
        fracs.push(1.0 - used / cap);
    }
    fracs.into_iter().reduce(f64::min)
}
