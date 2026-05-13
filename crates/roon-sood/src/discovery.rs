use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;

use tokio::net::UdpSocket;
use tokio::sync::{broadcast, watch};

use crate::{ROON_CORE_SERVICE_ID, SOOD_MULTICAST_IP, SOOD_PORT, SoodType, parse, serialize_query};

/// Information about a discovered Roon Core on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredCore {
    /// Unique identifier for this Roon Core instance.
    pub core_id: String,
    /// IP address to connect to (localhost-corrected if applicable).
    pub host: IpAddr,
    /// TCP port for the MOO/WebSocket API endpoint.
    pub http_port: u16,
    /// Human-readable core name broadcast in the SOOD response (e.g., "home"),
    /// if present. Roon Core always includes this in practice but the SOOD
    /// spec marks it as optional.
    pub name: Option<String>,
    /// Display version of the core (e.g., "2.65 (build 1648) earlyaccess"),
    /// if present in the SOOD response.
    pub display_version: Option<String>,
}

/// SOOD network discovery for Roon Cores.
pub struct SoodDiscovery {
    cancel_tx: watch::Sender<bool>,
    paired_tx: watch::Sender<bool>,
    task_handle: tokio::task::JoinHandle<()>,
}

impl SoodDiscovery {
    /// Start SOOD discovery.
    pub async fn start() -> Result<(Self, broadcast::Receiver<DiscoveredCore>), crate::SoodError> {
        let (core_tx, core_rx) = broadcast::channel::<DiscoveredCore>(16);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let (paired_tx, paired_rx) = watch::channel(false);

        let recv_socket = bind_recv_socket().await?;
        let send_socket = bind_send_socket().await?;

        let task_handle = tokio::spawn(discovery_loop(
            recv_socket,
            send_socket,
            core_tx,
            cancel_rx,
            paired_rx,
        ));

        Ok((
            SoodDiscovery {
                cancel_tx,
                paired_tx,
                task_handle,
            },
            core_rx,
        ))
    }

    /// Suppress periodic queries when paired (saves network traffic).
    pub fn set_paired(&self, paired: bool) {
        let _ = self.paired_tx.send(paired);
    }

    /// Stop discovery and release network resources.
    pub async fn stop(self) {
        let _ = self.cancel_tx.send(true);
        let _ = self.task_handle.await;
    }
}

async fn bind_recv_socket() -> Result<UdpSocket, crate::SoodError> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    socket
        .set_reuse_address(true)
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    #[cfg(target_os = "macos")]
    socket
        .set_reuse_port(true)
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    socket
        .set_nonblocking(true)
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, SOOD_PORT);
    socket
        .bind(&addr.into())
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    let multicast_addr: Ipv4Addr = SOOD_MULTICAST_IP
        .parse()
        .expect("hardcoded multicast IP is valid");
    socket
        .join_multicast_v4(&multicast_addr, &Ipv4Addr::UNSPECIFIED)
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    let std_socket: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(std_socket).map_err(|e| crate::SoodError::Io(e.to_string()))
}

async fn bind_send_socket() -> Result<UdpSocket, crate::SoodError> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    socket
        .set_broadcast(true)
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    socket
        .set_multicast_ttl_v4(1)
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    socket
        .set_nonblocking(true)
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    let addr = SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0);
    socket
        .bind(&addr.into())
        .map_err(|e| crate::SoodError::Io(e.to_string()))?;

    let std_socket: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(std_socket).map_err(|e| crate::SoodError::Io(e.to_string()))
}

fn build_query_packet() -> Vec<u8> {
    let mut props = HashMap::new();
    props.insert(
        "query_service_id".to_string(),
        Some(ROON_CORE_SERVICE_ID.to_string()),
    );
    props.insert("_tid".to_string(), Some(uuid::Uuid::new_v4().to_string()));
    serialize_query(&props)
}

/// Get all local IPv4 addresses for localhost detection.
fn get_local_ipv4_addrs() -> HashSet<IpAddr> {
    let mut addrs = HashSet::new();
    addrs.insert(IpAddr::V4(Ipv4Addr::LOCALHOST));

    // Read from /proc/net/fib_trie or fall back to parsing ip addr
    if let Ok(output) = std::process::Command::new("hostname").arg("-I").output()
        && let Ok(stdout) = std::str::from_utf8(&output.stdout)
    {
        for part in stdout.split_whitespace() {
            if let Ok(ip) = part.parse::<IpAddr>() {
                addrs.insert(ip);
            }
        }
    }

    addrs
}

