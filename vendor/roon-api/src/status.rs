use std::collections::HashMap;
use std::sync::Arc;

use roon_moo::MooMessage;
use roon_moo::connection::{ResponseSender, ServiceHandler};
use tokio::sync::Mutex;

/// Status service — reports extension status in Roon's UI.
///
/// This is a "provided" service: the extension provides it to Roon Core.
/// Call `set_status()` to update the displayed status message.
#[derive(Clone)]
pub struct StatusService {
    inner: Arc<Mutex<StatusInner>>,
}

struct StatusInner {
    message: String,
    is_error: bool,
    subscribers: HashMap<u32, ResponseSender>,
}

impl StatusService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(StatusInner {
                message: String::new(),
                is_error: false,
                subscribers: HashMap::new(),
            })),
        }
    }

    /// Update the status message displayed in Roon's extension settings.
    pub async fn set_status(&self, message: &str, is_error: bool) {
        let mut inner = self.inner.lock().await;
        inner.message = message.to_string();
        inner.is_error = is_error;

        let body = serde_json::json!({
            "message": inner.message,
            "is_error": inner.is_error,
        });

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
    }

    /// Service name for registration.
    pub fn service_name() -> &'static str {
        "com.roonlabs.status:1"
    }

    /// Build the MOO service handler.
    pub fn build_handler(&self) -> ServiceHandler {
        let state = self.clone();
        Arc::new(move |msg: MooMessage, responder: ResponseSender| {
            let method = msg.method().unwrap_or("").to_string();
            let state = state.clone();
            tokio::spawn(async move {
                match method.as_str() {
                    "subscribe_status" => {
                        let inner = state.inner.lock().await;
                        let body = serde_json::json!({
                            "message": inner.message,
                            "is_error": inner.is_error,
                        });
                        let _ = responder.send_continue("Subscribed", Some(body)).await;
                        // Can't hold lock while inserting since we need &mut
                        drop(inner);
                        state
                            .inner
                            .lock()
                            .await
                            .subscribers
                            .insert(msg.request_id, responder);
                    }
                    "unsubscribe_status" => {
                        let sub_key = msg
                            .json_body()
                            .and_then(|b| b["subscription_key"].as_u64())
                            .map(|k| k as u32);
                        if let Some(key) = sub_key {
                            state.inner.lock().await.subscribers.remove(&key);
                        }
                        let _ = responder.send_complete("Unsubscribed", None).await;
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

impl Default for StatusService {
    fn default() -> Self {
        Self::new()
    }
}
