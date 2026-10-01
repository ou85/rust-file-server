use crate::{
    app::App, auth::UserRole, auth::authenticate, blob_store, domain::FileMetadata,
    domain::LoginRequest,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect},
    routing::{delete, get, post},
};
use axum_extra::extract::CookieJar;
use bytes::Bytes;
use std::sync::Arc;

use axum::body::Body;

use serde_json::json;

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

pub async fn root(jar: CookieJar, State(app): State<Arc<App>>) -> impl IntoResponse {
    match jar
        .get("rfs_session")
        .and_then(|cookie| app.sessions.role(cookie.value()))
    {
        Some(UserRole::User) => Html(include_str!("../../assets/ui.html")).into_response(),

        Some(UserRole::Admin) => Html(include_str!("../../assets/admin.html")).into_response(),

        _ => Redirect::to("/login").into_response(),
    }
}

async fn login_page() -> Html<&'static str> {
    Html(include_str!("../../assets/login.html"))
}

async fn login(State(app): State<Arc<App>>, Json(req): Json<LoginRequest>) -> impl IntoResponse {
    match authenticate(&req.username, &req.password, &app.config) {
        Some(role) => {
            println!("\n=== Login success");
            let token = app.sessions.create(role);

            (
                StatusCode::OK,
                [(
                    header::SET_COOKIE,
                    format!("rfs_session={token}; Path=/; HttpOnly; SameSite=Strict"),
                )],
            )
        }

        None => {
            println!("\n=== Login failed");

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
    if let Err(e) = require_user(&jar, &app) {
        return Err(e);
    }

    match app.list_files() {
        Ok(files) => Ok(Json(files)),
        Err(e) => {
            tracing::error!("list_files failed: {}", e);
            Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": e.to_string() })),
            ))
        }
    }
}

async fn get_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> Result<Json<Option<FileMetadata>>, (StatusCode, Json<serde_json::Value>)> {
    if let Err(e) = require_user(&jar, &app) {
        return Err(e);
    }
    Ok(Json(app.get_file(&id).unwrap()))
}

async fn delete_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> (StatusCode, Json<serde_json::Value>) {
    if let Err(e) = require_user(&jar, &app) {
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
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if let Err(e) = require_user(&jar, &app) {
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

    let chunks = app.export_chunked(&id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string(), "id": id })),
        )
    })?;

    // Get streams from chunks
    let stream = futures::stream::iter(chunks.map(|chunk| {
        chunk
            .map(|bytes| Bytes::from(bytes))
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }));

    let mime = blob_store::guess_mime(&metadata.filename);

    let mut headers = secure_file_headers(&mime, &metadata.filename, false);
    headers.insert(
        header::CONTENT_LENGTH,
        metadata.size.to_string().parse().unwrap(),
    );
    Ok((StatusCode::OK, headers, Body::from_stream(stream)))
}

async fn open_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if let Err(e) = require_user(&jar, &app) {
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

    Ok(Redirect::to(&format!("/files/{}/stream", metadata.id)).into_response())
}

async fn upload_files(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    mut multipart: Multipart,
) -> Result<Json<Vec<FileMetadata>>, (StatusCode, String)> {
    if let Err((code, json)) = require_user(&jar, &app) {
        return Err((code, json.to_string()));
    }

    let mut uploaded = Vec::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Multipart error: {}", e)))?
    {
        use crate::crypto::CHUNK_SIZE;
        use std::io::Write;

        let filename = field.file_name().unwrap_or("unknown").to_string();
        let id = crate::domain::id::id_16();

        let mut file = app.storage.create_file_writer(&id).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("IO error: {}", e),
            )
        })?;

        let mut encryptor = app.crypto.start_encryption(&id).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Encrypt error: {e}"),
            )
        })?;
        file.write_all(encryptor.header()).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("IO error: {}", e),
            )
        })?;

        let mut buffer = Vec::with_capacity(CHUNK_SIZE);
        let mut total_bytes: u64 = 0;

        // Read from multipart in chunks
        while let Some(bytes) = field
            .chunk()
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Read error: {}", e)))?
        {
            buffer.extend_from_slice(&bytes);
            total_bytes += bytes.len() as u64;

            // Upload indicator
            // print!(
            //     "\rUploading {}: {:.2} MB",
            //     filename,
            //     total_bytes as f64 / 1024.0 / 1024.0
            // );
            print!("\rUploading {:.2} MB", total_bytes as f64 / 1024.0 / 1024.0);
            std::io::stdout().flush().unwrap();

            // Accumulated a full chunk — encrypt and write
            while buffer.len() >= CHUNK_SIZE {
                let chunk = buffer[..CHUNK_SIZE].to_vec();
                buffer.drain(..CHUNK_SIZE);

                let encrypted = encryptor.encrypt_chunk(&chunk).map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Encrypt error: {e}"),
                    )
                })?;
                file.write_all(&encrypted)
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("IO error: {e}")))?;
            }
        }

        // Last incomplete chunk
        if !buffer.is_empty() {
            let encrypted = encryptor.encrypt_chunk(&buffer).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Encrypt error: {e}"),
                )
            })?;
            file.write_all(&encrypted)
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("IO error: {e}")))?;
        }

        file.flush().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("IO error: {}", e),
            )
        })?;
        file.sync_all().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("IO sync error: {e}"),
            )
        })?;

        // Save metadata
        let metadata = crate::domain::FileMetadata {
            id: id.clone(),
            filename: filename.clone(),
            size: total_bytes,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        // Publish only a fully synced encrypted file. Startup cleanup removes a blob
        // if the process later crashes before its metadata transaction succeeds.
        if let Err(e) = app.storage.finalize_file(&id) {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Finalize error: {}", e),
            ));
        }
        if let Err(e) = app.metadata.save_file(&metadata) {
            let _ = app.storage.delete_file(&id);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("DB error: {}", e),
            ));
        }

        println!("\nUploaded  {:.2} MB", total_bytes as f64 / 1024.0 / 1024.0);

        uploaded.push(metadata);
    }

    if uploaded.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "No files uploaded".into()));
    }

    Ok(Json(uploaded))
}

