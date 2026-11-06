//! Reads served by NDSTRL: the agent sees tight excerpts, never the tree.

use crate::scope::{Scope, Verdict};
use anyhow::{bail, Result};
use ignore::WalkBuilder;
use regex::RegexBuilder;
use std::fmt::Write;

pub const MAX_LINE: usize = 240;
pub const MAX_READ_LINES: usize = 250;
pub const MAX_HITS: usize = 60;
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

fn clip(s: &str) -> String {
    let s = s.trim_end();
    if s.chars().count() <= MAX_LINE {
        s.to_string()
    } else {
        let cut: String = s.chars().take(MAX_LINE).collect();
        format!("{cut}…")
    }
}

fn walker(scope: &Scope) -> ignore::Walk {
    WalkBuilder::new(scope.root()).hidden(false).git_ignore(true).git_exclude(true).parents(true).build()
}

fn rel_of(scope: &Scope, p: &std::path::Path) -> Option<String> {
    p.strip_prefix(scope.root()).ok().map(|r| r.to_string_lossy().replace('\\', "/"))
}

fn glob_filter(glob: Option<&str>) -> Result<Option<globset::GlobMatcher>> {
    Ok(match glob {
        Some(g) if !g.is_empty() => Some(globset::Glob::new(g)?.compile_matcher()),
        _ => None,
    })
}

/// List files (respecting .gitignore), optionally filtered by glob.
pub fn list(scope: &Scope, glob: Option<&str>, limit: usize) -> Result<String> {
    let filter = glob_filter(glob)?;
    let mut files = Vec::new();
    let mut total = 0usize;
    for e in walker(scope).flatten() {
        if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(rel) = rel_of(scope, e.path()) else { continue };
        if scope.can_read(&rel) != Verdict::Allowed {
            continue;
        }
        if let Some(f) = &filter {
            if !f.is_match(&rel) {
                continue;
            }
        }
        total += 1;
        if files.len() < limit {
            files.push(rel);
        }
    }
    files.sort();
    let mut out = files.join("\n");
    if total > files.len() {
        let _ = write!(out, "\n… {} more; narrow the glob", total - files.len());
    }
    if total == 0 {
        out = "no files match".into();
    }
    Ok(out)
}

/// Regex search with a line of context, capped hits.
pub fn search(scope: &Scope, pattern: &str, glob: Option<&str>, ignore_case: bool) -> Result<String> {
    let re = RegexBuilder::new(pattern).case_insensitive(ignore_case).size_limit(1 << 20).build()?;
    let filter = glob_filter(glob)?;
    let mut out = String::new();
    let mut hits = 0usize;
    let mut files = 0usize;
    let mut truncated = false;
    for e in walker(scope).flatten() {
        if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(rel) = rel_of(scope, e.path()) else { continue };
        if scope.can_read(&rel) != Verdict::Allowed {
            continue;
        }
        if let Some(f) = &filter {
            if !f.is_match(&rel) {
                continue;
            }
        }
        if e.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue }; // binary
        let mut file_hit = false;
        for (i, line) in text.lines().enumerate() {
            if re.is_match(line) {
                if hits >= MAX_HITS {
                    truncated = true;
                    break;
                }
                if !file_hit {
                    files += 1;
                    file_hit = true;
                }
                hits += 1;
                let _ = writeln!(out, "{rel}:{}: {}", i + 1, clip(line));
            }
        }
        if truncated {
            break;
        }
    }
    if hits == 0 {
        return Ok(format!("no matches for /{pattern}/"));
    }
    let _ = write!(out, "{hits} hit(s) in {files} file(s)");
    if truncated {
        out.push_str(&format!(", stopped at {MAX_HITS}; narrow the pattern or glob"));
    }
    Ok(out)
}

/// Numbered excerpt of a file.
pub fn read(scope: &Scope, rel: &str, start: Option<usize>, end: Option<usize>) -> Result<String> {
    if let Verdict::Forbidden(why) = scope.can_read(rel) {
        bail!("{why}");
    }
    let p = scope.abs(rel);
    let meta = std::fs::metadata(&p).map_err(|_| anyhow::anyhow!("{rel} does not exist"))?;
    if meta.len() > MAX_FILE_BYTES {
        bail!("{rel} is {} bytes; search it instead of reading it", meta.len());
    }
    let text = std::fs::read_to_string(&p).map_err(|_| anyhow::anyhow!("{rel} is not UTF-8 text"))?;
    let lines: Vec<&str> = text.lines().collect();
    let n = lines.len();
    let s = start.unwrap_or(1).max(1);
    let mut e = end.unwrap_or(n).min(n);
    let mut note = String::new();
    if e + 1 > s + MAX_READ_LINES {
        e = s + MAX_READ_LINES - 1;
        note = format!("\n… cut at {MAX_READ_LINES} lines; file has {n}. Ask for start={}", e + 1);
    }
    if s > n {
        return Ok(format!("{rel} has {n} lines"));
    }
    let mut out = format!("{rel} lines {s}-{e} of {n}\n");
    for (i, l) in lines[s - 1..e].iter().enumerate() {
        let _ = writeln!(out, "{:>5}  {}", s + i, clip(l));
    }
    out.push_str(&note);
    Ok(out)
}
