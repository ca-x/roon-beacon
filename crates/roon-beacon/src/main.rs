//! roon-beacon
//!
//! Runs inside the local LAN and presents a remote Roon Core as a local SOOD
//! beacon plus TCP forwarding endpoint.

mod audio;
mod auth;
mod persistence;
mod player;

use anyhow::{Context, Result};
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{Html, IntoResponse},
    routing::{get, post},
    Json, Router,
};
use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, info, warn};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum AddrFamily {
    /// Prefer IPv4 addresses.
    V4,
    /// Prefer IPv6 addresses.
    V6,
    /// Use the first resolved address.
    Any,
}

use roon_sood::{
    self, serialize_query, serialize_response, SoodMessage, SoodType, ROON_CORE_SERVICE_ID,
    SOOD_PORT,
};

const SOOD_MCAST: Ipv4Addr = Ipv4Addr::new(239, 255, 90, 90);
const ROON_DISCOVERY_PORT: u16 = 9330;
const SOOD_ANNOUNCE_TTL: u32 = 64;
const WEB_UI_ADDR: &str = "127.0.0.1:9339";
const DEFAULT_LIVE_PROXY_TIMEOUT_MS: u64 = 800;

const ROON_PORTS: &[(u16, &str)] = &[
    (9100, "RAAT / legacy"),
    (9101, "RAAT Alt"),
    (9150, "Roon Core TCP"),
    (9200, "Roon Web Display"),
    (9330, "Roon Core API / Web Display"),
    (9331, "Roon Core API fallback"),
    (9332, "Roon Web Display HTTPS"),
    (55000, "Roon Core HTTPS"),
];

/// Discover Roon Servers, control music from the Web, and optionally bridge a remote Core.
///
/// `--local-ip` must be the LAN address visible to Roon Remote clients, not a
/// VPN/overlay address and not 127.0.0.1.
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Remote Roon Core host. IP addresses, host names, and DNS names are supported.
    #[arg(short = 'r', long, value_name = "HOST", requires = "local_ip")]
    remote_ip: Option<String>,

    /// Local LAN IP in the same network as Roon Remote clients.
    #[arg(short = 'l', long, value_name = "IP")]
    local_ip: Option<String>,

    /// Web server address. Non-loopback binds enable access-token authentication.
    #[arg(long, default_value = WEB_UI_ADDR)]
    web_bind: SocketAddr,

    /// Private directory for authorization state and generated Web access token.
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,

    /// Web access token (16–256 ASCII letters, digits, '-', '_' or '.').
    #[arg(long, env = "ROON_BEACON_WEB_TOKEN", hide_env_values = true)]
    web_token: Option<String>,

    /// Enable the experimental AirPlay-to-browser audio receiver.
    #[arg(long)]
    enable_audio: bool,

    /// AirPlay output name shown in Roon Settings > Audio.
    #[arg(long, default_value = "Roon Beacon Web")]
    audio_name: String,

    /// Fallback Core name shown in Roon Remote when live SOOD proxying is unavailable.
    #[arg(short = 'n', long, default_value = "Roon Core (Beacon)")]
    name: String,

    /// Active SOOD announcement interval in seconds; 0 disables announcements.
    #[arg(long, default_value_t = 10)]
    broadcast_interval: u64,

    /// Use UDP broadcast (255.255.255.255) instead of multicast.
    #[arg(long)]
    use_broadcast: bool,

    /// Retry delay after failed TCP upstream connections, in seconds.
    #[arg(long, default_value_t = 3)]
    retry_secs: u64,

    /// Address family preference for host name resolution.
    #[arg(long, value_enum, default_value_t = AddrFamily::V4)]
    prefer: AddrFamily,

    /// Skip local TCP forwarding ports, comma separated.
    #[arg(long, value_name = "PORTS", value_delimiter = ',')]
    skip_ports: Vec<u16>,

    /// Forward local RAAT Bridge/Output SOOD announcements to the remote Core.
    #[arg(long)]
    reverse_proxy: bool,

    /// Reachable address used when rewriting reverse-proxied local RAAT announcements.
    #[arg(long, value_name = "IP", alias = "tailscale-ip")]
    proxy_ip: Option<String>,

    /// Disable startup probing of the remote Core SOOD template.
    #[arg(long)]
    no_probe: bool,

    /// Startup probe timeout in seconds.
    #[arg(long, default_value_t = 3)]
    probe_timeout: u64,
}

#[derive(Clone, Debug, Serialize)]
struct RuntimeConfig {
    name: String,
    broadcast_interval_secs: u64,
    live_proxy_timeout_ms: u64,
    reverse_proxy: bool,
    proxy_ip: Option<String>,
}

impl RuntimeConfig {
    fn from_args(args: &Args) -> Self {
        Self {
            name: args.name.clone(),
            broadcast_interval_secs: args.broadcast_interval,
            live_proxy_timeout_ms: DEFAULT_LIVE_PROXY_TIMEOUT_MS,
            reverse_proxy: args.reverse_proxy,
            proxy_ip: normalized_optional_string(args.proxy_ip.clone()),
        }
    }
}

#[derive(Default)]
struct RuntimeMetrics {
    sood_queries: AtomicU64,
    sood_responses: AtomicU64,
    live_core_responses: AtomicU64,
    template_fallbacks: AtomicU64,
    reverse_announcements: AtomicU64,
    tcp_connections: AtomicU64,
    tcp_down_bytes: AtomicU64,
    tcp_up_bytes: AtomicU64,
}

