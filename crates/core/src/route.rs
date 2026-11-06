//! Routing: send each piece of work to the cheapest agent that can do it
//! well, across the subscriptions the person already pays for.
//!
//! The main agent keeps the judgment. Look-ups, research, and mechanical
//! edits go to a helper running headless on a cheaper model, which returns a
//! short summary instead of filling the main session's context.

use crate::meter::Usage;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

/// What kind of help the main agent is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Find and summarize code. Read-only.
    Lookup,
    /// Read external docs or the web. Read-only, needs `web` in the job's tools.
    Research,
    /// Apply a change that is already decided. Edits, verified with git.
    Edit,
    /// A real problem: debugging, design. Strong model, edits allowed.
    Hard,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "lookup" => Some(Kind::Lookup),
            "research" => Some(Kind::Research),
            "edit" => Some(Kind::Edit),
            "hard" => Some(Kind::Hard),
            _ => None,
        }
    }
    pub fn edits(self) -> bool {
        matches!(self, Kind::Edit | Kind::Hard)
    }
}

/// A subscription NDSTRL can drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Claude,
    Codex,
    Cursor,
}

impl Provider {
    pub fn bin(self) -> &'static str {
        match self {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
            Provider::Cursor => "cursor-agent",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Provider::Claude => "Claude",
            Provider::Codex => "Codex (ChatGPT plan)",
            Provider::Cursor => "Cursor",
        }
    }
    pub fn installed(self) -> bool {
        which(self.bin())
    }
}

fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(bin).is_file() || d.join(format!("{bin}.exe")).is_file()))
        .unwrap_or(false)
}

/// Where one piece of work goes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Route {
    pub provider: Provider,
    pub model: String,
    /// Hard cap for this one helper run, in list-price dollars.
    pub cap_usd: f64,
    pub why: String,
}

/// How full one subscription is: the fuller of its 5-hour and weekly
/// windows, 0 to 100. `None` when NDSTRL has no reading for it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Pressure {
    pub provider: Provider,
    pub used_pct: Option<f64>,
}

/// Unknown usage counts as half full: not preferred over a provider we
/// know is nearly idle, not avoided like one we know is nearly spent.
const UNKNOWN_PCT: f64 = 50.0;

/// Current pressure on every installed provider.
pub fn pressures() -> Vec<Pressure> {
    let claude = crate::plan::latest().and_then(|s| {
        let now = chrono::Utc::now().timestamp();
        let live = |w: Option<crate::plan::Window>| w.filter(|w| w.resets_at == 0 || w.resets_at > now).map(|w| w.used_percentage);
        match (live(s.five_hour), live(s.seven_day)) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0).max(b.unwrap_or(0.0))),
        }
    });
    let codex = crate::plan::home()
        .and_then(|h| std::fs::read_to_string(h.join("codex-limits.json")).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["used_pct"].as_f64());
    [(Provider::Claude, claude), (Provider::Codex, codex), (Provider::Cursor, None)]
        .into_iter()
        .filter(|(p, _)| p.installed())
        .map(|(provider, used_pct)| Pressure { provider, used_pct })
        .collect()
}

fn can_do(p: Provider, kind: Kind) -> bool {
    match p {
        Provider::Claude => true,
        // Web research is wired up on Claude only for now.
        Provider::Codex | Provider::Cursor => kind != Kind::Research,
    }
}

fn model_for(p: Provider, kind: Kind, allow_frontier: bool) -> (&'static str, f64, &'static str) {
    match (p, kind) {
        (Provider::Claude, Kind::Lookup) => ("claude-haiku-5-5", 0.10, "look-ups need reading, not judgment"),
        (Provider::Claude, Kind::Research) => ("claude-haiku-5-5", 0.15, "docs research needs reading, not judgment"),
        (Provider::Claude, Kind::Edit) => ("claude-sonnet-5-5", 0.40, "the change is already decided"),
        (Provider::Claude, Kind::Hard) if allow_frontier => ("claude-fable-5-1", 2.00, "hard problem, frontier model approved for this job"),
        (Provider::Claude, Kind::Hard) => ("claude-opus-5-5", 1.00, "hard problem"),
        (_, Kind::Hard) => ("default", 1.00, "hard problem"),
        (_, _) => ("default", 0.40, "routine work"),
    }
}

