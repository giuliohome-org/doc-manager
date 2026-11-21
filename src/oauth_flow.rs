use super::OAuthConfig;
use aws_lc_rs::{
    digest,
    rand::{SecureRandom, SystemRandom},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use reqwest::Url;
use rocket::http::{uri::Origin, Cookie, CookieJar, SameSite, Status};
use rocket::response::{self, Responder, Response};
use rocket::{Request, State};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const TRANSACTION_TTL: u64 = 600;
const CODE_TTL: u64 = 120;

#[derive(Serialize, Deserialize)]
struct Transaction {
    state: String,
    challenge: String,
    upstream_verifier: String,
    cookie_digest: String,
    offline_access: bool,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Grant {
    pub upstream_code: String,
    pub upstream_verifier: String,
    pub challenge: String,
    pub offline_access: bool,
}

type RedirectResult = Result<Url, Status>;

// These redirects carry OAuth codes or state. Set these headers on errors too.
pub(crate) struct FlowResponse(RedirectResult);

impl<'r> Responder<'r, 'static> for FlowResponse {
    fn respond_to(self, _: &'r Request<'_>) -> response::Result<'static> {
        let mut response = Response::build();
        response
            .raw_header("Cache-Control", "no-store")
            .raw_header("Pragma", "no-cache")
            .raw_header("Referrer-Policy", "no-referrer");
        match self.0 {
            Ok(url) => {
                response
                    .status(Status::Found)
                    .raw_header("Location", url.to_string());
            }
            Err(status) => {
                response.status(status);
            }
        }
        response.ok()
    }
}

fn parameters<'a>(uri: &'a Origin<'_>) -> Result<HashMap<&'a str, &'a str>, Status> {
    let query = uri.query().ok_or(Status::BadRequest)?;
    if query.as_str().len() > 8192 {
        return Err(Status::BadRequest);
    }
    let mut params = HashMap::new();
    for (key, value) in query.segments() {
        if params.insert(key, value).is_some() {
            return Err(Status::BadRequest);
        }
    }
    Ok(params)
}

fn random_secret() -> Result<String, Status> {
    let mut bytes = [0; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| Status::InternalServerError)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn s256(input: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, input.as_bytes()))
}

fn valid_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && URL_SAFE_NO_PAD
            .decode(challenge)
            .is_ok_and(|v| v.len() == 32)
}

fn scope_request(scope: Option<&str>) -> Option<bool> {
    let mut has_access = false;
    let mut offline = false;
    for item in scope.unwrap_or("mcp.access").split(' ') {
        match item {
            "mcp.access" => has_access = true,
            "offline_access" => offline = true,
            _ => return None,
        }
    }
    has_access.then_some(offline)
}

fn client_redirect(config: &OAuthConfig, state: &str, key: &str, value: &str) -> RedirectResult {
    let mut redirect = Url::parse(&config.redirect_uri).map_err(|_| Status::InternalServerError)?;
    redirect
        .query_pairs_mut()
        .append_pair(key, value)
        .append_pair("state", state)
        .append_pair("iss", &config.base);
    Ok(redirect)
}

// A unique cookie per transaction lets multiple concurrent logins finish in any order.
// The __Host- prefix prevents sibling domains setting an impersonating domain cookie.
fn cookie_name(upstream_verifier: &str) -> String {
    format!("__Host-docmanager-oauth-{}", &s256(upstream_verifier)[..22])
}

fn start(config: &OAuthConfig, uri: &Origin<'_>, cookies: &CookieJar<'_>) -> RedirectResult {
    let p = parameters(uri)?;
    let param = |key: &str| p.get(key).copied();
    if param("client_id") != Some(config.client_id.as_str())
        || param("redirect_uri") != Some(config.redirect_uri.as_str())
    {
        return Err(Status::BadRequest);
    }
    let state = param("state")
        .filter(|s| {
            !s.is_empty() && s.len() <= 2048 && s.bytes().all(|b| (0x20..=0x7e).contains(&b))
        })
        .ok_or(Status::BadRequest)?;
    let reject = |error: &str| client_redirect(config, state, "error", error);
    if param("response_type") != Some("code") {
        return reject("unsupported_response_type");
    }
    if param("resource") != Some(&config.resource()) {
        return reject("invalid_target");
    }
    let Some(challenge) = param("code_challenge").filter(|s| valid_challenge(s)) else {
        return reject("invalid_request");
    };
    if param("code_challenge_method") != Some("S256") {
        return reject("invalid_request");
    }
    let Some(offline_access) = scope_request(param("scope")) else {
        return reject("invalid_scope");
    };

    let cookie_secret = random_secret()?;
    let transaction = Transaction {
        state: state.to_owned(),
        challenge: challenge.to_owned(),
        upstream_verifier: random_secret()?,
        cookie_digest: s256(&cookie_secret),
        offline_access,
    };
    let sealed_state = config
        .seal("state", &transaction, TRANSACTION_TTL)
        .map_err(|_| Status::InternalServerError)?;
    let mut github =
        Url::parse(&config.github_authorize).map_err(|_| Status::InternalServerError)?;
    let github_scope = if offline_access {
        "read:user offline_access"
    } else {
        "read:user"
    };
    github
        .query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair(
            "redirect_uri",
            &format!("{}/mcp/oauth/callback", config.base),
        )
        .append_pair("scope", github_scope)
        .append_pair("state", &sealed_state)
        .append_pair("code_challenge_method", "S256")
        .append_pair("code_challenge", &s256(&transaction.upstream_verifier));
    cookies.add(
        Cookie::build((cookie_name(&transaction.upstream_verifier), cookie_secret))
            .path("/")
            .http_only(true)
            .secure(true)
            .same_site(SameSite::Lax)
            .max_age(rocket::time::Duration::seconds(TRANSACTION_TTL as i64))
            .build(),
    );
    Ok(github)
}

