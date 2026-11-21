//! Microsoft Entra v2 delegated access tokens for the Doc Manager MCP resource.

use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use reqwest::{Client, Url};
use rocket::http::Status;
use rocket::serde::json::{json, Json, Value};
use rocket::tokio::sync::Mutex;
use rocket::State;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::time::{Duration, Instant};
use uuid::Uuid;

const REQUIRED_SCOPE: &str = "mcp.access";
const KEY_LIFETIME: Duration = Duration::from_secs(3600);
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);
const ENTRA_AUTHORITY: &str = "https://login.microsoftonline.com";

pub struct OAuthConfig {
    public_base_url: String,
    tenant_id: String,
    api_client_id: String,
    connector_client_id: String,
    allowed_users: Vec<String>,
    issuer: String,
    jwks_url: String,
    http: Client,
    keys: Mutex<KeyCache>,
}

impl OAuthConfig {
    pub fn from_env() -> Result<Option<Self>, String> {
        match env::var("OAUTH_PROVIDER").as_deref() {
            Err(env::VarError::NotPresent) | Ok("") => return Ok(None),
            Ok("entra") => (),
            _ => return Err(
                "OAUTH_PROVIDER must be entra; configure resource-bound tokens before enabling MCP"
                    .into(),
            ),
        }
        let required =
            |name| env::var(name).map_err(|_| format!("{name} is required for MCP OAuth"));
        Self::new(
            &required("OAUTH_PUBLIC_BASE_URL")?,
            &required("OAUTH_TENANT_ID")?,
            &required("OAUTH_API_CLIENT_ID")?,
            &required("OAUTH_CONNECTOR_CLIENT_ID")?,
            &required("OAUTH_ALLOWED_USERS")?,
        )
        .map(Some)
    }

