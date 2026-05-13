//! roon-beacon
//!
//! 在本地局域网运行，把远端 NAS 上的 Roon Core 桥接过来。
//!
//! 核心问题：Roon 用私有 SOOD 协议发现 Core，必须在正确的局域网网卡上
//! 监听和响应，Tailscale 虚拟网卡不参与 SOOD。
//!
//! 架构：
//!   本地 Roon Remote
//!     │ SOOD UDP 239.255.90.90:9003
//!     ▼
//!   roon-beacon（绑定局域网网卡 192.168.1.x）
//!     │  SOOD 响应：告诉 Remote "Core 在 192.168.1.x:9100"
//!     │  TCP 监听：192.168.1.x:9100 等端口
//!     │
//!     │ Tailscale 隧道（100.x.x.x）
//!     ▼
//!   NAS Roon Core（10.10.68.x 或任意网段）

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum AddrFamily {
    /// 优先 IPv4
    V4,
    /// 优先 IPv6
    V6,
    /// 使用第一个解析到的地址
    Any,
}
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::time::{interval, sleep, Duration};
use tracing::{debug, error, info, warn};

use roon_sood::{
    self, serialize_query, serialize_response, SoodMessage, SoodType, ROON_CORE_SERVICE_ID,
    SOOD_PORT,
};

// ── 常量 ──────────────────────────────────────────────────────────────────────

const SOOD_MCAST: Ipv4Addr = Ipv4Addr::new(239, 255, 90, 90);
const ROON_DISCOVERY_PORT: u16 = 9330;

const ROON_PORTS: &[(u16, &str)] = &[
    (9100, "RAAT / legacy"),
    (9101, "RAAT Alt"),
    (9150, "Roon Core TCP"),
    (9200, "Roon Web Display"),
    (9330, "Roon Core API / Web Display"),
    (9331, "Roon Core API 备用"),
    (9332, "Roon Web Display HTTPS"),
    (55000, "Roon Core HTTPS"),
];

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Roon Beacon —— 将远端 NAS 的 Roon Core 以 SOOD 信标的形式广播到本地局域网
///
/// 关键：--local-ip 必须填本机【局域网】IP（客户端所在网段，如 192.168.1.10），
///       不要填 VPN / overlay 网络的 IP，也不能填 127.0.0.1
#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// 远端 NAS 的地址，支持三种写法：
    ///   • IP 地址（推荐）: 例如 100.107.146.8
    ///   • 主机名: 例如 lazycat
    ///   • 完整域名: 例如 nas.example.com
    #[arg(short = 'r', long, value_name = "HOST")]
    remote_ip: String,

    /// 【必填】本机局域网 IP（与 Roon Remote 所在的网段一致）
    /// 用 `ip addr` 或 `ifconfig` 查看，选客户端所在的网段那个
    /// 不要填 VPN / overlay 网络的 IP（例如 100.x.x.x）
    #[arg(short = 'l', long, value_name = "IP")]
    local_ip: String,

    /// Roon Remote 里显示的 Core 名称
    #[arg(short = 'n', long, default_value = "Roon Core (Beacon)")]
    name: String,

    /// 主动广播 SOOD 心跳间隔（秒），0 = 禁用
    #[arg(long, default_value_t = 10)]
    broadcast_interval: u64,

    /// 使用 UDP 广播（255.255.255.255）代替多播，适配不支持 IGMP 的路由器
    #[arg(long)]
    use_broadcast: bool,

    /// 连接远端失败后的重试间隔（秒）
    #[arg(long, default_value_t = 3)]
    retry_secs: u64,

    /// 域名解析时的地址族偏好（当 --remote-ip 是域名/主机名时生效）
    ///   v4 = 优先 IPv4（推荐）
    ///   v6 = 优先 IPv6
    ///   any = 使用第一个解析到的地址
    #[arg(long, value_enum, default_value_t = AddrFamily::V4)]
    prefer: AddrFamily,

    /// 跳过某些端口（本机已被其他程序占用，比如本地 Roon Bridge/RAATServer 占用 9200）
    /// 用逗号分隔：--skip-ports 9200,9100
    #[arg(long, value_name = "PORTS", value_delimiter = ',')]
    skip_ports: Vec<u16>,

    /// 反向 SOOD 代理：把本地 RAAT Bridge / Output 的多播广告转发给远端 Core
    /// 让 Core 在音频设备列表看到本地设备（广告里的 IP 改写为 --proxy-ip 指定的地址）
    #[arg(long)]
    reverse_proxy: bool,

    /// 反向代理时改写广告用的【桥接对外可达 IP】（Core 会来这个 IP 建立 RAAT 连接）
    /// 通常是桥接在 overlay 网络里的 IP（例如 VPN / Tailscale 分配的地址）
    #[arg(long, value_name = "IP", alias = "tailscale-ip")]
    proxy_ip: Option<String>,

    /// 禁用从远端 Core 探测真实 SOOD 字段（遇到问题时用）
    #[arg(long)]
    no_probe: bool,

    /// 探测远端 Core 的超时时间（秒）
    #[arg(long, default_value_t = 3)]
    probe_timeout: u64,
}

