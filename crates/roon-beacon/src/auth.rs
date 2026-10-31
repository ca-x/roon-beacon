use std::fs::OpenOptions;
use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use subtle::ConstantTimeEq;

const COOKIE_NAME: &str = "roon_beacon_session";

#[derive(Clone)]
pub struct WebAuth {
    token: Option<Arc<str>>,
}

impl WebAuth {
    pub fn open(bind: SocketAddr, data_dir: &Path, configured: Option<String>) -> Result<Self> {
        let token = match configured.filter(|token| !token.is_empty()) {
            Some(token) => Some(token),
            None if bind.ip().is_loopback() => None,
            None => {
                let path = data_dir.join("web-access-token");
                let token = match std::fs::read_to_string(&path) {
                    Ok(token) => token.trim().to_string(),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        let mut random = [0u8; 32];
                        getrandom::fill(&mut random).map_err(|error| {
                            anyhow::anyhow!("OS randomness unavailable: {error}")
                        })?;
                        let token: String =
                            random.iter().map(|byte| format!("{byte:02x}")).collect();
                        let mut options = OpenOptions::new();
                        options.write(true).create_new(true);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::OpenOptionsExt;
                            options.mode(0o600);
                        }
                        match options.open(&path) {
                            Ok(mut file) => {
                                file.write_all(token.as_bytes())?;
                                file.sync_all()?;
                                token
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                                std::fs::read_to_string(&path)?.trim().to_string()
                            }
                            Err(error) => {
                                return Err(error).context("cannot create Web access token")
                            }
                        }
                    }
                    Err(error) => return Err(error).context("cannot read Web access token"),
                };
                tracing::info!(
                    "Web authentication enabled; access token file: {}",
                    path.display()
                );
                Some(token)
            }
        };
        if let Some(token) = &token {
            if token.len() < 16
                || token.len() > 256
                || !token
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
            {
                bail!(
                    "Web access token must contain 16–256 ASCII letters, digits, '-', '_' or '.'"
                );
            }
        }
        Ok(Self {
            token: token.map(Arc::from),
        })
    }

    fn accepts(&self, candidate: &str) -> bool {
        self.token
            .as_ref()
            .is_none_or(|token| bool::from(token.as_bytes().ct_eq(candidate.as_bytes())))
    }

    fn authenticated(&self, headers: &HeaderMap) -> bool {
        if self.token.is_none() {
            return true;
        }
        if let Some(candidate) = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
        {
            if self.accepts(candidate) {
                return true;
            }
        }
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(';'))
            .filter_map(|cookie| cookie.trim().split_once('='))
            .any(|(name, value)| name == COOKIE_NAME && self.accepts(value))
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/api/auth", get(auth_status))
            .route("/api/login", post(login))
            .route("/api/logout", post(logout))
            .with_state(self.clone())
    }
}

fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(origin) = origin
        .to_str()
        .ok()
        .and_then(|value| value.parse::<Uri>().ok())
    else {
        return false;
    };
    matches!(origin.scheme_str(), Some("http" | "https"))
        && origin
            .authority()
            .is_some_and(|authority| authority.as_str().eq_ignore_ascii_case(host))
        && origin.path() == "/"
        && origin.query().is_none()
}

fn loopback_host(headers: &HeaderMap) -> bool {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<axum::http::uri::Authority>().ok())
        .is_some_and(|authority| {
            let host = authority
                .host()
                .trim_start_matches('[')
                .trim_end_matches(']');
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        })
}

pub async fn protect(State(auth): State<WebAuth>, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    if path.starts_with("/api/") {
        if auth.token.is_none() && !loopback_host(request.headers()) {
            return (StatusCode::FORBIDDEN, Json(json!({"error":"Unauthenticated loopback access requires a localhost or loopback-IP Host"}))).into_response();
        }
        if !same_origin(request.headers()) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error":"Cross-origin API requests are not allowed"})),
            )
                .into_response();
        }
        if !matches!(path, "/api/auth" | "/api/login" | "/api/health")
            && !auth.authenticated(request.headers())
        {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error":"Web access token required"})),
            )
                .into_response();
        }
    }
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    response
        .headers_mut()
        .insert("referrer-policy", "same-origin".parse().unwrap());
    response
}

