//! Model Context Protocol (Streamable HTTP) endpoint for the Doc Manager.
//!
//! Exposes a JSON-RPC 2.0 server at `POST /mcp`, gated by an OAuth bearer
//! token issued by an upstream IdP (currently GitHub) and validated by
//! the [`oauth`](crate::oauth) module. An unauthenticated request gets
//! `401 Unauthorized` with `WWW-Authenticate: Bearer realm="MCP",
//! resource_metadata="<base>/.well-known/oauth-protected-resource"` — the
//! signal that triggers Claude.ai to run the OAuth flow it has the
//! Client ID/Secret for.
//!
//! Encryption interplay: documents stored via the React UI use client-side
//! zero-knowledge AES-GCM (content prefix `DMENC1:` or attachment named
//! `*_dmencblob`). The MCP server never holds the password, so encrypted docs
//! are listed and surfaced with `encrypted: true` but their content is never
//! returned, and writes refuse to overwrite them. By design: encrypted = "not
//! shareable with Claude", plaintext = "shareable with Claude".

use crate::oauth::{self, AuthError, OAuthConfig, OAuthState};
use crate::AzureClient;
use futures::stream::StreamExt;
use rocket::http::{Header, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::response::{self, Responder, Response};
use rocket::serde::json::{json, Json, Value};
use rocket::serde::Deserialize;
use rocket::State;
use std::env;
use uuid::Uuid;

const PROTOCOL_VERSION: &str = "2025-06-18";
const SERVER_NAME: &str = "doc-manager";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

const ENC_TEXT_PREFIX: &str = "DMENC1:";
const ENC_FILE_BLOB_NAME: &str = "dmencblob";

fn is_encrypted_text(s: &str) -> bool {
    s.starts_with(ENC_TEXT_PREFIX)
}

fn is_encrypted_file_id(file_id: &str, doc_id: &str) -> bool {
    file_id == format!("{}_{}", doc_id, ENC_FILE_BLOB_NAME)
}

fn is_main_doc_blob(name: &str) -> bool {
    !name.contains('_')
}

fn read_only() -> bool {
    matches!(
        env::var("MCP_READ_ONLY").ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

// ---------- Bearer-header guard ----------

pub struct PresentedBearer(pub Option<String>);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for PresentedBearer {
    type Error = ();

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        let token = req
            .headers()
            .get_one("Authorization")
            .and_then(|h| h.strip_prefix("Bearer "))
            .map(str::to_string);
        Outcome::Success(PresentedBearer(token))
    }
}

async fn authenticate(
    presented: &str,
    oauth_cfg: Option<&State<OAuthConfig>>,
    oauth_state: Option<&State<OAuthState>>,
) -> Result<(), McpHttp> {
    let (Some(cfg), Some(state)) = (oauth_cfg, oauth_state) else {
        // OAuth not configured: MCP endpoint is effectively disabled.
        return Err(McpHttp::Status(Status::ServiceUnavailable));
    };

    if !presented.is_empty() {
        match oauth::validate_bearer(presented, cfg.inner(), state.inner()).await {
            Ok(user) => {
                println!("[mcp] auth ok: user={}", user.login);
                return Ok(());
            }
            Err(AuthError::Forbidden(msg)) => {
                eprintln!("[mcp] auth forbidden: {msg}");
                return Err(McpHttp::Status(Status::Forbidden));
            }
            Err(AuthError::Unauthorized(msg)) => {
                eprintln!("[mcp] auth unauthorized: {msg}");
            }
            Err(AuthError::Upstream(msg)) => {
                eprintln!("[mcp] auth upstream error: {msg}");
            }
        }
    }
    Err(McpHttp::Unauthorized {
        www_authenticate: Some(oauth::www_authenticate_header(cfg.inner())),
    })
}

// ---------- JSON-RPC types ----------

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

struct McpError {
    code: i32,
    message: String,
}

impl McpError {
    fn method_not_found(m: &str) -> Self {
        Self { code: -32601, message: format!("Method not found: {m}") }
    }
    fn invalid_params<S: Into<String>>(m: S) -> Self {
        Self { code: -32602, message: m.into() }
    }
    fn internal<S: Into<String>>(m: S) -> Self {
        Self { code: -32603, message: m.into() }
    }
    fn forbidden<S: Into<String>>(m: S) -> Self {
        Self { code: -32000, message: m.into() }
    }
}

// ---------- Custom responder ----------
//
// `Body` is the JSON-RPC success body, `Accepted` is the 202 we use for
// notifications, `Unauthorized` carries an optional `WWW-Authenticate`
// header (set when OAuth is configured), and `Status` is a thin escape
// hatch for raw status codes (403, 405, 503).

pub enum McpHttp {
    Body(Value),
    Accepted,
    Unauthorized { www_authenticate: Option<String> },
    Status(Status),
}

impl<'r> Responder<'r, 'static> for McpHttp {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        match self {
            McpHttp::Body(v) => Json(v).respond_to(req),
            McpHttp::Accepted => Response::build().status(Status::Accepted).ok(),
            McpHttp::Unauthorized { www_authenticate } => {
                let mut r = Response::build();
                r.status(Status::Unauthorized);
                if let Some(value) = www_authenticate {
                    r.header(Header::new("WWW-Authenticate", value));
                }
                r.ok()
            }
            McpHttp::Status(s) => Response::build().status(s).ok(),
        }
    }
}