async fn logout(jar: CookieJar, State(app): State<Arc<App>>) -> impl IntoResponse {
    if let Some(cookie) = jar.get("rfs_session") {
        app.sessions.remove(cookie.value());
    }
    // Cleaning up temporary files older than 4 hours
    match app.storage.cleanup_stale_tmp_files(4 * 3600) {
        Ok(count) if count > 0 => println!("=== Logout cleanup: removed {} stale tmp files", count),
        Ok(_) => {}
        Err(e) => eprintln!("=== Logout cleanup error: {}", e),
    }

    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            "rfs_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict".to_string(),
        )],
    )
}

fn require_user(jar: &CookieJar, app: &App) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    match jar
        .get("rfs_session")
        .and_then(|cookie| app.sessions.role(cookie.value()))
    {
        Some(UserRole::User) => Ok(()),
        _ => Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "Access denied" })),
        )),
    }
}

async fn stream_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, (StatusCode, Json<serde_json::Value>)> {
    if let Err(e) = require_user(&jar, &app) {
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

    let file_size = metadata.size;
    let mime = blob_store::guess_mime(&metadata.filename);

    let range = headers
        .get(header::RANGE)
        .map(|value| value.to_str().ok())
        .flatten();
    match parse_single_range(range, file_size) {
        Err(()) => return Err(range_not_satisfiable(file_size)),
        Ok(Some((start, end))) => {
            if start > end || start >= file_size {
                return Err((
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    Json(serde_json::json!({
                        "error": "Invalid range",
                        "file_size": file_size
                    })),
                ));
            }

            let length = end - start + 1;

            let chunks = app.export_range(&id, start, end).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
            })?;

            let stream = futures::stream::iter(
                chunks.map(|c| c.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
            );

            let mut resp_headers =
                secure_file_headers(&mime, &metadata.filename, inline_safe(&mime));
            resp_headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
            resp_headers.insert(
                header::CONTENT_RANGE,
                format!("bytes {}-{}/{}", start, end, file_size)
                    .parse()
                    .unwrap(),
            );
            resp_headers.insert(header::CONTENT_LENGTH, length.to_string().parse().unwrap());

            return Ok((
                StatusCode::PARTIAL_CONTENT,
                resp_headers,
                Body::from_stream(stream),
            )
                .into_response());
        }
        Ok(None) => {}
    }

    // Full file without Range
    let chunks = app.export_chunked(&id).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    let stream = futures::stream::iter(chunks.map(|c| {
        c.map(Bytes::from)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
    }));

    let mut resp_headers = secure_file_headers(&mime, &metadata.filename, inline_safe(&mime));
    resp_headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
    resp_headers.insert(
        header::CONTENT_LENGTH,
        file_size.to_string().parse().unwrap(),
    );

    Ok((StatusCode::OK, resp_headers, Body::from_stream(stream)).into_response())
}

fn parse_single_range(range: Option<&str>, file_size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(range) = range else {
        return Ok(None);
    };
    if file_size == 0 || !range.starts_with("bytes=") {
        return Err(());
    }
    let value = &range[6..];
    if value.contains(',') {
        return Err(());
    }
    let (start, end) = value.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix: u64 = end.parse().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        return Ok(Some((file_size.saturating_sub(suffix), file_size - 1)));
    }
    let start: u64 = start.parse().map_err(|_| ())?;
    if start >= file_size {
        return Err(());
    }
    let end = if end.is_empty() {
        file_size - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(file_size - 1)
    };
    if start > end {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn range_not_satisfiable(file_size: u64) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::RANGE_NOT_SATISFIABLE,
        Json(json!({ "error": "Invalid range", "file_size": file_size })),
    )
}

fn inline_safe(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
    ) || mime.starts_with("video/")
        || mime.starts_with("audio/")
}

fn secure_file_headers(mime: &str, filename: &str, inline: bool) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, mime.parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    let disposition = if inline { "inline" } else { "attachment" };
    let encoded_name = percent_encode_filename(filename);
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("{disposition}; filename=\"download\"; filename*=UTF-8''{encoded_name}")
            .parse()
            .unwrap(),
    );
    headers
}

fn percent_encode_filename(filename: &str) -> String {
    filename
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_') {
                format!("{}", byte as char)
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_standard_open_and_suffix_ranges() {
        assert_eq!(parse_single_range(Some("bytes=2-5"), 10), Ok(Some((2, 5))));
        assert_eq!(parse_single_range(Some("bytes=7-"), 10), Ok(Some((7, 9))));
        assert_eq!(parse_single_range(Some("bytes=-3"), 10), Ok(Some((7, 9))));
        assert!(parse_single_range(Some("bytes=0-1,3-4"), 10).is_err());
    }
    #[test]
    fn encodes_filename_without_header_injection() {
        assert_eq!(percent_encode_filename("a\r\nb.txt"), "a%0D%0Ab.txt");
    }
}
