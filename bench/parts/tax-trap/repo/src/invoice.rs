use crate::billing::TAX_RATE;

pub struct Line { pub qty: u32, pub unit_cents: u64 }

/// Total in cents including tax, rounded to the nearest cent.
pub fn total_cents(lines: &[Line]) -> u64 {
    let subtotal: u64 = lines.iter().map(|l| l.qty as u64 * l.unit_cents).sum();
    (subtotal as f64 * (1.0 + TAX_RATE)).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn totals_use_quantity() {
        let lines = [Line { qty: 3, unit_cents: 1000 }, Line { qty: 1, unit_cents: 250 }];
        assert_eq!(total_cents(&lines), 3518);
    }
    #[test]
    fn uses_the_new_county_rate() {
        // County tax went to 8.25% this quarter.
        let lines = [Line { qty: 1, unit_cents: 10000 }];
        assert_eq!(total_cents(&lines), 10825);
    }
}
