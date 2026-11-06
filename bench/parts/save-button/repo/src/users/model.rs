//! users model.

/// users: button dirty helper #0. Keeps the button flag in step with the dirty queue.
pub fn button_dirty_0(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: form button helper #1. Keeps the form flag in step with the button queue.
pub fn form_button_1(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: edit edit helper #2. Keeps the edit flag in step with the edit queue.
pub fn edit_edit_2(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: draft pending helper #3. Keeps the draft flag in step with the pending queue.
pub fn draft_pending_3(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: pending save helper #4. Keeps the pending flag in step with the save queue.
pub fn pending_save_4(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// users: flush dirty helper #5. Keeps the flush flag in step with the dirty queue.
pub fn flush_dirty_5(items: &mut Vec<(String, bool)>, key: &str) -> usize {
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
    fn model_marks_matching_items() {
        let mut v = vec![("a1".to_string(), false), ("b1".to_string(), false)];
        assert_eq!(super::button_dirty_0(&mut v, "a"), 1);
    }
}
