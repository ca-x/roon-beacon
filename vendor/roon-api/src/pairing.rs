use std::collections::HashMap;
use std::sync::Arc;

use roon_moo::MooMessage;
use roon_moo::connection::{ResponseSender, ServiceHandler};
use tokio::sync::Mutex;

use crate::token::StateStore;

type PairChangeFn = Box<dyn Fn(Option<String>, Option<String>) + Send + Sync>;

/// Shared pairing state tracking which core (if any) is the paired core.
#[derive(Clone)]
pub struct PairingState {
    inner: Arc<Mutex<PairingInner>>,
}

struct PairingInner {
    paired_core_id: Option<String>,
    /// Active subscribe_pairing subscribers: map of request_id → ResponseSender.
    subscribers: HashMap<u32, ResponseSender>,
    /// Callback invoked when pair changes — sends (old_core_id, new_core_id).
    on_pair_change: Option<PairChangeFn>,
}

impl PairingState {
    /// Create a new PairingState, restoring paired_core_id from the store.
    pub fn new(store: &dyn StateStore) -> Self {
        let paired_core_id = store.load_paired_core_id();
        Self {
            inner: Arc::new(Mutex::new(PairingInner {
                paired_core_id,
                subscribers: HashMap::new(),
                on_pair_change: None,
            })),
        }
    }

    /// Register a callback for pairing changes.
    pub async fn on_pair_change(
        &self,
        cb: impl Fn(Option<String>, Option<String>) + Send + Sync + 'static,
    ) {
        self.inner.lock().await.on_pair_change = Some(Box::new(cb));
    }

    /// Get the current paired core ID.
    pub async fn paired_core_id(&self) -> Option<String> {
        self.inner.lock().await.paired_core_id.clone()
    }

    /// Handle a `pair` request — switch to the new core and notify subscribers.
    pub async fn pair_with(&self, new_core_id: &str, store: &dyn StateStore) {
        let mut inner = self.inner.lock().await;
        let old_core_id = inner.paired_core_id.clone();
        inner.paired_core_id = Some(new_core_id.to_string());

        // Persist
        if let Err(e) = store.save_paired_core_id(Some(new_core_id)) {
            tracing::warn!("Failed to persist paired_core_id: {}", e);
        }

        // Notify subscribers
        let body = serde_json::json!({"paired_core_id": new_core_id});
        let mut dead = Vec::new();
        for (&req_id, sender) in &inner.subscribers {
            if sender
                .send_continue("Changed", Some(body.clone()))
                .await
                .is_err()
            {
                dead.push(req_id);
            }
        }
        for req_id in dead {
            inner.subscribers.remove(&req_id);
        }

        // Fire callback
        if let Some(ref cb) = inner.on_pair_change {
            cb(old_core_id, Some(new_core_id.to_string()));
        }
    }

    /// Build service handlers for the pairing service.
    pub fn build_handler(self, store: Arc<dyn StateStore>) -> ServiceHandler {
        Arc::new(move |msg: MooMessage, responder: ResponseSender| {
            let method = msg.method().unwrap_or("").to_string();
            let state = self.clone();
            let store = store.clone();
            tokio::spawn(async move {
                match method.as_str() {
                    "get_pairing" => {
                        let paired = state.paired_core_id().await;
                        let body = match paired {
                            Some(id) => serde_json::json!({"paired_core_id": id}),
                            None => serde_json::json!({}),
                        };
                        let _ = responder.send_complete("Success", Some(body)).await;
                    }
                    "pair" => {
                        // The requesting core becomes the paired core.
                        // We need the core_id from the connection context — for now,
                        // we extract it from the body if provided, or use the existing logic.
                        if let Some(body) = msg.json_body()
                            && let Some(core_id) = body["core_id"].as_str()
                        {
                            state.pair_with(core_id, &*store).await;
                        }
                        let _ = responder.send_complete("Success", None).await;
                    }
                    "subscribe_pairing" => {
                        let paired = state.paired_core_id().await;
                        let body = match paired {
                            Some(id) => serde_json::json!({"paired_core_id": id}),
                            None => serde_json::json!({}),
                        };
                        let _ = responder.send_continue("Subscribed", Some(body)).await;
                        // Register subscriber
                        state
                            .inner
                            .lock()
                            .await
                            .subscribers
                            .insert(msg.request_id, responder);
                    }
                    _ => {
                        let _ = responder
                            .send_complete(
                                "InvalidRequest",
                                Some(serde_json::json!({"error": format!("unknown method: {}", method)})),
                            )
                            .await;
                    }
                }
            });
        })
    }
}
