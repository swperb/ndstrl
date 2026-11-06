//! `ndstrl`: the NDSTRL binary.
//!
//!   ndstrl                 serve MCP on stdio (what harnesses launch)
//!   ndstrl install [claude|codex|all] [--dry-run]
//!   ndstrl init "<goal>"   write a starter spec in the current repo
//!   ndstrl status          print the open job's gauges

mod bench;
mod mcp;
mod tools;

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

fn main() {
    if let Err(e) = real_main() {
        eprintln!("ndstrl: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => mcp::Server::run(),
        Some("install") => install(&args[1..]),
        Some("init") => init(&args[1..]),
        Some("status") => status(),
        Some("bench") => bench::run(&args[1..]),
        Some("statusline") => statusline(),
        Some("plans") => plans(),
        Some("report") => report(&args[1..]),
        Some("--version" | "-V" | "version") => {
            println!("ndstrl {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h" | "help") => {
            println!("{}", HELP);
            Ok(())
        }
        Some(other) => bail!("unknown command {other}\n\n{HELP}"),
    }
}

const HELP: &str = "ndstrl: one job contract for every coding agent you use

USAGE
  ndstrl                      serve MCP on stdio (harnesses launch this)
  ndstrl install [target]     register with claude (default), codex, or all
        --dry-run             show what would change
  ndstrl init \"<goal>\"        write .ndstrl/job.toml in this repo
  ndstrl status               gauges for the job in this repo
  ndstrl plans                your AI plans, how full each is, and where work goes
  ndstrl report [--days N]    what your jobs cost, by project, and what used your windows
  ndstrl statusline           Claude Code status line: logs plan usage, prints gauges
  ndstrl bench [job..]        run bench jobs plain vs through ndstrl on real claude -p sessions
        --mode plain|ndstrl|both   --model <id>";

fn exe() -> Result<PathBuf> {
    Ok(std::env::current_exe()?.canonicalize()?)
}

fn install(args: &[String]) -> Result<()> {
    let dry = args.iter().any(|a| a == "--dry-run");
    let target = args.iter().find(|a| !a.starts_with("--")).map(String::as_str).unwrap_or("claude");
    let exe = exe()?;
    let home = dirs::home_dir().context("no home directory")?;
    if matches!(target, "claude" | "all") {
        install_claude(&home, &exe, dry)?;
        install_statusline(&home, &exe, dry)?;
    }
    if matches!(target, "codex" | "all") {
        install_codex(&home, &exe, dry)?;
    }
    if !matches!(target, "claude" | "codex" | "all") {
        bail!("unknown target {target}; use claude, codex, or all");
    }
    if !dry {
        println!("Done. Start a new session and give it a goal and a budget.");
    }
    Ok(())
}

/// Claude Code (CLI and Claude Desktop's Code tab) reads user-scope MCP
/// servers from ~/.claude.json. Edit only our own key, keep everything else.
fn install_claude(home: &Path, exe: &Path, dry: bool) -> Result<()> {
    let cfg_dir = std::env::var("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|_| home.to_path_buf());
    let path = cfg_dir.join(".claude.json");
    let mut root: serde_json::Value = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path)?).with_context(|| format!("parsing {}", path.display()))?
    } else {
        serde_json::json!({})
    };
    let entry = serde_json::json!({ "type": "stdio", "command": exe.display().to_string(), "args": [], "env": {} });
    let servers = root
        .as_object_mut()
        .context("~/.claude.json is not an object")?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    if servers.get("ndstrl") == Some(&entry) {
        println!("claude: already installed in {}", path.display());
        return Ok(());
    }
    if dry {
        println!("claude: would set mcpServers.ndstrl in {} to {}", path.display(), entry);
        return Ok(());
    }
    servers["ndstrl"] = entry;
    let backup = path.with_extension("json.ndstrl-backup");
    if path.exists() {
        std::fs::copy(&path, &backup)?;
    }
    let tmp = path.with_extension("json.ndstrl-tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&root)?)?;
    std::fs::rename(&tmp, &path)?;
    println!("claude: added ndstrl to {} (backup at {})", path.display(), backup.display());
    Ok(())
}

fn install_codex(home: &Path, exe: &Path, dry: bool) -> Result<()> {
    let dir = std::env::var("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".codex"));
    let path = dir.join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc: toml::Table = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let servers = doc.entry("mcp_servers").or_insert_with(|| toml::Value::Table(Default::default()));
    let servers = servers.as_table_mut().context("mcp_servers is not a table")?;
    let mut entry = toml::Table::new();
    entry.insert("command".into(), toml::Value::String(exe.display().to_string()));
    if servers.get("ndstrl").and_then(|v| v.as_table()) == Some(&entry) {
        println!("codex: already installed in {}", path.display());
        return Ok(());
    }
    if dry {
        println!("codex: would add [mcp_servers.ndstrl] command = {:?} to {}", exe.display().to_string(), path.display());
        return Ok(());
    }
    servers.insert("ndstrl".into(), toml::Value::Table(entry));
    std::fs::create_dir_all(&dir)?;
    if path.exists() {
        std::fs::copy(&path, path.with_extension("toml.ndstrl-backup"))?;
    }
    std::fs::write(&path, toml::to_string_pretty(&doc)?)?;
    println!("codex: added ndstrl to {}", path.display());
    Ok(())
}

fn init(args: &[String]) -> Result<()> {
    let goal = args.join(" ");
    if goal.trim().is_empty() {
        bail!("usage: ndstrl init \"<goal>\"");
    }
    let repo = tools::default_repo(&std::env::current_dir()?);
    let mut t = ndstrl_core::JobSpec { goal, ..Default::default() };
    t.test = guess_test(&repo);
    t.save(&repo)?;
    println!("wrote {}\nEdit allow, deny, test, and budget_usd there, then start a session and give it the goal.", ndstrl_core::JobSpec::path(&repo).display());
    Ok(())
}

fn guess_test(repo: &Path) -> Option<String> {
    let has = |f: &str| repo.join(f).exists();
    if has("Cargo.toml") {
        Some("cargo test".into())
    } else if has("package.json") {
        Some("npm test".into())
    } else if has("pyproject.toml") || has("pytest.ini") {
        Some("pytest -q".into())
    } else if has("go.mod") {
        Some("go test ./...".into())
    } else {
        None
    }
}

fn status() -> Result<()> {
    let repo = tools::default_repo(&std::env::current_dir()?);
    let job = ndstrl_core::Job::open(&repo, None, "cli")?;
    println!("{}\n{}", job.spec.goal, job.gauges().line());
    Ok(())
}

/// Make `ndstrl statusline` the status line so plan usage is logged for
/// free. Never replaces a status line the user already has.
fn install_statusline(home: &Path, exe: &Path, dry: bool) -> Result<()> {
    let cfg_dir = std::env::var("CLAUDE_CONFIG_DIR").map(PathBuf::from).unwrap_or_else(|_| home.join(".claude"));
    let path = cfg_dir.join("settings.json");
    let mut root: serde_json::Value = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path)?).with_context(|| format!("parsing {}", path.display()))?
    } else {
        serde_json::json!({})
    };
    let cmd = format!("\"{}\" statusline", exe.display());
    match root.get("statusLine") {
        Some(sl) if sl.get("command").and_then(|c| c.as_str()) == Some(cmd.as_str()) => {
            println!("statusline: already installed");
            return Ok(());
        }
        Some(sl) if !sl.is_null() => {
            println!(
                "statusline: you already have one ({}). Left it alone. To log plan usage, pipe its input through `ndstrl statusline --chain '<your command>'`.",
                sl
            );
            return Ok(());
        }
        _ => {}
    }
    let entry = serde_json::json!({ "type": "command", "command": cmd, "padding": 0, "refreshInterval": 60 });
    if dry {
        println!("statusline: would set statusLine in {} to {}", path.display(), entry);
        return Ok(());
    }
    root.as_object_mut().context("settings.json is not an object")?.insert("statusLine".into(), entry);
    if path.exists() {
        std::fs::copy(&path, path.with_extension("json.ndstrl-backup"))?;
    }
    std::fs::create_dir_all(&cfg_dir)?;
    std::fs::write(&path, serde_json::to_string_pretty(&root)?)?;
    println!("statusline: set in {} (logs plan usage to ~/.ndstrl, no tokens spent)", path.display());
    Ok(())
}