#[derive(Serialize)]
struct MetricsSnapshot {
    sood_queries: u64,
    sood_responses: u64,
    live_core_responses: u64,
    template_fallbacks: u64,
    reverse_announcements: u64,
    tcp_connections: u64,
    tcp_down_bytes: u64,
    tcp_up_bytes: u64,
}

impl RuntimeMetrics {
    fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            sood_queries: self.sood_queries.load(Ordering::Relaxed),
            sood_responses: self.sood_responses.load(Ordering::Relaxed),
            live_core_responses: self.live_core_responses.load(Ordering::Relaxed),
            template_fallbacks: self.template_fallbacks.load(Ordering::Relaxed),
            reverse_announcements: self.reverse_announcements.load(Ordering::Relaxed),
            tcp_connections: self.tcp_connections.load(Ordering::Relaxed),
            tcp_down_bytes: self.tcp_down_bytes.load(Ordering::Relaxed),
            tcp_up_bytes: self.tcp_up_bytes.load(Ordering::Relaxed),
        }
    }
}

#[derive(Clone)]
struct WebState {
    config: Arc<RwLock<RuntimeConfig>>,
    metrics: Arc<RuntimeMetrics>,
    started_at: Instant,
    remote_ip: Option<String>,
    local_ip: String,
    use_broadcast: bool,
    web_bind: SocketAddr,
}

#[derive(Deserialize)]
struct UpdateConfig {
    name: String,
    broadcast_interval_secs: u64,
    live_proxy_timeout_ms: u64,
    reverse_proxy: bool,
    proxy_ip: Option<String>,
}

#[derive(Serialize)]
struct StatusResponse {
    version: &'static str,
    remote_ip: Option<String>,
    local_ip: String,
    sood_port: u16,
    discovery_port: u16,
    web_ui_addr: String,
    forwarding_enabled: bool,
    use_broadcast: bool,
    uptime_secs: u64,
    config: RuntimeConfig,
    metrics: MetricsSnapshot,
}

fn normalized_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|v| {
        let trimmed = v.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

async fn status_snapshot(state: &WebState) -> StatusResponse {
    StatusResponse {
        version: env!("CARGO_PKG_VERSION"),
        remote_ip: state.remote_ip.clone(),
        local_ip: state.local_ip.clone(),
        sood_port: SOOD_PORT,
        discovery_port: ROON_DISCOVERY_PORT,
        web_ui_addr: state.web_bind.to_string(),
        forwarding_enabled: state.remote_ip.is_some(),
        use_broadcast: state.use_broadcast,
        uptime_secs: state.started_at.elapsed().as_secs(),
        config: state.config.read().await.clone(),
        metrics: state.metrics.snapshot(),
    }
}

async fn web_index() -> Html<&'static str> {
    Html(WEB_UI_HTML)
}

async fn web_styles() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        WEB_UI_CSS,
    )
}

async fn web_script() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        WEB_UI_JS,
    )
}

async fn web_favicon() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn web_logo() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], WEB_LOGO)
}

async fn web_favicon_png() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], WEB_FAVICON)
}

async fn web_touch_icon() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "image/png")], WEB_TOUCH_ICON)
}

async fn api_status(State(state): State<WebState>) -> Json<StatusResponse> {
    Json(status_snapshot(&state).await)
}

async fn api_update_config(
    State(state): State<WebState>,
    Json(input): Json<UpdateConfig>,
) -> Result<Json<StatusResponse>, (StatusCode, String)> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "name must not be empty".to_string(),
        ));
    }

    if input.broadcast_interval_secs > 3600 {
        return Err((
            StatusCode::BAD_REQUEST,
            "broadcast interval must be between 0 and 3600 seconds".to_string(),
        ));
    }

    if !(100..=10_000).contains(&input.live_proxy_timeout_ms) {
        return Err((
            StatusCode::BAD_REQUEST,
            "live proxy timeout must be between 100 and 10000 ms".to_string(),
        ));
    }

    let next = RuntimeConfig {
        name: name.to_string(),
        broadcast_interval_secs: input.broadcast_interval_secs,
        live_proxy_timeout_ms: input.live_proxy_timeout_ms,
        reverse_proxy: input.reverse_proxy,
        proxy_ip: normalized_optional_string(input.proxy_ip),
    };

    *state.config.write().await = next;
    info!("Runtime config updated from Web UI");

    Ok(Json(status_snapshot(&state).await))
}

async fn run_web_ui(
    listener: TcpListener,
    state: WebState,
    player: player::Player,
    audio: audio::AudioGateway,
    auth: auth::WebAuth,
) -> Result<()> {
    let router = Router::new()
        .route("/", get(web_index))
        .route("/assets/styles.css", get(web_styles))
        .route("/assets/app.js", get(web_script))
        .route("/favicon.ico", get(web_favicon))
        .route("/assets/logo.png", get(web_logo))
        .route("/assets/favicon.png", get(web_favicon_png))
        .route("/assets/apple-touch-icon.png", get(web_touch_icon))
        .route("/favicon.png", get(web_favicon_png))
        .route("/apple-touch-icon.png", get(web_touch_icon))
        .route("/api/status", get(api_status))
        .route("/api/config", post(api_update_config))
        .route(
            "/api/health",
            get(|| async {
                Json(serde_json::json!({"status":"ok","version":env!("CARGO_PKG_VERSION")}))
            }),
        )
        .with_state(state)
        .merge(player.router())
        .merge(audio.router())
        .merge(auth.router())
        .layer(axum::middleware::from_fn_with_state(auth, auth::protect));

    info!("Web UI listening at http://{}", listener.local_addr()?);
    axum::serve(listener, router)
        .await
        .context("Web UI server failed")
}