/// Pick the route: among installed providers that can do this kind of
/// work, the one whose subscription is least used, so no plan runs out
/// while another sits idle. Ties go to Claude, whose cost NDSTRL can meter.
pub fn choose(kind: Kind, allow_frontier: bool, pressure: &[Pressure], remaining_usd: Option<f64>) -> Result<Route, String> {
    let mut options: Vec<&Pressure> = pressure.iter().filter(|p| can_do(p.provider, kind)).collect();
    if options.is_empty() {
        return Err("no helper agent that can do this is installed (looked for claude, codex, cursor-agent)".into());
    }
    let pct = |p: &Pressure| p.used_pct.unwrap_or(UNKNOWN_PCT);
    options.sort_by(|a, b| {
        pct(a).partial_cmp(&pct(b)).unwrap().then_with(|| (a.provider != Provider::Claude).cmp(&(b.provider != Provider::Claude)))
    });
    let best = options[0];
    let (model, cap, why) = model_for(best.provider, kind, allow_frontier);
    let cap = remaining_usd.map(|r| cap.min(r.max(0.0))).unwrap_or(cap);
    if cap <= 0.0 {
        return Err("no budget left for a helper".into());
    }
    let why = if options.len() > 1 {
        let others: Vec<String> = options[1..]
            .iter()
            .map(|o| format!("{} {}", o.provider.label(), o.used_pct.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "unknown".into())))
            .collect();
        format!(
            "{why}; {} is the least used plan ({}) vs {}",
            best.provider.label(),
            best.used_pct.map(|p| format!("{p:.0}%")).unwrap_or_else(|| "unknown".into()),
            others.join(", ")
        )
    } else {
        why.to_string()
    };
    Ok(Route { provider: best.provider, model: model.into(), cap_usd: cap, why })
}

/// Which harness the main session should run in, when another plan has
/// far more room than the one in use. NDSTRL cannot move the main session;
/// it can say so.
pub fn main_session_hint(pressure: &[Pressure]) -> Option<String> {
    let claude = pressure.iter().find(|p| p.provider == Provider::Claude)?.used_pct?;
    let best = pressure.iter().filter(|p| p.provider != Provider::Claude && p.used_pct.is_some()).min_by(|a, b| a.used_pct.partial_cmp(&b.used_pct).unwrap())?;
    let other = best.used_pct?;
    (claude - other >= 30.0).then(|| {
        format!("Your Claude plan is at {claude:.0}% and {} at {other:.0}%. Consider running this job in {} instead; the job spec moves with it.", best.provider.label(), best.provider.label())
    })
}

/// What a helper run produced.
#[derive(Debug, Clone, Default, Serialize)]
pub struct HelperRun {
    pub ok: bool,
    pub summary: String,
    pub usage: Usage,
    pub usd: Option<f64>,
    pub secs: f64,
    pub error: Option<String>,
}

/// The brief a helper gets. Short, scoped, and asking for a short answer,
/// because the answer lands in the main agent's context.
pub fn brief(kind: Kind, goal: &str, task: &str, allow: &[String], deny: &[String]) -> String {
    let scope = if allow.is_empty() { "anywhere in the repo".to_string() } else { allow.join(", ") };
    let off = if deny.is_empty() { String::new() } else { format!(" Never edit: {}.", deny.join(", ")) };
    let rules = match kind {
        Kind::Lookup => "Read only; do not edit anything.".to_string(),
        Kind::Research => "Read only; do not edit anything. Use web search and fetch for docs when it helps.".to_string(),
        Kind::Edit => format!("Make exactly this change and nothing else. Edit only within: {scope}.{off} Do not refactor, rename, or reformat beyond it."),
        Kind::Hard => format!("Fix it with the smallest change that works. Edit only within: {scope}.{off}"),
    };
    format!(
        "You are a helper doing one task for another coding agent.\n\
         Overall goal: {goal}\n\
         Your task: {task}\n\
         Rules: {rules} Do not run shell commands.\n\
         Answer in at most 12 short lines: what you found or changed, with path:line references. No preamble, no process narration."
    )
}

