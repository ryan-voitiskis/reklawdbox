//! Streamable HTTP surface for running reklawdbox as a network service
//! (a container in a cluster) instead of a stdio child of the MCP client.
//!
//! ```text
//! REKLAWDBOX_HTTP_ADDR=0.0.0.0:8000              # turns HTTP mode on
//! MCP_BEARER_TOKEN=<random, 32+ characters>      # required
//! REKLAWDBOX_HTTP_ALLOWED_HOSTS=reklawdbox.example.com,reklawdbox   # Host header allowlist
//! ```
//!
//! Routes: `POST/GET/DELETE /mcp` (MCP Streamable HTTP, bearer token required)
//! and `GET /healthz` (no token, for probes). There is no TLS: put a reverse
//! proxy in front. Every session shares one server state, so changes staged in
//! one session are visible to the next — the same single-operator model as the
//! stdio server.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use sha2::{Digest, Sha256};

pub(crate) const ADDR_ENV: &str = "REKLAWDBOX_HTTP_ADDR";
pub(crate) const TOKEN_ENV: &str = "MCP_BEARER_TOKEN";
pub(crate) const HOSTS_ENV: &str = "REKLAWDBOX_HTTP_ALLOWED_HOSTS";

/// Short tokens are guessable over a network; 32 characters is
/// `openssl rand -hex 16`.
const MIN_TOKEN_LEN: usize = 32;

#[derive(Debug)]
pub(crate) struct HttpSettings {
    pub(crate) addr: SocketAddr,
    token: String,
    /// Empty means rmcp's loopback-only default.
    allowed_hosts: Vec<String>,
}

pub(crate) fn configured() -> bool {
    std::env::var_os(ADDR_ENV).is_some_and(|value| !value.is_empty())
}

impl HttpSettings {
    pub(crate) fn from_env() -> Result<Self, String> {
        let addr = std::env::var(ADDR_ENV).unwrap_or_default();
        let token = std::env::var(TOKEN_ENV).ok();
        let hosts = std::env::var(HOSTS_ENV).ok();
        Self::parse(&addr, token.as_deref(), hosts.as_deref())
    }

    fn parse(addr: &str, token: Option<&str>, hosts: Option<&str>) -> Result<Self, String> {
        let addr = addr.trim().parse::<SocketAddr>().map_err(|error| {
            format!("{ADDR_ENV}={addr:?} is not an address like 0.0.0.0:8000: {error}")
        })?;
        let token = token.map(str::trim).unwrap_or_default().to_string();
        if token.is_empty() {
            return Err(format!(
                "{TOKEN_ENV} is required in HTTP mode: the MCP tools read the library and write files"
            ));
        }
        if token.len() < MIN_TOKEN_LEN {
            return Err(format!(
                "{TOKEN_ENV} must have at least {MIN_TOKEN_LEN} characters (e.g. `openssl rand -hex 32`)"
            ));
        }
        let allowed_hosts = hosts
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(str::to_string)
            .collect();
        Ok(Self {
            addr,
            token,
            allowed_hosts,
        })
    }
}

/// SHA-256 of the expected token; comparing digests keeps the comparison
/// time independent of how many leading characters a guess got right.
#[derive(Clone)]
struct ExpectedToken(Arc<[u8; 32]>);

impl ExpectedToken {
    fn new(token: &str) -> Self {
        Self(Arc::new(Sha256::digest(token.as_bytes()).into()))
    }

