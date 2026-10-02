use crate::{
    app::App,
    blob_store,
    domain::{BulkDeleteRequest, FileMetadata, LoginRequest, UserRole},
};
use axum::{
    Json, Router,
    extract::{
        DefaultBodyLimit, Multipart, Path, Query, State,
        multipart::MultipartRejection,
        rejection::{JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{delete, get, post},
};
use axum_extra::extract::CookieJar;
use bytes::Bytes;
use std::sync::Arc;

use axum::body::Body;

use serde_json::json;
mod error;
use axum::http::HeaderValue;
use error::{ApiError, ApiResult};

fn log_stream_error(error: std::io::Error) -> std::io::Error {
    tracing::error!(error = %error, "File streaming failed");
    error
}

#[derive(serde::Serialize)]
struct StorageStats {
    used_bytes: u64,
    total_bytes: u64,
    available_bytes: u64,
}

#[derive(serde::Deserialize)]
struct FileListQuery {
    page: Option<usize>,
    per_page: Option<usize>,
    search: Option<String>,
    sort: Option<String>,
}

#[derive(serde::Serialize)]
struct FileListResponse {
    files: Vec<FileMetadata>,
    total: usize,
    page: usize,
    per_page: usize,
}

pub fn create_router(state: Arc<App>) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/login", post(login))
        .route("/login", get(login_page))
        .route("/logout", post(logout))
        .route("/health", get(health))
        .route("/storage", get(storage_stats))
        .route("/me", get(current_user))
        .route("/assets/icons/{name}", get(icon))
        .route("/files", get(list_files).delete(delete_files))
        .route("/files/{id}", get(get_file))
        .route("/files/upload", post(upload_files))
        .layer(DefaultBodyLimit::disable())
        .route("/files/{id}", delete(delete_file))
        .route("/files/{id}/open", get(open_file))
        .route("/files/{id}/player", get(video_player))
        .route("/files/{id}/stream", get(stream_file))
        .route("/files/{id}/download", get(download_file))
        .fallback(|| async { ApiError::NotFound("Page not found") })
        .layer(axum::middleware::from_fn(error::browser_error_pages))
        .with_state(state)
}

async fn health() -> &'static str {
    "OK"
}

async fn current_user(
    jar: CookieJar,
    State(app): State<Arc<App>>,
) -> ApiResult<Json<serde_json::Value>> {
    let account = session_user(&jar, &app)?;
    Ok(Json(json!({ "username": account.username })))
}

async fn icon(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
) -> ApiResult<impl IntoResponse> {
    require_user(&jar, &app)?;
    let (bytes, content_type) = match name.as_str() {
        "cancel.png" => (
            include_bytes!("../../assets/icons/cancel.png").as_slice(),
            "image/png",
        ),
        "delete.png" => (
            include_bytes!("../../assets/icons/delete.png").as_slice(),
            "image/png",
        ),
        "download.png" => (
            include_bytes!("../../assets/icons/download.png").as_slice(),
            "image/png",
        ),
        "cancel-48.gif" => (
            include_bytes!("../../assets/icons/cancel-48.gif").as_slice(),
            "image/gif",
        ),
        "cancel-static.png" => (
            include_bytes!("../../assets/icons/cancel-static.png").as_slice(),
            "image/png",
        ),
        "trash-48.gif" => (
            include_bytes!("../../assets/icons/trash-48.gif").as_slice(),
            "image/gif",
        ),
        "trash-static.png" => (
            include_bytes!("../../assets/icons/trash-static.png").as_slice(),
            "image/png",
        ),
        "download-48.gif" => (
            include_bytes!("../../assets/icons/download-48.gif").as_slice(),
            "image/gif",
        ),
        "download-48-static.png" => (
            include_bytes!("../../assets/icons/download-48-static.png").as_slice(),
            "image/png",
        ),
        _ => {
            return Err(ApiError::NotFound("Icon not found"));
        }
    };
    Ok(([(header::CONTENT_TYPE, content_type)], bytes))
}