/// Status line entry point. Must be fast and must never fail loudly.
fn statusline() -> Result<()> {
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let v: serde_json::Value = serde_json::from_str(&input).unwrap_or(serde_json::Value::Null);
    let args: Vec<String> = std::env::args().collect();
    let chain = args.iter().position(|a| a == "--chain").and_then(|i| args.get(i + 1)).cloned();

    let snap = ndstrl_core::plan::from_statusline(&v);
    if let Some(s) = &snap {
        let _ = ndstrl_core::plan::record(s);
    }
    let mut parts = Vec::new();
    if let Some(s) = snap.or_else(ndstrl_core::plan::latest) {
        let f = |w: Option<ndstrl_core::plan::Window>, n: &str| w.map(|w| format!("{n} {:.0}%", w.used_percentage));
        let p: Vec<String> = [f(s.five_hour, "5h"), f(s.seven_day, "7d")].into_iter().flatten().collect();
        if !p.is_empty() {
            parts.push(p.join(" "));
        }
    }
    let dir = v.pointer("/workspace/current_dir").and_then(|d| d.as_str()).map(PathBuf::from);
    if let Some(dir) = dir {
        let repo = tools::default_repo(&dir);
        if let Ok(text) = std::fs::read_to_string(repo.join(".ndstrl/state.json")) {
            if let Ok(st) = serde_json::from_str::<ndstrl_core::job::State>(&text) {
                let checks = match st.check_runs.last() {
                    None => "-",
                    Some(r) if r.passed => "pass",
                    Some(_) => "fail",
                };
                let open_q = st.questions.iter().any(|q| q.answer.is_none());
                parts.push(format!(
                    "job ${:.2} · {} files · checks {}{}{}",
                    st.usage.usd,
                    st.touched.len(),
                    checks,
                    if st.done { " · done" } else { "" },
                    if open_q { " · QUESTION" } else { "" }
                ));
            }
        }
    }
    let mut line = format!("ndstrl{}", if parts.is_empty() { String::new() } else { format!(" · {}", parts.join(" · ")) });
    if let Some(c) = chain {
        use std::io::Write;
        let shell = if cfg!(windows) { ("cmd", "/C") } else { ("sh", "-c") };
        if let Ok(mut child) = std::process::Command::new(shell.0)
            .args([shell.1, &c])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
        {
            if let Some(mut si) = child.stdin.take() {
                let _ = si.write_all(input.as_bytes());
            }
            if let Ok(o) = child.wait_with_output() {
                let theirs = String::from_utf8_lossy(&o.stdout).trim_end().to_string();
                if !theirs.is_empty() {
                    line = format!("{theirs}  {line}");
                }
            }
        }
    }
    println!("{line}");
    Ok(())
}

