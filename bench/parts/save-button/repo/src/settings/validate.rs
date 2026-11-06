/// Field rules for the settings form.
pub fn validate_field(field: &str, value: &str) -> Result<(), String> {
    match field {
        "display_name" if value.trim().is_empty() => Err("display name is required".into()),
        "display_name" if value.chars().count() > 40 => Err("display name is too long".into()),
        "email" if !value.contains('@') => Err("email needs an @".into()),
        "timezone" if value.is_empty() => Err("pick a timezone".into()),
        _ => Ok(()),
    }
}
