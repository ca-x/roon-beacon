use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex as StdMutex,
};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use roon_moo::{
    connection::{MooConnection, ServiceHandler},
    MooMessage, MooVerb,
};
use roon_sood::{DiscoveredCore, SoodDiscovery};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex, RwLock, Semaphore};
use tokio::time::{sleep, timeout};

use crate::persistence::SecureState;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct Player {
    inner: Arc<Inner>,
}

struct Inner {
    state: RwLock<PlayerState>,
    store: SecureState,
    stop: watch::Sender<bool>,
    tasks: StdMutex<Vec<tokio::task::JoinHandle<()>>>,
    images: reqwest::Client,
}

struct PlayerState {
    cores: BTreeMap<String, CoreEntry>,
    selected_core_id: Option<String>,
    discovery_error: Option<String>,
}

struct CoreEntry {
    summary: CoreSummary,
    endpoint: watch::Sender<DiscoveredCore>,
    peer: Option<Arc<Peer>>,
    zones: BTreeMap<String, Value>,
}

#[derive(Clone, Serialize)]
struct CoreSummary {
    core_id: String,
    name: String,
    host: IpAddr,
    http_port: u16,
    connection: &'static str,
    error: Option<String>,
}

#[derive(Serialize)]
struct PlayerSnapshot {
    cores: Vec<CoreSummary>,
    selected_core_id: Option<String>,
    zones: Vec<Value>,
    discovery_error: Option<String>,
}

struct Peer {
    connection: Option<MooConnection>,
    failed: AtomicBool,
    requests: Semaphore,
    browse: Mutex<()>,
    endpoint: DiscoveredCore,
}

impl Peer {
    fn connection(&self) -> &MooConnection {
        self.connection
            .as_ref()
            .expect("connection exists until Peer drops")
    }

    async fn request(&self, method: &str, body: Option<Value>) -> Result<MooMessage> {
        let _permit = timeout(REQUEST_TIMEOUT, self.requests.acquire()).await??;
        let response = match timeout(
            REQUEST_TIMEOUT,
            self.connection().send_request(method, body),
        )
        .await
        {
            Ok(response) => response?,
            Err(_) => {
                self.failed.store(true, Ordering::Relaxed);
                bail!("Roon request timed out; reconnecting");
            }
        };
        if !matches!(response.name.as_str(), "Success" | "Registered") {
            bail!("Roon returned {}", response.name);
        }
        Ok(response)
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(connection.close());
            }
        }
    }
}

impl Player {
    pub fn new(store: SecureState) -> Result<Self> {
        let (stop, _) = watch::channel(false);
        Ok(Self {
            inner: Arc::new(Inner {
                state: RwLock::new(PlayerState {
                    selected_core_id: store.selected_core_id(),
                    cores: BTreeMap::new(),
                    discovery_error: None,
                }),
                store,
                stop,
                tasks: StdMutex::new(Vec::new()),
                images: reqwest::Client::builder()
                    .timeout(REQUEST_TIMEOUT)
                    .redirect(reqwest::redirect::Policy::none())
                    .build()?,
            }),
        })
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/player", get(snapshot))
            .route("/api/player/core", post(select_core))
            .route("/api/player/control", post(control))
            .route("/api/player/seek", post(seek))
            .route("/api/player/volume", post(volume))
            .route("/api/player/mute", post(mute))
            .route("/api/player/browse", post(browse))
            .route("/api/player/image/:image_key", get(image))
            .with_state(self.clone())
    }

    fn spawn(&self, future: impl std::future::Future<Output = ()> + Send + 'static) {
        self.inner.tasks.lock().unwrap().push(tokio::spawn(future));
    }