fn plans() -> Result<()> {
    use ndstrl_core::route::{choose, pressures, Kind};
    let ps = pressures();
    if ps.is_empty() {
        println!("No AI plans found. Install Claude Code or Codex.");
        return Ok(());
    }
    for p in &ps {
        println!(
            "{:<22} {:<10} {:>12}   size {:>4}   room {:.2}",
            p.provider.label(),
            p.tier.replace('_', " "),
            p.used_pct.map(|u| format!("{u:.0}% used")).unwrap_or_else(|| "no reading".into()),
            p.weight,
            p.room()
        );
    }
    println!();
    for (k, name) in [(Kind::Lookup, "look-ups"), (Kind::Research, "research"), (Kind::Edit, "edits"), (Kind::Hard, "hard problems")] {
        match choose(k, false, &ps, None) {
            Ok(r) => println!("{name:<14} -> {} {}", r.provider.label(), r.model),
            Err(e) => println!("{name:<14} -> none ({e})"),
        }
    }
    println!("\nSizes are rough defaults by tier. Override in ~/.ndstrl/plans.toml, e.g. codex = 1.0");
    Ok(())
}

fn report(args: &[String]) -> Result<()> {
    use std::collections::BTreeMap;
    let days: i64 = args.iter().position(|a| a == "--days").and_then(|i| args.get(i + 1)).and_then(|d| d.parse().ok()).unwrap_or(7);
    let since = chrono::Utc::now() - chrono::Duration::days(days);
    let rows: Vec<_> = ndstrl_core::auto::history()
        .into_iter()
        .filter(|r| r.at.is_empty() || chrono::DateTime::parse_from_rfc3339(&r.at).map(|t| t >= since).unwrap_or(true))
        .collect();

    // Where the plan stands now.
    if let Some(s) = ndstrl_core::plan::latest() {
        let w = |x: Option<ndstrl_core::plan::Window>| x.map(|w| format!("{:.0}%", w.used_percentage)).unwrap_or_else(|| "-".into());
        println!("Claude plan now: 5-hour {}, week {}", w(s.five_hour), w(s.seven_day));
    } else {
        println!("Claude plan now: no status-line reading yet (open a Claude Code session once).");
    }
    if let Some((used, tier)) = ndstrl_core::route::codex_limits() {
        println!("Codex plan now:  {used:.0}% ({tier})");
    }
    println!();
    if rows.is_empty() {
        println!("No finished jobs in the last {days} days. Jobs are logged when they pass their checks or stop.");
        return Ok(());
    }

    let total: f64 = rows.iter().map(|r| r.usd).sum();
    let helpers: f64 = rows.iter().map(|r| r.helper_usd).sum();
    let p5: f64 = rows.iter().filter_map(|r| r.plan_5h_pts).sum();
    let p7: f64 = rows.iter().filter_map(|r| r.plan_7d_pts).sum();
    let measured = rows.iter().filter(|r| r.plan_7d_pts.is_some()).count();
    println!(
        "Last {days} days: {} jobs ({} done), ${total:.2} at API prices, ${:.2} per job, {:.0}% of it on helpers",
        rows.len(),
        rows.iter().filter(|r| r.done).count(),
        total / rows.len() as f64,
        if total > 0.0 { helpers / total * 100.0 } else { 0.0 }
    );
    if measured > 0 {
        println!("Plan windows used by these jobs: {p5:.1} points of 5-hour windows, {p7:.1} points of the week ({measured} jobs with readings)");
    }

    // By project: which workflows cost what.
    let mut by: BTreeMap<String, (usize, f64, f64, u64)> = BTreeMap::new();
    for r in &rows {
        let e = by.entry(if r.repo.is_empty() { "(unknown)".into() } else { r.repo.clone() }).or_default();
        e.0 += 1;
        e.1 += r.usd;
        e.2 += r.plan_7d_pts.unwrap_or(0.0);
        e.3 += r.minutes;
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by(|a, b| b.1 .1.partial_cmp(&a.1 .1).unwrap());
    println!("\nBy project:");
    for (repo, (n, usd, wk, mins)) in &by {
        let wk = if *wk > 0.0 { format!(", {wk:.1} pts of the week") } else { String::new() };
        println!("  {repo:<24} {n:>3} job{}  ${usd:>7.2}  {:>4} min{wk}", if *n == 1 { " " } else { "s" }, mins);
    }

    // The jobs that cost the most, so the expensive kinds of work stand out.
    let mut top: Vec<_> = rows.iter().collect();
    top.sort_by(|a, b| b.usd.partial_cmp(&a.usd).unwrap());
    println!("\nCostliest jobs:");
    for r in top.iter().take(5) {
        let goal: String = r.goal.chars().take(60).collect();
        println!(
            "  ${:>6.2}  {:>3} files {:>4} lines {:>4} min  {}{}",
            r.usd,
            r.files,
            r.lines,
            r.minutes,
            goal,
            if r.done { "" } else { "  (stopped)" }
        );
    }
    Ok(())
}
