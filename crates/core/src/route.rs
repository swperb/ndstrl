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
        self.path().is_some()
    }

    /// Where the CLI lives. Codex also ships inside the ChatGPT app.
    pub fn path(self) -> Option<std::path::PathBuf> {
        if self == Provider::Codex {
            if let Ok(p) = std::env::var("NDSTRL_CODEX") {
                return Some(p.into());
            }
        }
        if let Some(p) = which_path(self.bin()) {
            return Some(p);
        }
        let bundled: &[&str] = match self {
            Provider::Codex => &["/Applications/ChatGPT.app/Contents/Resources/codex-cli/bin/codex"],
            _ => &[],
        };
        bundled.iter().map(std::path::PathBuf::from).find(|p| p.is_file())
    }
}

fn which_path(bin: &str) -> Option<std::path::PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).flat_map(|d| [d.join(bin), d.join(format!("{bin}.exe"))]).find(|p| p.is_file())
}

/// The Codex plan's latest rate-limit reading, from its own session logs:
/// (fuller window's used percent, plan type). No request is made.
pub fn codex_limits() -> Option<(f64, String)> {
    let base = std::env::var("CODEX_HOME").map(std::path::PathBuf::from).ok().or_else(|| dirs::home_dir().map(|h| h.join(".codex")))?.join("sessions");
    // sessions/YYYY/MM/DD/rollout-*.jsonl; walk newest-first.
    let newest_dir = |p: &std::path::Path| -> Option<std::path::PathBuf> {
        let mut v: Vec<_> = std::fs::read_dir(p).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        v.sort();
        v.pop()
    };
    let day = newest_dir(&newest_dir(&newest_dir(&base)?)?)?;
    let mut files: Vec<_> = std::fs::read_dir(&day).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "jsonl").unwrap_or(false)).collect();
    files.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let now = chrono::Utc::now().timestamp();
    for f in files.iter().rev() {
        let text = std::fs::read_to_string(f).ok()?;
        for line in text.lines().rev().filter(|l| l.contains("\"rate_limits\"")) {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
            let Some(rl) = find_key(&v, "rate_limits") else { continue };
            let win = |k: &str| -> Option<f64> {
                let w = rl.get(k)?;
                let resets = w.get("resets_at").and_then(|x| x.as_i64()).unwrap_or(0);
                (resets == 0 || resets > now).then(|| w.get("used_percent").and_then(|x| x.as_f64())).flatten()
            };
            let used = win("primary").unwrap_or(0.0).max(win("secondary").unwrap_or(0.0));
            let plan = rl.get("plan_type").and_then(|x| x.as_str()).unwrap_or("unknown").to_string();
            return Some((used, plan));
        }
    }
    None
}

fn find_key<'a>(v: &'a serde_json::Value, k: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Object(m) => m.get(k).or_else(|| m.values().find_map(|x| find_key(x, k))),
        serde_json::Value::Array(a) => a.iter().find_map(|x| find_key(x, k)),
        _ => None,
    }
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

/// How full one subscription is, and how big it is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pressure {
    pub provider: Provider,
    /// The fuller of its windows, 0 to 100. `None` without a reading.
    pub used_pct: Option<f64>,
    /// The plan tier as the provider names it ("max_5x", "free", ...).
    pub tier: String,
    /// Rough usable work per window relative to a Claude Pro or ChatGPT
    /// Plus plan (1.0). A free plan with 0% used still has little room.
    pub weight: f64,
}

/// Unknown usage counts as half full: not preferred over a provider we
/// know is nearly idle, not avoided like one we know is nearly spent.
const UNKNOWN_PCT: f64 = 50.0;

impl Pressure {
    /// Room left, in plan-size units: what is unused times how big the plan is.
    pub fn room(&self) -> f64 {
        (100.0 - self.used_pct.unwrap_or(UNKNOWN_PCT)).max(0.0) / 100.0 * self.weight
    }
    pub fn is_free(&self) -> bool {
        self.tier == "free"
    }
    fn describe(&self) -> String {
        let used = self.used_pct.map(|p| format!("{p:.0}% used")).unwrap_or_else(|| "usage unknown".into());
        format!("{} {} ({used})", self.provider.label(), self.tier.replace('_', " "))
    }
}

