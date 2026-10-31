use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use bytes::Bytes;
use futures_util::stream;
use serde::Serialize;
use std::{
    io,
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    sync::{broadcast, watch, OwnedSemaphorePermit, Semaphore},
};

const MAX_LISTENERS: usize = 16;
const BUFFER_FRAMES: usize = 64;
const RETRY_DELAY: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(3);
const STDERR_LIMIT: usize = 2048;
const RECEIVER_CONFIG: &str = r#"general = {
    output_backend = "stdout";
    mdns_backend = "avahi";
    playback_mode = "stereo";
};
metadata = { enabled = "no"; };
diagnostics = { log_output_to = "stderr"; };
"#;
const ENCODER_ARGS: &[&str] = &[
    "-hide_banner",
    "-loglevel",
    "error",
    "-nostdin",
    "-f",
    "s16le",
    "-ar",
    "44100",
    "-ac",
    "2",
    "-probesize",
    "32",
    "-analyzeduration",
    "0",
    "-i",
    "pipe:0",
    "-map",
    "0:a:0",
    "-c:a",
    "libmp3lame",
    "-b:a",
    "192k",
    "-reservoir",
    "0",
    "-write_xing",
    "0",
    "-id3v2_version",
    "0",
    "-write_id3v1",
    "0",
    "-flush_packets",
    "1",
    "-f",
    "mp3",
    "pipe:1",
];

#[derive(Clone, Debug)]
pub struct AudioGateway {
    inner: Arc<GatewayInner>,
}

#[derive(Debug)]
struct GatewayInner {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
    stopped: watch::Receiver<bool>,
}

impl Drop for GatewayInner {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

#[derive(Debug)]
struct Shared {
    runtime: Mutex<Runtime>,
    listeners: Arc<Semaphore>,
    bytes_streamed: AtomicU64,
}

#[derive(Debug)]
struct Runtime {
    status: AudioStatus,
    audio: Option<broadcast::Sender<Bytes>>,
    last_frame: Option<Instant>,
}

#[derive(Clone, Debug, Serialize)]
struct AudioStatus {
    enabled: bool,
    available: bool,
    state: AudioState,
    name: String,
    listeners: usize,
    bytes_streamed: u64,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum AudioState {
    Disabled,
    Starting,
    Waiting,
    Streaming,
    Error,
}

impl Shared {
    fn runtime(&self) -> MutexGuard<'_, Runtime> {
        self.runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn status(&self) -> AudioStatus {
        let runtime = self.runtime();
        let mut status = runtime.status.clone();
        if status.state == AudioState::Streaming
            && runtime
                .last_frame
                .is_some_and(|last| last.elapsed() > IDLE_TIMEOUT)
        {
            status.state = AudioState::Waiting;
        }
        status.listeners = MAX_LISTENERS - self.listeners.available_permits();
        status.bytes_streamed = self.bytes_streamed.load(Ordering::Relaxed);
        status
    }
}

impl AudioGateway {
    pub async fn start(enabled: bool, name: String, data_dir: PathBuf) -> Result<Self> {
        Self::start_with_programs(
            enabled,
            name,
            data_dir,
            PathBuf::from("shairport-sync"),
            PathBuf::from("ffmpeg"),
        )
        .await
    }

    async fn start_with_programs(
        enabled: bool,
        name: String,
        data_dir: PathBuf,
        receiver: PathBuf,
        encoder: PathBuf,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            runtime: Mutex::new(Runtime {
                status: AudioStatus {
                    enabled,
                    available: false,
                    state: if enabled {
                        AudioState::Starting
                    } else {
                        AudioState::Disabled
                    },
                    name,
                    listeners: 0,
                    bytes_streamed: 0,
                    error: None,
                },
                audio: None,
                last_frame: None,
            }),
            listeners: Arc::new(Semaphore::new(MAX_LISTENERS)),
            bytes_streamed: AtomicU64::new(0),
        });
        let (stop, stop_receiver) = watch::channel(false);
        let (stopped_sender, stopped) = watch::channel(!enabled);
        if enabled {
            let shared = shared.clone();
            tokio::spawn(async move {
                supervise(shared, stop_receiver, data_dir, receiver, encoder).await;
                stopped_sender.send_replace(true);
            });
        }
        Ok(Self {
            inner: Arc::new(GatewayInner {
                shared,
                stop,
                stopped,
            }),
        })
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/audio/status", get(audio_status))
            .route("/api/audio/stream", get(audio_stream))
            .with_state(self.clone())
    }

