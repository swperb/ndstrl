//! The NDSTRL window: one screen. What is the agent doing, does it need
//! you, and how much of your plan is left.

use crate::model::{Status, View};
use dioxus::prelude::*;

pub const CSS: &str = include_str!("../assets/desk.css");

fn money(x: f64) -> String {
    format!("${x:.2}")
}

fn until(epoch: i64) -> String {
    let secs = epoch - chrono::Utc::now().timestamp();
    if secs <= 0 {
        return "resetting".into();
    }
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    if d > 0 { format!("{d}d {h}h") } else if h > 0 { format!("{h}h {m}m") } else { format!("{m}m") }
}

#[component]
pub fn Page(view: View, on_answer: EventHandler<(u32, String)>, on_steer: EventHandler<String>) -> Element {
    rsx! {
        div { class: "app",
            header { class: "top",
                span { class: "brand", "NDSTRL" }
                span { class: "repo", "{view.repo_name}" }
                Plan { view: view.clone() }
            }
            main { class: "body",
                match (&view.spec, &view.state) {
                    (Some(_), Some(_)) => rsx! { Job { view: view.clone(), on_answer, on_steer } },
                    _ => rsx! { Idle {} },
                }
            }
        }
    }
}

#[component]
fn Plan(view: View) -> Element {
    let g = view.gauges.clone();
    let snap = view.plan.clone();
    let five = g.as_ref().and_then(|g| g.plan_5h).map(|e| (e.used_percentage, e.resets_at, e.projected))
        .or_else(|| snap.as_ref().and_then(|s| s.five_hour).map(|w| (w.used_percentage, w.resets_at, false)));
    let week = g.as_ref().and_then(|g| g.plan_7d).map(|e| (e.used_percentage, e.resets_at, e.projected))
        .or_else(|| snap.as_ref().and_then(|s| s.seven_day).map(|w| (w.used_percentage, w.resets_at, false)));
    rsx! {
        div { class: "plan", title: "Claude plan usage, from Claude Code's status line",
            match five {
                Some((p, r, est)) => rsx! { span { "5h " b { "{p:.0}%" } if est { "~" } span { class: "dim", " · {until(r)}" } } },
                None => rsx! { span { class: "dim", "plan usage appears after a Claude Code session" } },
            }
            if let Some((p, r, est)) = week {
                span { "week " b { "{p:.0}%" } if est { "~" } span { class: "dim", " · {until(r)}" } }
            }
        }
    }
}

