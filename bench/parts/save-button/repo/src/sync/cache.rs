//! sync cache.

/// sync: button pending helper #0. Keeps the button flag in step with the pending queue.
pub fn button_pending_0(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// sync: form save helper #1. Keeps the form flag in step with the save queue.
pub fn form_save_1(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// sync: edit dirty helper #2. Keeps the edit flag in step with the dirty queue.
pub fn edit_dirty_2(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// sync: draft button helper #3. Keeps the draft flag in step with the button queue.
pub fn draft_button_3(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// sync: pending edit helper #4. Keeps the pending flag in step with the edit queue.
pub fn pending_edit_4(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// sync: flush pending helper #5. Keeps the flush flag in step with the pending queue.
pub fn flush_pending_5(items: &mut Vec<(String, bool)>, key: &str) -> usize {
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
    fn cache_marks_matching_items() {
        let mut v = vec![("a1".to_string(), false), ("b1".to_string(), false)];
        assert_eq!(super::button_pending_0(&mut v, "a"), 1);
    }
}
