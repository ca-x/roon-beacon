use std::sync::Arc;

use roon_moo::connection::MooConnection;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

/// Options for a browse request.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BrowseOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_session_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pop_all: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pop_levels: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_list: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_display_offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zone_or_output_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
}

/// Options for loading items from a browse list.
#[derive(Debug, Clone, Default, Serialize)]
pub struct LoadOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multi_session_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub set_display_offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

/// Result of a browse operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowseResult {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub list: Option<BrowseList>,
    #[serde(default)]
    pub item: Option<BrowseItem>,
}

/// A list within the browse hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowseList {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub level: u32,
    #[serde(default)]
    pub display_offset: Option<u32>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub image_key: Option<String>,
}

/// An item in a browse list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowseItem {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub item_key: Option<String>,
    #[serde(default)]
    pub image_key: Option<String>,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub input_prompt: Option<InputPrompt>,
}

/// Input prompt for search items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputPrompt {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub is_password: Option<bool>,
}

/// Result of a load operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadResult {
    #[serde(default)]
    pub items: Vec<BrowseItem>,
    #[serde(default)]
    pub offset: u32,
    #[serde(default)]
    pub list: Option<BrowseList>,
}

/// Browse service for navigating Roon's music library.
#[derive(Debug, Clone)]
pub struct Browse {
    connection: Arc<MooConnection>,
}

impl Browse {
    pub(crate) fn new(connection: Arc<MooConnection>) -> Self {
        Self { connection }
    }

    /// Navigate the browse hierarchy.
    pub async fn browse(&self, opts: BrowseOptions) -> Result<BrowseResult, ApiError> {
        let body = serde_json::to_value(&opts).map_err(|e| {
            ApiError::RegistryFailed(format!("failed to serialize browse options: {}", e))
        })?;

        let response = self
            .connection
            .send_request("com.roonlabs.browse:1/browse", Some(body))
            .await?;

        let result: BrowseResult = serde_json::from_value(
            response
                .json_body()
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .map_err(|e| ApiError::RegistryFailed(format!("failed to parse browse result: {}", e)))?;

        Ok(result)
    }

    /// Load items from the current browse list.
    pub async fn load(&self, opts: LoadOptions) -> Result<LoadResult, ApiError> {
        let body = serde_json::to_value(&opts).map_err(|e| {
            ApiError::RegistryFailed(format!("failed to serialize load options: {}", e))
        })?;

        let response = self
            .connection
            .send_request("com.roonlabs.browse:1/load", Some(body))
            .await?;

        let result: LoadResult = serde_json::from_value(
            response
                .json_body()
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        )
        .map_err(|e| ApiError::RegistryFailed(format!("failed to parse load result: {}", e)))?;

        Ok(result)
    }
}
