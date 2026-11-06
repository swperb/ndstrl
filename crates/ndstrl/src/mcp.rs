//! MCP over stdio: how every agent talks to NDSTRL.
//!
//! Hand-rolled JSON-RPC 2.0, newline delimited, so the binary has no async
//! runtime and no SDK churn. Implements initialize, ping, tools/list,
//! tools/call, and server-initiated elicitation for asking the human.

use crate::tools;
use ndstrl_core::{Job, Outcome};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::sync::mpsc::{channel, Receiver};

const SUPPORTED: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

pub const INSTRUCTIONS: &str = "NDSTRL holds the job (goal, editable paths, test command, budget) and enforces it, whichever agent you are. Call `job_open` first. Then work only through these tools: `search`, `read` and `list_files` to look at code (they return short excerpts, so do not dump whole files), `edit` and `write` to change it, and `run_checks` to run the tests on this computer instead of reasoning about them, and `delegate` to hand look-ups, docs research, and already-decided edits to a cheaper helper instead of doing them yourself. Every result ends with a status line: budget, files touched, checks, done. If the goal can only be met by going outside the job's scope (a refused path, a design choice, a second source of truth), call `ask` instead of working around it. If a result is a QUESTION, stop and put it to the user; never answer it yourself. If you are unsure whether a step fits the budget or the job, call `ask` before doing it. A MESSAGE FROM THE USER in a result is the user steering you: follow it. When `run_checks` passes, the job is done: call `receipt`, report, and stop. Do not polish after the tests pass.";

pub struct Server {
    job: Option<Job>,
    agent: String,
    elicitation: bool,
    next_id: u64,
    inbox: Receiver<Value>,
    stash: VecDeque<Value>,
    out: std::io::Stdout,
}

