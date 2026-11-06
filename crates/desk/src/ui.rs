//! Front office components. Pure functions of a `View`, so the desktop
//! webview and the static snapshot render the exact same markup.

use crate::model::{Event, ReceiptRow, Status, View};
use dioxus::prelude::*;
use ndstrl_core::Gauges;

pub const CSS: &str = include_str!("../assets/desk.css");

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tab {
    Floor,
    Receipts,
}

// ---- formatting ---------------------------------------------------------

fn money(x: f64) -> String {
    if x.abs() < 10.0 { format!("${x:.2}") } else { format!("${x:.0}") }
}

fn compact(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.2}M", n as f64 / 1e6),
    }
}

fn clock(rfc3339: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| t.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
        .unwrap_or_default()
}

fn until(epoch: i64) -> String {
    let secs = epoch - chrono::Utc::now().timestamp();
    if secs <= 0 {
        return "resetting".into();
    }
    let (d, h, m) = (secs / 86_400, (secs % 86_400) / 3600, (secs % 3600) / 60);
    if d > 0 { format!("resets in {d}d {h}h") } else if h > 0 { format!("resets in {h}h {m}m") } else { format!("resets in {m}m") }
}

fn bar_class(frac_used: f64) -> &'static str {
    if frac_used >= 0.9 { "bar bad" } else if frac_used >= 0.75 { "bar warn" } else { "bar" }
}

// ---- icons (inline, stroke follows currentColor) ---------------------------

#[component]
fn Icon(name: &'static str) -> Element {
    let d = match name {
        "floor" => "M3 21V9l6 4V9l6 4V5l6 4v12z",
        "receipt" => "M6 3h12v18l-3-2-3 2-3-2-3 2zM9 8h6M9 12h6",
        "bench" => "M4 20V10M10 20V4M16 20v-7M22 20H2",
        "alert" => "M12 3l9 16H3zM12 10v4M12 17h.01",
        _ => "M12 12h.01",
    };
    rsx! {
        svg { view_box: "0 0 24 24", fill: "none", stroke: "currentColor", stroke_width: "1.7", stroke_linecap: "round", stroke_linejoin: "round",
            path { d: "{d}" }
        }
    }
}

#[component]
fn Mark() -> Element {
    // A hatched die block: the shop's stamp.
    rsx! {
        svg { class: "brand-mark", view_box: "0 0 24 24", fill: "none",
            rect { x: "2.5", y: "2.5", width: "19", height: "19", rx: "4", fill: "var(--accent)" }
            path { d: "M7 17L17 7M7 12l5-5M12 17l5-5", stroke: "var(--bg)", stroke_width: "2", stroke_linecap: "round" }
        }
    }
}

// ---- page ------------------------------------------------------------------

#[component]
pub fn Page(view: View, tab: Tab, on_tab: EventHandler<Tab>) -> Element {
    let receipts = view.receipts.len();
    rsx! {
        div { class: "shell",
            aside { class: "side",
                div { class: "brand",
                    Mark {}
                    span { class: "brand-name", "NDSTRL" }
                    span { class: "brand-ver", "v{env!(\"CARGO_PKG_VERSION\")}" }
                }
                nav { class: "nav",
                    div { class: "nav-label", "Shop" }
                    button { class: if tab == Tab::Floor { "nav-item active" } else { "nav-item" }, onclick: move |_| on_tab.call(Tab::Floor),
                        Icon { name: "floor" } "Floor"
                    }
                    button { class: if tab == Tab::Receipts { "nav-item active" } else { "nav-item" }, onclick: move |_| on_tab.call(Tab::Receipts),
                        Icon { name: "receipt" } "Receipts"
                        span { class: "nav-count", "{receipts}" }
                    }
                }
                div { class: "side-foot",
                    div { class: "repo-name", "{view.repo_name}" }
                    div { class: "repo-path", title: "{view.repo.display()}", "{view.repo.display()}" }
                }
            }
            main { class: "main",
                match tab {
                    Tab::Floor => rsx! { Floor { view: view.clone() } },
                    Tab::Receipts => rsx! { Receipts { rows: view.receipts.clone() } },
                }
            }
        }
    }
}

