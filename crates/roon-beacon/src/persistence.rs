use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
struct StoredState {
    tokens: HashMap<String, String>,
    selected_core_id: Option<String>,
}

#[derive(Clone)]
pub struct SecureState {
    path: Arc<PathBuf>,
    state: Arc<Mutex<StoredState>>,
}

impl SecureState {
    pub fn open(directory: &Path) -> Result<Self> {
        create_private_directory(directory)?;
        let path = directory.join("roon-state.json");
        let state = match fs::read(&path) {
            Ok(contents) => serde_json::from_slice(&contents)
                .with_context(|| format!("invalid Roon authorization state: {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => StoredState::default(),
            Err(error) => return Err(error).context("cannot read Roon authorization state"),
        };
        Ok(Self {
            path: Arc::new(path),
            state: Arc::new(Mutex::new(state)),
        })
    }

    pub fn selected_core_id(&self) -> Option<String> {
        self.state.lock().unwrap().selected_core_id.clone()
    }

    pub fn select_core(&self, core_id: &str) -> Result<()> {
        self.update(|state| state.selected_core_id = Some(core_id.to_string()))
    }

    pub fn token(&self, core_id: &str) -> Option<String> {
        self.state.lock().unwrap().tokens.get(core_id).cloned()
    }

    pub fn save_token(&self, core_id: &str, token: &str) -> Result<()> {
        self.update(|state| {
            state.tokens.insert(core_id.to_string(), token.to_string());
        })
    }

    fn update(&self, update: impl FnOnce(&mut StoredState)) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        let mut next = state.clone();
        update(&mut next);
        private_write(&self.path, &serde_json::to_vec_pretty(&next)?)?;
        *state = next;
        Ok(())
    }
}

pub fn create_private_directory(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path).context("cannot create data directory")
}

pub fn private_write(path: &Path, contents: &[u8]) -> Result<()> {
    let candidate = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&candidate)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&candidate, path)?;
        #[cfg(unix)]
        if let Some(parent) = path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&candidate);
    }
    result.context("cannot atomically persist private state")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_and_selection_survive_restart() {
        let directory = tempfile::tempdir().unwrap();
        let store = SecureState::open(directory.path()).unwrap();
        store.save_token("core-a", "private-token").unwrap();
        store.select_core("core-a").unwrap();
        store.save_token("core-b", "another-token").unwrap();
        let reopened = SecureState::open(directory.path()).unwrap();
        assert_eq!(reopened.token("core-a").as_deref(), Some("private-token"));
        assert_eq!(reopened.selected_core_id().as_deref(), Some("core-a"));
        assert_eq!(reopened.token("core-b").as_deref(), Some("another-token"));
    }

    #[test]
    fn corrupt_credentials_fail_instead_of_silently_resetting() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("roon-state.json"), "not json").unwrap();
        assert!(SecureState::open(directory.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn authorization_file_is_private_after_each_update() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let store = SecureState::open(directory.path()).unwrap();
        for token in ["one", "two"] {
            store.save_token("core", token).unwrap();
            let permissions = fs::metadata(directory.path().join("roon-state.json"))
                .unwrap()
                .permissions();
            assert_eq!(permissions.mode() & 0o777, 0o600);
        }
    }
}
