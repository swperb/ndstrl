use super::button::{cancel_button, save_button, Button};
use crate::settings::SettingsForm;

/// The settings page footer: Cancel and Save.
pub fn settings_footer(form: &SettingsForm) -> Vec<Button> {
    vec![cancel_button(), save_button(form.save_enabled())]
}