/// Default capacity weights by tier. Rough by design; override them in
/// `~/.ndstrl/plans.toml`, e.g. `claude = 5.0` or `codex = 1.0`.
fn tier_weight(p: Provider, tier: &str) -> f64 {
    let t = tier.to_ascii_lowercase();
    match p {
        Provider::Claude if t.contains("20x") => 20.0,
        Provider::Claude if t.contains("5x") => 5.0,
        Provider::Claude if t.contains("max") => 5.0,
        Provider::Claude if t.contains("team") || t.contains("enterprise") => 1.5,
        Provider::Codex if t == "free" => 0.2,
        Provider::Codex if t == "plus" || t == "go" => 1.0,
        Provider::Codex if t == "pro" => 6.0,
        Provider::Codex if t.contains("team") || t.contains("business") || t.contains("enterprise") || t.contains("edu") => 1.5,
        _ => 1.0,
    }
}

fn overrides() -> std::collections::HashMap<String, f64> {
    crate::plan::home()
        .and_then(|h| std::fs::read_to_string(h.join("plans.toml")).ok())
        .and_then(|t| t.parse::<toml::Table>().ok())
        .map(|t| t.into_iter().filter_map(|(k, v)| v.as_float().or(v.as_integer().map(|i| i as f64)).map(|f| (k, f))).collect())
        .unwrap_or_default()
}

/// The Claude plan tier, from Claude Code's own config. Not a credential:
/// the same file names the account's rate-limit tier in plain text.
fn claude_tier() -> String {
    let path = std::env::var("CLAUDE_CONFIG_DIR")
        .map(|d| std::path::PathBuf::from(d).join(".claude.json"))
        .ok()
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude.json")));
    path.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| {
            let a = &v["oauthAccount"];
            a["userRateLimitTier"].as_str().or(a["organizationRateLimitTier"].as_str()).map(|s| s.trim_start_matches("default_claude_").to_string())
        })
        .unwrap_or_else(|| "unknown".into())
}

/// Current pressure and size of every installed provider's plan.
pub fn pressures() -> Vec<Pressure> {
    let claude_used = crate::plan::latest().and_then(|s| {
        let now = chrono::Utc::now().timestamp();
        let live = |w: Option<crate::plan::Window>| w.filter(|w| w.resets_at == 0 || w.resets_at > now).map(|w| w.used_percentage);
        match (live(s.five_hour), live(s.seven_day)) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0.0).max(b.unwrap_or(0.0))),
        }
    });
    let codex = codex_limits();
    let ov = overrides();
    let mk = |provider: Provider, used_pct: Option<f64>, tier: String| {
        let key = format!("{provider:?}").to_ascii_lowercase();
        let weight = ov.get(&key).copied().unwrap_or_else(|| tier_weight(provider, &tier));
        Pressure { provider, used_pct, tier, weight }
    };
    [
        mk(Provider::Claude, claude_used, claude_tier()),
        mk(Provider::Codex, codex.as_ref().map(|c| c.0), codex.map(|c| c.1).unwrap_or_else(|| "unknown".into())),
        mk(Provider::Cursor, None, "unknown".into()),
    ]
    .into_iter()
    .filter(|p| p.provider.installed())
    .collect()
}

