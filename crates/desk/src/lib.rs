//! NDSTRL front office: another client of the factory's files.

pub mod model;
pub mod ui;

use dioxus::prelude::*;

/// Root used for static snapshots: same Page, no live data, no clicks.
#[component]
pub fn SnapshotRoot(view: model::View, tab: ui::Tab) -> Element {
    rsx! { ui::Page { view, tab, on_tab: move |_| {} } }
}

/// Render a full HTML document for a view, for UI tests and screenshots.
pub fn snapshot_html(view: model::View, tab: ui::Tab, theme: Option<&str>) -> String {
    let mut vdom = VirtualDom::new_with_props(SnapshotRoot, SnapshotRootProps { view, tab });
    vdom.rebuild_in_place();
    let body = dioxus_ssr::render(&vdom);
    let theme_attr = theme.map(|t| format!(" data-theme=\"{t}\"")).unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"{theme_attr}><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>NDSTRL</title><style>{}</style></head><body>{body}</body></html>",
        ui::CSS
    )
}