    fn new(
        base_url: &str,
        tenant: &str,
        api: &str,
        connector: &str,
        users: &str,
    ) -> Result<Self, String> {
        let base = Url::parse(base_url).map_err(|_| "invalid OAUTH_PUBLIC_BASE_URL")?;
        let is_loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(base.scheme() == "https" || base.scheme() == "http" && is_loopback)
            || base.host().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path() != "/"
        {
            return Err(
                "OAUTH_PUBLIC_BASE_URL must be an HTTPS origin (HTTP is allowed on loopback)"
                    .into(),
            );
        }
        let guid = |name: &str, value: &str| {
            Uuid::parse_str(value.trim())
                .map(|id| id.to_string())
                .map_err(|_| format!("{name} must contain Entra GUIDs"))
        };
        let tenant_id = guid("OAUTH_TENANT_ID", tenant)?;
        let api_client_id = guid("OAUTH_API_CLIENT_ID", api)?;
        let connector_client_id = guid("OAUTH_CONNECTOR_CLIENT_ID", connector)?;
        if api_client_id == connector_client_id {
            return Err(
                "register separate Entra applications for the MCP API and connector".into(),
            );
        }
        let allowed_users = users
            .split(',')
            .map(|id| guid("OAUTH_ALLOWED_USERS", id))
            .collect::<Result<Vec<_>, _>>()?;
        let http = Client::builder()
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| format!("could not initialize the signing-key client: {e}"))?;
        Ok(Self {
            public_base_url: base.origin().ascii_serialization(),
            issuer: format!("{ENTRA_AUTHORITY}/{tenant_id}/v2.0"),
            jwks_url: format!("{ENTRA_AUTHORITY}/{tenant_id}/discovery/v2.0/keys"),
            tenant_id,
            api_client_id,
            connector_client_id,
            allowed_users,
            http,
            keys: Mutex::default(),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_test_jwks(mut self, url: String) -> Self {
        self.jwks_url = url;
        self
    }

    fn resource_scope(&self) -> String {
        format!("{}/mcp/{REQUIRED_SCOPE}", self.public_base_url)
    }

    pub async fn validate_bearer(&self, token: &str) -> Result<(), AuthError> {
        let header = decode_header(token).map_err(|_| AuthError::Unauthorized)?;
        if header.alg != Algorithm::RS256 {
            return Err(AuthError::Unauthorized);
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|kid| !kid.is_empty())
            .ok_or(AuthError::Unauthorized)?;
        let key = self.signing_key(kid).await?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&[&self.api_client_id]);
        validation.set_issuer(&[&self.issuer]);
        validation.set_required_spec_claims(&["aud", "iss", "exp", "nbf"]);
        validation.validate_nbf = true;
        validation.leeway = 0;
        let claims = decode::<AccessClaims>(token, &key, &validation)
            .map_err(|_| AuthError::Unauthorized)?
            .claims;
        if claims.ver != "2.0" || claims.tid != self.tenant_id || claims.aud != self.api_client_id {
            return Err(AuthError::Unauthorized);
        }
        if claims.azp != self.connector_client_id || !self.allowed_users.contains(&claims.oid) {
            return Err(AuthError::Forbidden);
        }
        if !claims
            .scp
            .as_deref()
            .unwrap_or("")
            .split_ascii_whitespace()
            .any(|s| s == REQUIRED_SCOPE)
        {
            return Err(AuthError::InsufficientScope);
        }
        Ok(())
    }

    async fn signing_key(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        let mut cache = self.keys.lock().await;
        let fresh = cache
            .loaded_at
            .is_some_and(|at| at.elapsed() < KEY_LIFETIME);
        if fresh {
            if let Some(key) = cache.keys.get(kid) {
                return Ok(key.clone());
            }
        }
        // Limit outbound fetches for unknown kids and share one refresh across concurrent requests.
        if cache
            .last_attempt
            .is_some_and(|at| at.elapsed() < REFRESH_INTERVAL)
        {
            return Err(if fresh {
                AuthError::Unauthorized
            } else {
                AuthError::KeyServiceUnavailable
            });
        }
        cache.last_attempt = Some(Instant::now());
        let response = self
            .http
            .get(&self.jwks_url)
            .send()
            .await
            .map_err(|_| AuthError::KeyServiceUnavailable)?;
        if !response.status().is_success() {
            return Err(AuthError::KeyServiceUnavailable);
        }
        let jwks: KeySet = response
            .json()
            .await
            .map_err(|_| AuthError::KeyServiceUnavailable)?;
        let mut keys = HashMap::new();
        for jwk in jwks.keys {
            if jwk.kty != "RSA"
                || jwk.usage.as_deref() != Some("sig")
                || jwk.alg.as_deref().is_some_and(|alg| alg != "RS256")
                || jwk.issuer.as_deref().is_some_and(|issuer| {
                    issuer != self.issuer
                        && issuer != "https://login.microsoftonline.com/{tenantid}/v2.0"
                })
            {
                continue;
            }
            if let (Some(kid), Some(n), Some(e)) = (jwk.kid, jwk.n, jwk.e) {
                let key = DecodingKey::from_rsa_components(&n, &e)
                    .map_err(|_| AuthError::KeyServiceUnavailable)?;
                keys.insert(kid, key);
            }
        }
        if keys.is_empty() {
            return Err(AuthError::KeyServiceUnavailable);
        }
        cache.keys = keys;
        cache.loaded_at = Some(Instant::now());
        cache.keys.get(kid).cloned().ok_or(AuthError::Unauthorized)
    }
}

#[derive(Default)]
struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    last_attempt: Option<Instant>,
    loaded_at: Option<Instant>,
}

#[derive(Deserialize)]
struct KeySet {
    keys: Vec<SigningKey>,
}

#[derive(Deserialize)]
struct SigningKey {
    kty: String,
    #[serde(rename = "use")]
    usage: Option<String>,
    alg: Option<String>,
    issuer: Option<String>,
    kid: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

#[derive(Deserialize)]
struct AccessClaims {
    aud: String,
    ver: String,
    tid: String,
    azp: String,
    oid: String,
    scp: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    Unauthorized,
    Forbidden,
    InsufficientScope,
    KeyServiceUnavailable,
}

pub fn www_authenticate_header(config: &OAuthConfig) -> String {
    format!(
        r#"Bearer realm="MCP", resource_metadata="{}/.well-known/oauth-protected-resource", scope="{} offline_access""#,
        config.public_base_url,
        config.resource_scope()
    )
}

#[get("/.well-known/oauth-protected-resource")]
pub fn protected_resource_metadata(
    config: &State<Option<OAuthConfig>>,
) -> Result<Json<Value>, Status> {
    let config = config.inner().as_ref().ok_or(Status::ServiceUnavailable)?;
    Ok(Json(json!({
        "resource": format!("{}/mcp", config.public_base_url),
        // Entra's OAuth 2.1 discovery advertises S256. Access tokens for the
        // registered v2 API still identify the tenant's /v2.0 issuer.
        "authorization_servers": [format!("{ENTRA_AUTHORITY}/{}/oauth21/2026-08", config.tenant_id)],
        "bearer_methods_supported": ["header"],
        "scopes_supported": [config.resource_scope(), "offline_access"],
    })))
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