    pub async fn start_discovery(&self, bridge_enabled: bool, remote: Option<IpAddr>) {
        let player = self.clone();
        self.spawn(async move {
            let mut stop = player.inner.stop.subscribe();
            loop {
                match SoodDiscovery::start().await {
                    Ok((discovery, mut receiver)) => {
                        player.inner.state.write().await.discovery_error = None;
                        loop {
                            tokio::select! {
                                _ = stop.changed() => break,
                                result = receiver.recv() => match result {
                                    Ok(core) if !(bridge_enabled && core.host.is_loopback() && core.http_port == 9330) => player.observe(core).await,
                                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
                                    Err(_) => break,
                                }
                            }
                        }
                        discovery.stop().await;
                    }
                    Err(error) => {
                        player.inner.state.write().await.discovery_error = Some(error.to_string());
                    }
                }
                if *stop.borrow() { break; }
                tokio::select! { _ = stop.changed() => break, _ = sleep(Duration::from_secs(5)) => {} }
            }
        });
        if let Some(remote) = remote {
            let player = self.clone();
            self.spawn(async move {
                let mut stop = player.inner.stop.subscribe();
                loop {
                    tokio::select! {
                        _ = stop.changed() => break,
                        _ = player.probe_remote(remote) => {},
                    }
                    tokio::select! { _ = stop.changed() => break, _ = sleep(Duration::from_secs(15)) => {} }
                }
            });
        }
    }

    async fn probe_remote(&self, remote: IpAddr) {
        let bind = if remote.is_ipv4() {
            "0.0.0.0:0"
        } else {
            "[::]:0"
        };
        let Ok(socket) = tokio::net::UdpSocket::bind(bind).await else {
            return;
        };
        let mut props = HashMap::new();
        props.insert(
            "query_service_id".to_string(),
            Some(roon_sood::ROON_CORE_SERVICE_ID.to_string()),
        );
        props.insert("_tid".to_string(), Some(uuid::Uuid::new_v4().to_string()));
        let packet = roon_sood::serialize_query(&props);
        if socket
            .send_to(&packet, SocketAddr::new(remote, roon_sood::SOOD_PORT))
            .await
            .is_err()
        {
            return;
        }
        let mut buffer = [0u8; 65535];
        if let Ok(Ok((length, from))) =
            timeout(Duration::from_secs(2), socket.recv_from(&mut buffer)).await
        {
            if from.ip() != remote {
                return;
            }
            if let Ok(message) = roon_sood::parse(&buffer[..length], from) {
                let property =
                    |key: &str| message.props.get(key).and_then(|value| value.as_deref());
                if message.msg_type != roon_sood::SoodType::Response
                    || property("service_id") != Some(roon_sood::ROON_CORE_SERVICE_ID)
                {
                    return;
                }
                if let (Some(core_id), Some(port)) = (
                    property("unique_id"),
                    property("http_port").and_then(|port| port.parse::<u16>().ok()),
                ) {
                    self.observe(DiscoveredCore {
                        core_id: core_id.to_string(),
                        host: remote,
                        http_port: port,
                        name: property("name").map(str::to_string),
                        display_version: None,
                    })
                    .await;
                }
            }
        }
    }

    async fn observe(&self, core: DiscoveredCore) {
        if core.http_port == 0
            || core.core_id.len() > 256
            || core.core_id.starts_with("roon-beacon-")
        {
            return;
        }
        let mut state = self.inner.state.write().await;
        if let Some(entry) = state.cores.get_mut(&core.core_id) {
            let changed = {
                let old = entry.endpoint.borrow();
                old.host != core.host || old.http_port != core.http_port
            };
            if changed {
                entry.summary.host = core.host;
                entry.summary.http_port = core.http_port;
                let _ = entry.endpoint.send(core);
            }
            return;
        }
        if state.cores.len() >= 32 {
            return;
        }
        let (endpoint, receiver) = watch::channel(core.clone());
        if state.selected_core_id.is_none() && self.inner.store.select_core(&core.core_id).is_ok() {
            state.selected_core_id = Some(core.core_id.clone());
        }
        state.cores.insert(
            core.core_id.clone(),
            CoreEntry {
                summary: CoreSummary {
                    core_id: core.core_id.clone(),
                    name: core
                        .name
                        .clone()
                        .unwrap_or_else(|| "Roon Server".to_string()),
                    host: core.host,
                    http_port: core.http_port,
                    connection: "connecting",
                    error: None,
                },
                endpoint,
                peer: None,
                zones: BTreeMap::new(),
            },
        );
        drop(state);
        let player = self.clone();
        self.spawn(async move {
            player.run_core(core.core_id, receiver).await;
        });
    }

