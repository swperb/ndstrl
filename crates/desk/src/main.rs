//! `ndstrl-desk [repo]`: the front office window. Watches the repo's
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
                .with_inner_size(LogicalSize::new(1320.0, 880.0))
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
    let mut tab = use_signal(|| ui::Tab::Floor);
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
    rsx! { ui::Page { view: view(), tab: tab(), on_tab: move |t| tab.set(t) } }
}
