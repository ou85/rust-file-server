use crate::{
    app::App, auth::UserRole, auth::authenticate, models::FileMetadata, models::LoginRequest,
    storage,
};
use axum::extract::{DefaultBodyLimit, Multipart, Path, State};
use axum::{
    Json, Router,
    http::StatusCode,
    http::header,
    response::Html,
    response::IntoResponse,
    response::Redirect,
    routing::{delete, get, post},
};
use std::sync::Arc;
use axum_extra::extract::CookieJar;

pub fn create_router(state: Arc<App>) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/login", post(login))
        .route("/login", get(login_page))
        .route("/logout", post(logout))
        .route("/health", get(health))
        .route("/files", get(list_files))
        .route("/files/{id}", get(get_file))
        .route("/files/upload", post(upload_files))
        .layer(DefaultBodyLimit::disable())
        .route("/files/{id}", delete(delete_file))
        .route("/files/{id}/open", get(open_file))
        .route("/files/{id}/download", get(download_file))
        .with_state(state)
}

async fn health() -> &'static str {
    "OK"
}

// pub async fn root() -> Redirect {
//     Redirect::to("/login")
// }

pub async fn root(jar: CookieJar) -> impl IntoResponse {
    match jar.get("rfs_role") {
        Some(cookie) if cookie.value() == "user" => {
            Html(include_str!("../assets/ui.html")).into_response()
        }

        Some(cookie) if cookie.value() == "admin" => {
            Html(include_str!("../assets/admin.html")).into_response()
        }

        _ => Redirect::to("/login").into_response(),
    }
}

async fn login_page() -> Html<&'static str> {
    Html(include_str!("../assets/login.html"))
}

async fn login(State(app): State<Arc<App>>, Json(req): Json<LoginRequest>) -> impl IntoResponse {
    println!("=== Login attempt: {}", req.username);
    match authenticate(&req.username, &req.password, &app.config) {
        Some(role) => {
            println!("=== Login success");
            let value = match role {
                UserRole::User => "user",
                UserRole::Admin => "admin",
            };

            (
                StatusCode::OK,
                [(
                    header::SET_COOKIE,
                    format!("rfs_role={}; Path=/; HttpOnly", value),
                )],
            )
        }

        None => {
            println!("=== Login failed");

            (
                StatusCode::UNAUTHORIZED,
                [(header::SET_COOKIE, "".to_string())],
            )
        }
    }
}

async fn list_files(State(app): State<Arc<App>>) -> Json<Vec<FileMetadata>> {
    Json(app.list_files().unwrap())
}

async fn get_file(
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> Json<Option<FileMetadata>> {
    Json(app.get_file(&id).unwrap())
}

async fn delete_file(
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> (StatusCode, Json<serde_json::Value>) {
    match app.delete_file(&id) {
        Ok(_) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "deleted": true,
                "id": id
            })),
        ),

        Err(err) => {
            // Check if the file was not found
            let msg = err.to_string();

            if msg.contains("not found") || msg.contains("No such file") {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "deleted": false,
                        "id": id,
                        "error": "File not found"
                    })),
                );
            }

            // Any other error → 500
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "deleted": false,
                    "id": id,
                    "error": msg
                })),
            )
        }
    }
}

async fn download_file(
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> Result<(StatusCode, [(String, String); 2], Vec<u8>), (StatusCode, Json<serde_json::Value>)> {
    let metadata = match app.get_file(&id) {
        Ok(Some(m)) => m,
        _ => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "File not found", "id": id })),
            ));
        }
    };

    let tmp_path = format!("/tmp/{}_download", id);
    app.export_file(&id, &tmp_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "id": id })),
        )
    })?;

    let bytes = std::fs::read(&tmp_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "id": id })),
        )
    })?;

    let _ = std::fs::remove_file(&tmp_path);

    let mime = storage::guess_mime(&metadata.filename).to_string();

    Ok((
        StatusCode::OK,
        [
            ("Content-Type".to_string(), mime),
            (
                "Content-Disposition".to_string(),
                format!("attachment; filename=\"{}\"", metadata.filename),
            ),
        ],
        bytes,
    ))
}

async fn open_file(
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> Result<(StatusCode, [(String, String); 1], Vec<u8>), (StatusCode, Json<serde_json::Value>)> {
    // get metadata to find out the file name.
    let metadata = match app.get_file(&id) {
        Ok(Some(m)) => m,
        _ => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "File not found",
                    "id": id
                })),
            ));
        }
    };

    // limit size for open file to 200MB
    let max_preview_size = app.config.max_preview_size_bytes;

    if metadata.size > max_preview_size {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": "File too large for preview",
                "max_size_mb": max_preview_size,
                "file_size": metadata.size,
                "id": id
            })),
        ));
    }

    // temporary path
    let tmp_path = format!("/tmp/{}_open", id);

    // export the file
    if let Err(err) = app.export_file(&id, &tmp_path) {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": err.to_string(),
                "id": id
            })),
        ));
    }

    // read the file
    let bytes = match std::fs::read(&tmp_path) {
        Ok(b) => b,
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": err.to_string(),
                    "id": id
                })),
            ));
        }
    };

    // delete the temporary file
    let _ = std::fs::remove_file(&tmp_path);

    // determine MIME by the ORIGINAL file name
    let mime = storage::guess_mime(&metadata.filename).to_string();

    Ok((StatusCode::OK, [("Content-Type".to_string(), mime)], bytes))
}

pub async fn upload_files(
    State(app): State<Arc<App>>,
    mut multipart: Multipart,
) -> Result<Json<Vec<FileMetadata>>, (StatusCode, String)> {
    let mut uploaded = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Multipart error: {}", e)))?
    {
        let filename = field.file_name().unwrap_or("unknown").to_string();

        let data = field
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Read error: {}", e)))?;

        let metadata = app.import_bytes(&filename, data.to_vec()).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Import error: {}", e),
            )
        })?;

        println!(
            "=== Uploaded: {} ({:.2} MB)",
            metadata.filename,
            metadata.size as f64 / 1024.0 / 1024.0
        );

        uploaded.push(metadata);
    }

    if uploaded.is_empty() {
        println!("Error: No files uploaded (StatusCode::BAD_REQUEST)");
        return Err((StatusCode::BAD_REQUEST, "No files uploaded".into()));
    }

    Ok(Json(uploaded))
}

async fn logout() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            "rfs_role=; Path=/; Max-Age=0; HttpOnly".to_string(),
        )],
    )
}