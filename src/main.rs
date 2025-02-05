#[macro_use] extern crate rocket;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rocket::serde::{json::Json, Serialize, Deserialize};
use rocket::State;
use uuid::Uuid;

use rocket::http::Method;
// use rocket::{get, routes};
use rocket_cors::{AllowedOrigins, CorsOptions};

use rocket::fs::{FileServer};

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

type Documents = Arc<Mutex<HashMap<String, Document>>>;

#[get("/")]
fn index() -> &'static str {
    "Welcome to the Document Manager! Use /documents endpoints to manage documents."
}

#[get("/documents")]
fn list_documents(docs: &State<Documents>) -> Json<Vec<Document>> {
    let docs_map = docs.lock().unwrap();
    let documents: Vec<Document> = docs_map.values().cloned().collect();
    Json(documents)
}

#[get("/documents/<id>")]
fn get_document(id: String, docs: &State<Documents>) -> Option<Json<Document>> {
    let docs_map = docs.lock().unwrap();
    docs_map.get(&id).map(|doc| Json(doc.clone()))
}

#[post("/documents", format = "json", data = "<request>")]
fn create_document(request: Json<NewDocumentRequest>, docs: &State<Documents>) -> Json<Document> {
    let id = Uuid::new_v4().to_string();
    let new_doc = Document {
        id: id.clone(),
        content: request.content.clone(),
    };

    let mut docs_map = docs.lock().unwrap();
    docs_map.insert(id, new_doc.clone());
    Json(new_doc)
}

#[put("/documents/<id>", format = "json", data = "<request>")]
fn update_document(
    id: String,
    request: Json<UpdateDocumentRequest>,
    docs: &State<Documents>,
) -> Option<Json<Document>> {
    let mut docs_map = docs.lock().unwrap();
    if let Some(doc) = docs_map.get_mut(&id) {
        doc.content = request.content.clone();
        return Some(Json(doc.clone()));
    }
    None
}

#[delete("/documents/<id>")]
fn delete_document(id: String, docs: &State<Documents>) -> Option<Json<&'static str>> {
    let mut docs_map = docs.lock().unwrap();
    if docs_map.remove(&id).is_some() {
        Some(Json("Document deleted successfully"))
    } else {
        None
    }
}

#[launch]
fn rocket() -> _ {
    let initial_docs = Documents::default();
    
    let cors = CorsOptions {
        allowed_origins: AllowedOrigins::all(),
        allowed_methods: vec![Method::Get, Method::Post, Method::Put, Method::Delete].into_iter().map(From::from).collect(),
        allow_credentials: true,
        ..Default::default()
    }
    .to_cors()
    .expect("CORS configuration failed");
    
    rocket::build()
        .manage(initial_docs)
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
