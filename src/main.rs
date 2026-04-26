#[macro_use]
extern crate rocket;

use azure_storage::StorageCredentials;
use azure_storage_blobs::prelude::*;
use futures::stream::StreamExt;
use rocket::form::Form;
use rocket::fs::FileServer;
use rocket::fs::TempFile;
use rocket::http::{ContentType, Header, Method, Status};
use rocket::response::Responder;
use rocket::serde::{json::Json, Deserialize, Serialize};
use rocket::tokio::io::AsyncReadExt;
use rocket::Request;
use rocket::Response;
use rocket::State;
use rocket_cors::{AllowedOrigins, CorsOptions};
use std::env;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
struct Document {
    id: String,
    title: String,
    content: String,
    file_id: Option<String>
}

#[derive(FromForm)]
struct DocumentForm<'r> {
    title: Option<&'r str>,
    content: &'r str,
    file: Option<TempFile<'r>>,
}

struct AzureClient {
    container_client: ContainerClient,
}

#[get("/")]
fn index() -> &'static str {
    "Welcome to the Document Manager! Use /documents endpoints to manage documents."
}

#[derive(Serialize)]
struct ErrorResponse {
    message: String,
}

#[catch(413)]
fn payload_too_large(req: &Request<'_>) -> Json<ErrorResponse> {
    let limit = req
        .rocket()
        .config()
        .limits
        .get("data-form")
        .or_else(|| req.rocket().config().limits.get("bytes"))
        .map(|b| format!("{}", b))
        .unwrap_or_else(|| "the configured limit".to_string());
    Json(ErrorResponse {
        message: format!("Upload too large. Maximum allowed size is {}.", limit),
    })
}

#[catch(422)]
fn unprocessable_entity() -> Json<ErrorResponse> {
    Json(ErrorResponse {
        message: "Request could not be processed. The upload may be too large or malformed."
            .to_string(),
    })
}

#[catch(404)]
fn not_found() -> Json<ErrorResponse> {
    Json(ErrorResponse {
        message: "Resource not found.".to_string(),
    })
}

#[catch(500)]
fn internal_error() -> Json<ErrorResponse> {
    Json(ErrorResponse {
        message: "Internal server error.".to_string(),
    })
}

#[get("/documents")]
async fn list_documents(
    client: &State<AzureClient>,
) -> Result<Json<Vec<Document>>, (Status, Json<ErrorResponse>)> {
    let mut documents = Vec::new();

    let blob_list = match client
        .container_client
        .list_blobs()
        .into_stream()
        .next()
        .await
    {
        Some(Ok(blob_list)) => blob_list,
        Some(Err(e)) => {
            eprintln!("Failed to list blobs: {}", e);
            return Err((
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to list blobs: {}", e),
                }),
            ));
        }
        None => {
            return Err((
                Status::NotFound,
                Json(ErrorResponse {
                    message: "No blobs found".to_string(),
                }),
            ))
        }
    };

    for blob in blob_list
        .blobs
        .blobs()
        .filter(|blob| !blob.name.contains('_'))
    {
        let blob_client = client.container_client.blob_client(blob.name.clone());
        let content = match blob_client.get_content().await {
            Ok(content) => content,
            Err(e) => {
                eprintln!("Failed to get blob content: {}", e);
                return Err((
                    Status::InternalServerError,
                    Json(ErrorResponse {
                        message: format!("Failed to get blob content: {}", e),
                    }),
                ));
            }
        };

        let title_blob_name = format!("title_{}", blob.name);
        let title = match client.container_client.blob_client(&title_blob_name).get_content().await {
            Ok(t) => String::from_utf8(t).unwrap_or_default(),
            Err(_) => String::new(),
        };

        let mut file_id = None;
        for blobfile in blob_list.blobs.blobs() {
            if blobfile.name.starts_with(&format!("{}_{}", blob.name, "")) {
            file_id = Some(blobfile.name.clone());
            break;
            }
        }
        let content_str =
            String::from_utf8(content).map_err(|e| {
                eprintln!("Failed to parse content: {}", e);
                (
                    Status::InternalServerError,
                    Json(ErrorResponse {
                        message: format!("Failed to parse content: {}", e),
                    }),
                )
            })?;
        documents.push(Document {
            id: blob.name.clone(),
            title,
            content: content_str,
            file_id: file_id,
        });
    }

    Ok(Json(documents))
}

