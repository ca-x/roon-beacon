use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// Trait for persisting SDK state (tokens, pairing, config).
///
/// Implement this trait to provide custom storage backends. The SDK
/// ships with `FileStateStore` (JSON file) and `MemoryStateStore` (in-memory).
///
/// The legacy `TokenStore` trait is still supported as a subset.
pub trait StateStore: Send + Sync + 'static {
    fn load_token(&self, core_id: &str) -> Option<String>;
    fn save_token(&self, core_id: &str, token: &str) -> Result<(), String>;
    fn load_paired_core_id(&self) -> Option<String>;
    fn save_paired_core_id(&self, core_id: Option<&str>) -> Result<(), String>;
}

/// Legacy alias for `StateStore`.
pub trait TokenStore: StateStore {}
impl<T: StateStore> TokenStore for T {}

/// In-memory state store for testing.
#[derive(Debug, Default)]
pub struct MemoryStateStore {
    state: Mutex<PersistedState>,
}

/// Persisted state structure (serialized to JSON).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default)]
    pub tokens: HashMap<String, String>,
    #[serde(default)]
    pub paired_core_id: Option<String>,
}

impl MemoryStateStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StateStore for MemoryStateStore {
    fn load_token(&self, core_id: &str) -> Option<String> {
        self.state.lock().unwrap().tokens.get(core_id).cloned()
    }

    fn save_token(&self, core_id: &str, token: &str) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .tokens
            .insert(core_id.to_string(), token.to_string());
        Ok(())
    }

    fn load_paired_core_id(&self) -> Option<String> {
        self.state.lock().unwrap().paired_core_id.clone()
    }

    fn save_paired_core_id(&self, core_id: Option<&str>) -> Result<(), String> {
        self.state.lock().unwrap().paired_core_id = core_id.map(|s| s.to_string());
        Ok(())
    }
}

/// File-based state store that persists tokens and pairing state as JSON.
#[derive(Debug)]
pub struct FileStateStore {
    path: PathBuf,
}

impl FileStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn read_state(&self) -> PersistedState {
        match std::fs::read_to_string(&self.path) {
            Ok(content) => {
                // Detect format: new format has a "tokens" key, legacy is a flat map
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                    if value.get("tokens").is_some() {
                        // New format
                        if let Ok(state) = serde_json::from_value::<PersistedState>(value) {
                            return state;
                        }
                    } else if let Ok(tokens) =
                        serde_json::from_value::<HashMap<String, String>>(value)
                    {
                        // Legacy format (flat token map)
                        return PersistedState {
                            tokens,
                            paired_core_id: None,
                        };
                    }
                }
                PersistedState::default()
            }
            Err(_) => PersistedState::default(),
        }
    }

    fn write_state(&self, state: &PersistedState) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create state directory: {}", e))?;
        }
        let content =
            serde_json::to_string_pretty(state).map_err(|e| format!("JSON error: {}", e))?;
        std::fs::write(&self.path, content)
            .map_err(|e| format!("failed to write state file: {}", e))
    }
}

impl StateStore for FileStateStore {
    fn load_token(&self, core_id: &str) -> Option<String> {
        self.read_state().tokens.get(core_id).cloned()
    }

    fn save_token(&self, core_id: &str, token: &str) -> Result<(), String> {
        let mut state = self.read_state();
        state.tokens.insert(core_id.to_string(), token.to_string());
        self.write_state(&state)
    }

    fn load_paired_core_id(&self) -> Option<String> {
        self.read_state().paired_core_id
    }

    fn save_paired_core_id(&self, core_id: Option<&str>) -> Result<(), String> {
        let mut state = self.read_state();
        state.paired_core_id = core_id.map(|s| s.to_string());
        self.write_state(&state)
    }
}

// Legacy aliases for backward compatibility
pub type MemoryTokenStore = MemoryStateStore;
pub type FileTokenStore = FileStateStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_store_roundtrip() {
        let store = MemoryStateStore::new();
        assert!(store.load_token("core-1").is_none());

        store.save_token("core-1", "token-abc").unwrap();
        assert_eq!(store.load_token("core-1").unwrap(), "token-abc");

        store.save_token("core-1", "token-xyz").unwrap();
        assert_eq!(store.load_token("core-1").unwrap(), "token-xyz");
    }

    #[test]
    fn test_memory_store_pairing() {
        let store = MemoryStateStore::new();
        assert!(store.load_paired_core_id().is_none());

        store.save_paired_core_id(Some("core-1")).unwrap();
        assert_eq!(store.load_paired_core_id().unwrap(), "core-1");

        store.save_paired_core_id(None).unwrap();
        assert!(store.load_paired_core_id().is_none());
    }

    #[test]
    fn test_file_store_roundtrip() {
        let dir = std::env::temp_dir().join("roon-api-test-state");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("state.json");

        let store = FileStateStore::new(&path);
        assert!(store.load_token("core-1").is_none());
        assert!(store.load_paired_core_id().is_none());

        store.save_token("core-1", "token-abc").unwrap();
        store.save_paired_core_id(Some("core-1")).unwrap();

        // Verify persistence across instances
        let store2 = FileStateStore::new(&path);
        assert_eq!(store2.load_token("core-1").unwrap(), "token-abc");
        assert_eq!(store2.load_paired_core_id().unwrap(), "core-1");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_file_store_legacy_migration() {
        let dir = std::env::temp_dir().join("roon-api-test-legacy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.json");

        // Write legacy format (flat token map)
        let legacy = serde_json::json!({"core-1": "old-token"});
        std::fs::write(&path, serde_json::to_string(&legacy).unwrap()).unwrap();

        let store = FileStateStore::new(&path);
        assert_eq!(store.load_token("core-1").unwrap(), "old-token");
        assert!(store.load_paired_core_id().is_none());

        // Save new data — migrates to new format
        store.save_paired_core_id(Some("core-1")).unwrap();
        let store2 = FileStateStore::new(&path);
        assert_eq!(store2.load_token("core-1").unwrap(), "old-token");
        assert_eq!(store2.load_paired_core_id().unwrap(), "core-1");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