// ---- floor -----------------------------------------------------------------

#[component]
fn Floor(view: View) -> Element {
    let (Some(t), Some(st), Some(g)) = (view.traveler.clone(), view.state.clone(), view.gauges.clone()) else {
        return rsx! { EmptyFloor { view: view.clone() } };
    };
    let status = view.status();
    let (pill_class, pill_text) = match status {
        Status::Running => ("pill run", "Running"),
        Status::Asking => ("pill ask", "Waiting on you"),
        Status::Done => ("pill ok", "Inspection passed"),
        Status::Capped => ("pill bad", "Stopped at cap"),
        Status::Idle => ("pill idle", "Idle"),
    };
    let open_q = st.questions.iter().find(|q| q.answer.is_none()).cloned();
    let machine = if st.machine.is_empty() { "unknown machine".to_string() } else { st.machine.clone() };
    rsx! {
        header { class: "head",
            div { class: "head-text",
                div { class: "eyebrow",
                    span { "JOB" }
                    b { "{t.id}" }
                    span { "·" }
                    span { "{machine}" }
                }
                h1 { "{t.goal}" }
            }
            span { class: "{pill_class}", span { class: "dot" } "{pill_text}" }
        }
        if let Some(q) = open_q {
            div { class: "question", role: "status",
                div { class: "question-icon", Icon { name: "alert" } }
                div { class: "question-title", "Question {q.id} is waiting on you" }
                div { class: "question-text", "{q.text}" }
                div { class: "question-opts",
                    for o in q.options.iter() {
                        span { class: "chip opt", "{o}" }
                    }
                }
            }
        }
        Kpis {
            g: g.clone(),
            budget_usd: t.budget_usd,
            runs: st.critic_runs.len(),
            last_passed: st.critic_runs.last().map(|r| r.passed),
            edits_since: st.edits_since_inspect,
            cap5: t.budget_plan_5h_pct,
            cap7: t.budget_plan_7d_pct,
        }
        div { class: "grid",
            div { style: "display:flex;flex-direction:column;gap:16px;min-width:0",
                section { class: "card",
                    div { class: "card-head", span { class: "card-title", "Traveler" } span { class: "card-meta", ".ndstrl/job.toml" } }
                    div { class: "card-body",
                        dl { class: "spec",
                            dt { "May edit" }
                            dd {
                                if t.allow.is_empty() { span { class: "chip", "whole repo" } }
                                for a in t.allow.iter() { span { class: "chip", "{a}" } }
                                for w in st.widened.iter() { span { class: "chip", title: "allowed by your answer", "{w} ✓" } }
                            }
                            dt { "Off limits" }
                            dd {
                                if t.deny.is_empty() { span { class: "muted", "nothing" } }
                                for d in t.deny.iter() { span { class: "chip deny", "{d}" } }
                            }
                            dt { "Inspection" }
                            dd {
                                match &t.test {
                                    Some(cmd) => rsx! { span { class: "chip", "{cmd}" } },
                                    None => rsx! { span { class: "muted", "none, so done is a judgment call" } },
                                }
                            }
                            dt { "Caps" }
                            dd {
                                if let Some(b) = t.budget_usd { span { class: "chip", "{money(b)} list" } }
                                if let Some(b) = t.budget_tokens { span { class: "chip", "{compact(b)} tok" } }
                                if let Some(p) = t.budget_plan_5h_pct { span { class: "chip", "{p}% of 5h" } }
                                if let Some(p) = t.budget_plan_7d_pct { span { class: "chip", "{p}% of week" } }
                                if t.budget_usd.is_none() && t.budget_tokens.is_none() && t.budget_plan_5h_pct.is_none() && t.budget_plan_7d_pct.is_none() {
                                    span { class: "muted", "none" }
                                }
                            }
                        }
                    }
                }
                section { class: "card",
                    div { class: "card-head", span { class: "card-title", "Files touched" } span { class: "card-meta", "{st.touched.len()}" } }
                    div { class: "card-body",
                        if st.touched.is_empty() {
                            div { class: "empty", "No edits yet." }
                        } else {
                            table { class: "files",
                                tbody {
                                    for (path, touch) in st.touched.iter() {
                                        tr {
                                            td { class: "path", "{path}" if touch.created { span { class: "tag", "NEW" } } }
                                            td { class: "num add", "+{touch.added}" }
                                            td { class: "num del", "−{touch.removed}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            section { class: "card",
                div { class: "card-head", span { class: "card-title", "Activity" } span { class: "card-meta", "{view.events.len()} events" } }
                div { class: "card-body",
                    if view.events.is_empty() {
                        div { class: "empty", "Nothing has happened yet." }
                    } else {
                        ol { class: "timeline",
                            for e in view.events.iter().take(60) {
                                Row { e: e.clone() }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn Kpis(
    g: Gauges,
    budget_usd: Option<f64>,
    runs: usize,
    last_passed: Option<bool>,
    edits_since: u32,
    cap5: Option<f64>,
    cap7: Option<f64>,
) -> Element {
    let spend_frac = match (g.spent_usd, budget_usd) {
        (Some(s), Some(b)) if b > 0.0 => Some((s / b).clamp(0.0, 1.0)),
        _ => None,
    };
    let (spend_value, spend_sub) = match (g.spent_usd, budget_usd) {
        (Some(s), Some(b)) => (money(s), format!("of {} list-equiv cap", money(b))),
        (Some(s), None) => (money(s), "list-equiv, no cap".to_string()),
        (None, _) => ("—".into(), "dollars not metered on this machine".into()),
    };
    let win = |e: Option<ndstrl_core::plan::Estimate>, job: Option<f64>, cap: Option<f64>| -> (String, String, bool, Option<f64>) {
        match e {
            Some(e) => (
                format!("{:.0}%", e.used_percentage),
                match (job, cap) {
                    (Some(j), Some(c)) => format!("job {j:.1} of {c} pts · {}", until(e.resets_at)),
                    (Some(j), None) if j >= 0.05 => format!("job +{j:.1} pts · {}", until(e.resets_at)),
                    _ => until(e.resets_at),
                },
                e.projected,
                Some(e.used_percentage / 100.0),
            ),
            None => ("—".into(), "no status line reading yet".into(), false, None),
        }
    };
    let (h5, h5s, h5p, h5f) = win(g.plan_5h, g.job_5h_pts, cap5);
    let (d7, d7s, d7p, d7f) = win(g.plan_7d, g.job_7d_pts, cap7);
    // Show the last verdict, but say plainly when edits have landed since.
    let (critic_class, critic_text) = match last_passed {
        None => ("kpi-value", "Not run"),
        Some(true) if edits_since == 0 || g.done => ("kpi-value ok", "Pass"),
        Some(true) => ("kpi-value stale", "Pass"),
        Some(false) if edits_since == 0 => ("kpi-value bad", "Fail"),
        Some(false) => ("kpi-value stale", "Fail"),
    };
    let critic_sub = if g.done {
        "part done · edits closed".to_string()
    } else if edits_since > 0 && runs > 0 {
        format!("stale · {edits_since} edit{} since last run", if edits_since == 1 { "" } else { "s" })
    } else {
        format!("{runs} run{}", if runs == 1 { "" } else { "s" })
    };
    rsx! {
        div { class: "kpis",
            div { class: "kpi",
                div { class: "kpi-label", "Spend" }
                div { class: "kpi-value", "{spend_value}" }
                if let Some(f) = spend_frac { div { class: bar_class(f), i { style: "width: {f * 100.0:.1}%" } } }
                div { class: "kpi-sub", "{spend_sub}" }
            }
            div { class: "kpi",
                div { class: "kpi-label", "5-hour window" }
                div { class: "kpi-value", "{h5}" if h5p { span { class: "est", title: "projected from list spend since the last reading", "~" } } }
                if let Some(f) = h5f { div { class: bar_class(f), i { style: "width: {f * 100.0:.1}%" } } }
                div { class: "kpi-sub", "{h5s}" }
            }
            div { class: "kpi",
                div { class: "kpi-label", "Weekly window" }
                div { class: "kpi-value", "{d7}" if d7p { span { class: "est", "~" } } }
                if let Some(f) = d7f { div { class: bar_class(f), i { style: "width: {f * 100.0:.1}%" } } }
                div { class: "kpi-sub", "{d7s}" }
            }
            div { class: "kpi",
                div { class: "kpi-label", "Tokens" }
                div { class: "kpi-value", "{compact(g.tokens)}" }
                div { class: "kpi-sub", "{g.meter}" }
            }
            div { class: "kpi",
                div { class: "kpi-label", "Inspection" }
                div { class: "{critic_class}", "{critic_text}" }
                div { class: "kpi-sub", "{critic_sub}" }
            }
        }
    }
}

#[component]
fn Row(e: Event) -> Element {
    let dot = match e.kind.as_str() {
        "pass" | "receipt" => "tl-dot ok",
        "fail" | "deny" | "cap" => "tl-dot bad",
        "ask" => "tl-dot ask",
        "edit" => "tl-dot edit",
        "inspect_start" | "open" => "tl-dot run",
        _ => "tl-dot",
    };
    rsx! {
        li { class: "tl",
            span { class: "tl-time", "{clock(&e.at)}" }
            span { class: "tl-rail", span { class: "{dot}" } }
            div { class: "tl-body",
                div { class: "tl-title", "{e.title}" }
                if !e.detail.is_empty() { div { class: "tl-detail", "{e.detail}" } }
            }
        }
    }
}

#[component]
fn EmptyFloor(view: View) -> Element {
    rsx! {
        header { class: "head",
            div { class: "head-text",
                div { class: "eyebrow", span { "FLOOR" } b { "{view.repo_name}" } }
                h1 { "No part on the floor" }
            }
            span { class: "pill idle", span { class: "dot" } "Idle" }
        }
        div { class: "hero-empty",
            h2 { "Send a part from any harness" }
            p { "Start a session in Claude Code or Codex with ndstrl installed, and give it a goal, limits, and a budget. The job shows up here as it runs." }
            span { class: "prompt-sample", "Fix the failing invoice test. Don't touch billing. Stop at $4." }
        }
    }
}

// ---- receipts --------------------------------------------------------------

#[component]
fn Receipts(rows: Vec<ReceiptRow>) -> Element {
    rsx! {
        header { class: "head",
            div { class: "head-text",
                div { class: "eyebrow", span { "RECEIPTS" } b { "{rows.len()} jobs" } }
                h1 { "Every run, what it cost, what it touched" }
            }
        }
        section { class: "card",
            div { class: "card-body",
                if rows.is_empty() {
                    div { class: "empty", "No receipts yet. A receipt is written when a part passes, hits its cap, or the machine calls receipt." }
                } else {
                    table { class: "receipts",
                        thead { tr { th { "Job" } th { "Result" } th { class: "r", "Files" } th { class: "r", "Spend" } } }
                        tbody {
                            for r in rows.iter() {
                                tr {
                                    td {
                                        div { "{r.goal}" }
                                        div { class: "mono muted", "{r.id}" }
                                    }
                                    td {
                                        span { class: match r.result.as_str() { "passed" => "pill ok", "capped" => "pill bad", _ => "pill idle" }, span { class: "dot" } "{r.result}" }
                                    }
                                    td { class: "r", "{r.files}" }
                                    td { class: "r", {r.usd.map(money).unwrap_or_else(|| "—".into())} }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