#[get("/documents/<id>")]
async fn get_document(
    id: &str,
    client: &State<AzureClient>,
) -> Result<Json<Document>, (Status, Json<ErrorResponse>)> {
    let blob_client = client.container_client.blob_client(id);
    match blob_client.get_content().await {
        Ok(content) => {
            let content_str =
                String::from_utf8(content).map_err(|e| {
                    eprintln!("Failed to parse content: {}", e);
                    (
                        Status::InternalServerError,
                        Json(ErrorResponse {
                            message: format!("Failed to parse content: {}", e),
                        }),
                    )
                })?;

            let title_blob_name = format!("title_{}", id);
            let title = match client.container_client.blob_client(&title_blob_name).get_content().await {
                Ok(t) => String::from_utf8(t).unwrap_or_default(),
                Err(_) => String::new(),
            };

            let blob_list = match client
                .container_client
                .list_blobs()
                .into_stream()
                .next()
                .await
            {
                Some(Ok(blob_list)) => blob_list,
                Some(Err(e)) => {
                    eprintln!("Failed to list blobs: {}", e);
                    return Err((
                        Status::InternalServerError,
                        Json(ErrorResponse {
                            message: format!("Failed to list blobs: {}", e),
                        }),
                    ));
                }
                None => {
                    return Err((
                        Status::NotFound,
                        Json(ErrorResponse {
                            message: "No blobs found".to_string(),
                        }),
                    ))
                }
            };
            // Search for the first blob with the same id part and a filename
            let mut file_id = None;
            for blobfile in blob_list.blobs.blobs() {
                if blobfile.name.starts_with(&format!("{}_{}", id, "")) {
                file_id = Some(blobfile.name.clone());
                break;
                }
            }
            Ok(Json(Document {
                id: id.to_string(),
                title,
                content: content_str,
                file_id: file_id, 
            }))
        }
        Err(e) => {
            eprintln!("Failed to get document: {}", e);
            Err((
                Status::NotFound,
                Json(ErrorResponse {
                    message: format!("Failed to get document: {}", e),
                }),
            ))
        }
    }
}

#[post("/documents", data = "<form>")]
async fn create_document(
    mut form: Form<DocumentForm<'_>>,
    client: &State<AzureClient>,
) -> Result<Json<Document>, (Status, Json<ErrorResponse>)> {
    let id = Uuid::new_v4().to_string();
    let content = form.content.as_bytes().to_vec();
    let title = form.title.unwrap_or("");

    let blob_client = client.container_client.blob_client(&id);
    blob_client.put_block_blob(content).await.map_err(|e| {
        eprintln!("Failed to create document: {}", e);
        (
            Status::InternalServerError,
            Json(ErrorResponse {
                message: format!("Failed to create document: {}", e),
            }),
        )
    })?;

    if !title.is_empty() {
        let title_blob_client = client.container_client.blob_client(&format!("title_{}", id));
        title_blob_client.put_block_blob(title.as_bytes().to_vec()).await.map_err(|e| {
            eprintln!("Failed to create title blob: {}", e);
            (
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to create title blob: {}", e),
                }),
            )
        })?;
    }

    let mut file_name_maybe = None;
    if let Some(file) = form.file.take() {
        let file_name = format!("{}_{}", id, file.name().unwrap_or("file"));
        file_name_maybe = Some(file_name.clone());
        let file_blob_client = client.container_client.blob_client(&file_name);
        let mut file_content = Vec::new();
        let mut file_handle = file.open().await.map_err(|e| {
            eprintln!("Failed to open file: {}", e);
            (
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to open file: {}", e),
                }),
            )
        })?;
        file_handle.read_to_end(&mut file_content).await.map_err(|e| {
            eprintln!("Failed to read file: {}", e);
            (
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to read file: {}", e),
                }),
            )
        })?;
        file_blob_client.put_block_blob(file_content).await.map_err(|e| {
            eprintln!("Failed to upload file: {}", e);
            (
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to upload file: {}", e),
                }),
            )
        })?;
    }

    Ok(Json(Document {
        id,
        title: title.to_string(),
        content:form.content.to_string(),
        file_id: file_name_maybe
    }))
}

#[put("/documents/<id>", data = "<form>")]
async fn update_document(
    id: &str,
    mut form: Form<DocumentForm<'_>>,
    client: &State<AzureClient>,
) -> Result<Json<Document>, (Status, Json<ErrorResponse>)> {
    let blob_client = client.container_client.blob_client(id);
    let content = form.content.as_bytes().to_vec();
    let title = form.title.unwrap_or("");

    blob_client.put_block_blob(content).await.map_err(|e| {
        eprintln!("Failed to update document: {}", e);
        (
            Status::InternalServerError,
            Json(ErrorResponse {
                message: format!("Failed to update document: {}", e),
            }),
        )
    })?;

    let title_blob_name = format!("title_{}", id);
    if !title.is_empty() {
        let title_blob_client = client.container_client.blob_client(&title_blob_name);
        title_blob_client.put_block_blob(title.as_bytes().to_vec()).await.map_err(|e| {
            eprintln!("Failed to update title blob: {}", e);
            (
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to update title blob: {}", e),
                }),
            )
        })?;
    } else {
        // Delete title blob if title is empty
        let title_blob_client = client.container_client.blob_client(&title_blob_name);
        let _ = title_blob_client.delete().await;
    }

    let mut file_name_maybe = None;
    if let Some(mut file) = form.file.take() {
        let file_name = format!("{}_{}", id, file.name().unwrap_or("file"));
        file_name_maybe = Some(file_name.clone());
        let file_blob_client = client.container_client.blob_client(&file_name);
        file.persist_to(&file_name).await.map_err(|e| {
            eprintln!("Failed to persist file: {}", e);
            (
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to persist file: {}", e),
                }),
            )
        })?;
        let file_content = std::fs::read(&file_name).map_err(|e| {
            eprintln!("Failed to read file: {}", e);
            (
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to read file: {}", e),
                }),
            )
        })?;
        file_blob_client
            .put_block_blob(file_content)
            .await
            .map_err(|e| {
                eprintln!("Failed to upload file: {}", e);
                (
                    Status::InternalServerError,
                    Json(ErrorResponse {
                        message: format!("Failed to upload file: {}", e),
                    }),
                )
            })?;
        std::fs::remove_file(&file_name).map_err(|e| {
            eprintln!("Failed to remove file: {}", e);
            (
                Status::InternalServerError,
                Json(ErrorResponse {
                    message: format!("Failed to remove file: {}", e),
                }),
            )
        })?;
    }

    Ok(Json(Document {
        id: id.to_string(),
        title: title.to_string(),
        content: form.content.to_string(),
        file_id: file_name_maybe
    }))
}