#[get("/mcp/oauth/authorize")]
pub(crate) fn authorize(
    config: &State<Option<OAuthConfig>>,
    uri: &Origin<'_>,
    cookies: &CookieJar<'_>,
) -> FlowResponse {
    FlowResponse(
        config
            .inner()
            .as_ref()
            .ok_or(Status::ServiceUnavailable)
            .and_then(|config| start(config, uri, cookies)),
    )
}

fn finish(config: &OAuthConfig, uri: &Origin<'_>, cookies: &CookieJar<'_>) -> RedirectResult {
    let p = parameters(uri)?;
    let state = p.get("state").ok_or(Status::BadRequest)?;
    let transaction: Transaction = config
        .open("state", state)
        .map_err(|_| Status::BadRequest)?;
    let name = cookie_name(&transaction.upstream_verifier);
    let cookie = cookies.get(&name).ok_or(Status::BadRequest)?;
    if cookie.value().len() != 43 || s256(cookie.value()) != transaction.cookie_digest {
        return Err(Status::BadRequest);
    }
    cookies.remove(
        Cookie::build(name)
            .path("/")
            .secure(true)
            .http_only(true)
            .same_site(SameSite::Lax)
            .build(),
    );
    let redirect = |key: &str, value: &str| client_redirect(config, &transaction.state, key, value);
    if let Some(&error) = p.get("error") {
        if p.contains_key("code") {
            return Err(Status::BadRequest);
        }
        return redirect(
            "error",
            match error {
                "access_denied" => "access_denied",
                "temporarily_unavailable" => "temporarily_unavailable",
                _ => "server_error",
            },
        );
    }
    let code = p
        .get("code")
        .filter(|v| !v.is_empty() && v.len() <= 1024)
        .ok_or(Status::BadRequest)?;
    let grant = Grant {
        upstream_code: (*code).to_owned(),
        upstream_verifier: transaction.upstream_verifier,
        challenge: transaction.challenge,
        offline_access: transaction.offline_access,
    };
    // The token endpoint redeems the embedded GitHub code, which is single-use.
    let sealed = config
        .seal("code", &grant, CODE_TTL)
        .map_err(|_| Status::InternalServerError)?;
    redirect("code", &sealed)
}

#[get("/mcp/oauth/callback")]
pub(crate) fn callback(
    config: &State<Option<OAuthConfig>>,
    uri: &Origin<'_>,
    cookies: &CookieJar<'_>,
) -> FlowResponse {
    FlowResponse(
        config
            .inner()
            .as_ref()
            .ok_or(Status::ServiceUnavailable)
            .and_then(|config| finish(config, uri, cookies)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_ambiguous_query_and_decodes_state_once() {
        let invalid = Origin::parse("/?state=first&%73tate=second").unwrap();
        assert_eq!(parameters(&invalid), Err(Status::BadRequest));
        let valid = Origin::parse("/?state=x%26y%2Bz%252F").unwrap();
        assert_eq!(parameters(&valid).unwrap()["state"], "x&y+z%2F");
    }

    #[test]
    fn downstream_s256_and_scope_validation() {
        assert!(valid_challenge(&s256("an input")));
        assert!(!valid_challenge(&"z".repeat(43)));
        assert!(!valid_challenge(&"a".repeat(42)));
        assert_eq!(scope_request(None), Some(false));
        assert_eq!(scope_request(Some("offline_access mcp.access")), Some(true));
        assert_eq!(scope_request(Some("offline_access")), None);
        assert_eq!(scope_request(Some("mcp.access repo")), None);
    }
}
