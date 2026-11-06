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
