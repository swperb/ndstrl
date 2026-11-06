//! Checks: compile and test on this computer, so the model
//! never spends tokens on what a compiler can catch.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckRun {
    pub command: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub secs: f64,
    /// Short, failure-focused excerpt for the model.
    pub summary: String,
}

const KEEP_LINES: usize = 40;

pub fn run(repo: &Path, command: &str, timeout: Duration) -> CheckRun {
    let started = Instant::now();
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(repo).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // Keep colors and progress bars out of the excerpt.
    cmd.env("NO_COLOR", "1").env("CARGO_TERM_COLOR", "never").env("CI", "1");

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CheckRun {
                command: command.into(),
                passed: false,
                exit_code: None,
                timed_out: false,
                secs: 0.0,
                summary: format!("could not start the checks: {e}"),
            }
        }
    };
    // Drain both pipes on threads so a chatty test cannot deadlock us.
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = se.read_to_end(&mut b);
        b
    });
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                timed_out = true;
                break child.wait().ok();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break None,
        }
    };
    let out = String::from_utf8_lossy(&t_out.join().unwrap_or_default()).into_owned();
    let err = String::from_utf8_lossy(&t_err.join().unwrap_or_default()).into_owned();
    let exit_code = status.and_then(|s| s.code());
    let passed = !timed_out && exit_code == Some(0);
    CheckRun {
        command: command.into(),
        passed,
        exit_code,
        timed_out,
        secs: started.elapsed().as_secs_f64(),
        summary: summarize(&out, &err, passed),
    }
}

/// On pass, a one-line tail. On fail, the lines that look like failures
/// with a little context, then the tail, capped.
pub fn summarize(stdout: &str, stderr: &str, passed: bool) -> String {
    let all: Vec<&str> = stdout.lines().chain(stderr.lines()).filter(|l| !l.trim().is_empty()).collect();
    if passed {
        return all.iter().rev().take(3).rev().cloned().collect::<Vec<_>>().join("\n");
    }
    let looks_bad = |l: &str| {
        let l = l.to_ascii_lowercase();
        ["error", "fail", "panicked", "assert", "expected", "exception", "traceback", " left:", " right:", "--> "]
            .iter()
            .any(|k| l.contains(k))
    };
    let mut keep = vec![false; all.len()];
    for (i, l) in all.iter().enumerate() {
        if looks_bad(l) {
            for j in i.saturating_sub(1)..(i + 3).min(all.len()) {
                keep[j] = true;
            }
        }
    }
    let mut picked: Vec<&str> = all.iter().zip(&keep).filter(|(_, k)| **k).map(|(l, _)| *l).collect();
    if picked.is_empty() {
        picked = all.iter().rev().take(KEEP_LINES).rev().cloned().collect();
    }
    let total = picked.len();
    let mut s: Vec<String> = picked.into_iter().take(KEEP_LINES).map(|l| l.chars().take(240).collect()).collect();
    if total > KEEP_LINES {
        s.push(format!("… {} more failure lines", total - KEEP_LINES));
    }
    s.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_and_fail() {
        let d = tempfile::tempdir().unwrap();
        let ok = run(d.path(), "echo fine", Duration::from_secs(10));
        assert!(ok.passed, "{ok:?}");
        let bad = run(d.path(), "echo 'error: boom' 1>&2; exit 3", Duration::from_secs(10));
        assert!(!bad.passed);
        assert_eq!(bad.exit_code, Some(3));
        assert!(bad.summary.contains("boom"));
    }

    #[test]
    fn timeout_kills() {
        let d = tempfile::tempdir().unwrap();
        let r = run(d.path(), "sleep 5", Duration::from_millis(200));
        assert!(r.timed_out && !r.passed);
    }
}