    async fn run_core(&self, core_id: String, mut endpoint: watch::Receiver<DiscoveredCore>) {
        let mut stop = self.inner.stop.subscribe();
        loop {
            let discovered = endpoint.borrow().clone();
            tokio::select! {
                _ = stop.changed() => break,
                _ = endpoint.changed() => {},
                result = self.serve_core(discovered) => {
                    if let Err(error) = result {
                        let mut state = self.inner.state.write().await;
                        if let Some(entry) = state.cores.get_mut(&core_id) {
                            if entry.summary.connection != "awaiting_authorization" {
                                entry.summary.connection = "offline";
                                entry.summary.error = Some(error.to_string());
                            }
                            entry.peer = None;
                            entry.zones.clear();
                        }
                    }
                    tokio::select! {
                        _ = stop.changed() => break,
                        _ = endpoint.changed() => {},
                        _ = sleep(Duration::from_secs(5)) => {},
                    }
                }
            }
            self.clear_peer(&core_id).await;
        }
        self.clear_peer(&core_id).await;
    }

    async fn set_connection(&self, core_id: &str, connection: &'static str, error: Option<String>) {
        if let Some(entry) = self.inner.state.write().await.cores.get_mut(core_id) {
            entry.summary.connection = connection;
            entry.summary.error = error;
        }
    }

    async fn clear_peer(&self, core_id: &str) {
        if let Some(entry) = self.inner.state.write().await.cores.get_mut(core_id) {
            entry.peer = None;
            entry.zones.clear();
        }
    }

