//! OAuth bearer-token validation for the MCP endpoint.
//!
//! When `OAUTH_PROVIDER` is set, Claude.ai's custom-connector advanced
//! settings hold the upstream Client ID + Client Secret and Claude.ai
//! itself runs the OAuth 2.1 + PKCE flow directly against the IdP.
//! This module's job on the server side is therefore minimal:
//!
//!   1. **Advertise** the IdP via the two well-known metadata endpoints
//!      (RFC 9728 + RFC 8414) so Claude.ai can discover authorize/token
//!      URLs after our 401 response.
//!   2. **Validate** the bearer token Claude.ai eventually presents on
//!      `/mcp` by calling the IdP's userinfo endpoint, with a short cache
//!      keyed by SHA-256 of the token.
//!
//! Single-tenant by design: `OAUTH_ALLOWED_USERS` is required, so an
//! empty allowlist cannot accidentally let any GitHub user in.

use reqwest::Client;
use rocket::serde::json::{json, Json, Value};
use rocket::State;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::env;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

const USERINFO_CACHE_TTL: Duration = Duration::from_secs(300);
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const USER_AGENT: &str = concat!("doc-manager/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    GitHub,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::GitHub => "github",
        }
    }
}

pub struct OAuthConfig {
    pub provider: Provider,
    pub public_base_url: String,
    pub allowed_users: Vec<String>,
    http: Client,
}

impl OAuthConfig {
    /// Read config from env. `Ok(None)` means OAuth is opt-out (the
    /// caller continues with the static-bearer path only). `Err(_)` means
    /// the user *tried* to enable OAuth but the config is incomplete —
    /// the caller should refuse to start.
    pub fn from_env() -> Result<Option<Self>, String> {
        let Some(raw) = env::var("OAUTH_PROVIDER").ok().filter(|v| !v.is_empty()) else {
            return Ok(None);
        };
        let provider = match raw.to_ascii_lowercase().as_str() {
            "github" => Provider::GitHub,
            other => {
                return Err(format!(
                    "OAUTH_PROVIDER='{other}' not supported (only 'github' is implemented today)"
                ))
            }
        };

        let public_base_url = env::var("OAUTH_PUBLIC_BASE_URL")
            .map_err(|_| "OAUTH_PUBLIC_BASE_URL is required when OAUTH_PROVIDER is set".to_string())?
            .trim_end_matches('/')
            .to_string();
        if public_base_url.is_empty() {
            return Err("OAUTH_PUBLIC_BASE_URL must not be empty".into());
        }

        let allowed_users: Vec<String> = env::var("OAUTH_ALLOWED_USERS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if allowed_users.is_empty() {
            return Err(
                "OAUTH_ALLOWED_USERS must list at least one user when OAUTH_PROVIDER is set"
                    .into(),
            );
        }

        let http = Client::builder()
            .user_agent(USER_AGENT)
            .timeout(HTTP_TIMEOUT)
            .build()
            .map_err(|e| format!("reqwest client: {e}"))?;

        Ok(Some(Self {
            provider,
            public_base_url,
            allowed_users,
            http,
        }))
    }
}

#[derive(Default)]
pub struct OAuthState {
    cache: Mutex<HashMap<[u8; 32], CachedUser>>,
}

struct CachedUser {
    login: String,
    fetched_at: SystemTime,
}

#[derive(Debug)]
pub enum AuthError {
    Unauthorized(String),
    Forbidden(String),
    Upstream(String),
}

pub struct AuthenticatedUser {
    pub login: String,
}

fn token_key(token: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

pub async fn validate_bearer(
    token: &str,
    config: &OAuthConfig,
    state: &OAuthState,
) -> Result<AuthenticatedUser, AuthError> {
    if token.is_empty() {
        return Err(AuthError::Unauthorized("missing bearer token".into()));
    }

    let key = token_key(token);

    {
        let mut cache = state.cache.lock().unwrap();
        prune_cache(&mut cache);
        if let Some(entry) = cache.get(&key) {
            let login = entry.login.clone();
            drop(cache);
            return enforce_allowlist(login, config);
        }
    }

    let login = match config.provider {
        Provider::GitHub => fetch_github_login(&config.http, token).await?,
    };

    {
        let mut cache = state.cache.lock().unwrap();
        cache.insert(
            key,
            CachedUser {
                login: login.clone(),
                fetched_at: SystemTime::now(),
            },
        );
    }

    enforce_allowlist(login, config)
}

fn enforce_allowlist(login: String, config: &OAuthConfig) -> Result<AuthenticatedUser, AuthError> {
    let allowed = config
        .allowed_users
        .iter()
        .any(|u| u.eq_ignore_ascii_case(&login));
    if !allowed {
        return Err(AuthError::Forbidden(format!(
            "user '{login}' not in OAUTH_ALLOWED_USERS"
        )));
    }
    Ok(AuthenticatedUser { login })
}

fn prune_cache(cache: &mut HashMap<[u8; 32], CachedUser>) {
    let now = SystemTime::now();
    cache.retain(|_, v| {
        v.fetched_at
            .checked_add(USERINFO_CACHE_TTL)
            .map(|exp| exp > now)
            .unwrap_or(false)
    });
}

async fn fetch_github_login(http: &Client, token: &str) -> Result<String, AuthError> {
    let response = http
        .get("https://api.github.com/user")
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| AuthError::Upstream(format!("github /user: {e}")))?;

    let status = response.status();
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(AuthError::Unauthorized(format!(
            "github rejected token: {status}"
        )));
    }
    if !status.is_success() {
        return Err(AuthError::Upstream(format!(
            "github /user returned {status}"
        )));
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| AuthError::Upstream(format!("github /user body: {e}")))?;
    let login = body
        .get("login")
        .and_then(|v| v.as_str())
        .ok_or_else(|| AuthError::Upstream("github /user: no 'login' field".into()))?;
    Ok(login.to_string())
}

pub fn www_authenticate_header(config: &OAuthConfig) -> String {
    format!(
        r#"Bearer realm="MCP", resource_metadata="{base}/.well-known/oauth-protected-resource""#,
        base = config.public_base_url,
    )
}

// ---------- Well-known metadata routes ----------

#[get("/.well-known/oauth-protected-resource")]
pub fn protected_resource_metadata(config: &State<OAuthConfig>) -> Json<Value> {
    let scopes: Vec<&str> = match config.provider {
        Provider::GitHub => vec!["read:user"],
    };
    Json(json!({
        "resource": format!("{}/mcp", config.public_base_url),
        "authorization_servers": [config.public_base_url.clone()],
        "bearer_methods_supported": ["header"],
        "scopes_supported": scopes,
    }))
}

#[get("/.well-known/oauth-authorization-server")]
pub fn authorization_server_metadata(config: &State<OAuthConfig>) -> Json<Value> {
    let body = match config.provider {
        Provider::GitHub => json!({
            "issuer": "https://github.com",
            "authorization_endpoint": "https://github.com/login/oauth/authorize",
            "token_endpoint": "https://github.com/login/oauth/access_token",
            "response_types_supported": ["code"],
            "grant_types_supported": ["authorization_code"],
            "code_challenge_methods_supported": ["S256"],
            "scopes_supported": ["read:user"],
        }),
    };
    Json(body)
}
