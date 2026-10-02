//! Blob storage adapter backed by Apache `object_store`.
//!
//! Keeps the small `container_client.blob_client(name).get_content()` /
//! `put_block_blob()` / `delete()` surface the routes were written against,
//! so the document layout (`<id>`, `title_<id>`, `<id>_<filename>`) in the
//! `documents` container is unchanged.
//!
//! Blob CRUD and listing go through `object_store`. Container auto-creation
//! (which `object_store` does not offer) uses `object_store`'s own
//! `AzureAuthorizer` to sign a HEAD/PUT `?restype=container` request that is
//! sent with `reqwest`. The access key and the signed `Authorization` header
//! never appear in errors or logs, and these types deliberately do not
//! implement `Debug`.

use futures::stream::TryStreamExt;
use object_store::azure::{AzureAccessKey, AzureAuthorizer, AzureCredential, MicrosoftAzureBuilder};
use object_store::client::{HttpRequest, HttpRequestBody};
#[cfg(test)]
use object_store::memory::InMemory;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt};
use reqwest::header::{HeaderValue, CONTENT_LENGTH};
use reqwest::{Method, StatusCode, Url};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

pub(crate) type StorageResult<T> = Result<T, object_store::Error>;

/// Startup/configuration error. Messages never contain credentials.
#[derive(Debug)]
pub(crate) struct StorageInitError(String);

impl StorageInitError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl fmt::Display for StorageInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StorageInitError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContainerStatus {
    Exists,
    Created,
    /// HEAD said 404 but a concurrent creator won the race (409 ContainerAlreadyExists).
    AlreadyExists,
}

/// Preserve lazy configuration failures for credential-free MCP discovery.
/// No fallback store can accept data when Azure is unavailable.
#[derive(Clone)]
enum StorageBackend {
    Ready(Arc<dyn ObjectStore>),
    Unavailable(String),
}

impl StorageBackend {
    fn as_store(&self) -> StorageResult<&dyn ObjectStore> {
        match self {
            Self::Ready(store) => Ok(store.as_ref()),
            Self::Unavailable(message) => Err(object_store::Error::Generic {
                store: "Azure",
                source: Box::new(StorageInitError::new(message.clone())),
            }),
        }
    }
}

pub(crate) struct ContainerClient {
    store: StorageBackend,
    bootstrap: Option<AzureContainerBootstrap>,
}

impl ContainerClient {
    /// Azure Blob container using SharedKey (account + Base64 access key).
    /// Performs no network I/O; call [`Self::ensure_container`] at startup.
    pub(crate) fn azure(
        account: &str,
        access_key: &str,
        container: &str,
    ) -> Result<Self, StorageInitError> {
        let endpoint = Url::parse(&format!("https://{account}.blob.core.windows.net"))
            .map_err(|_| StorageInitError::new("AZURE_STORAGE_ACCOUNT is not a valid account name"))?;
        let bootstrap = AzureContainerBootstrap::new(account, access_key, container, endpoint)?;
        let store = MicrosoftAzureBuilder::new()
            .with_account(account)
            .with_access_key(access_key)
            .with_container_name(container)
            .build()
            .map_err(|e| StorageInitError::new(format!("Azure storage configuration: {e}")))?;
        Ok(Self {
            store: StorageBackend::Ready(Arc::new(store)),
            bootstrap: Some(bootstrap),
        })
    }

    /// Keep configured Azure storage, deferring configuration errors until an
    /// operation is attempted, as the legacy Azure client did during discovery.
    pub(crate) fn azure_for_introspection(account: &str, access_key: &str, container: &str) -> Self {
        Self::azure(account, access_key, container).unwrap_or_else(|error| Self {
            store: StorageBackend::Unavailable(error.to_string()),
            bootstrap: None,
        })
    }

    /// Process-local storage is only used by unit tests.
    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            store: StorageBackend::Ready(Arc::new(InMemory::new())),
            bootstrap: None,
        }
    }

    /// Create the container if it does not exist yet (no-op for in-memory).
    pub(crate) async fn ensure_container(&self) -> Result<ContainerStatus, StorageInitError> {
        if let StorageBackend::Unavailable(message) = &self.store {
            return Err(StorageInitError::new(message.clone()));
        }
        match &self.bootstrap {
            Some(bootstrap) => bootstrap.ensure().await,
            None => Ok(ContainerStatus::Exists),
        }
    }

    pub(crate) fn blob_client(&self, name: impl AsRef<str>) -> BlobClient {
        BlobClient {
            store: self.store.clone(),
            name: name.as_ref().to_string(),
        }
    }

    /// Every blob name in the container (all pages), in lexical order.
    pub(crate) async fn list_blob_names(&self) -> StorageResult<Vec<String>> {
        let mut names: Vec<String> = self
            .store
            .as_store()?
            .list(None)
            .map_ok(|meta| meta.location.to_string())
            .try_collect()
            .await?;
        names.sort();
        Ok(names)
    }
}