#[component]
fn Job(view: View, on_answer: EventHandler<(u32, String)>, on_steer: EventHandler<String>) -> Element {
    let spec = view.spec.clone().unwrap();
    let st = view.state.clone().unwrap();
    let g = view.gauges.clone().unwrap();
    let status = view.status();
    let open_q = st.questions.iter().find(|q| q.answer.is_none()).cloned();
    let decisions: Vec<_> = st.questions.iter().filter(|q| q.answer.is_some()).rev().take(8).cloned().collect();
    let mut draft = use_signal(String::new);
    // An open question from the agent itself is answered by what you type;
    // otherwise what you type is a message steering the agent.
    let agent_q = open_q.as_ref().filter(|q| matches!(q.kind, ndstrl_core::job::QuestionKind::Agent)).map(|q| q.id);
    let send = move |_| {
        let text = draft.peek().trim().to_string();
        if text.is_empty() {
            return;
        }
        match agent_q {
            Some(id) => on_answer.call((id, text)),
            None => on_steer.call(text),
        }
        draft.set(String::new());
    };
    let agent = if st.agent.is_empty() { "The agent".to_string() } else { st.agent.split(' ').next().unwrap_or("").to_string() };
    let files = st.touched.len();
    let checks = match st.check_runs.last() {
        None => "checks not run yet".to_string(),
        Some(_) if st.edits_since_inspect > 0 && !st.done => "checks out of date".to_string(),
        Some(r) if r.passed => "checks pass".to_string(),
        Some(_) => "checks failing".to_string(),
    };
    let (dot, headline) = match status {
        Status::Asking => ("ask", "Needs your answer".to_string()),
        Status::Done => ("ok", "Done".to_string()),
        Status::Capped => ("bad", "Stopped at budget".to_string()),
        _ => ("run", format!("{agent} is working")),
    };
    let detail = format!("{files} file{} changed · {checks}", if files == 1 { "" } else { "s" });
    let spend = g.spent_usd;
    let frac = match (spend, spec.budget_usd) {
        (Some(s), Some(b)) if b > 0.0 => Some((s / b).clamp(0.0, 1.0)),
        _ => None,
    };
    rsx! {
        h1 { class: "goal", "{spec.goal}" }
        div { class: "status",
            span { class: "dot is-{dot}" }
            span { class: "headline", "{headline}" }
            span { class: "dim", "{detail}" }
        }
        if let Some(q) = open_q {
            div { class: "ask",
                p { "{q.text}" }
                div { class: "buttons",
                    for (i, o) in q.options.iter().enumerate() {
                        button {
                            class: if i == 0 { "btn primary" } else { "btn" },
                            onclick: {
                                let o = o.clone();
                                let id = q.id;
                                move |_| on_answer.call((id, o.clone()))
                            },
                            "{o}"
                        }
                    }
                    button {
                        class: "btn ghost",
                        title: "Decline this and have the agent carry on with the rest of the job",
                        onclick: {
                            let id = q.id;
                            let skip = if matches!(q.kind, ndstrl_core::job::QuestionKind::Agent) { "skip".to_string() } else { q.options.last().cloned().unwrap_or_else(|| "skip".into()) };
                            move |_| on_answer.call((id, skip.clone()))
                        },
                        "Skip this"
                    }
                }
            }
        }
        div { class: "steer",
            input {
                class: "steer-input",
                r#type: "text",
                placeholder: if agent_q.is_some() { "Answer the agent…" } else { "Message the agent: steer it, narrow it, tell it to skip something…" },
                value: "{draft}",
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e| {
                    if e.key() == Key::Enter {
                        let text = draft.peek().trim().to_string();
                        if !text.is_empty() {
                            match agent_q {
                                Some(id) => on_answer.call((id, text)),
                                None => on_steer.call(text),
                            }
                            draft.set(String::new());
                        }
                    }
                },
            }
            button { class: "btn", onclick: send, "Send" }
        }
        div { class: "budget",
            div { class: "bar", i { style: "width: {frac.unwrap_or(0.0) * 100.0:.1}%" } }
            div { class: "budget-text",
                match (spend, spec.budget_usd) {
                    (Some(s), Some(b)) => rsx! { span { "{money(s)} of {money(b)}" } span { class: "dim", " at API prices" } },
                    (Some(s), None) => rsx! { span { "{money(s)}" } span { class: "dim", " at API prices, no cap" } },
                    _ => rsx! { span { class: "dim", "cost not metered for this agent" } },
                }
            }
        }
        if !st.notes.is_empty() {
            ul { class: "log",
                for n in st.notes.iter().rev().take(4) {
                    li { span { class: "dim", if n.delivered { "You told the agent: " } else { "Waiting for the agent's next step: " } } "{n.text}" }
                }
            }
        }
        if !st.touched.is_empty() {
            ul { class: "files",
                for (path, t) in st.touched.iter() {
                    li {
                        span { class: "path", "{path}" }
                        span { class: "add", "+{t.added}" }
                        span { class: "del", "−{t.removed}" }
                    }
                }
            }
        }
        if !decisions.is_empty() {
            div { class: "section-label", "Decisions" }
            ul { class: "log",
                for q in decisions.iter() {
                    li {
                        span { "{q.text}" }
                        span { class: "dim", " → {q.answer.clone().unwrap_or_default()}" }
                        if q.answered_via.as_deref() == Some("agent") { span { class: "dim", " (relayed by the agent)" } }
                    }
                }
            }
        }
    }
}

#[component]
fn Idle() -> Element {
    rsx! {
        div { class: "idle",
            h1 { class: "goal", "Nothing running" }
            p { class: "dim", "Start a session in Claude Code with a goal and a budget. It shows up here." }
            code { "Fix the failing invoice test. Don't touch billing. Stop at $4." }
        }
    }
}