// ── 工具函数 ─────────────────────────────────────────────────────────────────

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
    // 附加 ASCII
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

/// 通过 Tailscale 单播向远端 Roon Core 发送 SOOD 查询，获取真实响应模板
async fn probe_core_template(
    remote_ip: &str,
    timeout_secs: u64,
) -> Result<HashMap<String, String>> {
    use tokio::time::timeout;

    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("无法创建探测 socket")?;
    sock.connect(fmt_addr(remote_ip, SOOD_PORT))
        .await
        .with_context(|| format!("无法连接到 {}", fmt_addr(remote_ip, SOOD_PORT)))?;

    let queries = [
        (
            "d52b2cb7-02c5-48fc-981b-a10f0aadd93b",
            "ROON_SERVER",
        ),
        (ROON_CORE_SERVICE_ID, "ROON_API"),
    ];

    for (sid, label) in &queries {
        let tid = uuid::Uuid::new_v4().to_string();
        let mut props = HashMap::new();
        props.insert("_tid".to_string(), Some(tid));
        props.insert("query_service_id".to_string(), Some(sid.to_string()));
        let pkt = serialize_query(&props);

        info!("🔎 探测远端 Core ({label}) → {}", fmt_addr(remote_ip, SOOD_PORT));
        sock.send(&pkt).await.context("发送探测包失败")?;

        let mut buf = vec![0u8; 4096];
        match timeout(Duration::from_secs(timeout_secs), sock.recv(&mut buf)).await {
            Ok(Ok(n)) => {
                let dummy_src = SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0));
                match roon_sood::parse(&buf[..n], dummy_src) {
                    Ok(msg) if msg.msg_type == SoodType::Response => {
                        info!("✅ 收到 Core 响应（{label}），字段数: {}", msg.props.len());
                        for (k, v) in &msg.props {
                            debug!("   Core 字段: {k} = {v:?}");
                        }
                        let flat: HashMap<String, String> = msg
                            .props
                            .into_iter()
                            .filter_map(|(k, v)| v.map(|vv| (k, vv)))
                            .collect();
                        return Ok(flat);
                    }
                    Ok(_) => warn!("收到的包不是有效的 SOOD 响应"),
                    Err(e) => warn!("解析 SOOD 响应失败: {e}"),
                }
            }
            Ok(Err(e)) => warn!("探测 recv 错误: {e}"),
            Err(_) => warn!("探测超时（{timeout_secs}s），尝试下一个 service_id"),
        }
    }

    anyhow::bail!(
        "无法从 {remote_ip} 探测到 Roon Core 的 SOOD 响应\n\
         → 检查：1) 网络是否可达  2) NAS 上 Roon Server 是否运行\n\
         → 验证：在 NAS 上运行 `ss -ulnp | grep 9003`",
        remote_ip = remote_ip
    )
}

