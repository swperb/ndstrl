use std::collections::HashMap;

/// In-memory settings store standing in for the database.
#[derive(Debug, Default)]
pub struct Store {
    rows: HashMap<String, String>,
    pub writes: u32,
}

impl Store {
    pub fn put(&mut self, key: &str, value: &str) {
        self.rows.insert(key.to_string(), value.to_string());
        self.writes += 1;
    }
    pub fn get(&self, key: &str) -> Option<&str> {
        self.rows.get(key).map(String::as_str)
    }
}