// ---------- Routes ----------

#[get("/mcp")]
pub async fn mcp_get(
    bearer: PresentedBearer,
    oauth_cfg: Option<&State<OAuthConfig>>,
    oauth_state: Option<&State<OAuthState>>,
) -> McpHttp {
    let token = bearer.0.as_deref().unwrap_or("");
    if let Err(resp) = authenticate(token, oauth_cfg, oauth_state).await {
        return resp;
    }
    // Server-initiated SSE streams not supported; clients should POST.
    McpHttp::Status(Status::MethodNotAllowed)
}

#[post("/mcp", data = "<req>")]
pub async fn mcp_post(
    bearer: PresentedBearer,
    oauth_cfg: Option<&State<OAuthConfig>>,
    oauth_state: Option<&State<OAuthState>>,
    req: Json<JsonRpcRequest>,
    client: &State<AzureClient>,
) -> McpHttp {
    let token = bearer.0.as_deref().unwrap_or("");
    if let Err(resp) = authenticate(token, oauth_cfg, oauth_state).await {
        return resp;
    }
    handle(req.into_inner(), client.inner()).await
}

async fn handle(req: JsonRpcRequest, client: &AzureClient) -> McpHttp {
    let id = req.id;
    let is_notification = id.is_none();
    let result = dispatch(&req.method, req.params, client).await;
    if is_notification {
        return McpHttp::Accepted;
    }
    let id = id.unwrap_or(Value::Null);
    McpHttp::Body(match result {
        Ok(v) => json!({"jsonrpc": "2.0", "id": id, "result": v}),
        Err(e) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": e.code, "message": e.message}
        }),
    })
}

async fn dispatch(
    method: &str,
    params: Option<Value>,
    client: &AzureClient,
) -> Result<Value, McpError> {
    match method {
        "initialize" => Ok(initialize_result()),
        "notifications/initialized" | "notifications/cancelled" => Ok(Value::Null),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools_list()),
        "tools/call" => tools_call(params, client).await,
        m => Err(McpError::method_not_found(m)),
    }
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION },
        "instructions": concat!(
            "Manage plaintext documents in the user's Azure Blob-backed vault. ",
            "Documents marked encrypted: true use client-side AES-GCM and cannot ",
            "be decrypted server-side; the password never leaves the user's browser. ",
            "Encrypted documents can be listed and discovered by title but not read or overwritten."
        ),
    })
}

fn tools_list() -> Value {
    let mut tools = vec![
        json!({
            "name": "list_documents",
            "description": "List every document in the vault with its id, title, encryption status, attachment status, and content size.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        }),
        json!({
            "name": "get_document",
            "description": "Fetch a single plaintext document by id. Refuses encrypted documents (their content is unreadable server-side).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string", "description": "The document id (UUID, exactly as returned by list_documents)."}
                },
                "required": ["id"],
                "additionalProperties": false
            }
        }),
        json!({
            "name": "search_documents",
            "description": "Case-insensitive substring search across titles (always) and plaintext content. Returns id, title, encrypted flag, what matched, and a short snippet.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "Search term."}
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }),
    ];
    if !read_only() {
        tools.push(json!({
            "name": "create_document",
            "description": "Create a new plaintext document. Returns the new id. Cannot create encrypted documents.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "content": {"type": "string"}
                },
                "required": ["title", "content"],
                "additionalProperties": false
            }
        }));
        tools.push(json!({
            "name": "update_document",
            "description": "Replace the content (and optionally the title) of an existing plaintext document. Refuses if the document is encrypted.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {"type": "string"},
                    "content": {"type": "string"},
                    "title": {"type": "string"}
                },
                "required": ["id", "content"],
                "additionalProperties": false
            }
        }));
    }
    json!({ "tools": tools })
}

