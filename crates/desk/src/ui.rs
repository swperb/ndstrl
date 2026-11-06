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
pub fn Page(view: View, on_answer: EventHandler<(u32, String)>) -> Element {
    rsx! {
        div { class: "app",
            header { class: "top",
                span { class: "brand", "NDSTRL" }
                span { class: "repo", "{view.repo_name}" }
                Plan { view: view.clone() }
            }
            main { class: "body",
                match (&view.spec, &view.state) {
                    (Some(_), Some(_)) => rsx! { Job { view: view.clone(), on_answer } },
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
fn Job(view: View, on_answer: EventHandler<(u32, String)>) -> Element {
    let spec = view.spec.clone().unwrap();
    let st = view.state.clone().unwrap();
    let g = view.gauges.clone().unwrap();
    let status = view.status();
    let open_q = st.questions.iter().find(|q| q.answer.is_none()).cloned();
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
                }
            }
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
