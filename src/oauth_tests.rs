use super::*;
use crate::{mcp, storage::ContainerClient, AzureClient};
use aws_lc_rs::digest::{digest, SHA256};
use rocket::http::{ContentType, Cookie, Header};
use rocket::local::asynchronous::{Client as RocketClient, LocalResponse};
use rocket::tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
use std::collections::HashMap;

const BASE: &str = "https://docs.example.test";
const MCP: &str = "https://docs.example.test/mcp";
const CLIENT: &str = "ExampleClient";
const REDIRECT: &str = "https://chat.example.test/connector/oauth/test";
const SECRET: &str = "local-test-secret";
const USER: u64 = 42;
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
const STATE: &str = "state.bound-to-the-connector";

fn config(base: &str, client: &str) -> OAuthConfig {
    OAuthConfig::new(base, client, REDIRECT, "42", &STANDARD.encode([0x11; 32])).unwrap()
}

async fn app(config: OAuthConfig) -> RocketClient {
    RocketClient::untracked(
        rocket::build()
            .manage(Some(config))
            .manage(mcp::PublicIntrospection(false))
            .manage(AzureClient {
                container_client: ContainerClient::in_memory(),
            })
            .mount(
                "/",
                routes![
                    protected_resource_metadata,
                    authorization_server_metadata,
                    flow::authorize,
                    flow::callback,
                    exchange::token,
                    mcp::mcp_get,
                    mcp::mcp_post
                ],
            ),
    )
    .await
    .unwrap()
}

fn fields(pairs: &[(&str, &str)]) -> String {
    let mut url = Url::parse(BASE).unwrap();
    url.query_pairs_mut().extend_pairs(pairs);
    url.query().unwrap().to_string()
}

fn params(url: &Url) -> HashMap<String, String> {
    url.query_pairs().into_owned().collect()
}

async fn authorize<'a>(client: &'a RocketClient, resource: &str) -> LocalResponse<'a> {
    client
        .get(format!(
            "/mcp/oauth/authorize?{}",
            fields(&[
                ("client_id", CLIENT),
                ("response_type", "code"),
                ("resource", resource),
                ("redirect_uri", REDIRECT),
                ("scope", "mcp.access offline_access"),
                ("state", STATE),
                ("code_challenge_method", "S256"),
                ("code_challenge", CHALLENGE),
            ])
        ))
        .dispatch()
        .await
}

fn redirect(response: &LocalResponse<'_>) -> Url {
    assert!(
        response.status().code / 100 == 3,
        "expected redirect; got {}",
        response.status()
    );
    Url::parse(response.headers().get_one("Location").unwrap()).unwrap()
}

async fn token_post<'a>(
    client: &'a RocketClient,
    pairs: &[(&str, &str)],
    basic: bool,
) -> LocalResponse<'a> {
    let mut request = client.post("/mcp/oauth/token").header(ContentType::Form);
    let mut pairs = pairs.to_vec();
    if basic {
        let credentials = STANDARD.encode(format!("{CLIENT}:{SECRET}"));
        request = request.header(Header::new("Authorization", format!("Basic {credentials}")));
    } else {
        pairs.extend([("client_id", CLIENT), ("client_secret", SECRET)]);
    }
    request.body(fields(&pairs)).dispatch().await
}

async fn rpc<'a>(
    client: &'a RocketClient,
    token: &str,
    method: &str,
    args: Value,
) -> LocalResponse<'a> {
    client
        .post("/mcp")
        .header(Header::new("Authorization", format!("Bearer {token}")))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":args}))
        .dispatch()
        .await
}

async fn result(response: LocalResponse<'_>) -> Value {
    assert_eq!(response.status(), Status::Ok);
    let reply = response.into_json::<Value>().await.unwrap();
    assert!(reply["error"].is_null(), "{reply}");
    reply["result"].clone()
}

#[rocket::async_test]
async fn discovers_resource_and_its_authorization_server() {
    let client = app(config(BASE, CLIENT)).await;
    let res = client
        .post("/mcp")
        .json(&json!({"id":1,"method":"initialize"}))
        .dispatch()
        .await;
    assert_eq!(res.status(), Status::Unauthorized);
    let challenge = res.headers().get_one("WWW-Authenticate").unwrap();
    assert!(challenge.contains(&format!(
        r#"resource_metadata="{BASE}/.well-known/oauth-protected-resource""#
    )));
    let resource = client
        .get("/.well-known/oauth-protected-resource")
        .dispatch()
        .await
        .into_json::<Value>()
        .await
        .unwrap();
    assert_eq!(resource["resource"], MCP);
    assert_eq!(resource["authorization_servers"], json!([BASE]));
    assert_eq!(
        resource["scopes_supported"],
        json!(["mcp.access", "offline_access"])
    );
    let metadata = client
        .get("/.well-known/oauth-authorization-server")
        .dispatch()
        .await
        .into_json::<Value>()
        .await
        .unwrap();
    assert_eq!(metadata["issuer"], BASE);
    assert_eq!(
        metadata["authorization_endpoint"],
        format!("{BASE}/mcp/oauth/authorize")
    );
    assert_eq!(
        metadata["token_endpoint"],
        format!("{BASE}/mcp/oauth/token")
    );
    assert_eq!(
        metadata["code_challenge_methods_supported"],
        json!(["S256"])
    );
}