const WEB_UI_HTML: &str = include_str!("web/index.html");
const WEB_UI_CSS: &str = include_str!("web/styles.css");
const WEB_UI_JS: &str = include_str!("web/app.js");
const WEB_LOGO: &[u8] = include_bytes!("web/logo.png");
const WEB_FAVICON: &[u8] = include_bytes!("web/favicon.png");
const WEB_TOUCH_ICON: &[u8] = include_bytes!("web/apple-touch-icon.png");

fn hex_dump(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 4);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && i % 16 == 0 {
            s.push('\n');
            s.push_str("              ");
        } else if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02x}", b));
    }
    // Append an ASCII gutter for debug logs.
    s.push_str("  |");
    for b in bytes {
        let c = if b.is_ascii_graphic() || *b == b' ' {
            *b as char
        } else {
            '.'
        };
        s.push(c);
    }
    s.push('|');
    s
}

fn make_sood_response(tid: &str, name: &str, local_ip: &str) -> Result<Vec<u8>> {
    let unique_id = format!("roon-beacon-{}", local_ip.replace('.', "-"));
    let http_port = ROON_DISCOVERY_PORT.to_string();

    let mut props = HashMap::new();
    props.insert("_tid".to_string(), Some(tid.to_string()));
    props.insert(
        "service_id".to_string(),
        Some(ROON_CORE_SERVICE_ID.to_string()),
    );
    props.insert("unique_id".to_string(), Some(unique_id));
    props.insert("display_name".to_string(), Some(name.to_string()));
    props.insert("name".to_string(), Some(name.to_string()));
    props.insert("http_port".to_string(), Some(http_port));

    Ok(serialize_response(&props))
}

/// Probe the remote Roon Core over unicast SOOD and capture a real response template.
async fn probe_core_template(
    remote_ip: &str,
    timeout_secs: u64,
) -> Result<HashMap<String, String>> {
    use tokio::time::timeout;

    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("failed to create probe socket")?;
    sock.connect(fmt_addr(remote_ip, SOOD_PORT))
        .await
        .with_context(|| format!("failed to connect to {}", fmt_addr(remote_ip, SOOD_PORT)))?;

    let queries = [
        ("d52b2cb7-02c5-48fc-981b-a10f0aadd93b", "ROON_SERVER"),
        (ROON_CORE_SERVICE_ID, "ROON_API"),
    ];

    for (sid, label) in &queries {
        let tid = uuid::Uuid::new_v4().to_string();
        let mut props = HashMap::new();
        props.insert("_tid".to_string(), Some(tid));
        props.insert("query_service_id".to_string(), Some(sid.to_string()));
        let pkt = serialize_query(&props);

        info!(
            "Probing remote Core ({label}) at {}",
            fmt_addr(remote_ip, SOOD_PORT)
        );
        sock.send(&pkt)
            .await
            .context("failed to send probe packet")?;

        let mut buf = vec![0u8; 4096];
        match timeout(Duration::from_secs(timeout_secs), sock.recv(&mut buf)).await {
            Ok(Ok(n)) => {
                let dummy_src = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0));
                match roon_sood::parse(&buf[..n], dummy_src) {
                    Ok(msg) if msg.msg_type == SoodType::Response => {
                        info!(
                            "Received Core response for {label}; {} fields",
                            msg.props.len()
                        );
                        for (k, v) in &msg.props {
                            debug!("   Core field: {k} = {v:?}");
                        }
                        let flat: HashMap<String, String> = msg
                            .props
                            .into_iter()
                            .filter_map(|(k, v)| v.map(|vv| (k, vv)))
                            .collect();
                        return Ok(flat);
                    }
                    Ok(_) => warn!("probe packet was not a SOOD response"),
                    Err(e) => warn!("failed to parse SOOD probe response: {e}"),
                }
            }
            Ok(Err(e)) => warn!("probe receive error: {e}"),
            Err(_) => warn!("probe timed out after {timeout_secs}s; trying next service_id"),
        }
    }

    anyhow::bail!(
        "failed to probe a Roon Core SOOD response from {remote_ip}\n\
         Check that the network path is reachable and Roon Server is running.\n\
         On the Core host, `ss -ulnp | grep 9003` should show a listener.",
        remote_ip = remote_ip
    )
}

/// Forward a client SOOD query unchanged to the remote Core and return the real response.
async fn proxy_query_to_core(
    remote_ip: &str,
    phone_query_raw: &[u8],
    timeout_ms: u64,
) -> Result<Vec<u8>> {
    use tokio::time::timeout;

    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("failed to bind SOOD proxy socket")?;
    sock.connect(fmt_addr(remote_ip, SOOD_PORT))
        .await
        .context("failed to connect SOOD proxy socket")?;
    sock.send(phone_query_raw)
        .await
        .context("failed to send proxied SOOD query")?;

    let mut buf = vec![0u8; 4096];
    let n = timeout(Duration::from_millis(timeout_ms), sock.recv(&mut buf))
        .await
        .context("SOOD proxy receive timed out")?
        .context("failed to receive proxied SOOD response")?;

    Ok(buf[..n].to_vec())
}

