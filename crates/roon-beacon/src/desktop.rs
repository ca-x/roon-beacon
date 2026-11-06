use super::*;

pub struct DesktopServer {
    address: SocketAddr,
    token: String,
    player: player::Player,
    audio: audio::AudioGateway,
    task: tokio::task::JoinHandle<Result<()>>,
}

impl DesktopServer {
    pub async fn start(data_dir: PathBuf) -> Result<Self> {
        let store = persistence::SecureState::open(&data_dir)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let address = listener.local_addr()?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("OS randomness unavailable: {error}"))?;
        let token: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let auth = auth::WebAuth::open(address, &data_dir, Some(token.clone()))?;
        let player = player::Player::new(store)?;
        let audio =
            audio::AudioGateway::start_native("Roon Beacon Desktop".to_string(), data_dir).await?;
        player.start_discovery(false, None).await;
        let state = WebState {
            config: Arc::new(RwLock::new(RuntimeConfig {
                name: "Roon Beacon Desktop".to_string(),
                broadcast_interval_secs: 0,
                live_proxy_timeout_ms: DEFAULT_LIVE_PROXY_TIMEOUT_MS,
                reverse_proxy: false,
                proxy_ip: None,
            })),
            metrics: Arc::new(RuntimeMetrics::default()),
            started_at: Instant::now(),
            remote_ip: None,
            local_ip: "127.0.0.1".to_string(),
            use_broadcast: false,
            web_bind: address,
        };
        let task = tokio::spawn(run_web_ui(
            listener,
            state,
            player.clone(),
            audio.clone(),
            auth,
        ));
        Ok(Self {
            address,
            token,
            player,
            audio,
            task,
        })
    }

    pub fn launch_url(&self) -> String {
        format!("http://{}/#desktop-token={}", self.address, self.token)
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub async fn shutdown(self) {
        self.task.abort();
        let _ = self.task.await;
        tokio::join!(self.player.shutdown(), self.audio.shutdown());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn desktop_is_loopback_only_and_requires_its_session_token() {
        let directory = tempfile::tempdir().unwrap();
        let server = DesktopServer::start(directory.path().to_path_buf())
            .await
            .unwrap();
        assert!(server.address().ip().is_loopback());
        let client = reqwest::Client::new();
        let base = format!("http://{}", server.address());
        assert_eq!(
            client
                .get(format!("{base}/api/player"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let response = client
            .post(format!("{base}/api/login"))
            .header("Origin", &base)
            .header(header::CONTENT_TYPE, "application/json")
            .body(serde_json::json!({"token":server.token}).to_string())
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        assert!(response.headers().get(header::SET_COOKIE).is_some());
        server.shutdown().await;
    }
}