#[rocket::async_test]
async fn mcp_rejects_invalid_audience_client_purpose_lifetime_user_and_scope() {
    let cfg = config(BASE, CLIENT);
    let valid = Access {
        user_id: USER,
        scope: SCOPE.to_owned(),
    };
    let credentials = [
        ("raw-github-bearer".into(), Status::Unauthorized),
        (
            config("https://other.example.test", CLIENT)
                .issue_access(USER)
                .unwrap(),
            Status::Unauthorized,
        ),
        (
            config(BASE, "OtherClient").issue_access(USER).unwrap(),
            Status::Unauthorized,
        ),
        (
            cfg.seal("refresh", &valid, 60).unwrap(),
            Status::Unauthorized,
        ),
        (cfg.seal("access", &valid, 0).unwrap(), Status::Unauthorized),
        (
            cfg.seal(
                "access",
                &Access {
                    user_id: 99,
                    scope: SCOPE.into(),
                },
                60,
            )
            .unwrap(),
            Status::Forbidden,
        ),
        (
            cfg.seal(
                "access",
                &Access {
                    user_id: USER,
                    scope: "other".into(),
                },
                60,
            )
            .unwrap(),
            Status::Forbidden,
        ),
    ];
    let client = app(cfg).await;
    for (token, status) in credentials {
        assert_eq!(
            rpc(&client, &token, "tools/list", json!({})).await.status(),
            status
        );
    }
}

#[rocket::async_test]
async fn cancelled_login_preserves_state_and_requires_the_initiating_browser() {
    let client = app(config(BASE, CLIENT)).await;
    let start = authorize(&client, MCP).await;
    let state = params(&redirect(&start))["state"].clone();
    let cookie = start.cookies().iter().next().unwrap().clone().into_owned();
    let callback = format!(
        "/mcp/oauth/callback?{}",
        fields(&[("state", &state), ("error", "access_denied")])
    );
    let unbound = client.get(&callback).dispatch().await;
    assert_eq!(unbound.status(), Status::BadRequest);
    assert!(unbound.headers().get_one("Location").is_none());
    let cancelled = client.get(&callback).cookie(cookie).dispatch().await;
    let destination = redirect(&cancelled);
    assert_eq!(destination.as_str().split('?').next().unwrap(), REDIRECT);
    assert_eq!(params(&destination)["error"], "access_denied");
    assert_eq!(params(&destination)["state"], STATE);
    assert!(!params(&destination).contains_key("code"));
}

// Four loopback HTTP calls model GitHub's code exchange, identity lookup,
// refresh-token rotation and second identity lookup. No external account is used.
async fn mock_github(listener: TcpListener, upstream_challenge: String) {
    for index in 0..4 {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        let mut request_line = String::new();
        stream.read_line(&mut request_line).await.unwrap();
        let mut headers = HashMap::new();
        loop {
            let mut line = String::new();
            assert_ne!(stream.read_line(&mut line).await.unwrap(), 0);
            if line == "\r\n" {
                break;
            }
            let (name, value) = line.trim_end().split_once(':').unwrap();
            headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
        }
        let body = if index % 2 == 0 {
            assert!(request_line.starts_with("POST /token "));
            let mut bytes = vec![0; headers["content-length"].parse().unwrap()];
            stream.read_exact(&mut bytes).await.unwrap();
            let form = params(&Url::parse(&format!("{BASE}/?{}", String::from_utf8(bytes).unwrap())).unwrap());
            assert_eq!(form["client_id"], CLIENT);
            assert_eq!(form["client_secret"], SECRET);
            assert!(!form.contains_key("resource"));
            if index == 0 {
                assert_eq!(form["code"], "github-code");
                assert_eq!(form["redirect_uri"], format!("{BASE}/mcp/oauth/callback"));
                assert_ne!(form["code_verifier"], VERIFIER);
                assert_eq!(URL_SAFE_NO_PAD.encode(digest(&SHA256, form["code_verifier"].as_bytes())), upstream_challenge);
            } else {
                assert_eq!(form["grant_type"], "refresh_token");
                assert_eq!(form["refresh_token"], "github-refresh-0");
            }
            json!({"access_token": format!("github-access-{index}"), "token_type":"bearer",
                "refresh_token": format!("github-refresh-{index}"), "refresh_token_expires_in":3600})
        } else {
            assert!(request_line.starts_with("GET /user "));
            assert_eq!(headers["authorization"], format!("Bearer github-access-{}", index - 1));
            json!({"id":USER})
        }.to_string();
        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        stream
            .get_mut()
            .write_all(response.as_bytes())
            .await
            .unwrap();
    }
}

