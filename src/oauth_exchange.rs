//! Exchange a GitHub authorization grant for a Doc Manager access token.

use super::{flow::Grant, OAuthConfig, ACCESS_TTL};
use aws_lc_rs::digest::{digest, SHA256};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use rocket::data::{Data, ToByteUnit};
use rocket::http::{ContentType, RawStr, Status};
use rocket::request::{FromRequest, Outcome};
use rocket::response::{self, Responder};
use rocket::serde::json::{json, Json, Value};
use rocket::{Request, Response, State};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub struct TokenHeaders<'r> {
    basic: Option<&'r str>,
    valid: bool,
}

#[rocket::async_trait]
impl<'r> FromRequest<'r> for TokenHeaders<'r> {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let mut auth = req.headers().get("Authorization");
        let basic = auth.next();
        let valid = auth.next().is_none()
            && req
                .content_type()
                .is_some_and(|ct| ct == &ContentType::Form);
        Outcome::Success(Self { basic, valid })
    }
}

pub struct TokenResponse(Status, Value);

impl<'r> Responder<'r, 'static> for TokenResponse {
    fn respond_to(self, request: &'r Request<'_>) -> response::Result<'static> {
        let mut response = Response::build_from(Json(self.1).respond_to(request)?);
        response
            .status(self.0)
            .raw_header("Cache-Control", "no-store")
            .raw_header("Pragma", "no-cache");
        if self.0 == Status::Unauthorized {
            response.raw_header("WWW-Authenticate", "Basic realm=\"Doc Manager\"");
        }
        response.ok()
    }
}

fn error(name: &str) -> TokenResponse {
    match name {
        "invalid_client" => auth_event!("token.invalid_client"),
        "invalid_grant" => auth_event!("token.invalid_grant"),
        "invalid_request" => auth_event!("token.invalid_request"),
        "invalid_scope" => auth_event!("token.invalid_scope"),
        "temporarily_unavailable" => auth_event!("token.unavailable"),
        "server_error" => auth_event!("token.server_error"),
        _ => (),
    }
    let status = match name {
        "invalid_client" => Status::Unauthorized,
        "temporarily_unavailable" => Status::ServiceUnavailable,
        "server_error" => Status::InternalServerError,
        _ => Status::BadRequest,
    };
    TokenResponse(status, json!({ "error": name }))
}

struct Params(HashMap<String, String>);

impl Params {
    fn parse(body: &str) -> Result<Self, TokenResponse> {
        let mut params = HashMap::new();
        for field in body.split('&').filter(|field| !field.is_empty()) {
            let (name, value) = field.split_once('=').unwrap_or((field, ""));
            let name = RawStr::new(name)
                .url_decode()
                .map_err(|_| error("invalid_request"))?;
            let value = RawStr::new(value)
                .url_decode()
                .map_err(|_| error("invalid_request"))?;
            if params
                .insert(name.into_owned(), value.into_owned())
                .is_some()
            {
                return Err(error("invalid_request"));
            }
        }
        Ok(Self(params))
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    fn require(&self, name: &str) -> Result<&str, TokenResponse> {
        self.get(name)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| error("invalid_request"))
    }

    // Returns the presented client ID and secret. The caller selects the
    // registered client; GitHub verifies the secret during the exchange.
    fn credentials(&self, basic: Option<&str>) -> Result<(String, String), TokenResponse> {
        let (client_id, secret) = if let Some(header) = basic {
            if self.get("client_secret").is_some() {
                return Err(error("invalid_request"));
            }
            let (scheme, encoded) = header
                .split_once(' ')
                .ok_or_else(|| error("invalid_client"))?;
            if !scheme.eq_ignore_ascii_case("Basic") || encoded.len() > 4096 {
                return Err(error("invalid_client"));
            }
            let bytes = STANDARD
                .decode(encoded)
                .map_err(|_| error("invalid_client"))?;
            let credentials = std::str::from_utf8(&bytes).map_err(|_| error("invalid_client"))?;
            let (id, password) = credentials
                .split_once(':')
                .ok_or_else(|| error("invalid_client"))?;
            let decode = |s: &str| {
                RawStr::new(s)
                    .url_decode()
                    .map(|v| v.into_owned())
                    .map_err(|_| error("invalid_client"))
            };
            let id = decode(id)?;
            // A repeated public identifier is not a second authentication method.
            if self.get("client_id").is_some_and(|body_id| body_id != id) {
                return Err(error("invalid_request"));
            }
            (id, decode(password)?)
        } else {
            (
                self.get("client_id").unwrap_or_default().to_owned(),
                self.get("client_secret").unwrap_or_default().to_owned(),
            )
        };
        if client_id.is_empty() || secret.is_empty() {
            return Err(error("invalid_client"));
        }
        Ok((client_id, secret))
    }
}

