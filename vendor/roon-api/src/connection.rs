use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, watch};

use crate::core::Core;
use crate::event::RoonEvent;
use crate::pairing::PairingState;
use crate::registry::{self, ExtensionInfo};
use crate::token::StateStore;

const BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// Connection lifecycle state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Discovering,
    Connecting,
    Registering,
    Connected,
    Reconnecting { attempt: u32 },
}

impl fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => write!(f, "Disconnected"),
            Self::Discovering => write!(f, "Discovering"),
            Self::Connecting => write!(f, "Connecting"),
            Self::Registering => write!(f, "Registering"),
            Self::Connected => write!(f, "Connected"),
            Self::Reconnecting { attempt } => write!(f, "Reconnecting (attempt {})", attempt),
        }
    }
}

/// Manages the connection lifecycle to a Roon Core with explicit state transitions.
pub struct ConnectionManager {
    state_tx: watch::Sender<ConnectionState>,
    state_rx: watch::Receiver<ConnectionState>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        Self { state_tx, state_rx }
    }

    /// Subscribe to state changes.
    pub fn watch_state(&self) -> watch::Receiver<ConnectionState> {
        self.state_rx.clone()
    }

    /// Current state.
    pub fn state(&self) -> ConnectionState {
        self.state_rx.borrow().clone()
    }

    fn set_state(&self, state: ConnectionState) {
        tracing::info!("Connection state: {}", state);
        let _ = self.state_tx.send(state);
    }

    /// Mark as connected (for initial connection established outside the manager).
    pub fn set_connected(&self) {
        self.set_state(ConnectionState::Connected);
    }

    /// Run the direct-connection lifecycle: connect → monitor → reconnect loop.
    pub(crate) async fn run_direct(
        &self,
        url: String,
        info: ExtensionInfo,
        store: Arc<dyn StateStore>,
        pairing: PairingState,
        event_tx: broadcast::Sender<RoonEvent>,
    ) {
        let mut backoff = BACKOFF_INITIAL;
        let mut attempt = 1u32;

        loop {
            self.set_state(ConnectionState::Reconnecting { attempt });
            tokio::time::sleep(backoff).await;

            self.set_state(ConnectionState::Connecting);
            self.set_state(ConnectionState::Registering);

            match registry::perform_handshake(&url, &info, &store, &pairing).await {
                Ok(core) => {
                    self.set_state(ConnectionState::Connected);
                    let _ = event_tx.send(RoonEvent::CorePaired(core.clone()));

                    wait_disconnect(&core).await;

                    let _ = event_tx.send(RoonEvent::CoreLost {
                        core_id: core.core_id().to_string(),
                    });
                    self.set_state(ConnectionState::Disconnected);
                    backoff = BACKOFF_INITIAL;
                    attempt = 1;
                }
                Err(e) => {
                    tracing::warn!("Reconnection attempt {} failed: {}", attempt, e);
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                    attempt += 1;
                }
            }
        }
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

async fn wait_disconnect(core: &Core) {
    loop {
        tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;
        if !core.is_alive() {
            break;
        }
    }
}