/// Resolve the remote host supplied through `--remote-ip`.
async fn resolve_remote_host(host: &str, prefer: AddrFamily) -> Result<String> {
    use tokio::net::lookup_host;

    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(host.to_string());
    }

    let probe = format!("{host}:80");
    let addrs: Vec<std::net::SocketAddr> = lookup_host(&probe)
        .await
        .with_context(|| {
            format!(
                "failed to resolve host '{host}'. Try `ping {host}` or `getent ahosts {host}` and check DNS configuration."
            )
        })?
        .collect();

    if addrs.is_empty() {
        anyhow::bail!("'{host}' resolved to no addresses");
    }

    let chosen = match prefer {
        AddrFamily::V4 => {
            if let Some(a) = addrs.iter().find(|a| a.is_ipv4()) {
                Some(*a)
            } else {
                tracing::warn!(
                    "--prefer v4 was requested, but '{host}' has no IPv4 record; falling back"
                );
                addrs.first().copied()
            }
        }
        AddrFamily::V6 => {
            if let Some(a) = addrs.iter().find(|a| a.is_ipv6()) {
                Some(*a)
            } else {
                tracing::warn!(
                    "--prefer v6 was requested, but '{host}' has no IPv6 record; falling back"
                );
                addrs.first().copied()
            }
        }
        AddrFamily::Any => addrs.first().copied(),
    };

    chosen
        .map(|a| a.ip().to_string())
        .ok_or_else(|| anyhow::anyhow!("'{host}' resolved to no usable address"))
}

/// Build a legal host:port string, adding brackets for IPv6 addresses.
fn fmt_addr(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Rewrite and forward local RAAT Bridge/Output SOOD announcements to the remote Core.
async fn forward_local_announcement(
    raw: &[u8],
    msg: &SoodMessage,
    remote_ip: &str,
    proxy_ip: &str,
    local_ip: &str,
) -> Result<()> {
    // Avoid forwarding Core announcements back to itself.
    if let Some(Some(sid)) = msg.props.get("service_id") {
        if sid == ROON_CORE_SERVICE_ID {
            return Ok(());
        }
    }
    // Avoid forwarding responses generated by this bridge.
    if let Some(Some(uid)) = msg.props.get("unique_id") {
        if uid.starts_with("roon-beacon-") {
            return Ok(());
        }
    }
    if let Some(Some(dn)) = msg.props.get("display_name") {
        if dn.contains("(Beacon)") || dn.contains("(Bridge)") {
            return Ok(());
        }
    }

    // Rewrite any exact IP-valued TLV fields.
    let mut new_pkt = raw.to_vec();
    if let Some(pos) = find_subsequence(&new_pkt, local_ip.as_bytes()) {
        let mut rebuilt = Vec::with_capacity(new_pkt.len() + 16);
        rebuilt.extend_from_slice(&new_pkt[..pos]);
        rebuilt.extend_from_slice(proxy_ip.as_bytes());
        rebuilt.extend_from_slice(&new_pkt[pos + local_ip.len()..]);
        if proxy_ip.len() != local_ip.len() {
            new_pkt = rebuild_sood_pkt_with_ip_replaced(raw, local_ip, proxy_ip)?;
        } else {
            new_pkt = rebuilt;
        }
    }

    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("failed to bind reverse proxy socket")?;
    sock.send_to(&new_pkt, fmt_addr(remote_ip, SOOD_PORT))
        .await
        .context("failed to forward reverse SOOD announcement")?;

    let dn = msg
        .props
        .get("display_name")
        .and_then(|v| v.as_deref())
        .or(msg.props.get("name").and_then(|v| v.as_deref()))
        .unwrap_or("?");
    info!("Reverse announcement forwarded to Core ({remote_ip}): {dn}");
    Ok(())
}

/// Replace exact IP-valued SOOD fields, rebuilding TLV lengths when necessary.
fn rebuild_sood_pkt_with_ip_replaced(pkt: &[u8], old_ip: &str, new_ip: &str) -> Result<Vec<u8>> {
    if pkt.len() < 7 || &pkt[..4] != b"SOOD" {
        anyhow::bail!("not a SOOD packet");
    }
    let field_count = pkt[6] as usize;
    let mut i = 7;
    let mut out = Vec::with_capacity(pkt.len() + 16);
    out.extend_from_slice(&pkt[..7]);

    for _ in 0..field_count {
        if i >= pkt.len() {
            break;
        }
        let klen = pkt[i] as usize;
        i += 1;
        if i + klen > pkt.len() {
            break;
        }
        let key = pkt[i..i + klen].to_vec();
        i += klen;
        if i + 2 > pkt.len() {
            break;
        }
        let vlen = u16::from_be_bytes([pkt[i], pkt[i + 1]]) as usize;
        i += 2;
        let val = if vlen == 0xffff {
            None
        } else {
            if i + vlen > pkt.len() {
                break;
            }
            let v = pkt[i..i + vlen].to_vec();
            i += vlen;
            Some(v)
        };

        out.push(klen as u8);
        out.extend_from_slice(&key);

        match val {
            None => out.extend_from_slice(&0xffffu16.to_be_bytes()),
            Some(v) => {
                let v_str = String::from_utf8_lossy(&v);
                if v_str == old_ip {
                    let nb = new_ip.as_bytes();
                    out.extend_from_slice(&(nb.len() as u16).to_be_bytes());
                    out.extend_from_slice(nb);
                } else {
                    out.extend_from_slice(&(v.len() as u16).to_be_bytes());
                    out.extend_from_slice(&v);
                }
            }
        }
    }
    Ok(out)
}

/// Build a fallback response from the probed Core template while rewriting host fields.
fn make_sood_response_from_template(
    tid: &str,
    template: &HashMap<String, String>,
    local_ip: &str,
    _name: &str,
) -> Result<Vec<u8>> {
    let http_port_str = ROON_DISCOVERY_PORT.to_string();
    let mut props: HashMap<String, Option<String>> = HashMap::new();

    // `_tid` must match the client query.
    props.insert("_tid".to_string(), Some(tid.to_string()));

    for (k, v) in template {
        match k.as_str() {
            "_tid" => continue,
            "http_port" => {
                props.insert(k.clone(), Some(http_port_str.clone()));
            }
            "tcp_addr" | "ip" | "address" => {
                props.insert(k.clone(), Some(local_ip.to_string()));
            }
            _ => {
                props.insert(k.clone(), Some(v.clone()));
            }
        }
    }

    Ok(serialize_response(&props))
}

fn should_answer_query(msg: &SoodMessage) -> bool {
    if msg.msg_type != SoodType::Query {
        return false;
    }

    match msg.props.get("query_service_id").and_then(|v| v.as_deref()) {
        Some(service_id) => service_id == ROON_CORE_SERVICE_ID,
        None => true,
    }
}

/// Create the SOOD UDP socket on the selected LAN interface.
async fn make_sood_socket(local_ip_v4: Ipv4Addr, use_broadcast: bool) -> Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};

    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("failed to create UDP socket")?;

    sock.set_reuse_address(true).ok();
    sock.set_ttl(SOOD_ANNOUNCE_TTL).ok();

    if use_broadcast {
        sock.set_broadcast(true).ok();
    }

    sock.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, SOOD_PORT).into())
        .with_context(|| format!("failed to bind UDP 0.0.0.0:{SOOD_PORT}; port may be in use"))?;

    if !use_broadcast {
        sock.join_multicast_v4(&SOOD_MCAST, &local_ip_v4)
            .with_context(|| {
                format!(
                    "failed to join multicast group {SOOD_MCAST} on interface {local_ip_v4}; confirm --local-ip is a LAN address"
                )
            })?;

        sock.set_multicast_if_v4(&local_ip_v4)
            .context("failed to set multicast interface")?;

        sock.set_multicast_ttl_v4(SOOD_ANNOUNCE_TTL).ok();
        sock.set_multicast_loop_v4(true).ok();
    }

    let std_sock: std::net::UdpSocket = sock.into();
    std_sock.set_nonblocking(true)?;
    UdpSocket::from_std(std_sock).context("failed to convert UDP socket to tokio")
}

