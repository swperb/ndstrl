use studio::billing::BillingProfile;

#[test]
fn billing_save_follows_card_edits() {
    let mut b = BillingProfile::default();
    assert!(!b.save_enabled());
    b.edit_card("4242");
    assert!(b.save_enabled());
}
