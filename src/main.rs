#[macro_use] extern crate rocket;

use azure_storage::StorageCredentials;
use azure_storage_blobs::prelude::*;

use rocket::serde::{json::Json, Serialize, Deserialize};
use rocket::State;
use std::env;
use uuid::Uuid;

use rocket::http::Method;
// use rocket::{get, routes};
use rocket_cors::{AllowedOrigins, CorsOptions};

use rocket::fs::{FileServer};

use futures::stream::StreamExt;

#[derive(Serialize, Deserialize, Clone)]
#[serde(crate = "rocket::serde")]
struct Document {
    id: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
struct NewDocumentRequest {
    content: String,
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
struct UpdateDocumentRequest {
    content: String,
}

struct AzureClient {
    container_client: ContainerClient,
}


#[get("/")]
fn index() -> &'static str {
    "Welcome to the Document Manager! Use /documents endpoints to manage documents."
}

#[get("/documents")]
async fn list_documents(client: &State<AzureClient>) -> Json<Vec<Document>> {
    let mut documents = Vec::new();
    
    let blob_list = client.container_client.list_blobs().into_stream().next().await.unwrap().unwrap();
    
    for blob in blob_list.blobs.blobs() {
        let blob_client = client.container_client.blob_client(blob.name.clone());
        let content = blob_client.get_content().await.unwrap();
        documents.push(Document {
            id: blob.name.clone(),
            content: String::from_utf8(content).unwrap(),
        });
    }

    Json(documents)
}


#[get("/documents/<id>")]
async fn get_document(id: &str, client: &State<AzureClient>) -> Option<Json<Document>> {
    let blob_client = client.container_client.blob_client(id);
    match blob_client.get_content().await {
        Ok(content) => Some(Json(Document {
            id: id.to_string(),
            content: String::from_utf8(content).unwrap(),
        })),
        Err(_) => None,
    }
}


#[post("/documents", format = "json", data = "<request>")]
async fn create_document(request: Json<NewDocumentRequest>, client: &State<AzureClient>) -> Json<Document> {
    let id = Uuid::new_v4().to_string();
    let content = request.content.as_bytes().to_vec();
    
    let blob_client = client.container_client.blob_client(&id);
    blob_client.put_block_blob(content).await.unwrap();

    Json(Document {
        id,
        content: request.content.clone(),
    })
}

#[put("/documents/<id>", format = "json", data = "<request>")]
async fn update_document(
    id: &str,
    request: Json<UpdateDocumentRequest>,
    client: &State<AzureClient>,
) -> Option<Json<Document>> {
    let blob_client = client.container_client.blob_client(id);
    let content = request.content.as_bytes().to_vec();
    
    match blob_client.put_block_blob(content).await {
        Ok(_) => Some(Json(Document {
            id: id.to_string(),
            content: request.content.clone(),
        })),
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

#[launch]
async fn rocket() -> _ {
    let account = env::var("AZURE_STORAGE_ACCOUNT").expect("AZURE_STORAGE_ACCOUNT not set");
    let access_key = env::var("AZURE_STORAGE_ACCESS_KEY").expect("AZURE_STORAGE_ACCESS_KEY not set");
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
        AllowedOrigins::some_exact(&[ // 3.
        "http://127.0.0.1:5173",
        "http://127.0.0.1:8000",
        "https://doc-manager.giuliohome.org",
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
            delete_document
        ])
}
