//! `ndstrl-desk [repo]`: the NDSTRL window. Watches the repo's
//! `.ndstrl` files and the plan snapshot, once a second.

use dioxus::desktop::{Config, LogicalSize, WindowBuilder};
use dioxus::prelude::*;
use ndstrl_desk::{model, ui};
use std::path::PathBuf;
use std::sync::OnceLock;

static REPO: OnceLock<PathBuf> = OnceLock::new();

fn main() {
    let start = std::env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| std::env::current_dir().unwrap());
    let repo = model::default_repo(&start.canonicalize().unwrap_or(start));
    REPO.set(repo).ok();
    let cfg = Config::new()
        .with_custom_head(format!("<style>{}</style>", ui::CSS))
        .with_menu(None)
        .with_window(
            WindowBuilder::new()
                .with_title("NDSTRL")
                .with_inner_size(LogicalSize::new(760.0, 620.0))
                .with_min_inner_size(LogicalSize::new(380.0, 520.0)),
        );
    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(Root);
}

#[component]
fn Root() -> Element {
    let repo = REPO.get().cloned().unwrap_or_default();
    let mut view = use_signal({
        let repo = repo.clone();
        move || model::load(&repo)
    });
    let answer_repo = repo.clone();
    let steer_repo = repo.clone();
    use_future(move || {
        let repo = repo.clone();
        async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                let next = model::load(&repo);
                if *view.peek() != next {
                    view.set(next);
                }
            }
        }
    });
    rsx! {
        ui::Page {
            view: view(),
            on_answer: move |(id, answer): (u32, String)| {
                // The server applies it on the agent's next step; show it now.
                let _ = ndstrl_core::Job::post_answer(&answer_repo, id, &answer);
                let mut v = view.peek().clone();
                if let Some(st) = v.state.as_mut() {
                    if let Some(q) = st.questions.iter_mut().find(|q| q.id == id) {
                        q.answer = Some(answer.clone());
                        q.answered_via = Some("human".into());
                    }
                }
                view.set(v);
            },
            on_steer: move |text: String| {
                let _ = ndstrl_core::Job::post_steer(&steer_repo, &text);
                let mut v = view.peek().clone();
                if let Some(st) = v.state.as_mut() {
                    st.notes.push(ndstrl_core::job::Note { at: String::new(), text, delivered: false });
                }
                view.set(v);
            },
        }
    }
}
