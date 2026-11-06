use std::sync::Arc;

use roon_moo::MooMessage;
use roon_moo::connection::{ResponseSender, ServiceHandler};
use tokio::sync::Mutex;

/// Callback for volume/mute change requests from Roon Core.
pub type VolumeCallback = Arc<dyn Fn(VolumeRequest) + Send + Sync>;

/// A request from Roon Core to change volume or mute state.
#[derive(Debug, Clone)]
pub enum VolumeRequest {
    SetVolume {
        control_key: String,
        mode: String,
        value: f64,
    },
    SetMute {
        control_key: String,
        is_muted: bool,
    },
}

/// Volume Control service — allows Roon to control external device volume.
///
/// Provided by the extension to Roon Core. Roon Core sends volume change
/// requests when the user adjusts volume through the Roon UI.
#[derive(Clone)]
pub struct VolumeControlService {
    inner: Arc<Mutex<VolumeInner>>,
}

struct VolumeInner {
    controls: Vec<VolumeControlDef>,
    callback: Option<VolumeCallback>,
}

/// Definition of a volume control endpoint.
#[derive(Debug, Clone)]
pub struct VolumeControlDef {
    pub control_key: String,
    pub display_name: String,
    pub volume_type: String,
    pub volume_min: f64,
    pub volume_max: f64,
    pub volume_step: f64,
    pub volume_value: f64,
    pub is_muted: bool,
}

impl VolumeControlService {
    pub fn new(controls: Vec<VolumeControlDef>, callback: VolumeCallback) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VolumeInner {
                controls,
                callback: Some(callback),
            })),
        }
    }

    /// Update the current volume value for a control (call after external change).
    pub async fn update_volume(&self, control_key: &str, value: f64, is_muted: bool) {
        let mut inner = self.inner.lock().await;
        for control in &mut inner.controls {
            if control.control_key == control_key {
                control.volume_value = value;
                control.is_muted = is_muted;
            }
        }
    }

    pub fn service_name() -> &'static str {
        "com.roonlabs.volume:1"
    }

    pub fn build_handler(&self) -> ServiceHandler {
        let state = self.clone();
        Arc::new(move |msg: MooMessage, responder: ResponseSender| {
            let method = msg.method().unwrap_or("").to_string();
            let state = state.clone();
            tokio::spawn(async move {
                match method.as_str() {
                    "set_volume" => {
                        if let Some(body) = msg.json_body() {
                            let inner = state.inner.lock().await;
                            if let Some(ref cb) = inner.callback {
                                cb(VolumeRequest::SetVolume {
                                    control_key: body["control_key"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    mode: body["mode"].as_str().unwrap_or("absolute").to_string(),
                                    value: body["value"].as_f64().unwrap_or(0.0),
                                });
                            }
                        }
                        let _ = responder.send_complete("Success", None).await;
                    }
                    "set_mute" => {
                        if let Some(body) = msg.json_body() {
                            let inner = state.inner.lock().await;
                            if let Some(ref cb) = inner.callback {
                                cb(VolumeRequest::SetMute {
                                    control_key: body["control_key"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    is_muted: body["how"]
                                        .as_str()
                                        .map(|h| h == "mute")
                                        .unwrap_or(false),
                                });
                            }
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
                                    "volume_type": c.volume_type,
                                    "volume_min": c.volume_min,
                                    "volume_max": c.volume_max,
                                    "volume_step": c.volume_step,
                                    "volume_value": c.volume_value,
                                    "is_muted": c.is_muted,
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