/// 把手机发来的 SOOD 查询**原样**透传到 NAS Core 取真实响应
async fn proxy_query_to_core(
    remote_ip: &str,
    phone_query_raw: &[u8],
    _timeout_secs: u64,
) -> Result<Vec<u8>> {
    use tokio::time::timeout;

    let sock = UdpSocket::bind("0.0.0.0:0")
        .await
        .context("proxy socket bind 失败")?;
    sock.connect(fmt_addr(remote_ip, SOOD_PORT))
        .await
        .context("proxy connect 失败")?;
    sock.send(phone_query_raw).await.context("proxy send 失败")?;

    let mut buf = vec![0u8; 4096];
    let n = timeout(
        Duration::from_millis(800),
        sock.recv(&mut buf),
    )
    .await
    .context("proxy recv 超时（Core 未响应）")?
    .context("proxy recv 失败")?;

    Ok(buf[..n].to_vec())
}

/// 解析 --remote-ip 支持的三种输入
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
                "无法解析主机名 '{host}'：\n\
                 → 可以先用 `ping {host}` 或 `getent ahosts {host}` 测试\n\
                 → 如果是内网域名，确认 DNS 配置"
            )
        })?
        .collect();

    if addrs.is_empty() {
        anyhow::bail!("'{host}' 解析后没有 IP 地址");
    }

    let chosen = match prefer {
        AddrFamily::V4 => {
            if let Some(a) = addrs.iter().find(|a| a.is_ipv4()) {
                Some(*a)
            } else {
                tracing::warn!("⚠️  --prefer v4 但 '{host}' 没有 IPv4 记录，回退到 IPv6");
                addrs.first().copied()
            }
        }
        AddrFamily::V6 => {
            if let Some(a) = addrs.iter().find(|a| a.is_ipv6()) {
                Some(*a)
            } else {
                tracing::warn!("⚠️  --prefer v6 但 '{host}' 没有 IPv6 记录，回退到 IPv4");
                addrs.first().copied()
            }
        }
        AddrFamily::Any => addrs.first().copied(),
    };

    chosen
        .map(|a| a.ip().to_string())
        .ok_or_else(|| anyhow::anyhow!("'{host}' 解析后没有可用地址"))
}

/// 把 IP + 端口拼成合法的 host:port 字符串（IPv6 加方括号）
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

/// 反向 SOOD 代理：本机 RAAT Bridge / Output 在 239.255.90.90 发的广告 → 改写后单播给远端 Core
async fn forward_local_announcement(
    raw: &[u8],
    msg: &SoodMessage,
    remote_ip: &str,
    proxy_ip: &str,
    local_ip: &str,
) -> Result<()> {
    // 不转发 Core 自己的广告（避免回环）
    if let Some(Some(sid)) = msg.props.get("service_id") {
        if sid == ROON_CORE_SERVICE_ID {
            return Ok(());
        }
    }
    // 不转发桥接自己的响应
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

    // 改写 IP 字段
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

    let sock = UdpSocket::bind("0.0.0.0:0").await.context("bind 失败")?;
    sock.send_to(&new_pkt, fmt_addr(remote_ip, SOOD_PORT))
        .await
        .context("反向代理 send 失败")?;

    let dn = msg
        .props
        .get("display_name")
        .and_then(|v| v.as_deref())
        .or(msg.props.get("name").and_then(|v| v.as_deref()))
        .unwrap_or("?");
    info!("🔁 反向广告 → Core ({remote_ip}): {dn}");
    Ok(())
}