async fn storage_stats(
    jar: CookieJar,
    State(app): State<Arc<App>>,
) -> ApiResult<Json<StorageStats>> {
    require_user(&jar, &app)?;
    let path = std::ffi::CString::new(app.config.data_dir.as_os_str().as_encoded_bytes())
        .map_err(|error| ApiError::internal("Invalid data directory", error))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(ApiError::internal(
            "Read filesystem statistics",
            std::io::Error::last_os_error(),
        ));
    }
    let stats = unsafe { stats.assume_init() };
    let block_size = stats.f_frsize as u64;
    let total_bytes = (stats.f_blocks as u64).saturating_mul(block_size);
    let available_bytes = (stats.f_bavail as u64).saturating_mul(block_size);
    let used_bytes = directory_size(&app.config.data_dir)
        .map_err(|error| ApiError::internal("Measure data directory", error))?;
    Ok(Json(StorageStats {
        used_bytes,
        total_bytes,
        available_bytes,
    }))
}

fn directory_size(path: &std::path::Path) -> std::io::Result<u64> {
    let mut total: u64 = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            total = total.saturating_add(directory_size(&entry.path())?);
        } else if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}

pub async fn root(jar: CookieJar, State(app): State<Arc<App>>) -> impl IntoResponse {
    match jar
        .get("rfs_session")
        .and_then(|cookie| app.current_user(cookie.value()).ok().flatten())
    {
        Some(account) if account.role == UserRole::User => {
            Html(include_str!("../../assets/ui.html")).into_response()
        }
        Some(account) if account.role == UserRole::Admin => {
            Html(include_str!("../../assets/admin.html")).into_response()
        }
        _ => Redirect::to("/login").into_response(),
    }
}

async fn login_page() -> Html<&'static str> {
    Html(include_str!("../../assets/login.html"))
}

async fn login(
    State(app): State<Arc<App>>,
    request: Result<Json<LoginRequest>, JsonRejection>,
) -> ApiResult<Response> {
    let Json(req) = request.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    let account = app
        .metadata
        .get_user_by_username(&req.username)
        .map_err(|error| ApiError::internal("Read user account", error))?;
    match account.filter(|account| crate::auth::verify_password(&req.password, account)) {
        Some(account) => {
            tracing::info!(username = %account.username, role = ?account.role, "Login succeeded");
            let token = app
                .sessions
                .create(&account)
                .map_err(|error| ApiError::internal("Create session", error))?;

            Ok((
                StatusCode::OK,
                [(
                    header::SET_COOKIE,
                    format!("rfs_session={token}; Path=/; HttpOnly; SameSite=Strict"),
                )],
            )
                .into_response())
        }

        None => {
            tracing::warn!("Login failed");

            Err(ApiError::Unauthorized)
        }
    }
}

async fn list_files(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    query: Result<Query<FileListQuery>, QueryRejection>,
) -> ApiResult<Json<FileListResponse>> {
    require_user(&jar, &app)?;
    let Query(query) = query.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;

    match app.list_files() {
        Ok(mut files) => {
            let search = query.search.unwrap_or_default().to_lowercase();
            files.retain(|file| file.filename.to_lowercase().contains(&search));
            match query.sort.as_deref().unwrap_or("recent") {
                "name-asc" => {
                    files.sort_by(|a, b| a.filename.to_lowercase().cmp(&b.filename.to_lowercase()))
                }
                "name-desc" => {
                    files.sort_by(|a, b| b.filename.to_lowercase().cmp(&a.filename.to_lowercase()))
                }
                "size-desc" => files.sort_by(|a, b| b.size.cmp(&a.size)),
                "size-asc" => files.sort_by(|a, b| a.size.cmp(&b.size)),
                "oldest" => files.sort_by_key(|file| file.created_at),
                _ => files.sort_by_key(|file| std::cmp::Reverse(file.created_at)),
            }
            let total = files.len();
            let per_page = match query.per_page.unwrap_or(20) {
                10 => 10,
                50 => 50,
                _ => 20,
            };
            let total_pages = total.div_ceil(per_page).max(1);
            let page = query.page.unwrap_or(1).clamp(1, total_pages);
            let start = (page - 1) * per_page;
            let files = files.into_iter().skip(start).take(per_page).collect();
            Ok(Json(FileListResponse {
                files,
                total,
                page,
                per_page,
            }))
        }
        Err(error) => Err(ApiError::internal("List files", error)),
    }
}

