//! Opt-in integration tests against a disposable, local Azurite instance.
//!
//! These tests use only Azurite's public development account, never application
//! environment variables or real Azure credentials. Run with Azurite listening
//! on 127.0.0.1:10039 and `cargo test azure_integration_tests -- --ignored`.

use super::*;
use base64::Engine as _;

const ACCOUNT: &str = "devstoreaccount1";
// Public emulator credential documented by the Azurite project.
const KEY: &str =
    "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";
const ENDPOINT: &str = "http://127.0.0.1:10039";

fn unique_container() -> String {
    format!("adapter-test-{}", uuid::Uuid::new_v4().simple())
}

fn bootstrap(container: &str, key: &str) -> AzureContainerBootstrap {
    AzureContainerBootstrap::new(
        ACCOUNT,
        key,
        container,
        Url::parse(&format!("{ENDPOINT}/{ACCOUNT}")).unwrap(),
    )
    .unwrap()
}

fn emulator_client(container: &str) -> ContainerClient {
    // In object_store 0.14.2, use_emulator(true) ignores with_endpoint and
    // reads the process-global AZURITE_BLOB_STORAGE_URL instead. Use an
    // explicit account-scoped endpoint so parallel tests need no env mutation.
    let store = MicrosoftAzureBuilder::new()
        .with_allow_http(true)
        .with_endpoint(format!("{ENDPOINT}/{ACCOUNT}"))
        .with_account(ACCOUNT)
        .with_access_key(KEY)
        .with_container_name(container)
        .build()
        .unwrap();
    ContainerClient {
        store: StorageBackend::Ready(Arc::new(store)),
        bootstrap: Some(bootstrap(container, KEY)),
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

async fn remove_container(container: &str) {
    let response = bootstrap(container, KEY)
        .send(&http_client(), Method::DELETE)
        .await
        .expect("local emulator container cleanup request");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[rocket::async_test]
#[ignore = "requires disposable local Azurite at 127.0.0.1:10039"]
async fn bootstrap_signs_real_requests_and_creates_a_private_container_once() {
    let container = unique_container();
    let client = emulator_client(&container);
    let control = bootstrap(&container, KEY);
    let http = http_client();

    for method in [Method::HEAD, Method::PUT] {
        let signed = control.signed_request(method).unwrap();
        assert!(
            signed
                .headers()
                .get(reqwest::header::AUTHORIZATION)
                .unwrap()
                .is_sensitive(),
            "signed Authorization must be redacted by header diagnostics"
        );
    }

    let before = control.send(&http, Method::HEAD).await.unwrap();
    assert_eq!(before.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        client.ensure_container().await.unwrap(),
        ContainerStatus::Created
    );
    assert_eq!(
        client.ensure_container().await.unwrap(),
        ContainerStatus::Exists
    );
    assert!(client.list_blob_names().await.unwrap().is_empty());

    let after = control.send(&http, Method::HEAD).await.unwrap();
    assert_eq!(after.status(), StatusCode::OK);
    assert!(
        after.headers().get("x-ms-blob-public-access").is_none(),
        "new containers must not grant anonymous access"
    );

    client
        .blob_client("private.txt")
        .put_block_blob(b"private".to_vec())
        .await
        .unwrap();
    let anonymous = http
        .get(format!("{ENDPOINT}/{ACCOUNT}/{container}/private.txt"))
        .send()
        .await
        .unwrap();
    assert!(
        matches!(
            anonymous.status(),
            StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
        ),
        "anonymous reads must fail, got {}",
        anonymous.status()
    );
    assert_eq!(
        client
            .blob_client("private.txt")
            .get_content()
            .await
            .unwrap(),
        b"private"
    );
    remove_container(&container).await;
}

#[rocket::async_test]
#[ignore = "requires disposable local Azurite at 127.0.0.1:10039"]
async fn azure_crud_preserves_documents_titles_attachments_and_exact_names() {
    let container = unique_container();
    let client = emulator_client(&container);
    assert_eq!(
        client.ensure_container().await.unwrap(),
        ContainerStatus::Created
    );

    let id = uuid::Uuid::new_v4().to_string();
    let title = format!("title_{id}");
    let attachment = format!("{id}_caffè 100%25 #1.bin");
    let percent = format!("{id}_literal%2Fname.txt");
    let binary = vec![0, 255, 128, 13, 10, 0, 1, 254, 42];
    let encrypted = b"{\"encrypted\":true,\"ciphertext\":\"AAECA/8=\"}".to_vec();
    let entries = vec![
        (
            id.clone(),
            "Testo del documento: caffè ☕".as_bytes().to_vec(),
        ),
        (title.clone(), "Titolo con accenti è".as_bytes().to_vec()),
        (attachment.clone(), binary.clone()),
        (percent.clone(), encrypted.clone()),
    ];

    for (name, content) in &entries {
        client
            .blob_client(name)
            .put_block_blob(content.clone())
            .await
            .unwrap();
    }
    let mut expected: Vec<String> = entries.iter().map(|(name, _)| name.clone()).collect();
    expected.sort();
    assert_eq!(client.list_blob_names().await.unwrap(), expected);
    for (name, content) in &entries {
        assert_eq!(
            client.blob_client(name).get_content().await.unwrap(),
            *content
        );
    }

    // Both zero-byte and overwrite operations must retain the same blob name.
    client
        .blob_client(&id)
        .put_block_blob(Vec::new())
        .await
        .unwrap();
    assert!(client
        .blob_client(&id)
        .get_content()
        .await
        .unwrap()
        .is_empty());
    client
        .blob_client(&id)
        .put_block_blob(encrypted.clone())
        .await
        .unwrap();
    assert_eq!(
        client.blob_client(&id).get_content().await.unwrap(),
        encrypted
    );
    assert_eq!(
        client.blob_client(&attachment).get_content().await.unwrap(),
        binary
    );
    assert_eq!(client.list_blob_names().await.unwrap(), expected);

    let absent = client.blob_client(format!("{id}_absent"));
    assert!(matches!(
        absent.get_content().await.unwrap_err(),
        object_store::Error::NotFound { .. }
    ));

    for (name, _) in &entries {
        let blob = client.blob_client(name);
        blob.delete().await.unwrap();
        assert!(matches!(
            blob.get_content().await.unwrap_err(),
            object_store::Error::NotFound { .. }
        ));
    }
    assert!(client.list_blob_names().await.unwrap().is_empty());
    remove_container(&container).await;
}

#[rocket::async_test]
#[ignore = "requires disposable local Azurite at 127.0.0.1:10039"]
async fn bootstrap_rejects_a_wrong_shared_key_without_exposing_credentials() {
    let container = unique_container();
    let wrong_key = base64::engine::general_purpose::STANDARD.encode([0x42; 64]);
    let error = bootstrap(&container, &wrong_key)
        .ensure()
        .await
        .unwrap_err();
    let message = error.to_string();
    assert!(
        message.contains("403"),
        "expected authorization failure, got {message}"
    );
    assert!(!message.contains(KEY));
    assert!(!message.contains(&wrong_key));
    assert!(!message.to_ascii_lowercase().contains("authorization:"));
    assert!(!message.to_ascii_lowercase().contains("sharedkey "));

    // The authorization failure must not create a container.
    let response = bootstrap(&container, KEY)
        .send(&http_client(), Method::HEAD)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