/// 在 SOOD 包里把所有 IP 字段值替换为新 IP（处理长度不同的情况）
fn rebuild_sood_pkt_with_ip_replaced(
    pkt: &[u8],
    old_ip: &str,
    new_ip: &str,
) -> Result<Vec<u8>> {
    if pkt.len() < 7 || &pkt[..4] != b"SOOD" {
        anyhow::bail!("不是 SOOD 包");
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

/// 用模板字段构造响应：保留 Core 的所有字段，只改写 IP/端口
fn make_sood_response_from_template(
    tid: &str,
    template: &HashMap<String, String>,
    local_ip: &str,
    _name: &str,
) -> Result<Vec<u8>> {
    let http_port_str = ROON_DISCOVERY_PORT.to_string();
    let mut props: HashMap<String, Option<String>> = HashMap::new();

    // _tid 总是用查询的 tid
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

    match msg
        .props
        .get("query_service_id")
        .and_then(|v| v.as_deref())
    {
        Some(service_id) => service_id == ROON_CORE_SERVICE_ID,
        None => true,
    }
}

/// 创建 SOOD UDP socket，绑定到指定局域网网卡
async fn make_sood_socket(local_ip_v4: Ipv4Addr, use_broadcast: bool) -> Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};

    let sock = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))
        .context("创建 UDP socket 失败")?;

    sock.set_reuse_address(true).ok();

    if use_broadcast {
        sock.set_broadcast(true).ok();
    }

    sock.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, SOOD_PORT).into())
        .with_context(|| format!("绑定 UDP 0.0.0.0:{SOOD_PORT} 失败（被占用？需要 sudo？）"))?;

    if !use_broadcast {
        sock.join_multicast_v4(&SOOD_MCAST, &local_ip_v4)
            .with_context(|| {
                format!(
                    "加入多播组 {SOOD_MCAST} 失败（接口 {local_ip_v4}）\n\
                     → 请确认 --local-ip 是局域网 IP（如 192.168.1.x），不是 overlay 网络 IP"
                )
            })?;

        sock.set_multicast_if_v4(&local_ip_v4)
            .context("set_multicast_if_v4 失败")?;

        sock.set_multicast_loop_v4(true).ok();
    }

    let std_sock: std::net::UdpSocket = sock.into();
    std_sock.set_nonblocking(true)?;
    UdpSocket::from_std(std_sock).context("转换 tokio UdpSocket 失败")
}