async fn get_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> ApiResult<Json<Option<FileMetadata>>> {
    if let Err(e) = require_user(&jar, &app) {
        return Err(e);
    }
    Ok(Json(app.get_file(&id).map_err(|error| {
        ApiError::internal("Read file metadata", error)
    })?))
}

async fn delete_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> ApiResult<Json<serde_json::Value>> {
    require_user(&jar, &app)?;
    app.delete_file(&id)
        .map_err(|error| ApiError::internal("Delete file", error))?;
    tracing::info!(file_id = %id, "File deleted");
    Ok(Json(json!({"deleted": true, "id": id})))
}

async fn delete_files(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    request: Result<Json<BulkDeleteRequest>, JsonRejection>,
) -> ApiResult<Json<serde_json::Value>> {
    require_user(&jar, &app)?;
    let Json(request) = request.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;
    if request.ids.is_empty() || request.ids.len() > 1_000 {
        return Err(ApiError::BadRequest(
            "Provide between 1 and 1000 file IDs".into(),
        ));
    }

    let mut deleted = Vec::with_capacity(request.ids.len());
    for id in request.ids {
        app.delete_file(&id)
            .map_err(|error| ApiError::internal("Delete selected file", error))?;
        deleted.push(id);
    }
    tracing::info!(count = deleted.len(), "Selected files deleted");
    Ok(Json(json!({ "deleted": deleted })))
}

fn lookup_file(app: &App, id: &str) -> ApiResult<FileMetadata> {
    app.get_file(id)
        .map_err(|error| ApiError::internal("Read file metadata", error))?
        .ok_or(ApiError::NotFound("File not found"))
}

async fn download_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> ApiResult<impl IntoResponse> {
    if let Err(e) = require_user(&jar, &app) {
        return Err(e);
    }

    let metadata = lookup_file(&app, &id)?;
    let chunks = app
        .export_chunked(&id)
        .map_err(|error| ApiError::internal("Open file for download", error))?;

    // Get streams from chunks
    let stream = futures::stream::iter(chunks.map(|chunk| {
        chunk
            .map(|bytes| Bytes::from(bytes))
            .map_err(log_stream_error)
    }));

    let mime = blob_store::guess_mime(&metadata.filename);

    let mut headers = secure_file_headers(&mime, &metadata.filename, false)?;
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(metadata.size));
    Ok((StatusCode::OK, headers, Body::from_stream(stream)))
}

async fn open_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> ApiResult<impl IntoResponse> {
    if let Err(e) = require_user(&jar, &app) {
        return Err(e);
    }

    let metadata = lookup_file(&app, &id)?;

    Ok(Redirect::to(&format!("/files/{}/stream", metadata.id)).into_response())
}