async fn tools_call(params: Option<Value>, client: &AzureClient) -> Result<Value, McpError> {
    let params = params.ok_or_else(|| McpError::invalid_params("Missing params"))?;
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_params("Missing tool name"))?;
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = match name {
        "list_documents" => tool_list(client).await,
        "get_document" => tool_get(args, client).await,
        "search_documents" => tool_search(args, client).await,
        "create_document" if !read_only() => tool_create(args, client).await,
        "update_document" if !read_only() => tool_update(args, client).await,
        "create_document" | "update_document" => {
            Err(McpError::forbidden("Server is in MCP_READ_ONLY mode"))
        }
        other => Err(McpError::method_not_found(&format!("tool {other}"))),
    };

    Ok(match result {
        Ok(text) => json!({
            "content": [{"type": "text", "text": text}],
            "isError": false
        }),
        Err(err) => json!({
            "content": [{"type": "text", "text": format!("Error: {}", err.message)}],
            "isError": true
        }),
    })
}

// ---------- Tool implementations ----------

async fn list_blob_names(client: &AzureClient) -> Result<Vec<String>, McpError> {
    let mut stream = client.container_client.list_blobs().into_stream();
    let mut names = Vec::new();
    while let Some(page) = stream.next().await {
        let page = page.map_err(|e| McpError::internal(format!("list_blobs: {e}")))?;
        for b in page.blobs.blobs() {
            names.push(b.name.clone());
        }
    }
    Ok(names)
}

async fn fetch_title(client: &AzureClient, id: &str) -> String {
    let title_blob = format!("title_{id}");
    match client
        .container_client
        .blob_client(&title_blob)
        .get_content()
        .await
    {
        Ok(b) => String::from_utf8(b).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

async fn tool_list(client: &AzureClient) -> Result<String, McpError> {
    let names = list_blob_names(client).await?;
    let mut summaries = Vec::new();

    for name in &names {
        if !is_main_doc_blob(name) {
            continue;
        }
        let id = name.clone();
        let blob_client = client.container_client.blob_client(&id);
        let content = match blob_client.get_content().await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let content_str = String::from_utf8_lossy(&content).into_owned();
        let encrypted = is_encrypted_text(&content_str);

        let attachment = names.iter().find(|n| {
            n.starts_with(&format!("{id}_")) && !n.starts_with("title_")
        });
        let has_attachment = attachment.is_some();
        let encrypted_attachment = attachment
            .map(|f| is_encrypted_file_id(f, &id))
            .unwrap_or(false);

        let title = fetch_title(client, &id).await;

        summaries.push(json!({
            "id": id,
            "title": title,
            "encrypted": encrypted,
            "has_attachment": has_attachment,
            "encrypted_attachment": encrypted_attachment,
            "size": content.len(),
        }));
    }

    serde_json::to_string_pretty(&summaries)
        .map_err(|e| McpError::internal(format!("serialize: {e}")))
}

async fn tool_get(args: Value, client: &AzureClient) -> Result<String, McpError> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_params("Missing 'id'"))?;

    let content = client
        .container_client
        .blob_client(id)
        .get_content()
        .await
        .map_err(|e| McpError::internal(format!("Document not found: {e}")))?;
    let content_str = String::from_utf8(content)
        .map_err(|e| McpError::internal(format!("Non-UTF8 content: {e}")))?;
    let title = fetch_title(client, id).await;

    if is_encrypted_text(&content_str) {
        return Err(McpError::forbidden(format!(
            "Document {id} ('{title}') is encrypted client-side; only the user's browser holds the key."
        )));
    }

    serde_json::to_string_pretty(&json!({
        "id": id,
        "title": title,
        "content": content_str,
    }))
    .map_err(|e| McpError::internal(format!("serialize: {e}")))
}