/// Run a Claude Code helper headless.
pub fn run_claude(repo: &Path, route: &Route, kind: Kind, prompt: &str) -> HelperRun {
    let started = Instant::now();
    let tools = match kind {
        Kind::Lookup => "Read,Grep,Glob",
        Kind::Research => "Read,Grep,Glob,WebSearch,WebFetch",
        Kind::Edit | Kind::Hard => "Read,Grep,Glob,Edit,Write,MultiEdit",
    };
    let mut cmd = Command::new("claude");
    cmd.current_dir(repo)
        .arg("-p")
        .arg(prompt)
        .args(["--model", &route.model])
        .args(["--output-format", "json"])
        // No MCP servers in the helper: no recursion into NDSTRL itself.
        .args(["--strict-mcp-config", "--mcp-config", r#"{"mcpServers":{}}"#])
        .args(["--allowedTools", tools])
        .args(["--disallowedTools", "Bash,Task,NotebookEdit"])
        .args(["--max-budget-usd", &format!("{:.2}", route.cap_usd)]);
    // A helper launched from inside a session must not inherit its identity.
    for (k, _) in std::env::vars() {
        if k == "CLAUDECODE" || k.starts_with("CLAUDE_CODE_") || k == "CLAUDE_PID" || k == "CLAUDE_EFFORT" || k == "NDSTRL_TRANSCRIPT" {
            cmd.env_remove(&k);
        }
    }
    let out = match run_with_timeout(cmd, Duration::from_secs(600)) {
        Ok(o) => o,
        Err(e) => return HelperRun { error: Some(e), secs: started.elapsed().as_secs_f64(), ..Default::default() },
    };
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap_or(serde_json::Value::Null);
    if v.is_null() {
        return HelperRun {
            error: Some(format!("no JSON from claude: {}", String::from_utf8_lossy(&out).chars().take(300).collect::<String>())),
            secs: started.elapsed().as_secs_f64(),
            ..Default::default()
        };
    }
    let u = &v["usage"];
    let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let usage = Usage {
        input: g("input_tokens"),
        output: g("output_tokens"),
        cache_read: g("cache_read_input_tokens"),
        cache_write_5m: u.pointer("/cache_creation/ephemeral_5m_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        cache_write_1h: u.pointer("/cache_creation/ephemeral_1h_input_tokens").and_then(|x| x.as_u64()).unwrap_or(g("cache_creation_input_tokens")),
        usd: v["total_cost_usd"].as_f64().unwrap_or(0.0),
        responses: v["num_turns"].as_u64().unwrap_or(0),
        ..Default::default()
    };
    let is_error = v["is_error"].as_bool().unwrap_or(false);
    HelperRun {
        ok: !is_error,
        summary: v["result"].as_str().unwrap_or("").trim().to_string(),
        usd: v["total_cost_usd"].as_f64(),
        usage,
        secs: started.elapsed().as_secs_f64(),
        error: is_error.then(|| v["subtype"].as_str().unwrap_or("error").to_string()),
    }
}

/// Codex and Cursor adapters land once their CLIs are installed and signed
/// in on this machine; until then routing never selects them.
pub fn run_other(route: &Route) -> HelperRun {
    HelperRun { error: Some(format!("{} helper runs are not wired up yet", route.provider.label())), ..Default::default() }
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Vec<u8>, String> {
    use std::io::Read;
    use std::process::Stdio;
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("could not start helper: {e}"))?;
    let mut so = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("helper timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(reader.join().unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balances_across_plans() {
        let p = |provider, used_pct| Pressure { provider, used_pct };
        // Claude busy, Codex idle: routine work moves to Codex.
        let r = choose(Kind::Edit, false, &[p(Provider::Claude, Some(78.0)), p(Provider::Codex, Some(12.0))], None).unwrap();
        assert_eq!(r.provider, Provider::Codex);
        // Research stays on Claude however busy it is (web tools).
        let r = choose(Kind::Research, false, &[p(Provider::Claude, Some(78.0)), p(Provider::Codex, Some(12.0))], None).unwrap();
        assert_eq!(r.provider, Provider::Claude);
        // Unknown counts as half full: a known-idle Claude wins, a known-busy one loses.
        assert_eq!(choose(Kind::Lookup, false, &[p(Provider::Claude, Some(20.0)), p(Provider::Cursor, None)], None).unwrap().provider, Provider::Claude);
        assert_eq!(choose(Kind::Lookup, false, &[p(Provider::Claude, Some(70.0)), p(Provider::Cursor, None)], None).unwrap().provider, Provider::Cursor);
        // Ties go to Claude; models follow the kind; budget caps the helper.
        let r = choose(Kind::Lookup, false, &[p(Provider::Claude, None), p(Provider::Codex, None)], None).unwrap();
        assert_eq!((r.provider, r.model.as_str()), (Provider::Claude, "claude-haiku-5-5"));
        assert_eq!(choose(Kind::Hard, true, &[p(Provider::Claude, None)], None).unwrap().model, "claude-fable-5-1");
        assert!((choose(Kind::Edit, false, &[p(Provider::Claude, None)], Some(0.05)).unwrap().cap_usd - 0.05).abs() < 1e-9);
        assert!(choose(Kind::Edit, false, &[p(Provider::Claude, None)], Some(0.0)).is_err());
        assert!(main_session_hint(&[p(Provider::Claude, Some(72.0)), p(Provider::Codex, Some(20.0))]).unwrap().contains("Codex"));
        assert!(main_session_hint(&[p(Provider::Claude, Some(40.0)), p(Provider::Codex, Some(20.0))]).is_none());
    }
}
