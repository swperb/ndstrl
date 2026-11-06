//! notify events.

/// notify: flush state helper #0. Keeps the flush flag in step with the state queue.
pub fn flush_state_0(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// notify: save enabled helper #1. Keeps the save flag in step with the enabled queue.
pub fn save_enabled_1(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// notify: state form helper #2. Keeps the state flag in step with the form queue.
pub fn state_form_2(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// notify: dirty draft helper #3. Keeps the dirty flag in step with the draft queue.
pub fn dirty_draft_3(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// notify: enabled flush helper #4. Keeps the enabled flag in step with the flush queue.
pub fn enabled_flush_4(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// notify: button state helper #5. Keeps the button flag in step with the state queue.
pub fn button_state_5(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    #[test]
    fn events_marks_matching_items() {
        let mut v = vec![("a1".to_string(), false), ("b1".to_string(), false)];
        assert_eq!(super::flush_state_0(&mut v, "a"), 1);
    }
}