    fn matches(&self, presented: &str) -> bool {
        let presented: [u8; 32] = Sha256::digest(presented.as_bytes()).into();
        presented
            .iter()
            .zip(self.0.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

fn bearer(request: &Request) -> Option<&str> {
    let value = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

async fn require_bearer(
    State(expected): State<ExpectedToken>,
    request: Request,
    next: Next,
) -> Response {
    match bearer(&request) {
        Some(token) if expected.matches(token) => next.run(request).await,
        _ => (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "missing or invalid bearer token\n",
        )
            .into_response(),
    }
}

pub(crate) fn router(server: crate::mcp::ReklawdboxServer, settings: &HttpSettings) -> Router {
    let mut config = StreamableHttpServerConfig::default();
    if !settings.allowed_hosts.is_empty() {
        config = config.with_allowed_hosts(settings.allowed_hosts.clone());
    }
    let mcp = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let protected = Router::new()
        .nest_service("/mcp", mcp)
        .layer(middleware::from_fn_with_state(
            ExpectedToken::new(&settings.token),
            require_bearer,
        ));
    Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .merge(protected)
}

pub(crate) async fn serve(
    server: crate::mcp::ReklawdboxServer,
    settings: HttpSettings,
) -> Result<(), Box<dyn std::error::Error>> {
    let app = router(server, &settings);
    let listener = tokio::net::TcpListener::bind(settings.addr).await?;
    tracing::info!(
        "MCP over HTTP on http://{}/mcp (allowed hosts: {})",
        listener.local_addr()?,
        if settings.allowed_hosts.is_empty() {
            "loopback only".to_string()
        } else {
            settings.allowed_hosts.join(", ")
        }
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// PID 1 in a container ignores SIGTERM unless it installs a handler.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn parse_requires_address_and_a_long_token() {
        assert!(HttpSettings::parse("0.0.0.0:8000", Some(TOKEN), None).is_ok());
        assert!(HttpSettings::parse("nonsense", Some(TOKEN), None).is_err());
        assert!(HttpSettings::parse("0.0.0.0:8000", None, None).is_err());
        assert!(HttpSettings::parse("0.0.0.0:8000", Some("  "), None).is_err());
        assert!(HttpSettings::parse("0.0.0.0:8000", Some("short"), None).is_err());
    }

    #[test]
    fn parse_splits_allowed_hosts() {
        let settings =
            HttpSettings::parse("127.0.0.1:0", Some(TOKEN), Some(" a.example , b ,,")).unwrap();
        assert_eq!(settings.allowed_hosts, ["a.example", "b"]);
        let settings = HttpSettings::parse("127.0.0.1:0", Some(TOKEN), Some("")).unwrap();
        assert!(settings.allowed_hosts.is_empty());
    }

    #[test]
    fn token_comparison() {
        let expected = ExpectedToken::new(TOKEN);
        assert!(expected.matches(TOKEN));
        assert!(!expected.matches(&TOKEN[1..]));
        assert!(!expected.matches(""));
    }

    async fn spawn() -> SocketAddr {
        let settings = HttpSettings::parse("127.0.0.1:0", Some(TOKEN), None).unwrap();
        let app = router(crate::mcp::ReklawdboxServer::new(None), &settings);
        let listener = tokio::net::TcpListener::bind(settings.addr).await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        addr
    }

    fn initialize() -> serde_json::Value {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "http-test", "version": "0"}
            }
        })
    }

    #[tokio::test]
    async fn healthz_is_open_and_mcp_needs_the_token() {
        let addr = spawn().await;
        let client = reqwest::Client::new();

        let health = client
            .get(format!("http://{addr}/healthz"))
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), 200);

        let url = format!("http://{addr}/mcp");
        let post = |token: Option<&str>| {
            let mut request = client
                .post(&url)
                .header(header::ACCEPT, "application/json, text/event-stream")
                .json(&initialize());
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send()
        };

        assert_eq!(post(None).await.unwrap().status(), 401);
        assert_eq!(post(Some("wrong")).await.unwrap().status(), 401);

        let ok = post(Some(TOKEN)).await.unwrap();
        assert_eq!(ok.status(), 200);
        assert!(ok.headers().contains_key("mcp-session-id"));
        let body = ok.text().await.unwrap();
        assert!(
            body.contains("\"serverInfo\""),
            "unexpected initialize reply: {body}"
        );
    }
}