async fn discovery_loop(
    recv_socket: UdpSocket,
    send_socket: UdpSocket,
    core_tx: broadcast::Sender<DiscoveredCore>,
    mut cancel_rx: watch::Receiver<bool>,
    paired_rx: watch::Receiver<bool>,
) {
    let multicast_target: SocketAddr = SocketAddr::V4(SocketAddrV4::new(
        SOOD_MULTICAST_IP.parse().unwrap(),
        SOOD_PORT,
    ));
    let broadcast_target: SocketAddr =
        SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::BROADCAST, SOOD_PORT));

    // Cache local addresses, refresh periodically
    let local_addrs = Arc::new(tokio::sync::Mutex::new(get_local_ipv4_addrs()));

    send_query(&send_socket, &multicast_target, &broadcast_target).await;

    let mut scan_interval = tokio::time::interval(std::time::Duration::from_secs(10));
    let mut iface_interval = tokio::time::interval(std::time::Duration::from_secs(5));
    let mut tick_count: u64 = 0;
    let mut recv_buf = vec![0u8; 65535];
    let mut send_buf = vec![0u8; 65535];

    loop {
        tokio::select! {
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    break;
                }
            }

            // Interface polling (every 5s)
            _ = iface_interval.tick() => {
                let new_addrs = get_local_ipv4_addrs();
                let mut addrs = local_addrs.lock().await;
                if *addrs != new_addrs {
                    tracing::debug!("Network interfaces changed");
                    *addrs = new_addrs;
                    // Trigger immediate query on interface change
                    send_query(&send_socket, &multicast_target, &broadcast_target).await;
                }
            }

            // Periodic query
            _ = scan_interval.tick() => {
                tick_count += 1;
                // Skip queries when paired
                if *paired_rx.borrow() {
                    continue;
                }
                if tick_count <= 6 || tick_count.is_multiple_of(6) {
                    send_query(&send_socket, &multicast_target, &broadcast_target).await;
                }
            }

            result = recv_socket.recv_from(&mut recv_buf) => {
                match result {
                    Ok((len, from)) => {
                        let addrs = local_addrs.lock().await;
                        if let Some(core) = process_response(&recv_buf[..len], from, &addrs) {
                            let _ = core_tx.send(core);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("SOOD recv error (recv_socket): {}", e);
                    }
                }
            }

            result = send_socket.recv_from(&mut send_buf) => {
                match result {
                    Ok((len, from)) => {
                        let addrs = local_addrs.lock().await;
                        if let Some(core) = process_response(&send_buf[..len], from, &addrs) {
                            let _ = core_tx.send(core);
                        }
                    }
                    Err(e) => {
                        tracing::warn!("SOOD recv error (send_socket): {}", e);
                    }
                }
            }
        }
    }
}

async fn send_query(
    send_socket: &UdpSocket,
    multicast_target: &SocketAddr,
    broadcast_target: &SocketAddr,
) {
    let packet = build_query_packet();

    if let Err(e) = send_socket.send_to(&packet, multicast_target).await {
        tracing::debug!("SOOD multicast send failed: {}", e);
    }
    if let Err(e) = send_socket.send_to(&packet, broadcast_target).await {
        tracing::debug!("SOOD broadcast send failed: {}", e);
    }
}