    pub async fn shutdown(&self) {
        self.inner.stop.send_replace(true);
        let mut stopped = self.inner.stopped.clone();
        let _ = stopped.wait_for(|done| *done).await;
    }
}

async fn audio_status(State(gateway): State<AudioGateway>) -> Json<AudioStatus> {
    Json(gateway.inner.shared.status())
}

struct Listener {
    _permit: OwnedSemaphorePermit,
}

async fn audio_stream(State(gateway): State<AudioGateway>) -> Response {
    let shared = &gateway.inner.shared;
    let receiver = {
        let runtime = shared.runtime();
        match runtime.audio.as_ref() {
            Some(audio) if runtime.status.available => audio.subscribe(),
            _ => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    };
    let permit = match shared.listeners.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return StatusCode::TOO_MANY_REQUESTS.into_response(),
    };
    let listener = Listener { _permit: permit };
    let stop = gateway.inner.stop.subscribe();
    let audio = stream::unfold(Some((receiver, listener, stop)), |state| async move {
        let (mut receiver, listener, mut stop) = state?;
        if *stop.borrow() {
            return None;
        }
        let received = tokio::select! {
            biased;
            _ = stop.changed() => return None,
            received = receiver.recv() => received,
        };
        match received {
            Ok(bytes) => Some((Ok::<_, io::Error>(bytes), Some((receiver, listener, stop)))),
            Err(broadcast::error::RecvError::Closed) => None,
            Err(broadcast::error::RecvError::Lagged(_)) => Some((
                Err(io::Error::other("audio listener fell behind; reconnect")),
                None,
            )),
        }
    });
    (
        [
            (header::CONTENT_TYPE, "audio/mpeg"),
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        Body::from_stream(audio),
    )
        .into_response()
}

async fn supervise(
    shared: Arc<Shared>,
    mut stop: watch::Receiver<bool>,
    data_dir: PathBuf,
    receiver: PathBuf,
    encoder: PathBuf,
) {
    while !*stop.borrow() {
        {
            let mut runtime = shared.runtime();
            runtime.status.state = AudioState::Starting;
            runtime.status.available = false;
            runtime.status.error = None;
        }
        let result = run_pipeline(&shared, &mut stop, &data_dir, &receiver, &encoder).await;
        if *stop.borrow() {
            break;
        }
        {
            let mut runtime = shared.runtime();
            runtime.audio = None;
            runtime.status.available = false;
            runtime.status.state = AudioState::Error;
            runtime.status.error = Some(match result {
                Ok(()) => "audio pipeline stopped unexpectedly".to_owned(),
                Err(error) => format!("{error:#}"),
            });
        }
        tokio::select! {
            _ = stop.changed() => break,
            _ = tokio::time::sleep(RETRY_DELAY) => {},
        }
    }
    let mut runtime = shared.runtime();
    runtime.audio = None;
    runtime.status.enabled = false;
    runtime.status.available = false;
    runtime.status.state = AudioState::Disabled;
    runtime.status.error = None;
}

async fn run_pipeline(
    shared: &Shared,
    stop: &mut watch::Receiver<bool>,
    data_dir: &std::path::Path,
    receiver_program: &std::path::Path,
    encoder_program: &std::path::Path,
) -> Result<()> {
    if !cfg!(target_endian = "little") {
        bail!("the Shairport stdout gateway requires a little-endian host");
    }
    let config_dir = data_dir.join("audio");
    tokio::fs::create_dir_all(&config_dir)
        .await
        .context("creating audio configuration directory")?;
    let config_path = config_dir.join("shairport-sync.conf");
    tokio::fs::write(&config_path, RECEIVER_CONFIG)
        .await
        .context("writing audio receiver configuration")?;
    if *stop.borrow() {
        return Ok(());
    }
    let mut encoder = Command::new(encoder_program)
        .args(ENCODER_ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("starting MP3 encoder {}", encoder_program.display()))?;
    let name = shared.runtime().status.name.clone();
    let receiver = Command::new(receiver_program)
        .arg("-c")
        .arg(&config_path)
        .args(["-o", "stdout", "-m", "avahi", "-a", &name])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn();
    let mut receiver = match receiver {
        Ok(receiver) => receiver,
        Err(error) => {
            terminate(&mut encoder).await;
            return Err(error).with_context(|| {
                format!("starting AirPlay receiver {}", receiver_program.display())
            });
        }
    };
    let result = transfer_audio(shared, stop, &mut receiver, &mut encoder).await;
    {
        let mut runtime = shared.runtime();
        runtime.audio = None;
        runtime.status.available = false;
    }
    tokio::join!(terminate(&mut receiver), terminate(&mut encoder));
    result
}

async fn terminate(child: &mut Child) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

async fn transfer_audio(
    shared: &Shared,
    stop: &mut watch::Receiver<bool>,
    receiver: &mut Child,
    encoder: &mut Child,
) -> Result<()> {
    let mut pcm = receiver.stdout.take().context("receiver stdout missing")?;
    let mut input = encoder.stdin.take().context("encoder stdin missing")?;
    let output = encoder.stdout.take().context("encoder stdout missing")?;
    let receiver_stderr = receiver.stderr.take().context("receiver stderr missing")?;
    let encoder_stderr = encoder.stderr.take().context("encoder stderr missing")?;
    let (audio, _) = broadcast::channel(BUFFER_FRAMES);
    {
        let mut runtime = shared.runtime();
        runtime.audio = Some(audio.clone());
        runtime.status.state = AudioState::Waiting;
        runtime.status.available = true;
        runtime.last_frame = None;
    }
    if *stop.borrow() {
        return Ok(());
    }
    let receiver_errors = capture_stderr(receiver_stderr);
    let encoder_errors = capture_stderr(encoder_stderr);
    tokio::pin!(receiver_errors, encoder_errors);
    tokio::select! {
        biased;
        _ = stop.changed() => Ok(()),
        errors = &mut receiver_errors => bail!("AirPlay receiver exited: {}", errors?),
        errors = &mut encoder_errors => bail!("MP3 encoder exited: {}", errors?),
        status = receiver.wait() => {
            let errors = receiver_errors.await?;
            bail!("AirPlay receiver exited ({:?}): {errors}", status?);
        },
        status = encoder.wait() => {
            let errors = encoder_errors.await?;
            bail!("MP3 encoder exited ({:?}): {errors}", status?);
        },
        result = tokio::io::copy(&mut pcm, &mut input) => {
            result.context("forwarding receiver PCM")?;
            bail!("AirPlay receiver PCM ended");
        },
        result = broadcast_frames(output, shared, audio) => {
            result?;
            bail!("MP3 encoder stream ended");
        },
    }
}

async fn capture_stderr(mut stderr: impl AsyncRead + Unpin) -> io::Result<String> {
    let mut retained = Vec::with_capacity(STDERR_LIMIT);
    let mut buffer = [0; 1024];
    loop {
        let count = stderr.read(&mut buffer).await?;
        if count == 0 {
            return Ok(String::from_utf8_lossy(&retained).trim().to_owned());
        }
        let excess = (retained.len() + count).saturating_sub(STDERR_LIMIT);
        retained.drain(..excess);
        retained.extend_from_slice(&buffer[..count]);
    }
}

async fn broadcast_frames(
    mut output: impl AsyncRead + Unpin,
    shared: &Shared,
    audio: broadcast::Sender<Bytes>,
) -> Result<()> {
    loop {
        let mut header = [0; 4];
        output
            .read_exact(&mut header)
            .await
            .context("reading MP3 frame")?;
        if header[0] != 0xff || header[1] & 0xfe != 0xfa || header[2] & 0xfc != 0xb0 {
            bail!("encoder did not produce 192 kbps / 44.1 kHz MPEG-1 Layer III frames");
        }
        let frame_length = 144 * 192_000 / 44_100 + usize::from((header[2] >> 1) & 1);
        let mut frame = vec![0; frame_length];
        frame[..4].copy_from_slice(&header);
        output
            .read_exact(&mut frame[4..])
            .await
            .context("reading MP3 frame payload")?;
        shared
            .bytes_streamed
            .fetch_add(frame.len() as u64, Ordering::Relaxed);
        {
            let mut runtime = shared.runtime();
            runtime.status.state = AudioState::Streaming;
            runtime.last_frame = Some(Instant::now());
        }
        let _ = audio.send(Bytes::from(frame));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn status(gateway: &AudioGateway) -> serde_json::Value {
        let response = gateway
            .router()
            .oneshot(
                Request::get("/api/audio/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    async fn wait_for_state(gateway: &AudioGateway, state: AudioState) {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if gateway.inner.shared.status().state == state {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("waiting for {state:?}: {:?}", gateway.inner.shared.status()));
    }

    #[tokio::test]
    async fn disabled_does_not_start_programs_or_create_files() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("unused");
        let gateway = AudioGateway::start_with_programs(
            false,
            "Test output".to_owned(),
            data_dir.clone(),
            directory.path().join("missing-receiver"),
            directory.path().join("missing-encoder"),
        )
        .await
        .unwrap();
        assert_eq!(
            status(&gateway).await,
            serde_json::json!({
                "enabled": false, "available": false, "state": "disabled",
                "name": "Test output", "listeners": 0, "bytes_streamed": 0, "error": null
            })
        );
        assert!(!data_dir.exists());
        let response = gateway
            .router()
            .oneshot(
                Request::get("/api/audio/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        gateway.shutdown().await;
    }

    #[tokio::test]
    async fn missing_encoder_is_reported_without_failing_start() {
        let directory = tempfile::tempdir().unwrap();
        let gateway = AudioGateway::start_with_programs(
            true,
            "Missing encoder".to_owned(),
            directory.path().to_owned(),
            directory.path().join("missing-receiver"),
            directory.path().join("missing-encoder"),
        )
        .await
        .unwrap();
        wait_for_state(&gateway, AudioState::Error).await;
        let snapshot = status(&gateway).await;
        assert_eq!(snapshot["available"], false);
        assert_eq!(snapshot["enabled"], true);
        assert!(snapshot["error"]
            .as_str()
            .unwrap()
            .contains("missing-encoder"));
        gateway.shutdown().await;
        assert_eq!(status(&gateway).await["state"], "disabled");
    }

    async fn injected_gateway() -> (AudioGateway, broadcast::Sender<Bytes>) {
        let gateway = AudioGateway::start(false, "Test".to_owned(), PathBuf::new())
            .await
            .unwrap();
        let (sender, _) = broadcast::channel(BUFFER_FRAMES);
        {
            let mut runtime = gateway.inner.shared.runtime();
            runtime.audio = Some(sender.clone());
            runtime.status.available = true;
        }
        (gateway, sender)
    }

    #[tokio::test]
    async fn lagging_clients_disconnect_instead_of_skipping_audio() {
        let (gateway, sender) = injected_gateway().await;
        let response = audio_stream(State(gateway.clone())).await;
        assert_eq!(response.status(), StatusCode::OK);
        for _ in 0..=BUFFER_FRAMES {
            sender.send(Bytes::from_static(b"frame")).unwrap();
        }
        let mut body = response.into_body();
        assert!(body.frame().await.unwrap().is_err());
        assert_eq!(gateway.inner.shared.status().listeners, 0);
        assert!(body.frame().await.is_none());
    }

    #[tokio::test]
    async fn listeners_are_bounded_and_release_slots_on_disconnect() {
        let (gateway, _sender) = injected_gateway().await;
        let mut responses = Vec::new();
        for _ in 0..MAX_LISTENERS {
            responses.push(audio_stream(State(gateway.clone())).await);
        }
        assert_eq!(gateway.inner.shared.status().listeners, MAX_LISTENERS);
        assert_eq!(
            audio_stream(State(gateway.clone())).await.status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(responses);
        assert_eq!(gateway.inner.shared.status().listeners, 0);
    }

    #[tokio::test]
    async fn stderr_retention_is_bounded() {
        let bytes = vec![b'x'; STDERR_LIMIT * 10];
        assert_eq!(
            capture_stderr(bytes.as_slice()).await.unwrap().len(),
            STDERR_LIMIT
        );
    }

    #[cfg(unix)]
    fn script(directory: &std::path::Path, name: &str, contents: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{contents}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    fn process_exists(pid: &str) -> bool {
        std::process::Command::new("kill")
            .args(["-0", pid.trim()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    }

    #[cfg(unix)]
    fn ffmpeg_available() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn missing_receiver_is_reported_and_encoder_is_reaped() {
        if !ffmpeg_available() {
            assert!(std::env::var_os("CI").is_none(), "CI must install ffmpeg");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let gateway = AudioGateway::start_with_programs(
            true,
            "Missing receiver".to_owned(),
            directory.path().to_owned(),
            directory.path().join("missing-receiver"),
            PathBuf::from("ffmpeg"),
        )
        .await
        .unwrap();
        wait_for_state(&gateway, AudioState::Error).await;
        assert!(status(&gateway).await["error"]
            .as_str()
            .unwrap()
            .contains("missing-receiver"));
        gateway.shutdown().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn synthetic_pcm_encodes_and_streams_over_http_and_reaps_on_shutdown() {
        use tokio::io::AsyncWriteExt;
        if !ffmpeg_available() {
            assert!(std::env::var_os("CI").is_none(), "CI must install ffmpeg");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let receiver_pid = directory.path().join("receiver.pid");
        let encoder_pid = directory.path().join("encoder.pid");
        let receiver = script(
            directory.path(),
            "receiver",
            &format!(
                "echo $$ > '{}'\nexec ffmpeg -hide_banner -loglevel error -nostdin -re -f lavfi -i sine=frequency=440:sample_rate=44100 -ac 2 -f s16le pipe:1",
                receiver_pid.display()
            ),
        );
        let encoder = script(
            directory.path(),
            "encoder",
            &format!("echo $$ > '{}'\nexec ffmpeg \"$@\"", encoder_pid.display()),
        );
        let gateway = AudioGateway::start_with_programs(
            true,
            "Synthetic output".to_owned(),
            directory.path().to_owned(),
            receiver,
            encoder,
        )
        .await
        .unwrap();
        wait_for_state(&gateway, AudioState::Streaming).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = gateway.router();
        let (server_stop, server_stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = server_stopped.await;
                })
                .await
                .unwrap();
        });
        let mut response = reqwest::get(format!("http://{address}/api/audio/stream"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/mpeg");
        assert!(response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store"));
        let mut encoded = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while encoded.len() < 24_000 {
                encoded.extend_from_slice(&response.chunk().await.unwrap().unwrap());
            }
        })
        .await
        .unwrap();
        assert_eq!(encoded[0], 0xff);
        assert_ne!(&encoded[..3], b"ID3");
        assert!(!encoded
            .windows(4)
            .any(|window| window == b"Xing" || window == b"Info"));
        let mut offset = 0;
        while offset + 627 <= encoded.len() {
            assert_eq!(encoded[offset], 0xff);
            assert_eq!(encoded[offset + 4], 0);
            assert_eq!(encoded[offset + 5] >> 7, 0);
            offset += 144 * 192_000 / 44_100 + usize::from((encoded[offset + 2] >> 1) & 1);
        }
        assert_eq!(gateway.inner.shared.status().listeners, 1);
        let mut decoder = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "mp3",
                "-i",
                "pipe:0",
                "-f",
                "s16le",
                "pipe:1",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        decoder
            .stdin
            .take()
            .unwrap()
            .write_all(&encoded)
            .await
            .unwrap();
        let decoded = decoder.wait_with_output().await.unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        assert!(decoded.stdout.len() > 100_000);
        assert!(decoded
            .stdout
            .as_chunks::<2>()
            .0
            .iter()
            .any(|sample| i16::from_le_bytes(*sample).unsigned_abs() > 100));
        assert!(gateway.inner.shared.status().bytes_streamed >= encoded.len() as u64);
        let receiver_pid = std::fs::read_to_string(receiver_pid).unwrap();
        let encoder_pid = std::fs::read_to_string(encoder_pid).unwrap();
        assert!(process_exists(&receiver_pid));
        assert!(process_exists(&encoder_pid));
        tokio::join!(gateway.shutdown(), gateway.shutdown());
        assert!(!process_exists(&receiver_pid));
        assert!(!process_exists(&encoder_pid));
        tokio::time::timeout(Duration::from_secs(5), async {
            while response.chunk().await.unwrap().is_some() {}
        })
        .await
        .unwrap();
        assert_eq!(gateway.inner.shared.status().listeners, 0);
        drop(response);
        let _ = server_stop.send(());
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn dropping_last_gateway_reaps_the_receiver() {
        if !ffmpeg_available() {
            assert!(std::env::var_os("CI").is_none(), "CI must install ffmpeg");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let pid_file = directory.path().join("receiver.pid");
        let receiver = script(
            directory.path(),
            "receiver",
            &format!("echo $$ > '{}'\nexec sleep 60", pid_file.display()),
        );
        let gateway = AudioGateway::start_with_programs(
            true,
            "Drop test".to_owned(),
            directory.path().to_owned(),
            receiver,
            PathBuf::from("ffmpeg"),
        )
        .await
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !pid_file.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let pid = std::fs::read_to_string(pid_file).unwrap();
        let clone = gateway.clone();
        drop(gateway);
        assert!(process_exists(&pid));
        drop(clone);
        tokio::time::timeout(Duration::from_secs(5), async {
            while process_exists(&pid) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn receiver_failure_is_retried_only_after_both_children_are_reaped() {
        if !ffmpeg_available() {
            assert!(std::env::var_os("CI").is_none(), "CI must install ffmpeg");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let receiver_pids = directory.path().join("receiver.pids");
        let encoder_pids = directory.path().join("encoder.pids");
        let receiver = script(
            directory.path(),
            "receiver",
            &format!(
                "echo $$ >> '{}'\nif [ \"$(wc -l < '{}')\" -eq 1 ]; then\n  echo synthetic-receiver-failure >&2\n  exit 7\nfi\nexec sleep 60",
                receiver_pids.display(), receiver_pids.display()
            ),
        );
        let encoder = script(
            directory.path(),
            "encoder",
            &format!(
                "echo $$ >> '{}'\nexec ffmpeg \"$@\"",
                encoder_pids.display()
            ),
        );
        let gateway = AudioGateway::start_with_programs(
            true,
            "Retry test".to_owned(),
            directory.path().to_owned(),
            receiver,
            encoder,
        )
        .await
        .unwrap();
        wait_for_state(&gateway, AudioState::Error).await;
        let first_receiver = std::fs::read_to_string(&receiver_pids).unwrap();
        assert!(!process_exists(first_receiver.trim()));
        if let Ok(first_encoder) = std::fs::read_to_string(&encoder_pids) {
            assert!(!process_exists(first_encoder.trim()));
        }
        tokio::time::timeout(RETRY_DELAY + Duration::from_secs(5), async {
            loop {
                if std::fs::read_to_string(&receiver_pids)
                    .is_ok_and(|pids| pids.lines().count() == 2)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        wait_for_state(&gateway, AudioState::Waiting).await;
        gateway.shutdown().await;
        for path in [receiver_pids, encoder_pids] {
            if let Ok(pids) = std::fs::read_to_string(path) {
                for pid in pids.lines() {
                    assert!(!process_exists(pid), "child {pid} survived retry/shutdown");
                }
            }
        }
    }
}
