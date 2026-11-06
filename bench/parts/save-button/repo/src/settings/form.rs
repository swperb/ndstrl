use super::validate::validate_field;

/// The settings page form. `dirty` tracks unsaved edits; the Save button
/// is enabled only while the form is dirty and valid.
#[derive(Debug, Default, Clone)]
pub struct SettingsForm {
    pub display_name: String,
    pub email: String,
    pub timezone: String,
    pub dirty: bool,
    pub saving: bool,
}

impl SettingsForm {
    pub fn load(display_name: &str, email: &str, timezone: &str) -> Self {
        Self {
            display_name: display_name.into(),
            email: email.into(),
            timezone: timezone.into(),
            dirty: false,
            saving: false,
        }
    }

    /// Apply a user edit to one field.
    pub fn edit(&mut self, field: &str, value: &str) -> Result<(), String> {
        validate_field(field, value)?;
        let slot = match field {
            "display_name" => &mut self.display_name,
            "email" => &mut self.email,
            "timezone" => &mut self.timezone,
            other => return Err(format!("unknown field {other}")),
        };
        if slot.as_str() != value {
            *slot = value.to_string();
        }
        Ok(())
    }

    /// Whether the Save button should be enabled.
    pub fn save_enabled(&self) -> bool {
        self.dirty && !self.saving && self.is_valid()
    }

    pub fn is_valid(&self) -> bool {
        ["display_name", "email", "timezone"]
            .iter()
            .zip([&self.display_name, &self.email, &self.timezone])
            .all(|(f, v)| validate_field(f, v).is_ok())
    }
}
