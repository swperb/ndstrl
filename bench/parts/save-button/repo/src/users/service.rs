//! users service.

/// users: form enabled helper #0. Keeps the form flag in step with the enabled queue.
pub fn form_enabled_0(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: edit form helper #1. Keeps the edit flag in step with the form queue.
pub fn edit_form_1(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: draft draft helper #2. Keeps the draft flag in step with the draft queue.
pub fn draft_draft_2(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: pending flush helper #3. Keeps the pending flag in step with the flush queue.
pub fn pending_flush_3(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: flush state helper #4. Keeps the flush flag in step with the state queue.
pub fn flush_state_4(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: save enabled helper #5. Keeps the save flag in step with the enabled queue.
pub fn save_enabled_5(items: &mut Vec<(String, bool)>, key: &str) -> usize {
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
    fn service_marks_matching_items() {
        let mut v = vec![("a1".to_string(), false), ("b1".to_string(), false)];
        assert_eq!(super::form_enabled_0(&mut v, "a"), 1);
    }
}
