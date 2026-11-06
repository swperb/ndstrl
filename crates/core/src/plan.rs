//! Plan usage: how much of the subscription's 5-hour and 7-day windows
//! are used, as close to the truth as we can get without spending usage.
//!
//! Source: Claude Code hands its status line command `rate_limits` on
//! stdin (Pro/Max/Ultra, after the first response of a session), derived
//! from response headers it already receives. `ndstrl statusline` records
//! those snapshots under `~/.ndstrl/`. Limits are account-wide, so any
//! session refreshes them; between refreshes NDSTRL projects forward
//! using a learned "plan percent per list-price dollar" rate.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
pub struct Window {
    pub used_percentage: f64,
    /// Unix epoch seconds.
    pub resets_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    /// Unix epoch seconds when observed.
    pub at: i64,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    /// The session's list-price cost at the moment of the snapshot, from the
    /// same status line payload. Pairs with the windows for calibration.
    #[serde(default)]
    pub session_list_usd: Option<f64>,
    #[serde(default)]
    pub session_id: Option<String>,
}

pub fn home() -> Option<PathBuf> {
    std::env::var("NDSTRL_HOME").map(PathBuf::from).ok().or_else(|| dirs::home_dir().map(|h| h.join(".ndstrl")))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Parse the status line payload.
pub fn from_statusline(v: &serde_json::Value) -> Option<Snapshot> {
    let rl = v.get("rate_limits")?;
    let win = |k: &str| -> Option<Window> {
        let w = rl.get(k)?;
        Some(Window {
            used_percentage: w.get("used_percentage")?.as_f64()?,
            resets_at: w.get("resets_at").and_then(|x| x.as_i64()).unwrap_or(0),
        })
    };
    let s = Snapshot {
        at: now(),
        five_hour: win("five_hour"),
        seven_day: win("seven_day"),
        session_list_usd: v.pointer("/cost/total_cost_usd").and_then(|x| x.as_f64()),
        session_id: v.get("session_id").and_then(|x| x.as_str()).map(String::from),
    };
    (s.five_hour.is_some() || s.seven_day.is_some()).then_some(s)
}

pub fn latest() -> Option<Snapshot> {
    let p = home()?.join("plan.json");
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// Store a snapshot. The history only grows when a percentage moves, so
/// a status line that fires every few hundred milliseconds stays cheap.
pub fn record(s: &Snapshot) -> std::io::Result<()> {
    let Some(dir) = home() else { return Ok(()) };
    std::fs::create_dir_all(&dir)?;
    let prev = latest();
    let moved = prev.as_ref().map(|p| p.five_hour != s.five_hour || p.seven_day != s.seven_day).unwrap_or(true);
    let tmp = dir.join("plan.json.tmp");
    std::fs::write(&tmp, serde_json::to_string(s)?)?;
    std::fs::rename(tmp, dir.join("plan.json"))?;
    if moved {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("plan.jsonl"))?;
        writeln!(f, "{}", serde_json::to_string(s)?)?;
    }
    Ok(())
}

/// Learned plan percentage per list-price dollar, per window, from
/// consecutive snapshots of one session inside one window. Median of the
/// observed slopes, so one odd reading does not swing it.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct Rates {
    pub five_hour_pct_per_usd: Option<f64>,
    pub seven_day_pct_per_usd: Option<f64>,
    pub samples: usize,
}

pub fn rates() -> Rates {
    let Some(dir) = home() else { return Rates::default() };
    let Ok(text) = std::fs::read_to_string(dir.join("plan.jsonl")) else { return Rates::default() };
    let snaps: Vec<Snapshot> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    let mut five = Vec::new();
    let mut seven = Vec::new();
    for w in snaps.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if a.session_id.is_none() || a.session_id != b.session_id {
            continue;
        }
        let (Some(ua), Some(ub)) = (a.session_list_usd, b.session_list_usd) else { continue };
        let du = ub - ua;
        if du < 0.05 {
            continue; // too small to say anything
        }
        let slope = |x: Option<Window>, y: Option<Window>| -> Option<f64> {
            let (x, y) = (x?, y?);
            (x.resets_at == y.resets_at && y.used_percentage >= x.used_percentage)
                .then(|| (y.used_percentage - x.used_percentage) / du)
        };
        if let Some(s) = slope(a.five_hour, b.five_hour) {
            five.push(s);
        }
        if let Some(s) = slope(a.seven_day, b.seven_day) {
            seven.push(s);
        }
    }
    let median = |mut v: Vec<f64>| -> Option<f64> {
        if v.is_empty() {
            return None;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        Some(v[v.len() / 2])
    };
    let samples = five.len().max(seven.len());
    Rates { five_hour_pct_per_usd: median(five), seven_day_pct_per_usd: median(seven), samples }
}

/// Best current estimate of one window, given list-price dollars spent
/// since the snapshot was taken.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Estimate {
    pub used_percentage: f64,
    pub resets_at: i64,
    /// Seconds since the snapshot it rests on.
    pub age_secs: i64,
    /// True when projected forward with a learned rate.
    pub projected: bool,
}

pub fn estimate(snap: &Snapshot, which: &str, usd_since: f64, rates: &Rates) -> Option<Estimate> {
    let (w, rate) = match which {
        "five_hour" => (snap.five_hour?, rates.five_hour_pct_per_usd),
        _ => (snap.seven_day?, rates.seven_day_pct_per_usd),
    };
    let t = now();
    if w.resets_at > 0 && t >= w.resets_at {
        return None; // the window rolled over; the snapshot says nothing now
    }
    let (pct, projected) = match rate {
        Some(r) if usd_since > 0.0 => ((w.used_percentage + r * usd_since).min(100.0), true),
        _ => (w.used_percentage, false),
    };
    Some(Estimate { used_percentage: pct, resets_at: w.resets_at, age_secs: t - snap.at, projected })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_learns() {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("NDSTRL_HOME", d.path());
        let mk = |p5: f64, p7: f64, usd: f64| {
            serde_json::json!({
                "session_id": "s1", "cost": { "total_cost_usd": usd },
                "rate_limits": { "five_hour": { "used_percentage": p5, "resets_at": 4102444800i64 },
                                 "seven_day": { "used_percentage": p7, "resets_at": 4102444800i64 } }
            })
        };
        assert!(from_statusline(&serde_json::json!({ "model": {} })).is_none());
        record(&from_statusline(&mk(10.0, 40.0, 0.0)).unwrap()).unwrap();
        record(&from_statusline(&mk(12.0, 40.5, 1.0)).unwrap()).unwrap();
        record(&from_statusline(&mk(14.0, 41.0, 2.0)).unwrap()).unwrap();
        let r = rates();
        assert_eq!(r.five_hour_pct_per_usd, Some(2.0));
        assert_eq!(r.seven_day_pct_per_usd, Some(0.5));
        let e = estimate(&latest().unwrap(), "five_hour", 0.5, &r).unwrap();
        assert!((e.used_percentage - 15.0).abs() < 1e-9 && e.projected);
    }
}
