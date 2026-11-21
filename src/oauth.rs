//! GitHub authenticates the user. Doc Manager issues opaque tokens for its own MCP resource.
use aws_lc_rs::aead::{Aad, Nonce, RandomizedNonceKey, AES_256_GCM_SIV, NONCE_LEN};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use reqwest::{Client, Url};
use rocket::{
    http::Status,
    serde::json::{json, Json, Value},
    State,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    borrow::Cow,
    env,
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

// Only compile-time event names can be logged. Each call site emits at most once
// per 30 seconds per replica, including when several requests arrive together.
macro_rules! auth_event {
    ($label:literal) => {{
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        if $crate::oauth::sample_event(&NEXT) {
            println!("{}", concat!("doc_manager_oauth ", $label));
        }
    }};
}
pub(crate) use auth_event;

pub(crate) fn sample_event(next: &AtomicU64) -> bool {
    static START: OnceLock<Instant> = OnceLock::new();
    let seconds = START.get_or_init(Instant::now).elapsed().as_secs();
    let previous = next.load(Ordering::Relaxed);
    seconds >= previous
        && next
            .compare_exchange(previous, seconds + 30, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

#[path = "oauth_exchange.rs"]
pub(crate) mod exchange;
#[path = "oauth_flow.rs"]
pub(crate) mod flow;

const ACCESS_TTL: u64 = 900;
const SCOPE: &str = "mcp.access";

pub struct OAuthConfig {
    base: String,
    client_id: String,
    redirect_uri: String,
    additional_redirect_uri: Option<String>,
    allowed_ids: Vec<u64>,
    key: RandomizedNonceKey,
    github_authorize: String,
    github_token: String,
    github_user: String,
    http: Client,
}

#[derive(Clone, Copy)]
struct RegisteredRedirect<'a> {
    uri: &'a str,
    primary: bool,
}

impl RegisteredRedirect<'_> {
    // Existing grants use the original AEAD context, which already contains the
    // primary URI. Alternate grants also authenticate their exact callback URI.
    fn purpose<'a>(self, purpose: &'a str) -> Cow<'a, str> {
        if self.primary {
            Cow::Borrowed(purpose)
        } else {
            Cow::Owned(format!("{purpose}\0{}", self.uri))
        }
    }
}

fn web_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "invalid OAuth URL")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || local && url.scheme() == "http")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err("OAuth URLs must use HTTPS (HTTP allowed on loopback), with no user information, query or fragment".into());
    }
    Ok(url)
}

impl OAuthConfig {
    pub fn from_env() -> Result<Option<Self>, String> {
        match env::var("OAUTH_PROVIDER").as_deref() {
            Err(env::VarError::NotPresent) | Ok("") => return Ok(None),
            Ok("github") => (),
            _ => return Err("OAUTH_PROVIDER must be github".into()),
        }
        let required =
            |name| env::var(name).map_err(|_| format!("{name} is required for MCP OAuth"));
        let additional = match env::var("OAUTH_CONNECTOR_ADDITIONAL_REDIRECT_URI") {
            Ok(uri) => Some(uri),
            Err(env::VarError::NotPresent) => None,
            Err(_) => return Err("invalid OAUTH_CONNECTOR_ADDITIONAL_REDIRECT_URI".into()),
        };
        Self::new(
            &required("OAUTH_PUBLIC_BASE_URL")?,
            &required("OAUTH_GITHUB_CLIENT_ID")?,
            &required("OAUTH_CONNECTOR_REDIRECT_URI")?,
            additional.as_deref(),
            &required("OAUTH_GITHUB_USER_IDS")?,
            &required("OAUTH_TOKEN_KEY")?,
        )
        .map(Some)
    }

