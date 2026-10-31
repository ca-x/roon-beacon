use std::{collections::BTreeMap, net::IpAddr, path::PathBuf, sync::Arc};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::{watch, Mutex};

use crate::persistence::{create_private_directory, private_write};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NetworkSelection {
    pub interface: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NetworkInterface {
    pub name: String,
    pub addresses: Vec<IpAddr>,
}

#[derive(Debug, Serialize)]
pub struct NetworkSettings {
    pub interface: Option<String>,
    pub interfaces: Vec<NetworkInterface>,
}

#[derive(Debug)]
pub struct AudioNetwork {
    selection: watch::Sender<NetworkSelection>,
    saving: Mutex<()>,
    path: PathBuf,
}

impl AudioNetwork {
    pub fn load(data_dir: &std::path::Path) -> Result<Self> {
        let path = data_dir.join("audio-network.json");
        let selection = match std::fs::read(&path) {
            Ok(contents) => {
                serde_json::from_slice(&contents).context("invalid audio network settings")?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                NetworkSelection::default()
            }
            Err(error) => return Err(error).context("reading audio network settings"),
        };
        let (selection, _) = watch::channel(selection);
        Ok(Self {
            selection,
            saving: Mutex::new(()),
            path,
        })
    }

    pub fn subscribe(&self) -> watch::Receiver<NetworkSelection> {
        self.selection.subscribe()
    }

    pub fn settings(&self) -> Result<NetworkSettings> {
        Ok(NetworkSettings {
            interface: self.selection.borrow().interface.clone(),
            interfaces: interfaces()?,
        })
    }

    pub async fn save(self: &Arc<Self>, selection: NetworkSelection) -> Result<()> {
        let network = self.clone();
        tokio::spawn(async move { network.save_serialized(selection).await })
            .await
            .context("saving audio network settings")?
    }

    async fn save_serialized(&self, selection: NetworkSelection) -> Result<()> {
        let _saving = self.saving.lock().await;
        selection.validate()?;
        let path = self.path.clone();
        let contents = serde_json::to_vec_pretty(&selection)?;
        tokio::task::spawn_blocking(move || -> Result<()> {
            if let Some(parent) = path.parent() {
                create_private_directory(parent)?;
            }
            private_write(&path, &contents)
        })
        .await
        .context("saving audio network settings")??;
        self.selection.send_if_modified(|current| {
            if *current == selection {
                return false;
            }
            *current = selection;
            true
        });
        Ok(())
    }
}

impl NetworkSelection {
    pub fn validate(&self) -> Result<()> {
        if let Some(name) = &self.interface {
            if name.chars().any(char::is_control)
                || !interfaces()?.iter().any(|item| item.name == *name)
            {
                bail!("audio network interface is unavailable; choose an available LAN interface or Automatic");
            }
        }
        Ok(())
    }

    pub fn receiver_config(&self, template: &str) -> Result<String> {
        self.validate()?;
        let Some(name) = &self.interface else {
            return Ok(template.to_owned());
        };
        let name = name.replace('\\', "\\\\").replace('"', "\\\"");
        Ok(template.replacen(
            "general = {",
            &format!("general = {{\n    interface = \"{name}\";"),
            1,
        ))
    }
}

pub fn interfaces() -> Result<Vec<NetworkInterface>> {
    let mut interfaces: BTreeMap<String, Vec<IpAddr>> = BTreeMap::new();
    for interface in if_addrs::get_if_addrs().context("enumerating audio network interfaces")? {
        let address = interface.ip();
        if interface.is_loopback() || address.is_unspecified() || address.is_multicast() {
            continue;
        }
        interfaces.entry(interface.name).or_default().push(address);
    }
    Ok(interfaces
        .into_iter()
        .map(|(name, mut addresses)| {
            addresses.sort();
            addresses.dedup();
            NetworkInterface { name, addresses }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn selection_is_private_persistent_and_only_notifies_changes() {
        let directory = tempfile::tempdir().unwrap();
        let network = Arc::new(AudioNetwork::load(directory.path()).unwrap());
        let mut changed = network.subscribe();
        assert_eq!(network.settings().unwrap().interface, None);
        network.save(NetworkSelection::default()).await.unwrap();
        assert!(!changed.has_changed().unwrap());
        let Some(interface) = interfaces().unwrap().into_iter().next() else {
            return;
        };
        let selection = NetworkSelection {
            interface: Some(interface.name),
        };
        network.save(selection.clone()).await.unwrap();
        assert_eq!(*changed.borrow_and_update(), selection);
        assert_eq!(
            AudioNetwork::load(directory.path())
                .unwrap()
                .settings()
                .unwrap()
                .interface,
            selection.interface
        );
        network.save(selection).await.unwrap();
        assert!(!changed.has_changed().unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.path().join("audio-network.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[tokio::test]
    async fn rejected_interface_does_not_replace_saved_selection() {
        let directory = tempfile::tempdir().unwrap();
        let network = Arc::new(AudioNetwork::load(directory.path()).unwrap());
        let changed = network.subscribe();
        assert!(network
            .save(NetworkSelection {
                interface: Some("not-a-real-interface\n\"".to_owned())
            })
            .await
            .is_err());
        assert!(!directory.path().join("audio-network.json").exists());
        assert!(!changed.has_changed().unwrap());
    }

    #[tokio::test]
    async fn persistence_failure_does_not_notify_or_replace_selection() {
        let directory = tempfile::tempdir().unwrap();
        let network = Arc::new(AudioNetwork::load(directory.path()).unwrap());
        std::fs::create_dir(directory.path().join("audio-network.json")).unwrap();
        let changed = network.subscribe();
        assert!(network.save(NetworkSelection::default()).await.is_err());
        assert!(!changed.has_changed().unwrap());
        assert_eq!(network.settings().unwrap().interface, None);
    }

    #[test]
    fn unavailable_saved_interface_can_be_loaded_and_recovered() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("audio-network.json"),
            br#"{"interface":"missing-beacon-interface"}"#,
        )
        .unwrap();
        let network = AudioNetwork::load(directory.path()).unwrap();
        assert_eq!(
            network.settings().unwrap().interface.as_deref(),
            Some("missing-beacon-interface")
        );
        assert!(network.subscribe().borrow().validate().is_err());
        assert_eq!(
            NetworkSelection::default()
                .receiver_config("general = {\n};")
                .unwrap(),
            "general = {\n};"
        );
    }

    #[tokio::test]
    async fn cancelled_request_keeps_persistence_and_watch_publication_together() {
        let Some(interface) = interfaces().unwrap().into_iter().next() else {
            return;
        };
        let directory = tempfile::tempdir().unwrap();
        let network = Arc::new(AudioNetwork::load(directory.path()).unwrap());
        let mut changed = network.subscribe();
        let saving = network.saving.lock().await;
        let request = {
            let network = network.clone();
            let selection = NetworkSelection {
                interface: Some(interface.name.clone()),
            };
            tokio::spawn(async move { network.save(selection).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while Arc::strong_count(&network) < 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        drop(saving);
        tokio::time::timeout(std::time::Duration::from_secs(3), changed.changed())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            changed.borrow_and_update().interface.as_deref(),
            Some(interface.name.as_str())
        );
        assert_eq!(
            AudioNetwork::load(directory.path())
                .unwrap()
                .settings()
                .unwrap()
                .interface,
            network.settings().unwrap().interface
        );
        network.save(NetworkSelection::default()).await.unwrap();
        assert_eq!(
            AudioNetwork::load(directory.path())
                .unwrap()
                .settings()
                .unwrap()
                .interface,
            None
        );
        assert_eq!(network.settings().unwrap().interface, None);
    }
}