fn matches_challenge(verifier: &str, challenge: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        && URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes())) == challenge
}

#[derive(Deserialize, Serialize)]
struct RefreshGrant {
    token: String,
    user: u64,
}

#[derive(Deserialize)]
struct GithubToken {
    access_token: String,
    token_type: String,
    refresh_token: Option<String>,
    refresh_token_expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct GithubUser {
    id: u64,
}

async fn exchange(
    config: &OAuthConfig,
    params: Params,
    basic: Option<&str>,
) -> Result<TokenResponse, TokenResponse> {
    let (client_id, secret) = params.credentials(basic)?;
    let config = config
        .client(&client_id)
        .ok_or_else(|| error("invalid_client"))?;
    if params.get("resource") != Some(&config.resource()) {
        if params.get("resource").is_some() {
            auth_event!("token.wrong_resource");
        } else {
            auth_event!("token.missing_resource");
        }
        return Err(error("invalid_target"));
    }
    let mut upstream = vec![
        ("client_id", config.client_id.clone()),
        ("client_secret", secret),
    ];
    let (offline, expected_user) = match params.require("grant_type")? {
        "authorization_code" => {
            let requested = params.require("redirect_uri")?;
            let Some(registered) = config.redirects().find(|r| r.uri == requested) else {
                auth_event!("token.redirect_mismatch");
                return Err(error("invalid_grant"));
            };
            let grant: Grant = config
                .open(&registered.purpose("code"), params.require("code")?)
                .map_err(|_| error("invalid_grant"))?;
            if !matches_challenge(params.require("code_verifier")?, &grant.challenge) {
                auth_event!("token.pkce_mismatch");
                return Err(error("invalid_grant"));
            }
            upstream.extend([
                ("code", grant.upstream_code),
                ("code_verifier", grant.upstream_verifier),
                (
                    "redirect_uri",
                    format!("{}/mcp/oauth/callback", config.base),
                ),
            ]);
            (grant.offline_access, None)
        }
        "refresh_token" => {
            let grant: RefreshGrant = config
                .open("refresh", params.require("refresh_token")?)
                .map_err(|_| error("invalid_grant"))?;
            if !config.allowed_ids.contains(&grant.user) {
                auth_event!("token.refresh_identity_rejected");
                return Err(error("invalid_grant"));
            }
            let mut offline = true;
            if let Some(scope) = params.get("scope") {
                let scopes: Vec<_> = scope.split_ascii_whitespace().collect();
                if !scopes.contains(&"mcp.access")
                    || scopes
                        .iter()
                        .any(|s| !matches!(*s, "mcp.access" | "offline_access"))
                {
                    return Err(error("invalid_scope"));
                }
                offline = scopes.contains(&"offline_access");
            }
            upstream.extend([
                ("grant_type", "refresh_token".to_owned()),
                ("refresh_token", grant.token),
            ]);
            (offline, Some(grant.user))
        }
        _ => return Err(error("unsupported_grant_type")),
    };

    // GitHub authenticates the registered confidential client and atomically consumes
    // its single-use code/refresh token, including across concurrent ACA replicas.
    let github = config
        .http
        .post(&config.github_token)
        .header("Accept", "application/json")
        .form(&upstream)
        .send()
        .await
        .map_err(|_| error("temporarily_unavailable"))?;
    if github.status().is_server_error() || github.status().as_u16() == 429 {
        return Err(error("temporarily_unavailable"));
    }
    let status = github.status();
    let body: Value = github
        .json()
        .await
        .map_err(|_| error("temporarily_unavailable"))?;
    if let Some(code) = body.get("error").and_then(Value::as_str) {
        let kind = match code {
            "incorrect_client_credentials" | "invalid_client" => "invalid_client",
            "server_error" | "temporarily_unavailable" => "temporarily_unavailable",
            _ => "invalid_grant",
        };
        return Err(error(kind));
    }
    if !status.is_success() {
        return Err(error("temporarily_unavailable"));
    }
    let token: GithubToken =
        serde_json::from_value(body).map_err(|_| error("temporarily_unavailable"))?;
    if !token.token_type.eq_ignore_ascii_case("bearer") || token.access_token.is_empty() {
        return Err(error("invalid_grant"));
    }
    let identity = config
        .http
        .get(&config.github_user)
        .header("User-Agent", "Doc-Manager-OAuth")
        .header("Accept", "application/vnd.github+json")
        .bearer_auth(token.access_token)
        .send()
        .await
        .map_err(|_| error("temporarily_unavailable"))?;
    if identity.status().as_u16() == 401 {
        return Err(error("invalid_grant"));
    }
    if !identity.status().is_success() {
        return Err(error("temporarily_unavailable"));
    }
    let user: GithubUser = identity
        .json()
        .await
        .map_err(|_| error("temporarily_unavailable"))?;
    if !config.allowed_ids.contains(&user.id) || expected_user.is_some_and(|id| id != user.id) {
        auth_event!("token.identity_rejected");
        return Err(error("invalid_grant"));
    }

    let access = config
        .issue_access(user.id)
        .map_err(|_| error("server_error"))?;
    let mut result = json!({ "access_token": access, "token_type": "Bearer", "expires_in": ACCESS_TTL, "scope": "mcp.access" });
    // A narrower access-token scope on refresh does not narrow the original
    // refresh grant (RFC 6749 §6). GitHub invalidates its old refresh token.
    if offline || expected_user.is_some() {
        if let (Some(refresh), Some(ttl)) = (
            token.refresh_token.filter(|s| !s.is_empty()),
            token.refresh_token_expires_in.filter(|ttl| *ttl > 0),
        ) {
            let refresh = config
                .seal(
                    "refresh",
                    &RefreshGrant {
                        token: refresh,
                        user: user.id,
                    },
                    ttl.min(180 * 24 * 3600),
                )
                .map_err(|_| error("server_error"))?;
            result["refresh_token"] = json!(refresh);
            if offline {
                result["scope"] = json!("mcp.access offline_access");
            }
        }
    }
    if result.get("refresh_token").is_some() {
        auth_event!("token.issued_with_refresh");
    } else {
        auth_event!("token.issued_access_only");
    }
    Ok(TokenResponse(Status::Ok, result))
}

#[post("/mcp/oauth/token", data = "<data>")]
pub async fn token(
    config: &State<Option<OAuthConfig>>,
    headers: TokenHeaders<'_>,
    data: Data<'_>,
) -> TokenResponse {
    auth_event!("token.received");
    let Some(config) = config.inner().as_ref() else {
        return error("temporarily_unavailable");
    };
    if !headers.valid {
        return error("invalid_request");
    }
    let body = match data.open(16.kibibytes()).into_string().await {
        Ok(body) if body.is_complete() => body,
        _ => return error("invalid_request"),
    };
    let params = match Params::parse(&body) {
        Ok(params) => params,
        Err(response) => return response,
    };
    match params.get("grant_type") {
        Some("authorization_code") => auth_event!("token.authorization_code"),
        Some("refresh_token") => auth_event!("token.refresh"),
        _ => auth_event!("token.unknown_grant_type"),
    }
    exchange(config, params, headers.basic)
        .await
        .unwrap_or_else(|response| response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_s256_and_verifier_syntax() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert!(matches_challenge(
            verifier,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        ));
        assert!(!matches_challenge(verifier, verifier));
        assert!(!matches_challenge(
            "x",
            &URL_SAFE_NO_PAD.encode(digest(&SHA256, b"x"))
        ));
    }

    #[test]
    fn rejects_duplicate_params_and_mixed_client_authentication() {
        assert!(Params::parse("resource=a&%72esource=b").is_err());
        let empty = Params::parse("").ok().unwrap();
        let basic = format!("Basic {}", STANDARD.encode("client:p%3A%2Bb"));
        let presented = |id: &str, secret: &str| Some((id.to_owned(), secret.to_owned()));
        assert_eq!(
            empty.credentials(Some(&basic)).ok(),
            presented("client", "p:+b")
        );
        assert!(empty.credentials(None).is_err());
        let repeated = Params::parse("client_id=client").ok().unwrap();
        assert_eq!(
            repeated.credentials(Some(&basic)).ok(),
            presented("client", "p:+b")
        );
        assert!(repeated.credentials(None).is_err());
        let conflicting = Params::parse("client_id=another-client").ok().unwrap();
        assert!(conflicting.credentials(Some(&basic)).is_err());
        let body = Params::parse("client_id=client&client_secret=other")
            .ok()
            .unwrap();
        assert!(body.credentials(Some(&basic)).is_err());
        assert_eq!(body.credentials(None).ok(), presented("client", "other"));
    }
}
