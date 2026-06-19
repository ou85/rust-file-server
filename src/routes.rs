use crate::{
    app::App, auth::UserRole, auth::authenticate, models::FileMetadata, models::LoginRequest,
    storage,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect},
    routing::{delete, get, post},
};
use axum_extra::extract::CookieJar;
use std::sync::Arc;

use axum::body::Body;
// use tokio_util::io::ReaderStream;
// use tokio::io::{AsyncReadExt, AsyncSeekExt};

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
        .route("/files/{id}/stream", get(stream_file))
        .route("/files/{id}/download", get(download_file))
        .with_state(state)
}

async fn health() -> &'static str {
    "OK"
}

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
    // println!("=== Login attempt: {}", req.username);
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

async fn list_files(
    jar: CookieJar,
    State(app): State<Arc<App>>,
) -> Result<Json<Vec<FileMetadata>>, (StatusCode, Json<serde_json::Value>)> {
    if let Err(e) = require_user(&jar) {
        return Err(e);
    }

    Ok(Json(app.list_files().unwrap()))
}

async fn get_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> Result<Json<Option<FileMetadata>>, (StatusCode, Json<serde_json::Value>)> {
    if let Err(e) = require_user(&jar) {
        return Err(e);
    }
    Ok(Json(app.get_file(&id).unwrap()))
}

async fn delete_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(e) = require_user(&jar) {
        return e;
    }
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
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> Result<(StatusCode, [(String, String); 2], Vec<u8>), (StatusCode, Json<serde_json::Value>)> {
    if let Err(e) = require_user(&jar) {
        return Err(e);
    }
    let metadata = match app.get_file(&id) {
        Ok(Some(m)) => m,
        _ => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "File not found", "id": id })),
            ));
        }
    };

    // let tmp_path = format!("/tmp/{}_download", id);
    // app.export_file(&id, &tmp_path).map_err(|e| {
    //     (
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(serde_json::json!({ "error": e.to_string(), "id": id })),
    //     )
    // })?;

    // let bytes = std::fs::read(&tmp_path).map_err(|e| {
    //     (
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(serde_json::json!({ "error": e.to_string(), "id": id })),
    //     )
    // })?;

    // let _ = std::fs::remove_file(&tmp_path);
    //-----------------------------------------------------------------------
    // Change to bytes
    let bytes = app.export_to_bytes(&id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "id": id })),
        )
    })?;
    //-----------------------------------------------------------------------

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
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> Result<(StatusCode, [(String, String); 1], Vec<u8>), (StatusCode, Json<serde_json::Value>)> {
    if let Err(e) = require_user(&jar) {
        return Err(e);
    }
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

    // // temporary path
    // let tmp_path = format!("/tmp/{}_open", id);

    // // export the file
    // if let Err(err) = app.export_file(&id, &tmp_path) {
    //     return Err((
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(serde_json::json!({
    //             "error": err.to_string(),
    //             "id": id
    //         })),
    //     ));
    // }

    // // read the file
    // let bytes = match std::fs::read(&tmp_path) {
    //     Ok(b) => b,
    //     Err(err) => {
    //         return Err((
    //             StatusCode::INTERNAL_SERVER_ERROR,
    //             Json(serde_json::json!({
    //                 "error": err.to_string(),
    //                 "id": id
    //             })),
    //         ));
    //     }
    // };

    // // delete the temporary file
    // let _ = std::fs::remove_file(&tmp_path);
    //
    //--------------------------------------------------------------
    // Change to bytes
    let bytes = app.export_to_bytes(&id).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": err.to_string(),
                "id": id
            })),
        )
    })?;
    //--------------------------------------------------------------

    // determine MIME by the ORIGINAL file name
    let mime = storage::guess_mime(&metadata.filename).to_string();

    Ok((StatusCode::OK, [("Content-Type".to_string(), mime)], bytes))
}

async fn upload_files(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    mut multipart: Multipart,
) -> Result<Json<Vec<FileMetadata>>, (StatusCode, String)> {
    if let Err((code, json)) = require_auth(&jar) {
        return Err((code, json.to_string()));
    }
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

fn require_auth(jar: &CookieJar) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match jar.get("rfs_role") {
        Some(cookie) if cookie.value() == "user" || cookie.value() == "admin" => Ok(()),
        _ => Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "Unauthorized" })),
        )),
    }
}

fn require_user(jar: &CookieJar) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match jar.get("rfs_role") {
        Some(cookie) if cookie.value() == "user" => Ok(()),
        _ => Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Access denied" })),
        )),
    }
}

