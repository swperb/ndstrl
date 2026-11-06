/// A rendered button. `enabled` comes from the page's state; the UI layer
/// never decides it.
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub label: String,
    pub enabled: bool,
}

pub fn save_button(enabled: bool) -> Button {
    Button { label: "Save".into(), enabled }
}

pub fn cancel_button() -> Button {
    Button { label: "Cancel".into(), enabled: true }
}
