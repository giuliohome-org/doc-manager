use super::*;
use crate::{mcp, storage::ContainerClient, AzureClient};
use aws_lc_rs::{
    rand::SystemRandom,
    rsa::{KeyPair, KeySize},
    signature::{KeyPair as _, RSA_PKCS1_SHA256},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rocket::http::Header;
use rocket::local::asynchronous::Client as RocketClient;
use rocket::tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use std::time::{SystemTime, UNIX_EPOCH};

const TENANT: &str = "00000000-0000-4000-8000-000000000001";
const API: &str = "00000000-0000-4000-8000-000000000002";
const CONNECTOR: &str = "00000000-0000-4000-8000-000000000003";
const USER: &str = "00000000-0000-4000-8000-000000000004";
const BASE: &str = "https://docs.example.test";

fn claims() -> Value {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    json!({"aud":API, "iss":format!("{ENTRA_AUTHORITY}/{TENANT}/v2.0"), "tid":TENANT, "ver":"2.0",
        "azp":CONNECTOR, "oid":USER, "scp":REQUIRED_SCOPE, "nbf":now-30, "exp":now+300})
}

fn token(key: &KeyPair, claims: &Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT","kid":"test-signing-key"}"#);
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap());
    let input = format!("{header}.{payload}");
    let mut signature = vec![0; key.public_modulus_len()];
    key.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        input.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

// Only the public JWKS travels over this loopback connection.
async fn test_issuer() -> (OAuthConfig, KeyPair) {
    let key = KeyPair::generate(KeySize::Rsa2048).unwrap();
    let public = key.public_key();
    let keys = json!({"keys":[{"kty":"RSA", "use":"sig", "kid":"test-signing-key",
        "n":URL_SAFE_NO_PAD.encode(public.modulus().big_endian_without_leading_zero()),
        "e":URL_SAFE_NO_PAD.encode(public.exponent().big_endian_without_leading_zero())}]});
    let body = keys.to_string();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let url = format!("http://{}/keys", listener.local_addr().unwrap());
    rocket::tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = [0; 2048];
        let count = stream.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..count]).to_ascii_lowercase();
        assert!(request.starts_with("get /keys "));
        assert!(!request.contains("authorization:"));
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream.write_all(response.as_bytes()).await.unwrap();
    });
    (
        OAuthConfig::new(BASE, TENANT, API, CONNECTOR, USER)
            .unwrap()
            .with_test_jwks(url),
        key,
    )
}

#[test]
fn require_separate_apps_and_valid_subject_allowlist() {
    assert!(OAuthConfig::new(BASE, TENANT, API, API, USER).is_err());
    assert!(OAuthConfig::new(BASE, TENANT, API, CONNECTOR, "").is_err());
    assert!(OAuthConfig::new(BASE, TENANT, API, CONNECTOR, "github-user").is_err());
    assert!(OAuthConfig::new("http://docs.example.test", TENANT, API, CONNECTOR, USER).is_err());
}

#[rocket::async_test]
async fn validates_signature_lifetime_authority_and_delegation() {
    let (config, key) = test_issuer().await;
    assert_eq!(
        config.validate_bearer(&token(&key, &claims())).await,
        Ok(())
    );
    let other_key = KeyPair::generate(KeySize::Rsa2048).unwrap();
    assert_eq!(
        config.validate_bearer(&token(&other_key, &claims())).await,
        Err(AuthError::Unauthorized)
    );
    assert_eq!(
        config.validate_bearer("opaque-provider-token").await,
        Err(AuthError::Unauthorized)
    );
    for (claim, value, expected) in [
        (
            "iss",
            json!("https://other.example"),
            AuthError::Unauthorized,
        ),
        ("aud", json!(CONNECTOR), AuthError::Unauthorized),
        ("tid", json!(CONNECTOR), AuthError::Unauthorized),
        ("ver", json!("1.0"), AuthError::Unauthorized),
        ("exp", json!(1), AuthError::Unauthorized),
        ("nbf", json!(4_000_000_000_u64), AuthError::Unauthorized),
        ("azp", json!(USER), AuthError::Forbidden),
        ("oid", json!(CONNECTOR), AuthError::Forbidden),
        (
            "scp",
            json!("mcp.access.extra"),
            AuthError::InsufficientScope,
        ),
        ("scp", Value::Null, AuthError::InsufficientScope),
    ] {
        let mut invalid = claims();
        invalid[claim] = value;
        assert_eq!(
            config.validate_bearer(&token(&key, &invalid)).await,
            Err(expected),
            "{claim}"
        );
    }
}

