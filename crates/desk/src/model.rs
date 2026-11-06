//! Read-only view of one repo's floor. The desk never writes factory
//! state; it reads the same files the server writes and computes gauges
//! with the same core functions.

use ndstrl_core::job::{gauges_of, remaining_fraction_of, State};
use ndstrl_core::{Gauges, JobSpec};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct View {
    pub repo: PathBuf,
    pub repo_name: String,
    pub spec: Option<JobSpec>,
    pub state: Option<State>,
    pub gauges: Option<Gauges>,
    pub remaining: Option<f64>,
    pub plan: Option<ndstrl_core::plan::Snapshot>,
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
    let spec = JobSpec::load(repo).ok().flatten();
    let state: Option<State> = std::fs::read_to_string(dir.join("state.json")).ok().and_then(|s| serde_json::from_str(&s).ok());
    // A state left over from an earlier spec is not this job.
    let state = match (&spec, state) {
        (Some(t), Some(s)) if s.job_id == t.id => Some(s),
        _ => None,
    };
    let gauges = match (&spec, &state) {
        (Some(t), Some(s)) => Some(gauges_of(t, s)),
        _ => None,
    };
    let remaining = match (&spec, &gauges) {
        (Some(t), Some(g)) => remaining_fraction_of(t, g),
        _ => None,
    };
    View {
        repo_name: repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        repo: repo.to_path_buf(),
        plan: ndstrl_core::plan::latest(),
        spec,
        state,
        gauges,
        remaining,
    }
}