/// Run SOOD discovery responses and active announcements.
async fn run_sood(
    local_ip_str: String,
    use_broadcast: bool,
    template: Option<Arc<HashMap<String, String>>>,
    remote_ip: String,
    config: Arc<RwLock<RuntimeConfig>>,
    metrics: Arc<RuntimeMetrics>,
) -> Result<()> {
    let local_ip_v4: Ipv4Addr = local_ip_str
        .parse()
        .context("--local-ip must be a valid IPv4 address")?;

    let socket = Arc::new(
        make_sood_socket(local_ip_v4, use_broadcast)
            .await
            .context("failed to initialize SOOD socket")?,
    );

    let mode = if use_broadcast {
        "broadcast"
    } else {
        "multicast"
    };
    info!("SOOD listening on 0.0.0.0:{SOOD_PORT}; mode={mode}; interface={local_ip_str}");
    info!("Roon Remote queries will resolve to {local_ip_str}:{ROON_DISCOVERY_PORT}");

    let sock2 = Arc::clone(&socket);
    let lip = local_ip_str.clone();
    let tpl = template.clone();
    let config_for_announcements = Arc::clone(&config);
    tokio::spawn(async move {
        sleep(Duration::from_millis(500)).await;
        loop {
            let cfg = config_for_announcements.read().await.clone();
            if cfg.broadcast_interval_secs == 0 {
                sleep(Duration::from_secs(1)).await;
                continue;
            }

            let pkt_res = if let Some(ref t) = tpl {
                make_sood_response_from_template("*", t, &lip, &cfg.name)
            } else {
                make_sood_response("*", &cfg.name, &lip)
            };
            let pkt = match pkt_res {
                Ok(pkt) => pkt,
                Err(e) => {
                    warn!("failed to build SOOD announcement packet: {e:#}");
                    sleep(Duration::from_secs(cfg.broadcast_interval_secs)).await;
                    continue;
                }
            };
            let dest = if use_broadcast {
                SocketAddr::from((Ipv4Addr::new(255, 255, 255, 255), SOOD_PORT))
            } else {
                SocketAddr::from((SOOD_MCAST, SOOD_PORT))
            };
            match sock2.send_to(&pkt, dest).await {
                Ok(_) => info!("SOOD announcement sent to {dest} via {mode}"),
                Err(e) => warn!("failed to send SOOD announcement: {e}"),
            }

            sleep(Duration::from_secs(cfg.broadcast_interval_secs)).await;
        }
    });

    let mut tid_cache: HashMap<String, (Vec<u8>, std::time::Instant)> = HashMap::new();
    let mut buf = vec![0u8; 4096];
    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                warn!("SOOD receive error: {e}");
                continue;
            }
        };

        let raw = &buf[..len];
        debug!("SOOD packet received from {src}: {len}B");

        let msg = match roon_sood::parse(raw, src) {
            Ok(msg) => msg,
            Err(e) => {
                debug!("failed to parse SOOD packet: {e}");
                continue;
            }
        };

        let cfg = config.read().await.clone();
        if cfg.reverse_proxy && msg.msg_type == SoodType::Response {
            if let Some(ref pip) = cfg.proxy_ip {
                if let Err(e) =
                    forward_local_announcement(raw, &msg, &remote_ip, pip, &local_ip_str).await
                {
                    debug!("reverse SOOD proxy failed: {e}");
                } else {
                    metrics
                        .reverse_announcements
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            continue;
        }

        if !should_answer_query(&msg) {
            continue;
        }
        metrics.sood_queries.fetch_add(1, Ordering::Relaxed);

        let tid = msg
            .props
            .get("_tid")
            .and_then(|v| v.as_deref())
            .unwrap_or("*");
        let dest = msg.from;
        let query_sid = msg
            .props
            .get("query_service_id")
            .and_then(|v| v.as_deref())
            .unwrap_or("<none>");
        info!("SOOD query from {src}: tid={tid} query_service_id={query_sid}");
        for (k, v) in &msg.props {
            debug!("   query field: {k} = {v:?}");
        }

        let now = std::time::Instant::now();
        tid_cache.retain(|_, (_, t)| now.duration_since(*t) < Duration::from_secs(60));

        let resp_bytes = if let Some((cached, _)) = tid_cache.get(tid) {
            debug!(
                "reusing cached SOOD response for duplicate tid ({}B)",
                cached.len()
            );
            cached.clone()
        } else {
            let live_resp = proxy_query_to_core(&remote_ip, raw, cfg.live_proxy_timeout_ms).await;
            let (bytes, is_real) = match live_resp {
                Ok(real_pkt) => {
                    info!("Using live Core SOOD response ({}B)", real_pkt.len());
                    debug!("   live Core packet: {}", hex_dump(&real_pkt));
                    metrics.live_core_responses.fetch_add(1, Ordering::Relaxed);
                    (real_pkt, true)
                }
                Err(e) => {
                    debug!("live SOOD proxy failed; falling back to template: {e}");
                    metrics.template_fallbacks.fetch_add(1, Ordering::Relaxed);
                    let resp_res = if let Some(ref t) = template {
                        make_sood_response_from_template(tid, t, &local_ip_str, &cfg.name)
                    } else {
                        make_sood_response(tid, &cfg.name, &local_ip_str)
                    };
                    let r = match resp_res {
                        Ok(r) => r,
                        Err(e) => {
                            warn!("failed to build SOOD response packet: {e:#}");
                            continue;
                        }
                    };
                    (r, false)
                }
            };
            if is_real {
                tid_cache.insert(tid.to_string(), (bytes.clone(), now));
            }
            bytes
        };

        if let Err(e) = socket.send_to(&resp_bytes, dest).await {
            warn!("failed to send SOOD unicast response to {dest}: {e}");
        } else {
            metrics.sood_responses.fetch_add(1, Ordering::Relaxed);
            info!("SOOD response sent to {dest} ({}B)", resp_bytes.len());
            debug!("   response bytes: {}", hex_dump(&resp_bytes));
        }
    }
}