async fn rpc(client: &RocketClient, bearer: &str, method: &str, params: Value) -> Value {
    let res = client
        .post("/mcp")
        .header(Header::new("Authorization", format!("Bearer {bearer}")))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Ok);
    let reply = res.into_json::<Value>().await.unwrap();
    assert!(reply["error"].is_null(), "{reply}");
    reply["result"].clone()
}

#[rocket::async_test]
async fn connector_discovers_authenticates_and_calls_tools() {
    let (config, key) = test_issuer().await;
    let client = RocketClient::tracked(
        rocket::build()
            .manage(Some(config))
            .manage(mcp::PublicIntrospection(false))
            .manage(AzureClient {
                container_client: ContainerClient::in_memory(),
            })
            .mount(
                "/",
                routes![protected_resource_metadata, mcp::mcp_get, mcp::mcp_post],
            ),
    )
    .await
    .unwrap();
    let unauth = client
        .post("/mcp")
        .json(&json!({"jsonrpc":"2.0","method":"initialize","id":1}))
        .dispatch()
        .await;
    assert_eq!(unauth.status(), Status::Unauthorized);
    let challenge = unauth.headers().get_one("WWW-Authenticate").unwrap();
    assert!(challenge.contains(&format!(
        r#"resource_metadata="{BASE}/.well-known/oauth-protected-resource""#
    )));
    assert!(challenge.contains(&format!(r#"scope="{BASE}/mcp/mcp.access offline_access""#)));
    let metadata = client
        .get("/.well-known/oauth-protected-resource")
        .dispatch()
        .await
        .into_json::<Value>()
        .await
        .unwrap();
    assert_eq!(metadata["resource"], format!("{BASE}/mcp"));
    assert_eq!(
        metadata["authorization_servers"][0],
        format!("{ENTRA_AUTHORITY}/{TENANT}/oauth21/2026-08")
    );
    assert_eq!(
        metadata["scopes_supported"],
        json!([format!("{BASE}/mcp/mcp.access"), "offline_access"])
    );
    let mut wrong = claims();
    wrong["aud"] = json!(CONNECTOR);
    let res = client
        .post("/mcp")
        .header(Header::new(
            "Authorization",
            format!("Bearer {}", token(&key, &wrong)),
        ))
        .json(&json!({"id":1,"method":"tools/list"}))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Unauthorized);
    let mut insufficient = claims();
    insufficient["scp"] = json!("other.scope");
    let res = client
        .post("/mcp")
        .header(Header::new(
            "Authorization",
            format!("Bearer {}", token(&key, &insufficient)),
        ))
        .json(&json!({"id":1,"method":"tools/list"}))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Forbidden);
    assert!(res
        .headers()
        .get_one("WWW-Authenticate")
        .unwrap()
        .contains(r#"error="insufficient_scope""#));
    let bearer = token(&key, &claims());
    let init = rpc(&client, &bearer, "initialize", json!({})).await;
    assert_eq!(init["serverInfo"]["name"], "doc-manager");
    let list = rpc(&client, &bearer, "tools/list", json!({})).await;
    assert!(list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "get_document"));
    let created = rpc(
        &client,
        &bearer,
        "tools/call",
        json!({"name":"create_document","arguments":{"title":"smoke","content":"test data"}}),
    )
    .await;
    let doc: Value = serde_json::from_str(created["content"][0]["text"].as_str().unwrap()).unwrap();
    let retrieved = rpc(
        &client,
        &bearer,
        "tools/call",
        json!({"name":"get_document","arguments":{"id":doc["id"]}}),
    )
    .await;
    let result: Value =
        serde_json::from_str(retrieved["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(result["content"], "test data");
}