/// SOOD 服务：监听查询、响应、主动广播
async fn run_sood(
    local_ip_str: String,
    name: String,
    broadcast_interval_secs: u64,
    use_broadcast: bool,
    template: Option<Arc<HashMap<String, String>>>,
    remote_ip: String,
    probe_timeout: u64,
    reverse_proxy: bool,
    proxy_ip: Option<String>,
) -> Result<()> {
    let local_ip_v4: Ipv4Addr = local_ip_str
        .parse()
        .context("--local-ip 必须是合法的 IPv4 地址")?;

    let socket = Arc::new(
        make_sood_socket(local_ip_v4, use_broadcast)
            .await
            .context("SOOD socket 初始化失败")?,
    );

    let mode = if use_broadcast { "广播" } else { "多播" };
    info!("📡 SOOD 监听 0.0.0.0:{SOOD_PORT}，{mode} 网卡: {local_ip_str}");
    info!("   Roon Remote 查询时将回复：Core 在 {local_ip_str}:{ROON_DISCOVERY_PORT}");

    // ── 主动广播任务 ──
    if broadcast_interval_secs > 0 {
        let sock2 = Arc::clone(&socket);
        let lip = local_ip_str.clone();
        let n = name.clone();
        let use_broadcast = use_broadcast;
        let tpl = template.clone();
        tokio::spawn(async move {
            sleep(Duration::from_millis(500)).await;
            let mut ticker = interval(Duration::from_secs(broadcast_interval_secs));
            loop {
                ticker.tick().await;
                let pkt_res = if let Some(ref t) = tpl {
                    make_sood_response_from_template("*", t, &lip, &n)
                } else {
                    make_sood_response("*", &n, &lip)
                };
                let pkt = match pkt_res {
                    Ok(pkt) => pkt,
                    Err(e) => {
                        warn!("SOOD 广播包构造失败: {e:#}");
                        continue;
                    }
                };
                let dest = if use_broadcast {
                    SocketAddr::from((Ipv4Addr::new(255, 255, 255, 255), SOOD_PORT))
                } else {
                    SocketAddr::from((SOOD_MCAST, SOOD_PORT))
                };
                match sock2.send_to(&pkt, dest).await {
                    Ok(_) => info!(
                        "📡 SOOD {} → {}",
                        if use_broadcast { "广播" } else { "多播" },
                        dest
                    ),
                    Err(e) => warn!("SOOD 广播失败: {e}"),
                }
            }
        });
    }

    // ── 监听并响应查询 ──
    let mut tid_cache: HashMap<String, (Vec<u8>, std::time::Instant)> = HashMap::new();
    let mut buf = vec![0u8; 4096];
    loop {
        let (len, src) = match socket.recv_from(&mut buf).await {
            Ok(x) => x,
            Err(e) => {
                warn!("SOOD recv 错误: {e}");
                continue;
            }
        };

        let raw = &buf[..len];
        debug!("SOOD 收到 from {src}: {len}B");

        let msg = match roon_sood::parse(raw, src) {
            Ok(msg) => msg,
            Err(e) => {
                debug!("SOOD 解析失败: {e}");
                continue;
            }
        };

        // ── 反向 SOOD 代理：本机 RAAT Bridge / Output 发的广告（Response）转给 Core ──
        if reverse_proxy && msg.msg_type == SoodType::Response {
            if let Some(ref pip) = proxy_ip {
                if let Err(e) =
                    forward_local_announcement(raw, &msg, &remote_ip, pip, &local_ip_str).await
                {
                    debug!("反向代理失败: {e}");
                }
            }
            continue;
        }

        // 只响应 Roon Core service 查询
        if !should_answer_query(&msg) {
            continue;
        }

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
        info!("🔍 SOOD 查询 from {src} tid={tid} query_service_id={query_sid}");
        for (k, v) in &msg.props {
            debug!("   查询字段: {k} = {v:?}");
        }

        // 缓存检查
        let now = std::time::Instant::now();
        tid_cache.retain(|_, (_, t)| now.duration_since(*t) < Duration::from_secs(60));

        let resp_bytes = if let Some((cached, _)) = tid_cache.get(tid) {
            debug!("♻️  同 tid 复用缓存 ({}B)", cached.len());
            cached.clone()
        } else {
            let live_resp = proxy_query_to_core(&remote_ip, raw, probe_timeout).await;
            let (bytes, is_real) = match live_resp {
                Ok(real_pkt) => {
                    info!("⚡ 使用 Core 实时真包响应 ({}B)", real_pkt.len());
                    debug!("   Core 真包: {}", hex_dump(&real_pkt));
                    (real_pkt, true)
                }
                Err(e) => {
                    debug!("Core 透传失败，退回模板: {e}");
                    let resp_res = if let Some(ref t) = template {
                        make_sood_response_from_template(tid, t, &local_ip_str, &name)
                    } else {
                        make_sood_response(tid, &name, &local_ip_str)
                    };
                    let r = match resp_res {
                        Ok(r) => r,
                        Err(e) => {
                            warn!("SOOD 响应包构造失败: {e:#}");
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
            warn!("SOOD 单播响应失败 → {dest}: {e}");
        } else {
            info!("✅ SOOD 响应 → {dest}  ({}B)", resp_bytes.len());
            debug!("   响应字节: {}", hex_dump(&resp_bytes));
        }
    }
}

// ── TCP 转发 ──────────────────────────────────────────────────────────────────

async fn pipe(mut r: io::ReadHalf<TcpStream>, mut w: io::WriteHalf<TcpStream>) -> u64 {
    let n = io::copy(&mut r, &mut w).await.unwrap_or(0);
    let _ = w.shutdown().await;
    n
}

async fn handle_tcp(client: TcpStream, remote: Arc<String>, port: u16, retry_secs: u64) {
    let peer = client.peer_addr().map(|a| a.to_string()).unwrap_or_default();
    info!("[TCP:{port}] ← {peer}");

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
                warn!("[TCP:{port}] 连不上远端 {}: {e}，{retry_secs}s 后重试", *remote);
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
    info!("[TCP:{port}] 隧道建立 {peer} ↔ {}", fmt_addr(&remote, port));

    let (cr, cw) = io::split(client);
    let (ur, uw) = io::split(upstream);
    let (down, up) = tokio::join!(pipe(ur, cw), pipe(cr, uw));
    info!("[TCP:{port}] 断开 {peer} ↓{down}B ↑{up}B");
}

async fn run_tcp_forwarder(
    local_ip: String,
    port: u16,
    remote: Arc<String>,
    retry_secs: u64,
) -> Result<()> {
    let bind = format!("{local_ip}:{port}");
    let listener = TcpListener::bind(&bind)
        .await
        .with_context(|| format!("无法监听 {bind}（端口被占用？）"))?;

    let desc = ROON_PORTS
        .iter()
        .find(|&&(p, _)| p == port)
        .map(|&(_, d)| d)
        .unwrap_or("custom");
    info!("✓ TCP {bind} → {}  [{desc}]", fmt_addr(&remote, port));

    loop {
        match listener.accept().await {
            Ok((sock, _)) => {
                let r = Arc::clone(&remote);
                tokio::spawn(async move { handle_tcp(sock, r, port, retry_secs).await });
            }
            Err(e) => {
                error!("[TCP:{port}] accept 错误: {e}");
                sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

// ── 主程序 ────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "roon_beacon=info".into()),
        )
        .without_time()
        .init();

    let args = Args::parse();

    let local_ip_v4: Ipv4Addr = args
        .local_ip
        .parse()
        .with_context(|| format!("--local-ip '{}' 不是合法的 IPv4 地址", args.local_ip))?;

    let first_octet = local_ip_v4.octets()[0];
    let second_octet = local_ip_v4.octets()[1];
    if first_octet == 100 && (64..=127).contains(&second_octet) {
        warn!("⚠️  检测到 --local-ip 是 Tailscale IP（100.64.x.x）");
        warn!("   SOOD 广播需要绑定到局域网网卡（如 192.168.x.x），否则 Roon Remote 发现不了");
        warn!("   请用 `ip addr` 或 `ifconfig` 查看局域网 IP 后重新运行");
    }
    if local_ip_v4.is_loopback() {
        anyhow::bail!("--local-ip 不能是 127.0.0.1，需要局域网 IP");
    }

    let remote_ip_resolved = resolve_remote_host(&args.remote_ip, args.prefer).await?;
    if remote_ip_resolved != args.remote_ip {
        info!(
            "🌐 域名解析: {} → {}  (prefer={:?})",
            args.remote_ip, remote_ip_resolved, args.prefer
        );
    }

    let remote = Arc::new(remote_ip_resolved.clone());
    let local_ip = args.local_ip.clone();

    println!();
    println!("  ╔═══════════════════════════════════════════════════════╗");
    println!(
        "  ║             🎵  Roon Beacon  v{}                    ║",
        env!("CARGO_PKG_VERSION")
    );
    println!("  ╠═══════════════════════════════════════════════════════╣");
    println!(
        "  ║  远端 NAS             : {:<32}║",
        &remote_ip_resolved
    );
    println!("  ║  本机局域网 IP        : {:<32}║", local_ip);
    println!("  ║  Core 显示名称        : {:<32}║", args.name);
    println!(
        "  ║  SOOD 广播间隔        : {:<32}║",
        format!("{}s", args.broadcast_interval)
    );
    println!("  ╚═══════════════════════════════════════════════════════╝");
    println!();
    println!("  ℹ️  Roon Remote 发现流程：");
    println!(
        "     Remote 发 SOOD 查询 → 本程序回复 \"Core 在 {}:{}\"",
        local_ip, ROON_DISCOVERY_PORT
    );
    println!(
        "     Remote 连接 {}:{} → 本程序转发到 {}:{}",
        local_ip, ROON_DISCOVERY_PORT, &remote_ip_resolved, ROON_DISCOVERY_PORT
    );
    println!();

    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    // 启动前：尝试从远端 Core 探测真实 SOOD 字段作为响应模板
    let template: Option<Arc<HashMap<String, String>>> = if args.no_probe {
        info!("⏭  已禁用 Core 探测（--no-probe），使用内置字段模板");
        None
    } else {
        match probe_core_template(&remote_ip_resolved, args.probe_timeout).await {
            Ok(tpl) => {
                info!("🎯 将使用真实 Core 字段作为响应模板（{}个字段）", tpl.len());
                Some(Arc::new(tpl))
            }
            Err(e) => {
                warn!("⚠️  探测 Core 失败，退回使用内置模板: {e:#}");
                None
            }
        }
    };

    // SOOD 发现服务
    {
        let lip = local_ip.clone();
        let name = args.name.clone();
        let bcast = args.broadcast_interval;
        let use_bcast = args.use_broadcast;
        let tpl = template.clone();
        let rip = remote_ip_resolved.clone();
        let ptimeout = args.probe_timeout;
        let rp = args.reverse_proxy;
        let tsip = args.proxy_ip.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = run_sood(
                lip, name, bcast, use_bcast, tpl, rip, ptimeout, rp, tsip,
            )
            .await
            {
                error!("❌ SOOD 启动失败: {e:#}");
                error!("   如果是权限问题：macOS/Linux 请加 sudo，Windows 请以管理员运行");
                error!("   如果端口 9003 被占用：sudo lsof -i UDP:9003");
            }
        }));
    }

    // TCP 转发（所有 Roon 端口）
    let skip: std::collections::HashSet<u16> = args.skip_ports.iter().copied().collect();
    for (port, desc) in ROON_PORTS {
        if skip.contains(port) {
            info!("⏭  跳过 TCP :{port}  [{desc}]（--skip-ports）");
            continue;
        }
        match std::net::TcpListener::bind(format!("{}:{}", local_ip, port)) {
            Ok(l) => drop(l),
            Err(e) => {
                warn!("⏭  跳过 TCP :{port}  [{desc}]（已被占用: {e}）");
                warn!("   这通常是本机 Roon Bridge / RAATServer 在用，正常现象");
                continue;
            }
        }
        let lip = local_ip.clone();
        let r = Arc::clone(&remote);
        let port = *port;
        let retry = args.retry_secs;
        tasks.push(tokio::spawn(async move {
            if let Err(e) = run_tcp_forwarder(lip, port, r, retry).await {
                error!("❌ TCP 转发器 :{port} 失败: {e:#}");
            }
        }));
    }

    info!("🚀 运行中（Ctrl+C 退出）");
    info!("   等待 Roon Remote 扫描...");
    info!(
        "   若 30s 内未发现：Roon Remote → 设置 → Roon Core → 手动连接 → {}",
        local_ip
    );
    println!();

    for t in tasks {
        let _ = t.await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop<'a>(msg: &'a SoodMessage, key: &str) -> Option<&'a str> {
        msg.props.get(key).and_then(|v| v.as_deref())
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
        assert_eq!(
            msg.props.get("_tid").and_then(|v| v.as_deref()),
            Some("42")
        );
        assert_eq!(
            msg.props
                .get("query_service_id")
                .and_then(|v| v.as_deref()),
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
        let packet =
            make_sood_response("45", "Roon Core (Beacon)", "192.168.1.10").unwrap();
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
