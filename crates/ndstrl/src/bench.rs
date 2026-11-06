//! `ndstrl bench`: run the same job on a plain agent and through NDSTRL,
//! with real headless Claude Code sessions, and score both.
//!
//! This is how NDSTRL measures itself. Every change to NDSTRL can be
//! benched against fixed parts on real usage, so "better" is a number:
//! accepted, tokens, list-price equivalent, turns, files outside the
//! scope, questions asked, and how close the NDSTRL meter came to what
//! the harness reported.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Deserialize)]
struct Part {
    prompt: String,
    test: String,
    #[serde(default)]
    allow: Vec<String>,
    #[serde(default)]
    deny: Vec<String>,
    budget_usd: f64,
    /// Hidden check run after the agent finishes: does the change keep
    /// the meaning of the codebase, not just turn tests green?
    #[serde(default)]
    faithful: Option<String>,
    /// "ask" when the right outcome is to stop and ask, not to finish.
    #[serde(default)]
    expect: Option<String>,
}

#[derive(Debug, Serialize)]
struct Score {
    part: String,
    mode: String,
    accepted: bool,
    /// Green and faithful, or correctly stopped when the job says to ask.
    good: bool,
    tokens: u64,
    list_usd: f64,
    ndstrl_meter_usd: Option<f64>,
    turns: u64,
    secs: f64,
    files: Vec<String>,
    outside_scope: Vec<String>,
    questions: u64,
    error: Option<String>,
    summary: String,
}

pub fn run(args: &[String]) -> Result<()> {
    let root = find_bench_root()?;
    let mut modes = vec!["plain", "ndstrl"];
    let mut model: Option<String> = None;
    let mut trials: usize = 1;
    let mut names = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--mode" => {
                let m = it.next().context("--mode needs plain|ndstrl|both")?;
                modes = match m.as_str() {
                    "plain" => vec!["plain"],
                    "ndstrl" => vec!["ndstrl"],
                    "both" => vec!["plain", "ndstrl"],
                    other => bail!("unknown mode {other}"),
                };
            }
            "--model" => model = Some(it.next().context("--model needs a value")?.clone()),
            "--trials" => trials = it.next().context("--trials needs a number")?.parse().context("--trials needs a number")?,
            other => names.push(other.to_string()),
        }
    }
    let parts_dir = root.join("parts");
    if names.is_empty() {
        for e in std::fs::read_dir(&parts_dir)? {
            let e = e?;
            if e.path().join("part.toml").exists() {
                names.push(e.file_name().to_string_lossy().into_owned());
            }
        }
        names.sort();
    }
    let exe = std::env::current_exe()?.canonicalize()?;
    let mut scores = Vec::new();
    for name in &names {
        let dir = parts_dir.join(name);
        let part: Part = toml::from_str(&std::fs::read_to_string(dir.join("part.toml"))?)
            .with_context(|| format!("parsing {name}/part.toml"))?;
        for trial in 1..=trials {
        for mode in &modes {
            eprintln!("bench: {name} on {mode}, trial {trial} …");
            let s = run_one(name, &dir, &part, mode, &exe, model.as_deref())?;
            eprintln!(
                "bench: {name} {mode}: accepted={} ${:.3} {} tok, {} turns, outside={:?}, questions={}",
                s.accepted, s.list_usd, s.tokens, s.turns, s.outside_scope, s.questions
            );
            scores.push(s);
        }
        }
    }
    let out = write_report(&root, &scores, model.as_deref())?;
    println!("{}", std::fs::read_to_string(&out)?);
    println!("report: {}", out.display());
    Ok(())
}

fn find_bench_root() -> Result<PathBuf> {
    let mut p = std::env::current_dir()?;
    loop {
        if p.join("bench/parts").is_dir() {
            return Ok(p.join("bench"));
        }
        if !p.pop() {
            bail!("no bench/parts directory above the current directory");
        }
    }
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            if e.file_name() == "target" {
                continue;
            }
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), &to)?;
        }
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let o = Command::new("git")
        .args(["-c", "user.email=bench@ndstrl", "-c", "user.name=bench"])
        .args(args)
        .current_dir(dir)
        .output()?;
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// A harness launched from inside another harness must not inherit the
/// parent's session identity, or NDSTRL would meter the wrong transcript.
fn scrub(cmd: &mut Command) {
    for (k, _) in std::env::vars() {
        if k == "CLAUDECODE" || k.starts_with("CLAUDE_CODE_") || k == "CLAUDE_PID" || k == "CLAUDE_EFFORT" {
            cmd.env_remove(&k);
        }
    }
}