async fn video_player(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
) -> ApiResult<impl IntoResponse> {
    require_user(&jar, &app)?;
    let metadata = lookup_file(&app, &id)?;
    let mime = blob_store::guess_mime(&metadata.filename);
    if !mime.starts_with("video/") {
        return Err(ApiError::NotFound("Video file not found"));
    }

    let title = escape_html(&metadata.filename);
    let stream_url = format!("/files/{}/stream", percent_encode_filename(&id));
    let page = format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{title} · RSFS</title>
<style>
* {{ box-sizing: border-box; }}
body {{ margin: 0; min-height: 100vh; display: grid; grid-template-rows: auto 1fr; background: #171715; color: #f7f7f5; font: 15px system-ui, sans-serif; }}
header {{ padding: 14px 20px; border-bottom: 1px solid #3b3b38; overflow-wrap: anywhere; }}
main {{ min-height: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 14px; padding: 20px; }}
video {{ display: block; width: min(100%, 1200px); max-height: calc(100vh - 130px); background: #000; }}
</style></head><body><header>{title}</header><main>
<video controls autoplay preload="metadata" playsinline><source src="{stream_url}" type="{mime}">This browser cannot play this video format.</video>
</main></body></html>"#,
    );
    let mut response = Html(page).into_response();
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; media-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn upload_files(
    jar: CookieJar,
    State(app): State<Arc<App>>,
    multipart: Result<Multipart, MultipartRejection>,
) -> ApiResult<Json<Vec<FileMetadata>>> {
    require_user(&jar, &app)?;
    let mut multipart = multipart.map_err(|error| ApiError::RequestRejected {
        status: error.status(),
        message: error.body_text(),
    })?;

    let mut uploaded = Vec::new();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(format!("Multipart error: {error}")))?
    {
        use crate::crypto::CHUNK_SIZE;
        use std::io::Write;

        let filename = field.file_name().unwrap_or("unknown").to_string();
        let id = crate::domain::id::id_16();

        let mut file = app
            .storage
            .create_file_writer(&id)
            .map_err(|error| ApiError::internal("Create upload file", error))?;
        let mut encryptor = app
            .crypto
            .start_encryption(&id)
            .map_err(|error| ApiError::internal("Initialize upload encryption", error))?;
        file.write_all(encryptor.header())
            .map_err(|error| ApiError::internal("Write encrypted header", error))?;

        let mut buffer = Vec::with_capacity(CHUNK_SIZE);
        let mut total_bytes: u64 = 0;

        // Read from multipart in chunks
        while let Some(bytes) = field
            .chunk()
            .await
            .map_err(|error| ApiError::BadRequest(format!("Multipart read error: {error}")))?
        {
            buffer.extend_from_slice(&bytes);
            total_bytes += bytes.len() as u64;

            // Accumulated a full chunk — encrypt and write
            while buffer.len() >= CHUNK_SIZE {
                let chunk = buffer[..CHUNK_SIZE].to_vec();
                buffer.drain(..CHUNK_SIZE);

                let encrypted = encryptor
                    .encrypt_chunk(&chunk)
                    .map_err(|error| ApiError::internal("Encrypt upload chunk", error))?;
                file.write_all(&encrypted)
                    .map_err(|error| ApiError::internal("Write upload chunk", error))?;
            }
        }

        // Last incomplete chunk
        if !buffer.is_empty() {
            let encrypted = encryptor
                .encrypt_chunk(&buffer)
                .map_err(|error| ApiError::internal("Encrypt final upload chunk", error))?;
            file.write_all(&encrypted)
                .map_err(|error| ApiError::internal("Write final upload chunk", error))?;
        }

        file.flush()
            .map_err(|error| ApiError::internal("Flush upload", error))?;
        file.sync_all()
            .map_err(|error| ApiError::internal("Sync upload", error))?;

        // Save metadata
        let metadata = crate::domain::FileMetadata {
            id: id.clone(),
            filename: filename.clone(),
            size: total_bytes,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| ApiError::internal("Read current time", error))?
                .as_secs(),
        };

        // Publish only a fully synced encrypted file. Startup cleanup removes a blob
        // if the process later crashes before its metadata transaction succeeds.
        app.storage
            .finalize_file(&id)
            .map_err(|error| ApiError::internal("Publish upload", error))?;
        if let Err(error) = app.metadata.save_file(&metadata) {
            if let Err(cleanup_error) = app.storage.delete_file(&id) {
                tracing::error!(file_id = %id, error = %cleanup_error, "Failed to clean up unpublished blob");
            }
            return Err(ApiError::internal("Save upload metadata", error));
        }
        tracing::info!(file_id = %id, bytes = total_bytes, "Upload completed");

        uploaded.push(metadata);
    }

    if uploaded.is_empty() {
        return Err(ApiError::BadRequest("No files uploaded".into()));
    }

    Ok(Json(uploaded))
}

async fn logout(jar: CookieJar, State(app): State<Arc<App>>) -> impl IntoResponse {
    if let Some(cookie) = jar.get("rfs_session") {
        app.sessions.remove(cookie.value());
    }
    // Cleaning up temporary files older than 4 hours
    match app.storage.cleanup_stale_tmp_files(4 * 3600) {
        Ok(count) if count > 0 => tracing::info!(count, "Removed stale upload files"),
        Ok(_) => {}
        Err(error) => tracing::warn!(error = %error, "Logout upload cleanup failed"),
    }

    (
        StatusCode::OK,
        [(
            header::SET_COOKIE,
            "rfs_session=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict".to_string(),
        )],
    )
}

fn session_user(jar: &CookieJar, app: &App) -> ApiResult<crate::domain::UserAccount> {
    let cookie = jar.get("rfs_session").ok_or(ApiError::Forbidden)?;
    app.current_user(cookie.value())
        .map_err(|error| ApiError::internal("Read session user", error))?
        .ok_or(ApiError::Forbidden)
}

fn require_user(jar: &CookieJar, app: &App) -> ApiResult<()> {
    (session_user(jar, app)?.role == UserRole::User)
        .then_some(())
        .ok_or(ApiError::Forbidden)
}

async fn stream_file(
    jar: CookieJar,
    Path(id): Path<String>,
    State(app): State<Arc<App>>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    if let Err(e) = require_user(&jar, &app) {
        return Err(e);
    }

    let metadata = lookup_file(&app, &id)?;

    let file_size = metadata.size;
    let mime = blob_store::guess_mime(&metadata.filename);

    let range = headers
        .get(header::RANGE)
        .map(|value| value.to_str().ok())
        .flatten();
    match parse_single_range(range, file_size) {
        Err(()) => return Err(ApiError::InvalidRange(file_size)),
        Ok(Some((start, end))) => {
            if start > end || start >= file_size {
                return Err(ApiError::InvalidRange(file_size));
            }

            let length = end - start + 1;

            let chunks = app
                .export_range(&id, start, end)
                .map_err(|error| ApiError::internal("Open file range", error))?;

            let stream = futures::stream::iter(chunks.map(|chunk| {
                chunk.map_err(|error| {
                    tracing::error!(error = %error, "Range streaming failed");
                    error
                })
            }));

            let mut resp_headers = secure_file_headers(
                &mime,
                &metadata.filename,
                inline_safe(&mime, &metadata.filename),
            )?;
            resp_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            resp_headers.insert(
                header::CONTENT_RANGE,
                HeaderValue::from_str(&format!("bytes {start}-{end}/{file_size}"))
                    .map_err(|error| ApiError::internal("Build range header", error))?,
            );
            resp_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));

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
    let chunks = app
        .export_chunked(&id)
        .map_err(|error| ApiError::internal("Open file stream", error))?;

    let stream =
        futures::stream::iter(chunks.map(|c| c.map(Bytes::from).map_err(log_stream_error)));

    let mut resp_headers = secure_file_headers(
        &mime,
        &metadata.filename,
        inline_safe(&mime, &metadata.filename),
    )?;
    resp_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    resp_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file_size));

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

fn inline_safe(mime: &str, filename: &str) -> bool {
    matches!(
        mime,
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "image/svg+xml"
            | "application/pdf"
    ) || mime.starts_with("video/")
        || mime.starts_with("audio/")
        || is_text_preview(filename, mime)
}

fn is_text_preview(filename: &str, mime: &str) -> bool {
    let name = filename.to_ascii_lowercase();
    matches!(
        name.rsplit('.').next(),
        Some(
            "txt"
                | "md"
                | "markdown"
                | "text"
                | "log"
                | "csv"
                | "json"
                | "xml"
                | "yaml"
                | "yml"
                | "toml"
                | "ini"
                | "conf"
        )
    ) && matches!(
        mime,
        "text/plain"
            | "text/markdown"
            | "text/csv"
            | "application/json"
            | "application/xml"
            | "text/xml"
            | "application/octet-stream"
    )
}

fn secure_file_headers(mime: &str, filename: &str, inline: bool) -> ApiResult<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime)
            .map_err(|error| ApiError::internal("Build content type header", error))?,
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    let disposition = if inline { "inline" } else { "attachment" };
    let encoded_name = percent_encode_filename(filename);
    headers.insert(
        header::CONTENT_DISPOSITION,
        format!("{disposition}; filename=\"download\"; filename*=UTF-8''{encoded_name}")
            .parse()
            .map_err(|error| ApiError::internal("Build content disposition header", error))?,
    );
    if inline && mime == "image/svg+xml" {
        headers.insert(
            "content-security-policy",
            HeaderValue::from_static("sandbox"),
        );
    }
    Ok(headers)
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
    use crate::{
        auth::hash_password,
        config::Config,
        domain::{UserAccount, UserRole},
    };
    use axum::{body::to_bytes, http::Request};
    use std::{fs, path::PathBuf};
    use tower::ServiceExt;

    fn test_app() -> (Arc<App>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("rfs-web-test-{}", uuid::Uuid::new_v4()));
        let config = Config {
            blobs_dir: dir.join("blobs"),
            tmp_dir: dir.join("tmp"),
            metadata_path: dir.join("metadata.redb"),
            data_dir: dir.clone(),
            encryption_key: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                [7u8; 32],
            ),
            bind_address: "127.0.0.1:0".into(),
        };
        App::initialize(&config, "admin", "test-password").unwrap();
        let app = Arc::new(App::new(config).unwrap());
        app.metadata
            .create_user(&UserAccount {
                id: "test-user".into(),
                username: "user".into(),
                password_hash: hash_password("test-password").unwrap(),
                auth_version: 1,
                role: UserRole::User,
                password_change_required: false,
            })
            .unwrap();
        (app, dir)
    }

    fn user_cookie(app: &App) -> String {
        format!(
            "rfs_session={}",
            app.sessions
                .create(&app.metadata.get_user_by_username("user").unwrap().unwrap())
                .unwrap()
        )
    }
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
        assert!(inline_safe("application/pdf", "document.pdf"));
        assert!(inline_safe("image/svg+xml", "drawing.svg"));
        assert!(inline_safe("text/markdown", "README.md"));
        assert!(inline_safe("text/plain", "notes.txt"));
        assert!(inline_safe("application/json", "data.json"));
        assert!(!inline_safe("text/html", "page.html"));
        assert!(!inline_safe("application/zip", "archive.zip"));
    }

    #[tokio::test]
    async fn current_user_endpoint_returns_stored_username() {
        let (app, dir) = test_app();
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/me")
                    .header(header::COOKIE, user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["username"],
            "user"
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn browser_navigation_gets_pages_for_all_error_statuses() {
        let (app, dir) = test_app();
        app.metadata
            .save_file(&FileMetadata {
                id: "range-test".into(),
                filename: "video.mp4".into(),
                size: 10,
                created_at: 0,
            })
            .unwrap();
        app.metadata.insert_invalid_record("corrupt");
        for (uri, method, body, status) in [
            ("/login", "POST", "{", StatusCode::BAD_REQUEST),
            (
                "/login",
                "POST",
                r#"{"username":"unknown","password":"wrong"}"#,
                StatusCode::UNAUTHORIZED,
            ),
            (
                "/assets/icons/unknown.png",
                "GET",
                "",
                StatusCode::NOT_FOUND,
            ),
            ("/unknown", "GET", "", StatusCode::NOT_FOUND),
            ("/health", "DELETE", "", StatusCode::METHOD_NOT_ALLOWED),
            (
                "/files/range-test/stream",
                "GET",
                "",
                StatusCode::RANGE_NOT_SATISFIABLE,
            ),
            (
                "/files/corrupt/download",
                "GET",
                "",
                StatusCode::INTERNAL_SERVER_ERROR,
            ),
        ] {
            let response = create_router(app.clone())
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .method(method)
                        .header(header::ACCEPT, "text/html,application/xhtml+xml,*/*;q=0.8")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::RANGE, "bytes=100-")
                        .header(header::COOKIE, user_cookie(&app))
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{uri}");
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/html; charset=utf-8"
            );
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */10");
            }
            let body = String::from_utf8(
                to_bytes(response.into_body(), 16384)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            assert!(body.contains("Back"));
            assert!(!body.contains("not valid JSON"));
        }
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/storage")
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = String::from_utf8(
            to_bytes(response.into_body(), 16384)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(body.contains("Access denied"));
        let response = create_router(app)
            .oneshot(
                Request::builder()
                    .uri("/unknown")
                    .method("HEAD")
                    .header(header::ACCEPT, "text/html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(
            to_bytes(response.into_body(), 16384)
                .await
                .unwrap()
                .is_empty()
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn database_errors_return_json_without_panicking() {
        let (app, dir) = test_app();
        app.metadata.insert_invalid_record("corrupt");
        for uri in [
            "/files",
            "/files/corrupt",
            "/files/corrupt/open",
            "/files/corrupt/stream",
            "/files/corrupt/download",
        ] {
            let response = create_router(app.clone())
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("cookie", user_cookie(&app))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::INTERNAL_SERVER_ERROR,
                "{uri}"
            );
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                json!({"error": "Internal server error"})
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn rejected_requests_use_json_errors() {
        let (app, dir) = test_app();
        for (uri, method, content_type, body, status) in [
            (
                "/files?per_page=bad",
                "GET",
                "application/json",
                "",
                StatusCode::BAD_REQUEST,
            ),
            (
                "/login",
                "POST",
                "application/json",
                "{",
                StatusCode::BAD_REQUEST,
            ),
            (
                "/files/upload",
                "POST",
                "application/octet-stream",
                "bad",
                StatusCode::BAD_REQUEST,
            ),
            (
                "/files/missing/stream",
                "GET",
                "application/json",
                "",
                StatusCode::NOT_FOUND,
            ),
        ] {
            let response = create_router(app.clone())
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .method(method)
                        .header("cookie", user_cookie(&app))
                        .header("content-type", content_type)
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{uri}");
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
            let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            assert!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"].is_string()
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn storage_reports_data_directory_usage() {
        let (app, dir) = test_app();
        fs::write(dir.join("sample.bin"), [1u8; 17]).unwrap();
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/storage")
                    .header("cookie", user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let stats: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(stats["used_bytes"].as_u64().unwrap() >= 17);
        assert!(stats["total_bytes"].as_u64().unwrap() > 0);
        assert!(stats["available_bytes"].as_u64().unwrap() > 0);
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn file_list_returns_requested_server_side_page() {
        let (app, dir) = test_app();
        for index in 0..23 {
            app.metadata
                .save_file(&FileMetadata {
                    id: format!("id-{index}"),
                    filename: format!("item-{index:02}.txt"),
                    size: index,
                    created_at: index,
                })
                .unwrap();
        }
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/files?page=2&per_page=10&sort=name-asc&search=item-")
                    .header("cookie", user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(result["total"], 23);
        assert_eq!(result["page"], 2);
        assert_eq!(result["per_page"], 10);
        assert_eq!(result["files"].as_array().unwrap().len(), 10);
        assert_eq!(result["files"][0]["filename"], "item-10.txt");
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn video_player_uses_native_controls_and_escapes_filename() {
        let (app, dir) = test_app();
        app.metadata
            .save_file(&FileMetadata {
                id: "video-id".into(),
                filename: "movie<demo>.mp4".into(),
                size: 123,
                created_at: 1,
            })
            .unwrap();
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .uri("/files/video-id/player")
                    .header("cookie", user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .contains_key(header::CONTENT_SECURITY_POLICY)
        );
        let page = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(page.contains("movie&lt;demo&gt;.mp4"));
        assert!(page.contains("<video controls autoplay preload=\"metadata\" playsinline>"));
        assert!(page.contains("/files/video-id/stream"));
        assert!(!page.contains("Playback not supported?"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn upload_and_delete_endpoints_change_file_list() {
        let (app, dir) = test_app();
        let boundary = "test-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"hello.txt\"\r\nContent-Type: text/plain\r\n\r\nhello rsfs\r\n--{boundary}--\r\n"
        );
        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/files/upload")
                    .header("cookie", user_cookie(&app))
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let uploaded: Vec<FileMetadata> =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(uploaded.len(), 1);
        let id = uploaded[0].id.clone();

        let response = create_router(app.clone())
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/files/{id}"))
                    .header("cookie", user_cookie(&app))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(app.list_files().unwrap().is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
}