/// Process a received SOOD response, applying localhost detection.
fn process_response(
    buf: &[u8],
    from: SocketAddr,
    local_addrs: &HashSet<IpAddr>,
) -> Option<DiscoveredCore> {
    let msg = match parse(buf, from) {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!("SOOD parse error: {}", e);
            return None;
        }
    };

    if msg.msg_type != SoodType::Response {
        return None;
    }

    let core_id = msg.props.get("unique_id")?.as_ref()?.clone();
    let http_port_str = msg.props.get("http_port")?.as_ref()?;
    let http_port: u16 = http_port_str.parse().ok()?;
    let name = msg.props.get("name").and_then(|v| v.clone());
    let display_version = msg.props.get("display_version").and_then(|v| v.clone());

    // Localhost detection: if the response IP is one of our own addresses,
    // connect to 127.0.0.1 instead (avoids loopback issues)
    let host = if local_addrs.contains(&msg.from.ip()) {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    } else {
        msg.from.ip()
    };

    Some(DiscoveredCore {
        core_id,
        host,
        http_port,
        name,
        display_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_response_valid() {
        let mut props = HashMap::new();
        props.insert(
            "service_id".to_string(),
            Some(ROON_CORE_SERVICE_ID.to_string()),
        );
        props.insert("unique_id".to_string(), Some("test-core-123".to_string()));
        props.insert("http_port".to_string(), Some("9100".to_string()));
        props.insert("_tid".to_string(), Some("tid-placeholder".to_string()));

        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD\x02R");
        for (name, value) in &props {
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            match value {
                Some(v) => {
                    buf.extend_from_slice(&(v.len() as u16).to_be_bytes());
                    buf.extend_from_slice(v.as_bytes());
                }
                None => {
                    buf.extend_from_slice(&0xFFFFu16.to_be_bytes());
                }
            }
        }

        let from: SocketAddr = "192.168.1.100:9003".parse().unwrap();
        let empty_local = HashSet::new();
        let core = process_response(&buf, from, &empty_local).unwrap();
        assert_eq!(core.core_id, "test-core-123");
        assert_eq!(core.http_port, 9100);
        assert_eq!(core.host, IpAddr::V4("192.168.1.100".parse().unwrap()));
    }

    #[test]
    fn test_process_response_localhost_detection() {
        let mut props = HashMap::new();
        props.insert("unique_id".to_string(), Some("local-core".to_string()));
        props.insert("http_port".to_string(), Some("9330".to_string()));

        let mut buf = Vec::new();
        buf.extend_from_slice(b"SOOD\x02R");
        for (name, value) in &props {
            buf.push(name.len() as u8);
            buf.extend_from_slice(name.as_bytes());
            if let Some(v) = value {
                buf.extend_from_slice(&(v.len() as u16).to_be_bytes());
                buf.extend_from_slice(v.as_bytes());
            }
        }

        let from: SocketAddr = "192.168.1.20:9003".parse().unwrap();
        let mut local = HashSet::new();
        local.insert(IpAddr::V4("192.168.1.20".parse().unwrap()));

        let core = process_response(&buf, from, &local).unwrap();
        assert_eq!(core.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
    }

    #[test]
    fn test_process_response_ignores_queries() {
        let buf = b"SOOD\x02Q";
        let from: SocketAddr = "192.168.1.100:9003".parse().unwrap();
        assert!(process_response(buf, from, &HashSet::new()).is_none());
    }

    #[test]
    fn test_process_response_missing_fields() {
        let buf = b"SOOD\x02R";
        let from: SocketAddr = "192.168.1.100:9003".parse().unwrap();
        assert!(process_response(buf, from, &HashSet::new()).is_none());
    }

    #[test]
    fn test_build_query_packet_is_valid() {
        let packet = build_query_packet();
        let from: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let msg = parse(&packet, from).unwrap();
        assert_eq!(msg.msg_type, SoodType::Query);
        assert_eq!(
            msg.props.get("query_service_id").unwrap().as_ref().unwrap(),
            ROON_CORE_SERVICE_ID
        );
        assert!(msg.props.contains_key("_tid"));
    }

    #[tokio::test]
    async fn test_loopback_send_recv() {
        let recv = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let recv_addr = recv.local_addr().unwrap();

        let send = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        send.set_broadcast(true).unwrap();

        let packet = build_query_packet();
        send.send_to(&packet, recv_addr).await.unwrap();

        let mut buf = vec![0u8; 65535];
        let (len, from) =
            tokio::time::timeout(std::time::Duration::from_secs(1), recv.recv_from(&mut buf))
                .await
                .unwrap()
                .unwrap();

        let msg = parse(&buf[..len], from).unwrap();
        assert_eq!(msg.msg_type, SoodType::Query);
    }

    #[test]
    fn test_get_local_ipv4_addrs() {
        let addrs = get_local_ipv4_addrs();
        assert!(addrs.contains(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }
}