    async fn serve_core(&self, endpoint: DiscoveredCore) -> Result<()> {
        self.set_connection(&endpoint.core_id, "connecting", None)
            .await;
        let ping: ServiceHandler = Arc::new(|message, responder| {
            tokio::spawn(async move {
                let status = if message.name == "com.roonlabs.ping:1/ping" {
                    "Success"
                } else {
                    "InvalidRequest"
                };
                let _ = responder.send_complete(status, None).await;
            });
        });
        let handlers = HashMap::from([("com.roonlabs.ping:1".to_string(), ping)]);
        let url = format!(
            "ws://{}/api",
            SocketAddr::new(endpoint.host, endpoint.http_port)
        );
        let connection = timeout(REQUEST_TIMEOUT, MooConnection::connect(&url, handlers)).await??;
        let peer = Arc::new(Peer {
            connection: Some(connection),
            failed: AtomicBool::new(false),
            requests: Semaphore::new(16),
            browse: Mutex::new(()),
            endpoint: endpoint.clone(),
        });
        let info = peer.request("com.roonlabs.registry:1/info", None).await?;
        let info = info
            .json_body()
            .context("Roon returned empty server information")?;
        if info["core_id"].as_str() != Some(endpoint.core_id.as_str()) {
            bail!("Discovered Core identity does not match the API endpoint");
        }
        if let Some(name) = info["display_name"].as_str() {
            if let Some(entry) = self
                .inner
                .state
                .write()
                .await
                .cores
                .get_mut(&endpoint.core_id)
            {
                entry.summary.name = name.to_string();
            }
        }
        self.set_connection(&endpoint.core_id, "awaiting_authorization", None)
            .await;
        let mut registration = json!({
            "extension_id":"io.github.ca-x.roon-beacon",
            "display_name":"Roon Beacon",
            "display_version":env!("CARGO_PKG_VERSION"),
            "publisher":"Roon Beacon contributors",
            "email":"roon-beacon@users.noreply.github.com",
            "website":"https://github.com/ca-x/roon-beacon",
            "required_services":["com.roonlabs.transport:2","com.roonlabs.browse:1"],
            "optional_services":[],
            "provided_services":["com.roonlabs.ping:1"]
        });
        if let Some(token) = self.inner.store.token(&endpoint.core_id) {
            registration["token"] = Value::String(token);
        }
        let mut registration_events = peer
            .connection()
            .subscribe("com.roonlabs.registry:1/register", registration)
            .await?;
        let response = timeout(Duration::from_secs(120), async {
            loop {
                let event = registration_events
                    .recv()
                    .await
                    .context("Roon registration connection closed")?;
                if event.name == "Registered" && event.verb == MooVerb::Continue {
                    return Ok::<MooMessage, anyhow::Error>(event);
                }
                if event.verb == MooVerb::Complete {
                    bail!("Roon registration ended: {}", event.name);
                }
            }
        })
        .await;
        let response = match response {
            Ok(response) => response,
            Err(_) => Err(anyhow!(
                "Waiting for Roon extension authorization; retrying"
            )),
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                self.set_connection(
                    &endpoint.core_id,
                    "awaiting_authorization",
                    Some("Enable Roon Beacon in Roon Settings → Extensions".to_string()),
                )
                .await;
                return Err(error);
            }
        };
        let body = response
            .json_body()
            .context("Roon returned empty registration")?;
        let token = body["token"]
            .as_str()
            .context("Roon registration did not provide an authorization token")?;
        self.inner.store.save_token(&endpoint.core_id, token)?;
        let mut events = peer
            .connection()
            .subscribe(
                "com.roonlabs.transport:2/subscribe_zones",
                json!({"subscription_key":0}),
            )
            .await?;
        if let Some(entry) = self
            .inner
            .state
            .write()
            .await
            .cores
            .get_mut(&endpoint.core_id)
        {
            entry.peer = Some(peer.clone());
            entry.summary.connection = "connected";
            entry.summary.error = None;
        }
        let mut health = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                registration = registration_events.recv() => {
                    let registration = registration.context("Roon registration connection closed")?;
                    if registration.name == "Registered" && registration.verb == MooVerb::Continue {
                        if let Some(token) = registration.json_body().and_then(|body|body["token"].as_str()) {
                            self.inner.store.save_token(&endpoint.core_id,token)?;
                        }
                    } else {
                        self.set_connection(&endpoint.core_id,"awaiting_authorization",Some("Roon authorization ended; enable Roon Beacon in Roon Settings → Extensions".to_string())).await;
                        self.clear_peer(&endpoint.core_id).await;
                        bail!("Roon extension registration ended");
                    }
                }
                event = events.recv() => {
                    let event = event.context("Roon zone subscription closed")?;
                    if event.verb == MooVerb::Complete { bail!("Roon zone subscription ended"); }
                    if let Some(body) = event.json_body() {
                        if let Some(entry) = self.inner.state.write().await.cores.get_mut(&endpoint.core_id) {
                            apply_zones(&mut entry.zones, &event.name, body);
                        }
                    }
                }
                _ = health.tick() => {
                    if !peer.connection().is_alive() || peer.failed.load(Ordering::Relaxed) { bail!("Roon connection lost; retrying"); }
                }
            }
        }
    }

    async fn selected(&self) -> Result<(Arc<Peer>, Vec<Value>), ApiError> {
        let state = self.inner.state.read().await;
        let entry = state
            .selected_core_id
            .as_ref()
            .and_then(|id| state.cores.get(id))
            .ok_or_else(|| {
                ApiError::new(
                    StatusCode::CONFLICT,
                    "Select a discovered Roon Server first",
                )
            })?;
        let peer = entry.peer.as_ref().filter(|peer| peer.connection().is_alive() && !peer.failed.load(Ordering::Relaxed))
            .cloned().ok_or_else(|| ApiError::new(StatusCode::CONFLICT,"Roon Server is not connected; enable Roon Beacon in Roon Settings → Extensions"))?;
        Ok((peer, entry.zones.values().cloned().collect()))
    }

    pub async fn shutdown(&self) {
        let _ = self.inner.stop.send(true);
        let tasks = std::mem::take(&mut *self.inner.tasks.lock().unwrap());
        for mut task in tasks {
            if timeout(Duration::from_secs(2), &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

fn apply_zones(zones: &mut BTreeMap<String, Value>, status: &str, body: &Value) {
    if status == "Subscribed" {
        zones.clear();
    }
    for key in ["zones", "zones_added", "zones_changed"] {
        if let Some(updates) = body[key].as_array() {
            for zone in updates {
                if let Some(zone_id) = zone["zone_id"].as_str() {
                    zones.insert(zone_id.to_string(), zone.clone());
                }
            }
        }
    }
    if let Some(removed) = body["zones_removed"].as_array() {
        for zone in removed.iter().filter_map(Value::as_str) {
            zones.remove(zone);
        }
    }
    if let Some(seeks) = body["zones_seek_changed"].as_array() {
        for seek in seeks {
            if let Some(zone) = seek["zone_id"].as_str().and_then(|id| zones.get_mut(id)) {
                if let Some(position) = seek.get("seek_position") {
                    zone["seek_position"] = position.clone();
                    if zone["now_playing"].is_object() {
                        zone["now_playing"]["seek_position"] = position.clone();
                    }
                }
                if let Some(remaining) = seek.get("queue_time_remaining") {
                    zone["queue_time_remaining"] = remaining.clone();
                }
            }
        }
    }
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: &str) -> Self {
        Self {
            status,
            message: message.to_string(),
        }
    }
    fn bad(message: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, &error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error":self.message}))).into_response()
    }
}

async fn snapshot(State(player): State<Player>) -> Json<PlayerSnapshot> {
    let state = player.inner.state.read().await;
    Json(PlayerSnapshot {
        cores: state
            .cores
            .values()
            .map(|entry| entry.summary.clone())
            .collect(),
        selected_core_id: state.selected_core_id.clone(),
        zones: state
            .selected_core_id
            .as_ref()
            .and_then(|id| state.cores.get(id))
            .filter(|entry| entry.summary.connection == "connected")
            .map(|entry| entry.zones.values().cloned().collect())
            .unwrap_or_default(),
        discovery_error: state.discovery_error.clone(),
    })
}

#[derive(Deserialize)]
struct Selection {
    core_id: String,
}

async fn select_core(
    State(player): State<Player>,
    Json(input): Json<Selection>,
) -> Result<Json<Value>, ApiError> {
    let mut state = player.inner.state.write().await;
    if !state.cores.contains_key(&input.core_id) {
        return Err(ApiError::bad("Unknown Roon Server"));
    }
    player.inner.store.select_core(&input.core_id)?;
    state.selected_core_id = Some(input.core_id);
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Control {
    Play,
    Pause,
    Playpause,
    Stop,
    Previous,
    Next,
}

impl Control {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Playpause => "playpause",
            Self::Stop => "stop",
            Self::Previous => "previous",
            Self::Next => "next",
        }
    }
}

#[derive(Deserialize)]
struct ControlRequest {
    zone_id: String,
    control: Control,
}

fn selected_zone<'data>(zones: &'data [Value], zone_id: &str) -> Result<&'data Value, ApiError> {
    zones
        .iter()
        .find(|zone| zone["zone_id"].as_str() == Some(zone_id))
        .ok_or_else(|| ApiError::bad("Unknown playback zone"))
}

