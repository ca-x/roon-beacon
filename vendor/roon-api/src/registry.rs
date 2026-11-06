use std::collections::HashMap;
use std::sync::Arc;

use roon_moo::connection::{MooConnection, ServiceHandler};

use crate::core::{Core, CoreInner};
use crate::error::ApiError;
use crate::pairing::PairingState;
use crate::token::StateStore;

/// Extension registration info sent to Roon Core.
#[derive(Debug, Clone)]
pub(crate) struct ExtensionInfo {
    pub extension_id: String,
    pub display_name: String,
    pub display_version: String,
    pub publisher: String,
    pub email: String,
    pub website: Option<String>,
    pub required_services: Vec<String>,
    pub optional_services: Vec<String>,
    pub provided_services: Vec<String>,
}

/// Perform the registry handshake on a MOO connection.
pub(crate) async fn perform_handshake(
    url: &str,
    info: &ExtensionInfo,
    store: &Arc<dyn StateStore>,
    pairing: &PairingState,
) -> Result<Core, ApiError> {
    let service_handlers = build_service_handlers(pairing.clone(), store.clone());

    let connection = MooConnection::connect(url, service_handlers).await?;
    let connection = Arc::new(connection);

    // Step 1: Info request
    let info_response = connection
        .send_request("com.roonlabs.registry:1/info", None)
        .await?;

    let info_body = info_response
        .json_body()
        .ok_or_else(|| ApiError::RegistryFailed("info response has no body".into()))?;

    let core_id = info_body["core_id"]
        .as_str()
        .ok_or_else(|| ApiError::RegistryFailed("missing core_id in info response".into()))?
        .to_string();
    let core_display_name = info_body["display_name"]
        .as_str()
        .unwrap_or("Unknown Core")
        .to_string();
    let core_display_version = info_body["display_version"]
        .as_str()
        .unwrap_or("")
        .to_string();

    // Step 2: Look up persisted token
    let existing_token = store.load_token(&core_id);

    // Step 3: Register
    let mut provided = vec![
        "com.roonlabs.pairing:1".to_string(),
        "com.roonlabs.ping:1".to_string(),
    ];
    provided.extend(info.provided_services.clone());

    let mut reg_body = serde_json::json!({
        "extension_id": info.extension_id,
        "display_name": info.display_name,
        "display_version": info.display_version,
        "publisher": info.publisher,
        "email": info.email,
        "required_services": info.required_services,
        "optional_services": info.optional_services,
        "provided_services": provided
    });

    if let Some(ref token) = existing_token {
        reg_body["token"] = serde_json::Value::String(token.clone());
    }
    if let Some(ref website) = info.website {
        reg_body["website"] = serde_json::Value::String(website.clone());
    }

    let reg_response = connection
        .send_request("com.roonlabs.registry:1/register", Some(reg_body))
        .await?;

    if reg_response.name != "Registered" {
        return Err(ApiError::RegistryFailed(format!(
            "expected 'Registered', got '{}'",
            reg_response.name
        )));
    }

    let reg_body = reg_response
        .json_body()
        .ok_or_else(|| ApiError::RegistryFailed("register response has no body".into()))?;

    let new_token = reg_body["token"].as_str().map(|s| s.to_string());
    let http_port = reg_body["http_port"].as_u64().unwrap_or(0) as u16;

    // Step 4: Persist token
    if let Some(ref token) = new_token
        && let Err(e) = store.save_token(&core_id, token)
    {
        tracing::warn!("Failed to persist token for core {}: {}", core_id, e);
    }

    // Extract host from URL for image service
    let host = extract_host(url);

    Ok(Core {
        inner: Arc::new(CoreInner {
            core_id,
            display_name: core_display_name,
            display_version: core_display_version,
            token: new_token,
            http_port,
            host,
            connection,
        }),
    })
}

/// Perform handshake using a one-time token (skips info request).
pub(crate) async fn perform_handshake_with_token(
    url: &str,
    info: &ExtensionInfo,
    token: &str,
    store: &Arc<dyn StateStore>,
    pairing: &PairingState,
) -> Result<Core, ApiError> {
    let service_handlers = build_service_handlers(pairing.clone(), store.clone());

    let connection = MooConnection::connect(url, service_handlers).await?;
    let connection = Arc::new(connection);

    let mut provided = vec![
        "com.roonlabs.pairing:1".to_string(),
        "com.roonlabs.ping:1".to_string(),
    ];
    provided.extend(info.provided_services.clone());

    let reg_body = serde_json::json!({
        "extension_id": info.extension_id,
        "display_name": info.display_name,
        "display_version": info.display_version,
        "publisher": info.publisher,
        "email": info.email,
        "token": token,
        "required_services": info.required_services,
        "optional_services": info.optional_services,
        "provided_services": provided
    });

    let reg_response = connection
        .send_request(
            "com.roonlabs.registry:1/register_one_time_token",
            Some(reg_body),
        )
        .await?;

    if reg_response.name != "Registered" {
        return Err(ApiError::RegistryFailed(format!(
            "expected 'Registered', got '{}'",
            reg_response.name
        )));
    }

    let reg_body = reg_response
        .json_body()
        .ok_or_else(|| ApiError::RegistryFailed("register response has no body".into()))?;

    let core_id = reg_body["core_id"].as_str().unwrap_or("").to_string();
    let core_display_name = reg_body["display_name"]
        .as_str()
        .unwrap_or("Unknown Core")
        .to_string();
    let core_display_version = reg_body["display_version"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let new_token = reg_body["token"].as_str().map(|s| s.to_string());
    let http_port = reg_body["http_port"].as_u64().unwrap_or(0) as u16;

    if let Some(ref token) = new_token
        && !core_id.is_empty()
        && let Err(e) = store.save_token(&core_id, token)
    {
        tracing::warn!("Failed to persist token: {}", e);
    }

    let host = extract_host(url);

    Ok(Core {
        inner: Arc::new(CoreInner {
            core_id,
            display_name: core_display_name,
            display_version: core_display_version,
            token: new_token,
            http_port,
            host,
            connection,
        }),
    })
}

/// Extract host from a ws:// URL.
fn extract_host(url: &str) -> String {
    url.strip_prefix("ws://")
        .unwrap_or(url)
        .split(':')
        .next()
        .unwrap_or("127.0.0.1")
        .to_string()
}

fn build_service_handlers(
    pairing: PairingState,
    store: Arc<dyn StateStore>,
) -> HashMap<String, ServiceHandler> {
    let mut handlers: HashMap<String, ServiceHandler> = HashMap::new();

    // Ping service
    let ping_handler: ServiceHandler = Arc::new(|_msg, responder| {
        tokio::spawn(async move {
            let _ = responder.send_complete("Success", None).await;
        });
    });
    handlers.insert("com.roonlabs.ping:1".to_string(), ping_handler);

    // Pairing service with state management
    handlers.insert(
        "com.roonlabs.pairing:1".to_string(),
        pairing.build_handler(store),
    );

    handlers
}
