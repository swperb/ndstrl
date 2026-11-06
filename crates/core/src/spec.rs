//! The job spec: the contract every agent has to honor.
//!
//! It lives at `<repo>/.ndstrl/job.toml` so it survives the session, the
//! harness, and the model. Nothing in here is a prompt; every field is
//! enforced by NDSTRL's tools.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DIR: &str = ".ndstrl";
pub const FILE: &str = "job.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobSpec {
    /// Stable id for receipts. Generated when the job is first opened.
    #[serde(default)]
    pub id: String,
    /// What done means, in the user's words.
    pub goal: String,
    /// Globs the agent may edit. Empty means the whole repo.
    #[serde(default)]
    pub allow: Vec<String>,
    /// Globs the agent may never edit, even if `allow` matches.
    #[serde(default)]
    pub deny: Vec<String>,
    /// The check command. Exit code 0 means the job is done.
    #[serde(default)]
    pub test: Option<String>,
    /// Cap in API list-price dollars: real transcript tokens priced at
    /// list rates. On subscription plans this is a usage proxy, not a bill.
    #[serde(default)]
    pub budget_usd: Option<f64>,
    /// Token cap, for harnesses whose dollars we cannot read.
    #[serde(default)]
    pub budget_tokens: Option<u64>,
    /// Cap in percentage points of the plan's 5-hour window this job may use.
    #[serde(default)]
    pub budget_plan_5h_pct: Option<f64>,
    /// Cap in percentage points of the plan's weekly window this job may use.
    #[serde(default)]
    pub budget_plan_7d_pct: Option<f64>,
    /// Hand off to a fresh session once the agent's context passes this
    /// many tokens. Long sessions drift; a clean restart with a handoff
    /// note is cheaper than a confused agent.
    #[serde(default = "default_max_context")]
    pub max_context_tokens: Option<u64>,
    /// Ask before the job touches more than this many files.
    #[serde(default)]
    pub max_files: Option<usize>,
    /// Ask before the job's total changed lines pass this.
    #[serde(default)]
    pub max_changed_lines: Option<usize>,
    /// Ask before the job runs longer than this many minutes.
    #[serde(default)]
    pub max_minutes: Option<u64>,
    /// A broader command (usually the full test suite) that must also pass
    /// before the job counts as done, so a fix here cannot break things there.
    #[serde(default)]
    pub guard: Option<String>,
    /// Extra tools this job may use, beyond reading and editing code.
    /// "web" lets helpers search and read docs online.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Whether hard problems may go to the frontier model (Fable). Off by
    /// default: one Fable question can eat a big share of a week's plan.
    #[serde(default)]
    pub allow_frontier: bool,
    /// Whether checks may use the network. Recorded, and enforced
    /// where the platform allows it.
    #[serde(default)]
    pub network: bool,
    /// Ask before a step once less than this fraction of the budget is left.
    #[serde(default = "default_ask_below")]
    pub ask_below: f64,
    /// Seconds before a check run is killed.
    #[serde(default = "default_check_timeout")]
    #[serde(alias = "critic_timeout_secs")]
    pub check_timeout_secs: u64,
}

fn default_max_context() -> Option<u64> {
    Some(160_000)
}
fn default_ask_below() -> f64 {
    0.25
}
fn default_check_timeout() -> u64 {
    600
}

impl Default for JobSpec {
    fn default() -> Self {
        Self {
            id: String::new(),
            goal: String::new(),
            allow: vec![],
            deny: vec![],
            test: None,
            budget_usd: None,
            budget_tokens: None,
            budget_plan_5h_pct: None,
            budget_plan_7d_pct: None,
            max_context_tokens: default_max_context(),
            max_files: None,
            max_changed_lines: None,
            max_minutes: None,
            guard: None,
            tools: vec![],
            allow_frontier: false,
            network: false,
            ask_below: default_ask_below(),
            check_timeout_secs: default_check_timeout(),
        }
    }
}

impl JobSpec {
    pub fn path(repo: &Path) -> PathBuf {
        repo.join(DIR).join(FILE)
    }

    pub fn load(repo: &Path) -> Result<Option<Self>> {
        let p = Self::path(repo);
        if !p.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
        let t: JobSpec = toml::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
        t.validate()?;
        Ok(Some(t))
    }

    pub fn save(&self, repo: &Path) -> Result<()> {
        self.validate()?;
        let dir = repo.join(DIR);
        std::fs::create_dir_all(&dir)?;
        ensure_gitignore(&dir)?;
        let text = format!(
            "# NDSTRL job spec. NDSTRL enforces this; agents cannot edit it.\n{}",
            toml::to_string_pretty(self)?
        );
        std::fs::write(Self::path(repo), text)?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.goal.trim().is_empty() {
            bail!("the spec needs a goal");
        }
        if let Some(b) = self.budget_usd {
            if !(b > 0.0) {
                bail!("budget_usd must be positive");
            }
        }
        if !(0.0..1.0).contains(&self.ask_below) {
            bail!("ask_below must be in [0, 1)");
        }
        Ok(())
    }

    pub fn new_id(goal: &str) -> String {
        let slug: String = goal
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .take(5)
            .collect::<Vec<_>>()
            .join("-");
        format!("{}-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S"), slug)
    }
}

/// Runtime files (state, ledger, receipts) stay out of git; the spec
/// itself is meant to be committed so the job can move between agents.
fn ensure_gitignore(dir: &Path) -> Result<()> {
    let gi = dir.join(".gitignore");
    if !gi.exists() {
        std::fs::write(gi, "state.json\nledger.jsonl\ninbox.jsonl\n")?;
    }
    Ok(())
}
