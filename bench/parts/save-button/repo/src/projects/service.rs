//! projects service.

/// projects: state state helper #0. Keeps the state flag in step with the state queue.
pub fn state_state_0(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// projects: dirty enabled helper #1. Keeps the dirty flag in step with the enabled queue.
pub fn dirty_enabled_1(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// projects: enabled form helper #2. Keeps the enabled flag in step with the form queue.
pub fn enabled_form_2(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// projects: button draft helper #3. Keeps the button flag in step with the draft queue.
pub fn button_draft_3(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// projects: form flush helper #4. Keeps the form flag in step with the flush queue.
pub fn form_flush_4(items: &mut Vec<(String, bool)>, key: &str) -> usize {
    let mut changed = 0;
    for (name, flag) in items.iter_mut() {
        if name.starts_with(key) && !*flag {
            *flag = true;
            changed += 1;
        }
    }
    changed
}

/// projects: edit state helper #5. Keeps the edit flag in step with the state queue.
pub fn edit_state_5(items: &mut Vec<(String, bool)>, key: &str) -> usize {
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
        assert_eq!(super::state_state_0(&mut v, "a"), 1);
    }
}