    fn new(
        base: &str,
        client: &str,
        redirect: &str,
        additional: Option<&str>,
        users: &str,
        key: &str,
    ) -> Result<Self, String> {
        let base = web_url(base)?;
        if base.path() != "/" {
            return Err("OAUTH_PUBLIC_BASE_URL must be an origin".into());
        }
        let additional = additional.filter(|uri| !uri.is_empty() && *uri != redirect);
        for uri in std::iter::once(redirect).chain(additional) {
            web_url(uri)?;
            if uri.contains('*') {
                return Err("OAuth redirect URIs must be exact".into());
            }
        }
        if client.is_empty()
            || client.len() > 128
            || !client.bytes().all(|x| x.is_ascii_alphanumeric())
        {
            return Err("invalid GitHub OAuth client ID".into());
        }
        let allowed_ids = users
            .split(',')
            .map(|s| {
                s.trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|id| *id > 0)
                    .ok_or("OAUTH_GITHUB_USER_IDS must contain numeric GitHub account IDs")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let secret = STANDARD
            .decode(key)
            .map_err(|_| "invalid base64 OAUTH_TOKEN_KEY")?;
        let key = RandomizedNonceKey::new(&AES_256_GCM_SIV, &secret)
            .map_err(|_| "OAUTH_TOKEN_KEY must encode 32 random bytes")?;
        let http = Client::builder()
            .user_agent(concat!("doc-manager/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "could not initialize GitHub client")?;
        Ok(Self {
            base: base.origin().ascii_serialization(),
            client_id: client.into(),
            redirect_uri: redirect.into(),
            additional_redirect_uri: additional.map(str::to_owned),
            allowed_ids,
            key,
            http,
            github_authorize: "https://github.com/login/oauth/authorize".into(),
            github_token: "https://github.com/login/oauth/access_token".into(),
            github_user: "https://api.github.com/user".into(),
        })
    }

    fn resource(&self) -> String {
        format!("{}/mcp", self.base)
    }

    fn redirects(&self) -> impl Iterator<Item = RegisteredRedirect<'_>> {
        std::iter::once(RegisteredRedirect {
            uri: &self.redirect_uri,
            primary: true,
        })
        .chain(
            self.additional_redirect_uri
                .as_deref()
                .map(|uri| RegisteredRedirect {
                    uri,
                    primary: false,
                }),
        )
    }

    // Authenticating the configuration as AAD binds every grant to its issuer, exact audience,
    // registered client, callback and token type, including across replicas sharing a key.
    fn context(&self, purpose: &str) -> Aad<Vec<u8>> {
        Aad::from(
            format!(
                "doc-manager-v1\0{}\0{}\0{}\0{}\0{purpose}",
                self.base,
                self.resource(),
                self.client_id,
                self.redirect_uri
            )
            .into_bytes(),
        )
    }

    fn seal<T: Serialize>(&self, purpose: &str, data: &T, ttl: u64) -> Result<String, ()> {
        let exp = now()?.checked_add(ttl).ok_or(())?;
        let mut ciphertext = serde_json::to_vec(&Envelope { exp, data }).map_err(|_| ())?;
        let nonce = self
            .key
            .seal_in_place_append_tag(self.context(purpose), &mut ciphertext)
            .map_err(|_| ())?;
        let mut token = nonce.as_ref().to_vec();
        token.extend_from_slice(&ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(token))
    }

    fn open<T: DeserializeOwned>(&self, purpose: &str, token: &str) -> Result<T, ()> {
        if token.len() > 16_384 {
            return Err(());
        }
        let mut wire = URL_SAFE_NO_PAD.decode(token).map_err(|_| ())?;
        let (nonce, ciphertext) = wire.split_at_mut_checked(NONCE_LEN).ok_or(())?;
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| ())?;
        let plaintext = self
            .key
            .open_in_place(nonce, self.context(purpose), ciphertext)
            .map_err(|_| ())?;
        let envelope: Envelope<T> = serde_json::from_slice(plaintext).map_err(|_| ())?;
        if envelope.exp <= now()? {
            return Err(());
        }
        Ok(envelope.data)
    }

    fn issue_access(&self, id: u64) -> Result<String, ()> {
        if !self.allowed_ids.contains(&id) {
            return Err(());
        }
        self.seal(
            "access",
            &Access {
                user_id: id,
                scope: SCOPE.to_owned(),
            },
            ACCESS_TTL,
        )
    }

    pub fn validate_bearer(&self, token: &str) -> Result<(), AuthError> {
        let access: Access = self
            .open("access", token)
            .map_err(|_| AuthError::Unauthorized)?;
        if !self.allowed_ids.contains(&access.user_id) {
            return Err(AuthError::Forbidden);
        }
        if access.scope != SCOPE {
            return Err(AuthError::InsufficientScope);
        }
        Ok(())
    }
}

fn now() -> Result<u64, ()> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| ())
}

#[derive(Deserialize, Serialize)]
struct Envelope<T> {
    exp: u64,
    data: T,
}
#[derive(Deserialize, Serialize)]
struct Access {
    user_id: u64,
    scope: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AuthError {
    Unauthorized,
    Forbidden,
    InsufficientScope,
}

pub fn www_authenticate_header(config: &OAuthConfig) -> String {
    format!(
        r#"Bearer realm="Doc Manager", resource_metadata="{}/.well-known/oauth-protected-resource/mcp", scope="mcp.access offline_access""#,
        config.base
    )
}

#[get("/.well-known/oauth-protected-resource")]
pub fn protected_resource_metadata(
    config: &State<Option<OAuthConfig>>,
) -> Result<Json<Value>, Status> {
    let c = config.inner().as_ref().ok_or(Status::ServiceUnavailable)?;
    auth_event!("discovery.resource");
    Ok(Json(
        json!({"resource":c.resource(), "resource_name":"Doc Manager",
        "authorization_servers":[c.base], "bearer_methods_supported":["header"],
        "scopes_supported":[SCOPE, "offline_access"]}),
    ))
}

#[get("/.well-known/oauth-protected-resource/mcp")]
pub fn mcp_resource_metadata(config: &State<Option<OAuthConfig>>) -> Result<Json<Value>, Status> {
    protected_resource_metadata(config)
}

#[get("/.well-known/oauth-authorization-server")]
pub fn authorization_server_metadata(
    config: &State<Option<OAuthConfig>>,
) -> Result<Json<Value>, Status> {
    let c = config.inner().as_ref().ok_or(Status::ServiceUnavailable)?;
    auth_event!("discovery.issuer");
    Ok(Json(
        json!({"issuer":c.base, "authorization_endpoint":format!("{}/mcp/oauth/authorize",c.base),
        "token_endpoint":format!("{}/mcp/oauth/token",c.base), "response_types_supported":["code"],
        "authorization_response_iss_parameter_supported":true,
        "grant_types_supported":["authorization_code","refresh_token"],
        "token_endpoint_auth_methods_supported":["client_secret_basic","client_secret_post"],
        "code_challenge_methods_supported":["S256"], "scopes_supported":[SCOPE, "offline_access"]}),
    ))
}

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
