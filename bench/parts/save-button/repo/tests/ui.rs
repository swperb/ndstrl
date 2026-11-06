use studio::ui::button::save_button;
use studio::ui::theme::button_color;

#[test]
fn disabled_save_is_grey() {
    let b = save_button(false);
    assert!(!b.enabled);
    assert_eq!(button_color(b.enabled), "#9aa0a6");
}
