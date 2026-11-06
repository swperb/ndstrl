//! The meter: what the run has actually cost.
//!
//! Two sources, reported side by side:
//! * the harness transcript (real usage the provider billed), when the
//!   NDSTRL can find it. Claude Code writes one JSONL per session and
//!   exports `CLAUDE_CODE_SESSION_ID` to the MCP servers it spawns.
//! * NDSTRL's own meter: tokens its tools sent back to the model.
//!   Works on every harness, undercounts the model's own thinking.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// USD per million tokens.
#[derive(Debug, Clone, Copy)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

impl Price {
    const fn std(input: f64, output: f64) -> Self {
        Price { input, output, cache_write_5m: input * 1.25, cache_write_1h: input * 2.0, cache_read: input * 0.1 }
    }
    const fn read(mut self, cache_read: f64) -> Self {
        self.cache_read = cache_read;
        self
    }
}

/// Anthropic first-party list prices, cached 2026-10-06. Matched by prefix,
/// longest first, so dated or suffixed ids still price.
pub fn price_for(model: &str) -> Option<Price> {
    const TABLE: &[(&str, Price)] = &[
        ("claude-fable-5-1", Price::std(10.0, 50.0).read(0.25)),
        ("claude-mythos-5-1", Price::std(10.0, 50.0).read(0.25)),
        ("claude-fable-5", Price::std(10.0, 50.0)),
        ("claude-mythos-5", Price::std(10.0, 50.0)),
        ("claude-opus-5-5", Price::std(4.0, 20.0).read(0.20)),
        ("claude-opus-5", Price::std(5.0, 25.0)),
        ("claude-opus-4-8", Price::std(5.0, 25.0)),
        ("claude-opus-4-7", Price::std(5.0, 25.0)),
        ("claude-opus-4-6", Price::std(5.0, 25.0)),
        ("claude-opus-4-5", Price::std(5.0, 25.0)),
        ("claude-opus-4", Price::std(15.0, 75.0)),
        ("claude-sonnet-5-5", Price::std(2.0, 10.0)),
        ("claude-sonnet-5", Price::std(2.0, 10.0)),
        ("claude-sonnet-4", Price::std(3.0, 15.0)),
        ("claude-haiku-5-5", Price::std(0.10, 0.50)),
        ("claude-haiku-4-5", Price::std(1.0, 5.0)),
    ];
    let m = model.trim_start_matches("anthropic.").trim_start_matches("us.anthropic.");
    TABLE
        .iter()
        .filter(|(p, _)| m.starts_with(p))
        .max_by_key(|(p, _)| p.len())
        .map(|(_, price)| *price)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub usd: f64,
    /// Responses whose model had no price; their tokens count, dollars do not.
    pub unpriced_responses: u64,
    pub responses: u64,
    /// Size of the agent's context on its latest response (input + cache
    /// read + cache write). This is what grows until a session rots.
    #[serde(default)]
    pub last_context: u64,
}

