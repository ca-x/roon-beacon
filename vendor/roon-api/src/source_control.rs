use std::sync::Arc;

use roon_moo::MooMessage;
use roon_moo::connection::{ResponseSender, ServiceHandler};
use tokio::sync::Mutex;

/// Callback for source control requests from Roon Core.
pub type SourceCallback = Arc<dyn Fn(SourceRequest) + Send + Sync>;

/// A request from Roon Core to change source state.
#[derive(Debug, Clone)]
pub enum SourceRequest {
    Standby { control_key: String },
    ConvenienceSwitch { control_key: String },
}

/// Source Control service — allows Roon to switch inputs on external devices.
///
/// Provided by the extension to Roon Core.
#[derive(Clone)]
pub struct SourceControlService {
    inner: Arc<Mutex<SourceInner>>,
}

struct SourceInner {
    controls: Vec<SourceControlDef>,
    callback: Option<SourceCallback>,
}

/// Definition of a source control endpoint.
#[derive(Debug, Clone)]
pub struct SourceControlDef {
    pub control_key: String,
    pub display_name: String,
    pub supports_standby: bool,
    pub status: String,
}

impl SourceControlService {
    pub fn new(controls: Vec<SourceControlDef>, callback: SourceCallback) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SourceInner {
                controls,
                callback: Some(callback),
            })),
        }
    }

    /// Update the status of a source control.
    pub async fn update_status(&self, control_key: &str, status: &str) {
        let mut inner = self.inner.lock().await;
        for control in &mut inner.controls {
            if control.control_key == control_key {
                control.status = status.to_string();
            }
        }
    }

    pub fn service_name() -> &'static str {
        "com.roonlabs.source_control:1"
    }

    pub fn build_handler(&self) -> ServiceHandler {
        let state = self.clone();
        Arc::new(move |msg: MooMessage, responder: ResponseSender| {
            let method = msg.method().unwrap_or("").to_string();
            let state = state.clone();
            tokio::spawn(async move {
                match method.as_str() {
                    "standby" => {
                        let control_key = msg
                            .json_body()
                            .and_then(|b| b["control_key"].as_str())
                            .unwrap_or("")
                            .to_string();
                        let inner = state.inner.lock().await;
                        if let Some(ref cb) = inner.callback {
                            cb(SourceRequest::Standby { control_key });
                        }
                        let _ = responder.send_complete("Success", None).await;
                    }
                    "convenience_switch" => {
                        let control_key = msg
                            .json_body()
                            .and_then(|b| b["control_key"].as_str())
                            .unwrap_or("")
                            .to_string();
                        let inner = state.inner.lock().await;
                        if let Some(ref cb) = inner.callback {
                            cb(SourceRequest::ConvenienceSwitch { control_key });
                        }
                        let _ = responder.send_complete("Success", None).await;
                    }
                    "get_all" => {
                        let inner = state.inner.lock().await;
                        let controls: Vec<serde_json::Value> = inner
                            .controls
                            .iter()
                            .map(|c| {
                                serde_json::json!({
                                    "control_key": c.control_key,
                                    "display_name": c.display_name,
                                    "supports_standby": c.supports_standby,
                                    "status": c.status,
                                })
                            })
                            .collect();
                        let _ = responder
                            .send_complete(
                                "Success",
                                Some(serde_json::json!({"controls": controls})),
                            )
                            .await;
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
