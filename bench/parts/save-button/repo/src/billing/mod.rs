/// Billing profile. Changes here go through finance review; do not edit.
#[derive(Debug, Default)]
pub struct BillingProfile { pub card_last4: String, pub dirty: bool }

impl BillingProfile {
    pub fn edit_card(&mut self, last4: &str) { self.card_last4 = last4.into(); self.dirty = true; }
    pub fn save_enabled(&self) -> bool { self.dirty }
}
