//! The job engine: the open job and every rule NDSTRL enforces while an
//! agent works on it.

use crate::checks::{self, CheckRun};
use crate::meter::{estimate_tokens, TranscriptMeter, Usage};
use crate::reads;
use crate::scope::{Scope, Verdict};
use crate::spec::{JobSpec, DIR};
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
    /// The next edit is outside the job's scope.
    Scope { path: String },
    /// The next step would spend into the last part of the budget.
    Budget { remaining_fraction: f64 },
    /// The agent itself asked.
    #[serde(alias = "machine")]
    Agent,
    /// The next edit would push past the job's file or line cap.
    Size { what: String },
    /// The job has run past its time cap.
    Time { minutes: u64 },
    /// The agent wants a tool the job does not allow yet (for example web).
    Tool { name: String },
    /// Auto mode drafted the job; nothing runs until the person approves.
    Proposal { widen: Vec<crate::auto::Widen> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: u32,
    pub kind: QuestionKind,
    pub text: String,
    pub options: Vec<String>,
    pub answer: Option<String>,
    /// "human" when NDSTRL asked the person directly (elicitation or the
    /// window), "agent" when the agent relayed the answer.
    pub answered_via: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Denial {
    pub at: String,
    pub tool: String,
    pub path: String,
    pub reason: String,
}

/// Everything NDSTRL remembers about the open job. Written to
/// `.ndstrl/state.json` after every change so the front office can watch.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub job_id: String,
    pub opened_at: String,
    #[serde(alias = "machine")]
    pub agent: String,
    pub touched: BTreeMap<String, Touch>,
    #[serde(alias = "critic_runs")]
    pub check_runs: Vec<CheckRun>,
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
    /// Edits applied since the last check run; nonzero means its verdict is stale.
    #[serde(default)]
    pub edits_since_inspect: u32,
    /// Usage from earlier sessions of this job (after a handoff or resume).
    #[serde(default)]
    pub carried: Usage,
    /// The transcript whose session was handed off; that session is done.
    #[serde(default)]
    pub handoff_transcript: Option<String>,
    #[serde(default)]
    pub handoff: Option<String>,
    /// Extra files and lines a human approved past the caps.
    #[serde(default)]
    pub files_allowance: usize,
    #[serde(default)]
    pub lines_allowance: usize,
    /// Minutes of runtime a human approved past the time cap.
    #[serde(default)]
    pub minutes_allowance: u64,
    /// Tools a human allowed during the job.
    #[serde(default)]
    pub widened_tools: Vec<String>,
    /// What helpers on other models cost, and what they did.
    #[serde(default)]
    pub delegated: Usage,
    #[serde(default)]
    pub helper_runs: Vec<HelperRecord>,
    /// Messages the person sent from the window, delivered to the agent
    /// with its next tool result.
    #[serde(default)]
    pub notes: Vec<Note>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Note {
    pub at: String,
    pub text: String,
    pub delivered: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HelperRecord {
    pub at: String,
    pub kind: String,
    pub agent: String,
    pub model: String,
    pub usd: Option<f64>,
    pub secs: f64,
    pub ok: bool,
    pub changed: Vec<String>,
    pub reverted: Vec<String>,
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
    pub checks: &'static str,
    pub done: bool,
    pub open_question: Option<u32>,
    pub plan_5h: Option<crate::plan::Estimate>,
    pub plan_7d: Option<crate::plan::Estimate>,
    /// Percentage points of each window this job has used.
    pub job_5h_pts: Option<f64>,
    pub job_7d_pts: Option<f64>,
    /// The agent's live context size, and the handoff limit.
    pub context_tokens: Option<u64>,
    pub max_context_tokens: Option<u64>,
}

impl Gauges {
    pub fn line(&self) -> String {
        let money = match (self.spent_usd, self.budget_usd) {
            (Some(s), Some(b)) => format!("${s:.2} of ${b:.2} at API prices"),
            (Some(s), None) => format!("${s:.2} at API prices, no cap"),
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
        let ctx = match (self.context_tokens, self.max_context_tokens) {
            (Some(c), Some(m)) => format!(" | context {}k of {}k", c / 1000, m / 1000),
            (Some(c), None) => format!(" | context {}k", c / 1000),
            _ => String::new(),
        };
        format!(
            "[ndstrl] {money}{tok} ({}){plan}{ctx} | touched {} | checks {} | done {}{q}",
            self.meter,
            self.files_touched,
            self.checks,
            if self.done { "yes" } else { "no" }
        )
    }
}

pub struct Job {
    pub repo: PathBuf,
    pub spec: JobSpec,
    pub state: State,
    scope: Scope,
    meter: Option<TranscriptMeter>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Job {
    /// Open (or resume) a job in `repo`. If the repo has a spec and
    /// `fresh` is None, it is resumed; otherwise `fresh` becomes the spec.
    pub fn open(repo: &Path, fresh: Option<JobSpec>, agent: &str) -> Result<Self> {
        let repo = repo.canonicalize().with_context(|| format!("no such repo {}", repo.display()))?;
        let existing = JobSpec::load(&repo)?;
        let (mut spec, resumed) = match (fresh, existing) {
            (Some(t), _) => (t, false),
            (None, Some(t)) => (t, true),
            (None, None) => bail!("no spec at {}; give a goal to start one", JobSpec::path(&repo).display()),
        };
        if spec.id.is_empty() {
            spec.id = JobSpec::new_id(&spec.goal);
        }
        spec.save(&repo)?;
        let scope = Scope::new(&repo, &spec.allow, &spec.deny)?;

        let state_path = repo.join(DIR).join("state.json");
        let prior: Option<State> = std::fs::read_to_string(&state_path).ok().and_then(|s| serde_json::from_str(&s).ok());
        let prior_transcript = prior.as_ref().and_then(|p| p.transcript.clone());
        let prior_job = prior.as_ref().map(|p| p.job_id.clone()).unwrap_or_default();
        let mut state = match prior {
            Some(s) if resumed && s.job_id == spec.id => s,
            _ => State { job_id: spec.id.clone(), opened_at: now(), ..Default::default() },
        };
        state.agent = agent.to_string();
        // Resumed in a new session (often after a handoff): carry the old
        // session's spend so the budget stays honest, and start the meter fresh.
        let found_now = TranscriptMeter::discover().map(|p| p.display().to_string());
        if resumed && state.transcript.is_some() && state.transcript != found_now {
            let old = std::mem::take(&mut state.usage);
            state.carried.add(&old);
        }
        if state.plan_at_open.is_none() {
            state.plan_at_open = crate::plan::latest();
        }
        // A fresh job meters the whole harness session: "stop at $4" means
        // the session. Only when an earlier job already ran in this same
        // session does the count start at this job's opening.
        let found = TranscriptMeter::discover();
        let earlier_job_here = prior_transcript.is_some()
            && prior_transcript == found.as_ref().map(|p| p.display().to_string())
            && prior_job != spec.id;
        let since = if earlier_job_here { state.opened_at.clone() } else { String::new() };
        let meter = found.map(|p| TranscriptMeter::new(p, since));
        state.transcript = meter.as_ref().map(|m| m.path().display().to_string());
        let mut job = Job { repo, spec, state, scope, meter };
        job.rescope()?;
        job.refresh();
        job.log("open", serde_json::json!({ "goal": job.spec.goal, "resumed": resumed, "agent": agent }));
        job.persist();
        Ok(job)
    }

    fn rescope(&mut self) -> Result<()> {
        self.scope = Scope::with_widened(&self.repo, &self.spec.allow, &self.spec.deny, &self.state.widened)?;
        Ok(())
    }

    // ---- meter -----------------------------------------------------------

    pub fn refresh(&mut self) {
        self.apply_inbox();
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
        gauges_of(&self.spec, &self.state)
    }

    pub fn remaining_fraction(&self) -> Option<f64> {
        remaining_fraction_of(&self.spec, &self.gauges())
    }

    /// Stops that apply to every tool, reads included: the budget cap and
    /// a session that was handed off because its context grew too large.
    fn hard_stops(&mut self) -> Option<Outcome> {
        if self.state.capped {
            return Some(Outcome::Blocked("this job was stopped (budget, size, or time). The receipt is written. Stop and report.".into()));
        }
        if self.state.transcript.is_some() && self.state.handoff_transcript == self.state.transcript {
            return Some(Outcome::Blocked(format!(
                "this session was handed off because its context got too large. Tell the user to start a fresh session and say \"resume the ndstrl job\". The handoff is in {}.",
                self.state.handoff.as_deref().unwrap_or(".ndstrl/handoff.md")
            )));
        }
        if let Some(max) = self.spec.max_context_tokens {
            let ctx = self.state.usage.last_context;
            if ctx > max && self.state.transcript.is_some() {
                let path = self.write_handoff().unwrap_or_default();
                self.state.handoff_transcript = self.state.transcript.clone();
                self.state.handoff = Some(path.clone());
                self.log("handoff", serde_json::json!({ "context": ctx, "max": max, "path": path }));
                self.persist();
                return Some(Outcome::Blocked(format!(
                    "context is at {}k tokens, past this job's {}k limit. Long sessions drift, so NDSTRL wrote a handoff to {path} and stopped this session. Tell the user to start a fresh session and say \"resume the ndstrl job\"; nothing is lost.",
                    ctx / 1000,
                    max / 1000
                )));
            }
        }
        None
    }

    fn minutes_running(&self) -> u64 {
        chrono::DateTime::parse_from_rfc3339(&self.state.opened_at)
            .map(|t| ((chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds().max(0) / 60) as u64)
            .unwrap_or(0)
    }

    /// Run before every step that spends. Returns a block or a question
    /// if the agent should not proceed.
    fn gate(&mut self, tool: &str) -> Option<Outcome> {
        self.refresh();
        if let Some(b) = self.hard_stops() {
            return Some(b);
        }
        if let Some(q) = self.state.questions.iter().find(|q| q.answer.is_none()) {
            return Some(Outcome::Blocked(format!(
                "question {} is still open: \"{}\". Put it to the user and pass their answer to `answer`.",
                q.id, q.text
            )));
        }
        if let Some(max) = self.spec.max_minutes {
            let m = self.minutes_running();
            if m > max + self.state.minutes_allowance && tool != "receipt" {
                let q = self.ask_inner(
                    QuestionKind::Time { minutes: m },
                    format!("This job has run {m} minutes, past its {max}-minute limit. Keep going, or stop here with a receipt?"),
                    vec!["Keep going".into(), "Stop".into()],
                );
                return Some(Outcome::Ask(q));
            }
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
            let line = if self.state.budget_ok_below.is_some() { ok_below / 2.0 } else { self.spec.ask_below };
            if f < line && tool != "receipt" {
                let pct = (f * 100.0).round();
                let q = self.ask_inner(
                    QuestionKind::Budget { remaining_fraction: f },
                    format!(
                        "Only {pct}% of the budget is left and the job is not done. Keep going, or stop here with a receipt?"
                    ),
                    vec!["continue".into(), "stop".into()],
                );
                return Some(Outcome::Ask(q));
            }
        }
        None
    }

    // ---- questions -------------------------------------------------------

    /// Answers the person gave in the NDSTRL window. The window only
    /// records intent; the rules are applied here, the same as any answer.
    fn apply_inbox(&mut self) {
        let p = self.repo.join(DIR).join("inbox.jsonl");
        let Ok(text) = std::fs::read_to_string(&p) else { return };
        if text.trim().is_empty() {
            return;
        }
        let _ = std::fs::write(&p, "");
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            if let Some(text) = v["steer"].as_str() {
                self.state.notes.push(Note { at: now(), text: text.to_string(), delivered: false });
                self.log("steer", serde_json::json!({ "text": text }));
                continue;
            }
            let (Some(id), Some(ans)) = (v["question_id"].as_u64(), v["answer"].as_str()) else { continue };
            let _ = self.answer(id as u32, ans, "human");
        }
    }

    /// Queue an answer from outside the server (the window). Safe to call
    /// whether or not a server is running; it is applied on the next step.
    pub fn post_answer(repo: &Path, question_id: u32, answer: &str) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(repo.join(DIR).join("inbox.jsonl"))?;
        writeln!(f, "{}", serde_json::json!({ "question_id": question_id, "answer": answer, "at": now() }))
    }

    /// Queue a message for the agent from outside the server (the window).
    pub fn post_steer(repo: &Path, text: &str) -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(repo.join(DIR).join("inbox.jsonl"))?;
        writeln!(f, "{}", serde_json::json!({ "steer": text, "at": now() }))
    }

    /// Messages from the person not yet shown to the agent; marks them shown.
    pub fn take_notes(&mut self) -> Vec<String> {
        self.refresh();
        let out: Vec<String> = self.state.notes.iter().filter(|n| !n.delivered).map(|n| n.text.clone()).collect();
        if !out.is_empty() {
            for n in self.state.notes.iter_mut() {
                n.delivered = true;
            }
            self.persist();
        }
        out
    }

    fn ask_inner(&mut self, kind: QuestionKind, text: String, options: Vec<String>) -> Question {
        let id = self.state.questions.len() as u32 + 1;
        let q = Question { id, kind, text, options, answer: None, answered_via: None };
        self.state.questions.push(q.clone());
        self.log("ask", serde_json::to_value(&q).unwrap_or_default());
        self.persist();
        q
    }

    /// Auto mode: draft the job from the goal and ask the person to approve
    /// it. Reads work while the question is open; edits do not.
    pub fn open_auto(repo: &Path, goal: &str, agent: &str) -> Result<(Self, Question)> {
        let repo = repo.canonicalize()?;
        let p = crate::auto::propose(&repo, goal);
        let mut job = Job::open(&repo, Some(p.spec.clone()), agent)?;
        let q = job.ask_inner(QuestionKind::Proposal { widen: p.widen.clone() }, p.summary(), p.options());
        Ok((job, q))
    }

    /// The agent asks the user something. Always allowed, never paid.
    pub fn ask(&mut self, text: &str, options: Vec<String>) -> Outcome {
        Outcome::Ask(self.ask_inner(QuestionKind::Agent, text.to_string(), options))
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
            QuestionKind::Scope { path } => format!("{path} stays off limits. Finish within the job's scope, or stop and report."),
            QuestionKind::Budget { remaining_fraction } if yes => {
                self.state.budget_ok_below = Some(remaining_fraction);
                "Approved to keep spending. You will be asked again if half of what is left goes too.".into()
            }
            QuestionKind::Budget { .. } => {
                self.state.capped = true;
                let r = self.write_receipt().unwrap_or_default();
                format!("The user chose to stop. Receipt: {r}. Report and end the run.")
            }
            QuestionKind::Agent if answer.trim().eq_ignore_ascii_case("skip") => {
                "The user chose to skip this. Drop that part and continue with the rest of the job.".to_string()
            }
            QuestionKind::Agent => format!("Recorded. The user said: {answer}"),
            QuestionKind::Size { what } if yes || answer.to_ascii_lowercase().starts_with("keep") => {
                if what == "files" {
                    self.state.files_allowance += self.spec.max_files.unwrap_or(1).max(1);
                } else {
                    self.state.lines_allowance += self.spec.max_changed_lines.unwrap_or(50).max(1);
                }
                format!("Approved: the job may grow past its {what} limit (you will be asked again if it doubles).")
            }
            QuestionKind::Time { .. } if yes || answer.to_ascii_lowercase().starts_with("keep") => {
                self.state.minutes_allowance += self.spec.max_minutes.unwrap_or(30).max(1);
                "Approved to keep going. You will be asked again later.".into()
            }
            QuestionKind::Proposal { widen } => {
                let a = answer.trim().to_ascii_lowercase();
                if a.starts_with("cancel") || a.starts_with("no") || a.starts_with("stop") {
                    self.state.capped = true;
                    "The user cancelled the job. Stop and report.".to_string()
                } else {
                    let mut applied = Vec::new();
                    for w in widen.iter().filter(|w| a.contains(&w.option.to_ascii_lowercase()) || w.option.to_ascii_lowercase().contains(&a)) {
                        match &w.apply {
                            crate::auto::Apply::Web => self.state.widened_tools.push("web".into()),
                            crate::auto::Apply::Paths(dirs) => {
                                self.spec.allow.extend(dirs.iter().cloned());
                                self.spec.max_files = self.spec.max_files.map(|m| m * 2);
                                let _ = self.spec.save(&self.repo);
                                let _ = self.rescope();
                            }
                            crate::auto::Apply::Frontier => {
                                self.spec.allow_frontier = true;
                                let _ = self.spec.save(&self.repo);
                            }
                        }
                        applied.push(w.option.clone());
                    }
                    if applied.is_empty() {
                        "Approved as drafted. Go ahead.".to_string()
                    } else {
                        format!("Approved with: {}. Go ahead.", applied.join(", "))
                    }
                }
            }
            QuestionKind::Tool { name } if yes => {
                self.state.widened_tools.push(name.clone());
                format!("{name} is now allowed for this job.")
            }
            QuestionKind::Tool { name } => format!("{name} stays off for this job. Work with what the repo has."),
            QuestionKind::Size { .. } | QuestionKind::Time { .. } => {
                self.state.capped = true;
                let r = self.write_receipt().unwrap_or_default();
                format!("The user chose to stop. Receipt: {r}. Report and end the run.")
            }
        };
        self.log("answer", serde_json::json!({ "id": id, "answer": answer, "via": via }));
        self.persist();
        Outcome::Ok(msg)
    }

    // ---- reads (cheap, metered by NDSTRL) -----------------------------------

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

    /// Reads are allowed while a question is open (the agent may need
    /// them to explain itself) but not after a hard stop.
    fn cap_only(&mut self) -> Option<Outcome> {
        self.refresh();
        self.hard_stops()
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
                "checks already pass, so the job is done. No more edits. Call `receipt` and report.".into(),
            ));
        }
        let rel = self.scope.rel(path).map_err(|e| Outcome::Denied(e.to_string()))?;
        match self.scope.can_edit(&rel) {
            Verdict::Allowed => {
                if let Some(max) = self.spec.max_files {
                    let limit = max + self.state.files_allowance;
                    if !self.state.touched.contains_key(&rel) && self.state.touched.len() >= limit {
                        let q = self.ask_inner(
                            QuestionKind::Size { what: "files".into() },
                            format!("The agent wants to change a new file, {rel}, which would make {} files. This job is capped at {limit}. Allow it to grow?", limit + 1),
                            vec!["Allow more files".into(), "Stop here".into()],
                        );
                        return Err(Outcome::Ask(q));
                    }
                }
                Ok(rel)
            }
            Verdict::Forbidden(why) => {
                self.deny(tool, &rel, &why);
                Err(Outcome::Denied(why))
            }
            Verdict::OutOfScope(why) => {
                self.deny(tool, &rel, &why);
                let q = self.ask_inner(
                    QuestionKind::Scope { path: rel.clone() },
                    format!("The agent wants to edit {rel}, which this job keeps off limits. Allow it for this job?"),
                    vec![format!("Allow {rel}"), "Keep it off limits".into()],
                );
                Err(Outcome::Ask(q))
            }
        }
    }

    fn deny(&mut self, tool: &str, path: &str, reason: &str) {
        self.state.denials.push(Denial { at: now(), tool: tool.into(), path: path.into(), reason: reason.into() });
        self.log("deny", serde_json::json!({ "tool": tool, "path": path, "reason": reason }));
    }

    /// Ask before an edit pushes the job past its changed-lines cap.
    fn size_gate(&mut self, rel: &str, before: Option<&str>, after: &str) -> Option<Outcome> {
        let max = self.spec.max_changed_lines?;
        let limit = max + self.state.lines_allowance;
        let (a, r) = diff_counts(before, after);
        let so_far: usize = self.state.touched.values().map(|t| t.added + t.removed).sum();
        if so_far + a + r <= limit {
            return None;
        }
        let q = self.ask_inner(
            QuestionKind::Size { what: "lines".into() },
            format!(
                "This edit to {rel} (+{a} -{r}) would bring the job to {} changed lines. It is capped at {limit}. Allow it to grow?",
                so_far + a + r
            ),
            vec!["Allow more changes".into(), "Stop here".into()],
        );
        Some(Outcome::Ask(q))
    }

    fn record_touch(&mut self, rel: &str, before: Option<&str>, after: &str) {
        let (add, rem) = diff_counts(before, after);
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
        if let Some(q) = self.size_gate(&rel, Some(&before), &after) {
            return q;
        }
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
        if let Some(q) = self.size_gate(&rel, before.as_deref(), content) {
            return q;
        }
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

    // ---- checks ----------------------------------------------------------

    pub fn inspect(&mut self) -> Outcome {
        if let Some(b) = self.gate("inspect") {
            return b;
        }
        let Some(cmd) = self.spec.test.clone() else {
            return Outcome::Denied("the job has no test command, so NDSTRL cannot run checks. Ask the user what done looks like.".into());
        };
        self.log("inspect_start", serde_json::json!({ "command": cmd }));
        self.persist();
        let run = checks::run(&self.repo, &cmd, Duration::from_secs(self.spec.check_timeout_secs));
        // The job's own test passing is not enough if a guard suite is set:
        // a fix here must not break things elsewhere.
        let guard_run = match (&self.spec.guard, run.passed) {
            (Some(g), true) => Some(checks::run(&self.repo, g, Duration::from_secs(self.spec.check_timeout_secs))),
            _ => None,
        };
        if let Some(gr) = &guard_run {
            if !gr.passed {
                let text = format!(
                    "The job's test passes, but the guard suite fails (exit {:?}): `{}`. Something else broke; fix it before the job counts as done.\n{}",
                    gr.exit_code, gr.command, gr.summary
                );
                self.log("inspect", serde_json::json!({ "passed": false, "secs": run.secs + gr.secs, "exit": gr.exit_code, "guard": true }));
                self.state.check_runs.push(run);
                self.state.check_runs.push(gr.clone());
                self.state.edits_since_inspect = 0;
                self.charge(&text);
                self.persist();
                return Outcome::Ok(text);
            }
        }
        let passed = run.passed;
        let text = if passed {
            self.state.done = true;
            format!(
                "PASS in {:.1}s: `{}`. The job is done; edits are now closed. Call `receipt`.\n{}",
                run.secs, run.command, run.summary
            )
        } else if run.timed_out {
            format!("TIMED OUT after {:.0}s: `{}`\n{}", run.secs, run.command, run.summary)
        } else {
            format!("FAIL (exit {:?}) in {:.1}s: `{}`\n{}", run.exit_code, run.secs, run.command, run.summary)
        };
        self.log("inspect", serde_json::json!({ "passed": passed, "secs": run.secs, "exit": run.exit_code }));
        self.state.check_runs.push(run);
        self.state.edits_since_inspect = 0;
        self.charge(&text);
        self.persist();
        Outcome::Ok(text)
    }

    // ---- receipt ---------------------------------------------------------

    pub fn write_receipt(&mut self) -> Result<String> {
        self.refresh();
        let g = self.gauges();
        let t = &self.spec;
        let s = &self.state;
        let dir = self.repo.join(DIR).join("receipts");
        std::fs::create_dir_all(&dir)?;
        let mut md = String::new();
        use std::fmt::Write as _;
        let _ = writeln!(md, "# Receipt {}\n", t.id);
        let _ = writeln!(md, "- Goal: {}", t.goal);
        let _ = writeln!(md, "- Agent: {}", s.agent);
        let _ = writeln!(md, "- Opened: {}  Closed: {}", s.opened_at, now());
        let _ = writeln!(
            md,
            "- Result: {}",
            if s.done { "checks passed" } else if s.capped { "stopped at the cap" } else { "not done" }
        );
        let _ = writeln!(md, "\n## Spend\n");
        let cap = t.budget_usd.map(|b| format!("${b:.2}")).unwrap_or_else(|| "none".into());
        match g.spent_usd {
            Some(v) => {
                let _ = writeln!(md, "- ${v:.4} at API list price (a usage proxy on subscription plans, not a bill), budget {cap}");
            }
            None => {
                let _ = writeln!(md, "- dollars not metered for this agent; budget {cap}");
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
        let _ = writeln!(md, "- NDSTRL tool output sent to the model (estimate, tokens): {}", s.factory_tokens);
        let _ = writeln!(md, "\n## Files\n");
        if s.touched.is_empty() {
            let _ = writeln!(md, "- none");
        }
        for (p, t) in &s.touched {
            let _ = writeln!(md, "- {p}  +{} -{}{}", t.added, t.removed, if t.created { " (new)" } else { "" });
        }
        let _ = writeln!(md, "\n## Checks\n");
        for r in &s.check_runs {
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
        if s.receipt.is_none() && (s.done || s.capped) {
            crate::auto::record_history(&crate::auto::HistoryRow {
                files: s.touched.len(),
                lines: s.touched.values().map(|x| x.added + x.removed).sum(),
                usd: g.spent_usd.unwrap_or(s.delegated.usd),
                done: s.done,
                at: now(),
                repo: self.repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                goal: t.goal.clone(),
                agent: s.agent.clone(),
                tokens: g.tokens,
                plan_5h_pts: g.job_5h_pts,
                plan_7d_pts: g.job_7d_pts,
                helper_usd: s.delegated.usd,
                minutes: self.minutes_running(),
            });
        }
        std::fs::write(
            dir.join(format!("{}.json", t.id)),
            serde_json::to_string_pretty(&serde_json::json!({ "spec": t, "state": s, "gauges": g }))?,
        )?;
        let shown = path.strip_prefix(&self.repo).unwrap_or(&path).display().to_string();
        self.state.receipt = Some(shown.clone());
        self.log("receipt", serde_json::json!({ "path": shown }));
        self.persist();
        Ok(shown)
    }

    // ---- helpers on other models ------------------------------------------

    pub fn tool_allowed(&self, name: &str) -> bool {
        self.spec.tools.iter().any(|t| t == name) || self.state.widened_tools.iter().any(|t| t == name)
    }

    /// Hand a piece of work to a cheaper (or stronger) agent. The helper's
    /// cost counts against this job's budget; its edits are checked with git
    /// and anything outside the job's scope is put back.
    pub fn delegate(&mut self, task: &str, kind: &str) -> Outcome {
        let Some(kind) = crate::route::Kind::parse(kind) else {
            return Outcome::Denied("kind must be lookup, research, edit, or hard".into());
        };
        if let Some(b) = self.gate("delegate") {
            return b;
        }
        if kind.edits() && self.state.done {
            return Outcome::Blocked("checks already pass, so the job is done. No more edits. Call `receipt` and report.".into());
        }
        if kind == crate::route::Kind::Research && !self.tool_allowed("web") {
            let q = self.ask_inner(
                QuestionKind::Tool { name: "web".into() },
                format!("The agent wants to look this up on the web: \"{task}\". Allow web access for this job?"),
                vec!["Allow web".into(), "Keep it offline".into()],
            );
            return Outcome::Ask(q);
        }
        let g = self.gauges();
        let route = match crate::route::choose(kind, self.spec.allow_frontier, &crate::route::pressures(), g.remaining_usd) {
            Ok(r) => r,
            Err(e) => return Outcome::Denied(e),
        };
        // Codex helpers read through a shell, so even a look-up could touch
        // files; snapshot those too and put back anything they change.
        let snap = if kind.edits() || route.provider != crate::route::Provider::Claude {
            match git::Snapshot::take(&self.repo) {
                Some(s) => Some(s),
                None => return Outcome::Denied("helpers may only edit inside a git repo, so NDSTRL can check and undo their changes. Do the edit yourself with `edit`.".into()),
            }
        } else {
            None
        };
        let mut allow = self.spec.allow.clone();
        allow.extend(self.state.widened.iter().cloned());
        let prompt = crate::route::brief(route.provider, kind, &self.spec.goal, task, &allow, &self.spec.deny);
        self.log("delegate_start", serde_json::json!({ "kind": kind, "agent": route.provider, "model": route.model, "task": task }));
        self.persist();
        let run = match route.provider {
            crate::route::Provider::Claude => crate::route::run_claude(&self.repo, &route, kind, &prompt),
            crate::route::Provider::Codex => crate::route::run_codex(&self.repo, &route, &prompt),
            _ => crate::route::run_other(&route),
        };
        self.state.delegated.add(&run.usage);

        let (mut changed, mut reverted) = (Vec::new(), Vec::new());
        if let Some(snap) = snap {
            for c in snap.changes(&self.repo) {
                if c.path.starts_with(".ndstrl/") {
                    continue;
                }
                let verdict = if kind.edits() { self.scope.can_edit(&c.path) } else { Verdict::Forbidden("look-up helpers may not change files".into()) };
                if verdict == Verdict::Allowed {
                    let before = c.before.as_deref().map(|b| String::from_utf8_lossy(b).into_owned());
                    let after = c.after.as_deref().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
                    self.record_touch(&c.path, before.as_deref(), &after);
                    changed.push(c.path.clone());
                } else {
                    c.restore(&self.repo);
                    self.deny("delegate", &c.path, "a helper changed it outside the job's scope; NDSTRL put it back");
                    reverted.push(c.path.clone());
                }
            }
        }
        let model_short = route.model.trim_start_matches("claude-").to_string();
        self.state.helper_runs.push(HelperRecord {
            at: now(),
            kind: format!("{kind:?}").to_lowercase(),
            agent: route.provider.label().into(),
            model: route.model.clone(),
            usd: run.usd,
            secs: run.secs,
            ok: run.ok,
            changed: changed.clone(),
            reverted: reverted.clone(),
        });
        self.log("delegate", serde_json::json!({ "model": route.model, "usd": run.usd, "secs": run.secs, "ok": run.ok, "changed": changed, "reverted": reverted }));
        let cost = match (run.usd, route.provider) {
            (Some(u), _) => format!("${u:.3}"),
            (None, crate::route::Provider::Codex) => format!("{} tokens on your ChatGPT plan", run.usage.tokens()),
            _ => "cost unknown".into(),
        };
        let mut text = format!("Helper: {} {model_short} ({}), {cost}, {:.0}s.\n", route.provider.label(), route.why, run.secs);
        match &run.error {
            Some(e) if run.summary.is_empty() => text.push_str(&format!("It failed: {e}\n")),
            _ => text.push_str(&run.summary),
        }
        if !changed.is_empty() {
            text.push_str(&format!("\nChanged (recorded on the job): {}", changed.join(", ")));
        }
        if !reverted.is_empty() {
            text.push_str(&format!("\nPut back (outside the job's scope): {}", reverted.join(", ")));
        }
        if kind.edits() && !changed.is_empty() {
            text.push_str("\nRun `run_checks` to verify.");
        }
        self.charge(&text);
        self.persist();
        Outcome::Ok(text)
    }

    // ---- handoff ---------------------------------------------------------

    /// A short note a fresh session can start from: the goal, the limits,
    /// what changed, where the checks stand, and the last few steps.
    pub fn write_handoff(&mut self) -> Result<String> {
        use std::fmt::Write as _;
        let s = &self.state;
        let t = &self.spec;
        let mut md = String::new();
        let _ = writeln!(md, "# Handoff for job {}\n", t.id);
        let _ = writeln!(md, "Goal: {}\n", t.goal);
        let _ = writeln!(md, "Editable: {}", if t.allow.is_empty() { "whole repo".into() } else { t.allow.join(", ") });
        if !t.deny.is_empty() || !s.widened.is_empty() {
            let _ = writeln!(md, "Off limits: {}  (allowed by the user: {})", t.deny.join(", "), s.widened.join(", "));
        }
        let _ = writeln!(md, "Check command: {}\n", t.test.as_deref().unwrap_or("none"));
        let _ = writeln!(md, "## Changed so far");
        if s.touched.is_empty() {
            let _ = writeln!(md, "- nothing yet");
        }
        for (p, x) in &s.touched {
            let _ = writeln!(md, "- {p} (+{} -{})", x.added, x.removed);
        }
        let _ = writeln!(md, "\n## Checks");
        match s.check_runs.last() {
            None => {
                let _ = writeln!(md, "Not run yet.");
            }
            Some(r) => {
                let stale = if s.edits_since_inspect > 0 { format!(" ({} edits since; rerun)", s.edits_since_inspect) } else { String::new() };
                let _ = writeln!(md, "Last run: {}{stale}\n```\n{}\n```", if r.passed { "PASS" } else { "FAIL" }, r.summary);
            }
        }
        let answered: Vec<_> = s.questions.iter().filter(|q| q.answer.is_some()).collect();
        if !answered.is_empty() {
            let _ = writeln!(md, "\n## Decisions the user made");
            for q in answered {
                let _ = writeln!(md, "- {} -> {}", q.text, q.answer.as_deref().unwrap_or(""));
            }
        }
        let _ = writeln!(md, "\n## Next\nStart from the failing check or the goal. Re-read only the files listed above; do not re-explore the repo.");
        let path = self.repo.join(DIR).join("handoff.md");
        std::fs::write(&path, md)?;
        Ok(format!("{DIR}/handoff.md"))
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
            let _ = writeln!(f, "{}", serde_json::json!({ "at": now(), "job": self.spec.id, "kind": kind, "data": data }));
        }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }
}

/// Git snapshots so helper edits can be checked and undone.
mod git {
    use std::collections::HashMap;
    use std::path::Path;
    use std::process::Command;

    fn dirty(repo: &Path) -> Option<Vec<String>> {
        let o = Command::new("git")
            .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
            .current_dir(repo)
            .output()
            .ok()?;
        if !o.status.success() {
            return None;
        }
        let mut out = Vec::new();
        let mut parts = o.stdout.split(|b| *b == 0).filter(|p| !p.is_empty());
        while let Some(p) = parts.next() {
            if p.len() < 4 {
                continue;
            }
            let renamed = p[0] == b'R' || p[0] == b'C';
            out.push(String::from_utf8_lossy(&p[3..]).into_owned());
            if renamed {
                parts.next(); // the old path
            }
        }
        Some(out)
    }

    fn head(repo: &Path, path: &str) -> Option<Vec<u8>> {
        let o = Command::new("git").args(["show", &format!("HEAD:{path}")]).current_dir(repo).output().ok()?;
        o.status.success().then_some(o.stdout)
    }

    pub struct Snapshot {
        before: HashMap<String, Option<Vec<u8>>>,
    }

    pub struct Change {
        pub path: String,
        pub before: Option<Vec<u8>>,
        pub after: Option<Vec<u8>>,
    }

    impl Change {
        /// Put the file back exactly as it was before the helper ran.
        pub fn restore(&self, repo: &Path) {
            let p = repo.join(&self.path);
            match &self.before {
                Some(b) => {
                    let _ = std::fs::write(p, b);
                }
                None => {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    }

    impl Snapshot {
        pub fn take(repo: &Path) -> Option<Snapshot> {
            let paths = dirty(repo)?;
            let before = paths.into_iter().map(|p| {
                let c = std::fs::read(repo.join(&p)).ok();
                (p, c)
            });
            Some(Snapshot { before: before.collect() })
        }

        /// Files whose content differs from before the helper ran.
        pub fn changes(&self, repo: &Path) -> Vec<Change> {
            let now = dirty(repo).unwrap_or_default();
            let mut paths: Vec<String> = now.clone();
            for p in self.before.keys() {
                if !paths.contains(p) {
                    paths.push(p.clone());
                }
            }
            let mut out = Vec::new();
            for p in paths {
                let after = std::fs::read(repo.join(&p)).ok();
                let before = match self.before.get(&p) {
                    Some(b) => b.clone(),
                    None => head(repo, &p),
                };
                if before != after {
                    out.push(Change { path: p, before, after });
                }
            }
            out
        }
    }
}

fn diff_counts(before: Option<&str>, after: &str) -> (usize, usize) {
    let (mut add, mut rem) = (0, 0);
    for c in similar::TextDiff::from_lines(before.unwrap_or(""), after).iter_all_changes() {
        match c.tag() {
            similar::ChangeTag::Insert => add += 1,
            similar::ChangeTag::Delete => rem += 1,
            _ => {}
        }
    }
    (add, rem)
}

// ---- pure views, shared by the server and the window -----------------------

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

/// Gauges from the spec and saved state alone. The server and the
/// front office both call this, so they can never disagree.
pub fn gauges_of(t: &JobSpec, s: &State) -> Gauges {
    let has_meter = s.transcript.is_some();
    // Helper runs report their own exact cost, so they count even when the
    // main agent's transcript cannot be read.
    let spent = if has_meter && s.usage.unpriced_responses == 0 && s.carried.unpriced_responses == 0 {
        Some(s.usage.usd + s.carried.usd + s.delegated.usd)
    } else if s.delegated.responses > 0 {
        Some(s.delegated.usd)
    } else {
        None
    };
    let tokens = if has_meter { s.usage.tokens() + s.carried.tokens() + s.delegated.tokens() } else { s.factory_tokens + s.delegated.tokens() };
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
        meter: if has_meter { "transcript" } else if s.delegated.responses > 0 { "helpers only; main agent unmetered" } else { "NDSTRL estimate" },
        files_touched: s.touched.len(),
        checks: match s.check_runs.last() {
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
        context_tokens: has_meter.then_some(s.usage.last_context),
        max_context_tokens: t.max_context_tokens,
    }
}

/// Fraction of the tightest budget left: list dollars, tokens, or plan
/// window points, whichever is closest to its cap.
pub fn remaining_fraction_of(t: &JobSpec, g: &Gauges) -> Option<f64> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The meter reads process environment; tests that open jobs take turns.
    static ENV: Mutex<()> = Mutex::new(());

    /// Tests may run inside a Claude Code session; never meter that one.
    fn isolate() -> std::sync::MutexGuard<'static, ()> {
        let g = ENV.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CLAUDE_CODE_SESSION_ID");
        std::env::remove_var("NDSTRL_TRANSCRIPT");
        g
    }

    fn transcript_line(rid: &str, context: u64) -> String {
        format!(
            r#"{{"type":"assistant","requestId":"{rid}","timestamp":"2099-01-01T00:00:00.000Z","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":0,"output_tokens":1000,"cache_read_input_tokens":{context},"cache_creation":{{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}}}}}}}}"#
        )
    }

    #[test]
    fn context_ceiling_hands_off_and_a_fresh_session_resumes() {
        let _g = isolate();
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::write(d.path().join("src/a.rs"), "a\n").unwrap();
        let t1 = d.path().join("t1.jsonl");
        std::fs::write(&t1, transcript_line("r1", 50_000) + "\n").unwrap();
        std::env::set_var("NDSTRL_TRANSCRIPT", &t1);
        let spec = JobSpec { goal: "fix a".into(), max_context_tokens: Some(100_000), ..Default::default() };
        let mut job = Job::open(d.path(), Some(spec), "test").unwrap();
        assert!(matches!(job.edit("src/a.rs", "a", "b", false), Outcome::Ok(_)));
        // The session's context balloons past the ceiling.
        std::fs::write(&t1, transcript_line("r1", 50_000) + "\n" + &transcript_line("r2", 180_000) + "\n").unwrap();
        match job.read("src/a.rs", None, None) {
            Outcome::Blocked(m) => assert!(m.contains("handoff"), "{m}"),
            other => panic!("expected a handoff block, got {other:?}"),
        }
        let note = std::fs::read_to_string(d.path().join(".ndstrl/handoff.md")).unwrap();
        assert!(note.contains("fix a") && note.contains("src/a.rs"));
        let spent_before = job.gauges().spent_usd.unwrap();
        // A fresh session resumes: new transcript, small context, spend carried.
        let t2 = d.path().join("t2.jsonl");
        std::fs::write(&t2, transcript_line("s1", 20_000) + "\n").unwrap();
        std::env::set_var("NDSTRL_TRANSCRIPT", &t2);
        let mut job = Job::open(d.path(), None, "test").unwrap();
        assert!(matches!(job.edit("src/a.rs", "b", "c", false), Outcome::Ok(_)));
        assert!(job.gauges().spent_usd.unwrap() > spent_before);
        std::env::remove_var("NDSTRL_TRANSCRIPT");
    }

    #[test]
    fn blast_radius_and_guard() {
        let _g = isolate();
        let d = tempfile::tempdir().unwrap();
        for f in ["a", "b", "c"] {
            std::fs::write(d.path().join(f), "x\n").unwrap();
        }
        let spec = JobSpec {
            goal: "small fix".into(),
            max_files: Some(2),
            test: Some("true".into()),
            guard: Some("test -f guard_ok".into()),
            ..Default::default()
        };
        let mut job = Job::open(d.path(), Some(spec), "test").unwrap();
        assert!(matches!(job.edit("a", "x", "y", false), Outcome::Ok(_)));
        assert!(matches!(job.edit("b", "x", "y", false), Outcome::Ok(_)));
        let q = match job.edit("c", "x", "y", false) {
            Outcome::Ask(q) => q,
            other => panic!("third file should ask, got {other:?}"),
        };
        assert!(matches!(job.answer(q.id, "Stop here", "human"), Outcome::Ok(_)));
        assert!(job.state.capped);

        let d = tempfile::tempdir().unwrap();
        let spec = JobSpec { goal: "g".into(), test: Some("true".into()), guard: Some("test -f guard_ok".into()), ..Default::default() };
        let mut job = Job::open(d.path(), Some(spec), "test").unwrap();
        match job.inspect() {
            Outcome::Ok(t) => assert!(t.contains("guard suite fails"), "{t}"),
            o => panic!("{o:?}"),
        }
        assert!(!job.state.done);
        std::fs::write(d.path().join("guard_ok"), "").unwrap();
        assert!(matches!(job.inspect(), Outcome::Ok(t) if t.starts_with("PASS")));
        assert!(job.state.done);
    }

    #[test]
    fn helper_edits_outside_scope_are_put_back() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git").args(["-c", "user.email=t@t", "-c", "user.name=t"]).args(args).current_dir(r).output().unwrap()
        };
        git(&["init", "-q"]);
        std::fs::write(r.join("in.rs"), "a\n").unwrap();
        std::fs::write(r.join("out.rs"), "b\n").unwrap();
        std::fs::write(r.join("dirty.rs"), "c\n").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "base"]);
        // A file the person was already editing, before the helper ran.
        std::fs::write(r.join("dirty.rs"), "mine\n").unwrap();
        let snap = git::Snapshot::take(r).unwrap();
        // The helper changes everything it can reach.
        std::fs::write(r.join("in.rs"), "A\n").unwrap();
        std::fs::write(r.join("out.rs"), "B\n").unwrap();
        std::fs::write(r.join("dirty.rs"), "helper\n").unwrap();
        std::fs::write(r.join("new.rs"), "n\n").unwrap();
        let mut ch = snap.changes(r);
        ch.sort_by(|a, b| a.path.cmp(&b.path));
        let paths: Vec<_> = ch.iter().map(|c| c.path.as_str()).collect();
        assert_eq!(paths, ["dirty.rs", "in.rs", "new.rs", "out.rs"]);
        for c in ch.iter().filter(|c| c.path != "in.rs") {
            c.restore(r);
        }
        assert_eq!(std::fs::read_to_string(r.join("in.rs")).unwrap(), "A\n");
        assert_eq!(std::fs::read_to_string(r.join("out.rs")).unwrap(), "b\n");
        assert_eq!(std::fs::read_to_string(r.join("dirty.rs")).unwrap(), "mine\n");
        assert!(!r.join("new.rs").exists());
    }

    #[test]
    fn window_message_reaches_the_agent_once() {
        let _g = isolate();
        let d = tempfile::tempdir().unwrap();
        let mut job = Job::open(d.path(), Some(JobSpec { goal: "g".into(), ..Default::default() }), "test").unwrap();
        Job::post_steer(d.path(), "Skip the CSS; only fix the logic").unwrap();
        assert_eq!(job.take_notes(), vec!["Skip the CSS; only fix the logic".to_string()]);
        assert!(job.take_notes().is_empty());
        let q = match job.ask("Refactor the form too?", vec!["yes".into(), "no".into()]) {
            Outcome::Ask(q) => q,
            o => panic!("{o:?}"),
        };
        Job::post_answer(d.path(), q.id, "skip").unwrap();
        job.refresh();
        assert_eq!(job.state.questions[0].answer.as_deref(), Some("skip"));
    }

    #[test]
    fn window_answer_reaches_the_server() {
        let _g = isolate();
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("src")).unwrap();
        std::fs::write(d.path().join("README.md"), "old\n").unwrap();
        let spec = JobSpec { goal: "g".into(), allow: vec!["src".into()], ..Default::default() };
        let mut job = Job::open(d.path(), Some(spec), "test").unwrap();
        let q = match job.edit("README.md", "old", "new", false) {
            Outcome::Ask(q) => q,
            other => panic!("expected a question, got {other:?}"),
        };
        assert!(matches!(job.edit("src/a.rs", "x", "y", false), Outcome::Blocked(_)));
        Job::post_answer(d.path(), q.id, "allow README.md").unwrap();
        assert!(matches!(job.edit("README.md", "old", "new", false), Outcome::Ok(_)));
        let answered = &job.state.questions[0];
        assert_eq!(answered.answered_via.as_deref(), Some("human"));
    }
}