fn can_do(p: &Pressure, kind: Kind) -> bool {
    let tool_ok = match p.provider {
        Provider::Claude => true,
        // Web research is wired up on Claude only for now.
        Provider::Codex | Provider::Cursor => kind != Kind::Research,
    };
    // Quality floor: free plans run weaker default models; give them
    // read-only look-ups, never changes to the person's code.
    tool_ok && !(p.is_free() && kind != Kind::Lookup)
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

/// Pick the route: among installed plans that can do this kind of work,
/// the one with the most room left, sized by plan. So a big plan at 60%
/// still beats a free plan at 0%, and no paid plan runs out while another
/// sits idle. Ties go to Claude, whose cost NDSTRL can meter.
pub fn choose(kind: Kind, allow_frontier: bool, pressure: &[Pressure], remaining_usd: Option<f64>) -> Result<Route, String> {
    let mut options: Vec<&Pressure> = pressure.iter().filter(|p| can_do(p, kind)).collect();
    if options.is_empty() {
        return Err("no installed plan can take this kind of work (free plans only do look-ups)".into());
    }
    options.sort_by(|a, b| {
        b.room().partial_cmp(&a.room()).unwrap().then_with(|| (a.provider != Provider::Claude).cmp(&(b.provider != Provider::Claude)))
    });
    let best = options[0];
    let (model, cap, why) = model_for(best.provider, kind, allow_frontier);
    let cap = remaining_usd.map(|r| cap.min(r.max(0.0))).unwrap_or(cap);
    if cap <= 0.0 {
        return Err("no budget left for a helper".into());
    }
    let skipped: Vec<String> = pressure
        .iter()
        .filter(|p| p.provider != best.provider)
        .map(|p| if can_do(p, kind) { p.describe() } else { format!("{} (not for this kind of work)", p.describe()) })
        .collect();
    let why = if skipped.is_empty() {
        why.to_string()
    } else {
        format!("{why}; most room left on {} vs {}", best.describe(), skipped.join(", "))
    };
    Ok(Route { provider: best.provider, model: model.into(), cap_usd: cap, why })
}

/// Which harness the main session should run in, when another plan has
/// clearly more room than Claude. NDSTRL cannot move the main session; it
/// can say so.
pub fn main_session_hint(pressure: &[Pressure]) -> Option<String> {
    let claude = pressure.iter().find(|p| p.provider == Provider::Claude)?;
    claude.used_pct?;
    let best = pressure
        .iter()
        .filter(|p| p.provider != Provider::Claude && p.used_pct.is_some() && !p.is_free())
        .max_by(|a, b| a.room().partial_cmp(&b.room()).unwrap())?;
    (best.room() >= claude.room() * 2.0 && best.room() >= 0.5).then(|| {
        format!(
            "{} has much more room than {}. Consider running this job in {} instead; the job spec moves with it.",
            best.describe(),
            claude.describe(),
            best.provider.label()
        )
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
pub fn brief(provider: Provider, kind: Kind, goal: &str, task: &str, allow: &[String], deny: &[String]) -> String {
    let scope = if allow.is_empty() { "anywhere in the repo".to_string() } else { allow.join(", ") };
    let off = if deny.is_empty() { String::new() } else { format!(" Never edit: {}.", deny.join(", ")) };
    let rules = match kind {
        Kind::Lookup => "Read only; do not edit anything.".to_string(),
        Kind::Research => "Read only; do not edit anything. Use web search and fetch for docs when it helps.".to_string(),
        Kind::Edit => format!("Make exactly this change and nothing else. Edit only within: {scope}.{off} Do not refactor, rename, or reformat beyond it."),
        Kind::Hard => format!("Fix it with the smallest change that works. Edit only within: {scope}.{off}"),
    };
    let shell = match provider {
        // Codex reads code through its shell; allow read-only commands only.
        Provider::Codex | Provider::Cursor => "You may run read-only shell commands (cat, sed -n, rg, ls) to read code; run nothing that changes files, installs, or uses the network.",
        Provider::Claude => "Do not run shell commands.",
    };
    format!(
        "You are a helper doing one task for another coding agent.\n\
         Overall goal: {goal}\n\
         Your task: {task}\n\
         Rules: {rules} {shell}\n\
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

/// Run a Codex helper headless on the person's ChatGPT plan. Usage comes
/// from `turn.completed`; there is no dollar figure for plan runs.
pub fn run_codex(repo: &Path, route: &Route, prompt: &str) -> HelperRun {
    let started = Instant::now();
    let Some(bin) = Provider::Codex.path() else {
        return HelperRun { error: Some("codex is not installed".into()), ..Default::default() };
    };
    let mut cmd = Command::new(bin);
    // read-only disables Codex's shell in this build, so it could not read
    // code at all. workspace-write is safe here: NDSTRL checks every change
    // with git and puts back anything the task did not allow.
    cmd.args(["exec", "--json", "--sandbox", "workspace-write", "--color", "never", "-c", "notify=[]", "-C"])
        .arg(repo)
        .arg(prompt);
    if route.model != "default" {
        cmd.args(["-m", &route.model]);
    }
    let out = match run_with_timeout(cmd, Duration::from_secs(600)) {
        Ok(o) => o,
        Err(e) => return HelperRun { error: Some(e), secs: started.elapsed().as_secs_f64(), ..Default::default() },
    };
    let mut summary = String::new();
    let mut usage = Usage::default();
    let mut error = None;
    for line in String::from_utf8_lossy(&out).lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        match v["type"].as_str() {
            Some("item.completed") if v["item"]["type"] == "agent_message" => {
                summary = v["item"]["text"].as_str().unwrap_or("").trim().to_string();
            }
            Some("turn.completed") => {
                let u = &v["usage"];
                let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
                let cached = g("cached_input_tokens");
                usage.input += g("input_tokens").saturating_sub(cached);
                usage.cache_read += cached;
                usage.cache_write_5m += g("cache_write_input_tokens");
                usage.output += g("output_tokens") + g("reasoning_output_tokens");
                usage.responses += 1;
            }
            Some("turn.failed") | Some("error") => {
                error = Some(v.get("error").or(v.get("message")).map(|e| e.to_string()).unwrap_or_else(|| "codex failed".into()));
            }
            _ => {}
        }
    }
    HelperRun { ok: error.is_none() && !summary.is_empty(), summary, usage, usd: None, secs: started.elapsed().as_secs_f64(), error }
}

/// Cursor's agent CLI is not wired up yet; routing never selects it until
/// `cursor-agent` is installed.
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
    fn weighs_plans_by_size() {
        let p = |provider, used: Option<f64>, tier: &str| Pressure { provider, used_pct: used, tier: tier.into(), weight: tier_weight(provider, tier) };
        let max5 = |u| p(Provider::Claude, u, "claude_max_5x");
        // A free plan at 0% does not outweigh Max 5x at 60%.
        let r = choose(Kind::Lookup, false, &[max5(Some(60.0)), p(Provider::Codex, Some(0.0), "free")], None).unwrap();
        assert_eq!(r.provider, Provider::Claude);
        // But a nearly spent Max does hand look-ups to the free plan.
        let r = choose(Kind::Lookup, false, &[max5(Some(97.0)), p(Provider::Codex, Some(0.0), "free")], None).unwrap();
        assert_eq!(r.provider, Provider::Codex);
        // Free plans never get edits or hard problems.
        let r = choose(Kind::Edit, false, &[max5(Some(97.0)), p(Provider::Codex, Some(0.0), "free")], None).unwrap();
        assert_eq!(r.provider, Provider::Claude);
        // Two paid plans balance by room: Max 5x at 90% (0.5) vs Plus at 20% (0.8).
        let r = choose(Kind::Edit, false, &[max5(Some(90.0)), p(Provider::Codex, Some(20.0), "plus")], None).unwrap();
        assert_eq!(r.provider, Provider::Codex);
        // Research stays on Claude; ties go to Claude; models follow the kind.
        assert_eq!(choose(Kind::Research, false, &[max5(Some(99.0)), p(Provider::Codex, Some(0.0), "pro")], None).unwrap().provider, Provider::Claude);
        let r = choose(Kind::Lookup, false, &[p(Provider::Claude, None, "x"), p(Provider::Codex, None, "y")], None).unwrap();
        assert_eq!((r.provider, r.model.as_str()), (Provider::Claude, "claude-haiku-5-5"));
        assert_eq!(choose(Kind::Hard, true, &[max5(None)], None).unwrap().model, "claude-fable-5-1");
        assert!((choose(Kind::Edit, false, &[max5(None)], Some(0.05)).unwrap().cap_usd - 0.05).abs() < 1e-9);
        assert!(choose(Kind::Edit, false, &[max5(None)], Some(0.0)).is_err());
        // The main-session hint ignores free plans and needs a clear gap.
        assert!(main_session_hint(&[max5(Some(95.0)), p(Provider::Codex, Some(0.0), "free")]).is_none());
        assert!(main_session_hint(&[max5(Some(95.0)), p(Provider::Codex, Some(10.0), "pro")]).unwrap().contains("Codex"));
        assert!(main_session_hint(&[max5(Some(40.0)), p(Provider::Codex, Some(10.0), "plus")]).is_none());
    }
}