async fn stream_file(
    _jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    // --------------------------------------------------------------
    // if let Err(e) = require_auth(&jar) {
    //     return Err(e);
    // }

    // let metadata = match app.get_file(&id) {
    //     Ok(Some(m)) => m,
    //     _ => {
    //         return Err((
    //             StatusCode::NOT_FOUND,
    //             Json(serde_json::json!({
    //                 "error": "File not found",
    //                 "id": id
    //             })),
    //         ));
    //     }
    // };

    // let tmp_path = format!("/tmp/{}_stream", id);

    // app.export_file(&id, &tmp_path).map_err(|e| {
    //     (
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(serde_json::json!({
    //             "error": e.to_string()
    //         })),
    //     )
    // })?;

    // let mut file = tokio::fs::File::open(&tmp_path).await.map_err(|e| {
    //     (
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(serde_json::json!({
    //             "error": e.to_string()
    //         })),
    //     )
    // })?;

    // let file_size = file
    //     .metadata()
    //     .await
    //     .map_err(|e| {
    //         (
    //             StatusCode::INTERNAL_SERVER_ERROR,
    //             Json(serde_json::json!({
    //                 "error": e.to_string()
    //             })),
    //         )
    //     })?
    //     .len();

    // let mime = storage::guess_mime(&metadata.filename);

    // if let Some(range_header) = headers.get(header::RANGE) {
    //     if let Ok(range_str) = range_header.to_str() {
    //         if let Some(range) = range_str.strip_prefix("bytes=") {
    //             let parts: Vec<&str> = range.split('-').collect();

    //             let start: u64 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);

    //             let end: u64 = parts
    //                 .get(1)
    //                 .and_then(|s| s.parse().ok())
    //                 .unwrap_or(file_size - 1);

    //             let end = end.min(file_size - 1);

    //             let length = end - start + 1;

    //             file.seek(std::io::SeekFrom::Start(start))
    //                 .await
    //                 .map_err(|e| {
    //                     (
    //                         StatusCode::INTERNAL_SERVER_ERROR,
    //                         Json(serde_json::json!({
    //                             "error": e.to_string()
    //                         })),
    //                     )
    //                 })?;

    //             let mut buffer = vec![0u8; length as usize];

    //             file.read_exact(&mut buffer).await.map_err(|e| {
    //                 (
    //                     StatusCode::INTERNAL_SERVER_ERROR,
    //                     Json(serde_json::json!({
    //                         "error": e.to_string()
    //                     })),
    //                 )
    //             })?;

    //             let _ = tokio::fs::remove_file(&tmp_path).await;

    //             let mut partial_headers = HeaderMap::new();

    //             partial_headers.insert(header::CONTENT_TYPE, mime.parse().unwrap());

    //             partial_headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());

    //             partial_headers.insert(
    //                 header::CONTENT_RANGE,
    //                 format!("bytes {}-{}/{}", start, end, file_size)
    //                     .parse()
    //                     .unwrap(),
    //             );

    //             partial_headers.insert(header::CONTENT_LENGTH, length.to_string().parse().unwrap());

    //             return Ok((
    //                 StatusCode::PARTIAL_CONTENT,
    //                 partial_headers,
    //                 Body::from(buffer),
    //             ));
    //         }
    //     }
    // }

    // let bytes = tokio::fs::read(&tmp_path).await.map_err(|e| {
    //     (
    //         StatusCode::INTERNAL_SERVER_ERROR,
    //         Json(serde_json::json!({
    //             "error": e.to_string()
    //         })),
    //     )
    // })?;

    // let _ = tokio::fs::remove_file(&tmp_path).await;

    // let mut headers = HeaderMap::new();

    // headers.insert(header::CONTENT_TYPE, mime.parse().unwrap());

    // headers.insert(
    //     header::CONTENT_LENGTH,
    //     bytes.len().to_string().parse().unwrap(),
    // );

    // Ok((StatusCode::OK, headers, Body::from(bytes)))
    // --------------------------------------------------------------
    //
    // Refactor to bytes
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

    let bytes = app.export_to_bytes(&id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let file_size = bytes.len() as u64;
    let mime = storage::guess_mime(&metadata.filename);

    if let Some(range_header) = headers.get(header::RANGE) {
        if let Ok(range_str) = range_header.to_str() {
            if let Some(range) = range_str.strip_prefix("bytes=") {
                let parts: Vec<&str> = range.split('-').collect();
                let start: u64 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
                let end: u64 = parts
                    .get(1)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(file_size - 1)
                    .min(file_size - 1);

                let chunk = bytes[start as usize..=end as usize].to_vec();
                let length = chunk.len();

                let mut partial_headers = HeaderMap::new();
                partial_headers.insert(header::CONTENT_TYPE, mime.parse().unwrap());
                partial_headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
                partial_headers.insert(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, file_size)
                        .parse()
                        .unwrap(),
                );
                partial_headers.insert(header::CONTENT_LENGTH, length.to_string().parse().unwrap());

                return Ok((
                    StatusCode::PARTIAL_CONTENT,
                    partial_headers,
                    Body::from(chunk),
                ));
            }
        }
    }

    // Full file
    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(header::CONTENT_TYPE, mime.parse().unwrap());
    resp_headers.insert(
        header::CONTENT_LENGTH,
        file_size.to_string().parse().unwrap(),
    );

    Ok((StatusCode::OK, resp_headers, Body::from(bytes)))
}
