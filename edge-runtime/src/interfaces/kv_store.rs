//! `edge:kv-store` — durable key-value persistence.
//!
//! Backed by an in-memory HashMap for the MVP.

use std::sync::RwLock;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

struct KvEntry {
    value: Vec<u8>,
    expires_at: Option<u64>,
}

pub struct KvStore {
    data: RwLock<HashMap<String, KvEntry>>,
}

impl KvStore {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let data = self.data.read().unwrap();
        match data.get(key) {
            Some(entry) => {
                if let Some(expires_at) = entry.expires_at {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    if now > expires_at {
                        drop(data);
                        self.delete(key)?;
                        return Ok(None);
                    }
                }
                Ok(Some(entry.value.clone()))
            }
            None => Ok(None),
        }
    }

    pub fn set(&self, key: String, value: Vec<u8>, ttl_secs: Option<u32>) -> Result<(), String> {
        let expires_at = ttl_secs.map(|s| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + s as u64
        });
        let mut data = self.data.write().unwrap();
        data.insert(key, KvEntry { value, expires_at });
        Ok(())
    }

    pub fn delete(&self, key: &str) -> Result<(), String> {
        let mut data = self.data.write().unwrap();
        data.remove(key);
        Ok(())
    }

    pub fn list_keys(&self, prefix: &str) -> Result<Vec<String>, String> {
        let data = self.data.read().unwrap();
        Ok(data.keys().filter(|k| k.starts_with(prefix)).cloned().collect())
    }
}