async fn control(
    State(player): State<Player>,
    Json(input): Json<ControlRequest>,
) -> Result<Json<Value>, ApiError> {
    let (peer, zones) = player.selected().await?;
    let zone = selected_zone(&zones, &input.zone_id)?;
    let capability = match input.control {
        Control::Play => Some("is_play_allowed"),
        Control::Pause => Some("is_pause_allowed"),
        Control::Next => Some("is_next_allowed"),
        Control::Previous => Some("is_previous_allowed"),
        Control::Playpause => Some(if zone["state"] == "playing" {
            "is_pause_allowed"
        } else {
            "is_play_allowed"
        }),
        Control::Stop => None,
    };
    if capability.is_some_and(|key| zone[key].as_bool() != Some(true)) {
        return Err(ApiError::bad(
            "This playback action is not currently available",
        ));
    }
    peer.request(
        "com.roonlabs.transport:2/control",
        Some(json!({"zone_or_output_id":input.zone_id,"control":input.control.as_str()})),
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct SeekRequest {
    zone_id: String,
    seconds: f64,
}

async fn seek(
    State(player): State<Player>,
    Json(input): Json<SeekRequest>,
) -> Result<Json<Value>, ApiError> {
    let (peer, zones) = player.selected().await?;
    let zone = selected_zone(&zones, &input.zone_id)?;
    let length = zone["now_playing"]["length"].as_f64().unwrap_or(0.0);
    if zone["is_seek_allowed"].as_bool() != Some(true)
        || !input.seconds.is_finite()
        || input.seconds < 0.0
        || input.seconds > length
    {
        return Err(ApiError::bad(
            "Seek position is unavailable or outside the current track",
        ));
    }
    peer.request("com.roonlabs.transport:2/seek",Some(json!({"zone_or_output_id":input.zone_id,"how":"absolute","seconds":input.seconds.round()}))).await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct VolumeRequest {
    output_id: String,
    value: f64,
}

fn selected_output<'data>(
    zones: &'data [Value],
    output_id: &str,
) -> Result<&'data Value, ApiError> {
    zones
        .iter()
        .filter_map(|zone| zone["outputs"].as_array())
        .flatten()
        .find(|output| output["output_id"].as_str() == Some(output_id))
        .ok_or_else(|| ApiError::bad("Unknown audio output"))
}

async fn volume(
    State(player): State<Player>,
    Json(input): Json<VolumeRequest>,
) -> Result<Json<Value>, ApiError> {
    let (peer, zones) = player.selected().await?;
    let volume = &selected_output(&zones, &input.output_id)?["volume"];
    let minimum = volume["min"].as_f64().unwrap_or(0.0);
    let maximum = volume["max"].as_f64().unwrap_or(0.0);
    if !matches!(volume["type"].as_str(), Some("number" | "db"))
        || !input.value.is_finite()
        || input.value < minimum
        || input.value > maximum
    {
        return Err(ApiError::bad(
            "Output volume is fixed, unsupported, or outside its allowed range",
        ));
    }
    peer.request(
        "com.roonlabs.transport:2/change_volume",
        Some(json!({"output_id":input.output_id,"how":"absolute","value":input.value})),
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct MuteRequest {
    output_id: String,
    is_muted: bool,
}

async fn mute(
    State(player): State<Player>,
    Json(input): Json<MuteRequest>,
) -> Result<Json<Value>, ApiError> {
    let (peer, zones) = player.selected().await?;
    if selected_output(&zones, &input.output_id)?["volume"]["is_muted"]
        .as_bool()
        .is_none()
    {
        return Err(ApiError::bad("This output does not support mute"));
    }
    peer.request(
        "com.roonlabs.transport:2/mute",
        Some(json!({"output_id":input.output_id,"how":if input.is_muted {"mute"} else {"unmute"}})),
    )
    .await?;
    Ok(Json(json!({"ok":true})))
}

#[derive(Deserialize)]
struct BrowseRequest {
    session: String,
    #[serde(default = "browse_hierarchy")]
    hierarchy: String,
    item_key: Option<String>,
    pop_all: Option<bool>,
    pop_levels: Option<u32>,
    input: Option<String>,
    zone_id: Option<String>,
    offset: Option<u32>,
    count: Option<u32>,
}

fn browse_hierarchy() -> String {
    "browse".to_string()
}

async fn browse(
    State(player): State<Player>,
    Json(input): Json<BrowseRequest>,
) -> Result<Json<Value>, ApiError> {
    if input.session.is_empty()
        || input.session.len() > 128
        || !matches!(input.hierarchy.as_str(), "browse" | "search")
        || input.count.is_some_and(|count| count == 0 || count > 100)
        || input.offset.is_some_and(|offset| offset > 1_000_000)
        || input.item_key.as_ref().is_some_and(|key| key.len() > 1024)
        || input.input.as_ref().is_some_and(|value| value.len() > 4096)
    {
        return Err(ApiError::bad(
            "Invalid browse session, hierarchy, or page bounds",
        ));
    }
    let (peer, zones) = player.selected().await?;
    if let Some(zone) = &input.zone_id {
        selected_zone(&zones, zone)?;
    }
    let _guard = peer.browse.lock().await;
    let mut options = json!({"hierarchy":input.hierarchy,"multi_session_key":input.session});
    for (key, value) in [
        ("item_key", input.item_key.map(Value::String)),
        ("pop_all", input.pop_all.map(Value::Bool)),
        ("pop_levels", input.pop_levels.map(Value::from)),
        ("input", input.input.map(Value::String)),
        ("zone_or_output_id", input.zone_id.map(Value::String)),
    ] {
        if let Some(value) = value {
            options[key] = value;
        }
    }
    let response = peer
        .request("com.roonlabs.browse:1/browse", Some(options))
        .await?;
    let mut response = response.json_body().cloned().unwrap_or(Value::Null);
    if response["action"] != "list" {
        let object = response
            .as_object_mut()
            .context("Roon returned an invalid browse response")?;
        object.entry("items").or_insert_with(|| json!([]));
        object.entry("offset").or_insert_with(|| json!(0));
        return Ok(Json(response));
    }
    let loaded=peer.request("com.roonlabs.browse:1/load",Some(json!({"hierarchy":input.hierarchy,"multi_session_key":input.session,"offset":input.offset.unwrap_or(0),"count":input.count.unwrap_or(50)}))).await?;
    let loaded = loaded
        .json_body()
        .context("Roon returned an empty browse list")?;
    let list = loaded
        .get("list")
        .filter(|value| value.is_object())
        .unwrap_or(&response["list"]);
    Ok(Json(
        json!({"action":"list","list":list,"items":loaded["items"],"offset":loaded["offset"]}),
    ))
}

async fn image(
    State(player): State<Player>,
    Path(image_key): Path<String>,
) -> Result<Response, ApiError> {
    if image_key.len() > 1024 || image_key.is_empty() {
        return Err(ApiError::bad("Invalid cover image key"));
    }
    let (peer, _) = player.selected().await?;
    let mut url = reqwest::Url::parse(&format!(
        "http://{}/",
        SocketAddr::new(peer.endpoint.host, peer.endpoint.http_port)
    ))
    .map_err(|error| anyhow!(error))?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("invalid Core image address"))?
        .extend(["api", "image", image_key.as_str()]);
    url.query_pairs_mut().extend_pairs([
        ("scale", "fit"),
        ("width", "640"),
        ("height", "640"),
        ("format", "image/jpeg"),
    ]);
    let mut response = player
        .inner
        .images
        .get(url)
        .send()
        .await
        .map_err(|error| anyhow!("Cannot fetch Roon cover: {error}"))?
        .error_for_status()
        .map_err(|error| anyhow!("Cannot fetch Roon cover: {error}"))?;
    let limit = 5 * 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(ApiError::bad("Cover image exceeds the size limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| anyhow!("Cannot read Roon cover: {error}"))?
    {
        if bytes.len() + chunk.len() > limit {
            return Err(ApiError::bad("Cover image exceeds the size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "Roon did not return a JPEG cover",
        ));
    }
    Ok(([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response())
}

#[cfg(test)]
mod tests;
