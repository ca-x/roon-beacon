use super::*;
use axum::body::{to_bytes, Body};
use futures_util::{SinkExt, StreamExt};
use roon_moo::MooBody;
use tokio::io::AsyncWriteExt;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

struct MockCore {
    endpoint: DiscoveredCore,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    authorized: Arc<AtomicBool>,
    fail_commands: Arc<AtomicBool>,
    changes: tokio::sync::broadcast::Sender<Value>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockCore {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn mock_zone() -> Value {
    json!({"zone_id":"zone-a","display_name":"Living room","state":"playing",
        "is_previous_allowed":true,"is_next_allowed":true,"is_play_allowed":false,
        "is_pause_allowed":true,"is_seek_allowed":true,
        "outputs":[{"output_id":"out-a","display_name":"Speaker","volume":{"type":"number","min":0,"max":100,"step":1,"value":32,"is_muted":false}}],
        "now_playing":{"length":240,"seek_position":12,"image_key":"cover-a","one_line":{"line1":"A real test track"},"two_line":{"line1":"A real test track","line2":"An artist"}}})
}

async fn mock_core(id: &str, authorized_initially: bool) -> MockCore {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let authorized = Arc::new(AtomicBool::new(authorized_initially));
    let fail_commands = Arc::new(AtomicBool::new(false));
    let (changes, _) = tokio::sync::broadcast::channel::<Value>(8);
    let task_requests = requests.clone();
    let task_authorized = authorized.clone();
    let task_fail = fail_commands.clone();
    let task_changes = changes.clone();
    let core_id = id.to_string();
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=>{
                    let Ok((mut stream,_))=accepted else {break;};
                    let requests=task_requests.clone();
                    let authorized=task_authorized.clone();
                    let fail_commands=task_fail.clone();
                    let core_id=core_id.clone();
                    let mut changes=task_changes.subscribe();
                    connections.spawn(async move {
                        let mut prefix=[0u8;128];
                        let length=stream.peek(&mut prefix).await.unwrap_or(0);
                        if prefix[..length].starts_with(b"GET /api/image/") {
                            let bytes=b"HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: 4\r\nConnection: close\r\n\r\n\xff\xd8\xff\xd9";
                            let _=stream.write_all(bytes).await;
                            return;
                        }
                        let Ok(mut socket)=tokio_tungstenite::accept_async(stream).await else {return;};
                        let mut zone_request=None;
                        let mut registration_request=None;
                        loop {
                            tokio::select! {
                                incoming=socket.next()=>{
                                    let Some(Ok(Message::Binary(bytes)))=incoming else {break;};
                                    let request=roon_moo::parse(&bytes).unwrap();
                                    let body=request.json_body().cloned().unwrap_or(Value::Null);
                                    requests.lock().await.push((request.name.clone(),body));
                                    let (verb,name,body)=match request.name.as_str() {
                                        "com.roonlabs.registry:1/info"=>(MooVerb::Complete,"Success",json!({"core_id":core_id,"display_name":"Mock Roon","display_version":"2.0"})),
                                        "com.roonlabs.registry:1/register"=>{
                                            registration_request=Some(request.request_id);
                                            if authorized.load(Ordering::Relaxed) {(MooVerb::Continue,"Registered",json!({"core_id":core_id,"token":"durable-roon-token","http_port":0}))}
                                            else {(MooVerb::Complete,"NotAuthorized",Value::Null)}
                                        },
                                        "com.roonlabs.transport:2/subscribe_zones"=>{
                                            zone_request=Some(request.request_id);
                                            (MooVerb::Continue,"Subscribed",json!({"zones":[mock_zone()]}))
                                        },
                                        "com.roonlabs.browse:1/browse" if request.json_body().is_some_and(|body|body["item_key"]=="message-error")=>(MooVerb::Complete,"Success",json!({"action":"message","message":"This album cannot be played","is_error":true})),
                                        "com.roonlabs.browse:1/browse"=>(MooVerb::Complete,"Success",json!({"action":"list","list":{"title":"Albums","count":1,"level":1}})),
                                        "com.roonlabs.browse:1/load"=>(MooVerb::Complete,"Success",json!({"offset":0,"items":[{"title":"Kind of Blue","subtitle":"Miles Davis","item_key":"item-1","hint":"action_list"}]})),
                                        "com.roonlabs.ping:1/ping"=>(MooVerb::Complete,"Success",Value::Null),
                                        _ if fail_commands.load(Ordering::Relaxed)=>(MooVerb::Complete,"NotSupported",Value::Null),
                                        _=>(MooVerb::Complete,"Success",Value::Null),
                                    };
                                    let response=MooMessage {verb,name:name.to_string(),request_id:request.request_id,headers:HashMap::new(),body:(!body.is_null()).then_some(MooBody::Json(body))};
                                    if socket.send(Message::Binary(roon_moo::serialize(&response).into())).await.is_err(){break;}
                                }
                                Ok(body)=changes.recv(), if zone_request.is_some()=>{
                                    let response=if body["__registration"]=="Unregistered" {
                                        MooMessage {verb:MooVerb::Complete,name:"Unregistered".to_string(),request_id:registration_request.unwrap(),headers:HashMap::new(),body:None}
                                    } else {
                                        MooMessage {verb:MooVerb::Continue,name:"Changed".to_string(),request_id:zone_request.unwrap(),headers:HashMap::new(),body:Some(MooBody::Json(body))}
                                    };
                                    if socket.send(Message::Binary(roon_moo::serialize(&response).into())).await.is_err(){break;}
                                }
                            }
                        }
                    });
                }
                _=connections.join_next(), if !connections.is_empty()=>{},
            }
        }
    });
    MockCore {
        endpoint: DiscoveredCore {
            core_id: id.to_string(),
            host: address.ip(),
            http_port: address.port(),
            name: Some("Discovered mock".to_string()),
            display_version: None,
        },
        requests,
        authorized,
        fail_commands,
        changes,
        task,
    }
}

