use super::form::SettingsForm;
use super::store::Store;

/// Persist the form. Clears `dirty` on success so the Save button disables
/// again until the next edit.
pub fn save(form: &mut SettingsForm, store: &mut Store) -> Result<(), String> {
    if !form.is_valid() {
        return Err("form is invalid".into());
    }
    form.saving = true;
    store.put("display_name", &form.display_name);
    store.put("email", &form.email);
    store.put("timezone", &form.timezone);
    form.saving = false;
    form.dirty = false;
    Ok(())
}