impl Server {
    pub fn run() -> anyhow::Result<()> {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<Value>(&line) {
                    Ok(v) => {
                        if tx.send(v).is_err() {
                            break;
                        }
                    }
                    Err(e) => eprintln!("ndstrl: bad json from client: {e}"),
                }
            }
        });
        let mut s = Server {
            job: None,
            agent: "unknown".into(),
            elicitation: false,
            next_id: 1,
            inbox: rx,
            stash: VecDeque::new(),
            out: std::io::stdout(),
        };
        loop {
            let msg = match s.stash.pop_front() {
                Some(m) => m,
                None => match s.inbox.recv() {
                    Ok(m) => m,
                    Err(_) => return Ok(()), // client closed stdin
                },
            };
            s.handle(msg);
        }
    }

    fn send(&mut self, v: Value) {
        let mut lock = self.out.lock();
        let _ = writeln!(lock, "{v}");
        let _ = lock.flush();
    }

    fn reply(&mut self, id: Value, result: Value) {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
    }

    fn error(&mut self, id: Value, code: i64, message: &str) {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }));
    }

    fn handle(&mut self, msg: Value) {
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("").to_string();
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            return; // notification: initialized, cancelled, etc.
        };
        if method.is_empty() {
            return; // a stray response to a request we already gave up on
        }
        match method.as_str() {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("");
                let version = if SUPPORTED.contains(&asked) { asked } else { SUPPORTED[0] };
                if let Some(ci) = params.get("clientInfo") {
                    let name = ci.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let ver = ci.get("version").and_then(|v| v.as_str()).unwrap_or("");
                    self.agent = format!("{name} {ver}").trim().to_string();
                }
                self.elicitation = params.pointer("/capabilities/elicitation").is_some();
                let result = json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "ndstrl", "title": "NDSTRL", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                });
                self.reply(id, result);
            }
            "ping" => self.reply(id, json!({})),
            "tools/list" => self.reply(id, json!({ "tools": tools::definitions() })),
            "tools/call" => {
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let result = self.call(&name, &args);
                self.reply(id, result);
            }
            _ => self.error(id, -32601, &format!("method not found: {method}")),
        }
    }

    fn call(&mut self, name: &str, args: &Value) -> Value {
        if name == "job_open" {
            return match tools::open(args, &self.agent) {
                Ok((job, outcome)) => {
                    self.job = Some(job);
                    // An auto-mode draft goes straight to the person when the
                    // client can ask them; otherwise the agent relays it.
                    let outcome = match outcome {
                        Outcome::Ask(q) if self.elicitation => match self.elicit(q).0 {
                            Outcome::Ok(t) => Outcome::Ok(format!("{t}\n\n{}", tools::describe(self.job.as_ref().expect("job")))),
                            o => o,
                        },
                        o => o,
                    };
                    let g = self.job.as_ref().map(|j| j.gauges());
                    render(outcome, g)
                }
                Err(e) => render(Outcome::Denied(format!("{e:#}")), None),
            };
        }
        if self.job.is_none() {
            // Resume a spec already sitting in the working directory.
            if let Ok(cwd) = std::env::current_dir() {
                if ndstrl_core::JobSpec::path(&tools::default_repo(&cwd)).exists() {
                    if let Ok(j) = Job::open(&tools::default_repo(&cwd), None, &self.agent) {
                        self.job = Some(j);
                    }
                }
            }
        }
        let Some(job) = self.job.as_mut() else {
            return render(
                Outcome::Denied("no job is open. Call `job_open` with the repo and the user's goal first.".into()),
                None,
            );
        };
        let outcome = tools::dispatch(job, name, args);
        let outcome = match outcome {
            Outcome::Ask(q) if self.elicitation => {
                let from_agent = matches!(q.kind, ndstrl_core::job::QuestionKind::Agent);
                let (o, approved) = self.elicit(q);
                if approved && !from_agent {
                    // The human cleared the gate; run the held step now
                    // instead of making the agent call again.
                    let job = self.job.as_mut().expect("job open");
                    let pre = match &o {
                        Outcome::Ok(t) => t.clone(),
                        _ => String::new(),
                    };
                    match tools::dispatch(job, name, args) {
                        Outcome::Ok(t) => Outcome::Ok(format!("{pre}\n{t}")),
                        other => other,
                    }
                } else {
                    o
                }
            }
            o => o,
        };
        // Anything the person typed in the window rides on this result.
        let outcome = match self.job.as_mut().map(|j| j.take_notes()).unwrap_or_default() {
            notes if notes.is_empty() => outcome,
            notes => {
                let msg = notes.iter().map(|n| format!("MESSAGE FROM THE USER (sent from the NDSTRL window): {n}")).collect::<Vec<_>>().join("\n");
                match outcome {
                    Outcome::Ok(t) => Outcome::Ok(format!("{msg}\n\n{t}")),
                    Outcome::Denied(t) => Outcome::Denied(format!("{t}\n\n{msg}")),
                    Outcome::Blocked(t) => Outcome::Blocked(format!("{t}\n\n{msg}")),
                    Outcome::Ask(q) => Outcome::Ask(q),
                }
            }
        };
        let g = self.job.as_ref().map(|j| j.gauges());
        render(outcome, g)
    }

    /// Ask the human directly through the client, bypassing the model.
    fn elicit(&mut self, q: ndstrl_core::Question) -> (Outcome, bool) {
        let id = format!("ndstrl-{}", self.next_id);
        self.next_id += 1;
        let schema = json!({
            "type": "object",
            "properties": { "choice": { "type": "string", "title": "Your call", "enum": q.options } },
            "required": ["choice"],
        });
        self.send(json!({
            "jsonrpc": "2.0", "id": id, "method": "elicitation/create",
            "params": { "message": format!("NDSTRL: {}", q.text), "requestedSchema": schema },
        }));
        // Wait for our response; anything else that arrives is handled later.
        let resp = loop {
            match self.inbox.recv() {
                Ok(m) if m.get("id").and_then(|v| v.as_str()) == Some(id.as_str()) && m.get("method").is_none() => break Some(m),
                Ok(m) => self.stash.push_back(m),
                Err(_) => break None,
            }
        };
        let job = self.job.as_mut().expect("job open while eliciting");
        let action = resp.as_ref().and_then(|r| r.pointer("/result/action")).and_then(|a| a.as_str());
        match action {
            Some("accept") => {
                let choice = resp
                    .as_ref()
                    .and_then(|r| r.pointer("/result/content/choice"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string();
                let yes = q.options.first().map(|f| f == &choice).unwrap_or(false);
                let declined = q.options.len() > 1 && q.options.last() == Some(&choice);
                let o = match job.answer(q.id, &choice, "human") {
                    Outcome::Ok(t) if declined => Outcome::Denied(format!("The user said no (\"{choice}\"). {t}")),
                    Outcome::Ok(t) => Outcome::Ok(format!("NDSTRL asked the user directly. They chose \"{choice}\". {t}")),
                    o => o,
                };
                (o, yes)
            }
            Some("decline") => {
                let no = q.options.last().cloned().unwrap_or_else(|| "no".into());
                let o = match job.answer(q.id, &no, "human") {
                    Outcome::Ok(t) => Outcome::Denied(format!("The user declined. {t}")),
                    o => o,
                };
                (o, false)
            }
            // Cancelled, unsupported in practice, or an error: fall back to the relay.
            _ => (Outcome::Ask(q), false),
        }
    }
}

pub fn render(o: Outcome, g: Option<ndstrl_core::Gauges>) -> Value {
    let gline = g.as_ref().map(|g| g.line()).unwrap_or_else(|| "[ndstrl] no job open".into());
    let (status, body, is_error, question) = match o {
        Outcome::Ok(t) => ("ok", t, false, Value::Null),
        Outcome::Denied(t) => ("denied", format!("DENIED: {t}"), true, Value::Null),
        Outcome::Blocked(t) => ("blocked", format!("BLOCKED: {t}"), false, Value::Null),
        Outcome::Ask(q) => {
            let opts = if q.options.is_empty() { String::new() } else { format!("\nOptions: {}", q.options.join(" | ")) };
            let text = format!(
                "QUESTION {} for the user. Do not answer it yourself and do not work around it.\n{}{}\nStop, put this to the user, then call `answer` with question_id={} and their words.",
                q.id, q.text, opts, q.id
            );
            ("ask", text, false, serde_json::to_value(&q).unwrap_or_default())
        }
    };
    // Some harnesses (Claude Code among them) hand the model only
    // structuredContent when it is present, so the full text rides in it too.
    let text = format!("{body}\n{gline}");
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": { "result": text, "status": status, "gauges": g, "question": question },
        "isError": is_error,
    })
}