impl Usage {
    pub fn tokens(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    /// Fold in usage from an earlier session of the same job.
    pub fn add(&mut self, o: &Usage) {
        self.input += o.input;
        self.output += o.output;
        self.cache_write_5m += o.cache_write_5m;
        self.cache_write_1h += o.cache_write_1h;
        self.cache_read += o.cache_read;
        self.usd += o.usd;
        self.unpriced_responses += o.unpriced_responses;
        self.responses += o.responses;
    }
}

/// Incrementally reads a Claude Code transcript, counting only responses
/// at or after the job opened.
pub struct TranscriptMeter {
    path: PathBuf,
    since: String,
    offset: u64,
    seen: HashSet<String>,
    pub usage: Usage,
}

impl TranscriptMeter {
    /// `since` is an RFC 3339 UTC timestamp; transcript timestamps use the
    /// same `...Z` form, so string comparison orders them.
    pub fn new(path: PathBuf, since: String) -> Self {
        Self { path, since, offset: 0, seen: HashSet::new(), usage: Usage::default() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Locate the transcript for the current harness session, if any.
    pub fn discover() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("NDSTRL_TRANSCRIPT") {
            let p = PathBuf::from(p);
            if p.exists() {
                return Some(p);
            }
        }
        let id = std::env::var("CLAUDE_CODE_SESSION_ID").ok()?;
        let base = std::env::var("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .ok()
            .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))?
            .join("projects");
        let name = format!("{id}.jsonl");
        for entry in std::fs::read_dir(&base).ok()?.flatten() {
            let candidate = entry.path().join(&name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        None
    }

    /// Read any new lines and fold them into `usage`.
    pub fn poll(&mut self) -> std::io::Result<&Usage> {
        let mut f = std::fs::File::open(&self.path)?;
        let len = f.metadata()?.len();
        if len < self.offset {
            // Rewritten (compaction or rotation): start over.
            self.offset = 0;
            self.seen.clear();
            self.usage = Usage::default();
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(f);
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 || !line.ends_with('\n') {
                break; // EOF or a line still being written
            }
            self.offset += n as u64;
            self.ingest(&line);
        }
        Ok(&self.usage)
    }

    fn ingest(&mut self, line: &str) {
        // Cheap filter before parsing: only assistant lines carry usage.
        if !line.contains("\"usage\"") {
            return;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            return;
        }
        if let Some(ts) = v.get("timestamp").and_then(|t| t.as_str()) {
            if ts < self.since.as_str() {
                return;
            }
        }
        let msg = &v["message"];
        // One API response is split over several lines, each repeating the
        // same usage. Count each response once.
        let key = v
            .get("requestId")
            .and_then(|r| r.as_str())
            .or_else(|| msg.get("id").and_then(|r| r.as_str()))
            .unwrap_or_default()
            .to_string();
        if !key.is_empty() && !self.seen.insert(key) {
            return;
        }
        let u = &msg["usage"];
        let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let input = g("input_tokens");
        let output = g("output_tokens");
        let read = g("cache_read_input_tokens");
        let (w5, w1) = match u.get("cache_creation") {
            Some(cc) => (
                cc.get("ephemeral_5m_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
                cc.get("ephemeral_1h_input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
            ),
            None => (g("cache_creation_input_tokens"), 0),
        };
        let model = msg.get("model").and_then(|m| m.as_str()).unwrap_or("");
        self.usage.input += input;
        self.usage.output += output;
        self.usage.cache_read += read;
        self.usage.cache_write_5m += w5;
        self.usage.cache_write_1h += w1;
        self.usage.responses += 1;
        self.usage.last_context = input + read + w5 + w1;
        match price_for(model) {
            Some(p) => {
                self.usage.usd += (input as f64 * p.input
                    + output as f64 * p.output
                    + read as f64 * p.cache_read
                    + w5 as f64 * p.cache_write_5m
                    + w1 as f64 * p.cache_write_1h)
                    / 1_000_000.0;
            }
            None => self.usage.unpriced_responses += 1,
        }
    }
}

/// Rough token count for text NDSTRL returns. Four bytes a token is
/// the usual English-and-code average; good enough for a meter that is
/// labelled as an estimate.
pub fn estimate_tokens(text: &str) -> u64 {
    (text.len() as u64).div_ceil(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn prices_by_longest_prefix() {
        assert_eq!(price_for("claude-opus-5-5").unwrap().cache_read, 0.20);
        assert_eq!(price_for("claude-opus-5").unwrap().input, 5.0);
        assert_eq!(price_for("claude-opus-4-20250514").unwrap().input, 15.0);
        assert_eq!(price_for("claude-opus-4-7").unwrap().input, 5.0);
        assert!(price_for("gpt-5").is_none());
    }

    #[test]
    fn dedupes_split_responses_and_respects_since() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("t.jsonl");
        let mut f = std::fs::File::create(&p).unwrap();
        let line = |rid: &str, ts: &str| {
            format!(
                r#"{{"type":"assistant","requestId":"{rid}","timestamp":"{ts}","message":{{"model":"claude-opus-5-5","usage":{{"input_tokens":1000000,"output_tokens":0,"cache_read_input_tokens":1000000,"cache_creation":{{"ephemeral_5m_input_tokens":0,"ephemeral_1h_input_tokens":0}}}}}}}}"#
            )
        };
        writeln!(f, "{}", line("old", "2026-11-06T09:00:00.000Z")).unwrap();
        writeln!(f, "{}", line("a", "2026-11-06T10:00:00.000Z")).unwrap();
        writeln!(f, "{}", line("a", "2026-11-06T10:00:00.100Z")).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{}}}}"#).unwrap();
        drop(f);
        let mut m = TranscriptMeter::new(p.clone(), "2026-11-06T09:30:00Z".into());
        let u = m.poll().unwrap().clone();
        assert_eq!(u.responses, 1);
        assert!((u.usd - 4.20).abs() < 1e-9, "{}", u.usd);
        // Appending later is picked up incrementally.
        let mut f = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
        writeln!(f, "{}", line("b", "2026-11-06T10:05:00.000Z")).unwrap();
        assert_eq!(m.poll().unwrap().responses, 2);
    }
}