async fn auth_status(State(auth): State<WebAuth>, headers: HeaderMap) -> Json<serde_json::Value> {
    Json(json!({"required": auth.token.is_some(), "authenticated": auth.authenticated(&headers)}))
}

#[derive(Deserialize)]
struct Login {
    token: String,
}

async fn login(State(auth): State<WebAuth>, Json(input): Json<Login>) -> Response {
    if !auth.accepts(&input.token) {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"Incorrect Web access token"})),
        )
            .into_response();
    }
    if auth.token.is_none() {
        return Json(json!({"ok":true})).into_response();
    }
    let cookie = format!(
        "{COOKIE_NAME}={}; Path=/; HttpOnly; SameSite=Strict",
        input.token
    );
    ([(header::SET_COOKIE, cookie)], Json(json!({"ok":true}))).into_response()
}

async fn logout() -> impl IntoResponse {
    (
        [(
            header::SET_COOKIE,
            format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
        )],
        Json(json!({"ok":true})),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn test_router() -> Router {
        let auth = WebAuth {
            token: Some(Arc::from("test-access-token-1234")),
        };
        Router::new()
            .merge(auth.router())
            .route("/api/private", get(|| async { "private" }))
            .layer(axum::middleware::from_fn_with_state(auth, protect))
    }

    #[tokio::test]
    async fn private_api_requires_cookie_or_bearer() {
        let response = test_router()
            .oneshot(
                Request::builder()
                    .uri("/api/private")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        for (header_name, value) in [
            (header::COOKIE, "roon_beacon_session=test-access-token-1234"),
            (header::AUTHORIZATION, "Bearer test-access-token-1234"),
        ] {
            let response = test_router()
                .oneshot(
                    Request::builder()
                        .uri("/api/private")
                        .header(header_name, value)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn login_sets_private_cookie_and_rejects_foreign_origin() {
        let request = |origin: &str| {
            Request::builder()
                .method("POST")
                .uri("/api/login")
                .header(header::HOST, "localhost:9339")
                .header(header::ORIGIN, origin)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"token":"test-access-token-1234"}"#))
                .unwrap()
        };
        let response = test_router()
            .oneshot(request("http://localhost:9339"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.contains("HttpOnly; SameSite=Strict"));
        let response = test_router()
            .oneshot(request("http://attacker.test"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn public_binding_generates_durable_secret_without_exposing_it() {
        let directory = tempfile::tempdir().unwrap();
        let first = WebAuth::open("0.0.0.0:9339".parse().unwrap(), directory.path(), None).unwrap();
        let second =
            WebAuth::open("0.0.0.0:9339".parse().unwrap(), directory.path(), None).unwrap();
        assert_eq!(first.token.as_ref().unwrap().len(), 64);
        assert!(second.accepts(first.token.as_ref().unwrap()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.path().join("web-access-token"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn empty_compose_environment_uses_generated_authentication() {
        let directory = tempfile::tempdir().unwrap();
        let auth = WebAuth::open(
            "0.0.0.0:9339".parse().unwrap(),
            directory.path(),
            Some(String::new()),
        )
        .unwrap();
        assert!(auth.token.is_some());
        assert!(directory.path().join("web-access-token").exists());
    }

    #[tokio::test]
    async fn unauthenticated_loopback_rejects_untrusted_hosts_with_or_without_origin() {
        let directory = tempfile::tempdir().unwrap();
        let auth =
            WebAuth::open("127.0.0.1:9339".parse().unwrap(), directory.path(), None).unwrap();
        let router = Router::new()
            .route("/api/private", get(|| async { "private" }))
            .layer(axum::middleware::from_fn_with_state(auth, protect));
        for origin in [None, Some("http://attacker.test:9339")] {
            let mut request = Request::builder()
                .uri("/api/private")
                .header(header::HOST, "attacker.test:9339");
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            let response = router
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        let response = router
            .oneshot(
                Request::builder()
                    .uri("/api/private")
                    .header(header::HOST, "localhost:9339")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
