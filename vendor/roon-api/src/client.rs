use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::{Mutex, broadcast};

use crate::connection::ConnectionManager;
use crate::core::Core;
use crate::error::ApiError;
use crate::event::RoonEvent;
use crate::pairing::PairingState;
use crate::registry::{self, ExtensionInfo};
use crate::token::{MemoryStateStore, StateStore};

/// Builder for constructing a `RoonClient`.
pub struct RoonClientBuilder {
    extension_id: String,
    display_name: String,
    display_version: String,
    publisher: String,
    email: String,
    website: Option<String>,
    token_store: Option<Arc<dyn StateStore>>,
    required_services: Vec<String>,
    optional_services: Vec<String>,
    provided_services: Vec<String>,
}

impl RoonClientBuilder {
    /// Create a new builder with the required extension metadata.
    pub fn new(
        extension_id: &str,
        display_name: &str,
        display_version: &str,
        publisher: &str,
        email: &str,
    ) -> Self {
        Self {
            extension_id: extension_id.to_string(),
            display_name: display_name.to_string(),
            display_version: display_version.to_string(),
            publisher: publisher.to_string(),
            email: email.to_string(),
            website: None,
            token_store: None,
            required_services: Vec::new(),
            optional_services: Vec::new(),
            provided_services: Vec::new(),
        }
    }

    pub fn website(mut self, url: &str) -> Self {
        self.website = Some(url.to_string());
        self
    }

    pub fn token_store(mut self, store: impl StateStore) -> Self {
        self.token_store = Some(Arc::new(store));
        self
    }

    pub fn require_transport(mut self) -> Self {
        self.required_services
            .push("com.roonlabs.transport:2".to_string());
        self
    }

    pub fn require_browse(mut self) -> Self {
        self.required_services
            .push("com.roonlabs.browse:1".to_string());
        self
    }

    pub fn optional_transport(mut self) -> Self {
        self.optional_services
            .push("com.roonlabs.transport:2".to_string());
        self
    }

    pub fn optional_browse(mut self) -> Self {
        self.optional_services
            .push("com.roonlabs.browse:1".to_string());
        self
    }

    pub fn provide_service(mut self, service_name: &str) -> Self {
        self.provided_services.push(service_name.to_string());
        self
    }

    pub fn build(self) -> Result<RoonClient, ApiError> {
        let store: Arc<dyn StateStore> = self
            .token_store
            .unwrap_or_else(|| Arc::new(MemoryStateStore::new()));

        let pairing = PairingState::new(&*store);
        let (event_tx, _) = broadcast::channel::<RoonEvent>(32);
        let conn_manager = ConnectionManager::new();

        Ok(RoonClient {
            info: ExtensionInfo {
                extension_id: self.extension_id,
                display_name: self.display_name,
                display_version: self.display_version,
                publisher: self.publisher,
                email: self.email,
                website: self.website,
                required_services: self.required_services,
                optional_services: self.optional_services,
                provided_services: self.provided_services,
            },
            store,
            pairing,
            event_tx,
            conn_manager,
        })
    }
}

/// The main Roon SDK client.
pub struct RoonClient {
    info: ExtensionInfo,
    store: Arc<dyn StateStore>,
    pairing: PairingState,
    event_tx: broadcast::Sender<RoonEvent>,
    conn_manager: ConnectionManager,
}

impl RoonClient {
    pub fn attach_registered(
        &self,
        connection: Arc<roon_moo::connection::MooConnection>,
        core_id: &str,
        display_name: &str,
        host: &str,
        http_port: u16,
    ) -> Core {
        let core = Core {
            inner: Arc::new(crate::core::CoreInner {
                core_id: core_id.to_string(),
                display_name: display_name.to_string(),
                display_version: String::new(),
                token: None,
                http_port,
                host: host.to_string(),
                connection,
            }),
        };
        let _ = self.event_tx.send(RoonEvent::CorePaired(core.clone()));
        core
    }

    pub fn events(&self) -> broadcast::Receiver<RoonEvent> {
        self.event_tx.subscribe()
    }

    pub fn pairing(&self) -> &PairingState {
        &self.pairing
    }

    /// Watch connection state transitions.
    pub fn watch_connection_state(
        &self,
    ) -> tokio::sync::watch::Receiver<crate::connection::ConnectionState> {
        self.conn_manager.watch_state()
    }