#[delete("/documents/<id>")]
async fn delete_document(
    id: &str,
    client: &State<AzureClient>,
) -> Result<Json<&'static str>, (Status, Json<ErrorResponse>)> {
    let blob_client = client.container_client.blob_client(id);
    blob_client.delete().await.map_err(|e| {
        eprintln!("Failed to delete document: {}", e);
        (
            Status::InternalServerError,
            Json(ErrorResponse {
                message: format!("Failed to delete document: {}", e),
            }),
        )
    })?;
    // Also delete the title blob, ignore errors if it doesn't exist
    let title_blob_client = client.container_client.blob_client(&format!("title_{}", id));
    let _ = title_blob_client.delete().await;
    Ok(Json("Document deleted successfully"))
}

struct DownloadResponse {
    content: Vec<u8>,
    filename: String,
    is_binary: bool,
}

impl<'r> Responder<'r, 'static> for DownloadResponse {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut response = Response::build();
        response.header(Header::new(
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", self.filename),
        ));
        if self.is_binary {
            response.header(ContentType::Binary);
        } else {
            response.header(ContentType::Plain);
        }
        response.sized_body(self.content.len(), std::io::Cursor::new(self.content));
        response.ok()
    }
}

#[get("/documents/download/<id>")]
async fn download_document(
    id: &str,
    client: &State<AzureClient>,
) -> Result<DownloadResponse, (Status, Json<ErrorResponse>)> {
    let blob_client = client.container_client.blob_client(id);
    let content = blob_client.get_content().await.map_err(|e| {
        eprintln!("Failed to download document: {}", e);
        (
            Status::NotFound,
            Json(ErrorResponse {
                message: format!("Failed to download document: {}", e),
            }),
        )
    })?;
    let is_binary = !std::str::from_utf8(&content).is_ok();
    Ok(DownloadResponse {
        content,
        filename: format!("{}.{}", id, if is_binary { "bin" } else { "txt" }),
        is_binary,
    })
}

#[launch]
async fn rocket() -> _ {
    let account = env::var("AZURE_STORAGE_ACCOUNT").expect("AZURE_STORAGE_ACCOUNT not set");
    let access_key =
        env::var("AZURE_STORAGE_ACCESS_KEY").expect("AZURE_STORAGE_ACCESS_KEY not set");
    let exact_origin =
        env::var("RUST_ROCKET_EXACT_ORIGIN").expect("RUST_ROCKET_EXACT_ORIGIN not set");
    let container_name = "documents";

    let storage_credentials = StorageCredentials::access_key(account.clone(), access_key);
    let blob_service_client = BlobServiceClient::new(account, storage_credentials);
    let container_client = blob_service_client.container_client(container_name);

    // Check if the container exists
    match container_client.get_properties().await {
        Ok(_) => {
            // Container exists, no need to create it
            println!("Container already exists.");
        }
        Err(_) => {
            // Container doesn't exist, create it
            println!("Container does not exist. Creating...");
            container_client.create().await.unwrap(); // Handle unwrap appropriately
        }
    }

    let azure_client = AzureClient { container_client };

    let cors = CorsOptions {
        allowed_origins: AllowedOrigins::some_exact(&[
            //
            // "http://localhost:5173",
            // "http://127.0.0.1:8000",
            // "https://doc-manager.giuliohome.org",
            exact_origin,
        ]),
        allowed_methods: vec![Method::Get, Method::Post, Method::Put, Method::Delete]
            .into_iter()
            .map(From::from)
            .collect(),
        allow_credentials: true,
        ..Default::default()
    }
    .to_cors()
    .expect("CORS configuration failed");

    rocket::build()
        .manage(azure_client)
        .attach(cors)
        // Serve React static files
        .mount("/", FileServer::from("./frontend/dist"))
        // API Route
        .mount(
            "/api",
            routes![
                index,
                list_documents,
                get_document,
                create_document,
                update_document,
                delete_document,
                download_document
            ],
        )
        .register(
            "/api",
            catchers![
                payload_too_large,
                unprocessable_entity,
                not_found,
                internal_error
            ],
        )
}