async fn tool_search(args: Value, client: &AzureClient) -> Result<String, McpError> {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_params("Missing 'query'"))?
        .to_lowercase();
    if query.is_empty() {
        return Err(McpError::invalid_params("Empty 'query'"));
    }

    let names = list_blob_names(client).await?;
    let mut hits = Vec::new();

    for name in &names {
        if !is_main_doc_blob(name) {
            continue;
        }
        let id = name.clone();
        let title = fetch_title(client, &id).await;
        let title_match = title.to_lowercase().contains(&query);

        let content = match client.container_client.blob_client(&id).get_content().await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let content_str = String::from_utf8_lossy(&content).into_owned();
        let encrypted = is_encrypted_text(&content_str);
        let content_match = !encrypted && content_str.to_lowercase().contains(&query);

        if !title_match && !content_match {
            continue;
        }

        let snippet = if content_match {
            let lower = content_str.to_lowercase();
            let pos = lower.find(&query).unwrap_or(0);
            let mut start = pos.saturating_sub(40);
            while start > 0 && !content_str.is_char_boundary(start) {
                start -= 1;
            }
            let mut end = (pos + query.len() + 40).min(content_str.len());
            while end < content_str.len() && !content_str.is_char_boundary(end) {
                end += 1;
            }
            content_str[start..end].to_string()
        } else {
            String::new()
        };

        let matched = if title_match && content_match {
            "title+content"
        } else if title_match {
            "title"
        } else {
            "content"
        };

        hits.push(json!({
            "id": id,
            "title": title,
            "encrypted": encrypted,
            "matched": matched,
            "snippet": snippet,
        }));
    }

    serde_json::to_string_pretty(&hits)
        .map_err(|e| McpError::internal(format!("serialize: {e}")))
}

async fn tool_create(args: Value, client: &AzureClient) -> Result<String, McpError> {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_params("Missing 'title'"))?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_params("Missing 'content'"))?;

    if is_encrypted_text(content) {
        return Err(McpError::invalid_params(
            "Refusing to write a payload starting with the encrypted prefix DMENC1:",
        ));
    }

    let id = Uuid::new_v4().to_string();
    client
        .container_client
        .blob_client(&id)
        .put_block_blob(content.as_bytes().to_vec())
        .await
        .map_err(|e| McpError::internal(format!("put content: {e}")))?;

    if !title.is_empty() {
        client
            .container_client
            .blob_client(&format!("title_{id}"))
            .put_block_blob(title.as_bytes().to_vec())
            .await
            .map_err(|e| McpError::internal(format!("put title: {e}")))?;
    }

    serde_json::to_string_pretty(&json!({
        "id": id,
        "title": title,
        "content_bytes": content.len(),
        "status": "created",
    }))
    .map_err(|e| McpError::internal(format!("serialize: {e}")))
}

async fn tool_update(args: Value, client: &AzureClient) -> Result<String, McpError> {
    let id = args
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_params("Missing 'id'"))?;
    let content = args
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| McpError::invalid_params("Missing 'content'"))?;
    let new_title = args.get("title").and_then(|v| v.as_str());

    if is_encrypted_text(content) {
        return Err(McpError::invalid_params(
            "Refusing to write a payload starting with the encrypted prefix DMENC1:",
        ));
    }

    let existing = client
        .container_client
        .blob_client(id)
        .get_content()
        .await
        .map_err(|e| McpError::internal(format!("Document not found: {e}")))?;
    let existing_str = String::from_utf8_lossy(&existing).into_owned();
    if is_encrypted_text(&existing_str) {
        return Err(McpError::forbidden(format!(
            "Document {id} is encrypted client-side; refusing to overwrite from the server."
        )));
    }

    client
        .container_client
        .blob_client(id)
        .put_block_blob(content.as_bytes().to_vec())
        .await
        .map_err(|e| McpError::internal(format!("put content: {e}")))?;

    if let Some(t) = new_title {
        let title_blob = format!("title_{id}");
        if t.is_empty() {
            let _ = client.container_client.blob_client(&title_blob).delete().await;
        } else {
            client
                .container_client
                .blob_client(&title_blob)
                .put_block_blob(t.as_bytes().to_vec())
                .await
                .map_err(|e| McpError::internal(format!("put title: {e}")))?;
        }
    }

    serde_json::to_string_pretty(&json!({
        "id": id,
        "status": "updated",
        "content_bytes": content.len(),
    }))
    .map_err(|e| McpError::internal(format!("serialize: {e}")))
}