    /// Start SOOD-based discovery with auto-connect and reconnection.
    pub async fn start_discovery(&self) -> Result<(), ApiError> {
        let (discovery, mut core_rx) = roon_sood::SoodDiscovery::start().await?;

        let info = self.info.clone();
        let event_tx = self.event_tx.clone();
        let store = self.store.clone();
        let pairing = self.pairing.clone();

        let event_tx_pair = event_tx.clone();
        pairing
            .on_pair_change(move |old, _new| {
                if let Some(old_id) = old {
                    let _ = event_tx_pair.send(RoonEvent::CoreUnpaired { core_id: old_id });
                }
            })
            .await;

        tokio::spawn(async move {
            let connected_cores: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

            while let Ok(discovered) = core_rx.recv().await {
                {
                    let cores = connected_cores.lock().await;
                    if cores.contains(&discovered.core_id) {
                        continue;
                    }
                }

                let url = format!("ws://{}:{}/api", discovered.host, discovered.http_port);

                let _ = event_tx.send(RoonEvent::CoreFound {
                    core_id: discovered.core_id.clone(),
                    display_name: String::new(),
                });

                match registry::perform_handshake(&url, &info, &store, &pairing).await {
                    Ok(core) => {
                        let core_id = discovered.core_id.clone();
                        connected_cores.lock().await.insert(core_id.clone());

                        if pairing.paired_core_id().await.is_none() {
                            pairing.pair_with(&core_id, &*store).await;
                        }

                        let cores_ref = connected_cores.clone();
                        let event_tx_ref = event_tx.clone();
                        let core_ref = core.clone();
                        tokio::spawn(async move {
                            wait_disconnect(&core_ref).await;
                            cores_ref.lock().await.remove(&core_id);
                            let _ = event_tx_ref.send(RoonEvent::CoreLost {
                                core_id: core_id.clone(),
                            });
                            tracing::info!("Core {} disconnected", core_id);
                        });

                        let _ = event_tx.send(RoonEvent::CorePaired(core));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to register with core at {}: {}", url, e);
                    }
                }
            }
            discovery.stop().await;
        });

        Ok(())
    }

    /// Connect directly to a known Roon Core with automatic reconnection.
    ///
    /// Uses a ConnectionManager state machine:
    /// Disconnected → Connecting → Registering → Connected → (disconnect) → Reconnecting
    pub async fn connect(&self, host: &str, port: u16) -> Result<Core, ApiError> {
        let url = format!("ws://{}:{}/api", host, port);

        // Initial connection
        let core =
            registry::perform_handshake(&url, &self.info, &self.store, &self.pairing).await?;

        if self.pairing.paired_core_id().await.is_none() {
            self.pairing.pair_with(core.core_id(), &*self.store).await;
        }

        let _ = self.event_tx.send(RoonEvent::CorePaired(core.clone()));

        // Spawn ConnectionManager for lifecycle management
        let info = self.info.clone();
        let store = self.store.clone();
        let pairing = self.pairing.clone();
        let event_tx = self.event_tx.clone();
        let initial_core = core.clone();

        let manager = ConnectionManager::new();
        manager.set_connected();

        tokio::spawn(async move {
            // Monitor the initial connection first
            wait_disconnect(&initial_core).await;
            let core_id = initial_core.core_id().to_string();
            let _ = event_tx.send(RoonEvent::CoreLost {
                core_id: core_id.clone(),
            });

            // Run reconnection lifecycle
            manager
                .run_direct(url, info, store, pairing, event_tx)
                .await;
        });

        Ok(core)
    }

    /// Connect using a one-time token (skips discovery and info request).
    pub async fn connect_with_token(
        &self,
        host: &str,
        port: u16,
        token: &str,
    ) -> Result<Core, ApiError> {
        let url = format!("ws://{}:{}/api", host, port);
        let core = registry::perform_handshake_with_token(
            &url,
            &self.info,
            token,
            &self.store,
            &self.pairing,
        )
        .await?;
        let _ = self.event_tx.send(RoonEvent::CorePaired(core.clone()));
        Ok(core)
    }
}

async fn wait_disconnect(core: &Core) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if !core.is_alive() {
            break;
        }
    }
}