#[rocket::async_test]
async fn connector_completes_github_pkce_exchange_and_refresh_and_calls_mcp() {
    let mut cfg = config(BASE, CLIENT);
    let server = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let github = format!("http://{}", server.local_addr().unwrap());
    cfg.github_authorize = format!("{github}/authorize");
    cfg.github_token = format!("{github}/token");
    cfg.github_user = format!("{github}/user");
    let client = app(cfg).await;
    let wrong = authorize(&client, "https://other.example.test/mcp").await;
    let rejected = redirect(&wrong);
    assert_eq!(rejected.as_str().split('?').next().unwrap(), REDIRECT);
    assert_eq!(params(&rejected)["error"], "invalid_target");
    let start = authorize(&client, MCP).await;
    let destination = redirect(&start);
    assert_eq!(
        destination.as_str().split('?').next().unwrap(),
        format!("{github}/authorize")
    );
    let upstream = params(&destination);
    assert_eq!(upstream["client_id"], CLIENT);
    assert_eq!(
        upstream["redirect_uri"],
        format!("{BASE}/mcp/oauth/callback")
    );
    assert_eq!(upstream["code_challenge_method"], "S256");
    assert_eq!(upstream["scope"], "read:user offline_access");
    assert_ne!(upstream["code_challenge"], CHALLENGE);
    assert_ne!(upstream["state"], STATE);
    let cookie: Cookie<'_> = start.cookies().iter().next().unwrap().clone();
    let callback = client
        .get(format!(
            "/mcp/oauth/callback?{}",
            fields(&[("state", &upstream["state"]), ("code", "github-code")])
        ))
        .cookie(cookie.into_owned())
        .dispatch()
        .await;
    let returned = redirect(&callback);
    assert_eq!(returned.as_str().split('?').next().unwrap(), REDIRECT);
    let downstream = params(&returned);
    assert_eq!(downstream["state"], STATE);
    assert_eq!(downstream["iss"], BASE);
    let code = downstream["code"].as_str();
    assert_ne!(code, "github-code");

    let github_calls =
        rocket::tokio::spawn(mock_github(server, upstream["code_challenge"].clone()));
    let code_params = |resource, verifier| {
        [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT),
            ("code_verifier", verifier),
            ("resource", resource),
        ]
    };
    for (resource, verifier, expected_error) in [
        ("https://other.example.test/mcp", VERIFIER, "invalid_target"),
        (MCP, "invalid-verifier", "invalid_grant"),
    ] {
        let denied = token_post(&client, &code_params(resource, verifier), false).await;
        assert_eq!(denied.status(), Status::BadRequest);
        assert_eq!(
            denied.into_json::<Value>().await.unwrap()["error"],
            expected_error
        );
    }
    let granted = token_post(&client, &code_params(MCP, VERIFIER), true).await;
    assert_eq!(granted.status(), Status::Ok);
    assert_eq!(granted.headers().get_one("Cache-Control"), Some("no-store"));
    let tokens = granted.into_json::<Value>().await.unwrap();
    assert_eq!(tokens["scope"], "mcp.access offline_access");
    let access = tokens["access_token"].as_str().unwrap();
    let refresh = tokens["refresh_token"].as_str().unwrap();
    assert_ne!(access, "github-access-0");
    assert_ne!(refresh, "github-refresh-0");
    for wrong_token in [refresh, code, "github-access-0"] {
        assert_eq!(
            rpc(&client, wrong_token, "tools/list", json!({}))
                .await
                .status(),
            Status::Unauthorized
        );
    }
    let init = result(rpc(&client, access, "initialize", json!({})).await).await;
    assert_eq!(init["serverInfo"]["name"], "doc-manager");
    let list = result(rpc(&client, access, "tools/list", json!({})).await).await;
    assert!(list["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "list_documents"));
    let refresh_params = |resource| {
        [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh),
            ("resource", resource),
        ]
    };
    let wrong = token_post(
        &client,
        &refresh_params("https://other.example.test/mcp"),
        false,
    )
    .await;
    assert_eq!(wrong.status(), Status::BadRequest);
    assert_eq!(
        wrong.into_json::<Value>().await.unwrap()["error"],
        "invalid_target"
    );
    let mut narrowed = refresh_params(MCP).to_vec();
    narrowed.push(("scope", "mcp.access"));
    let renewed = token_post(&client, &narrowed, false).await;
    assert_eq!(renewed.status(), Status::Ok);
    let renewed = renewed.into_json::<Value>().await.unwrap();
    assert_eq!(renewed["scope"], "mcp.access");
    assert_ne!(renewed["refresh_token"].as_str().unwrap(), refresh);
    assert_ne!(renewed["access_token"], access);
    let docs = result(
        rpc(
            &client,
            renewed["access_token"].as_str().unwrap(),
            "tools/call",
            json!({"name":"list_documents","arguments":{}}),
        )
        .await,
    )
    .await;
    let documents: Value =
        serde_json::from_str(docs["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(documents, json!([]));
    github_calls.await.unwrap();
}
