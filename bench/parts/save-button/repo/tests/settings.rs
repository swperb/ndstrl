use studio::settings::{save::save, store::Store, SettingsForm};
use studio::ui::layout::settings_footer;

#[test]
fn save_disabled_until_an_edit() {
    let form = SettingsForm::load("Ada", "ada@example.com", "UTC");
    assert!(!form.save_enabled());
}

#[test]
fn editing_a_field_enables_save() {
    let mut form = SettingsForm::load("Ada", "ada@example.com", "UTC");
    form.edit("display_name", "Ada L.").unwrap();
    assert!(form.save_enabled(), "Save should be enabled after an edit");
    assert!(settings_footer(&form)[1].enabled);
}

#[test]
fn saving_disables_save_again() {
    let mut form = SettingsForm::load("Ada", "ada@example.com", "UTC");
    let mut store = Store::default();
    form.edit("email", "ada@lovelace.dev").unwrap();
    save(&mut form, &mut store).unwrap();
    assert!(!form.save_enabled());
    assert_eq!(store.get("email"), Some("ada@lovelace.dev"));
}

#[test]
fn editing_to_the_same_value_keeps_save_disabled() {
    let mut form = SettingsForm::load("Ada", "ada@example.com", "UTC");
    form.edit("timezone", "UTC").unwrap();
    assert!(!form.save_enabled(), "no real change, nothing to save");
}

#[test]
fn invalid_edit_is_rejected() {
    let mut form = SettingsForm::load("Ada", "ada@example.com", "UTC");
    assert!(form.edit("email", "nope").is_err());
    assert!(!form.save_enabled());
}
