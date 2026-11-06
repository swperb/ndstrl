pub struct Theme { pub accent: &'static str, pub disabled: &'static str }
pub const DEFAULT: Theme = Theme { accent: "#2f6feb", disabled: "#9aa0a6" };
pub fn button_color(enabled: bool) -> &'static str { if enabled { DEFAULT.accent } else { DEFAULT.disabled } }