async fn wait_for(player: &Player, predicate: impl Fn(&PlayerSnapshot) -> bool) {
    timeout(Duration::from_secs(8), async {
        loop {
            let snapshot = snapshot(State(player.clone())).await.0;
            if predicate(&snapshot) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("player reached expected public state");
}

async fn api(player: &Player, path: &str, body: Value) -> (StatusCode, Value) {
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = player.router().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn discovered_port_authorization_playback_browse_and_cover_work() {
    let server = mock_core("core-a", true).await;
    let directory = tempfile::tempdir().unwrap();
    let store = SecureState::open(directory.path()).unwrap();
    let player = Player::new(store.clone()).unwrap();
    player.observe(server.endpoint.clone()).await;
    wait_for(&player, |state| state.zones.len() == 1).await;
    assert_eq!(store.token("core-a").as_deref(), Some("durable-roon-token"));
    let requests = [
        (
            "/api/player/control",
            json!({"zone_id":"zone-a","control":"pause"}),
        ),
        ("/api/player/seek", json!({"zone_id":"zone-a","seconds":80})),
        (
            "/api/player/volume",
            json!({"output_id":"out-a","value":45}),
        ),
        (
            "/api/player/mute",
            json!({"output_id":"out-a","is_muted":true}),
        ),
    ];
    for (path, body) in requests {
        let (status, response) = api(&player, path, body).await;
        assert_eq!(status, StatusCode::OK, "{response}");
    }
    let (status,loaded)=api(&player,"/api/player/browse",json!({"session":"browser-one","hierarchy":"browse","pop_all":true,"zone_id":"zone-a","count":20})).await;
    assert_eq!(status, StatusCode::OK, "{loaded}");
    assert_eq!(loaded["list"]["title"], "Albums");
    assert_eq!(loaded["items"][0]["title"], "Kind of Blue");
    let response = player
        .router()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/player/image/cover-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(
        &to_bytes(response.into_body(), 1024).await.unwrap()[..],
        &[0xff, 0xd8, 0xff, 0xd9]
    );
    let recorded = server.requests.lock().await.clone();
    assert!(recorded
        .iter()
        .any(|(name, body)| name == "com.roonlabs.transport:2/control"
            && body == &json!({"zone_or_output_id":"zone-a","control":"pause"})));
    assert!(recorded.iter().any(
        |(name, body)| name == "com.roonlabs.transport:2/change_volume"
            && body == &json!({"output_id":"out-a","how":"absolute","value":45.0})
    ));
    assert!(recorded
        .iter()
        .any(|(name, body)| name == "com.roonlabs.browse:1/load"
            && body["multi_session_key"] == "browser-one"
            && body["count"] == 20));
    player.shutdown().await;
    let reopened = Player::new(SecureState::open(directory.path()).unwrap()).unwrap();
    reopened.observe(server.endpoint.clone()).await;
    wait_for(&reopened, |state| !state.zones.is_empty()).await;
    assert!(server
        .requests
        .lock()
        .await
        .iter()
        .any(|(name, body)| name == "com.roonlabs.registry:1/register"
            && body["token"] == "durable-roon-token"));
    reopened.shutdown().await;
}

#[tokio::test]
async fn invalid_commands_are_rejected_and_core_failures_are_not_success() {
    let server = mock_core("core-a", true).await;
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(SecureState::open(directory.path()).unwrap()).unwrap();
    player.observe(server.endpoint.clone()).await;
    wait_for(&player, |state| !state.zones.is_empty()).await;
    for (path, body) in [
        (
            "/api/player/control",
            json!({"zone_id":"unknown","control":"pause"}),
        ),
        (
            "/api/player/seek",
            json!({"zone_id":"zone-a","seconds":-10}),
        ),
        (
            "/api/player/seek",
            json!({"zone_id":"zone-a","seconds":500}),
        ),
        (
            "/api/player/volume",
            json!({"output_id":"out-a","value":101}),
        ),
        ("/api/player/core", json!({"core_id":"unknown"})),
        (
            "/api/player/browse",
            json!({"session":"browser","count":1000}),
        ),
    ] {
        assert_eq!(api(&player, path, body).await.0, StatusCode::BAD_REQUEST);
    }
    server.fail_commands.store(true, Ordering::Relaxed);
    let (status, response) = api(
        &player,
        "/api/player/control",
        json!({"zone_id":"zone-a","control":"pause"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(response["error"].as_str().unwrap().contains("NotSupported"));
    player.shutdown().await;
}

#[tokio::test]
async fn core_selection_is_durable_and_all_zone_delta_types_apply() {
    let first = mock_core("core-a", true).await;
    let second = mock_core("core-b", true).await;
    let directory = tempfile::tempdir().unwrap();
    let store = SecureState::open(directory.path()).unwrap();
    let player = Player::new(store.clone()).unwrap();
    player.observe(first.endpoint.clone()).await;
    player.observe(second.endpoint.clone()).await;
    wait_for(&player, |state| {
        state.cores.len() == 2
            && state
                .cores
                .iter()
                .all(|core| core.connection == "connected")
            && !state.zones.is_empty()
    })
    .await;
    assert_eq!(
        api(&player, "/api/player/core", json!({"core_id":"core-b"}))
            .await
            .0,
        StatusCode::OK
    );
    wait_for(&player, |state| {
        state.selected_core_id.as_deref() == Some("core-b") && !state.zones.is_empty()
    })
    .await;
    assert_eq!(store.selected_core_id().as_deref(), Some("core-b"));
    second.changes.send(json!({"zones_added":[{"zone_id":"zone-b","display_name":"New room"}],"zones_seek_changed":[{"zone_id":"zone-a","seek_position":65}],"zones_changed":[{"zone_id":"zone-c","display_name":"Changed room"}]})).unwrap();
    wait_for(&player, |state| {
        state.zones.len() == 3
            && state
                .zones
                .iter()
                .any(|zone| zone["now_playing"]["seek_position"] == 65)
    })
    .await;
    second
        .changes
        .send(json!({"zones_removed":["zone-b","zone-c"]}))
        .unwrap();
    wait_for(&player, |state| state.zones.len() == 1).await;
    player.shutdown().await;
}

#[tokio::test]
async fn unauthorized_core_is_visible_without_leaking_tokens() {
    let server = mock_core("core-a", false).await;
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(SecureState::open(directory.path()).unwrap()).unwrap();
    player.observe(server.endpoint.clone()).await;
    wait_for(&player, |state| {
        state
            .cores
            .first()
            .is_some_and(|core| core.connection == "awaiting_authorization" && core.error.is_some())
    })
    .await;
    let state = snapshot(State(player.clone())).await.0;
    assert!(state.zones.is_empty());
    assert!(!serde_json::to_string(&state)
        .unwrap()
        .contains("durable-roon-token"));
    assert_eq!(
        api(
            &player,
            "/api/player/control",
            json!({"zone_id":"zone-a","control":"pause"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    server.authorized.store(true, Ordering::Relaxed);
    wait_for(&player, |state| !state.zones.is_empty()).await;
    player.shutdown().await;
}

#[tokio::test]
async fn disconnect_clears_zones_and_a_new_announced_port_reconnects() {
    let first = mock_core("core-a", true).await;
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(SecureState::open(directory.path()).unwrap()).unwrap();
    player.observe(first.endpoint.clone()).await;
    wait_for(&player, |state| !state.zones.is_empty()).await;
    drop(first);
    wait_for(&player, |state| {
        state.zones.is_empty() && state.cores[0].connection == "offline"
    })
    .await;
    let replacement = mock_core("core-a", true).await;
    player.observe(replacement.endpoint.clone()).await;
    wait_for(&player, |state| {
        !state.zones.is_empty() && state.cores[0].http_port == replacement.endpoint.http_port
    })
    .await;
    assert!(replacement
        .requests
        .lock()
        .await
        .iter()
        .any(|(method, body)| {
            method == "com.roonlabs.registry:1/register" && body["token"] == "durable-roon-token"
        }));
    player.shutdown().await;
}

#[tokio::test]
async fn mismatched_core_identity_never_receives_saved_credentials() {
    let server = mock_core("real-core", true).await;
    let directory = tempfile::tempdir().unwrap();
    let store = SecureState::open(directory.path()).unwrap();
    store
        .save_token("advertised-core", "must-not-leak")
        .unwrap();
    let player = Player::new(store).unwrap();
    let mut endpoint = server.endpoint.clone();
    endpoint.core_id = "advertised-core".to_string();
    player.observe(endpoint).await;
    wait_for(&player, |state| state.cores[0].connection == "offline").await;
    assert!(server
        .requests
        .lock()
        .await
        .iter()
        .all(|(method, _)| { method != "com.roonlabs.registry:1/register" }));
    player.shutdown().await;
}

#[tokio::test]
async fn browse_messages_preserve_explanations_and_error_flags() {
    let server = mock_core("core-a", true).await;
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(SecureState::open(directory.path()).unwrap()).unwrap();
    player.observe(server.endpoint.clone()).await;
    wait_for(&player, |state| !state.zones.is_empty()).await;
    let (status, body) = api(
        &player,
        "/api/player/browse",
        json!({"session":"browser","item_key":"message-error"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["message"], "This album cannot be played");
    assert_eq!(body["is_error"], true);
    player.shutdown().await;
}

#[tokio::test]
async fn revoked_registration_clears_zones_even_while_websocket_stays_open() {
    let server = mock_core("core-a", true).await;
    let directory = tempfile::tempdir().unwrap();
    let player = Player::new(SecureState::open(directory.path()).unwrap()).unwrap();
    player.observe(server.endpoint.clone()).await;
    wait_for(&player, |state| !state.zones.is_empty()).await;
    server.authorized.store(false, Ordering::Relaxed);
    server
        .changes
        .send(json!({"__registration":"Unregistered"}))
        .unwrap();
    wait_for(&player, |state| {
        state.zones.is_empty() && state.cores[0].connection == "awaiting_authorization"
    })
    .await;
    assert_eq!(
        api(
            &player,
            "/api/player/control",
            json!({"zone_id":"zone-a","control":"pause"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    player.shutdown().await;
}