pub(crate) struct BlobClient {
    store: StorageBackend,
    name: String,
}

impl BlobClient {
    // `Path::parse` keeps the stored name verbatim; `Path::from` would
    // percent-encode characters such as `%` a second time.
    fn path(&self) -> StorageResult<Path> {
        Ok(Path::parse(&self.name)?)
    }

    pub(crate) async fn get_content(&self) -> StorageResult<Vec<u8>> {
        let path = self.path()?;
        let bytes = self.store.as_store()?.get(&path).await?.bytes().await?;
        Ok(bytes.to_vec())
    }

    /// Single-shot upload that overwrites any existing blob.
    pub(crate) async fn put_block_blob(&self, content: Vec<u8>) -> StorageResult<()> {
        let path = self.path()?;
        self.store.as_store()?.put(&path, content.into()).await?;
        Ok(())
    }

    pub(crate) async fn delete(&self) -> StorageResult<()> {
        let path = self.path()?;
        self.store.as_store()?.delete(&path).await
    }
}

struct AzureContainerBootstrap {
    account: String,
    credential: AzureCredential,
    container_url: Url,
}

impl AzureContainerBootstrap {
    fn new(
        account: &str,
        access_key: &str,
        container: &str,
        endpoint: Url,
    ) -> Result<Self, StorageInitError> {
        // Discard the decode error: it can echo a character of the key.
        let key = AzureAccessKey::try_new(access_key)
            .map_err(|_| StorageInitError::new("AZURE_STORAGE_ACCESS_KEY is not valid Base64"))?;
        let mut container_url = endpoint;
        container_url
            .path_segments_mut()
            .map_err(|_| StorageInitError::new("invalid Azure blob endpoint"))?
            .pop_if_empty()
            .push(container);
        container_url.set_query(Some("restype=container"));
        Ok(Self {
            account: account.to_string(),
            credential: AzureCredential::AccessKey(key),
            container_url,
        })
    }

    async fn ensure(&self) -> Result<ContainerStatus, StorageInitError> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| StorageInitError::new(format!("HTTP client: {e}")))?;

        let head = self.send(&http, Method::HEAD).await?;
        match head.status() {
            StatusCode::OK => return Ok(ContainerStatus::Exists),
            StatusCode::NOT_FOUND => {}
            status => {
                return Err(StorageInitError::new(format!(
                    "container check failed: HTTP {status}{}",
                    describe_code(error_code(head).await)
                )))
            }
        }

        let put = self.send(&http, Method::PUT).await?;
        match put.status() {
            StatusCode::CREATED => Ok(ContainerStatus::Created),
            StatusCode::CONFLICT => match error_code(put).await {
                Some(code) if code == "ContainerAlreadyExists" => Ok(ContainerStatus::AlreadyExists),
                code => Err(StorageInitError::new(format!(
                    "container create failed: HTTP {}{}",
                    StatusCode::CONFLICT,
                    describe_code(code)
                ))),
            },
            status => Err(StorageInitError::new(format!(
                "container create failed: HTTP {status}{}",
                describe_code(error_code(put).await)
            ))),
        }
    }

    fn signed_request(&self, method: Method) -> Result<HttpRequest, StorageInitError> {
        let is_put = method == Method::PUT;
        let mut request = http::Request::builder()
            .method(method)
            .uri(self.container_url.as_str())
            .body(HttpRequestBody::empty())
            .map_err(|_| StorageInitError::new("invalid container request"))?;
        if is_put {
            // Create Container has no body but Azure requires Content-Length.
            request
                .headers_mut()
                .insert(CONTENT_LENGTH, HeaderValue::from_static("0"));
        }
        AzureAuthorizer::new(&self.credential, &self.account)
            .try_authorize(&mut request)
            .map_err(|_| StorageInitError::new("failed to sign container request"))?;
        if let Some(value) = request.headers_mut().get_mut(reqwest::header::AUTHORIZATION) {
            value.set_sensitive(true);
        }
        Ok(request)
    }

    async fn send(
        &self,
        http: &reqwest::Client,
        method: Method,
    ) -> Result<reqwest::Response, StorageInitError> {
        let (parts, _empty_body) = self.signed_request(method)?.into_parts();
        // Send exactly the method, URI and headers that were signed.
        let url = Url::parse(&parts.uri.to_string())
            .map_err(|_| StorageInitError::new("invalid container request URI"))?;
        let method = parts.method.clone();
        http.request(parts.method, url)
            .headers(parts.headers)
            .send()
            .await
            .map_err(|e| {
                // reqwest errors carry the URL but never request headers.
                StorageInitError::new(format!("container {method} request failed: {e}"))
            })
    }
}