async fn pipe(mut r: io::ReadHalf<TcpStream>, mut w: io::WriteHalf<TcpStream>) -> u64 {
    let n = io::copy(&mut r, &mut w).await.unwrap_or(0);
    let _ = w.shutdown().await;
    n
}

async fn handle_tcp(
    client: TcpStream,
    remote: Arc<String>,
    port: u16,
    retry_secs: u64,
    metrics: Arc<RuntimeMetrics>,
) {
    let peer = client
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    info!("[TCP:{port}] ← {peer}");
    metrics.tcp_connections.fetch_add(1, Ordering::Relaxed);

    let upstream = loop {
        match TcpStream::connect(fmt_addr(&remote, port)).await {
            Ok(s) => {
                let _ = s.set_nodelay(true);
                let sock_ref = socket2::SockRef::from(&s);
                let _ = sock_ref.set_tcp_keepalive(
                    &socket2::TcpKeepalive::new()
                        .with_time(Duration::from_secs(30))
                        .with_interval(Duration::from_secs(10)),
                );
                break s;
            }
            Err(e) => {
                warn!(
                    "[TCP:{port}] failed to connect to upstream {}: {e}; retrying in {retry_secs}s",
                    *remote
                );
                sleep(Duration::from_secs(retry_secs)).await;
            }
        }
    };
    let _ = client.set_nodelay(true);
    {
        let sock_ref = socket2::SockRef::from(&client);
        let _ = sock_ref.set_tcp_keepalive(
            &socket2::TcpKeepalive::new()
                .with_time(Duration::from_secs(30))
                .with_interval(Duration::from_secs(10)),
        );
    }
    info!(
        "[TCP:{port}] tunnel established {peer} ↔ {}",
        fmt_addr(&remote, port)
    );

    let (cr, cw) = io::split(client);
    let (ur, uw) = io::split(upstream);
    let (down, up) = tokio::join!(pipe(ur, cw), pipe(cr, uw));
    metrics.tcp_down_bytes.fetch_add(down, Ordering::Relaxed);
    metrics.tcp_up_bytes.fetch_add(up, Ordering::Relaxed);
    info!("[TCP:{port}] disconnected {peer}; down={down}B up={up}B");
}

