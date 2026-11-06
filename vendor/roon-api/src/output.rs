use serde::{Deserialize, Serialize};

/// A Roon audio output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub output_id: String,
    pub display_name: String,
    #[serde(default)]
    pub zone_id: String,
    #[serde(default)]
    pub volume: Option<Volume>,
    #[serde(default)]
    pub source_controls: Vec<SourceControl>,
    #[serde(default)]
    pub can_group_with_output_ids: Vec<String>,
}

/// Volume information for an output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Volume {
    #[serde(rename = "type")]
    pub volume_type: String,
    #[serde(default)]
    pub min: f64,
    #[serde(default)]
    pub max: f64,
    #[serde(default)]
    pub value: f64,
    #[serde(default)]
    pub step: f64,
    #[serde(default)]
    pub is_muted: Option<bool>,
}

/// Source control information for an output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceControl {
    pub display_name: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub supports_standby: bool,
    #[serde(default)]
    pub control_key: String,
}