/// Azure error code from `x-ms-error-code`, falling back to the XML body `<Code>`.
async fn error_code(response: reqwest::Response) -> Option<String> {
    if let Some(code) = response
        .headers()
        .get("x-ms-error-code")
        .and_then(|v| v.to_str().ok())
    {
        return Some(code.to_string());
    }
    let body = response.text().await.ok()?;
    let start = body.find("<Code>")? + "<Code>".len();
    let len = body[start..].find("</Code>")?;
    Some(body[start..start + len].to_string())
}

fn describe_code(code: Option<String>) -> String {
    code.map(|c| format!(" ({c})")).unwrap_or_default()
}

#[cfg(test)]
#[path = "storage_azurite_tests.rs"]
mod azure_integration_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use rocket::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use rocket::tokio::net::TcpListener;
    use rocket::tokio::task::JoinHandle;

    fn test_key() -> String {
        base64::engine::general_purpose::STANDARD.encode(b"unit-test-only-not-a-real-key")
    }

    #[rocket::async_test]
    async fn write_read_overwrite_delete() {
        let client = ContainerClient::in_memory();
        let blob = client.blob_client("doc1");

        blob.put_block_blob(b"first".to_vec()).await.unwrap();
        assert_eq!(blob.get_content().await.unwrap(), b"first");

        blob.put_block_blob(b"second".to_vec()).await.unwrap();
        assert_eq!(blob.get_content().await.unwrap(), b"second");

        blob.delete().await.unwrap();
        assert!(blob.get_content().await.is_err());
        assert!(client.list_blob_names().await.unwrap().is_empty());
    }

    #[rocket::async_test]
    async fn missing_blob_is_not_found() {
        let client = ContainerClient::in_memory();
        let err = client.blob_client("nope").get_content().await.unwrap_err();
        assert!(matches!(err, object_store::Error::NotFound { .. }), "{err}");
    }

    #[rocket::async_test]
    async fn names_with_percent_unicode_and_spaces_round_trip() {
        let client = ContainerClient::in_memory();
        let names = ["id_100%25 done.md", "title_caffè ☕", "id_my file (1).txt"];
        for (i, name) in names.iter().enumerate() {
            client
                .blob_client(name)
                .put_block_blob(vec![i as u8; 3])
                .await
                .unwrap();
        }
        for (i, name) in names.iter().enumerate() {
            assert_eq!(client.blob_client(name).get_content().await.unwrap(), vec![i as u8; 3]);
        }
        let mut expected: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(client.list_blob_names().await.unwrap(), expected);
    }

    #[rocket::async_test]
    async fn list_is_lexical_and_complete() {
        let client = ContainerClient::in_memory();
        let inserted = ["title_b", "b", "a_file.txt", "title_a", "a", "B"];
        for name in inserted {
            client.blob_client(name).put_block_blob(Vec::new()).await.unwrap();
        }
        assert_eq!(
            client.list_blob_names().await.unwrap(),
            vec!["B", "a", "a_file.txt", "b", "title_a", "title_b"]
        );
    }

    #[rocket::async_test]
    async fn offline_construction_needs_no_credentials_or_network() {
        let client = ContainerClient::in_memory();
        assert_eq!(client.ensure_container().await.unwrap(), ContainerStatus::Exists);
        assert!(client.list_blob_names().await.unwrap().is_empty());

        // Azure construction is offline too; only ensure_container() hits the network.
        assert!(ContainerClient::azure("devstoreaccount1", &test_key(), "documents").is_ok());
    }

    #[test]
    fn invalid_access_key_is_rejected_without_echoing_it() {
        let Err(err) = ContainerClient::azure("acct", "placeholderkey", "documents") else {
            panic!("non-Base64 key must be rejected");
        };
        assert!(!err.to_string().contains("placeholderkey"));
    }

    #[test]
    fn introspection_preserves_configured_azure_storage() {
        let client = ContainerClient::azure_for_introspection(
            "testaccount", &test_key(), "documents",
        );
        assert!(matches!(client.store, StorageBackend::Ready(_)));
        let bootstrap = client.bootstrap.as_ref().expect("Azure bootstrap must be retained");
        assert_eq!(bootstrap.account, "testaccount");
        assert_eq!(bootstrap.container_url.as_str(),
            "https://testaccount.blob.core.windows.net/documents?restype=container");
    }

    #[rocket::async_test]
    async fn introspection_defers_placeholder_errors_without_persisting_data() {
        let client = ContainerClient::azure_for_introspection(
            "devstoreaccount1", "placeholderkey", "documents",
        );
        let blob = client.blob_client("probe");
        for result in [
            blob.put_block_blob(b"must not be saved".to_vec()).await,
            blob.get_content().await.map(|_| ()),
            blob.delete().await,
            client.list_blob_names().await.map(|_| ()),
        ] {
            let message = result.expect_err("unconfigured Azure must not accept operations").to_string();
            assert!(message.contains("AZURE_STORAGE_ACCESS_KEY"));
            assert!(!message.contains("placeholderkey"));
        }
    }

    /// Serves one canned response per connection on localhost and returns the raw request heads.
    async fn mock_azure(responses: Vec<String>) -> (Url, JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = rocket::tokio::spawn(async move {
            let mut seen = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut head = Vec::new();
                let mut chunk = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    head.extend_from_slice(&chunk[..n]);
                }
                seen.push(String::from_utf8_lossy(&head).to_lowercase());
                socket.write_all(response.as_bytes()).await.unwrap();
                let _ = socket.shutdown().await;
            }
            seen
        });
        (Url::parse(&format!("http://{addr}")).unwrap(), handle)
    }

    fn bootstrap_for(endpoint: Url) -> AzureContainerBootstrap {
        AzureContainerBootstrap::new("testacct", &test_key(), "documents", endpoint).unwrap()
    }

    const OK_200: &str = "HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    const NOT_FOUND_404: &str = "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nx-ms-error-code: ContainerNotFound\r\nconnection: close\r\n\r\n";
    const CREATED_201: &str = "HTTP/1.1 201 Created\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";

    #[rocket::async_test]
    async fn bootstrap_existing_container() {
        let (endpoint, server) = mock_azure(vec![OK_200.into()]).await;
        let status = bootstrap_for(endpoint).ensure().await.unwrap();
        assert_eq!(status, ContainerStatus::Exists);

        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 1);
        assert!(seen[0].starts_with("head /documents?restype=container http/1.1\r\n"), "{}", seen[0]);
        assert!(seen[0].contains("\r\nauthorization: sharedkey testacct:"));
        assert!(seen[0].contains("\r\nx-ms-version: "));
        assert!(seen[0].contains("\r\ndate: "));
    }

    #[rocket::async_test]
    async fn bootstrap_creates_missing_container() {
        let (endpoint, server) = mock_azure(vec![NOT_FOUND_404.into(), CREATED_201.into()]).await;
        let status = bootstrap_for(endpoint).ensure().await.unwrap();
        assert_eq!(status, ContainerStatus::Created);

        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].starts_with("put /documents?restype=container http/1.1\r\n"), "{}", seen[1]);
        assert!(seen[1].contains("\r\ncontent-length: 0\r\n"));
        assert!(seen[1].contains("\r\nauthorization: sharedkey testacct:"));
    }

    #[rocket::async_test]
    async fn bootstrap_tolerates_create_race() {
        let conflict = "HTTP/1.1 409 Conflict\r\ncontent-length: 0\r\nx-ms-error-code: ContainerAlreadyExists\r\nconnection: close\r\n\r\n";
        let (endpoint, _server) = mock_azure(vec![NOT_FOUND_404.into(), conflict.into()]).await;
        let status = bootstrap_for(endpoint).ensure().await.unwrap();
        assert_eq!(status, ContainerStatus::AlreadyExists);
    }

    #[rocket::async_test]
    async fn bootstrap_rejects_other_conflicts() {
        let body = "<?xml version=\"1.0\" encoding=\"utf-8\"?><Error><Code>ContainerBeingDeleted</Code></Error>";
        let conflict = format!(
            "HTTP/1.1 409 Conflict\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let (endpoint, _server) = mock_azure(vec![NOT_FOUND_404.into(), conflict]).await;
        let err = bootstrap_for(endpoint).ensure().await.unwrap_err();
        assert!(err.to_string().contains("ContainerBeingDeleted"), "{err}");
    }

    #[rocket::async_test]
    async fn bootstrap_propagates_auth_failure_without_secrets() {
        let forbidden = "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\nx-ms-error-code: AuthenticationFailed\r\nconnection: close\r\n\r\n";
        let (endpoint, server) = mock_azure(vec![forbidden.into()]).await;
        let err = bootstrap_for(endpoint).ensure().await.unwrap_err().to_string();
        assert!(err.contains("403") && err.contains("AuthenticationFailed"), "{err}");
        assert!(!err.contains(&test_key()));
        assert!(!err.to_lowercase().contains("sharedkey"));
        assert_eq!(server.await.unwrap().len(), 1, "must not attempt PUT after HEAD error");
    }

    #[rocket::async_test]
    async fn bootstrap_does_not_follow_redirects() {
        let redirect = "HTTP/1.1 307 Temporary Redirect\r\nlocation: http://127.0.0.1:9/elsewhere\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let (endpoint, _server) = mock_azure(vec![redirect.into()]).await;
        let err = bootstrap_for(endpoint).ensure().await.unwrap_err();
        assert!(err.to_string().contains("307"), "{err}");
    }
}