fn run_one(name: &str, src: &Path, part: &Part, mode: &str, exe: &Path, model: Option<&str>) -> Result<Score> {
    let work = std::env::temp_dir().join(format!("ndstrl-bench-{}-{}-{}", name, mode, std::process::id()));
    let _ = std::fs::remove_dir_all(&work);
    copy_dir(&src.join("repo"), &work)?;
    git(&work, &["init", "-q"])?;
    git(&work, &["add", "-A"])?;
    git(&work, &["commit", "-qm", "part"])?;

    let mcp = if mode == "ndstrl" {
        serde_json::json!({ "mcpServers": { "ndstrl": { "type": "stdio", "command": exe.display().to_string(), "args": [] } } })
    } else {
        serde_json::json!({ "mcpServers": {} })
    };
    let mcp_path = work.join(".bench-mcp.json");
    std::fs::write(&mcp_path, mcp.to_string())?;

    let mut cmd = Command::new("claude");
    cmd.current_dir(&work)
        .arg("-p")
        .arg(&part.prompt)
        .args(["--output-format", "json", "--strict-mcp-config", "--mcp-config"])
        .arg(&mcp_path)
        // Hard stop for the bench itself, independent of NDSTRL.
        .args(["--max-budget-usd", &format!("{}", part.budget_usd * 2.0)]);
    if mode == "ndstrl" {
        cmd.args(["--allowedTools", "mcp__ndstrl__*"])
            .args(["--disallowedTools", "Edit,Write,MultiEdit,NotebookEdit,Bash,Read,Grep,Glob"]);
    } else {
        cmd.args(["--allowedTools", "Read,Edit,Write,MultiEdit,Grep,Glob,Bash"]);
    }
    if let Some(m) = model {
        cmd.args(["--model", m]);
    }
    scrub(&mut cmd);
    let out = cmd.output().context("running claude; is Claude Code installed?")?;
    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or(Value::Null);

    // Inspect independently of what the agent claims.
    let accepted = Command::new("sh")
        .args(["-c", &part.test])
        .current_dir(&work)
        .env("NO_COLOR", "1")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let faithful = part.faithful.as_ref().map(|c| {
        Command::new("sh").args(["-c", c]).current_dir(&work).output().map(|o| o.status.success()).unwrap_or(false)
    });
    let changed = git(&work, &["status", "--porcelain", "--untracked-files=all"])?;
    let files: Vec<String> = changed
        .lines()
        .filter_map(|l| l.get(3..).map(|s| s.trim().to_string()))
        .filter(|f| !f.starts_with(".ndstrl") && f != ".bench-mcp.json" && !f.starts_with("target"))
        .collect();
    let scope = ndstrl_core::scope::Scope::new(&work, &part.allow, &part.deny)?;
    let outside_scope: Vec<String> =
        files.iter().filter(|f| scope.can_edit(f) != ndstrl_core::scope::Verdict::Allowed).cloned().collect();

    let state: Option<Value> = std::fs::read_to_string(work.join(".ndstrl/state.json")).ok().and_then(|s| serde_json::from_str(&s).ok());
    let questions = state.as_ref().and_then(|s| s["questions"].as_array().map(|a| a.len() as u64)).unwrap_or(0);
    let ndstrl_meter_usd = state.as_ref().and_then(|s| s["usage"]["usd"].as_f64());

    let u = &v["usage"];
    let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let tokens = g("input_tokens") + g("output_tokens") + g("cache_creation_input_tokens") + g("cache_read_input_tokens");
    let result = v["result"].as_str().unwrap_or("").to_string();
    let error = if v.is_null() {
        Some(format!("no JSON from claude: {}", String::from_utf8_lossy(&out.stderr).chars().take(300).collect::<String>()))
    } else if v["is_error"].as_bool() == Some(true) {
        Some(v["subtype"].as_str().unwrap_or("error").to_string())
    } else {
        None
    };
    let _ = std::fs::write(work.join(".bench-result.json"), serde_json::to_string_pretty(&v).unwrap_or_default());
    let good = match part.expect.as_deref() {
        // Stopping without going out of scope, or finishing faithfully, both count.
        Some("ask") => outside_scope.is_empty() && (faithful.unwrap_or(true) && (accepted || files.is_empty())),
        _ => accepted && faithful.unwrap_or(true) && outside_scope.is_empty(),
    };
    Ok(Score {
        part: name.into(),
        mode: mode.into(),
        accepted,
        good,
        tokens,
        list_usd: v["total_cost_usd"].as_f64().unwrap_or(0.0),
        ndstrl_meter_usd,
        turns: v["num_turns"].as_u64().unwrap_or(0),
        secs: v["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0,
        files,
        outside_scope,
        questions,
        error,
        summary: result.chars().take(400).collect(),
    })
}

fn write_report(root: &Path, scores: &[Score], model: Option<&str>) -> Result<PathBuf> {
    let dir = root.join("results");
    std::fs::create_dir_all(&dir)?;
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let mut md = String::new();
    let _ = writeln!(md, "# Bench {stamp}\n");
    let _ = writeln!(md, "Model: {}. Dollars are API list-price equivalents of real tokens, not a bill.\n", model.unwrap_or("harness default"));
    let _ = writeln!(md, "| job | agent | good | green | tokens | list $ | ndstrl meter $ | turns | secs | files | outside scope | questions |");
    let _ = writeln!(md, "| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- | --- | ---: |");
    for s in scores {
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {:.3} | {} | {} | {:.1} | {} | {} | {} |",
            s.part,
            s.mode,
            if s.good { "yes" } else { "NO" },
            if s.accepted { "yes" } else { "no" },
            s.tokens,
            s.list_usd,
            s.ndstrl_meter_usd.map(|x| format!("{x:.3}")).unwrap_or_else(|| "-".into()),
            s.turns,
            s.secs,
            if s.files.is_empty() { "-".into() } else { s.files.join(", ") },
            if s.outside_scope.is_empty() { "-".into() } else { s.outside_scope.join(", ") },
            s.questions
        );
    }
    let _ = writeln!(md, "\n## What each agent said\n");
    for s in scores {
        let _ = writeln!(md, "**{} / {}**{}\n\n> {}\n", s.part, s.mode, s.error.as_ref().map(|e| format!(" (error: {e})")).unwrap_or_default(), s.summary.replace('\n', "\n> "));
    }
    let path = dir.join(format!("{stamp}.md"));
    std::fs::write(&path, md)?;
    let mut jl = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("history.jsonl"))?;
    use std::io::Write;
    for s in scores {
        writeln!(jl, "{}", serde_json::json!({ "at": stamp.to_string(), "model": model, "score": s }))?;
    }
    Ok(path)
}
