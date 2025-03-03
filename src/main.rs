#[macro_use] extern crate rocket;

use azure_storage::StorageCredentials;
use azure_storage_blobs::prelude::*;
use rocket::serde::{json::Json, Serialize, Deserialize};
use rocket::State;
use std::env;
use uuid::Uuid;
use rocket::http::{ContentType, Header, Method};
use rocket_cors::{AllowedOrigins, CorsOptions};
use rocket::fs::{FileServer};
use futures::stream::StreamExt;
use rocket::form::Form;
use rocket::fs::TempFile;
use rocket::response::Responder;
use rocket::Request;
use rocket::Response;

#[derive(Serialize, Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
struct Document {
    id: String,
    content: String,
    file_url: Option<String>,
}

#[derive(FromForm)]
struct DocumentForm<'r> {
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

#[get("/documents")]
async fn list_documents(client: &State<AzureClient>) -> Result<Json<Vec<Document>>, String> {
    let mut documents = Vec::new();
    
    let blob_list = match client.container_client.list_blobs().into_stream().next().await {
        Some(Ok(blob_list)) => blob_list,
        Some(Err(e)) => return Err(format!("Failed to list blobs: {}", e)),
        None => return Err("No blobs found".to_string()),
    };
    
    for blob in blob_list.blobs.blobs() {
        let blob_client = client.container_client.blob_client(blob.name.clone());
        let content = blob_client.get_content().await
            .map_err(|e| format!("Failed to get blob content: {}", e))?;
        documents.push(Document {
            id: blob.name.clone(),
            content: String::from_utf8(content).map_err(|e| format!("Failed to parse content: {}", e))?,
            file_url: None, // Update this if you store file URLs separately
        });
    }

    Ok(Json(documents))
}

#[get("/documents/<id>")]
async fn get_document(id: &str, client: &State<AzureClient>) -> Option<Json<Document>> {
    let blob_client = client.container_client.blob_client(id);
    match blob_client.get_content().await {
        Ok(content) => {
            let file_url = blob_client.url().ok().map(|url| url.to_string());
            Some(Json(Document {
                id: id.to_string(),
                content: String::from_utf8(content).unwrap(),
                file_url, // Return the file URL
            }))
        },
        Err(_) => None,
    }
}

#[post("/documents", data = "<form>")]
async fn create_document(mut form: Form<DocumentForm<'_>>, client: &State<AzureClient>) -> Json<Document> {
    let id = Uuid::new_v4().to_string();
    let content = form.content.as_bytes().to_vec();
    
    let blob_client = client.container_client.blob_client(&id);
    blob_client.put_block_blob(content).await.unwrap();

    let mut file_url = None;
    if let Some(mut file) = form.file.take() {
        let file_name = format!("{}_{}", id, file.name().unwrap_or("file"));
        let file_blob_client = client.container_client.blob_client(&file_name);
        file.persist_to(&file_name).await.unwrap();
        let file_content = std::fs::read(&file_name).unwrap();
        file_blob_client.put_block_blob(file_content).await.unwrap();
        std::fs::remove_file(&file_name).unwrap();
        file_url = Some(file_blob_client.url().unwrap().to_string());
    }

    Json(Document {
        id,
        content: form.content.to_string(),
        file_url, // Return the file URL if a file is uploaded
    })
}

#[put("/documents/<id>", data = "<form>")]
async fn update_document(id: &str, mut form: Form<DocumentForm<'_>>, client: &State<AzureClient>) -> Option<Json<Document>> {
    let blob_client = client.container_client.blob_client(id);
    let content = form.content.as_bytes().to_vec();
    
    match blob_client.put_block_blob(content).await {
        Ok(_) => {
            let mut file_url = None;
            if let Some(mut file) = form.file.take() {
                let file_name = format!("{}_{}", id, file.name().unwrap_or("file"));
                let file_blob_client = client.container_client.blob_client(&file_name);
                file.persist_to(&file_name).await.unwrap();
                let file_content = std::fs::read(&file_name).unwrap();
                file_blob_client.put_block_blob(file_content).await.unwrap();
                std::fs::remove_file(&file_name).unwrap();
                file_url = Some(file_blob_client.url().unwrap().to_string());
            }
            Some(Json(Document {
                id: id.to_string(),
                content: form.content.to_string(),
                file_url, // Return the file URL if a file is uploaded
            }))
        },
        Err(_) => None,
    }
}

#[delete("/documents/<id>")]
async fn delete_document(id: &str, client: &State<AzureClient>) -> Option<Json<&'static str>> {
    let blob_client = client.container_client.blob_client(id);
    match blob_client.delete().await {
        Ok(_) => Some(Json("Document deleted successfully")),
        Err(_) => None,
    }
}

struct DownloadResponse {
    content: String,
    filename: String,
}

impl<'r> Responder<'r, 'static> for DownloadResponse {
    fn respond_to(self, _: &'r Request<'_>) -> rocket::response::Result<'static> {
        Response::build()
            .header(ContentType::Plain)
            .header(Header::new("Content-Disposition", format!("attachment; filename=\"{}\"", self.filename)))
            .sized_body(self.content.len(), std::io::Cursor::new(self.content))
            .ok()
    }
}

#[get("/documents/download/<id>")]
async fn download_document(id: &str, client: &State<AzureClient>) -> Option<DownloadResponse> {
    let blob_client = client.container_client.blob_client(id);
    match blob_client.get_content().await {
        Ok(content) => Some(DownloadResponse {
            content: String::from_utf8(content).unwrap(),
            filename: format!("{}.txt", id),
        }),
        Err(_) => None,
    }
}

#[launch]
async fn rocket() -> _ {
    let account = env::var("AZURE_STORAGE_ACCOUNT").expect("AZURE_STORAGE_ACCOUNT not set");
    let access_key = env::var("AZURE_STORAGE_ACCESS_KEY").expect("AZURE_STORAGE_ACCESS_KEY not set");
    let exact_origin = env::var("RUST_ROCKET_EXACT_ORIGIN").expect("RUST_ROCKET_EXACT_ORIGIN not set");
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
        allowed_origins:
        AllowedOrigins::some_exact(&[ //
        // "http://localhost:5173",
        // "http://127.0.0.1:8000",
        // "https://doc-manager.giuliohome.org",
        exact_origin
        ]),
        allowed_methods: vec![Method::Get, Method::Post, Method::Put, Method::Delete].into_iter().map(From::from).collect(),
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
        .mount("/api", routes![
            index,
            list_documents,
            get_document,
            create_document,
            update_document,
            delete_document,
            download_document
        ])
}