async fn run_tcp_forwarder(
    local_ip: String,
    port: u16,
    remote: Arc<String>,
    retry_secs: u64,
    metrics: Arc<RuntimeMetrics>,
) -> Result<()> {
    let bind = format!("{local_ip}:{port}");
    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("failed to listen on {bind}; port may be in use"))?;

    let desc = ROON_PORTS
        .iter()
        .find(|&&(p, _)| p == port)
        .map(|&(_, d)| d)
        .unwrap_or("custom");
    info!("TCP {bind} -> {}  [{desc}]", fmt_addr(&remote, port));

    loop {
        match listener.accept().await {
            Ok((sock, _)) => {
                let r = Arc::clone(&remote);
                let m = Arc::clone(&metrics);
                tokio::spawn(async move { handle_tcp(sock, r, port, retry_secs, m).await });
            }
            Err(e) => {
                error!("[TCP:{port}] accept error: {e}");
                sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "roon_beacon=info".into()))
        .without_time()
        .init();

    let args = Args::parse();

    if let Some(local_ip) = &args.local_ip {
        let address: Ipv4Addr = local_ip
            .parse()
            .context("--local-ip must be a valid IPv4 LAN address")?;
        if address.is_loopback()
            || address.is_unspecified()
            || address.is_multicast()
            || address.is_broadcast()
        {
            anyhow::bail!(
                "--local-ip must be a usable LAN address, not loopback, wildcard or multicast"
            );
        }
        if address.octets()[0] == 100 && (64..=127).contains(&address.octets()[1]) {
            warn!("--local-ip looks like a Tailscale address; use the LAN address visible to Roon Remote");
        }
    }
    let remote_ip_resolved = match &args.remote_ip {
        Some(host) => Some(resolve_remote_host(host, args.prefer).await?),
        None => None,
    };
    let store = persistence::SecureState::open(&args.data_dir)?;
    let auth = auth::WebAuth::open(args.web_bind, &args.data_dir, args.web_token.clone())?;
    let listener = TcpListener::bind(args.web_bind)
        .await
        .with_context(|| format!("cannot bind Web UI to {}", args.web_bind))?;
    let player = player::Player::new(store)?;
    let audio = audio::AudioGateway::start(
        args.enable_audio,
        args.audio_name.clone(),
        args.data_dir.clone(),
    )
    .await?;
    player
        .start_discovery(
            args.remote_ip.is_some(),
            remote_ip_resolved
                .as_ref()
                .and_then(|host| host.parse().ok()),
        )
        .await;
    let local_ip = args
        .local_ip
        .clone()
        .unwrap_or_else(|| "0.0.0.0".to_string());
    let runtime_config = Arc::new(RwLock::new(RuntimeConfig::from_args(&args)));
    let metrics = Arc::new(RuntimeMetrics::default());

    println!();
    println!("  Roon Beacon v{}", env!("CARGO_PKG_VERSION"));
    println!(
        "  Mode              : {}",
        if remote_ip_resolved.is_some() {
            "Bridge + Web player"
        } else {
            "Web player"
        }
    );
    if let Some(remote) = &remote_ip_resolved {
        println!("  Remote Core       : {remote}");
    }
    println!("  Local LAN IP      : {}", local_ip);
    println!("  Web UI            : http://{}", args.web_bind);
    println!("  Fallback Core Name: {}", args.name);
    println!("  SOOD Interval     : {}s", args.broadcast_interval);
    println!();
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    if let Some(remote_ip_resolved) = &remote_ip_resolved {
        let remote = Arc::new(remote_ip_resolved.clone());
        let template: Option<Arc<HashMap<String, String>>> = if args.no_probe {
            info!("Core probing disabled by --no-probe; using built-in fallback template");
            None
        } else {
            match probe_core_template(remote_ip_resolved, args.probe_timeout).await {
                Ok(tpl) => {
                    info!(
                        "Using real Core fields as fallback template ({} fields)",
                        tpl.len()
                    );
                    Some(Arc::new(tpl))
                }
                Err(e) => {
                    warn!("Core probe failed; falling back to built-in template: {e:#}");
                    None
                }
            }
        };

        {
            let lip = local_ip.clone();
            let use_bcast = args.use_broadcast;
            let tpl = template.clone();
            let rip = remote_ip_resolved.clone();
            let cfg = Arc::clone(&runtime_config);
            let m = Arc::clone(&metrics);
            tasks.push(tokio::spawn(async move {
                if let Err(e) = run_sood(lip, use_bcast, tpl, rip, cfg, m).await {
                    error!("SOOD service failed: {e:#}");
                    error!(
                    "If UDP {SOOD_PORT} is in use, inspect it with `sudo lsof -i UDP:{SOOD_PORT}`"
                );
                }
            }));
        }

        let skip: std::collections::HashSet<u16> = args.skip_ports.iter().copied().collect();
        for (port, desc) in ROON_PORTS {
            if skip.contains(port) {
                info!("Skipping TCP :{port} [{desc}] because of --skip-ports");
                continue;
            }
            match std::net::TcpListener::bind(format!("{}:{}", local_ip, port)) {
                Ok(l) => drop(l),
                Err(e) => {
                    warn!("Skipping TCP :{port} [{desc}] because the port is unavailable: {e}");
                    warn!(
                        "This is common when local Roon Bridge or RAATServer already owns the port"
                    );
                    continue;
                }
            }
            let lip = local_ip.clone();
            let r = Arc::clone(&remote);
            let port = *port;
            let retry = args.retry_secs;
            let m = Arc::clone(&metrics);
            tasks.push(tokio::spawn(async move {
                if let Err(e) = run_tcp_forwarder(lip, port, r, retry, m).await {
                    error!("TCP forwarder :{port} failed: {e:#}");
                }
            }));
        }
    }
    let state = WebState {
        config: Arc::clone(&runtime_config),
        metrics: Arc::clone(&metrics),
        started_at: Instant::now(),
        remote_ip: remote_ip_resolved,
        local_ip,
        use_broadcast: args.use_broadcast,
        web_bind: args.web_bind,
    };
    let mut web_task = tokio::spawn(run_web_ui(
        listener,
        state,
        player.clone(),
        audio.clone(),
        auth,
    ));
    info!(
        "Running; enable Roon Beacon in Roon Settings → Extensions to authorize playback control"
    );
    let outcome = tokio::select! {
        result = shutdown_signal() => result,
        result = &mut web_task => Err(anyhow::anyhow!("Web server stopped unexpectedly: {result:?}")),
    };
    info!("Shutdown requested; stopping services");
    web_task.abort();
    for task in tasks {
        task.abort();
    }
    player.shutdown().await;
    audio.shutdown().await;
    outcome
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.context("cannot listen for Ctrl+C"),
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c()
        .await
        .context("cannot listen for Ctrl+C")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop<'a>(msg: &'a SoodMessage, key: &str) -> Option<&'a str> {
        msg.props.get(key).and_then(|v| v.as_deref())
    }

    #[test]
    fn normalizes_optional_proxy_ip() {
        assert_eq!(normalized_optional_string(None), None);
        assert_eq!(normalized_optional_string(Some("   ".to_string())), None);
        assert_eq!(
            normalized_optional_string(Some("  100.64.0.10  ".to_string())),
            Some("100.64.0.10".to_string())
        );
    }

    #[test]
    fn embedded_web_ui_uses_split_assets() {
        assert!(WEB_UI_HTML.contains(r#"href="/assets/styles.css""#));
        assert!(WEB_UI_HTML.contains(r#"src="/assets/app.js""#));
        assert!(WEB_UI_HTML.contains(r#"href="/assets/favicon.png""#));
        assert!(WEB_UI_HTML.contains(r#"href="/assets/apple-touch-icon.png""#));
        assert!(!WEB_UI_CSS.is_empty());
        assert!(WEB_UI_HTML.contains(r#"type="module""#));
        assert!(!WEB_UI_JS.is_empty());
    }

    #[test]
    fn supplied_branding_is_embedded_as_transparent_png_assets() {
        for (bytes, dimension) in [(WEB_LOGO, 256u32), (WEB_FAVICON, 64), (WEB_TOUCH_ICON, 180)] {
            assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            assert_eq!(
                u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
                dimension
            );
            assert_eq!(
                u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
                dimension
            );
            assert_eq!(bytes[25], 6);
        }
    }

    #[test]
    fn parses_binary_sood_query() {
        let mut props = HashMap::new();
        props.insert("_tid".to_string(), Some("42".to_string()));
        props.insert(
            "query_service_id".to_string(),
            Some(ROON_CORE_SERVICE_ID.to_string()),
        );
        let packet = serialize_query(&props);
        let src: SocketAddr = "192.168.1.50:55000".parse().unwrap();

        let msg = roon_sood::parse(&packet, src).unwrap();

        assert_eq!(msg.msg_type, SoodType::Query);
        assert_eq!(msg.props.get("_tid").and_then(|v| v.as_deref()), Some("42"));
        assert_eq!(
            msg.props.get("query_service_id").and_then(|v| v.as_deref()),
            Some(ROON_CORE_SERVICE_ID)
        );
        assert_eq!(msg.from, src);
        assert!(should_answer_query(&msg));
    }

    #[test]
    fn honors_sood_reply_override() {
        let mut props = HashMap::new();
        props.insert("_tid".to_string(), Some("43".to_string()));
        props.insert(
            "query_service_id".to_string(),
            Some(ROON_CORE_SERVICE_ID.to_string()),
        );
        props.insert("_replyaddr".to_string(), Some("192.168.1.60".to_string()));
        props.insert("_replyport".to_string(), Some("9003".to_string()));
        let packet = serialize_query(&props);
        let src: SocketAddr = "192.168.1.50:55000".parse().unwrap();
        let expected: SocketAddr = "192.168.1.60:9003".parse().unwrap();

        let msg = roon_sood::parse(&packet, src).unwrap();

        assert_eq!(msg.from, expected);
        // _replyaddr / _replyport should be removed from props by parse
        assert!(!msg.props.contains_key("_replyaddr"));
        assert!(!msg.props.contains_key("_replyport"));
    }

    #[test]
    fn filters_other_sood_services() {
        let mut props = HashMap::new();
        props.insert("_tid".to_string(), Some("44".to_string()));
        props.insert(
            "query_service_id".to_string(),
            Some("not-roon-core".to_string()),
        );
        let packet = serialize_query(&props);
        let src: SocketAddr = "192.168.1.50:55000".parse().unwrap();

        let msg = roon_sood::parse(&packet, src).unwrap();

        assert!(!should_answer_query(&msg));
    }

    #[test]
    fn builds_core_response_on_roon_discovery_port() {
        let packet = make_sood_response("45", "Roon Core (Beacon)", "192.168.1.10").unwrap();
        let src: SocketAddr = "192.168.1.50:55000".parse().unwrap();

        let msg = roon_sood::parse(&packet, src).unwrap();

        assert_eq!(msg.msg_type, SoodType::Response);
        assert_eq!(prop(&msg, "_tid"), Some("45"));
        assert_eq!(prop(&msg, "service_id"), Some(ROON_CORE_SERVICE_ID));
        assert_eq!(prop(&msg, "unique_id"), Some("roon-beacon-192-168-1-10"));
        assert_eq!(prop(&msg, "display_name"), Some("Roon Core (Beacon)"));
        assert_eq!(prop(&msg, "http_port"), Some("9330"));
    }

    #[test]
    fn rejects_old_json_discovery_shape() {
        let src: SocketAddr = "192.168.1.50:55000".parse().unwrap();
        let json = br#"{"_type":"SOOD","_tid":"46"}"#;

        assert!(roon_sood::parse(json, src).is_err());
    }
}
